//! Pure decisions for CLI upgrades. The native executable owns every effect.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use skilld_core::is_release_version;

/// How long a latest-version check stays fresh.
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;
/// How long a failed standalone upgrade waits before it retries the same version.
pub const RETRY_INTERVAL_SECONDS: u64 = 60 * 60;
/// The largest release binary skilld downloads.
pub const MAX_BINARY_BYTES: usize = 64 * 1024 * 1024;
pub const RELEASE_MANIFEST_ASSET: &str = "skilld-release.txt";
pub const RELEASE_SIGNATURE_ASSET: &str = "skilld-release.sig";
pub const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/skilld-dev/skilld/releases/latest";
pub const NPM_LATEST_URL: &str = "https://registry.npmjs.org/skilld/latest";

/// How the running skilld executable was installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallChannel {
    /// Installed by the skilld install script. skilld upgrades it in place.
    Standalone,
    /// Launched by the npm loader. The package manager owns upgrades.
    Npm(PackageRunner),
    /// A development build or another package manager. skilld never checks.
    Unmanaged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageRunner {
    Npx,
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageRunner {
    /// Parses the `SKILLD_LAUNCHER` value the npm loader sets.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "npx" => Some(Self::Npx),
            "npm" => Some(Self::Npm),
            "pnpm" => Some(Self::Pnpm),
            "yarn" => Some(Self::Yarn),
            "bun" => Some(Self::Bun),
            _ => None,
        }
    }

    const fn upgrade_command(self) -> &'static str {
        match self {
            Self::Npx => "npx skilld@latest",
            Self::Npm => "npm install --global skilld",
            Self::Pnpm => "pnpm add --global skilld",
            Self::Yarn => "yarn global add skilld",
            Self::Bun => "bun add --global skilld",
        }
    }
}

/// The upgrade state skilld keeps in its data directory.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeState {
    /// Unix seconds of the last latest-version check, successful or not.
    #[serde(default)]
    pub checked_at: u64,
    #[serde(default)]
    pub latest: Option<String>,
    #[serde(default)]
    pub attempted_version: Option<String>,
    #[serde(default)]
    pub attempted_at: Option<u64>,
    /// The error code of the last failed check or upgrade, for diagnosis.
    #[serde(default)]
    pub last_error: Option<String>,
}

impl UpgradeState {
    /// Parses stored state. Unreadable state starts over, because it only schedules work.
    pub fn parse(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpgradeWorker {
    Check,
    Install { version: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpgradeNotice {
    Available { version: String, command: String },
    Installing { version: String },
}

impl UpgradeNotice {
    pub fn message(&self) -> String {
        match self {
            Self::Available { version, command } => {
                format!("skilld {version} is available. Run {command} to upgrade.")
            }
            Self::Installing { version } => format!(
                "Upgrading skilld to {version} in the background. Restart skilld to use it."
            ),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpgradePlan {
    pub worker: Option<UpgradeWorker>,
    pub notice: Option<UpgradeNotice>,
}

/// Decides the background work and the notice for one command run.
pub fn plan_upgrade(
    current: &str,
    channel: &InstallChannel,
    state: &UpgradeState,
    now: u64,
) -> UpgradePlan {
    if *channel == InstallChannel::Unmanaged {
        return UpgradePlan::default();
    }
    let stale = state.checked_at > now || now - state.checked_at >= CHECK_INTERVAL_SECONDS;
    let check = UpgradePlan {
        worker: stale.then_some(UpgradeWorker::Check),
        notice: None,
    };
    let Some(latest) = state
        .latest
        .as_deref()
        .filter(|latest| is_newer(latest, current))
    else {
        return check;
    };
    match channel {
        InstallChannel::Npm(runner) => UpgradePlan {
            worker: check.worker,
            notice: Some(UpgradeNotice::Available {
                version: latest.to_owned(),
                command: runner.upgrade_command().to_owned(),
            }),
        },
        InstallChannel::Standalone => {
            let waiting = state.attempted_version.as_deref() == Some(latest)
                && state.attempted_at.is_some_and(|attempted| {
                    attempted <= now && now - attempted < RETRY_INTERVAL_SECONDS
                });
            if waiting {
                return check;
            }
            UpgradePlan {
                worker: Some(UpgradeWorker::Install {
                    version: latest.to_owned(),
                }),
                notice: Some(UpgradeNotice::Installing {
                    version: latest.to_owned(),
                }),
            }
        }
        InstallChannel::Unmanaged => UpgradePlan::default(),
    }
}

/// True when `candidate` is a strictly newer release than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(candidate), Some(current)) => candidate.cmp(&current) == Ordering::Greater,
        _ => false,
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Version<'a> {
    core: [u64; 3],
    prerelease: Option<Vec<&'a str>>,
}

impl<'a> Version<'a> {
    fn parse(value: &'a str) -> Option<Self> {
        if !is_release_version(value) {
            return None;
        }
        let (core, prerelease) = match value.split_once('-') {
            Some((core, prerelease)) => (core, Some(prerelease.split('.').collect())),
            None => (value, None),
        };
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        Some(Self {
            core: [parts.next()??, parts.next()??, parts.next()??],
            prerelease,
        })
    }
}

impl Ord for Version<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => left
                    .iter()
                    .zip(right.iter())
                    .map(
                        |(left, right)| match (left.parse::<u64>(), right.parse::<u64>()) {
                            (Ok(left), Ok(right)) => left.cmp(&right),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => left.cmp(right),
                        },
                    )
                    .find(|ordering| *ordering != Ordering::Equal)
                    .unwrap_or_else(|| left.len().cmp(&right.len())),
            })
    }
}

impl PartialOrd for Version<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The release asset name for one native target, from Rust `target_os`, `target_arch`,
/// and `target_env` values.
pub fn release_asset(os: &str, arch: &str, env: &str) -> Option<&'static str> {
    Some(match (os, arch, env) {
        ("linux", "x86_64", "gnu") => "skilld-cli-linux-x64-gnu",
        ("linux", "x86_64", "musl") => "skilld-cli-linux-x64-musl",
        ("linux", "aarch64", "gnu") => "skilld-cli-linux-arm64-gnu",
        ("linux", "aarch64", "musl") => "skilld-cli-linux-arm64-musl",
        ("macos", "aarch64", _) => "skilld-cli-darwin-arm64",
        ("macos", "x86_64", _) => "skilld-cli-darwin-x64",
        ("windows", "x86_64", _) => "skilld-cli-win32-x64-msvc.exe",
        ("windows", "aarch64", _) => "skilld-cli-win32-arm64-msvc.exe",
        _ => return None,
    })
}

/// The download URL for one asset of one exact release.
pub fn release_asset_url(version: &str, asset: &str) -> Option<String> {
    is_release_version(version).then(|| {
        format!("https://github.com/skilld-dev/skilld/releases/download/v{version}/{asset}")
    })
}

/// Reads the version from the GitHub latest-release response.
pub fn parse_github_release(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release = serde_json::from_slice(body).ok()?;
    release
        .tag_name
        .strip_prefix('v')
        .filter(|version| is_release_version(version))
        .map(str::to_owned)
}

/// Reads the version from the npm registry `latest` response.
pub fn parse_npm_latest(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Latest {
        version: String,
    }
    let latest: Latest = serde_json::from_slice(body).ok()?;
    is_release_version(&latest.version).then_some(latest.version)
}
