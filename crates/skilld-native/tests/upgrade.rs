#![cfg(unix)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest, Sha256};
use skilld_command::upgrade::{InstallChannel, PackageRunner, UpgradeNotice, UpgradeState};
use skilld_core::{ReleasePin, RemoteError};
use skilld_native::upgrade::{
    INSTALL_MARKER, InstallTarget, ReleaseFetcher, before_command, install_channel,
    install_release, read_state, run_worker,
};

const ASSET: &str = "skilld-cli-linux-x64-gnu";
const BASE: &str = "https://github.com/skilld-dev/skilld/releases/download/v9.0.0";

#[derive(Default)]
struct FakeFetcher {
    responses: BTreeMap<String, Vec<u8>>,
    requested: RefCell<Vec<String>>,
}

impl ReleaseFetcher for FakeFetcher {
    fn get(&self, url: &str, limit: usize) -> Result<Vec<u8>, RemoteError> {
        self.requested.borrow_mut().push(url.to_owned());
        self.responses
            .get(url)
            .filter(|body| body.len() <= limit)
            .cloned()
            .ok_or_else(|| RemoteError::new("UPGRADE_DOWNLOAD_FAILED", "missing"))
    }
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[3; 32])
}

fn pin() -> ReleasePin {
    ReleasePin {
        public_key: URL_SAFE_NO_PAD.encode(signing_key().verifying_key().as_bytes()),
    }
}

fn script(version: &str) -> Vec<u8> {
    format!("#!/bin/sh\necho 'skilld {version}'\n").into_bytes()
}

/// A release whose manifest names `signed_binary` and whose asset serves `served_binary`.
fn release(key: &SigningKey, signed_binary: &[u8], served_binary: &[u8]) -> FakeFetcher {
    let digest: String = Sha256::digest(signed_binary)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let manifest = format!("skilld-release-v1\nversion 9.0.0\n{digest}  {ASSET}\n").into_bytes();
    let mut message = b"skilld-release-v1\0".to_vec();
    message.extend_from_slice(&Sha256::digest(&manifest));
    let signature = URL_SAFE_NO_PAD.encode(key.sign(&message).to_bytes());
    let mut fetcher = FakeFetcher::default();
    fetcher
        .responses
        .insert(format!("{BASE}/skilld-release.txt"), manifest);
    fetcher
        .responses
        .insert(format!("{BASE}/skilld-release.sig"), signature.into_bytes());
    fetcher
        .responses
        .insert(format!("{BASE}/{ASSET}"), served_binary.to_vec());
    fetcher
}

fn installed(directory: &Path) -> PathBuf {
    let executable = directory.join("skilld");
    fs::write(&executable, script("3.0.0")).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(directory.join(INSTALL_MARKER), "{}").unwrap();
    executable
}

fn target<'a>(executable: &'a Path, pin: &'a ReleasePin) -> InstallTarget<'a> {
    InstallTarget {
        executable,
        current_version: "3.0.0",
        asset: ASSET,
        pin,
    }
}

fn leftovers(directory: &Path) -> Vec<String> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".skilld-upgrade"))
        .collect()
}

#[test]
fn a_verified_release_replaces_the_standalone_executable() {
    let directory = tempfile::tempdir().unwrap();
    let executable = installed(directory.path());
    let pin = pin();
    let fetcher = release(&signing_key(), &script("9.0.0"), &script("9.0.0"));

    install_release(&target(&executable, &pin), &fetcher, "9.0.0").unwrap();

    assert_eq!(fs::read(&executable).unwrap(), script("9.0.0"));
    assert!(leftovers(directory.path()).is_empty());
}

#[test]
fn a_tampered_binary_leaves_the_executable_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let executable = installed(directory.path());
    let pin = pin();
    let fetcher = release(&signing_key(), &script("9.0.0"), &script("9.0.0-evil"));

    let error = install_release(&target(&executable, &pin), &fetcher, "9.0.0").unwrap_err();

    assert_eq!(error.code, "RELEASE_DIGEST_MISMATCH");
    assert_eq!(fs::read(&executable).unwrap(), script("3.0.0"));
    assert!(leftovers(directory.path()).is_empty());
}

#[test]
fn a_release_signed_by_another_key_is_never_downloaded() {
    let directory = tempfile::tempdir().unwrap();
    let executable = installed(directory.path());
    let pin = pin();
    let attacker = SigningKey::from_bytes(&[4; 32]);
    let fetcher = release(&attacker, &script("9.0.0"), &script("9.0.0"));

    let error = install_release(&target(&executable, &pin), &fetcher, "9.0.0").unwrap_err();

    assert_eq!(error.code, "RELEASE_SIGNATURE_INVALID");
    assert_eq!(fs::read(&executable).unwrap(), script("3.0.0"));
    assert!(
        !fetcher
            .requested
            .borrow()
            .iter()
            .any(|url| url.ends_with(ASSET)),
        "the binary download must wait for a valid signature"
    );
}

