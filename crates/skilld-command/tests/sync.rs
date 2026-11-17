use std::fs;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::json;
use sha2::{Digest, Sha256};
use skilld_command::{LocalHost, run};
use skilld_command::{
    PreparedRemoteSkill, RemoteLatestCommit, RemoteProvider, RemoteSourceState,
    RemoteUpdateComparison, RemoteUpdateResult,
};
use skilld_core::{
    CommitSha, LockedSource, PreparedFile, RemoteError, RemoteSelector, SearchResponse,
    SourceSelector, SourceStatus,
};

fn fixture() -> (tempfile::TempDir, LocalHost) {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir_all(project.join(".skills")).unwrap();
    for name in ["alpha", "beta"] {
        let source = project.join("sources").join(name);
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Test.\n---\nOriginal.\n"),
        )
        .unwrap();
    }
    fs::write(project.join(".skills/skilld.json"), json!({
        "version": 1,
        "name": "fixture",
        "agents": ["codex", "claude-code", "opencode"],
        "mode": "symlink",
        "skills": {"alpha": {"source": "../sources/alpha"}, "beta": {"source": "../sources/beta"}},
        "requires": {"pr": ["alpha", "beta"]}
    }).to_string()).unwrap();
    let host = LocalHost::new(project, root.path().join("global"));
    (root, host)
}

