use skilld_command::ResolvedTarget;
use skilld_command::doctor::{DoctorOptions, ScanContext, ScanPhase, ScanProgress, scan};
use skilld_command::doctor_actions::ActionPreview;
use skilld_core::AgentTargetId;
use skilld_native::doctor_ui::{Model, Screen, WorkState, render_snapshot};
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
    model.toggle_group();
    let snapshot = render_snapshot(&model, 110, 26);
    assert!(snapshot.contains("skills.sh installs"));
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
    let model = Model {
        work: WorkState::Applying,
        filter: "example".into(),
        message: "Could not read affected files".into(),
        ..Model::default()
    };
    let output = render_snapshot(&model, 80, 24);
    assert!(output.contains("Wait for the result"));
    assert!(!output.contains("q cancel"));
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
    assert!(model.can_review_action(80, 24));
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
