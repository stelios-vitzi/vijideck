//! Conservative, directory-based cleanup. Never follows symlinks or Windows reparse points.
use std::{
    collections::{HashSet, VecDeque, hash_map::DefaultHasher},
    ffi::OsStr,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Stamp {
    directory: bool,
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub source: &'static str,
    pub kind: &'static str,
    pub bytes: u64,
    pub files: usize,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub issue: Option<String>,
    pub search: String,
    /// Present only for a complete measurement; deletion requires it.
    fingerprint: Option<Fingerprint>,
}

/// Constant-size, order-independent summary of a folder's contents (relative
/// paths, kinds, sizes, and timestamps). Two independently seeded 64-bit
/// SipHash sums replace a per-file manifest, so memory no longer grows with the
/// number of measured files, and subtrees can be measured in any order or in
/// parallel. Deletion re-inventories the folder and requires an exact match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Fingerprint {
    entries: u64,
    bytes: u64,
    a: u64,
    b: u64,
}

impl Fingerprint {
    fn merge(&mut self, other: &Fingerprint) {
        self.entries += other.entries;
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.a = self.a.wrapping_add(other.a);
        self.b = self.b.wrapping_add(other.b);
    }
}

#[derive(Default)]
struct Measured {
    fingerprint: Fingerprint,
    files: usize,
    newest: Option<SystemTime>,
}

impl Measured {
    fn record(&mut self, parent: u64, name: &OsStr, stamp: &Stamp) {
        // Directory timestamps are excluded: any added, removed, or changed child
        // already changes the fingerprint, and on NTFS the timestamps reported by
        // parent enumeration are updated lazily, which would cause false mismatches.
        let identity = if stamp.directory { None } else { Some(stamp) };
        let entry = |seed: u8| {
            let mut h = DefaultHasher::new();
            (seed, parent, name, stamp.directory, identity).hash(&mut h);
            h.finish()
        };
        let f = &mut self.fingerprint;
        f.entries += 1;
        f.bytes = f.bytes.saturating_add(stamp.len);
        f.a = f.a.wrapping_add(entry(0xa5));
        f.b = f.b.wrapping_add(entry(0x5a));
        self.files += usize::from(!stamp.directory);
        self.newest = self.newest.max(stamp.modified);
    }
    fn merge(&mut self, other: &Measured) {
        self.fingerprint.merge(&other.fingerprint);
        self.files += other.files;
        self.newest = self.newest.max(other.newest);
    }
}

fn path_key(relative: &Path) -> u64 {
    let mut h = DefaultHasher::new();
    relative.hash(&mut h);
    h.finish()
}

#[derive(Default)]
pub struct Scan {
    pub root: PathBuf,
    pub entries: Vec<Candidate>,
    pub visited: usize,
    pub measured_files: usize,
    pub workers: usize,
    pub elapsed: Duration,
    pub skipped: usize,
    pub warnings: Vec<String>,
    pub cancelled: bool,
}

#[derive(Clone, Default)]
pub struct Progress {
    pub directories: usize,
    pub files: usize,
    pub bytes: u64,
    pub found: usize,
    pub path: PathBuf,
}
pub enum ScanUpdate {
    Started(PathBuf, usize),
    Progress(Progress),
    Found(Candidate),
}

#[derive(Default)]
pub struct DeleteReport {
    pub removed: Vec<PathBuf>,
    pub bytes: u64,
    pub failures: Vec<String>,
}

fn linked(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// Filesystem identity used to keep traversal on one filesystem. On Unix this
/// catches every mount crossing, including ones absent from any mount list.
/// Windows volume mounts are reparse points and are already rejected by `linked`.
fn device(meta: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.dev()
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

/// Every mount point the OS knows about. `sysinfo::Disks` filters out tmpfs,
/// NFS, CIFS, and other virtual/network filesystems, so on Linux the kernel
/// mount table is read directly.
fn mount_points() -> HashSet<PathBuf> {
    #[allow(unused_mut)]
    let mut points: HashSet<PathBuf> = sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter_map(|d| d.mount_point().canonicalize().ok())
        .collect();
    #[cfg(target_os = "linux")]
    {
        let table = fs::read("/proc/self/mountinfo")
            .map(|t| (t, 4))
            .or_else(|_| fs::read("/proc/mounts").map(|t| (t, 1)));
        if let Ok((table, field)) = table {
            points.extend(linux_mount_points(&table, field));
        }
    }
    points
}

/// Mount points from a mountinfo (field 4) or /proc/mounts (field 1) table.
/// Paths are octal-escaped (`\040` for space) and may not be UTF-8.
#[cfg(target_os = "linux")]
fn linux_mount_points(table: &[u8], field: usize) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    table
        .split(|&b| b == b'\n')
        .filter_map(|line| line.split(|&b| b == b' ').nth(field))
        .filter(|raw| raw.first() == Some(&b'/'))
        .map(|raw| {
            let mut path = Vec::with_capacity(raw.len());
            let mut i = 0;
            while i < raw.len() {
                let octal = raw
                    .get(i + 1..i + 4)
                    .filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)));
                match octal {
                    Some(d) if raw[i] == b'\\' => {
                        path.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                        i += 4;
                    }
                    _ => {
                        path.push(raw[i]);
                        i += 1;
                    }
                }
            }
            PathBuf::from(std::ffi::OsString::from_vec(path))
        })
        .collect()
}

