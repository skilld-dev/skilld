use skilld_command::ResolvedTarget;
use skilld_command::doctor::{DoctorOptions, ScanContext, ScanPhase, ScanProgress, scan};
use skilld_command::doctor_actions::ActionPreview;
use skilld_core::AgentTargetId;
use skilld_native::doctor_ui::{Browse, Model, PathFilter, Screen, WorkState, render_snapshot};
use std::fs;

#[test]
fn loading_screen_explains_work_instead_of_showing_an_empty_result() {
    let snapshot = render_snapshot(&Model::default(), 100, 26);
    assert!(snapshot.contains("Finding Skill files"));
    assert!(snapshot.contains("q cancel"));
    assert!(!snapshot.contains("No Skills in this group"));
}

#[test]
fn loading_progress_shows_scope_counts_elapsed_and_respects_no_color() {
    let mut model = Model {
        roots: vec!["/home/test".into()],
        elapsed: std::time::Duration::from_secs(12),
        progress: Some(ScanProgress {
            phase: ScanPhase::Fingerprint {
                completed: 3,
                total: 12,
            },
            current_path: "/home/test/.claude/skills/example".into(),
            visited_directories: 128,
            skipped_directories: 7,
            found_skills: 12,
            problems: 0,
        }),
        ..Model::default()
    };
    let snapshot = render_snapshot(&model, 100, 26);
    for text in [
        "Scan: /home/test",
        "Checking Skill contents",
        "3 / 12 Skills",
        "12s elapsed",
        "128 directories scanned",
        "7 excluded",
    ] {
        assert!(snapshot.contains(text), "{text}");
    }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 26)).unwrap();
    terminal
        .draw(|f| skilld_native::doctor_ui::view(f, &model))
        .unwrap();
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.fg == ratatui::style::Color::Cyan)
    );
    model.color = false;
    terminal
        .draw(|f| skilld_native::doctor_ui::view(f, &model))
        .unwrap();
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .all(|c| c.fg == ratatui::style::Color::Reset && c.bg == ratatui::style::Color::Reset)
    );
}

#[test]
fn doctor_groups_skills_sh_and_previews_removal_without_changing_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".agents/skills/example");
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        "---\nname: example\ndescription: Read files.\n---\nRead the project.",
    )
    .unwrap();
    fs::write(root.path().join(".agents/.skill-lock.json"),r#"{"version":3,"skills":{"example":{"source":"owner/repository","sourceType":"github","skillFolderHash":""}}}"#).unwrap();
    let context = ScanContext {
        home: root.path().into(),
        global_store: root.path().join(".skilld/skills"),
        global_targets: vec![
            ResolvedTarget::new(AgentTargetId::Codex, root.path().join(".agents/skills")).unwrap(),
        ],
        skills_sh_global_lock: root.path().join(".agents/.skill-lock.json"),
    };
    let report = scan(&DoctorOptions::for_root(root.path()), &context).unwrap();
    let mut model = Model {
        report: Some(report),
        work: WorkState::Ready,
        ..Model::default()
    };
    let overview = render_snapshot(&model, 110, 26);
    assert!(overview.contains("Review skills.sh migration"));
    assert!(!model.can_review_action(110, 26));
    model.open_group();
    assert!(render_snapshot(&model, 110, 26).contains("Global Agent targets"));
    model.open_group();
    let snapshot = render_snapshot(&model, 110, 26);
    assert!(snapshot.contains("example"));
    assert!(snapshot.contains("m migrate"));
    assert!(snapshot.contains("d remove"));
    let owner_line = snapshot
        .lines()
        .find(|line| line.contains("Owner: skills.sh"))
        .unwrap();
    assert!(!owner_line.contains("example"));
    model.filter = "missing".into();
    assert!(model.visible().is_empty());
    assert!(render_snapshot(&model, 110, 26).contains("No Skills match"));
    model.filter = "example".into();
    assert_eq!(model.visible().len(), 1);
    model.preview = Some(ActionPreview {
        action: "Remove".into(),
        name: "example".into(),
        paths: vec![path.clone()],
        source: None,
        commit: None,
        changes_content: false,
        behaviors: vec![],
    });
    let review = render_snapshot(&model, 110, 26);
    assert!(review.contains("Review action"));
    assert!(review.contains("esc cancel"));
    assert!(path.join("SKILL.md").exists());
    let narrow = render_snapshot(&model, 40, 12);
    assert!(narrow.contains("Resize"));
}

