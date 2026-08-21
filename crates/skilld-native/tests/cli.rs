use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_skilld"))
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/v3-rust/local-skill")
}

fn run(project: &Path, data: &Path, home: &Path, args: &[&str]) -> Output {
    const SIGNALS: [&str; 18] = [
        "CLAUDE_CODE",
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CONFIG_DIR",
        "CURSOR_SESSION",
        "CURSOR_TRACE_ID",
        "WINDSURF_SESSION",
        "CLINE_TASK_ID",
        "CLINE_ACTIVE",
        "COPILOT_RUN_APP",
        "GEMINI_CLI",
        "GOOSE_SESSION",
        "AGENT_SESSION_ID",
        "AMP_SESSION",
        "OPENCODE_SESSION",
        "OPENCODE_SESSION_ID",
        "ROO_SESSION",
        "ANTIGRAVITY_CLI_ALIAS",
    ];
    let mut command = Command::new(binary());
    command
        .current_dir(project)
        .env("SKILLD_DATA_DIR", data)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .args(args);
    for signal in SIGNALS {
        command.env_remove(signal);
    }
    command.output().unwrap()
}

#[test]
fn version_reports_the_rust_package_version() {
    let output = Command::new(binary()).arg("--version").output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("skilld {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn install_help_gives_agents_actionable_source_and_target_grammar() {
    let output = Command::new(binary())
        .args(["install", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout).unwrap();
    for guidance in [
        "skilld:OWNER/REPOSITORY/SKILL",
        "github:OWNER/REPOSITORY/SKILL_PATH",
        "github:OWNER/REPOSITORY/SKILL_PATH#branch:BRANCH",
        "github:OWNER/REPOSITORY/SKILL_PATH#tag:TAG",
        "github:OWNER/REPOSITORY/SKILL_PATH#commit:SHA",
        "https://github.com/OWNER/REPOSITORY/tree/REF/SKILL_PATH",
        "Values: claude-code, cursor, windsurf, cline, codex, github-copilot,",
        "gemini-cli, goose, amp, opencode, roo, antigravity.",
        "Repeat --agent to select more than one Agent target.",
        "Values: copy, symlink. Default: install.mode. Its initial value is copy.",
        "Default: detected Agent targets. If none exist, use agent.targets.",
        "Default scope: current project.",
        "Direct installs set source status to unverified.",
        "Run skilld install without SOURCE to restore .skills/skilld-lock.yaml.",
        "skilld install skilld:skilld-dev/skills/find-skill --agent codex",
        "skilld install github:skilld-dev/skilld/skills/skilld --direct --agent codex",
    ] {
        assert!(help.contains(guidance), "missing help guidance: {guidance}");
    }
}

#[test]
fn direct_local_source_error_gives_an_agent_an_exact_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();

    let output = run(&project, &data, &home, &["install", "./skill", "--direct"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "DIRECT_SOURCE_REQUIRED: --direct cannot install a local Skill. Remove --direct and retry the same command.\n"
    );
}

#[test]
fn direct_bundled_source_error_gives_an_agent_an_exact_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();

    let output = run(&project, &data, &home, &["install", "skilld", "--direct"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "DIRECT_SOURCE_REQUIRED: --direct cannot install the skilld-maintained Skill. Run: skilld install skilld --global\n"
    );
}

#[test]
fn direct_hosted_source_error_gives_an_agent_an_exact_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();

    let output = run(
        &project,
        &data,
        &home,
        &[
            "install",
            "skilld:skilld-dev/skills/find-skill",
            "--direct",
            "--agent",
            "codex",
        ],
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "DIRECT_SOURCE_REQUIRED: --direct requires github:OWNER/REPOSITORY/SKILL_PATH or a GitHub tree URL. Remove --direct and retry the same command.\n"
    );
}

#[test]
fn local_install_list_view_and_remove_use_project_state() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    let install = run(
        &project,
        &data,
        &home,
        &[
            "install",
            fixture().to_str().unwrap(),
            "--agent",
            "codex",
            "--mode",
            "copy",
        ],
    );

    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    assert_eq!(
        String::from_utf8(install.stdout).unwrap(),
        "Installed Skill local-skill.\n"
    );
    assert_eq!(
        fs::read_to_string(project.join(".skills/local-skill/SKILL.md")).unwrap(),
        fs::read_to_string(fixture().join("SKILL.md")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(project.join(".agents/skills/local-skill/SKILL.md")).unwrap(),
        fs::read_to_string(fixture().join("SKILL.md")).unwrap()
    );

    let list = run(&project, &data, &home, &["list"]);
    assert!(list.status.success());
    assert_eq!(String::from_utf8(list.stdout).unwrap(), "local-skill\n");

    let view = run(&project, &data, &home, &["view", "local-skill"]);
    assert!(view.status.success());
    let details = String::from_utf8(view.stdout).unwrap();
    assert!(details.contains("Source status: local\n"));
    assert!(details.contains("Agent targets: codex (copy)\n"));

    let remove = run(&project, &data, &home, &["remove", "local-skill"]);
    assert!(remove.status.success());
    assert_eq!(
        String::from_utf8(remove.stdout).unwrap(),
        "Removed Skill local-skill.\n"
    );
    assert!(!project.join(".skills/local-skill").exists());
    assert!(!project.join(".agents/skills/local-skill").exists());
}

#[test]
fn install_without_a_target_returns_a_typed_result() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();

    let output = run(
        &project,
        &data,
        &home,
        &["install", fixture().to_str().unwrap()],
    );

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "TARGET_REQUIRED: select an Agent target with --agent or configure agent.targets\n"
    );
    assert!(!project.join(".skills").exists());
}

