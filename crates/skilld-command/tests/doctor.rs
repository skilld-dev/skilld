use skilld_command::ResolvedTarget;
use skilld_command::doctor::{DoctorOptions, ScanContext, ScanPhase, scan, scan_with_progress};
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
        ".config/Code/User/History/old/skills/cached",
        "app/.intellijPlatform/sandbox/skills/generated",
        "app/.data/kv-dump/skills/generated",
        "media/movie.trickplay/skills/generated",
        ".gemini/history/session/skills/old",
        "google-cloud-sdk/lib/skills/bundled",
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

#[cfg(unix)]
#[test]
fn default_scan_stays_inside_home_even_with_external_agent_targets() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    skill(home.path(), "app/.claude/skills/local", "Local");
    skill(outside.path(), "skills/external", "External");
    fs::create_dir_all(home.path().join(".claude/skills")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("skills/external"),
        home.path().join(".claude/skills/external"),
    )
    .unwrap();
    let mut ctx = context(home.path());
    ctx.global_targets
        .push(ResolvedTarget::new(AgentTargetId::Cursor, outside.path().join("skills")).unwrap());
    let mut options = DoctorOptions::for_root(home.path());
    options.roots.clear();
    let report = scan(&options, &ctx).unwrap();
    assert_eq!(
        report
            .skills
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["local"]
    );
    assert_eq!(report.roots, vec![home.path().to_path_buf()]);
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.message.contains("outside"))
    );
    options.roots.push(outside.path().to_path_buf());
    let explicit = scan(&options, &ctx).unwrap();
    assert_eq!(explicit.skills[0].name, "external");
}

#[test]
fn progress_reports_discovery_then_actual_content_totals() {
    let home = tempfile::tempdir().unwrap();
    skill(home.path(), "app/.claude/skills/example", "Local");
    let mut events = vec![];
    let report = scan_with_progress(
        &DoctorOptions::for_root(home.path()),
        &context(home.path()),
        &mut |p| events.push(p),
    )
    .unwrap();
    assert_eq!(events[0].phase, ScanPhase::Discover);
    assert!(events.iter().any(|p| p.phase
        == ScanPhase::Fingerprint {
            completed: 0,
            total: 1
        }));
    let last = events.last().unwrap();
    assert_eq!(
        last.phase,
        ScanPhase::Fingerprint {
            completed: 1,
            total: 1
        }
    );
    assert_eq!(last.found_skills, report.skills.len());
    assert_eq!(last.visited_directories, report.visited_directories);
}

#[cfg(unix)]
#[test]
fn home_scan_reports_external_metadata_without_classifying_the_skill_as_unmanaged() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    skill(home.path(), ".claude/skills/example", "Local");
    let mut ctx = context(home.path());
    ctx.global_store = outside.path().join("store");
    fs::create_dir_all(&ctx.global_store).unwrap();
    fs::write(
        ctx.global_store.join("skilld-lock.yaml"),
        "invalid external metadata",
    )
    .unwrap();
    let mut options = DoctorOptions::for_root(home.path());
    options.roots.clear();
    let report = scan(&options, &ctx).unwrap();
    assert!(matches!(
        report.skills[0].owner,
        skilld_command::doctor::DoctorOwner::Unavailable { .. }
    ));
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.message.contains("outside the home directory"))
    );
}

#[cfg(unix)]
#[test]
fn home_scan_checks_children_of_linked_agent_roots_before_inspection() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    skill(outside.path(), "external", "External");
    skill(home.path(), "targets/local", "Local");
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    std::os::unix::fs::symlink(
        home.path().join("targets"),
        home.path().join(".claude/skills"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("external"),
        home.path().join("targets/external"),
    )
    .unwrap();
    let mut options = DoctorOptions::for_root(home.path());
    options.roots.clear();
    let report = scan(&options, &context(home.path())).unwrap();
    assert_eq!(report.skills.len(), 1);
    assert_eq!(report.skills[0].name, "local");
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.path.ends_with(".claude/skills/external") && p.message.contains("outside"))
    );
}
