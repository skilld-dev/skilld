use skilld_core::{TrustedRoot, TrustedRootPin, verify_trusted_root};
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
    verify_trusted_root(root, &pin)?;
    Ok(())
}
