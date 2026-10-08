use skilld_command::ResolvedTarget;
use skilld_command::doctor::{DoctorOptions, ScanContext, scan};
use skilld_core::AgentTargetId;
use std::{fs, path::Path};

fn skill(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: example\ndescription: Test.\n---\n{text}"),
    )
    .unwrap();
}

fn context(home: &Path) -> ScanContext {
    ScanContext {
        home: home.to_path_buf(),
        global_store: home.join(".skilld/skills"),
        global_targets: vec![
            ResolvedTarget::new(AgentTargetId::ClaudeCode, home.join(".claude/skills")).unwrap(),
            ResolvedTarget::new(AgentTargetId::Codex, home.join(".agents/skills")).unwrap(),
        ],
        skills_sh_global_lock: home.join(".agents/.skill-lock.json"),
    }
}

#[test]
fn sweep_keeps_hidden_claude_skills_and_prunes_dependency_and_session_trees() {
    let home = tempfile::tempdir().unwrap();
    for path in [
        "app/.claude/skills/example",
        ".claude/skills/global",
        "app/node_modules/pkg/skills/dependency",
        ".codex/.tmp/plugins/staged",
        ".claude/projects/session/skills/old",
    ] {
        skill(home.path(), path, "Original");
    }
    fs::write(home.path().join("app/CLAUDE.md"), "Project instructions").unwrap();
    let report = scan(&DoctorOptions::for_root(home.path()), &context(home.path())).unwrap();
    assert_eq!(report.skills.len(), 2);
    assert!(report.skills.iter().any(|s| {
        s.paths
            .iter()
            .any(|p| p.path == home.path().join("app/.claude/skills/example"))
    }));
    assert_eq!(report.claude_files, vec![home.path().join("app/CLAUDE.md")]);
    assert!(report.skipped_directories >= 3);
}

#[test]
fn exclusions_prune_before_reading_and_invalid_globs_fail() {
    let home = tempfile::tempdir().unwrap();
    skill(home.path(), "app/.claude/skills/keep", "Keep");
    skill(home.path(), "archive/app/.claude/skills/skip", "Skip");
    let mut options = DoctorOptions::for_root(home.path());
    options.exclude.push("**/archive/**".into());
    let report = scan(&options, &context(home.path())).unwrap();
    assert_eq!(report.skills.len(), 1);
    options.exclude.push("[".into());
    assert!(scan(&options, &context(home.path())).is_err());
}

#[test]
fn duplicates_compare_supporting_files_not_only_instructions() {
    let home = tempfile::tempdir().unwrap();
    for name in ["one", "two", "three"] {
        skill(home.path(), &format!("app/.claude/skills/{name}"), "Same");
    }
    fs::write(
        home.path().join("app/.claude/skills/three/reference.md"),
        "Different",
    )
    .unwrap();
    let report = scan(&DoctorOptions::for_root(home.path()), &context(home.path())).unwrap();
    let one = report.skills.iter().find(|s| s.name == "one").unwrap();
    assert_eq!(one.duplicates.len(), 1);
    let three = report.skills.iter().find(|s| s.name == "three").unwrap();
    assert!(three.duplicates.is_empty());
}

#[cfg(unix)]
#[test]
fn links_share_one_skill_and_broken_links_are_reported_without_following_loops() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    skill(home.path(), ".agents/skills/example", "Same");
    fs::create_dir_all(home.path().join(".claude/skills")).unwrap();
    symlink(
        home.path().join(".agents/skills/example"),
        home.path().join(".claude/skills/example"),
    )
    .unwrap();
    symlink(
        home.path().join("missing"),
        home.path().join(".claude/skills/broken"),
    )
    .unwrap();
    symlink(home.path(), home.path().join("loop")).unwrap();
    let report = scan(&DoctorOptions::for_root(home.path()), &context(home.path())).unwrap();
    assert_eq!(report.skills.len(), 1);
    assert_eq!(report.skills[0].paths.len(), 2);
    assert!(report.problems.iter().any(|p| p.path.ends_with("broken")));
}

#[cfg(unix)]
#[test]
fn excluded_skill_links_are_not_read_and_explicit_target_roots_keep_their_scope() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    skill(home.path(), "source/skip", "Same");
    fs::create_dir_all(home.path().join("app/.claude")).unwrap();
    symlink(
        home.path().join("source"),
        home.path().join("app/.claude/skills"),
    )
    .unwrap();
    let mut options = DoctorOptions::for_root(&home.path().join("app/.claude/skills"));
    let report = scan(&options, &context(home.path())).unwrap();
    assert_eq!(
        report.skills[0].paths[0].agent,
        Some(AgentTargetId::ClaudeCode)
    );
    options.exclude.push("**/skip/**".into());
    assert!(
        scan(&options, &context(home.path()))
            .unwrap()
            .skills
            .is_empty()
    );
}