/// Reject a path when it, or any ancestor below the scan root, is a mount point
/// or sits on a different filesystem than the root.
fn check_boundary(root: &Path, path: &Path, mounts: &HashSet<PathBuf>) -> io::Result<()> {
    let root_device = device(&fs::symlink_metadata(root)?);
    for ancestor in path.ancestors().take_while(|a| *a != root) {
        if mounts.contains(ancestor) || root_device != device(&fs::symlink_metadata(ancestor)?) {
            return Err(io::Error::other("Path is on another mounted filesystem"));
        }
    }
    Ok(())
}

fn stamp(meta: &fs::Metadata) -> Stamp {
    Stamp {
        directory: meta.is_dir(),
        len: if meta.is_file() { meta.len() } else { 0 },
        modified: meta.modified().ok(),
        created: meta.created().ok(),
    }
}

fn has(names: &HashSet<String>, name: &str) -> bool {
    names.contains(name)
}

fn classify(path: &Path, siblings: &HashSet<String>) -> Option<(&'static str, &'static str)> {
    let name = path.file_name()?.to_str()?;
    let node = has(siblings, "package.json");
    match name {
        "target" if has(siblings, "Cargo.toml") => Some(("Rust / Cargo", "Build")),
        "target" if has(siblings, "pom.xml") => Some(("Maven", "Build")),
        "node_modules" if node => Some(("Node.js dependencies", "Cache")),
        ".next" if node => Some(("Next.js", "Build")),
        ".nuxt" | ".output" if node => Some(("Nuxt", "Build")),
        ".parcel-cache" if node => Some(("Parcel", "Cache")),
        ".turbo" if node => Some(("Turborepo", "Cache")),
        ".svelte-kit" if node => Some(("SvelteKit", "Build")),
        "build" if has(siblings, "CMakeLists.txt") && path.join("CMakeCache.txt").is_file() => {
            Some(("CMake", "Build"))
        }
        "bin" | "obj"
            if siblings.iter().any(|n| {
                n.ends_with(".csproj") || n.ends_with(".fsproj") || n.ends_with(".vbproj")
            }) =>
        {
            Some((".NET", "Build"))
        }
        "__pycache__" => Some(("Python bytecode", "Cache")),
        ".pytest_cache" => Some(("pytest", "Cache")),
        ".mypy_cache" => Some(("mypy", "Cache")),
        ".ruff_cache" => Some(("Ruff", "Cache")),
        ".gradle" if has(siblings, "build.gradle") || has(siblings, "build.gradle.kts") => {
            Some(("Gradle", "Cache"))
        }
        "build" if has(siblings, "build.gradle") || has(siblings, "build.gradle.kts") => {
            Some(("Gradle", "Build"))
        }
        _ => None,
    }
}

fn excluded(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    if crate::volumes::linux_internal_path(path) {
        return true;
    }
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if matches!(
        name.as_str(),
        ".git" | ".hg" | ".svn" | "$recycle.bin" | "system volume information"
    ) {
        return true;
    }
    #[cfg(windows)]
    {
        name == "windows" && path.parent().is_some_and(|p| p.parent().is_none())
    }
    #[cfg(not(windows))]
    {
        matches!(path.to_str(), Some("/proc" | "/sys" | "/dev"))
    }
}

fn note(scan: &mut Scan, path: &Path, error: impl std::fmt::Display) {
    scan.skipped += 1;
    if scan.warnings.len() < 100 {
        scan.warnings.push(format!("{}: {error}", path.display()));
    }
}

/// A previously found folder, as recorded in the result cache.
pub struct Cached {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: usize,
}

/// Show previous results instantly while a fresh scan runs. Each path is still
/// validated and reclassified against the current filesystem, but contents are
/// not re-measured: sizes come from the cache, and these candidates carry no
/// fingerprint, so they cannot be deleted until the scan measures them.
pub fn cached_preview(
    root: &Path,
    cached: Vec<Cached>,
    cancel: &AtomicBool,
    mut emit: impl FnMut(Candidate),
) {
    let mut mounts = mount_points();
    mounts.remove(root);
    for entry in cached {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if let Some(candidate) = preview_one(root, entry, &mounts) {
            emit(candidate);
        }
    }
}

fn preview_one(root: &Path, entry: Cached, mounts: &HashSet<PathBuf>) -> Option<Candidate> {
    let path = entry.path;
    validate_path(root, &path).ok()?;
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_dir() || linked(&meta) {
        return None;
    }
    let names: HashSet<String> = fs::read_dir(path.parent()?)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let (source, kind) = classify(&path, &names)?;
    check_boundary(root, &path, mounts).ok()?;
    Some(Candidate {
        search: format!("{} {source} {kind}", path.display()).to_lowercase(),
        path,
        source,
        kind,
        bytes: entry.bytes,
        files: entry.files,
        modified: meta.modified().ok(),
        created: meta.created().ok(),
        issue: None,
        fingerprint: None,
    })
}

/// A bounded pool overlaps independent directory reads. Rotational disks use one
/// worker to avoid seek storms; SSD/unknown storage uses at most eight workers.
pub fn scan(root: &Path, cancel: &AtomicBool, update: impl FnMut(ScanUpdate) + Send) -> Scan {
    scan_with_workers(root, cancel, 0, update)
}

