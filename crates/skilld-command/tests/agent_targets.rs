use std::fs;
use std::path::Path;

use skilld_command::{DetectionEnvironment, Host, LocalHost, TargetRoots};
use skilld_core::{
    AgentTargetId, InstallMode, InstallOperation, InstallRequest, InstallScope, InstallSource,
};

fn source(root: &Path) -> std::path::PathBuf {
    let source = root.join("source/example");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: example\ndescription: Test fixture.\n---\n\nfixture\n",
    )
    .unwrap();
    source
}

#[test]
fn every_project_signal_selects_the_matching_agent_target() {
    let cases = [
        (AgentTargetId::ClaudeCode, ".claude", ".claude/skills"),
        (AgentTargetId::Cursor, ".cursorrules", ".cursor/skills"),
        (
            AgentTargetId::Windsurf,
            ".windsurfrules",
            ".windsurf/skills",
        ),
        (AgentTargetId::Cline, ".cline", ".cline/skills"),
        (AgentTargetId::Codex, ".codex", ".agents/skills"),
        (
            AgentTargetId::GithubCopilot,
            ".github/copilot-instructions.md",
            ".github/skills",
        ),
        (AgentTargetId::GeminiCli, ".gemini", ".gemini/skills"),
        (AgentTargetId::Goose, ".goose", ".goose/skills"),
        (AgentTargetId::Amp, ".agents/AGENTS.md", ".agents/skills"),
        (AgentTargetId::Opencode, ".opencode", ".opencode/skills"),
        (AgentTargetId::Roo, ".roo", ".roo/skills"),
        (AgentTargetId::Antigravity, ".agent", ".agent/skills"),
        (AgentTargetId::Openclaw, ".openclaw", "skills"),
        (AgentTargetId::Hermes, ".hermes", ".hermes/skills"),
        (AgentTargetId::Kiro, ".kiro", ".kiro/skills"),
        (AgentTargetId::Kilo, ".kilo", ".kilo/skills"),
        (AgentTargetId::Droid, ".factory", ".factory/skills"),
        (AgentTargetId::Trae, ".trae", ".trae/skills"),
        (AgentTargetId::Zed, ".zed", ".agents/skills"),
    ];

    for (agent, signal, skills_dir) in cases {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let data = temporary.path().join("data");
        fs::create_dir_all(&project).unwrap();
        let signal_path = project.join(signal);
        if signal.contains('.') && Path::new(signal).extension().is_some() {
            fs::create_dir_all(signal_path.parent().unwrap()).unwrap();
            fs::write(&signal_path, "fixture").unwrap();
        } else {
            fs::create_dir_all(&signal_path).unwrap();
        }
        let source = source(temporary.path());
        let host = LocalHost::new(project.clone(), data);

        let names = host
            .install_request(InstallRequest {
                operation: InstallOperation::Install(InstallSource::Local(source)),
                scope: InstallScope::Project,
                targets: vec![],
                mode: None,
                allowed_behaviors: Vec::new(),
            })
            .unwrap();

        assert_eq!(
            names
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            ["example"],
            "{}",
            agent.as_str()
        );
        assert!(
            project.join(skills_dir).join("example/SKILL.md").exists(),
            "{}",
            agent.as_str()
        );
        let view = host.view("example", InstallScope::Project).unwrap();
        assert_eq!(view.skill.targets[0].agent, agent);
    }
}

