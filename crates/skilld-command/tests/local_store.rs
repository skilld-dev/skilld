use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use skilld_command::{
    LocalStore, PreparedStoreUpdate, ResolvedTarget, StoreError, TargetInstall, TransactionGate,
};
use skilld_core::{AgentTargetId, InstallMode, LockedSource, SkillName};

fn source(root: &Path, parent: &str, content: &str) -> PathBuf {
    named_source(root, parent, "example", content)
}

fn named_source(root: &Path, parent: &str, name: &str, content: &str) -> PathBuf {
    let path = root.join(parent).join(name);
    fs::create_dir_all(path.join("references")).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Test fixture.\n---\n\n{content}\n"),
    )
    .unwrap();
    fs::write(path.join("references/check.md"), "check").unwrap();
    path
}

#[test]
fn multi_skill_batch_commits_every_skill() {
    let temporary = tempfile::tempdir().unwrap();
    let alpha = named_source(temporary.path(), "old-alpha", "alpha", "old alpha");
    let beta = named_source(temporary.path(), "old-beta", "beta", "old beta");
    let new_alpha = named_source(temporary.path(), "new-alpha", "alpha", "new alpha");
    let new_beta = named_source(temporary.path(), "new-beta", "beta", "new beta");
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(
        AgentTargetId::Codex,
        temporary.path().join("project/.agents/skills"),
    );
    let install = TargetInstall {
        target: target.clone(),
        mode: InstallMode::Copy,
    };
    for source in [&alpha, &beta] {
        store
            .install_from(
                source,
                local_source(source),
                std::slice::from_ref(&install),
                std::slice::from_ref(&target),
            )
            .unwrap();
    }
    let updated = store
        .apply_update_batch(
            vec![
                PreparedStoreUpdate {
                    source: new_alpha,
                    locked_source: local_source(&alpha),
                    source_status: None,
                    targets: vec![install.clone()],
                },
                PreparedStoreUpdate {
                    source: new_beta,
                    locked_source: local_source(&beta),
                    source_status: None,
                    targets: vec![install],
                },
            ],
            std::slice::from_ref(&target),
        )
        .unwrap();

    assert_eq!(
        updated.iter().map(SkillName::as_str).collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert_eq!(
        fs::read_to_string(store.root().join("alpha/SKILL.md")).unwrap(),
        "---\nname: alpha\ndescription: Test fixture.\n---\n\nnew alpha\n"
    );
    assert_eq!(
        fs::read_to_string(target.root.join("beta/SKILL.md")).unwrap(),
        "---\nname: beta\ndescription: Test fixture.\n---\n\nnew beta\n"
    );
}

#[test]
fn multi_skill_batch_rolls_every_skill_back_when_the_lock_write_fails() {
    let temporary = tempfile::tempdir().unwrap();
    let alpha = named_source(temporary.path(), "old-alpha", "alpha", "old alpha");
    let beta = named_source(temporary.path(), "old-beta", "beta", "old beta");
    let new_alpha = named_source(temporary.path(), "new-alpha", "alpha", "new alpha");
    let new_beta = named_source(temporary.path(), "new-beta", "beta", "new beta");
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(
        AgentTargetId::Codex,
        temporary.path().join("project/.agents/skills"),
    );
    let install = TargetInstall {
        target: target.clone(),
        mode: InstallMode::Copy,
    };
    for source in [&alpha, &beta] {
        store
            .install_from(
                source,
                local_source(source),
                std::slice::from_ref(&install),
                std::slice::from_ref(&target),
            )
            .unwrap();
    }
    let before_lock = fs::read(store.root().join("skilld-lock.yaml")).unwrap();

    let error = store
        .apply_update_batch_with_gate(
            vec![
                PreparedStoreUpdate {
                    source: new_alpha,
                    locked_source: local_source(&alpha),
                    source_status: None,
                    targets: vec![install.clone()],
                },
                PreparedStoreUpdate {
                    source: new_beta,
                    locked_source: local_source(&beta),
                    source_status: None,
                    targets: vec![install],
                },
            ],
            std::slice::from_ref(&target),
            &RejectLockCommit,
        )
        .unwrap_err();

    assert_eq!(error.to_string(), "injected lock failure");
    for name in ["alpha", "beta"] {
        let expected =
            format!("---\nname: {name}\ndescription: Test fixture.\n---\n\nold {name}\n");
        assert_eq!(
            fs::read_to_string(store.root().join(name).join("SKILL.md")).unwrap(),
            expected
        );
        assert_eq!(
            fs::read_to_string(target.root.join(name).join("SKILL.md")).unwrap(),
            expected
        );
    }
    assert_eq!(
        fs::read(store.root().join("skilld-lock.yaml")).unwrap(),
        before_lock
    );
}

fn skill_text(content: &str) -> String {
    format!("---\nname: example\ndescription: Test fixture.\n---\n\n{content}\n")
}

fn resolved(agent: AgentTargetId, root: PathBuf) -> ResolvedTarget {
    ResolvedTarget::new(agent, root).unwrap()
}

fn local_source(path: &Path) -> LockedSource {
    LockedSource::Local {
        path: path.to_str().unwrap().to_owned(),
    }
}

#[test]
fn copy_install_lists_views_and_removes_managed_state() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = source(temporary.path(), "source", "first");
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(
        AgentTargetId::Codex,
        temporary.path().join("project/.agents/skills"),
    );
    let install = TargetInstall {
        target: target.clone(),
        mode: InstallMode::Copy,
    };

    let installed = store
        .install_from(
            &skill,
            local_source(&skill),
            &[install],
            std::slice::from_ref(&target),
        )
        .unwrap();

    assert_eq!(installed.as_str(), "example");
    assert_eq!(
        store.list(std::slice::from_ref(&target)).unwrap(),
        ["example"]
    );
    let view = store
        .view(&installed, std::slice::from_ref(&target))
        .unwrap();
    assert_eq!(view.name, "example");
    assert_eq!(view.skill.source_status.as_str(), "local");
    assert_eq!(view.skill.targets[0].agent, AgentTargetId::Codex);
    assert_eq!(
        fs::read_to_string(target.root.join("example/SKILL.md")).unwrap(),
        skill_text("first")
    );

    store
        .remove(&installed, std::slice::from_ref(&target))
        .unwrap();

    assert!(
        store
            .list(std::slice::from_ref(&target))
            .unwrap()
            .is_empty()
    );
    assert!(!store.root().join("example").exists());
    assert!(!target.root.join("example").exists());
}