fn scan_with_workers(
    root: &Path,
    cancel: &AtomicBool,
    requested: usize,
    update: impl FnMut(ScanUpdate) + Send,
) -> Scan {
    let started = Instant::now();
    let mut result = Scan {
        root: root.to_path_buf(),
        ..Scan::default()
    };
    let root = match root.canonicalize() {
        Ok(root) if root.is_dir() => root,
        Ok(_) => {
            note(&mut result, root, "Choose a directory or disk");
            return result;
        }
        Err(e) => {
            note(&mut result, root, e);
            return result;
        }
    };
    result.root = root.clone();
    #[cfg(target_os = "linux")]
    if crate::volumes::linux_internal_path(&root) {
        note(
            &mut result,
            &root,
            "System and container storage is excluded from cleanup",
        );
        return result;
    }
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .filter_map(|d| d.mount_point().canonicalize().ok().map(|p| (d, p)))
        .filter(|(_, p)| root.starts_with(p))
        .max_by_key(|(_, p)| p.components().count());
    let rotational = disk.is_some_and(|(d, _)| d.kind() == sysinfo::DiskKind::HDD);
    let workers = if requested > 0 {
        requested.clamp(1, 8)
    } else if rotational {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(2, usize::from)
            .clamp(1, 8)
    };
    result.workers = workers;
    let root_device = match fs::symlink_metadata(&root) {
        Ok(meta) => device(&meta),
        Err(e) => {
            note(&mut result, &root, e);
            return result;
        }
    };
    let engine = Engine {
        queue: Mutex::new(Work {
            jobs: VecDeque::from([Job::Discover(root.clone(), root_device)]),
            idle: 0,
            finished: false,
        }),
        ready: Condvar::new(),
        workers,
        idle: AtomicUsize::new(0),
        result: Mutex::new(result),
        cancel,
        mounts: {
            let mut mounts = mount_points();
            mounts.remove(&root);
            mounts
        },
        reporter: Mutex::new((Instant::now(), update)),
        directories: AtomicUsize::new(0),
        files: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
        found: AtomicUsize::new(0),
    };
    (engine.reporter.lock().unwrap().1)(ScanUpdate::Started(root.clone(), workers));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let engine = &engine;
            scope.spawn(move || engine.run());
        }
    });
    engine.progress(&root, true);
    let mut result = engine.result.into_inner().unwrap();
    result.visited = engine.directories.load(Ordering::Relaxed);
    result.measured_files = engine.files.load(Ordering::Relaxed);
    result.cancelled = cancel.load(Ordering::Relaxed);
    result.elapsed = started.elapsed();
    result.entries.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

enum Job {
    /// A directory outside any candidate, with its filesystem identity.
    Discover(PathBuf, u64),
    /// One directory (relative path) inside a candidate being measured.
    Measure(Arc<Measuring>, PathBuf),
}

