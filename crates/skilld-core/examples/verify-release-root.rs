use skilld_core::{
    ArtifactAttestation, TrustedRoot, TrustedRootPin, verify_attestation, verify_trusted_root,
};
use std::{env, error::Error, fs};

// Release preflight uses the same wire type and verifier as hosted CLI operations.
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let path = args.next().ok_or("trusted root document is required")?;
    let pin = TrustedRootPin {
        key_id: args.next().ok_or("root key id is required")?,
        public_key: args.next().ok_or("root public key is required")?,
    };
    let root: TrustedRoot = serde_json::from_slice(&fs::read(path)?)?;
    let root = verify_trusted_root(root, &pin)?;
    let attestation_path = args.next().ok_or("Artifact attestation is required")?;
    let attestation: ArtifactAttestation = serde_json::from_slice(&fs::read(attestation_path)?)?;
    verify_attestation(&attestation, &root)?;
    Ok(())
}
