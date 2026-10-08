use serde_json::json;
use sha2::{Digest, Sha256};
use skilld_command::doctor::DoctorOptions;
use skilld_command::doctor_actions::DoctorAction;
use skilld_command::doctor_metadata::fingerprint;
use skilld_command::{
    Host, LocalHost, PreparedRemoteSkill, RemoteLatestCommit, RemoteProvider, RemoteSourceState,
    RemoteUpdateComparison, RemoteUpdateResult,
};
use skilld_core::{
    CommitSha, LockedSource, PreparedFile, RemoteError, RemoteSelector, SearchResponse,
    SourceStatus,
};
use std::{fs, path::Path, sync::Arc};

struct Remote {
    text: String,
}
impl RemoteProvider for Remote {
    fn search(&self, _: &str, _: u8) -> Result<SearchResponse, RemoteError> {
        unreachable!()
    }
    fn prepare(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        assert!(!direct);
        let file = PreparedFile {
            path: "SKILL.md".into(),
            mode: 0o644,
            bytes: self.text.as_bytes().to_vec(),
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
                commit_sha: "1".repeat(40),
                skill_path: if selector.canonical().ends_with("/.") {
                    "."
                } else {
                    "skills/example"
                }
                .into(),
            },
            source_status: SourceStatus::Verified {
                artifact_id: format!("sha256:{}", "a".repeat(64)),
                content_sha256: "a".repeat(64),
                installed_sha256: digest
                    .finalize()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                attestation_key_id: "fixture".into(),
            },
            page_url: None,
            omitted_files: vec![],
            behavior_readings: vec![],
        })
    }
    fn prepare_exact(
        &self,
        _: &RemoteSelector,
        _: &CommitSha,
        _: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
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
}
const TEXT: &str = "---\nname: example\ndescription: Test.\n---\nRead the project.\n";
fn fixture() -> (tempfile::TempDir, LocalHost) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("app")).unwrap();
    fs::create_dir_all(root.path().join(".agents/skills/example")).unwrap();
    fs::write(root.path().join(".agents/skills/example/SKILL.md"), TEXT).unwrap();
    let tree = fingerprint(&root.path().join(".agents/skills/example"))
        .unwrap()
        .git_tree;
    fs::write(root.path().join(".agents/.skill-lock.json"),json!({"version":3,"dismissed":{"notice":true},"skills":{"example":{"source":"owner/repository","sourceType":"github","skillPath":"skills/example/SKILL.md","skillFolderHash":tree},"unrelated":{"source":"owner/other","sourceType":"github","skillFolderHash":""}}}).to_string()).unwrap();
    let host = LocalHost::new(root.path().join("app"), root.path().join(".skilld"))
        .with_remote_provider(Arc::new(Remote { text: TEXT.into() }));
    (root, host)
}
fn lock(root: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(root.join(".agents/.skill-lock.json")).unwrap()).unwrap()
}

#[test]
fn removal_backs_up_files_and_preserves_unrelated_foreign_metadata() {
    let (root, host) = fixture();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let done = host
        .doctor_plan(&report, 0, DoctorAction::Remove)
        .unwrap()
        .apply()
        .unwrap();
    assert!(!root.path().join(".agents/skills/example").exists());
    assert_eq!(
        fs::read_to_string(done.backup.join("target-0/SKILL.md")).unwrap(),
        TEXT
    );
    let value = lock(root.path());
    assert!(value["skills"].get("example").is_none());
    assert!(value["skills"].get("unrelated").is_some());
    assert_eq!(value["dismissed"]["notice"], true);
}

#[test]
fn changed_supporting_file_rejects_a_reviewed_removal() {
    let (root, host) = fixture();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Remove).unwrap();
    fs::write(
        root.path().join(".agents/skills/example/new.md"),
        "Keep this edit",
    )
    .unwrap();
    assert!(plan.apply().is_err());
    assert!(lock(root.path())["skills"].get("example").is_some());
    assert!(root.path().join(".agents/skills/example/new.md").is_file());
}

#[test]
fn concurrent_lock_edit_rejects_a_reviewed_removal() {
    let (root, host) = fixture();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Remove).unwrap();
    let mut value = lock(root.path());
    value["newField"] = json!("keep");
    fs::write(
        root.path().join(".agents/.skill-lock.json"),
        value.to_string(),
    )
    .unwrap();
    assert!(plan.apply().is_err());
    assert!(
        root.path()
            .join(".agents/skills/example/SKILL.md")
            .is_file()
    );
    assert_eq!(lock(root.path())["newField"], "keep");
}