#[test]
fn every_runtime_signal_selects_the_matching_agent_target() {
    let cases = [
        (AgentTargetId::ClaudeCode, "CLAUDE_CODE", ".claude/skills"),
        (AgentTargetId::Cursor, "CURSOR_SESSION", ".cursor/skills"),
        (
            AgentTargetId::Windsurf,
            "WINDSURF_SESSION",
            ".windsurf/skills",
        ),
        (AgentTargetId::Cline, "CLINE_TASK_ID", ".cline/skills"),
        (
            AgentTargetId::GithubCopilot,
            "COPILOT_RUN_APP",
            ".github/skills",
        ),
        (AgentTargetId::GeminiCli, "GEMINI_CLI", ".gemini/skills"),
        (AgentTargetId::Goose, "GOOSE_SESSION", ".goose/skills"),
        (AgentTargetId::Amp, "AMP_SESSION", ".agents/skills"),
        (
            AgentTargetId::Opencode,
            "OPENCODE_SESSION",
            ".opencode/skills",
        ),
        (AgentTargetId::Roo, "ROO_SESSION", ".roo/skills"),
        (
            AgentTargetId::Antigravity,
            "ANTIGRAVITY_CLI_ALIAS",
            ".agent/skills",
        ),
        (AgentTargetId::Openclaw, "OPENCLAW_SHELL", "skills"),
        (AgentTargetId::Hermes, "HERMES_AGENT", ".hermes/skills"),
        (AgentTargetId::Kiro, "AGENT_CONTEXT_OUT", ".kiro/skills"),
        (AgentTargetId::Kilo, "KILO_RUN_ID", ".kilo/skills"),
        (AgentTargetId::Zed, "ZED_TERM", ".agents/skills"),
    ];

    for (agent, signal, skills_dir) in cases {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let data = temporary.path().join("data");
        fs::create_dir_all(&project).unwrap();
        let source = source(temporary.path());
        let host = LocalHost::new(project.clone(), data)
            .with_detection_environment(DetectionEnvironment::new([signal.to_owned()]));

        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Local(source)),
            scope: InstallScope::Project,
            targets: vec![],
            mode: None,
            allowed_behaviors: Vec::new(),
        })
        .unwrap();

        assert!(
            project.join(skills_dir).join("example/SKILL.md").exists(),
            "{}",
            agent.as_str()
        );
    }
}

#[test]
fn every_new_agent_target_resolves_its_global_and_project_paths() {
    let cases = [
        (AgentTargetId::Openclaw, ".openclaw/skills", "skills"),
        (AgentTargetId::Hermes, ".hermes/skills", ".hermes/skills"),
        (AgentTargetId::Kiro, ".kiro/skills", ".kiro/skills"),
        (AgentTargetId::Kilo, ".kilo/skills", ".kilo/skills"),
        (AgentTargetId::Droid, ".factory/skills", ".factory/skills"),
        (AgentTargetId::Trae, ".trae/skills", ".trae/skills"),
        (AgentTargetId::Zed, ".agents/skills", ".agents/skills"),
    ];

    for (agent, global_dir, project_dir) in cases {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let data = temporary.path().join("data");
        let home = temporary.path().join("home");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&home).unwrap();
        let source = source(temporary.path());
        let host = LocalHost::new(project.clone(), data).with_target_roots(TargetRoots::new(
            home.clone(),
            home.join(".config"),
            home.join(".claude"),
            home.join(".openclaw"),
            home.join(".hermes"),
            home.join(".kiro"),
        ));

        for (scope, root, dir) in [
            (InstallScope::Global, &home, global_dir),
            (InstallScope::Project, &project, project_dir),
        ] {
            host.install_request(InstallRequest {
                operation: InstallOperation::Install(InstallSource::Local(source.clone())),
                scope,
                targets: vec![agent],
                mode: None,
                allowed_behaviors: Vec::new(),
            })
            .unwrap();

            assert!(
                root.join(dir).join("example/SKILL.md").exists(),
                "{} {:?}",
                agent.as_str(),
                scope
            );
        }
    }
}

#[test]
fn a_first_party_skills_directory_alone_does_not_select_openclaw() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    fs::create_dir_all(project.join("skills/team-skill")).unwrap();
    fs::write(project.join("skills/team-skill/SKILL.md"), "fixture").unwrap();
    fs::create_dir_all(project.join(".cursor")).unwrap();
    let source = source(temporary.path());
    let host = LocalHost::new(project.clone(), data);

    let names = host
        .install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Local(source)),
            scope: InstallScope::Project,
            targets: vec![],
            mode: None,
            allowed_behaviors: Vec::new(),
        })
        .unwrap();

    let names: Vec<&str> = names.iter().map(|skill| skill.name.as_str()).collect();
    assert_eq!(names, ["example"]);
    let skills = fs::read_dir(project.join("skills"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    assert_eq!(skills, ["team-skill"]);
    let view = host.view("example", InstallScope::Project).unwrap();
    assert_eq!(view.skill.targets[0].agent, AgentTargetId::Cursor);
}

#[test]
fn overridden_agent_homes_receive_global_installs_and_detection() {
    for (agent, home_name) in [
        (AgentTargetId::Openclaw, "openclaw-state"),
        (AgentTargetId::Hermes, "hermes-home"),
        (AgentTargetId::Kiro, "kiro-home"),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let data = temporary.path().join("data");
        let home = temporary.path().join("home");
        let agent_home = temporary.path().join(home_name);
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&agent_home).unwrap();
        let host = LocalHost::new(project, data).with_target_roots(TargetRoots::new(
            home.clone(),
            home.join(".config"),
            home.join(".claude"),
            agent_home.clone(),
            agent_home.clone(),
            agent_home.clone(),
        ));

        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Local(source(temporary.path()))),
            scope: InstallScope::Global,
            targets: vec![],
            mode: None,
            allowed_behaviors: Vec::new(),
        })
        .unwrap();

        assert!(
            agent_home.join("skills/example/SKILL.md").exists(),
            "{}",
            agent.as_str()
        );
        assert!(
            !home
                .join(format!(".{}/skills/example/SKILL.md", agent.as_str()))
                .exists(),
            "{}",
            agent.as_str()
        );
    }
}