/// A candidate measured as independent per-directory jobs, so one huge folder
/// is spread across all workers. It is reported once its last job finishes.
struct Measuring {
    path: PathBuf,
    device: u64,
    state: Mutex<MeasureState>,
}
struct MeasureState {
    candidate: Option<Candidate>,
    measured: Measured,
    outstanding: usize,
    issue: Option<String>,
}
struct Work {
    jobs: VecDeque<Job>,
    idle: usize,
    finished: bool,
}
struct Engine<'a, F> {
    queue: Mutex<Work>,
    ready: Condvar,
    workers: usize,
    /// Lock-free hint of `Work::idle`, read after every job.
    idle: AtomicUsize,
    result: Mutex<Scan>,
    cancel: &'a AtomicBool,
    mounts: HashSet<PathBuf>,
    reporter: Mutex<(Instant, F)>,
    directories: AtomicUsize,
    files: AtomicUsize,
    bytes: AtomicU64,
    found: AtomicUsize,
}
impl<F: FnMut(ScanUpdate) + Send> Engine<'_, F> {
    fn progress(&self, path: &Path, force: bool) {
        // UI updates are coalesced globally, not emitted once per worker/file.
        if let Ok(mut reporter) = self.reporter.try_lock()
            && (force || reporter.0.elapsed() >= Duration::from_millis(100))
        {
            reporter.0 = Instant::now();
            (reporter.1)(ScanUpdate::Progress(Progress {
                directories: self.directories.load(Ordering::Relaxed),
                files: self.files.load(Ordering::Relaxed),
                bytes: self.bytes.load(Ordering::Relaxed),
                found: self.found.load(Ordering::Relaxed),
                path: path.to_path_buf(),
            }));
        }
    }
    /// Each worker runs depth-first on its own stack without locking, and hands
    /// its oldest (shallowest, usually largest) pending directories to the shared
    /// queue only when another worker is idle. The scan ends when every worker
    /// is idle and the shared queue is empty.
    fn run(&self) {
        let mut local: Vec<Job> = Vec::new();
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                local.clear();
            }
            let job = match local.pop() {
                Some(job) => job,
                None => {
                    let mut work = self.queue.lock().unwrap();
                    loop {
                        if self.cancel.load(Ordering::Relaxed) {
                            work.jobs.clear();
                        }
                        if let Some(job) = work.jobs.pop_back() {
                            break job;
                        }
                        if work.finished {
                            return;
                        }
                        work.idle += 1;
                        if work.idle == self.workers {
                            work.finished = true;
                            self.ready.notify_all();
                            return;
                        }
                        self.idle.store(work.idle, Ordering::Relaxed);
                        work = self.ready.wait(work).unwrap();
                        work.idle -= 1;
                        self.idle.store(work.idle, Ordering::Relaxed);
                    }
                }
            };
            local.extend(match job {
                Job::Discover(path, device) => self.discover(path, device),
                Job::Measure(task, relative) => self.measure(task, relative),
            });
            if local.len() > 1 && self.idle.load(Ordering::Relaxed) > 0 {
                let mut work = self.queue.lock().unwrap();
                let shared = local.len() / 2;
                work.jobs.extend(local.drain(..shared));
                self.ready.notify_all();
            }
        }
    }
    fn discover(&self, dir: PathBuf, dir_device: u64) -> Vec<Job> {
        if self.cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        self.directories.fetch_add(1, Ordering::Relaxed);
        self.progress(&dir, false);
        let mut part = Scan::default();
        let mut directories = Vec::new();
        let mut names = HashSet::new();
        // The directory's own metadata was already checked when its parent was
        // enumerated, so no extra per-directory stat/open is needed here.
        let children = match fs::read_dir(&dir) {
            Ok(children) => children,
            Err(e) => {
                note(&mut part, &dir, e);
                self.merge(part);
                return Vec::new();
            }
        };
        for child in children {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let child = match child {
                Ok(c) => c,
                Err(e) => {
                    note(&mut part, &dir, e);
                    continue;
                }
            };
            let name = child.file_name();
            let name = name.to_string_lossy();
            // Only retain names used by detection. Avoid allocating paths or
            // requesting metadata for ordinary source files.
            if matches!(
                name.as_ref(),
                "Cargo.toml"
                    | "pom.xml"
                    | "package.json"
                    | "build.gradle"
                    | "build.gradle.kts"
                    | "CMakeLists.txt"
            ) || name.ends_with(".csproj")
                || name.ends_with(".fsproj")
                || name.ends_with(".vbproj")
            {
                names.insert(name.into_owned());
            }
            let kind = match child.file_type() {
                Ok(k) => k,
                Err(e) => {
                    note(&mut part, &child.path(), e);
                    continue;
                }
            };
            if kind.is_symlink() {
                part.skipped += 1;
                continue;
            }
            if !kind.is_dir() {
                continue;
            }
            let path = child.path();
            if excluded(&path) || self.mounts.contains(&path) {
                part.skipped += 1;
                continue;
            }
            match child.metadata() {
                Ok(meta) if !linked(&meta) && device(&meta) == dir_device => {
                    directories.push((path, meta))
                }
                Ok(_) => part.skipped += 1,
                Err(e) => note(&mut part, &path, e),
            }
        }
        self.merge(part);
        directories
            .into_iter()
            .map(|(path, meta)| {
                if let Some((source, kind)) = classify(&path, &names) {
                    let candidate = Candidate {
                        search: format!("{} {source} {kind}", path.display()).to_lowercase(),
                        path: path.clone(),
                        source,
                        kind,
                        bytes: 0,
                        files: 0,
                        modified: meta.modified().ok(),
                        created: meta.created().ok(),
                        issue: None,
                        fingerprint: None,
                    };
                    let task = Measuring {
                        path,
                        device: dir_device,
                        state: Mutex::new(MeasureState {
                            candidate: Some(candidate),
                            measured: Measured::default(),
                            outstanding: 1,
                            issue: None,
                        }),
                    };
                    Job::Measure(Arc::new(task), PathBuf::new())
                } else {
                    Job::Discover(path, device(&meta))
                }
            })
            .collect()
    }
    fn merge(&self, part: Scan) {
        if part.skipped == 0 && part.warnings.is_empty() {
            return;
        }
        let mut result = self.result.lock().unwrap();
        result.skipped += part.skipped;
        let remaining = 100usize.saturating_sub(result.warnings.len());
        result
            .warnings
            .extend(part.warnings.into_iter().take(remaining));
    }
    fn measure(&self, task: Arc<Measuring>, relative: PathBuf) -> Vec<Job> {
        let mut measured = Measured::default();
        let mut subdirs = Vec::new();
        let skip =
            self.cancel.load(Ordering::Relaxed) || task.state.lock().unwrap().issue.is_some();
        let outcome = if skip {
            Ok(())
        } else {
            self.directories.fetch_add(1, Ordering::Relaxed);
            let outcome = measure_dir(
                &task.path,
                &relative,
                task.device,
                &self.mounts,
                self.cancel,
                &mut measured,
                None,
                &mut subdirs,
            );
            self.files.fetch_add(measured.files, Ordering::Relaxed);
            self.progress(&task.path, false);
            outcome
        };
        let finished = {
            let mut state = task.state.lock().unwrap();
            if let Err(e) = outcome {
                state.issue.get_or_insert_with(|| e.to_string());
                subdirs.clear();
            }
            state.measured.merge(&measured);
            state.outstanding = state.outstanding + subdirs.len() - 1;
            // Interrupted candidates are not presented as fully measured.
            if state.outstanding == 0 && !self.cancel.load(Ordering::Relaxed) {
                let mut candidate = state.candidate.take().expect("finished once");
                if let Some(issue) = state.issue.take() {
                    candidate.issue = Some(issue);
                } else {
                    let m = &state.measured;
                    candidate.bytes = m.fingerprint.bytes;
                    candidate.files = m.files;
                    candidate.modified = candidate.modified.max(m.newest);
                    candidate.fingerprint = Some(m.fingerprint);
                }
                Some(candidate)
            } else {
                None
            }
        };
        if let Some(candidate) = finished {
            self.bytes.fetch_add(candidate.bytes, Ordering::Relaxed);
            self.found.fetch_add(1, Ordering::Relaxed);
            self.result.lock().unwrap().entries.push(candidate.clone());
            (self.reporter.lock().unwrap().1)(ScanUpdate::Found(candidate));
        }
        subdirs
            .into_iter()
            .map(|relative| Job::Measure(task.clone(), relative))
            .collect()
    }
}

