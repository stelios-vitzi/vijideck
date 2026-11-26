use super::capture;
#[cfg(windows)]
use std::path::PathBuf;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

#[derive(Clone, Copy)]
pub(super) enum Package {
    #[cfg(windows)]
    WindowsSetup { all_users: bool },
    #[cfg(windows)]
    WindowsPortable,
    #[cfg(target_os = "macos")]
    MacImage,
    #[cfg(target_os = "linux")]
    Debian,
    #[cfg(target_os = "linux")]
    LinuxPortable,
}

impl Package {
    pub(super) fn asset(self, version: &str) -> String {
        let suffix = match self {
            #[cfg(windows)]
            Self::WindowsSetup { .. } => "-setup.exe",
            #[cfg(windows)]
            Self::WindowsPortable => ".zip",
            #[cfg(target_os = "macos")]
            Self::MacImage => ".dmg",
            #[cfg(target_os = "linux")]
            Self::Debian => ".deb",
            #[cfg(target_os = "linux")]
            Self::LinuxPortable => ".tar.gz",
        };
        format!(
            "vijideck-{version}-{}-{}{suffix}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    }
}

pub(super) fn quiet(cmd: &mut Command) {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
}

#[cfg(windows)]
fn system_tool(name: &str) -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join(name)
}

#[cfg(windows)]
fn powershell(script: &str) -> Command {
    let mut cmd = Command::new(system_tool(r"WindowsPowerShell\v1.0\powershell.exe"));
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    cmd
}

#[cfg(windows)]
pub(super) fn package(target: &Path) -> Result<Package, String> {
    let folder = target
        .parent()
        .ok_or("VijiDeck's install folder is unavailable.")?;
    if target
        .file_name()
        .is_none_or(|name| !name.eq_ignore_ascii_case("vijideck.exe"))
    {
        return Ok(Package::WindowsPortable);
    }
    let mut cmd = powershell(
        r#"$ErrorActionPreference='Stop'
        $key = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{B975A203-0375-4E63-9A96-A82D276BC0F2}_is1'
        foreach ($hive in @('HKCU:', 'HKLM:')) {
            $path = Join-Path $hive $key
            if (Test-Path -LiteralPath $path) {
                $entry = Get-ItemProperty -LiteralPath $path
                $folder = $entry.'Inno Setup: App Path'
                if (!$folder) { $folder = $entry.InstallLocation }
                if ($folder -and ($folder.TrimEnd('\') -ieq $env:VIJIDECK_UPDATE_FOLDER.TrimEnd('\'))) {
                    $hive
                    exit
                }
            }
        }"#,
    );
    cmd.env("VIJIDECK_UPDATE_FOLDER", folder);
    let output = capture(cmd, "Find VijiDeck's install scope", 15, 4096)?;
    Ok(match String::from_utf8_lossy(&output).trim() {
        "HKLM:" => Package::WindowsSetup { all_users: true },
        "HKCU:" => Package::WindowsSetup { all_users: false },
        _ => Package::WindowsPortable,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn package(target: &Path) -> Result<Package, String> {
    if target == Path::new("/usr/bin/vijideck") && Path::new("/usr/bin/dpkg-query").is_file() {
        let mut cmd = Command::new("/usr/bin/dpkg-query");
        cmd.args(["-S", "/usr/bin/vijideck"]);
        if capture(cmd, "Find VijiDeck's package", 10, 8192)
            .is_ok_and(|bytes| bytes.starts_with(b"vijideck: /usr/bin/vijideck"))
        {
            return Ok(Package::Debian);
        }
    }
    Ok(Package::LinuxPortable)
}

#[cfg(target_os = "macos")]
fn bundle(target: &Path) -> Result<&Path, String> {
    let macos = target.parent().filter(|path| path.ends_with("MacOS"));
    let contents = macos
        .and_then(Path::parent)
        .filter(|path| path.ends_with("Contents"));
    contents
        .and_then(Path::parent)
        .filter(|path| path.extension().is_some_and(|value| value == "app"))
        .filter(|_| target.file_name().is_some_and(|name| name == "vijideck"))
        .ok_or_else(|| "Install VijiDeck.app from its disk image to use in-app updates.".into())
}

#[cfg(target_os = "macos")]
pub(super) fn package(target: &Path) -> Result<Package, String> {
    bundle(target).map(|_| Package::MacImage)
}

#[cfg(windows)]
pub(super) fn wait_for_exit(pid: u32) -> Result<(), String> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{ERROR_INVALID_PARAMETER, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Storage::FileSystem::SYNCHRONIZE,
        System::Threading::{OpenProcess, WaitForSingleObject},
    };
    // SAFETY: OpenProcess accepts a numeric PID and returns a uniquely owned handle.
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            return Ok(());
        }
        return Err(format!("Could not wait for VijiDeck to close: {error}"));
    }
    // SAFETY: OpenProcess succeeded, and the handle remains owned through the wait.
    let _owned = unsafe { OwnedHandle::from_raw_handle(handle) };
    match unsafe { WaitForSingleObject(handle, 30_000) } {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err("VijiDeck took too long to close.".into()),
        _ => Err(format!(
            "Could not wait for VijiDeck to close: {}",
            std::io::Error::last_os_error()
        )),
    }
}

#[cfg(unix)]
pub(super) fn wait_for_exit(pid: u32) -> Result<(), String> {
    use std::time::{Duration, Instant};
    let pid = i32::try_from(pid).map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        // SAFETY: Signal zero only queries the existence of a positive PID.
        if unsafe { libc::kill(pid, 0) } == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(format!("Could not wait for VijiDeck to close: {error}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("VijiDeck took too long to close.".into())
}

pub(super) fn install(
    package: Package,
    archive: &Path,
    target: &Path,
    work: &Path,
    _version: &str,
) -> Result<(), String> {
    match package {
        #[cfg(windows)]
        Package::WindowsSetup { all_users } => {
            let folder = target
                .parent()
                .ok_or("VijiDeck's install folder is unavailable.")?;
            let mut dir = std::ffi::OsString::from("/DIR=");
            dir.push(folder);
            let mut cmd = Command::new(archive);
            cmd.args([
                "/SILENT",
                "/SP-",
                "/SUPPRESSMSGBOXES",
                "/NOCLOSEAPPLICATIONS",
                "/NORESTARTAPPLICATIONS",
                "/NORESTART",
                "/RESTARTEXITCODE=8",
                if all_users {
                    "/ALLUSERS"
                } else {
                    "/CURRENTUSER"
                },
            ])
            .arg(dir);
            run_installer(cmd)
        }
        #[cfg(windows)]
        Package::WindowsPortable => extract_binary(
            &system_tool("tar.exe"),
            archive,
            work,
            "vijideck.exe",
            target,
        ),
        #[cfg(target_os = "linux")]
        Package::Debian => {
            let mut cmd = Command::new("/usr/bin/pkexec");
            cmd.args(["/usr/bin/dpkg", "--install"]).arg(archive);
            run_installer(cmd)
        }
        #[cfg(target_os = "linux")]
        Package::LinuxPortable => extract_binary(
            Path::new("/usr/bin/tar"),
            archive,
            work,
            &format!("vijideck-{_version}/vijideck"),
            target,
        ),
        #[cfg(target_os = "macos")]
        Package::MacImage => install_image(archive, target, work),
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn run_installer(mut cmd: Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|error| format!("Could not start the installer: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "The installer did not finish successfully ({status})."
        ))
    }
}

fn sibling_stage(target: &Path, work: &Path) -> Result<super::WorkDir, String> {
    let parent = target
        .parent()
        .ok_or("VijiDeck's install folder is unavailable.")?;
    let id = work
        .file_name()
        .ok_or("The update staging directory is unavailable.")?;
    let mut name = std::ffi::OsString::from(".vijideck-update-");
    name.push(id);
    let path = parent.join(name);
    fs::create_dir(&path)
        .map_err(|error| format!("Could not write to VijiDeck's install folder: {error}"))?;
    Ok(super::WorkDir(path))
}

#[cfg(any(windows, target_os = "linux"))]
fn extract_binary(
    tar: &Path,
    archive: &Path,
    work: &Path,
    member: &str,
    target: &Path,
) -> Result<(), String> {
    let stage = sibling_stage(target, work)?;
    let mut cmd = Command::new(tar);
    cmd.arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(&stage.0)
        .arg("--")
        .arg(member);
    capture(cmd, "Extract the updated executable", 60, 8192)?;
    let executable = stage.0.join(member);
    let metadata = fs::symlink_metadata(&executable).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("The update archive has no usable executable.".into());
    }
    fs::rename(executable, target).map_err(|error| format!("Could not replace VijiDeck: {error}"))
}

#[cfg(target_os = "macos")]
fn install_image(archive: &Path, target: &Path, work: &Path) -> Result<(), String> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let installed = bundle(target)?;
    let stage = sibling_stage(installed, work)?;
    let mount = work.join("mount");
    fs::create_dir(&mount).map_err(|error| error.to_string())?;
    let mut attach = Command::new("/usr/bin/hdiutil");
    attach
        .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
        .arg(&mount)
        .arg(archive);
    capture(attach, "Mount the update disk image", 60, 8192)?;
    let replacement = stage.0.join("VijiDeck.app");
    let mut copy = Command::new("/usr/bin/ditto");
    copy.arg(mount.join("VijiDeck.app")).arg(&replacement);
    let copied = capture(copy, "Copy the updated application bundle", 60, 8192);
    let mut detach = Command::new("/usr/bin/hdiutil");
    detach.arg("detach").arg(&mount);
    let detached = capture(detach, "Unmount the update disk image", 30, 8192);
    copied?;
    detached?;
    let mut codesign = Command::new("/usr/bin/codesign");
    codesign.args(["--verify", "--strict"]).arg(&replacement);
    capture(
        codesign,
        "Validate the updated application bundle",
        30,
        8192,
    )?;
    if !replacement.join("Contents/MacOS/vijideck").is_file() {
        return Err("The update disk image has no usable application bundle.".into());
    }
    let source =
        CString::new(replacement.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    let destination =
        CString::new(installed.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    // SAFETY: The paths are absolute NUL-terminated strings valid for the duration of the call.
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "Could not replace VijiDeck: {}",
            std::io::Error::last_os_error()
        ))
    }
}
