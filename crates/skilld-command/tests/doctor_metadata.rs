use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use skilld_command::doctor_metadata::{fingerprint, foreign_lock_without, read_foreign_lock};

fn skill(root: &Path) {
    fs::create_dir_all(root.join("references")).unwrap();
    fs::write(
        root.join("SKILL.md"),
        "---\nname: example\n---\nInstructions.\n",
    )
    .unwrap();
    fs::write(root.join("references/check.md"), "supporting file").unwrap();
}

fn global_lock(path: &Path) {
    fs::write(path, serde_json::to_vec(&json!({
        "version": 3,
        "skills": {
            "example": {"source":"owner/repository", "sourceType":"github", "skillPath":"skills/example/SKILL.md", "ref":"release/v1", "skillFolderHash":"1234567890123456789012345678901234567890", "extra":42},
            "other": {"source":"owner/other", "sourceType":"github", "skillFolderHash":""}
        },
        "dismissed":{"findSkillsPrompt":true},
        "lastSelectedAgents":["claude-code"],
        "future":{"nested":[1,2,3]}
    })).unwrap()).unwrap();
}

#[test]
fn fingerprint_matches_git_tree_and_tracks_supporting_files() {
    let temp = tempfile::tempdir().unwrap();
    skill(temp.path());
    fs::write(temp.path().join("references.txt"), "ordering").unwrap();
    fs::create_dir(temp.path().join("empty")).unwrap();
    let result = fingerprint(temp.path()).unwrap();
    for args in [["init", "--quiet"].as_slice(), ["add", "."].as_slice()] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(temp.path())
                .status()
                .unwrap()
                .success()
        );
    }
    let git = Command::new("git")
        .arg("write-tree")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(git.status.success());
    assert_eq!(
        result.git_tree,
        String::from_utf8(git.stdout).unwrap().trim()
    );
    assert_eq!(result.files, 3);
    assert_eq!(
        fingerprint(temp.path()).unwrap(),
        result,
        "ignore Git metadata"
    );
    fs::write(temp.path().join("references/check.md"), "changed").unwrap();
    let changed = fingerprint(temp.path()).unwrap();
    assert_ne!(changed.sha256, result.sha256);
    assert_ne!(changed.git_tree, result.git_tree);
}

#[test]
fn fingerprint_counts_empty_directories_and_frames_names_and_bytes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a"), "bc").unwrap();
    let before = fingerprint(temp.path()).unwrap();
    fs::remove_file(temp.path().join("a")).unwrap();
    fs::write(temp.path().join("ab"), "c").unwrap();
    let renamed = fingerprint(temp.path()).unwrap();
    assert_ne!(before.sha256, renamed.sha256);
    fs::create_dir(temp.path().join("empty")).unwrap();
    let with_empty = fingerprint(temp.path()).unwrap();
    assert_ne!(with_empty.sha256, renamed.sha256);
    assert_eq!(with_empty.git_tree, renamed.git_tree);
}

#[cfg(unix)]
#[test]
fn fingerprint_tracks_permissions_and_links_without_following_them() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    skill(temp.path());
    let before = fingerprint(temp.path()).unwrap();
    fs::set_permissions(
        temp.path().join("SKILL.md"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let executable = fingerprint(temp.path()).unwrap();
    assert_ne!(before.sha256, executable.sha256);
    assert_ne!(before.git_tree, executable.git_tree);
    fs::set_permissions(
        temp.path().join("SKILL.md"),
        fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    let permissions = fingerprint(temp.path()).unwrap();
    assert_ne!(permissions.sha256, executable.sha256);
    assert_eq!(permissions.git_tree, executable.git_tree);
    symlink("references", temp.path().join("linked")).unwrap();
    let linked = fingerprint(temp.path()).unwrap();
    assert_eq!(linked.files, permissions.files + 1);
    assert_eq!(linked.bytes, permissions.bytes + "references".len() as u64);
    assert_ne!(linked.sha256, permissions.sha256);
    symlink("../../outside", temp.path().join("escape")).unwrap();
    assert_eq!(
        fingerprint(temp.path()).unwrap_err().code,
        "DOCTOR_UNSUPPORTED_TREE"
    );
}

#[test]
fn fingerprint_limits_depth_and_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let oversized = fs::File::create(temp.path().join("large")).unwrap();
    oversized.set_len(64 * 1024 * 1024 + 1).unwrap();
    assert_eq!(
        fingerprint(temp.path()).unwrap_err().code,
        "DOCTOR_SCAN_LIMIT"
    );
    fs::remove_file(temp.path().join("large")).unwrap();
    let mut nested = temp.path().to_owned();
    for _ in 0..33 {
        nested.push("nested");
        fs::create_dir(&nested).unwrap();
    }
    assert_eq!(
        fingerprint(temp.path()).unwrap_err().code,
        "DOCTOR_SCAN_LIMIT"
    );
}

#[test]
fn fingerprint_limits_file_count() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..10_001 {
        fs::write(temp.path().join(index.to_string()), "").unwrap();
    }
    assert_eq!(
        fingerprint(temp.path()).unwrap_err().code,
        "DOCTOR_SCAN_LIMIT"
    );
}

