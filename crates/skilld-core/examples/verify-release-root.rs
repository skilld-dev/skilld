use skilld_core::{
    ArtifactAttestation, TrustedKeyStatus, TrustedRoot, TrustedRootPin, VerifiedTrustedRoot,
    verify_attestation, verify_trusted_root,
};
use std::{collections::BTreeMap, env, error::Error, fs};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

// Release preflight uses the same wire type and verifier as hosted CLI operations.
fn read_root(
    args: &mut impl Iterator<Item = String>,
) -> Result<VerifiedTrustedRoot, Box<dyn Error>> {
    let path = args.next().ok_or("trusted root document is required")?;
    let pin = TrustedRootPin {
        key_id: args.next().ok_or("root key id is required")?,
        public_key: args.next().ok_or("root public key is required")?,
    };
    let root: TrustedRoot = serde_json::from_slice(&fs::read(path)?)?;
    Ok(verify_trusted_root(root, &pin)?)
}

// Compare the complete acceptance policy, not just the live probe's signer.
// Active and overlapping keys both verify Artifacts. Retired and revoked keys do not.
fn supported_signers(
    root: &VerifiedTrustedRoot,
) -> Result<BTreeMap<&str, (&str, OffsetDateTime, OffsetDateTime)>, time::error::Parse> {
    root.root()
        .keys
        .iter()
        .filter(|key| {
            matches!(
                key.status,
                TrustedKeyStatus::Active | TrustedKeyStatus::Overlapping
            )
        })
        .map(|key| {
            Ok((
                key.key_id.as_str(),
                (
                    key.public_key.as_str(),
                    OffsetDateTime::parse(&key.not_before, &Rfc3339)?,
                    OffsetDateTime::parse(&key.not_after, &Rfc3339)?,
                ),
            ))
        })
        .collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let primary = read_root(&mut args)?;
    let attestation_path = args.next().ok_or("Artifact attestation is required")?;
    let attestation: ArtifactAttestation = serde_json::from_slice(&fs::read(attestation_path)?)?;
    verify_attestation(&attestation, &primary)?;
    let backup = read_root(&mut args)?;
    verify_attestation(&attestation, &backup)?;
    if supported_signers(&primary)? != supported_signers(&backup)? {
        return Err("the pinned roots' supported Artifact signers differ".into());
    }
    Ok(())
}