fn command(host: &LocalHost, args: &[&str]) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run(args.iter().copied(), host, &mut stdout, &mut stderr);
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn declaration(root: &Path, name: &str, agents: &[&str], source: &str, mode: &str) -> String {
    let path = root.join("project/.skills").join(format!("{name}.json"));
    fs::write(
        &path,
        json!({
            "version": 1, "name": name, "agents": agents, "mode": mode,
            "skills": {"alpha": {"source": source}}, "requires": {"consumer": ["alpha"]}
        })
        .to_string(),
    )
    .unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn shared_skill_keeps_every_declarations_targets_and_each_check_stays_current() {
    let (root, host) = fixture();
    let one = declaration(
        root.path(),
        "one",
        &["codex"],
        "../sources/alpha",
        "symlink",
    );
    let two = declaration(
        root.path(),
        "two",
        &["claude-code"],
        "../sources/alpha",
        "symlink",
    );
    assert_eq!(command(&host, &["skilld", "sync", "--manifest", &one]).0, 0);
    assert_eq!(command(&host, &["skilld", "sync", "--manifest", &two]).0, 0);
    for target in [".agents/skills", ".claude/skills"] {
        assert!(
            root.path()
                .join("project")
                .join(target)
                .join("alpha/SKILL.md")
                .is_file(),
            "{target}"
        );
    }
    for manifest in [&one, &two] {
        assert_eq!(
            command(
                &host,
                &["skilld", "sync", "--manifest", manifest, "--check"]
            )
            .0,
            0
        );
        assert_eq!(
            command(&host, &["skilld", "sync", "--manifest", manifest]).0,
            0
        );
    }
}

#[test]
fn changing_one_declarations_targets_preserves_targets_required_by_other_declarations() {
    let (root, host) = fixture();
    let one = declaration(
        root.path(),
        "one",
        &["codex", "opencode"],
        "../sources/alpha",
        "symlink",
    );
    let two = declaration(
        root.path(),
        "two",
        &["codex", "claude-code"],
        "../sources/alpha",
        "symlink",
    );
    for manifest in [&one, &two] {
        assert_eq!(
            command(&host, &["skilld", "sync", "--manifest", manifest]).0,
            0
        );
    }
    declaration(
        root.path(),
        "one",
        &["opencode"],
        "../sources/alpha",
        "symlink",
    );
    assert_eq!(command(&host, &["skilld", "sync", "--manifest", &one]).0, 0);
    for target in [".agents/skills", ".claude/skills", ".opencode/skills"] {
        assert!(
            root.path()
                .join("project")
                .join(target)
                .join("alpha/SKILL.md")
                .is_file()
        );
    }
    declaration(
        root.path(),
        "two",
        &["claude-code"],
        "../sources/alpha",
        "symlink",
    );
    assert_eq!(command(&host, &["skilld", "sync", "--manifest", &two]).0, 0);
    assert!(!root.path().join("project/.agents/skills/alpha").exists());
    for manifest in [&one, &two] {
        assert_eq!(
            command(
                &host,
                &["skilld", "sync", "--manifest", manifest, "--check"]
            )
            .0,
            0
        );
    }
}

#[test]
fn a_second_declaration_cannot_replace_an_owned_source_or_target_mode() {
    let (root, host) = fixture();
    let one = declaration(
        root.path(),
        "one",
        &["codex"],
        "../sources/alpha",
        "symlink",
    );
    assert_eq!(command(&host, &["skilld", "sync", "--manifest", &one]).0, 0);
    let other = root.path().join("project/other/alpha");
    fs::create_dir_all(&other).unwrap();
    fs::write(
        other.join("SKILL.md"),
        "---\nname: alpha\ndescription: Other.\n---\nOther instructions.\n",
    )
    .unwrap();
    let two = declaration(
        root.path(),
        "two",
        &["claude-code"],
        "../other/alpha",
        "symlink",
    );
    let result = command(&host, &["skilld", "sync", "--manifest", &two]);
    assert_ne!(result.0, 0);
    assert!(result.2.contains("one"), "{}", result.2);
    assert!(
        fs::read_to_string(root.path().join("project/.agents/skills/alpha/SKILL.md"))
            .unwrap()
            .contains("Original.")
    );
    assert!(!root.path().join("project/.claude/skills/alpha").exists());
    let two = declaration(root.path(), "two", &["codex"], "../sources/alpha", "copy");
    assert_ne!(command(&host, &["skilld", "sync", "--manifest", &two]).0, 0);
    assert!(
        fs::symlink_metadata(root.path().join("project/.agents/skills/alpha"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn sync_detects_permission_only_edits_and_updates_every_target_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (root, host) = fixture();
    let script = root.path().join("project/sources/alpha/scripts/tool.sh");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
    let one = declaration(
        root.path(),
        "one",
        &["codex"],
        "../sources/alpha",
        "symlink",
    );
    let two = declaration(
        root.path(),
        "two",
        &["claude-code"],
        "../sources/alpha",
        "copy",
    );
    assert_eq!(command(&host, &["skilld", "sync", "--manifest", &one]).0, 0);
    assert_eq!(command(&host, &["skilld", "sync", "--manifest", &two]).0, 0);
    for mode in [0o755, 0o644] {
        fs::set_permissions(&script, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(
            command(&host, &["skilld", "sync", "--manifest", &one, "--check"]).0,
            1
        );
        assert_eq!(command(&host, &["skilld", "sync", "--manifest", &one]).0, 0);
        for target in [".agents/skills", ".claude/skills"] {
            let installed = root
                .path()
                .join("project")
                .join(target)
                .join("alpha/scripts/tool.sh");
            assert_eq!(
                fs::metadata(installed).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
        assert_eq!(
            command(&host, &["skilld", "sync", "--manifest", &two, "--check"]).0,
            0
        );
    }
}

#[test]
fn sync_installs_required_skills_to_every_declared_agent_and_detects_local_edits() {
    let (root, host) = fixture();
    let first = command(&host, &["skilld", "sync"]);
    assert_eq!(first.0, 0, "{}", first.2);
    let project = root.path().join("project");
    for target in [".agents/skills", ".claude/skills", ".opencode/skills"] {
        assert!(
            fs::read_to_string(project.join(target).join("beta/SKILL.md"))
                .unwrap()
                .contains("Original.")
        );
    }
    assert_eq!(
        command(&host, &["skilld", "sync", "--check", "--json"]).0,
        0
    );
    fs::write(
        project.join("sources/alpha/SKILL.md"),
        "---\nname: alpha\ndescription: Test.\n---\nChanged.\n",
    )
    .unwrap();
    let checked = command(&host, &["skilld", "sync", "--check", "--json"]);
    assert_eq!(checked.0, 1, "{}", checked.2);
    let report: serde_json::Value = serde_json::from_str(&checked.1).unwrap();
    assert_eq!(report["data"]["current"], false);
    assert!(
        fs::read_to_string(project.join(".agents/skills/alpha/SKILL.md"))
            .unwrap()
            .contains("Original.")
    );
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    assert!(
        fs::read_to_string(project.join(".agents/skills/alpha/SKILL.md"))
            .unwrap()
            .contains("Changed.")
    );
    let removed = command(&host, &["skilld", "remove", "alpha"]);
    assert_ne!(removed.0, 0);
    assert!(removed.2.contains("REQUIRED_SKILL"));
}

#[test]
fn an_unmanaged_target_prevents_the_entire_sync() {
    let (root, host) = fixture();
    let target = root.path().join("project/.agents/skills/beta");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("SKILL.md"), "Owned by the user.").unwrap();
    let result = command(&host, &["skilld", "sync"]);
    assert_ne!(result.0, 0);
    assert_eq!(
        fs::read_to_string(target.join("SKILL.md")).unwrap(),
        "Owned by the user."
    );
    assert!(!root.path().join("project/.agents/skills/alpha").exists());
}

#[test]
fn a_missing_dependency_fails_before_any_installation() {
    let (root, host) = fixture();
    let manifest = root.path().join("project/.skills/skilld.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["requires"]["pr"] = json!(["missing"]);
    fs::write(manifest, value.to_string()).unwrap();
    let result = command(&host, &["skilld", "sync"]);
    assert_ne!(result.0, 0);
    assert!(result.2.contains("pr requires missing"), "{}", result.2);
    assert!(!root.path().join("project/.agents/skills/alpha").exists());
}

#[test]
fn sync_releases_removed_requirements_before_explicit_skill_removal() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    let manifest = root.path().join("project/.skills/skilld.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["requires"] = json!({});
    fs::write(manifest, value.to_string()).unwrap();
    assert_eq!(command(&host, &["skilld", "sync", "--check"]).0, 1);
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    let removed = command(&host, &["skilld", "remove", "alpha"]);
    assert_eq!(removed.0, 0, "{}", removed.2);
}

#[test]
fn global_sync_uses_global_agent_roots_and_keeps_the_project_store_separate() {
    let (root, host) = fixture();
    let result = command(&host, &["skilld", "sync", "--global"]);
    assert_eq!(result.0, 0, "{}", result.2);
    for target in [
        ".agents/skills",
        ".claude/skills",
        ".config/opencode/skills",
    ] {
        assert!(
            root.path().join(target).join("alpha/SKILL.md").is_file(),
            "{target}"
        );
    }
    assert!(root.path().join("global/skills/skilld-lock.yaml").is_file());
    assert!(
        !root
            .path()
            .join("project/.skills/skilld-lock.yaml")
            .exists()
    );
    assert_eq!(
        command(&host, &["skilld", "sync", "--global", "--check"]).0,
        0
    );
}

#[cfg(unix)]
#[test]
fn sync_adopts_only_identical_unmanaged_links() {
    let (root, host) = fixture();
    let target = root.path().join("project/.agents/skills");
    fs::create_dir_all(&target).unwrap();
    let original = root.path().join("project/sources/alpha");
    std::os::unix::fs::symlink(&original, target.join("alpha")).unwrap();
    let result = command(&host, &["skilld", "sync", "--adopt"]);
    assert_eq!(result.0, 0, "{}", result.2);
    assert_ne!(fs::canonicalize(target.join("alpha")).unwrap(), original);
    assert!(
        fs::read_to_string(original.join("SKILL.md"))
            .unwrap()
            .contains("Original.")
    );
}

#[cfg(unix)]
#[test]
fn adoption_refuses_a_symlink_with_different_bytes_without_changing_other_targets() {
    let (root, host) = fixture();
    let original = root.path().join("user-beta");
    fs::create_dir_all(&original).unwrap();
    fs::write(
        original.join("SKILL.md"),
        "---\nname: beta\ndescription: User.\n---\nUser edits.\n",
    )
    .unwrap();
    let target = root.path().join("project/.agents/skills");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&original, target.join("beta")).unwrap();
    let result = command(&host, &["skilld", "sync", "--adopt"]);
    assert_ne!(result.0, 0);
    assert_eq!(fs::canonicalize(target.join("beta")).unwrap(), original);
    assert!(!target.join("alpha").exists());
}

#[test]
fn check_detects_a_missing_agent_target_and_sync_restores_it() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    let target = root.path().join("project/.agents/skills/alpha");
    fs::remove_file(&target).unwrap();
    assert_eq!(
        command(&host, &["skilld", "sync", "--check", "--json"]).0,
        1
    );
    assert!(!target.exists());
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    assert!(target.join("SKILL.md").exists());
}

struct Hosted {
    calls: AtomicUsize,
    fail_beta: bool,
}

impl RemoteProvider for Hosted {
    fn search(&self, _: &str, _: u8) -> Result<SearchResponse, RemoteError> {
        unreachable!()
    }
    fn prepare(&self, _: &RemoteSelector, _: bool) -> Result<PreparedRemoteSkill, RemoteError> {
        unreachable!()
    }
    fn source_state(
        &self,
        _: &RemoteSelector,
        _: &str,
        _: &str,
    ) -> Result<RemoteSourceState, RemoteError> {
        unreachable!()
    }
    fn latest_commit(
        &self,
        _: &RemoteSelector,
        _: bool,
    ) -> Result<RemoteLatestCommit, RemoteError> {
        unreachable!()
    }
    fn compare_updates(
        &self,
        _: &[RemoteUpdateComparison],
    ) -> Result<Vec<RemoteUpdateResult>, RemoteError> {
        unreachable!()
    }
    fn prepare_exact(
        &self,
        selector: &RemoteSelector,
        commit: &CommitSha,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        assert!(!direct);
        self.calls.fetch_add(1, Ordering::SeqCst);
        let name = match &selector.source().selector {
            SourceSelector::NamedSkill { name } => name.as_str(),
            SourceSelector::Path { path } => path.rsplit('/').next().unwrap(),
        };
        if self.fail_beta && name == "beta" {
            return Err(RemoteError::new(
                "SOURCE_NOT_FOUND",
                "private source unavailable",
            ));
        }
        let file = PreparedFile {
            path: "SKILL.md".into(),
            mode: 0o644,
            bytes: format!("---\nname: {name}\ndescription: Hosted.\n---\nHosted instructions.\n")
                .into_bytes(),
        };
        let mut digest = Sha256::new();
        digest.update((file.path.len() as u64).to_be_bytes());
        digest.update(file.path.as_bytes());
        digest.update((file.bytes.len() as u64).to_be_bytes());
        digest.update(&file.bytes);
        Ok(PreparedRemoteSkill {
            files: vec![file],
            locked_source: LockedSource::Remote {
                source: selector.canonical(),
                commit_sha: commit.as_str().into(),
                skill_path: format!("skills/{name}"),
            },
            source_status: SourceStatus::Verified {
                artifact_id: format!("artifact-{name}"),
                content_sha256: "a".repeat(64),
                installed_sha256: digest
                    .finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                attestation_key_id: "key".into(),
            },
            page_url: None,
            omitted_files: Vec::new(),
            behavior_readings: Vec::new(),
        })
    }
}

fn hosted_fixture(fail_beta: bool) -> (tempfile::TempDir, LocalHost, Arc<Hosted>) {
    let (root, host) = fixture();
    let path = root.path().join("project/.skills/skilld.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for name in ["alpha", "beta"] {
        manifest["skills"][name]["source"] = json!(format!(
            "github:owner/private/skills/{name}#commit:{}",
            "a".repeat(40)
        ));
    }
    fs::write(path, manifest.to_string()).unwrap();
    let remote = Arc::new(Hosted {
        calls: AtomicUsize::new(0),
        fail_beta,
    });
    (root, host.with_remote_provider(remote.clone()), remote)
}

#[test]
fn pinned_hosted_sync_repeats_offline_and_restores_missing_targets_without_fetching() {
    let (root, host, remote) = hosted_fixture(false);
    let first = command(&host, &["skilld", "sync"]);
    assert_eq!(first.0, 0, "{}", first.2);
    assert_eq!(remote.calls.load(Ordering::SeqCst), 2);
    assert_eq!(command(&host, &["skilld", "sync", "--check"]).0, 0);
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    fs::remove_file(root.path().join("project/.agents/skills/alpha")).unwrap();
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    assert_eq!(remote.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn shared_hosted_skills_reuse_the_pin_and_reject_a_conflicting_pin_without_fetching() {
    let (root, host, remote) = hosted_fixture(false);
    let source = format!(
        "github:owner/private/skills/alpha#commit:{}",
        "a".repeat(40)
    );
    let one = declaration(root.path(), "one", &["codex"], &source, "symlink");
    let two = declaration(root.path(), "two", &["claude-code"], &source, "copy");
    for manifest in [&one, &two] {
        assert_eq!(
            command(&host, &["skilld", "sync", "--manifest", manifest]).0,
            0
        );
        assert_eq!(
            command(
                &host,
                &["skilld", "sync", "--manifest", manifest, "--check"]
            )
            .0,
            0
        );
    }
    assert_eq!(remote.calls.load(Ordering::SeqCst), 1);
    let old_lock = fs::read(root.path().join("project/.skills/skilld-lock.yaml")).unwrap();
    let other_source = format!(
        "github:owner/private/skills/alpha#commit:{}",
        "b".repeat(40)
    );
    declaration(root.path(), "two", &["claude-code"], &other_source, "copy");
    for extra in [vec![], vec!["--check"]] {
        let mut args = vec!["skilld", "sync", "--manifest", &two];
        args.extend(extra);
        let result = command(&host, &args);
        assert_ne!(result.0, 0);
        assert!(result.2.contains("one"), "{}", result.2);
    }
    assert_eq!(remote.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(root.path().join("project/.skills/skilld-lock.yaml")).unwrap(),
        old_lock
    );
    for target in [".agents/skills", ".claude/skills"] {
        assert!(
            root.path()
                .join("project")
                .join(target)
                .join("alpha/SKILL.md")
                .is_file()
        );
    }
}

#[test]
fn a_hosted_preparation_failure_leaves_every_agent_unchanged() {
    let (root, host, _) = hosted_fixture(true);
    let result = command(&host, &["skilld", "sync"]);
    assert_ne!(result.0, 0);
    assert!(result.2.contains("SOURCE_NOT_FOUND"), "{}", result.2);
    assert!(!root.path().join("project/.agents/skills/alpha").exists());
    assert!(
        !root
            .path()
            .join("project/.skills/skilld-lock.yaml")
            .exists()
    );
}

fn declare_only_alpha(root: &Path) {
    let manifest = root.join("project/.skills/skilld.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["skills"] = json!({"alpha": {"source": "../sources/alpha"}});
    value["requires"] = json!({"pr": ["alpha"]});
    fs::write(manifest, value.to_string()).unwrap();
}

fn lock(lockfile: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(lockfile).unwrap()).unwrap()
}

/// Rewrite one lockfile entry as an older CLI or `skilld install` records it.
fn forget_installer(lockfile: &Path, name: &str) {
    let mut value = lock(lockfile);
    value["skills"][name]
        .as_object_mut()
        .unwrap()
        .remove("installedBy")
        .unwrap();
    fs::write(lockfile, value.to_string()).unwrap();
}

fn locked_skills(lockfile: &Path) -> Vec<String> {
    let lock: serde_json::Value = serde_json::from_slice(&fs::read(lockfile).unwrap()).unwrap();
    lock["skills"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

const AGENT_TARGETS: [&str; 3] = [".agents/skills", ".claude/skills", ".opencode/skills"];

#[test]
fn check_names_each_undeclared_skill_and_sync_removes_it() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    declare_only_alpha(root.path());
    let project = root.path().join("project");
    let lockfile = project.join(".skills/skilld-lock.yaml");
    let before = fs::read(&lockfile).unwrap();

    let checked = command(&host, &["skilld", "sync", "--check"]);
    assert_eq!(checked.0, 1, "{}", checked.2);
    assert!(
        checked.1.contains("Skill beta is no longer declared."),
        "{}",
        checked.1
    );
    let json = command(&host, &["skilld", "sync", "--check", "--json"]);
    assert_eq!(json.0, 1, "{}", json.2);
    let report: serde_json::Value = serde_json::from_str(&json.1).unwrap();
    assert_eq!(report["data"]["current"], false);
    assert_eq!(
        report["data"]["undeclared"],
        json!([{"_tag": "Removable", "name": "beta"}])
    );
    assert_eq!(fs::read(&lockfile).unwrap(), before);
    assert!(project.join(".skills/beta/SKILL.md").is_file());
    for target in AGENT_TARGETS {
        assert!(project.join(target).join("beta/SKILL.md").is_file());
    }

    let synced = command(&host, &["skilld", "sync"]);
    assert_eq!(synced.0, 0, "{}", synced.2);
    assert!(
        synced
            .1
            .contains("Removed Skill beta. It is no longer declared."),
        "{}",
        synced.1
    );
    assert!(!project.join(".skills/beta").exists());
    for target in AGENT_TARGETS {
        assert!(fs::symlink_metadata(project.join(target).join("beta")).is_err());
        assert!(project.join(target).join("alpha/SKILL.md").is_file());
    }
    assert_eq!(locked_skills(&lockfile), ["alpha"]);
    assert_eq!(command(&host, &["skilld", "sync", "--check"]).0, 0);
}

#[test]
fn sync_keeps_and_reports_a_skill_that_skilld_install_added_to_its_store() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync", "--global"]).0, 0);
    let gamma = root.path().join("project/sources/gamma");
    fs::create_dir_all(&gamma).unwrap();
    fs::write(
        gamma.join("SKILL.md"),
        "---\nname: gamma\ndescription: Test.\n---\nOriginal.\n",
    )
    .unwrap();
    let installed = command(
        &host,
        &[
            "skilld",
            "install",
            "./sources/gamma",
            "--global",
            "--agent",
            "codex",
        ],
    );
    assert_eq!(installed.0, 0, "{}", installed.2);
    let data_root = root.path().join("global");
    let lockfile = data_root.join("skills/skilld-lock.yaml");
    let locked = lock(&lockfile);
    assert_eq!(locked["skills"]["alpha"]["installedBy"], "sync");
    assert!(locked["skills"]["gamma"].get("installedBy").is_none());
    let remove = format!(
        "SKILLD_DATA_DIR={} skilld remove gamma --global",
        data_root.display()
    );

    let checked = command(&host, &["skilld", "sync", "--global", "--check"]);
    assert_eq!(checked.0, 0, "{}", checked.2);
    assert!(
        checked
            .1
            .contains("Skill gamma is not declared. Sync did not install it, so sync keeps it."),
        "{}",
        checked.1
    );
    assert!(
        checked.1.contains(&format!("To remove it, run: {remove}")),
        "{}",
        checked.1
    );
    let json = command(&host, &["skilld", "sync", "--global", "--check", "--json"]);
    assert_eq!(json.0, 0, "{}", json.2);
    let report: serde_json::Value = serde_json::from_str(&json.1).unwrap();
    assert_eq!(report["data"]["current"], true);
    assert_eq!(
        report["data"]["undeclared"],
        json!([{"_tag": "Kept", "name": "gamma", "removeCommand": remove}])
    );

    let synced = command(&host, &["skilld", "sync", "--global"]);
    assert_eq!(synced.0, 0, "{}", synced.2);
    assert!(synced.1.contains("Sync did not install it"), "{}", synced.1);
    assert!(data_root.join("skills/gamma/SKILL.md").is_file());
    assert!(root.path().join(".agents/skills/gamma/SKILL.md").is_file());
    assert_eq!(locked_skills(&lockfile), ["alpha", "beta", "gamma"]);

    let removed = command(&host, &["skilld", "remove", "gamma", "--global"]);
    assert_eq!(removed.0, 0, "{}", removed.2);
    assert!(!data_root.join("skills/gamma").exists());
    assert!(fs::symlink_metadata(root.path().join(".agents/skills/gamma")).is_err());
}

#[test]
fn sync_keeps_and_reports_an_undeclared_entry_that_an_older_sync_wrote() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    let project = root.path().join("project");
    let lockfile = project.join(".skills/skilld-lock.yaml");
    forget_installer(&lockfile, "beta");
    declare_only_alpha(root.path());

    let checked = command(&host, &["skilld", "sync", "--check"]);
    assert!(
        checked
            .1
            .contains("Skill beta is not declared. Sync did not install it, so sync keeps it."),
        "{}",
        checked.1
    );
    assert!(
        checked.1.contains("To remove it, run: skilld remove beta"),
        "{}",
        checked.1
    );
    assert!(!checked.1.contains("no longer declared"), "{}", checked.1);
    let synced = command(&host, &["skilld", "sync"]);
    assert_eq!(synced.0, 0, "{}", synced.2);
    assert!(project.join(".skills/beta/SKILL.md").is_file());
    for target in AGENT_TARGETS {
        assert!(project.join(target).join("beta/SKILL.md").is_file());
    }
    assert_eq!(locked_skills(&lockfile), ["alpha", "beta"]);
    assert_eq!(command(&host, &["skilld", "sync", "--check"]).0, 0);

    let removed = command(&host, &["skilld", "remove", "beta"]);
    assert_eq!(removed.0, 0, "{}", removed.2);
    assert!(!project.join(".skills/beta").exists());
}

#[test]
fn sync_records_its_installer_on_declared_entries_that_an_older_sync_wrote() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    let lockfile = root.path().join("project/.skills/skilld-lock.yaml");
    for name in ["alpha", "beta"] {
        forget_installer(&lockfile, name);
    }
    let before = fs::read(&lockfile).unwrap();
    assert_eq!(command(&host, &["skilld", "sync", "--check"]).0, 0);
    assert_eq!(fs::read(&lockfile).unwrap(), before);

    let synced = command(&host, &["skilld", "sync"]);
    assert_eq!(synced.0, 0, "{}", synced.2);
    let locked = lock(&lockfile);
    for name in ["alpha", "beta"] {
        assert_eq!(locked["skills"][name]["installedBy"], "sync", "{name}");
    }
}

#[test]
fn sync_removes_the_store_entry_when_its_links_were_deleted_by_hand() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync", "--global"]).0, 0);
    declare_only_alpha(root.path());
    for target in [
        ".agents/skills",
        ".claude/skills",
        ".config/opencode/skills",
    ] {
        fs::remove_file(root.path().join(target).join("beta")).unwrap();
    }
    let checked = command(&host, &["skilld", "sync", "--global", "--check"]);
    assert_eq!(checked.0, 1, "{}", checked.2);
    assert!(checked.1.contains("Skill beta is no longer declared."));
    let synced = command(&host, &["skilld", "sync", "--global"]);
    assert_eq!(synced.0, 0, "{}", synced.2);
    assert!(!root.path().join("global/skills/beta").exists());
    assert_eq!(
        locked_skills(&root.path().join("global/skills/skilld-lock.yaml")),
        ["alpha"]
    );
}

#[cfg(unix)]
#[test]
fn sync_keeps_agent_target_links_that_point_outside_its_store() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync", "--global"]).0, 0);
    let other = root.path().join("other/skills/beta");
    fs::create_dir_all(&other).unwrap();
    fs::write(
        other.join("SKILL.md"),
        "---\nname: beta\ndescription: Other.\n---\nOther store.\n",
    )
    .unwrap();
    let link = root.path().join(".claude/skills/beta");
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&other, &link).unwrap();
    declare_only_alpha(root.path());

    let synced = command(&host, &["skilld", "sync", "--global"]);
    assert_eq!(synced.0, 0, "{}", synced.2);
    assert_eq!(fs::read_link(&link).unwrap(), other);
    assert!(other.join("SKILL.md").is_file());
    for target in [".agents/skills", ".config/opencode/skills"] {
        assert!(fs::symlink_metadata(root.path().join(target).join("beta")).is_err());
    }
    assert!(!root.path().join("global/skills/beta").exists());
    assert_eq!(
        locked_skills(&root.path().join("global/skills/skilld-lock.yaml")),
        ["alpha"]
    );
}

#[test]
fn sync_removes_matching_copies_and_keeps_edited_copies_of_an_undeclared_skill() {
    let (root, host) = fixture();
    let manifest = root.path().join("project/.skills/skilld.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["mode"] = json!("copy");
    fs::write(&manifest, value.to_string()).unwrap();
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    let project = root.path().join("project");
    let edited = project.join(".claude/skills/beta/SKILL.md");
    fs::write(
        &edited,
        "---\nname: beta\ndescription: Test.\n---\nUser edits.\n",
    )
    .unwrap();
    declare_only_alpha(root.path());

    let synced = command(&host, &["skilld", "sync"]);
    assert_eq!(synced.0, 0, "{}", synced.2);
    assert!(fs::read_to_string(&edited).unwrap().contains("User edits."));
    for target in [".agents/skills", ".opencode/skills"] {
        assert!(!project.join(target).join("beta").exists());
    }
    assert!(!project.join(".skills/beta").exists());
}

#[test]
fn sync_keeps_skills_that_another_declaration_in_the_store_names() {
    let (root, host) = fixture();
    assert_eq!(command(&host, &["skilld", "sync"]).0, 0);
    let one = declaration(
        root.path(),
        "one",
        &["codex"],
        "../sources/alpha",
        "symlink",
    );
    let checked = command(&host, &["skilld", "sync", "--manifest", &one, "--check"]);
    assert!(!checked.1.contains("no longer declared"), "{}", checked.1);
    assert_eq!(command(&host, &["skilld", "sync", "--manifest", &one]).0, 0);
    let project = root.path().join("project");
    assert!(project.join(".skills/beta/SKILL.md").is_file());
    for target in AGENT_TARGETS {
        assert!(project.join(target).join("beta/SKILL.md").is_file());
    }
    assert_eq!(
        command(&host, &["skilld", "sync", "--manifest", &one, "--check"]).0,
        0
    );
    assert_eq!(command(&host, &["skilld", "sync", "--check"]).0, 0);
}