#[test]
fn applying_never_promises_cancellation_and_filter_keeps_error_visible() {
    let mut model = Model {
        work: WorkState::Applying,
        filter: "example".into(),
        message: "Could not read affected files".into(),
        ..Model::default()
    };
    let output = render_snapshot(&model, 80, 24);
    assert!(output.contains("Wait for the result"));
    assert!(!output.contains("q cancel"));
    assert!(output.contains("Could not read affected files"));
    model.work = WorkState::Ready;
    let output = render_snapshot(&model, 80, 24);
    assert!(output.contains("Could not read affected files"));
    assert!(output.contains("Filter: example"));
}

#[test]
fn narrow_review_uses_full_content_area_and_scroll_cannot_hide_all_content() {
    let model = Model {
        work: WorkState::Ready,
        preview: Some(ActionPreview {
            action: "Remove".into(),
            name: "example".into(),
            paths: vec!["/home/test/.agents/skills/example".into()],
            source: None,
            commit: None,
            changes_content: false,
            behaviors: vec![],
        }),
        ..Model::default()
    };
    let output = render_snapshot(&model, 80, 24);
    assert!(output.contains("Affected Agent targets:"));
    assert!(output.contains("/home/test/.agents/skills/example"));
    assert!(output.contains("Enter applies"));
    let model = Model {
        detail_scroll: u16::MAX,
        ..model
    };
    assert!(render_snapshot(&model, 80, 24).contains("Enter applies"));
}

#[test]
fn help_is_readable_and_focus_survives_color_removal() {
    let model = Model {
        work: WorkState::Ready,
        screen: Screen::Help,
        color: false,
        ..Model::default()
    };
    let output = render_snapshot(&model, 80, 24);
    assert!(output.contains("> Help"));
    assert!(output.contains("Ctrl-C"));
    assert!(output.contains("Esc: back"));
}

#[test]
fn actions_are_unavailable_while_working_filtering_or_outside_skill_view() {
    let mut model = Model {
        work: WorkState::Ready,
        ..Model::default()
    };
    assert!(!model.can_review_action(80, 24));
    assert!(!model.can_review_action(59, 24));
    assert!(!model.can_review_action(80, 17));
    for work in [
        WorkState::Scanning,
        WorkState::Preparing,
        WorkState::Applying,
    ] {
        model.work = work;
        assert!(!model.can_review_action(80, 24));
    }
    model.work = WorkState::Ready;
    model.screen = Screen::Help;
    assert!(!model.can_review_action(80, 24));
    model.screen = Screen::Skills;
    model.editing_filter = true;
    assert!(!model.can_review_action(80, 24));
}

#[test]
fn problems_show_paths_and_reasons_inside_the_terminal() {
    let model = Model {
        work: WorkState::Ready,
        screen: Screen::Problems,
        report: Some(skilld_command::doctor::DoctorReport {
            roots: vec![],
            excludes: vec![],
            skills: vec![],
            claude_files: vec![],
            problems: vec![skilld_command::doctor::ScanProblem {
                path: "/home/test/restricted".into(),
                message: "Permission denied".into(),
            }],
            visited_directories: 1,
            skipped_directories: 0,
        }),
        ..Model::default()
    };
    let output = render_snapshot(&model, 80, 24);
    assert!(output.contains("/home/test/restricted"));
    assert!(output.contains("Permission denied"));
    assert!(output.contains("esc back"));
}

#[test]
fn scrolling_at_the_end_can_immediately_return_to_previous_content() {
    let mut model = Model {
        work: WorkState::Ready,
        screen: Screen::Help,
        ..Model::default()
    };
    let before = render_snapshot(&model, 60, 18);
    for _ in 0..1000 {
        model.move_selection(true);
    }
    let end = render_snapshot(&model, 60, 18);
    assert_ne!(before, end);
    model.move_selection(false);
    assert_ne!(render_snapshot(&model, 60, 18), end);
}

