//! CLI upgrade effects: channel detection, upgrade state, the background worker,
//! and the verified in-place replacement of a standalone install.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use skilld_command::upgrade::{
    GITHUB_LATEST_RELEASE_URL, InstallChannel, MAX_BINARY_BYTES, NPM_LATEST_URL, PackageRunner,
    RELEASE_MANIFEST_ASSET, RELEASE_SIGNATURE_ASSET, UpgradeNotice, UpgradeState, UpgradeWorker,
    is_newer, parse_github_release, parse_npm_latest, plan_upgrade, release_asset,
    release_asset_url,
};
use skilld_core::{ReleasePin, RemoteError, verify_release_asset, verify_release_manifest};

/// The environment variable that starts the background worker instead of a command.
pub const WORKER_VARIABLE: &str = "SKILLD_UPGRADE_WORKER";
/// The npm loader sets this to the package runner that launched skilld.
pub const LAUNCHER_VARIABLE: &str = "SKILLD_LAUNCHER";
/// The install script writes this file beside a standalone executable.
pub const INSTALL_MARKER: &str = "skilld-install.json";
const STATE_FILE: &str = "upgrade.json";
const LOCK_FILE: &str = "upgrade.lock";
const STALE_LOCK: Duration = Duration::from_secs(10 * 60);
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_SIGNATURE_BYTES: usize = 1024;
const MAX_METADATA_BYTES: usize = 256 * 1024;

type VersionParser = fn(&[u8]) -> Option<String>;

/// Downloads one HTTPS resource with a byte limit.
pub trait ReleaseFetcher {
    fn get(&self, url: &str, limit: usize) -> Result<Vec<u8>, RemoteError>;
}

/// The standalone executable an upgrade replaces, and the key its release must carry.
pub struct InstallTarget<'a> {
    pub executable: &'a Path,
    pub current_version: &'a str,
    pub asset: &'a str,
    pub pin: &'a ReleasePin,
}

/// Reads the install channel from the loader variable and the install marker.
pub fn install_channel(executable: &Path, launcher: Option<OsString>) -> InstallChannel {
    if let Some(runner) = launcher
        .as_deref()
        .and_then(|value| value.to_str())
        .and_then(PackageRunner::parse)
    {
        return InstallChannel::Npm(runner);
    }
    let marker = executable.with_file_name(INSTALL_MARKER);
    match fs::symlink_metadata(marker) {
        Ok(metadata) if metadata.is_file() => InstallChannel::Standalone,
        _ => InstallChannel::Unmanaged,
    }
}

/// The asset name for the running executable's target.
pub fn current_release_asset() -> Option<&'static str> {
    let env = if cfg!(target_env = "musl") {
        "musl"
    } else if cfg!(target_env = "gnu") {
        "gnu"
    } else if cfg!(target_env = "msvc") {
        "msvc"
    } else {
        ""
    };
    release_asset(std::env::consts::OS, std::env::consts::ARCH, env)
}

/// Plans the upgrade for one command, records it, and starts the worker.
///
/// Returns the notice to print after the command. Upgrade problems never change
/// the command result, so this reports nothing when state or spawning fails.
pub fn before_command(
    data_root: &Path,
    executable: &Path,
    channel: InstallChannel,
    current_version: &str,
    now: u64,
) -> Option<UpgradeNotice> {
    let mut state = read_state(data_root);
    let plan = plan_upgrade(current_version, &channel, &state, now);
    if let Some(worker) = &plan.worker {
        match worker {
            UpgradeWorker::Check => state.checked_at = now,
            UpgradeWorker::Install { version } => {
                state.attempted_version = Some(version.clone());
                state.attempted_at = Some(now);
            }
        }
        // Recording first stops concurrent commands from starting duplicate workers.
        write_state(data_root, &state).ok()?;
        spawn_worker(executable, worker).ok()?;
    }
    plan.notice
}

