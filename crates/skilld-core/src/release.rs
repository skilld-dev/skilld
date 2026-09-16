use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::remote::{RemoteError, verify_ed25519, verifying_key};

const RELEASE_DOMAIN: &[u8] = b"skilld-release-v1\0";
const MANIFEST_HEADER: &str = "skilld-release-v1";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_ASSET_NAME: usize = 128;

/// The release signing public key compiled into the CLI, in canonical base64url.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleasePin {
    pub public_key: String,
}

/// A release manifest whose signature, format, and version passed verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedRelease {
    version: String,
    assets: BTreeMap<String, [u8; 32]>,
}

impl VerifiedRelease {
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Verifies a signed release manifest for one exact version.
///
/// The signature covers `skilld-release-v1\0 || SHA256(manifest)`. The manifest names
/// its version, so an older signed manifest cannot stand in for a newer release.
pub fn verify_release_manifest(
    manifest: &[u8],
    signature: &str,
    pin: &ReleasePin,
    expected_version: &str,
) -> Result<VerifiedRelease, RemoteError> {
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(manifest_error("the release manifest is too large"));
    }
    let key = verifying_key(&pin.public_key, "the release public key is invalid")?;
    verify_ed25519(
        &key,
        RELEASE_DOMAIN,
        manifest,
        signature.trim(),
        "RELEASE_SIGNATURE_INVALID",
        "the release manifest signature is invalid",
    )
    .map_err(|error| {
        if error.code == "INVALID_BASE64URL" {
            RemoteError::new(
                "RELEASE_SIGNATURE_INVALID",
                "the release manifest signature is invalid",
            )
        } else {
            error
        }
    })?;
    let release = parse_manifest(manifest)?;
    if release.version != expected_version {
        return Err(RemoteError::new(
            "RELEASE_VERSION_MISMATCH",
            format!(
                "the signed release manifest is for skilld {}, not {expected_version}",
                release.version
            ),
        ));
    }
    Ok(release)
}

/// Checks one downloaded release asset against its signed digest.
pub fn verify_release_asset(
    release: &VerifiedRelease,
    asset: &str,
    bytes: &[u8],
) -> Result<(), RemoteError> {
    let expected = release.assets.get(asset).ok_or_else(|| {
        RemoteError::new(
            "RELEASE_ASSET_MISSING",
            format!("the signed release manifest does not name {asset}"),
        )
    })?;
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    if actual != *expected {
        return Err(RemoteError::new(
            "RELEASE_DIGEST_MISMATCH",
            format!("the downloaded {asset} does not match its signed digest"),
        ));
    }
    Ok(())
}

fn parse_manifest(manifest: &[u8]) -> Result<VerifiedRelease, RemoteError> {
    let text = std::str::from_utf8(manifest)
        .map_err(|_| manifest_error("the release manifest is not UTF-8"))?;
    let body = text
        .strip_suffix('\n')
        .ok_or_else(|| manifest_error("the release manifest must end with a newline"))?;
    if body.contains('\r') {
        return Err(manifest_error(
            "the release manifest must use LF line endings",
        ));
    }
    let mut lines = body.split('\n');
    if lines.next() != Some(MANIFEST_HEADER) {
        return Err(manifest_error(
            "the release manifest header is not skilld-release-v1",
        ));
    }
    let version = lines
        .next()
        .and_then(|line| line.strip_prefix("version "))
        .filter(|version| is_release_version(version))
        .ok_or_else(|| manifest_error("the release manifest version is invalid"))?
        .to_owned();
    let mut assets = BTreeMap::new();
    let mut names = BTreeSet::new();
    for line in lines {
        let (digest, name) = line
            .split_once("  ")
            .ok_or_else(|| manifest_error("a release manifest entry is invalid"))?;
        let digest = parse_sha256(digest)
            .ok_or_else(|| manifest_error("a release manifest digest is invalid"))?;
        if !is_asset_name(name) || !names.insert(name) {
            return Err(manifest_error("a release manifest asset name is invalid"));
        }
        assets.insert(name.to_owned(), digest);
    }
    if assets.is_empty() {
        return Err(manifest_error("the release manifest names no assets"));
    }
    Ok(VerifiedRelease { version, assets })
}

/// A `MAJOR.MINOR.PATCH` version with an optional dotted ASCII prerelease.
pub fn is_release_version(value: &str) -> bool {
    let (core, prerelease) = match value.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (value, None),
    };
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| is_number(part))
        && prerelease.is_none_or(|prerelease| {
            prerelease.split('.').all(|identifier| {
                !identifier.is_empty()
                    && identifier.len() <= 32
                    && identifier.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
        })
}

fn is_number(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 9
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

fn is_asset_name(name: &str) -> bool {
    name.starts_with("skilld-")
        && name.len() <= MAX_ASSET_NAME
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
        })
        && !name.contains("..")
}

fn parse_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks(2).enumerate() {
        let high = hex_value(chunk[0])?;
        let low = hex_value(chunk[1])?;
        digest[index] = (high << 4) | low;
    }
    Some(digest)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn manifest_error(message: &'static str) -> RemoteError {
    RemoteError::new("RELEASE_MANIFEST_INVALID", message)
}