/// Check a candidate folder itself: a real directory on its parent's filesystem.
/// Returns its stamp and the filesystem everything below it must stay on.
fn candidate_root(path: &Path, mounts: &HashSet<PathBuf>) -> io::Result<(Stamp, u64)> {
    let parent_device = device(&fs::symlink_metadata(path.parent().unwrap_or(path))?);
    let meta = fs::symlink_metadata(path)?;
    if mounts.contains(path) || linked(&meta) || !meta.is_dir() || device(&meta) != parent_device {
        return Err(io::Error::other(
            "Contains a mount, link, reparse point, or special file; deletion disabled",
        ));
    }
    Ok((stamp(&meta), parent_device))
}

/// Record one directory of a candidate (plus the candidate itself when
/// `relative` is empty). Entries are keyed by their parent's relative path, so
/// serial and parallel traversals produce identical fingerprints. Uses the
/// metadata returned by enumeration (no per-file stat/open on Windows) and
/// allocates no per-file paths unless a deletion listing is requested.
#[allow(clippy::too_many_arguments)]
fn measure_dir(
    root: &Path,
    relative: &Path,
    root_device: u64,
    mounts: &HashSet<PathBuf>,
    cancel: &AtomicBool,
    measured: &mut Measured,
    mut listing: Option<&mut Vec<(PathBuf, Stamp)>>,
    subdirs: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::other("Scan cancelled"));
    }
    if relative.as_os_str().is_empty() {
        let (stamp, _) = candidate_root(root, mounts)?;
        measured.record(path_key(relative), OsStr::new(""), &stamp);
        if let Some(listing) = listing.as_deref_mut() {
            listing.push((root.to_path_buf(), stamp));
        }
    }
    let dir = root.join(relative);
    let parent = path_key(relative);
    for child in fs::read_dir(&dir)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("Scan cancelled"));
        }
        let child = child?;
        let meta = child.metadata()?; // Does not follow links; cached by Windows enumeration.
        if linked(&meta) || (!meta.is_file() && !meta.is_dir()) {
            return Err(io::Error::other(
                "Contains a link, reparse point, or special file; deletion disabled",
            ));
        }
        let name = child.file_name();
        let stamp = stamp(&meta);
        measured.record(parent, &name, &stamp);
        if meta.is_dir() {
            let path = dir.join(&name);
            if device(&meta) != root_device || mounts.contains(&path) {
                return Err(io::Error::other(
                    "Contains a mount, link, reparse point, or special file; deletion disabled",
                ));
            }
            subdirs.push(relative.join(&name));
            if let Some(listing) = listing.as_deref_mut() {
                listing.push((path, stamp));
            }
        } else if let Some(listing) = listing.as_deref_mut() {
            listing.push((dir.join(&name), stamp));
        }
    }
    Ok(())
}

/// Strict serial inventory used right before deletion. Each queued directory is
/// re-checked fresh so a folder swapped for a link after listing is refused.
fn inventory(
    root: &Path,
    mounts: &HashSet<PathBuf>,
    listing: &mut Vec<(PathBuf, Stamp)>,
) -> io::Result<Measured> {
    let cancel = AtomicBool::new(false);
    let (_, root_device) = candidate_root(root, mounts)?;
    let mut measured = Measured::default();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        if !relative.as_os_str().is_empty() {
            let meta = fs::symlink_metadata(root.join(&relative))?;
            if linked(&meta) || !meta.is_dir() || device(&meta) != root_device {
                return Err(io::Error::other(
                    "Contains a mount, link, reparse point, or special file; deletion disabled",
                ));
            }
        }
        measure_dir(
            root,
            &relative,
            root_device,
            mounts,
            &cancel,
            &mut measured,
            Some(listing),
            &mut pending,
        )?;
    }
    Ok(measured)
}

/// Reject stale scans and link substitutions. Deletion is deliberately non-recursive:
/// new files appearing after validation prevent directory removal.
pub fn delete(root: &Path, candidates: &[Candidate]) -> DeleteReport {
    let mut report = DeleteReport::default();
    let mut mounts = mount_points();
    mounts.remove(root);
    for candidate in candidates {
        match delete_one(root, candidate, &mounts) {
            Ok(bytes) => {
                report.bytes = report.bytes.saturating_add(bytes);
                report.removed.push(candidate.path.clone());
            }
            Err(e) => report.failures.push(format!(
                "{}: {e}. Rescan before retrying; some files may have been removed.",
                candidate.path.display()
            )),
        }
    }
    report
}