/// Runs the background worker named by `WORKER_VARIABLE`.
///
/// `target` is `None` unless this is a standalone install with a compiled release key.
pub fn run_worker(
    value: &str,
    data_root: &Path,
    channel: InstallChannel,
    fetcher: &dyn ReleaseFetcher,
    target: Option<InstallTarget<'_>>,
    now: u64,
) {
    let Some(_lock) = WorkerLock::acquire(data_root) else {
        return;
    };
    let mut state = read_state(data_root);
    let result = if value == "check" {
        state.checked_at = now;
        let (url, parse): (_, VersionParser) = match channel {
            InstallChannel::Npm(_) => (NPM_LATEST_URL, parse_npm_latest),
            _ => (GITHUB_LATEST_RELEASE_URL, parse_github_release),
        };
        fetcher.get(url, MAX_METADATA_BYTES).and_then(|body| {
            let latest = parse(&body).ok_or_else(|| {
                RemoteError::new("UPGRADE_CHECK_INVALID", "the latest version is invalid")
            })?;
            state.latest = Some(latest);
            Ok(())
        })
    } else if let Some(version) = value.strip_prefix("install:") {
        state.attempted_version = Some(version.to_owned());
        state.attempted_at = Some(now);
        match target {
            Some(target) => install_release(&target, fetcher, version),
            None => Err(RemoteError::new(
                "UPGRADE_UNAVAILABLE",
                "only a standalone install with a release key upgrades itself",
            )),
        }
    } else {
        return;
    };
    state.last_error = result.err().map(|error| error.code.to_owned());
    // The worker has no terminal. A failed write only repeats the work later.
    let _ = write_state(data_root, &state);
}

/// Downloads, verifies, and installs one release over the running executable.
///
/// Nothing is written beside the executable until the signed manifest and the
/// binary digest both verify. The new binary runs only after verification.
pub fn install_release(
    target: &InstallTarget<'_>,
    fetcher: &dyn ReleaseFetcher,
    version: &str,
) -> Result<(), RemoteError> {
    if !is_newer(version, target.current_version) {
        return Err(RemoteError::new(
            "UPGRADE_NOT_NEWER",
            "skilld only upgrades to a newer version",
        ));
    }
    let url = |asset: &str| {
        release_asset_url(version, asset)
            .ok_or_else(|| RemoteError::new("UPGRADE_CHECK_INVALID", "the version is invalid"))
    };
    let manifest = fetcher.get(&url(RELEASE_MANIFEST_ASSET)?, MAX_MANIFEST_BYTES)?;
    let signature = fetcher.get(&url(RELEASE_SIGNATURE_ASSET)?, MAX_SIGNATURE_BYTES)?;
    let signature = String::from_utf8(signature).map_err(|_| {
        RemoteError::new(
            "RELEASE_SIGNATURE_INVALID",
            "the release manifest signature is invalid",
        )
    })?;
    let release = verify_release_manifest(&manifest, &signature, target.pin, version)?;
    let binary = fetcher.get(&url(target.asset)?, MAX_BINARY_BYTES)?;
    verify_release_asset(&release, target.asset, &binary)?;
    replace_executable(target.executable, &binary, version)
}

fn replace_executable(executable: &Path, binary: &[u8], version: &str) -> Result<(), RemoteError> {
    let staged = executable.with_file_name(format!(".skilld-upgrade-{}", std::process::id()));
    let result = stage(&staged, binary)
        .and_then(|()| check_version(&staged, version))
        .and_then(|()| swap(&staged, executable));
    if result.is_err() {
        // The staged file is ours; a failed removal leaves only an inert file.
        let _ = fs::remove_file(&staged);
    }
    result
}

fn stage(path: &Path, binary: &[u8]) -> Result<(), RemoteError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o755);
    }
    let mut file = options.open(path).map_err(upgrade_io)?;
    file.write_all(binary).map_err(upgrade_io)?;
    file.sync_all().map_err(upgrade_io)
}