#[cfg(unix)]
#[test]
fn symlink_install_points_at_the_canonical_skill() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = source(temporary.path(), "source", "linked");
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(
        AgentTargetId::Cursor,
        temporary.path().join("project/.cursor/skills"),
    );

    store
        .install_from(
            &skill,
            local_source(&skill),
            &[TargetInstall {
                target: target.clone(),
                mode: InstallMode::Symlink,
            }],
            std::slice::from_ref(&target),
        )
        .unwrap();

    let destination = target.root.join("example");
    assert!(
        fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(destination.join("SKILL.md")).unwrap(),
        skill_text("linked")
    );
}

#[test]
fn duplicate_agent_paths_apply_one_filesystem_change() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = source(temporary.path(), "source", "shared");
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let shared = temporary.path().join("project/.agents/skills");
    let codex = resolved(AgentTargetId::Codex, shared.clone());
    let amp = resolved(AgentTargetId::Amp, shared.clone());

    let name = store
        .install_from(
            &skill,
            local_source(&skill),
            &[
                TargetInstall {
                    target: codex.clone(),
                    mode: InstallMode::Copy,
                },
                TargetInstall {
                    target: amp.clone(),
                    mode: InstallMode::Copy,
                },
            ],
            &[codex.clone(), amp.clone()],
        )
        .unwrap();

    let view = store.view(&name, &[codex, amp]).unwrap();
    assert_eq!(view.skill.targets.len(), 2);
    assert_eq!(
        fs::read_to_string(shared.join("example/SKILL.md")).unwrap(),
        skill_text("shared")
    );
}

struct RejectLockCommit;

impl TransactionGate for RejectLockCommit {
    fn before_lock_commit(&self, _lockfile: &Path) -> Result<(), StoreError> {
        Err(StoreError::Filesystem("injected lock failure".to_owned()))
    }
}

#[test]
fn a_lock_commit_failure_restores_files_and_the_previous_lock() {
    let temporary = tempfile::tempdir().unwrap();
    let first = source(temporary.path(), "first", "first");
    let second = source(temporary.path(), "second", "second");
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(
        AgentTargetId::Codex,
        temporary.path().join("project/.agents/skills"),
    );
    let install = TargetInstall {
        target: target.clone(),
        mode: InstallMode::Copy,
    };
    store
        .install_from(
            &first,
            local_source(&first),
            std::slice::from_ref(&install),
            std::slice::from_ref(&target),
        )
        .unwrap();
    let before = fs::read(store.root().join("skilld-lock.yaml")).unwrap();

    let error = store
        .install_from_with_gate(
            &second,
            local_source(&second),
            &[install],
            std::slice::from_ref(&target),
            &RejectLockCommit,
        )
        .unwrap_err();

    assert_eq!(error.to_string(), "injected lock failure");
    assert_eq!(
        fs::read_to_string(store.root().join("example/SKILL.md")).unwrap(),
        skill_text("first")
    );
    assert_eq!(
        fs::read_to_string(target.root.join("example/SKILL.md")).unwrap(),
        skill_text("first")
    );
    assert_eq!(
        fs::read(store.root().join("skilld-lock.yaml")).unwrap(),
        before
    );
}

struct BlockingPanic {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

impl TransactionGate for BlockingPanic {
    fn before_lock_commit(&self, _lockfile: &Path) -> Result<(), StoreError> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        panic!("simulated process interruption");
    }
}