#[test]
fn an_existing_global_target_directory_is_detected() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(home.join(".agents/skills")).unwrap();
    fs::create_dir_all(&project).unwrap();
    let source = source(temporary.path());
    let host = LocalHost::new(project, data).with_target_roots(TargetRoots::new(
        home.clone(),
        home.join(".config"),
        home.join(".claude"),
        home.join(".openclaw"),
        home.join(".hermes"),
        home.join(".kiro"),
    ));

    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Local(source)),
        scope: InstallScope::Global,
        targets: vec![],
        mode: None,
        allowed_behaviors: Vec::new(),
    })
    .unwrap();

    assert!(home.join(".agents/skills/example/SKILL.md").exists());
}

/// A global symlink install writes the link into the Agent's own skills
/// directory, and the link resolves from there.
///
/// The link is relative, so its depth has to match the distance from the Agent
/// target directory to the Skill store. A wrong depth still creates a link and
/// still reports success, and the Agent then enumerates a directory of dead
/// entries, so the test reads SKILL.md back through the link.
#[test]
fn a_global_symlink_install_resolves_from_the_agent_skills_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("home/.skilld");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    let source = source(temporary.path());
    let host = LocalHost::new(project, data).with_target_roots(TargetRoots::new(
        home.clone(),
        home.join(".config"),
        home.join(".claude"),
        home.join(".openclaw"),
        home.join(".hermes"),
        home.join(".kiro"),
    ));

    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Local(source)),
        scope: InstallScope::Global,
        targets: vec![AgentTargetId::ClaudeCode],
        mode: Some(InstallMode::Symlink),
        allowed_behaviors: Vec::new(),
    })
    .unwrap();

    let installed = home.join(".claude/skills/example");
    assert!(
        fs::symlink_metadata(&installed)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(fs::read_link(&installed).unwrap().is_relative());
    assert_eq!(
        fs::read_to_string(installed.join("SKILL.md")).unwrap(),
        "---\nname: example\ndescription: Test fixture.\n---\n\nfixture\n"
    );
}

fn host_with_home(project: &Path, data: &Path, home: &Path) -> LocalHost {
    LocalHost::new(project.to_path_buf(), data.to_path_buf()).with_target_roots(TargetRoots::new(
        home.to_path_buf(),
        home.join(".config"),
        home.join(".claude"),
        home.join(".openclaw"),
        home.join(".hermes"),
        home.join(".kiro"),
    ))
}

fn install_detected(host: &LocalHost, root: &Path, scope: InstallScope) -> Vec<AgentTargetId> {
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Local(source(root))),
        scope,
        targets: vec![],
        mode: None,
        allowed_behaviors: Vec::new(),
    })
    .unwrap();
    host.view("example", scope)
        .unwrap()
        .skill
        .targets
        .iter()
        .map(|target| target.agent)
        .collect()
}

#[test]
fn a_shared_project_directory_selects_only_its_detected_targets() {
    let cases = [
        (
            ".agents/skills",
            vec![AgentTargetId::Codex, AgentTargetId::Amp, AgentTargetId::Zed],
        ),
        (".trae/skills", vec![AgentTargetId::Trae]),
        (".qoder/skills", vec![AgentTargetId::Qoder]),
        (".zencoder/skills", vec![AgentTargetId::Zencoder]),
    ];

    for (directory, expected) in cases {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let home = temporary.path().join("home");
        fs::create_dir_all(project.join(directory)).unwrap();
        fs::create_dir_all(&home).unwrap();
        let host = host_with_home(&project, &temporary.path().join("data"), &home);

        let targets = install_detected(&host, temporary.path(), InstallScope::Project);

        assert_eq!(targets, expected, "{directory}");
    }
}