fn check_version(path: &Path, version: &str) -> Result<(), RemoteError> {
    let output = Command::new(path)
        .arg("--version")
        .env("SKILLD_NO_UPGRADE", "1")
        .env_remove(WORKER_VARIABLE)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(upgrade_io)?;
    if !output.status.success() || output.stdout != format!("skilld {version}\n").as_bytes() {
        return Err(RemoteError::new(
            "UPGRADE_VERSION_MISMATCH",
            "the downloaded skilld reports another version",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn swap(staged: &Path, executable: &Path) -> Result<(), RemoteError> {
    fs::rename(staged, executable).map_err(upgrade_io)
}

#[cfg(windows)]
fn swap(staged: &Path, executable: &Path) -> Result<(), RemoteError> {
    // Windows cannot replace a running executable, but it can rename one.
    let previous = previous_executable(executable);
    let _ = fs::remove_file(&previous);
    fs::rename(executable, &previous).map_err(upgrade_io)?;
    fs::rename(staged, executable).or_else(|error| {
        let _ = fs::rename(&previous, executable);
        Err(upgrade_io(error))
    })
}

/// The path a Windows upgrade moves the replaced executable to.
pub fn previous_executable(executable: &Path) -> PathBuf {
    let mut name = executable.file_name().unwrap_or_default().to_owned();
    name.push(".old");
    executable.with_file_name(name)
}

fn spawn_worker(executable: &Path, worker: &UpgradeWorker) -> io::Result<()> {
    let value = match worker {
        UpgradeWorker::Check => "check".to_owned(),
        UpgradeWorker::Install { version } => format!("install:{version}"),
    };
    let mut command = Command::new(executable);
    command
        .env(WORKER_VARIABLE, value)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A new process group keeps Ctrl+C in the terminal from stopping the worker.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command.spawn().map(drop)
}

pub fn read_state(data_root: &Path) -> UpgradeState {
    fs::read(data_root.join(STATE_FILE))
        .map(|bytes| UpgradeState::parse(&bytes))
        .unwrap_or_default()
}

fn write_state(data_root: &Path, state: &UpgradeState) -> io::Result<()> {
    fs::create_dir_all(data_root)?;
    let bytes = serde_json::to_vec(state).map_err(io::Error::other)?;
    let mut file = tempfile::NamedTempFile::new_in(data_root)?;
    file.write_all(&bytes)?;
    file.persist(data_root.join(STATE_FILE))
        .map(drop)
        .map_err(|error| error.error)
}

struct WorkerLock(PathBuf);

impl WorkerLock {
    fn acquire(data_root: &Path) -> Option<Self> {
        fs::create_dir_all(data_root).ok()?;
        let path = data_root.join(LOCK_FILE);
        let stale = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > STALE_LOCK);
        if stale {
            // A crashed worker left this lock; removing it lets upgrades resume.
            let _ = fs::remove_file(&path);
        }
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()
            .map(|_| Self(path))
    }
}

impl Drop for WorkerLock {
    fn drop(&mut self) {
        // A lock left behind expires after STALE_LOCK.
        let _ = fs::remove_file(&self.0);
    }
}

/// The upgrade HTTP client: HTTPS only, bounded redirects, bounded bodies.
pub struct NativeReleaseFetcher {
    agent: ureq::Agent,
}

impl NativeReleaseFetcher {
    pub fn new(version: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(5)
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .user_agent(format!("skilld/{version}"))
            .build()
            .into();
        Self { agent }
    }
}

impl ReleaseFetcher for NativeReleaseFetcher {
    fn get(&self, url: &str, limit: usize) -> Result<Vec<u8>, RemoteError> {
        let mut response = self
            .agent
            .get(url)
            .header(
                "Accept",
                "application/vnd.github+json, application/json, */*",
            )
            .call()
            .map_err(|_| RemoteError::new("UPGRADE_DOWNLOAD_FAILED", "the download failed"))?;
        if response.status() != 200 {
            return Err(RemoteError::new(
                "UPGRADE_DOWNLOAD_FAILED",
                "the download returned an error status",
            ));
        }
        response
            .body_mut()
            .with_config()
            .limit(limit as u64)
            .read_to_vec()
            .map_err(|_| RemoteError::new("UPGRADE_DOWNLOAD_FAILED", "the download failed"))
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn upgrade_io(error: io::Error) -> RemoteError {
    RemoteError::new("UPGRADE_IO", error.to_string())
}
