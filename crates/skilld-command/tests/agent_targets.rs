use std::fs;
use std::path::Path;

use skilld_command::{DetectionEnvironment, Host, LocalHost, TargetRoots};
use skilld_core::{AgentTargetId, InstallOperation, InstallRequest, InstallScope, InstallSource};

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
            })
            .unwrap();

        assert_eq!(names, ["example"], "{}", agent.as_str());
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
    ));

    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Local(source)),
        scope: InstallScope::Global,
        targets: vec![],
        mode: None,
    })
    .unwrap();

    assert!(home.join(".agents/skills/example/SKILL.md").exists());
}