#[test]
fn a_waiting_install_recovers_an_interrupted_transaction() {
    let temporary = tempfile::tempdir().unwrap();
    let first = source(temporary.path(), "first", "first");
    let interrupted = source(temporary.path(), "interrupted", "interrupted");
    let final_source = source(temporary.path(), "final", "final");
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(
        AgentTargetId::Codex,
        temporary.path().join("project/.agents/skills"),
    );
    let install = TargetInstall {
        target: target.clone(),
        mode: InstallMode::Copy,
    };
    store
        .install_from(
            &first,
            local_source(&first),
            std::slice::from_ref(&install),
            std::slice::from_ref(&target),
        )
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let interrupted_store = store.clone();
    let interrupted_target = target.clone();
    let interrupted_install = install.clone();
    let first_thread = thread::spawn(move || {
        interrupted_store.install_from_with_gate(
            &interrupted,
            local_source(&interrupted),
            &[interrupted_install],
            &[interrupted_target],
            &BlockingPanic {
                entered: entered_tx,
                release: release_rx,
            },
        )
    });
    entered_rx.recv().unwrap();
    let waiting_store = store.clone();
    let waiting_target = target.clone();
    let (done_tx, done_rx) = mpsc::channel();
    let second_thread = thread::spawn(move || {
        let result = waiting_store.install_from(
            &final_source,
            local_source(&final_source),
            &[TargetInstall {
                target: waiting_target.clone(),
                mode: InstallMode::Copy,
            }],
            &[waiting_target],
        );
        done_tx.send(result).unwrap();
    });

    assert!(matches!(
        done_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release_tx.send(()).unwrap();
    assert!(first_thread.join().is_err());
    assert!(
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_ok()
    );
    second_thread.join().unwrap();
    assert_eq!(
        fs::read_to_string(store.root().join("example/SKILL.md")).unwrap(),
        skill_text("final")
    );
}

#[test]
fn target_paths_reject_parent_traversal() {
    let error = ResolvedTarget::new(
        AgentTargetId::Codex,
        PathBuf::from("/tmp/project/../outside"),
    )
    .unwrap_err();

    assert_eq!(error.code(), "INVALID_TARGET");
}

#[cfg(unix)]
#[test]
fn target_ancestor_links_are_rejected_before_installation() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let skill = source(temporary.path(), "source", "fixture");
    let actual = temporary.path().join("actual");
    let linked = temporary.path().join("linked");
    fs::create_dir(&actual).unwrap();
    symlink(&actual, &linked).unwrap();
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(AgentTargetId::Codex, linked.join("skills"));

    let error = store
        .install_from(
            &skill,
            local_source(&skill),
            &[TargetInstall {
                target: target.clone(),
                mode: InstallMode::Copy,
            }],
            &[target],
        )
        .unwrap_err();

    assert_eq!(error.code(), "INVALID_TARGET");
    assert!(!actual.join("skills").exists());
}

#[test]
fn remove_preserves_a_modified_target_copy() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = source(temporary.path(), "source", "managed");
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(
        AgentTargetId::Codex,
        temporary.path().join("project/.agents/skills"),
    );
    let name = store
        .install_from(
            &skill,
            local_source(&skill),
            &[TargetInstall {
                target: target.clone(),
                mode: InstallMode::Copy,
            }],
            std::slice::from_ref(&target),
        )
        .unwrap();
    fs::write(target.root.join("example/SKILL.md"), "user change").unwrap();

    let error = store
        .remove(&name, std::slice::from_ref(&target))
        .unwrap_err();

    assert_eq!(error.code(), "TARGET_CONFLICT");
    assert_eq!(
        fs::read_to_string(target.root.join("example/SKILL.md")).unwrap(),
        "user change"
    );
}

#[test]
fn invalid_skill_names_cannot_address_outside_paths() {
    assert!(SkillName::parse("../outside").is_err());
}

#[test]
fn install_rejects_a_frontmatter_name_that_differs_from_the_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = source(temporary.path(), "source", "fixture");
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: different\ndescription: Test fixture.\n---\n",
    )
    .unwrap();
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    let target = resolved(
        AgentTargetId::Codex,
        temporary.path().join("project/.agents/skills"),
    );

    let error = store
        .install_from(
            &skill,
            local_source(&skill),
            &[TargetInstall {
                target: target.clone(),
                mode: InstallMode::Copy,
            }],
            &[target],
        )
        .unwrap_err();

    assert_eq!(error.code(), "INVALID_SOURCE");
    assert!(!store.root().exists());
}

#[test]
fn recovery_rejects_a_journal_skill_with_path_traversal() {
    let temporary = tempfile::tempdir().unwrap();
    let store = LocalStore::new(temporary.path().join("project/.skills"));
    fs::create_dir_all(store.root().join(".skilld-transaction")).unwrap();
    fs::write(
        store.root().join(".skilld-transaction/state.json"),
        r#"{
          "version": 1,
          "transactionId": "fixture-1",
          "operation": "install",
          "skill": "../outside",
          "canonicalHadExisting": false,
          "targets": []
        }"#,
    )
    .unwrap();
    let outside = temporary.path().join("project/outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep"), "fixture").unwrap();

    let error = store.list(&[]).unwrap_err();

    assert_eq!(error.code(), "INVALID_LOCKFILE");
    assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(), "fixture");
}
