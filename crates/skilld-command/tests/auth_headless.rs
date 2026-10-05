use std::sync::atomic::{AtomicBool, Ordering};

use skilld_command::{CommandError, Host, InstalledSkill, run};
use skilld_core::{InstallScope, InstallSource};

struct ManualAuthHost(AtomicBool);

impl Host for ManualAuthHost {
    fn list(&self, _: InstallScope) -> Result<Vec<String>, CommandError> {
        unreachable!()
    }
    fn install(&self, _: InstallSource, _: InstallScope) -> Result<InstalledSkill, CommandError> {
        unreachable!()
    }
    fn auth_login_without_browser(&self) -> Result<(), CommandError> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn login_without_browser_uses_the_manual_authorization_boundary() {
    let host = ManualAuthHost(AtomicBool::new(false));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run(
        ["skilld", "auth", "login", "--no-browser"],
        &host,
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    assert!(host.0.load(Ordering::SeqCst));
}
