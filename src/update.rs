mod platform;

use crate::command::{self, Limits};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases");
const REPOSITORY: &str = "https://api.github.com/repos/SteliosDaVitzi/vijideck";
const MAX_PACKAGE: u64 = 256 * 1024 * 1024;
const HELPER: &str = if cfg!(windows) {
    "update-helper.exe"
} else {
    "update-helper"
};

#[derive(Clone)]
pub struct Release {
    pub version: String,
    asset: String,
    size: u64,
}

#[cfg(feature = "ui-preview")]
impl Release {
    pub fn preview() -> Self {
        Self {
            version: "0.4.0".into(),
            asset: String::new(),
            size: 18 * 1024 * 1024,
        }
    }
}

pub struct Staged {
    directory: WorkDir,
    release: Release,
    checksum: String,
    executable: PathBuf,
}

struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Result<Self, String> {
        let root = update_root()?;
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = root.join(format!("{}-{nonce}", std::process::id()));
        let mut builder = fs::DirBuilder::new();
        builder.recursive(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        _ = fs::remove_dir_all(&self.0);
    }
}

fn update_root() -> Result<PathBuf, String> {
    crate::storage::data_dir()
        .filter(|path| path.is_absolute())
        .map(|path| path.join("updates"))
        .ok_or_else(|| "The user data directory is unavailable.".into())
}

fn version_number(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split('.');
    let mut parse = || {
        let part = parts.next()?;
        if part.is_empty()
            || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        part.parse().ok()
    };
    let result = [parse()?, parse()?, parse()?];
    parts.next().is_none().then_some(result)
}

fn newer(version: &str, current: &str) -> bool {
    let current = current.split('+').next().unwrap_or(current);
    let (base, prerelease) = current.split_once('-').unwrap_or((current, ""));
    matches!((version_number(version), version_number(base)),
        (Some(next), Some(installed)) if next > installed || (next == installed && !prerelease.is_empty()))
}