fn validate_path(root: &Path, path: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if crate::volumes::linux_internal_path(path) {
        return Err(io::Error::other(
            "System and container storage is excluded from cleanup",
        ));
    }
    if !root.is_absolute()
        || path == root
        || !path.starts_with(root)
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(io::Error::other("Path is outside the scanned root"));
    }
    // Check every ancestor including the root, without following links.
    for ancestor in path.ancestors() {
        if linked(&fs::symlink_metadata(ancestor)?) {
            return Err(io::Error::other(
                "Path now contains a link or reparse point",
            ));
        }
    }
    Ok(())
}

fn delete_one(root: &Path, candidate: &Candidate, mounts: &HashSet<PathBuf>) -> io::Result<u64> {
    let Some(scanned) = candidate.fingerprint.filter(|_| candidate.issue.is_none()) else {
        return Err(io::Error::other("Incomplete scan"));
    };
    validate_path(root, &candidate.path)?;
    check_boundary(root, &candidate.path, mounts)?;
    // The listing exists only for this one folder while it is being deleted.
    let mut current = Vec::new();
    if inventory(&candidate.path, mounts, &mut current)?.fingerprint != scanned {
        return Err(io::Error::other("Contents changed since scanning"));
    }
    let mut directories = Vec::new();
    let mut bytes = 0u64;
    for (path, expected) in current {
        validate_path(root, &path)?;
        if expected.directory {
            directories.push(path);
            continue;
        }
        if stamp(&fs::symlink_metadata(&path)?) != expected {
            return Err(io::Error::other("File changed during deletion"));
        }
        fs::remove_file(&path)?;
        bytes = bytes.saturating_add(expected.len);
    }
    directories.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in directories {
        validate_path(root, &dir)?;
        fs::remove_dir(dir)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vijideck-cleanup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fn scan(&self) -> Scan {
            scan(&self.0, &AtomicBool::new(false), |_| {})
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn container_storage_is_skipped_in_scan_and_cached_results() {
        let f = Fixture::new();
        f.file(".local/share/docker/volumes/project/package.json", "{}");
        f.file(
            ".local/share/docker/volumes/project/node_modules/pkg/file",
            "keep",
        );
        f.file("project/package.json", "{}");
        f.file("project/node_modules/pkg/file", "123");
        let scan = f.scan();
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].path, f.0.join("project/node_modules"));
        let container_path = f.0.join(".local/share/docker/volumes/project/node_modules");
        let mut preview = Vec::new();
        cached_preview(
            &scan.root,
            vec![Cached {
                path: container_path,
                bytes: 4,
                files: 1,
            }],
            &AtomicBool::new(false),
            |candidate| preview.push(candidate),
        );
        assert!(preview.is_empty());
        let direct = super::scan(
            &f.0.join(".local/share/docker"),
            &AtomicBool::new(false),
            |_| {},
        );
        assert!(direct.entries.is_empty());
        assert!(direct.skipped > 0);
    }

    /// Real-tree benchmark: `VIJIDECK_BENCH_ROOT=<dir> cargo test --release
    /// benchmark_real_tree -- --ignored --nocapture`. Reports peak process memory.
    #[test]
    #[ignore]
    fn benchmark_real_tree() {
        let Some(root) = std::env::var_os("VIJIDECK_BENCH_ROOT") else {
            return;
        };
        let done = AtomicBool::new(false);
        let peak = std::thread::scope(|scope| {
            let sampler = scope.spawn(|| {
                let pid = sysinfo::get_current_pid().unwrap();
                let mut system = sysinfo::System::new();
                let mut peak = 0;
                while !done.load(Ordering::Relaxed) {
                    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                    peak = peak.max(system.process(pid).map_or(0, |p| p.memory()));
                    std::thread::sleep(Duration::from_millis(20));
                }
                peak
            });
            let start = Instant::now();
            let result = scan(Path::new(&root), &AtomicBool::new(false), |_| {});
            done.store(true, Ordering::Relaxed);
            println!(
                "real scan: {:.3}s, {} dirs, {} files, {} candidates, {} MiB",
                start.elapsed().as_secs_f64(),
                result.visited,
                result.measured_files,
                result.entries.len(),
                result.entries.iter().map(|e| e.bytes).sum::<u64>() >> 20
            );
            sampler.join().unwrap()
        });
        println!("peak process memory: {} MiB", peak >> 20);
    }

    /// Reproducible warm-cache traversal benchmark, excluding fixture creation.
    #[test]
    #[ignore]
    fn benchmark_scan() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target/cleanup-benchmark-data");
        if !root.join("ready").exists() {
            for project in 0..48 {
                let dir = root.join(format!("project-{project}"));
                fs::create_dir_all(dir.join("src")).unwrap();
                fs::write(dir.join("Cargo.toml"), "").unwrap();
                for file in 0..256 {
                    fs::write(dir.join(format!("src/file-{file}.rs")), [0; 128]).unwrap();
                }
                for sub in 0..24 {
                    let dir = dir.join(format!("target/debug/{sub}"));
                    fs::create_dir_all(&dir).unwrap();
                    for file in 0..32 {
                        fs::write(dir.join(format!("artifact-{file}")), [0; 128]).unwrap();
                    }
                }
            }
            fs::write(root.join("ready"), "").unwrap();
        }
        for _ in 0..4 {
            let start = Instant::now();
            let result = scan(&root, &AtomicBool::new(false), |_| {});
            assert_eq!(result.entries.len(), 48);
            assert_eq!(
                result.entries.iter().map(|e| e.bytes).sum::<u64>(),
                48 * 24 * 32 * 128
            );
            println!(
                "scan benchmark: {:.3}s, 48 projects, 49,152 files",
                start.elapsed().as_secs_f64()
            );
        }
    }
    #[test]
    fn recognizes_context_sizes_and_avoids_nested_double_counting() {
        let f = Fixture::new();
        f.file("rust/Cargo.toml", "");
        f.file("rust/target/debug/a", "1234");
        f.file("web/package.json", "{}");
        f.file("web/node_modules/pkg/__pycache__/a.pyc", "123456");
        f.file("source/bin/important.txt", "keep");
        f.file("source/build/important.txt", "keep");
        f.file("dotnet/app.csproj", "");
        f.file("dotnet/obj/a", "12");
        f.file(".git/__pycache__/keep", "keep");
        let result = f.scan();
        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.entries.iter().map(|e| e.bytes).sum::<u64>(), 12);
        assert!(result.entries.iter().all(|e| e.issue.is_none()));
    }
    #[test]
    fn cached_preview_is_revalidated_but_never_deletable() {
        let fixture = Fixture::new();
        let root = fixture.0.canonicalize().unwrap();
        let project = root.join("cached");
        std::fs::create_dir_all(project.join("node_modules")).unwrap();
        std::fs::write(project.join("package.json"), "{}").unwrap();
        std::fs::write(project.join("node_modules/file"), "hello").unwrap();
        let path = project.join("node_modules");
        let preview = |paths: &[&Path]| {
            let mut found = Vec::new();
            let cached = paths
                .iter()
                .map(|p| Cached {
                    path: p.to_path_buf(),
                    bytes: 5,
                    files: 1,
                })
                .collect();
            cached_preview(&root, cached, &AtomicBool::new(false), |c| found.push(c));
            found
        };
        let found = preview(&[&path, &root.join("missing/node_modules")]);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].bytes, found[0].files), (5, 1));
        let report = delete(&root, &found);
        assert_eq!(report.failures.len(), 1);
        assert!(path.join("file").exists());
        std::fs::remove_file(project.join("package.json")).unwrap();
        assert!(preview(&[&path]).is_empty());
    }

    #[test]
    fn large_candidate_is_split_across_workers_with_identical_fingerprint() {
        let f = Fixture::new();
        f.file("app/package.json", "{}");
        for d in 0..40 {
            for n in 0..5 {
                f.file(
                    &format!("app/node_modules/p{d}/lib/{n}.js"),
                    "x".repeat(n).as_str(),
                );
            }
        }
        let serial = scan_with_workers(&f.0, &AtomicBool::new(false), 1, |_| {});
        let parallel = scan_with_workers(&f.0, &AtomicBool::new(false), 8, |_| {});
        let (a, b) = (&serial.entries[0], &parallel.entries[0]);
        assert_eq!(a.files, 200);
        assert_eq!(a.bytes, 40 * 10);
        assert_eq!(a.fingerprint, b.fingerprint);
        let mut listing = Vec::new();
        let strict = inventory(&a.path, &HashSet::new(), &mut listing).unwrap();
        assert_eq!(Some(strict.fingerprint), a.fingerprint);
        assert_eq!(listing.len() as u64, strict.fingerprint.entries);
    }

    #[test]
    fn deletes_only_selected_and_rejects_changed_contents_and_wrong_root() {
        let f = Fixture::new();
        f.file("a/__pycache__/a.pyc", "123");
        f.file("b/__pycache__/b.pyc", "456");
        let result = f.scan();
        let chosen = result
            .entries
            .iter()
            .find(|e| e.path.ends_with("a/__pycache__"))
            .unwrap();
        assert_eq!(
            delete(&result.root.join("elsewhere"), std::slice::from_ref(chosen))
                .failures
                .len(),
            1
        );
        f.file("a/__pycache__/new.pyc", "new");
        assert_eq!(
            delete(&result.root, std::slice::from_ref(chosen))
                .failures
                .len(),
            1
        );
        assert!(f.0.join("a/__pycache__/a.pyc").exists());
        let result = f.scan();
        let chosen = result
            .entries
            .iter()
            .find(|e| e.path.ends_with("a/__pycache__"))
            .unwrap();
        let report = delete(&result.root, std::slice::from_ref(chosen));
        assert!(report.failures.is_empty());
        assert_eq!(report.bytes, 6);
        assert!(!chosen.path.exists());
        assert!(f.0.join("b/__pycache__/b.pyc").exists());
    }
    #[test]
    fn cancellation_and_missing_root_are_reported() {
        let f = Fixture::new();
        assert!(scan(&f.0, &AtomicBool::new(true), |_| {}).cancelled);
        assert_eq!(
            scan(&f.0.join("missing"), &AtomicBool::new(false), |_| {})
                .warnings
                .len(),
            1
        );
    }
    #[test]
    fn dev_folders_are_scanned_and_nested_mounts_are_blocked() {
        let f = Fixture::new();
        f.file("dev/project/__pycache__/a.pyc", "123");
        let result = f.scan();
        assert_eq!(result.entries.len(), 1);
        let candidate = &result.entries[0];
        let mounts = HashSet::from([candidate.path.clone()]);
        assert!(delete_one(&result.root, candidate, &mounts).is_err());
        assert!(candidate.path.join("a.pyc").exists());
    }
    #[test]
    fn mounts_between_root_and_candidate_are_blocked() {
        let f = Fixture::new();
        f.file("tmp/project/__pycache__/a.pyc", "123");
        let result = f.scan();
        assert_eq!(result.entries.len(), 1);
        let candidate = &result.entries[0];
        // An intermediate mount the scan did not know about (e.g. tmpfs at /tmp).
        let mounts = HashSet::from([result.root.join("tmp")]);
        assert!(delete_one(&result.root, candidate, &mounts).is_err());
        assert!(candidate.path.join("a.pyc").exists());
        assert!(check_boundary(&result.root, &candidate.path, &HashSet::new()).is_ok());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mount_table_includes_virtual_and_network_filesystems() {
        let mountinfo = b"22 1 8:1 / / rw - ext4 /dev/sda1 rw\n\
            30 22 0:30 / /tmp rw - tmpfs tmpfs rw\n\
            31 22 0:31 / /mnt/my\\040share rw - cifs //host/share rw\n\
            32 22 0:32 / /srv/nfs rw - nfs4 host:/export rw\n";
        let points = linux_mount_points(mountinfo, 4);
        for expected in ["/", "/tmp", "/mnt/my share", "/srv/nfs"] {
            assert!(points.contains(&PathBuf::from(expected)), "{expected}");
        }
        let mounts = b"tmpfs /tmp tmpfs rw 0 0\n";
        assert_eq!(linux_mount_points(mounts, 1), [PathBuf::from("/tmp")]);
        // The live table must include anything the kernel reports, e.g. /proc.
        assert!(mount_points().contains(Path::new("/proc")));
    }
    #[test]
    fn parallel_scan_matches_serial_and_streams_each_candidate_once() {
        let f = Fixture::new();
        for n in 0..24 {
            f.file(&format!("p{n}/package.json"), "{}");
            f.file(&format!("p{n}/node_modules/pkg/lib/a"), "1234567");
            f.file(&format!("p{n}/node_modules/pkg/__pycache__/a"), "12");
            f.file(&format!("p{n}/source/keep"), "keep");
        }
        let serial = scan_with_workers(&f.0, &AtomicBool::new(false), 1, |_| {});
        let mut streamed = HashSet::new();
        let parallel = scan_with_workers(&f.0, &AtomicBool::new(false), 4, |update| {
            if let ScanUpdate::Found(candidate) = update {
                assert!(streamed.insert(candidate.path));
            }
        });
        assert_eq!(serial.entries.len(), 24);
        assert_eq!(streamed.len(), 24);
        assert_eq!(serial.visited, parallel.visited);
        assert_eq!(parallel.measured_files, 48);
        for (a, b) in serial.entries.iter().zip(&parallel.entries) {
            assert_eq!(a.path, b.path);
            assert_eq!(a.bytes, 9);
            assert_eq!(a.bytes, b.bytes);
            assert_eq!(a.files, b.files);
            assert_eq!(a.fingerprint, b.fingerprint);
            assert!(b.issue.is_none());
        }
    }
    #[test]
    fn cancellation_during_work_returns_only_completed_candidates() {
        let f = Fixture::new();
        for n in 0..24 {
            f.file(&format!("p{n}/__pycache__/a"), "123");
        }
        let cancel = AtomicBool::new(false);
        let result = scan_with_workers(&f.0, &cancel, 1, |update| {
            if matches!(update, ScanUpdate::Found(_)) {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(result.cancelled);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].bytes, 3);
        assert!(result.entries[0].issue.is_none());
    }
    #[cfg(windows)]
    #[test]
    fn rejects_windows_junctions_in_contents_and_replaced_ancestors() {
        fn junction(target: &Path, link: &Path) {
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link.to_string_lossy().replace('/', "\\"))
                .arg(target.to_string_lossy().replace('/', "\\"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let f = Fixture::new();
        f.file("outside/keep", "keep");
        f.file("project/__pycache__/a.pyc", "123");
        junction(&f.0.join("outside"), &f.0.join("project/__pycache__/link"));
        let result = f.scan();
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].issue.is_some());
        assert_eq!(delete(&result.root, &result.entries).failures.len(), 1);
        fs::remove_dir(f.0.join("project/__pycache__/link")).unwrap();
        let result = f.scan();
        fs::rename(f.0.join("project"), f.0.join("moved")).unwrap();
        junction(&f.0.join("moved"), &f.0.join("project"));
        assert_eq!(delete(&result.root, &result.entries).failures.len(), 1);
        assert!(f.0.join("moved/__pycache__/a.pyc").exists());
        assert!(f.0.join("outside/keep").exists());
    }
    #[cfg(unix)]
    #[test]
    fn skips_links_and_blocks_linked_candidate_contents() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        f.file("source/keep", "keep");
        f.file("__pycache__/a", "a");
        symlink(f.0.join("source"), f.0.join("__pycache__/link")).unwrap();
        symlink(&f.0, f.0.join("loop")).unwrap();
        let result = f.scan();
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].issue.is_some());
        assert_eq!(delete(&result.root, &result.entries).failures.len(), 1);
        assert!(f.0.join("source/keep").exists());
    }
}