#[cfg(unix)]
#[test]
fn migration_preserves_claude_link_and_canonical_copy_then_transfers_ownership() {
    let (root, host) = fixture();
    fs::create_dir_all(root.path().join(".claude/skills")).unwrap();
    std::os::unix::fs::symlink(
        root.path().join(".agents/skills/example"),
        root.path().join(".claude/skills/example"),
    )
    .unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Migrate).unwrap();
    assert!(!plan.preview.changes_content);
    let done = plan.apply().unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join(".claude/skills/example/SKILL.md")).unwrap(),
        TEXT
    );
    assert!(
        fs::symlink_metadata(root.path().join(".claude/skills/example"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        !fs::symlink_metadata(root.path().join(".agents/skills/example"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let managed: serde_json::Value = serde_json::from_slice(
        &fs::read(root.path().join(".skilld/skills/skilld-lock.yaml")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        managed["skills"]["example"]["source"]["commit_sha"],
        "1".repeat(40)
    );
    assert_eq!(
        managed["skills"]["example"]["targets"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(lock(root.path())["skills"].get("example").is_none());
    assert!(done.backup.join("skills-lock.json").is_file());
}

#[test]
fn migration_does_not_overwrite_unverified_content() {
    let (root, host) = fixture();
    fs::write(
        root.path().join(".agents/skills/example/local.md"),
        "Local edit",
    )
    .unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    assert!(host.doctor_plan(&report, 0, DoctorAction::Migrate).is_err());
    assert!(
        root.path()
            .join(".agents/skills/example/local.md")
            .is_file()
    );
}

#[test]
fn installation_failure_restores_all_foreign_files() {
    let (root, host) = fixture();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Migrate).unwrap();
    fs::create_dir_all(root.path().join(".skilld/skills")).unwrap();
    fs::write(
        root.path().join(".skilld/skills/skilld-lock.yaml"),
        "invalid concurrent state",
    )
    .unwrap();
    assert!(plan.apply().is_err());
    assert_eq!(
        fs::read_to_string(root.path().join(".agents/skills/example/SKILL.md")).unwrap(),
        TEXT
    );
    assert!(lock(root.path())["skills"].get("example").is_some());
}

#[cfg(unix)]
#[test]
fn differently_named_alias_never_removes_an_unrelated_target() {
    let (root, host) = fixture();
    fs::remove_file(root.path().join(".agents/.skill-lock.json")).unwrap();
    fs::create_dir_all(root.path().join(".claude/skills/example")).unwrap();
    fs::write(
        root.path().join(".claude/skills/example/SKILL.md"),
        "Unrelated personal Skill",
    )
    .unwrap();
    std::os::unix::fs::symlink(
        root.path().join(".agents/skills/example"),
        root.path().join(".claude/skills/alias"),
    )
    .unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let index = report
        .skills
        .iter()
        .position(|s| s.canonical_path == root.path().join(".agents/skills/example"))
        .unwrap();
    let result = host.doctor_plan(&report, index, DoctorAction::Remove);
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(root.path().join(".claude/skills/example/SKILL.md")).unwrap(),
        "Unrelated personal Skill"
    );
}

#[cfg(unix)]
#[test]
fn retargeted_agent_root_rejects_stale_physical_paths() {
    use std::os::unix::fs::symlink;
    let (root, host) = fixture();
    for dir in ["old", "new"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
        symlink(
            root.path().join(".agents/skills/example"),
            root.path().join(dir).join("example"),
        )
        .unwrap();
    }
    fs::create_dir_all(root.path().join(".claude")).unwrap();
    symlink(root.path().join("old"), root.path().join(".claude/skills")).unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Remove).unwrap();
    fs::remove_file(root.path().join(".claude/skills")).unwrap();
    symlink(root.path().join("new"), root.path().join(".claude/skills")).unwrap();
    fs::remove_file(root.path().join("old/example")).unwrap();
    fs::create_dir(root.path().join("old/example")).unwrap();
    fs::write(root.path().join("old/example/keep.md"), "Unrelated").unwrap();
    assert!(plan.apply().is_err());
    assert!(root.path().join("old/example/keep.md").is_file());
    assert!(lock(root.path())["skills"].get("example").is_some());
}

#[test]
fn migration_preserves_recorded_source_ref() {
    let (root, host) = fixture();
    let mut value = lock(root.path());
    value["skills"]["example"]["ref"] = json!("release/v1");
    fs::write(
        root.path().join(".agents/.skill-lock.json"),
        value.to_string(),
    )
    .unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Migrate).unwrap();
    assert!(
        plan.preview
            .source
            .as_deref()
            .unwrap()
            .contains("release/v1")
    );
}

#[test]
fn unscanned_subagent_targets_block_foreign_lock_removal() {
    let (root, host) = fixture();
    let mut value = lock(root.path());
    value["skills"]["example"]["subagents"] = json!(["research"]);
    fs::write(
        root.path().join(".agents/.skill-lock.json"),
        value.to_string(),
    )
    .unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    assert!(host.doctor_plan(&report, 0, DoctorAction::Remove).is_err());
    assert!(lock(root.path())["skills"].get("example").is_some());
}

#[test]
fn newly_added_agent_target_rejects_stale_removal() {
    let (root, host) = fixture();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Remove).unwrap();
    fs::create_dir_all(root.path().join(".claude/skills/example")).unwrap();
    fs::write(root.path().join(".claude/skills/example/SKILL.md"), TEXT).unwrap();
    assert!(plan.apply().is_err());
    assert!(
        root.path()
            .join(".agents/skills/example/SKILL.md")
            .is_file()
    );
    assert!(lock(root.path())["skills"].get("example").is_some());
}

#[test]
fn newly_added_managed_skill_rejects_stale_migration() {
    let (root, host) = fixture();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Migrate).unwrap();
    fs::create_dir_all(root.path().join(".skilld/skills/example")).unwrap();
    fs::write(
        root.path().join(".skilld/skills/example/SKILL.md"),
        "Keep this",
    )
    .unwrap();
    assert!(plan.apply().is_err());
    assert_eq!(
        fs::read_to_string(root.path().join(".skilld/skills/example/SKILL.md")).unwrap(),
        "Keep this"
    );
    assert!(
        root.path()
            .join(".agents/skills/example/SKILL.md")
            .is_file()
    );
}

#[test]
fn migration_accepts_a_repository_root_skill() {
    let (root, host) = fixture();
    let mut value = lock(root.path());
    value["skills"]["example"]["skillPath"] = json!("SKILL.md");
    value["skills"]["example"]["source"] = json!("owner/example");
    fs::write(
        root.path().join(".agents/.skill-lock.json"),
        value.to_string(),
    )
    .unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let plan = host.doctor_plan(&report, 0, DoctorAction::Migrate).unwrap();
    assert!(plan.preview.source.as_deref().unwrap().ends_with("/."));
    plan.apply().unwrap();
    assert!(
        root.path()
            .join(".skilld/skills/example/SKILL.md")
            .is_file()
    );
    host.remove("example", skilld_core::InstallScope::Global)
        .unwrap();
    assert!(!root.path().join(".agents/skills/example").exists());
}

#[test]
fn legacy_codex_target_can_be_removed_but_cannot_be_migrated() {
    let (root, host) = fixture();
    fs::create_dir_all(root.path().join(".codex/skills")).unwrap();
    fs::rename(
        root.path().join(".agents/skills/example"),
        root.path().join(".codex/skills/example"),
    )
    .unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    assert!(host.doctor_plan(&report, 0, DoctorAction::Migrate).is_err());
    host.doctor_plan(&report, 0, DoctorAction::Remove)
        .unwrap()
        .apply()
        .unwrap();
    assert!(!root.path().join(".codex/skills/example").exists());
}

#[test]
fn source_check_reports_exact_contents_and_resolved_commit_without_mutation() {
    let (root, host) = fixture();
    let before = fs::read(root.path().join(".agents/.skill-lock.json")).unwrap();
    let mut options = DoctorOptions::for_root(root.path());
    options.check_sources = true;
    let report = host.doctor_scan(&options).unwrap();
    let matched = report.skills[0].source_match.as_ref().unwrap();
    assert_eq!(matched.result, "Exact directory match.");
    assert_eq!(
        matched.commit.as_deref(),
        Some("1111111111111111111111111111111111111111")
    );
    assert_eq!(
        fs::read(root.path().join(".agents/.skill-lock.json")).unwrap(),
        before
    );
    assert!(!root.path().join(".skilld/skills").exists());
}

#[cfg(unix)]
fn linked_source_fixture() -> (tempfile::TempDir, LocalHost) {
    let (root, host) = fixture();
    fs::remove_file(root.path().join(".agents/.skill-lock.json")).unwrap();
    fs::create_dir_all(root.path().join("source")).unwrap();
    fs::rename(
        root.path().join(".agents/skills/example"),
        root.path().join("source/example"),
    )
    .unwrap();
    fs::create_dir_all(root.path().join(".claude/skills")).unwrap();
    std::os::unix::fs::symlink(
        "../../source/example",
        root.path().join(".claude/skills/example"),
    )
    .unwrap();
    (root, host)
}

#[cfg(unix)]
#[test]
fn removing_only_an_agent_link_preserves_its_source_directory() {
    let (root, host) = linked_source_fixture();
    let source = root.path().join("source/example");
    let link = root.path().join(".claude/skills/example");
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let index = report
        .skills
        .iter()
        .position(|s| s.canonical_path == source)
        .unwrap();
    let plan = host
        .doctor_plan(&report, index, DoctorAction::Remove)
        .unwrap();
    assert_eq!(plan.preview.paths, vec![link.clone()]);
    let done = plan.apply().unwrap();
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read_to_string(source.join("SKILL.md")).unwrap(), TEXT);
    assert_eq!(
        fs::read_link(done.backup.join("target-0")).unwrap(),
        Path::new("../../source/example")
    );
}