fn fetch(url: &str, timeout: u64, accept: &str) -> Result<ureq::Body, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(5)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(timeout)))
        .user_agent(concat!("VijiDeck/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    agent
        .get(url)
        .header("Accept", accept)
        .call()
        .map(|response| response.into_body())
        .map_err(|error| error.to_string())
}

fn capture(cmd: Command, operation: &str, seconds: u64, bytes: usize) -> Result<Vec<u8>, String> {
    let output = command::capture(
        cmd,
        Limits {
            timeout: Duration::from_secs(seconds),
            output_bytes: bytes,
        },
    )
    .map_err(|error| format!("{operation}: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let message = detail.trim().chars().take(350).collect::<String>();
        return Err(format!("{operation} failed ({}). {message}", output.status));
    }
    Ok(output.stdout)
}

pub fn check() -> Result<Option<Release>, String> {
    let bytes = fetch(
        &format!("{REPOSITORY}/releases/latest"),
        20,
        "application/vnd.github+json",
    )?
    .into_with_config()
    .limit(512 * 1024)
    .read_to_vec()
    .map_err(|error| error.to_string())?;
    let json: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not read release information: {error}"))?;
    let Some(version) = release_version(&json)? else {
        return Ok(None);
    };
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let package = platform::package(&executable)?;
    let asset = package.asset(version);
    let size = asset_size(&json, &asset)
        .ok_or_else(|| format!("VijiDeck {version} has no compatible update package."))?;
    asset_size(&json, "SHA256SUMS.txt")
        .filter(|size| *size <= 64 * 1024)
        .ok_or("The release has no checksum file.")?;
    Ok(Some(Release {
        version: version.to_owned(),
        asset,
        size,
    }))
}

fn release_version(json: &Value) -> Result<Option<&str>, String> {
    if json["draft"].as_bool() == Some(true) || json["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let version = json["tag_name"]
        .as_str()
        .and_then(|tag| tag.strip_prefix('v'))
        .filter(|version| version_number(version).is_some())
        .ok_or("The release has an invalid version.")?;
    Ok(newer(version, VERSION).then_some(version))
}

fn asset_size(json: &Value, filename: &str) -> Option<u64> {
    json["assets"]
        .as_array()?
        .iter()
        .find(|asset| asset["name"].as_str() == Some(filename))
        .and_then(|asset| asset["size"].as_u64())
        .filter(|size| (1..=MAX_PACKAGE).contains(size))
}

fn checksum_for(manifest: &str, asset: &str) -> Option<String> {
    manifest.lines().find_map(|line| {
        let (sum, name) = line.split_once(char::is_whitespace)?;
        (name.trim_start().trim_start_matches('*') == asset && valid_checksum(sum))
            .then(|| sum.to_ascii_lowercase())
    })
}

fn valid_checksum(sum: &str) -> bool {
    sum.len() == 64 && sum.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn download(release: Release) -> Result<Staged, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let base = format!("{RELEASES}/download/v{}", release.version);
    let manifest = fetch(&format!("{base}/SHA256SUMS.txt"), 20, "*/*")?
        .into_with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|error| error.to_string())?;
    let checksum = checksum_for(&manifest, &release.asset)
        .ok_or("The update package has no SHA-256 checksum.")?;
    let directory = WorkDir::new()?;
    let path = directory.0.join(&release.asset);
    let body = fetch(&format!("{base}/{}", release.asset), 300, "*/*")?;
    let mut content = body.into_reader().take(release.size + 1);
    let mut file = fs::File::create_new(&path).map_err(|error| error.to_string())?;
    let size = io::copy(&mut content, &mut file).map_err(|error| error.to_string())?;
    if size != release.size {
        return Err("The download size does not match the published package.".into());
    }
    drop(file);
    verify(&path, &checksum)?;
    Ok(Staged {
        directory,
        release,
        checksum,
        executable,
    })
}

fn verify(path: &Path, expected: &str) -> Result<(), String> {
    if sha256(path)? == expected {
        Ok(())
    } else {
        Err("The update package failed SHA-256 verification.".into())
    }
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    io::copy(&mut file, &mut hasher).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}

impl Staged {
    pub fn version(&self) -> &str {
        &self.release.version
    }

    pub fn restart(self) -> Result<(), String> {
        let helper = self.directory.0.join(HELPER);
        fs::copy(&self.executable, &helper)
            .map_err(|error| format!("Could not prepare the update helper: {error}"))?;
        let mut cmd = Command::new(helper);
        cmd.arg("--vijideck-apply-update")
            .arg(std::process::id().to_string())
            .arg(&self.executable)
            .arg(&self.release.version)
            .arg(&self.checksum);
        platform::quiet(&mut cmd);
        cmd.spawn()
            .map_err(|error| format!("Could not start the update helper: {error}"))?;
        // The helper owns the directory until it relaunches VijiDeck.
        std::mem::forget(self.directory);
        Ok(())
    }
}

pub fn run_helper(args: &[OsString]) {
    let [pid, target, version, checksum] = args else {
        return;
    };
    let (Some(pid), Some(version), Some(checksum)) = (
        pid.to_str().and_then(|pid| pid.parse::<u32>().ok()),
        version
            .to_str()
            .filter(|version| version_number(version).is_some()),
        checksum.to_str().filter(|sum| valid_checksum(sum)),
    ) else {
        return;
    };
    let Ok(helper) = std::env::current_exe() else {
        return;
    };
    let Ok(root) = update_root() else {
        return;
    };
    let Some(directory) = helper.parent().filter(|path| path.parent() == Some(&root)) else {
        return;
    };
    let Some(id) = directory.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    if helper.file_name().is_none_or(|name| name != HELPER) || !valid_work_id(id) {
        return;
    }
    let target = Path::new(target);
    if !target.is_absolute() || target.starts_with(&root) || pid == 0 {
        return;
    }
    let result = platform::wait_for_exit(pid).and_then(|()| {
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("install.lock"))
            .map_err(|error| error.to_string())?;
        lock.try_lock()
            .map_err(|error| format!("Another update is being installed: {error}"))?;
        if !newer(version, VERSION) || sha256(target)? != sha256(&helper)? {
            return Err("VijiDeck changed since the update was downloaded.".into());
        }
        let package = platform::package(target)?;
        let archive = directory.join(package.asset(version));
        verify(&archive, checksum)?;
        platform::install(package, &archive, target, directory, version)
    });
    let mut app = Command::new(target);
    app.arg("--vijideck-update-result")
        .arg(id)
        .arg(result.err().unwrap_or_default());
    // Preserve the app's normal window behavior when relaunched.
    app.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Err(error) = app.spawn() {
        eprintln!("Could not reopen VijiDeck after updating: {error}");
    }
}

fn valid_work_id(id: &str) -> bool {
    id.split_once('-')
        .is_some_and(|(pid, nonce)| pid.parse::<u32>().is_ok() && nonce.parse::<u128>().is_ok())
}

pub fn finish(id: &str) -> Result<(), String> {
    if !valid_work_id(id) {
        return Ok(());
    }
    let path = update_root()?.join(id);
    // Windows holds the helper executable open until the previous process exits.
    for _ in 0..25 {
        match fs::remove_dir_all(&path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    Err("Could not remove the downloaded update files.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_stable_versions_numerically_without_downgrading() {
        assert!(newer("1.10.0", "1.9.2"));
        assert!(newer("1.10.0", "1.10.0-rc.1"));
        assert!(!newer("1.10.0", "1.10.0"));
        assert!(!newer("1.2.3", "1.2.4"));
        assert!(!newer("1.2.3-rc.1", "1.2.2"));
        assert!(!newer("1.02.3", "1.2.2"));
        assert!(!newer("1.2.3/../", "1.2.2"));
        assert_eq!(release_version(&json!({"draft": true})), Ok(None));
        assert_eq!(release_version(&json!({"prerelease": true})), Ok(None));
    }

    #[test]
    fn selects_exact_asset_and_checksum() {
        let asset = "vijideck-1.2.3-linux-x86_64.deb";
        let sum = "A".repeat(64);
        let manifest = format!("invalid  {asset}\n{sum}  {asset}.old\n{sum} *{asset}\n");
        assert_eq!(checksum_for(&manifest, asset), Some(sum.to_lowercase()));
        assert_eq!(checksum_for(&manifest, "other.deb"), None);
        let json = json!({"assets": [{"name": asset, "size": 1024}]});
        assert_eq!(asset_size(&json, asset), Some(1024));
        assert_eq!(asset_size(&json, "other.deb"), None);
        assert_eq!(
            asset_size(
                &json!({"assets": [{"name": asset, "size": MAX_PACKAGE + 1}]}),
                asset
            ),
            None
        );
    }
}
