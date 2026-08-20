mod embedded_skill;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use embedded_skill::EmbeddedSkilld;
use skilld_command::{DetectionEnvironment, LocalHost, TargetRoots, run, run_stdio_probe};

fn main() -> ExitCode {
    if env::var_os("SKILLD_PROBE_STDIO").as_deref() == Some(std::ffi::OsStr::new("1")) {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        let mut stderr = std::io::stderr().lock();
        let result = run_stdio_probe(&mut stdin, &mut stdout, &mut stderr);
        return ExitCode::from(result.exit_code);
    }

    let project_root = match env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("SERVICE_UNAVAILABLE: cannot read the current directory: {error}");
            return ExitCode::from(2);
        }
    };
    let global_root = global_root();
    let host = LocalHost::new(project_root, global_root)
        .with_target_roots(target_roots())
        .with_detection_environment(detection_environment())
        .with_bundled_provider(Arc::new(EmbeddedSkilld::new()));

    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let result = run(env::args_os(), &host, &mut stdout, &mut stderr);
    ExitCode::from(result.exit_code)
}

fn target_roots() -> TargetRoots {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let claude_home = env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    TargetRoots::new(home, config_home, claude_home)
}

fn detection_environment() -> DetectionEnvironment {
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
    DetectionEnvironment::new(
        SIGNALS
            .iter()
            .filter(|name| env::var_os(name).is_some())
            .map(|name| (*name).to_owned()),
    )
}

fn global_root() -> PathBuf {
    if let Some(path) = env::var_os("SKILLD_DATA_DIR") {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(path).join("skilld");
    }
    if let Some(path) = env::var_os("HOME") {
        return PathBuf::from(path).join(".skilld");
    }
    PathBuf::from(".skilld")
}