#[test]
fn shrinking_during_application_keeps_wait_instructions_truthful() {
    let model = Model {
        work: WorkState::Applying,
        ..Model::default()
    };
    let output = render_snapshot(&model, 50, 12);
    assert!(output.contains("Wait for the result"));
    assert!(!output.contains("q quit"));
}

#[test]
fn full_notice_keeps_long_errors_and_recovery_paths_reachable() {
    let mut model = Model {
        work: WorkState::Ready,
        screen: Screen::Notice,
        message: format!(
            "Removed: example. Backup: /home/test/{}recovery.json",
            "long-directory/".repeat(80)
        ),
        ..Model::default()
    };
    assert!(render_snapshot(&model, 80, 24).contains("Removed: example"));
    for _ in 0..1000 {
        model.move_selection(true);
    }
    assert!(render_snapshot(&model, 80, 24).contains("recovery.json"));
}

fn scan_fixture(root: &std::path::Path) -> skilld_command::doctor::DoctorReport {
    scan(
        &DoctorOptions::for_root(root),
        &ScanContext {
            home: root.into(),
            global_store: root.join(".skilld/skills"),
            global_targets: vec![],
            skills_sh_global_lock: root.join(".agents/.skill-lock.json"),
        },
    )
    .unwrap()
}

#[test]
fn recommendations_reduce_large_results_and_require_drilldown_before_actions() {
    let root = tempfile::tempdir().unwrap();
    for project in ["first", "second"] {
        for index in 0..40 {
            let path = root
                .path()
                .join(format!("{project}/.claude/skills/skill-{index}"));
            fs::create_dir_all(&path).unwrap();
            fs::write(
                path.join("SKILL.md"),
                format!("# Skill {index}\nRead {project}.\n"),
            )
            .unwrap();
        }
    }
    let mut model = Model {
        report: Some(scan_fixture(root.path())),
        work: WorkState::Ready,
        ..Model::default()
    };
    assert_eq!(model.visible().len(), 80);
    assert_eq!(model.entries().len(), 1);
    assert!(render_snapshot(&model, 100, 26).contains("Inspect unknown installs"));
    assert!(!model.can_review_action(100, 26));
    model.open_group();
    assert_eq!(model.entries().len(), 2);
    assert!(!model.can_review_action(100, 26));
    model.move_selection(true);
    model.open_group();
    assert_eq!(model.entries().len(), 40);
    assert!(model.can_review_action(100, 26));
    let selected = model.selected_skill().unwrap();
    assert!(
        model.report.as_ref().unwrap().skills[selected]
            .canonical_path
            .starts_with(root.path().join("second"))
    );
    model.back();
    assert_eq!(model.entries().len(), 2);
    model.back();
    assert_eq!(model.entries().len(), 1);
}

#[cfg(unix)]
#[test]
fn symlink_filter_keeps_alias_scope_and_duplicate_details_name_real_copies() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("catalog/example");
    let copy = root.path().join("project/.claude/skills/copied");
    let alias = root.path().join("project/.claude/skills/alias-only-name");
    for path in [&source, &copy] {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("SKILL.md"), "# Example\nRead files.\n").unwrap();
    }
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    let mut model = Model {
        report: Some(scan_fixture(root.path())),
        work: WorkState::Ready,
        ..Model::default()
    };
    assert_eq!(model.visible().len(), 2);
    model.toggle_group();
    assert_eq!(model.path_filter, PathFilter::Symlinks);
    assert_eq!(model.visible().len(), 1);
    model.filter = "alias-only-name".into();
    assert_eq!(model.visible().len(), 1);
    model.open_group();
    model.open_group();
    let snapshot = render_snapshot(&model, 180, 60);
    assert!(snapshot.contains("Symlinks:"));
    assert!(snapshot.contains("alias-only-name"));
    assert!(snapshot.contains(&format!("-> {}", source.display())));
    assert!(snapshot.contains("Duplicate: 1 identical directory copies."));
    assert!(snapshot.contains(&copy.display().to_string()));
    model.filter.clear();
    model.toggle_group();
    assert_eq!(model.path_filter, PathFilter::Directories);
    assert_eq!(model.visible().len(), 2);
    assert_eq!(model.browse, Browse::Owners);
}