#[test]
fn a_binary_that_reports_another_version_is_not_installed() {
    let directory = tempfile::tempdir().unwrap();
    let executable = installed(directory.path());
    let pin = pin();
    let fetcher = release(&signing_key(), &script("8.0.0"), &script("8.0.0"));

    let error = install_release(&target(&executable, &pin), &fetcher, "9.0.0").unwrap_err();

    assert_eq!(error.code, "UPGRADE_VERSION_MISMATCH");
    assert_eq!(fs::read(&executable).unwrap(), script("3.0.0"));
    assert!(leftovers(directory.path()).is_empty());
}

#[test]
fn an_older_or_equal_version_is_refused_before_any_download() {
    let directory = tempfile::tempdir().unwrap();
    let executable = installed(directory.path());
    let pin = pin();
    let fetcher = FakeFetcher::default();

    for version in ["3.0.0", "2.9.0", "3.0.0-beta.1"] {
        let error = install_release(&target(&executable, &pin), &fetcher, version).unwrap_err();
        assert_eq!(error.code, "UPGRADE_NOT_NEWER", "{version}");
    }
    assert!(fetcher.requested.borrow().is_empty());
}

#[test]
fn the_channel_comes_from_the_loader_or_a_regular_install_marker() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("skilld");

    assert_eq!(
        install_channel(&executable, None),
        InstallChannel::Unmanaged
    );
    assert_eq!(
        install_channel(&executable, Some("pnpm".into())),
        InstallChannel::Npm(PackageRunner::Pnpm)
    );

    let elsewhere = directory.path().join("elsewhere.json");
    fs::write(&elsewhere, "{}").unwrap();
    std::os::unix::fs::symlink(&elsewhere, directory.path().join(INSTALL_MARKER)).unwrap();
    assert_eq!(
        install_channel(&executable, None),
        InstallChannel::Unmanaged,
        "a symlinked marker does not make an install standalone"
    );

    fs::remove_file(directory.path().join(INSTALL_MARKER)).unwrap();
    fs::write(directory.path().join(INSTALL_MARKER), "{}").unwrap();
    assert_eq!(
        install_channel(&executable, None),
        InstallChannel::Standalone
    );
}

#[test]
fn the_check_worker_records_the_latest_npm_version() {
    let data = tempfile::tempdir().unwrap();
    let mut fetcher = FakeFetcher::default();
    fetcher.responses.insert(
        "https://registry.npmjs.org/skilld/latest".to_owned(),
        br#"{"version":"3.2.0"}"#.to_vec(),
    );

    run_worker(
        "check",
        data.path(),
        InstallChannel::Npm(PackageRunner::Npm),
        &fetcher,
        None,
        1_000,
    );

    let state = read_state(data.path());
    assert_eq!(state.latest.as_deref(), Some("3.2.0"));
    assert_eq!(state.checked_at, 1_000);
    assert_eq!(state.last_error, None);
}

#[test]
fn the_install_worker_records_a_failed_verification() {
    let directory = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let executable = installed(directory.path());
    let pin = pin();
    let fetcher = release(&signing_key(), &script("9.0.0"), &script("tampered"));

    run_worker(
        "install:9.0.0",
        data.path(),
        InstallChannel::Standalone,
        &fetcher,
        Some(target(&executable, &pin)),
        1_000,
    );

    let state = read_state(data.path());
    assert_eq!(state.last_error.as_deref(), Some("RELEASE_DIGEST_MISMATCH"));
    assert_eq!(state.attempted_version.as_deref(), Some("9.0.0"));
    assert_eq!(fs::read(&executable).unwrap(), script("3.0.0"));
}

#[test]
fn a_retry_after_a_failed_upgrade_names_the_previous_error() {
    let directory = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let executable = installed(directory.path());
    let state = UpgradeState {
        checked_at: 1_000,
        latest: Some("3.2.0".to_owned()),
        attempted_version: Some("3.2.0".to_owned()),
        attempted_at: Some(1_000),
        last_error: Some("UPGRADE_DOWNLOAD_FAILED".to_owned()),
    };
    fs::write(
        data.path().join("upgrade.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();

    let notice = before_command(
        data.path(),
        &executable,
        InstallChannel::Standalone,
        "3.0.0",
        5_000,
    );

    let message = notice
        .expect("an overdue retry still plans an upgrade")
        .message();
    assert!(
        message.contains("UPGRADE_DOWNLOAD_FAILED"),
        "the notice must name the previous failure: {message}"
    );
}

#[test]
fn a_known_npm_upgrade_prints_its_notice_without_a_worker() {
    let data = tempfile::tempdir().unwrap();
    let state = UpgradeState {
        checked_at: 1_000,
        latest: Some("3.2.0".to_owned()),
        ..UpgradeState::default()
    };
    fs::write(
        data.path().join("upgrade.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();

    let notice = before_command(
        data.path(),
        Path::new("/nonexistent/skilld"),
        InstallChannel::Npm(PackageRunner::Npx),
        "3.0.0",
        1_060,
    );

    assert_eq!(
        notice,
        Some(UpgradeNotice::Available {
            version: "3.2.0".to_owned(),
            command: "npx skilld@latest".to_owned(),
        })
    );
}
