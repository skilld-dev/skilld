use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest, Sha256};
use skilld_core::{ReleasePin, verify_release_asset, verify_release_manifest};

const DOMAIN: &[u8] = b"skilld-release-v1\0";
const BINARY: &[u8] = b"native skilld binary";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn pin(key: &SigningKey) -> ReleasePin {
    ReleasePin {
        public_key: URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes()),
    }
}

fn manifest(version: &str) -> Vec<u8> {
    let digest = hex(&Sha256::digest(BINARY));
    format!(
        "skilld-release-v1\nversion {version}\n{digest}  skilld-cli-linux-x64-gnu\n{}  skilld-cli-win32-x64-msvc.exe\n",
        "0".repeat(64)
    )
    .into_bytes()
}

fn sign(key: &SigningKey, manifest: &[u8]) -> String {
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(&Sha256::digest(manifest));
    URL_SAFE_NO_PAD.encode(key.sign(&message).to_bytes())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn a_signed_manifest_verifies_the_release_binary() {
    let key = key();
    let manifest = manifest("3.1.0");

    let release =
        verify_release_manifest(&manifest, &sign(&key, &manifest), &pin(&key), "3.1.0").unwrap();

    verify_release_asset(&release, "skilld-cli-linux-x64-gnu", BINARY).unwrap();
}

#[test]
fn a_changed_binary_fails_its_digest() {
    let key = key();
    let manifest = manifest("3.1.0");
    let release =
        verify_release_manifest(&manifest, &sign(&key, &manifest), &pin(&key), "3.1.0").unwrap();

    let error =
        verify_release_asset(&release, "skilld-cli-linux-x64-gnu", b"tampered").unwrap_err();
    assert_eq!(error.code, "RELEASE_DIGEST_MISMATCH");

    let error = verify_release_asset(&release, "skilld-cli-linux-arm64-gnu", BINARY).unwrap_err();
    assert_eq!(error.code, "RELEASE_ASSET_MISSING");
}

#[test]
fn a_manifest_signed_by_another_key_is_rejected() {
    let manifest = manifest("3.1.0");
    let other = SigningKey::from_bytes(&[9; 32]);

    let error = verify_release_manifest(&manifest, &sign(&other, &manifest), &pin(&key()), "3.1.0")
        .unwrap_err();

    assert_eq!(error.code, "RELEASE_SIGNATURE_INVALID");
}

#[test]
fn a_changed_manifest_breaks_its_signature() {
    let key = key();
    let signed = manifest("3.1.0");
    let signature = sign(&key, &signed);
    let mut changed = signed.clone();
    changed[40] ^= 1;

    let error = verify_release_manifest(&changed, &signature, &pin(&key), "3.1.0").unwrap_err();

    assert!(
        ["RELEASE_SIGNATURE_INVALID", "RELEASE_MANIFEST_INVALID"].contains(&error.code),
        "{}",
        error.code
    );
}

#[test]
fn an_older_signed_manifest_cannot_stand_in_for_the_requested_version() {
    let key = key();
    let old = manifest("3.0.0");

    let error = verify_release_manifest(&old, &sign(&key, &old), &pin(&key), "3.1.0").unwrap_err();

    assert_eq!(error.code, "RELEASE_VERSION_MISMATCH");
}

#[test]
fn a_signature_without_the_release_domain_is_rejected() {
    let key = key();
    let manifest = manifest("3.1.0");
    let undomained = URL_SAFE_NO_PAD.encode(key.sign(&Sha256::digest(&manifest)).to_bytes());

    let error = verify_release_manifest(&manifest, &undomained, &pin(&key), "3.1.0").unwrap_err();

    assert_eq!(error.code, "RELEASE_SIGNATURE_INVALID");
}

#[test]
fn malformed_manifests_are_rejected_even_when_signed() {
    let key = key();
    for manifest in [
        b"skilld-release-v2\nversion 3.1.0\n".to_vec(),
        b"skilld-release-v1\nversion 3.1.0\n".to_vec(),
        format!(
            "skilld-release-v1\nversion 3.1.0\n{}  ../skilld\n",
            "0".repeat(64)
        )
        .into_bytes(),
        format!(
            "skilld-release-v1\nversion 3.1.0\n{0}  skilld-a\n{0}  skilld-a\n",
            "0".repeat(64)
        )
        .into_bytes(),
        format!(
            "skilld-release-v1\nversion 3.1.0\n{}  skilld-a\n",
            "0".repeat(63)
        )
        .into_bytes(),
        format!(
            "skilld-release-v1\r\nversion 3.1.0\r\n{}  skilld-a\r\n",
            "0".repeat(64)
        )
        .into_bytes(),
    ] {
        let error = verify_release_manifest(&manifest, &sign(&key, &manifest), &pin(&key), "3.1.0")
            .unwrap_err();
        assert_eq!(
            error.code,
            "RELEASE_MANIFEST_INVALID",
            "{}",
            String::from_utf8_lossy(&manifest)
        );
    }
}

#[test]
fn a_manifest_signed_by_the_release_script_verifies() {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/release");
    let read = |name: &str| std::fs::read_to_string(fixtures.join(name)).unwrap();
    let pin = ReleasePin {
        public_key: read("public-key.txt").trim().to_owned(),
    };

    let release = verify_release_manifest(
        read("skilld-release.txt").as_bytes(),
        &read("skilld-release.sig"),
        &pin,
        "9.0.0",
    )
    .unwrap();

    verify_release_asset(&release, "skilld-cli-linux-x64-gnu", BINARY).unwrap();
}
