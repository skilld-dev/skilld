use skilld_command::ResolvedTarget;
use skilld_command::doctor::{DoctorOptions, ScanContext, scan};
use skilld_command::doctor_actions::ActionPreview;
use skilld_core::AgentTargetId;
use skilld_native::doctor_ui::{Model, render_snapshot};
use std::fs;

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
        busy: false,
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
    assert!(narrow.contains("Review action"));
}