#[test]
fn configured_target_restores_a_missing_target_copy_from_the_lockfile() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    let config = run(
        &project,
        &data,
        &home,
        &["config", "set", "agent.targets", "codex"],
    );
    assert!(config.status.success());
    let get = run(&project, &data, &home, &["config", "get", "agent.targets"]);
    assert_eq!(String::from_utf8(get.stdout).unwrap(), "codex\n");
    let list = run(&project, &data, &home, &["config", "list"]);
    assert_eq!(
        String::from_utf8(list.stdout).unwrap(),
        "agent.targets=codex\ninstall.mode=copy\n"
    );
    let install = run(
        &project,
        &data,
        &home,
        &["install", fixture().to_str().unwrap()],
    );
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    fs::remove_dir_all(project.join(".skills/local-skill")).unwrap();
    fs::remove_dir_all(project.join(".agents")).unwrap();

    let restore = run(&project, &data, &home, &["install"]);

    assert!(
        restore.status.success(),
        "{}",
        String::from_utf8_lossy(&restore.stderr)
    );
    assert_eq!(
        fs::read_to_string(project.join(".agents/skills/local-skill/SKILL.md")).unwrap(),
        fs::read_to_string(fixture().join("SKILL.md")).unwrap()
    );
    assert!(project.join(".skills/local-skill/SKILL.md").exists());
}

#[cfg(unix)]
#[test]
fn lockfile_restore_preserves_symlink_mode() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    assert!(
        run(
            &project,
            &data,
            &home,
            &["config", "set", "agent.targets", "codex"],
        )
        .status
        .success()
    );
    assert!(
        run(
            &project,
            &data,
            &home,
            &["install", fixture().to_str().unwrap(), "--mode", "symlink",],
        )
        .status
        .success()
    );
    let target = project.join(".agents/skills/local-skill");
    fs::remove_file(&target).unwrap();

    let restore = run(&project, &data, &home, &["install"]);

    assert!(
        restore.status.success(),
        "{}",
        String::from_utf8_lossy(&restore.stderr)
    );
    assert!(
        fs::symlink_metadata(target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn an_existing_project_signal_selects_its_agent_target() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(project.join(".cursor")).unwrap();
    fs::create_dir_all(&home).unwrap();

    let output = run(
        &project,
        &data,
        &home,
        &["install", fixture().to_str().unwrap()],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(project.join(".cursor/skills/local-skill/SKILL.md").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn native_auth_status_surfaces_an_unavailable_os_credential_store() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    let output = Command::new(binary())
        .current_dir(&project)
        .env("SKILLD_DATA_DIR", &data)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!(
                "unix:path={}",
                temporary.path().join("missing-bus").display()
            ),
        )
        .args(["auth", "status"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "SERVICE_UNAVAILABLE: The OS keychain operation failed.\n"
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn global_skilld_install_uses_the_global_agent_target() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    let output = Command::new(binary())
        .current_dir(&project)
        .env("SKILLD_DATA_DIR", &data)
        .env("HOME", &home)
        .args(["install", "skilld", "--global", "--agent", "codex"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(data.join("skills/skilld/SKILL.md")).unwrap(),
        include_str!("../../../skills/skilld/SKILL.md")
    );
    assert_eq!(
        fs::read_to_string(home.join(".agents/skills/skilld/SKILL.md")).unwrap(),
        include_str!("../../../skills/skilld/SKILL.md")
    );
    assert!(!project.join(".skills/skilld").exists());
}
