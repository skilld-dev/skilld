use skilld_command::{
    CommandError, CommandPlatform, Host, InstalledSkill, LocalStore, OutputContext, StoreError,
    run_with_output,
};
use skilld_core::{InstallScope, InstallSource, SearchResponse};

struct V2Host;

impl Host for V2Host {
    fn list(&self, _scope: InstallScope) -> Result<Vec<String>, CommandError> {
        Ok(vec!["vue".to_owned()])
    }

    fn install(
        &self,
        _source: InstallSource,
        _scope: InstallScope,
    ) -> Result<InstalledSkill, CommandError> {
        unreachable!("install is outside this test")
    }

    fn search(&self, _query: &str) -> Result<SearchResponse, CommandError> {
        unreachable!("search is outside this test")
    }

    fn auth_status(&self) -> Result<bool, CommandError> {
        Ok(true)
    }

    fn auth_logout(&self) -> Result<(), CommandError> {
        Ok(())
    }
}

const PLAIN: OutputContext = OutputContext::Plain {
    platform: CommandPlatform::Unix,
};

fn run(args: &[&str]) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(args, &V2Host, PLAIN, &mut stdout, &mut stderr);
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn v2_account_commands_run_their_auth_replacement() {
    assert_eq!(
        run(&["skilld", "whoami"]),
        run(&["skilld", "auth", "status"])
    );
    assert_eq!(
        run(&["skilld", "logout"]),
        run(&["skilld", "auth", "logout"])
    );
    assert_eq!(
        run(&["skilld", "--plain", "whoami"]),
        run(&["skilld", "--plain", "auth", "status"])
    );
    assert_eq!(run(&["skilld", "whoami"]).0, 0);
}

#[test]
fn v2_info_lists_installed_skills() {
    assert_eq!(run(&["skilld", "info"]), run(&["skilld", "list"]));
    assert_eq!(run(&["skilld", "info"]).1, "vue\n");
}

#[test]
fn removed_v2_commands_name_their_replacement() {
    let (code, stdout, stderr) = run(&["skilld", "author", "package", "vue"]);
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert!(stderr.starts_with("REMOVED_COMMAND"), "{stderr}");
    assert!(stderr.contains("generate-package-skill"), "{stderr}");

    for command in [
        "watch",
        "unwatch",
        "cache",
        "changes",
        "setup",
        "uninstall",
        "pull",
    ] {
        let (code, _, stderr) = run(&["skilld", command]);
        assert_eq!(code, 2, "{command}");
        assert!(stderr.starts_with("REMOVED_COMMAND"), "{command}: {stderr}");
        assert!(stderr.contains("migrate-v2-to-v3"), "{command}: {stderr}");
    }
}

#[test]
fn a_skill_named_like_a_v2_command_still_resolves_as_a_skill() {
    let (_, _, stderr) = run(&["skilld", "view", "watch"]);
    assert!(!stderr.contains("REMOVED_COMMAND"), "{stderr}");
}

#[test]
fn a_v2_lockfile_is_named_with_the_migration_step() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join(".skills");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("skilld-lock.yaml"),
        "skills:\n  vue:\n    packageName: vue\n    version: 3.5.0\n",
    )
    .unwrap();

    let error = LocalStore::new(root).list(&[]).unwrap_err();

    let StoreError::InvalidLockfile(message) = error else {
        panic!("expected an invalid lockfile, got {error:?}");
    };
    assert!(message.contains("skilld v2"), "{message}");
    assert!(message.contains("migrate-v2-to-v3"), "{message}");
}