#[cfg(unix)]
#[test]
fn fingerprint_rejects_links_that_escape_through_another_link() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("skill");
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(temp.path().join("outside"), "private").unwrap();
    symlink(".", root.join("nested/alias")).unwrap();
    symlink("alias/../../outside", root.join("nested/escape")).unwrap();
    assert_eq!(
        fingerprint(&root).unwrap_err().code,
        "DOCTOR_UNSUPPORTED_TREE"
    );
}

#[test]
fn foreign_lock_rejects_duplicate_json_fields() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".skill-lock.json");
    fs::write(&path, br#"{"version":3,"skills":{},"skills":{"example":{"source":"owner/repo","sourceType":"github"}}}"#).unwrap();
    assert_eq!(
        read_foreign_lock(&path, true).unwrap_err().code,
        "DOCTOR_INVALID_LOCKFILE"
    );
}

#[test]
fn foreign_lock_keeps_scoped_package_sources() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("skills-lock.json");
    fs::write(&path, serde_json::to_vec(&json!({"version":1,"skills":{"example":{"source":"@scope/package","sourceType":"node_modules","computedHash":"a".repeat(64)}}})).unwrap()).unwrap();
    let records = read_foreign_lock(&path, false).unwrap();
    assert_eq!(records[0].source, "@scope/package");
}

#[test]
fn foreign_lock_removes_one_record_and_preserves_unrelated_fields() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".skill-lock.json");
    global_lock(&path);
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let records = read_foreign_lock(&path, true).unwrap();
    let record = records
        .iter()
        .find(|record| record.name == "example")
        .unwrap();
    assert_eq!(record.source, "owner/repository");
    assert_eq!(
        record.skill_path.as_deref(),
        Some("skills/example/SKILL.md")
    );
    assert_eq!(record.revision.as_deref(), Some("release/v1"));
    assert_eq!(
        record.expected_tree.as_deref(),
        Some("1234567890123456789012345678901234567890")
    );
    let output: Value =
        serde_json::from_slice(&foreign_lock_without(&path, record).unwrap()).unwrap();
    let mut expected = original.clone();
    expected["skills"]
        .as_object_mut()
        .unwrap()
        .remove("example");
    assert_eq!(output, expected);
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
        original
    );
    fs::write(&path, b"{}").unwrap();
    assert_eq!(
        foreign_lock_without(&path, record).unwrap_err().code,
        "DOCTOR_STALE_LOCKFILE"
    );
}

#[test]
fn project_content_hash_is_distinct_from_git_tree() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("skills-lock.json");
    let hash = "a".repeat(64);
    fs::write(&path, serde_json::to_vec(&json!({"version":1,"skills":{"example":{"source":"owner/repository","sourceType":"github","computedHash":hash}}})).unwrap()).unwrap();
    let records = read_foreign_lock(&path, false).unwrap();
    assert_eq!(records[0].expected_content.as_deref(), Some(hash.as_str()));
    assert_eq!(records[0].expected_tree, None);
    assert!(!records[0].global);
}

#[test]
fn foreign_lock_rejects_malformed_versions_records_and_paths() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".skill-lock.json");
    for input in [
        json!({"version":4,"skills":{}}),
        json!({"version":2,"skills":{}}),
        json!({"version":3,"skills":[]}),
        json!({"version":3,"skills":{"../escape":{"source":"owner/repository","sourceType":"github"}}}),
        json!({"version":3,"skills":{"example":{"source":"owner/repository","sourceType":"github","skillPath":"../SKILL.md"}}}),
        json!({"version":3,"skills":{"example":{"source":"https://token@example.com/repo","sourceType":"git"}}}),
        json!({"version":3,"skills":{"example":{"source":"owner/repository","sourceType":"github","skillFolderHash":"wrong"}}}),
        json!({"version":3,"skills":{"example":{"sourceType":"github"}}}),
    ] {
        fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
        assert_eq!(
            read_foreign_lock(&path, true).unwrap_err().code,
            "DOCTOR_INVALID_LOCKFILE",
            "{input}"
        );
    }
    fs::write(&path, b"{invalid").unwrap();
    assert_eq!(
        read_foreign_lock(&path, true).unwrap_err().code,
        "DOCTOR_INVALID_LOCKFILE"
    );
}

#[cfg(unix)]
#[test]
fn foreign_lock_rejects_symlink_files() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".skill-lock.json");
    global_lock(&path);
    let records = read_foreign_lock(&path, true).unwrap();
    let actual = temp.path().join("actual");
    fs::rename(&path, &actual).unwrap();
    symlink(&actual, &path).unwrap();
    assert_eq!(
        read_foreign_lock(&path, true).unwrap_err().code,
        "DOCTOR_INVALID_LOCKFILE"
    );
    assert_eq!(
        foreign_lock_without(&path, &records[0]).unwrap_err().code,
        "DOCTOR_INVALID_LOCKFILE"
    );
}