#[test]
fn a_shared_global_directory_selects_only_its_detected_targets() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(home.join(".agents/skills")).unwrap();
    fs::create_dir_all(home.join(".config/agents/skills")).unwrap();
    fs::create_dir_all(home.join(".zencoder/skills")).unwrap();
    let host = host_with_home(&project, &temporary.path().join("data"), &home);

    let targets = install_detected(&host, temporary.path(), InstallScope::Global);

    assert_eq!(
        targets,
        [
            AgentTargetId::Codex,
            AgentTargetId::Amp,
            AgentTargetId::Zed,
            AgentTargetId::Zencoder,
        ]
    );
}

#[test]
fn an_explicit_only_target_installs_when_named() {
    let cases = [
        (AgentTargetId::Warp, ".agents/skills", ".agents/skills"),
        (
            AgentTargetId::Replit,
            ".config/agents/skills",
            ".agents/skills",
        ),
        (AgentTargetId::TraeCn, ".trae-cn/skills", ".trae/skills"),
        (
            AgentTargetId::Deepagents,
            ".deepagents/agent/skills",
            ".agents/skills",
        ),
    ];

    for (agent, global_dir, project_dir) in cases {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let home = temporary.path().join("home");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&home).unwrap();
        let host = host_with_home(&project, &temporary.path().join("data"), &home);

        for (scope, root, dir) in [
            (InstallScope::Global, &home, global_dir),
            (InstallScope::Project, &project, project_dir),
        ] {
            host.install_request(InstallRequest {
                operation: InstallOperation::Install(InstallSource::Local(source(
                    temporary.path(),
                ))),
                scope,
                targets: vec![agent],
                mode: None,
                allowed_behaviors: Vec::new(),
            })
            .unwrap();

            assert!(
                root.join(dir).join("example/SKILL.md").exists(),
                "{} {:?}",
                agent.as_str(),
                scope
            );
            let view = host.view("example", scope).unwrap();
            assert_eq!(view.skill.targets.len(), 1, "{}", agent.as_str());
            assert_eq!(view.skill.targets[0].agent, agent);
        }
    }
}

#[test]
fn an_installed_agent_marker_selects_its_new_target() {
    let project_cases = [
        (AgentTargetId::Continue, ".continue"),
        (AgentTargetId::Codebuddy, ".codebuddy"),
        (AgentTargetId::QwenCode, ".qwen/skills"),
        (AgentTargetId::TabnineCli, ".tabnine/agent/skills"),
    ];
    for (agent, marker) in project_cases {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let home = temporary.path().join("home");
        fs::create_dir_all(project.join(marker)).unwrap();
        fs::create_dir_all(&home).unwrap();
        let host = host_with_home(&project, &temporary.path().join("data"), &home);

        let targets = install_detected(&host, temporary.path(), InstallScope::Project);

        assert_eq!(targets, [agent], "{marker}");
    }

    let global_cases = [
        (AgentTargetId::QwenCode, ".qwen", ".qwen/skills"),
        (
            AgentTargetId::PositAssistant,
            ".positai",
            ".posit/assistant/skills",
        ),
        (
            AgentTargetId::Devin,
            ".config/devin",
            ".config/devin/skills",
        ),
        (
            AgentTargetId::Cortex,
            ".snowflake/cortex",
            ".snowflake/cortex/skills",
        ),
        (
            AgentTargetId::Crush,
            ".config/crush",
            ".config/crush/skills",
        ),
        (
            AgentTargetId::Kimchi,
            ".config/kimchi",
            ".config/kimchi/harness/skills",
        ),
        (AgentTargetId::Astrbot, ".astrbot", ".astrbot/data/skills"),
    ];
    for (agent, marker, skills_dir) in global_cases {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let home = temporary.path().join("home");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(home.join(marker)).unwrap();
        let host = host_with_home(&project, &temporary.path().join("data"), &home);

        let targets = install_detected(&host, temporary.path(), InstallScope::Global);

        assert_eq!(targets, [agent], "{marker}");
        assert!(
            home.join(skills_dir).join("example/SKILL.md").exists(),
            "{marker}"
        );
    }
}

#[test]
fn a_bare_astrbot_data_directory_alone_does_not_select_astrbot() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let home = temporary.path().join("home");
    fs::create_dir_all(project.join("data/skills")).unwrap();
    fs::create_dir_all(project.join(".cursor")).unwrap();
    fs::create_dir_all(&home).unwrap();
    let host = host_with_home(&project, &temporary.path().join("data"), &home);

    let targets = install_detected(&host, temporary.path(), InstallScope::Project);

    assert_eq!(targets, [AgentTargetId::Cursor]);
    assert!(!project.join("data/skills/example").exists());
}