#[cfg(unix)]
#[test]
fn retargeting_a_direct_skill_link_rejects_the_reviewed_removal() {
    let (root, host) = linked_source_fixture();
    let source = root.path().join("source/example");
    let link = root.path().join(".claude/skills/example");
    let other = root.path().join("source/other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("SKILL.md"), TEXT).unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let index = report
        .skills
        .iter()
        .position(|s| s.canonical_path == source)
        .unwrap();
    let plan = host
        .doctor_plan(&report, index, DoctorAction::Remove)
        .unwrap();
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("../../source/other", &link).unwrap();
    let error = plan.apply().unwrap_err();
    assert!(error.to_string().contains("link changed after review"));
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("../../source/other")
    );
    assert_eq!(fs::read_to_string(source.join("SKILL.md")).unwrap(), TEXT);
    assert_eq!(fs::read_to_string(other.join("SKILL.md")).unwrap(), TEXT);
}

#[cfg(unix)]
#[test]
fn agent_root_aliases_backup_the_same_physical_skill_only_once() {
    let (root, host) = fixture();
    fs::create_dir_all(root.path().join(".claude")).unwrap();
    std::os::unix::fs::symlink("../.agents/skills", root.path().join(".claude/skills")).unwrap();
    let physical = root.path().join(".agents/skills/example");
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let index = report
        .skills
        .iter()
        .position(|s| s.canonical_path == physical)
        .unwrap();
    let plan = host
        .doctor_plan(&report, index, DoctorAction::Remove)
        .unwrap();
    assert_eq!(plan.preview.paths, vec![physical.clone()]);
    let done = plan.apply().unwrap();
    assert!(fs::symlink_metadata(physical).is_err());
    assert_eq!(
        fs::read_to_string(done.backup.join("target-0/SKILL.md")).unwrap(),
        TEXT
    );
    assert!(fs::symlink_metadata(done.backup.join("target-1")).is_err());
    assert_eq!(
        fs::read_link(root.path().join(".claude/skills")).unwrap(),
        Path::new("../.agents/skills")
    );
    assert!(lock(root.path())["skills"].get("example").is_none());
}

#[cfg(unix)]
#[test]
fn removing_a_child_through_a_linked_root_preserves_the_root_link() {
    let (root, host) = fixture();
    fs::remove_file(root.path().join(".agents/.skill-lock.json")).unwrap();
    fs::remove_dir_all(root.path().join(".agents/skills/example")).unwrap();
    let shared = root.path().join("shared");
    let physical = shared.join("example");
    fs::create_dir_all(&physical).unwrap();
    fs::write(physical.join("SKILL.md"), TEXT).unwrap();
    fs::write(shared.join("keep.md"), "Keep this sibling").unwrap();
    fs::create_dir_all(root.path().join(".claude")).unwrap();
    let linked_root = root.path().join(".claude/skills");
    std::os::unix::fs::symlink("../shared", &linked_root).unwrap();
    let report = host
        .doctor_scan(&DoctorOptions::for_root(root.path()))
        .unwrap();
    let index = report
        .skills
        .iter()
        .position(|s| s.canonical_path == physical)
        .unwrap();
    let plan = host
        .doctor_plan(&report, index, DoctorAction::Remove)
        .unwrap();
    assert_eq!(plan.preview.paths, vec![physical.clone()]);
    let done = plan.apply().unwrap();
    assert!(fs::symlink_metadata(physical).is_err());
    assert_eq!(fs::read_link(&linked_root).unwrap(), Path::new("../shared"));
    assert_eq!(
        fs::read_to_string(shared.join("keep.md")).unwrap(),
        "Keep this sibling"
    );
    assert_eq!(
        fs::read_to_string(done.backup.join("target-0/SKILL.md")).unwrap(),
        TEXT
    );
}
