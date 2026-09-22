mod embedded_skill;
mod native_auth;
mod status;

use std::env;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use embedded_skill::EmbeddedSkilld;
use native_auth::NativeAccount;
use skilld_command::AccountProvider;
use skilld_command::upgrade::{InstallChannel, UpgradeNotice};
use skilld_command::weekly::{self, NoticeContext};
use skilld_command::{
    CommandError, CommandPlatform, DetectionEnvironment, Host, InstalledSkill, LocalHost,
    NativeRemoteConfig, OutputContext, SkilldRemote, TargetRoots, interactive_update_requested,
    run_stdio_probe, run_with_output,
};
use skilld_core::{
    InstallScope, InstallSource, ReleasePin, SearchResponse, SearchResult, SourceProvider,
    SourceRequest, SourceSelector, TrustedRootPin, VERSION,
};
use skilld_native::NativeHttpAdapter;
use skilld_native::select_ui::TtySkillChooser;
use skilld_native::update_ui::{
    CommandInteractiveUpdateHost, require_interactive_tty, run_interactive_update,
    write_static_summary,
};
use skilld_native::upgrade::{
    self as cli_upgrade, InstallTarget, LAUNCHER_VARIABLE, NativeReleaseFetcher, WORKER_VARIABLE,
};
use skilld_native::weekly as native_weekly;
use status::StatusLine;
use terminal_size::Width;

fn main() -> ExitCode {
    if env::var_os("SKILLD_PROBE_STDIO").as_deref() == Some(std::ffi::OsStr::new("1")) {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        let mut stderr = std::io::stderr().lock();
        let result = run_stdio_probe(&mut stdin, &mut stdout, &mut stderr);
        return ExitCode::from(result.exit_code);
    }
    if env::var_os("SKILLD_PROBE_SEARCH_OUTPUT").as_deref() == Some(std::ffi::OsStr::new("1")) {
        return run_search_output_probe();
    }

    if let Ok(worker) = env::var(WORKER_VARIABLE) {
        run_upgrade_worker(&worker);
        return ExitCode::SUCCESS;
    }
    let upgrade_notice = start_upgrade();

    let args = env::args_os().collect::<Vec<_>>();
    let interactive = interactive_update_requested(args.clone()).is_ok_and(|requested| requested);
    if interactive
        && let Err(error) = require_interactive_tty(
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
        )
    {
        eprintln!("{error}");
        return ExitCode::from(2);
    }

    let project_root = match env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("SERVICE_UNAVAILABLE: cannot read the current directory: {error}");
            return ExitCode::from(2);
        }
    };
    let global_root = global_root();
    let notice_root = global_root.clone();
    let detection = detection_environment();
    let output = OutputContext::auto(
        std::io::stdout().is_terminal(),
        active_agent_detected(),
        environment_enabled("CI"),
        environment_present("NO_COLOR"),
        env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb")),
        terminal_width(),
        CommandPlatform::current(),
    );
    let label = if interactive {
        None
    } else {
        status::status_label(args.iter().map(|arg| arg.to_string_lossy()))
    };
    let status = match label {
        Some(label) => StatusLine::for_terminal(label, output),
        None => StatusLine::disabled(),
    };
    let remote_progress = status.remote_progress();
    let account = Arc::new(NativeAccount::new());
    let auth_command = is_auth_command(args.iter().map(|arg| arg.to_string_lossy()));
    let host = LocalHost::new(project_root, global_root)
        .with_target_roots(target_roots())
        .with_detection_environment(detection.clone())
        .with_bundled_provider(Arc::new(EmbeddedSkilld::new()))
        .with_account_provider(account.clone())
        .with_remote_provider(Arc::new(
            SkilldRemote::new(
                Arc::new(NativeHttpAdapter::new()),
                account.clone(),
                native_remote_config(),
            )
            .with_progress(remote_progress),
        ));
    // Only a person at a terminal can answer the Skill picker. An Agent, a
    // pipe, or CI installs every Skill the ref names.
    let host = if asks_which_skills(&args) {
        host.with_skill_chooser(Arc::new(TtySkillChooser::new(!environment_present(
            "NO_COLOR",
        ))))
    } else {
        host
    };
    let host = if args.iter().skip(1).any(|arg| arg == "outdated")
        && !args.iter().any(|arg| arg == "--json" || arg == "--plain")
    {
        host.with_outdated_progress(Arc::new(status::OutdatedProgressLine::for_terminal(
            std::io::stderr().is_terminal(),
            active_agent_detected(),
            !environment_present("NO_COLOR"),
        )))
    } else {
        host
    };
    let host = Arc::new(host);

    if interactive {
        let interactive_host = Arc::new(CommandInteractiveUpdateHost::new(host));
        return match run_interactive_update(interactive_host, !environment_present("NO_COLOR")) {
            Ok(summary) => {
                let exit_code = summary.exit_code();
                let mut stdout = std::io::stdout().lock();
                if let Err(error) = write_static_summary(
                    &summary,
                    stdout.is_terminal() && !environment_present("NO_COLOR"),
                    &mut stdout,
                ) {
                    eprintln!("{error}");
                    ExitCode::from(2)
                } else if stdout.flush().is_err() {
                    eprintln!(
                        "TERMINAL_UNAVAILABLE: The Skill update summary could not be written."
                    );
                    ExitCode::from(2)
                } else {
                    print_upgrade_notice(upgrade_notice.as_ref());
                    print_weekly_notice(
                        &notice_root,
                        account.as_ref(),
                        weekly_notice_context(auth_command),
                    );
                    ExitCode::from(exit_code)
                }
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(2)
            }
        };
    }

    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr();
    let mut gated = status::GatedStderr::new(&mut stderr, status);
    let result = run_with_output(args, host.as_ref(), output, &mut stdout, &mut gated);
    gated.finish_status();
    print_upgrade_notice(upgrade_notice.as_ref());
    print_weekly_notice(
        &notice_root,
        account.as_ref(),
        weekly_notice_context(auth_command),
    );
    ExitCode::from(result.exit_code)
}

fn release_pin() -> Option<ReleasePin> {
    option_env!("SKILLD_RELEASE_PUBLIC_KEY").map(|public_key| ReleasePin {
        public_key: public_key.to_owned(),
    })
}

/// The install channel. A standalone build without a release key or a known
/// release asset cannot verify an upgrade, so it never attempts one.
fn upgrade_channel(executable: &std::path::Path) -> InstallChannel {
    match cli_upgrade::install_channel(executable, env::var_os(LAUNCHER_VARIABLE)) {
        InstallChannel::Standalone
            if release_pin().is_none() || cli_upgrade::current_release_asset().is_none() =>
        {
            InstallChannel::Unmanaged
        }
        channel => channel,
    }
}

/// Starts background upgrade work for a person at a terminal.
fn start_upgrade() -> Option<UpgradeNotice> {
    if environment_enabled("CI")
        || environment_present("SKILLD_NO_UPGRADE")
        || active_agent_detected()
        || !std::io::stderr().is_terminal()
    {
        return None;
    }
    let executable = env::current_exe().ok()?;
    let channel = upgrade_channel(&executable);
    #[cfg(windows)]
    if channel == InstallChannel::Standalone {
        // An earlier upgrade left the replaced executable here; it is safe to lose.
        let _ = std::fs::remove_file(cli_upgrade::previous_executable(&executable));
    }
    cli_upgrade::before_command(
        &global_root(),
        &executable,
        channel,
        VERSION,
        cli_upgrade::unix_now(),
    )
}

fn run_upgrade_worker(value: &str) {
    let Ok(executable) = env::current_exe() else {
        return;
    };
    let channel = upgrade_channel(&executable);
    let pin = release_pin();
    let target = match (channel, &pin, cli_upgrade::current_release_asset()) {
        (InstallChannel::Standalone, Some(pin), Some(asset)) => Some(InstallTarget {
            executable: &executable,
            current_version: VERSION,
            asset,
            pin,
        }),
        _ => None,
    };
    cli_upgrade::run_worker(
        value,
        &global_root(),
        channel,
        &NativeReleaseFetcher::new(VERSION),
        target,
        cli_upgrade::unix_now(),
    );
}

fn print_upgrade_notice(notice: Option<&UpgradeNotice>) {
    if let Some(notice) = notice {
        eprintln!("{}", notice.message());
    }
}

/// Tells a signed-out person that an account gets the weekly email.
///
/// It prints to stderr, so a `skilld run` piped into an Agent never carries it.
/// The same conditions as the upgrade check apply: a terminal, no CI, no Agent.
fn print_weekly_notice(
    data_root: &std::path::Path,
    account: &dyn AccountProvider,
    context: NoticeContext,
) {
    let state = native_weekly::read_state(data_root);
    let now = cli_upgrade::unix_now();
    // Every cheaper check returns first, so CI, an Agent, a pipe, an auth
    // command, and a throttled notice never pay for the credential read.
    if !weekly::should_show(
        &state,
        NoticeContext {
            signed_in: false,
            ..context
        },
        now,
    ) {
        return;
    }
    // A keychain read can fail on a locked or absent store. Treat that as
    // signed in, so a person who cannot be asked is never nagged.
    if account.status().unwrap_or(true) {
        return;
    }
    eprintln!("{}", weekly::NOTICE_MESSAGE);
    // A notice that printed but did not record would repeat every run, so a
    // failed write is worth surfacing rather than swallowing.
    if let Err(error) = native_weekly::write_state(data_root, &weekly::record_shown(&state, now)) {
        eprintln!("SERVICE_UNAVAILABLE: the weekly notice state could not be stored: {error}");
    }
}

/// What the environment says about the weekly notice. The credential state is
/// deliberately absent: `print_weekly_notice` reads it only if it can print.
fn weekly_notice_context(auth_command: bool) -> NoticeContext {
    NoticeContext {
        signed_in: false,
        auth_command,
        stderr_terminal: std::io::stderr().is_terminal(),
        stdout_terminal: std::io::stdout().is_terminal(),
        suppressed: environment_enabled("CI")
            || environment_present("SKILLD_NO_WEEKLY")
            || active_agent_detected(),
    }
}

/// Whether the command names the `auth` group, which already covers sign-in.
fn is_auth_command<'a>(args: impl Iterator<Item = std::borrow::Cow<'a, str>>) -> bool {
    args.skip(1)
        .find(|arg| !arg.starts_with('-'))
        .is_some_and(|arg| arg == "auth")
}

fn run_search_output_probe() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let output = OutputContext::auto(
        stdout.is_terminal(),
        active_agent_detected(),
        environment_enabled("CI"),
        environment_present("NO_COLOR"),
        env::var("TERM").is_ok_and(|term| term.eq_ignore_ascii_case("dumb")),
        terminal_width(),
        CommandPlatform::current(),
    );
    let mut args = vec!["skilld", "search", "output"];
    if env::args_os().any(|argument| argument == "--json") {
        args.push("--json");
    }
    if env::args_os().any(|argument| argument == "--plain") {
        args.push("--plain");
    }
    let result = run_with_output(args, &SearchOutputProbe, output, &mut stdout, &mut stderr);
    ExitCode::from(result.exit_code)
}

struct SearchOutputProbe;

impl Host for SearchOutputProbe {
    fn list(&self, _scope: InstallScope) -> Result<Vec<String>, CommandError> {
        unreachable!("list is outside the search output probe")
    }

    fn install(
        &self,
        _source: InstallSource,
        _scope: InstallScope,
    ) -> Result<InstalledSkill, CommandError> {
        unreachable!("install is outside the search output probe")
    }

    fn search(&self, _query: &str) -> Result<SearchResponse, CommandError> {
        Ok(SearchResponse {
            items: vec![SearchResult {
                name: "output-probe".to_owned(),
                description: Some("Checks native terminal output.".to_owned()),
                source: SourceRequest {
                    provider: SourceProvider::Github,
                    owner: "skilld-dev".to_owned(),
                    repository: "skilld".to_owned(),
                    selector: SourceSelector::NamedSkill {
                        name: "output-probe".to_owned(),
                    },
                    r#ref: None,
                },
                stargazer_count: 1,
            }],
            total: 1,
        })
    }
}

fn native_remote_config() -> NativeRemoteConfig {
    match (
        option_env!("SKILLD_ROOT_KEY_ID"),
        option_env!("SKILLD_ROOT_PUBLIC_KEY"),
    ) {
        (Some(key_id), Some(public_key)) => NativeRemoteConfig::Pinned(TrustedRootPin {
            key_id: key_id.to_owned(),
            public_key: public_key.to_owned(),
        }),
        _ => NativeRemoteConfig::Unconfigured,
    }
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
    let openclaw_home = env::var_os("OPENCLAW_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".openclaw"));
    let hermes_home = env::var_os("HERMES_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".hermes"));
    let kiro_home = env::var_os("KIRO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".kiro"));
    TargetRoots::new(
        home,
        config_home,
        claude_home,
        openclaw_home,
        hermes_home,
        kiro_home,
    )
}

fn detection_environment() -> DetectionEnvironment {
    const SIGNALS: [&str; 29] = [
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
        "OPENCLAW_SHELL",
        "OPENCLAW_CLI",
        "OPENCLAW_STATE_DIR",
        "HERMES_AGENT",
        "HERMES_SESSION_ID",
        "HERMES_HOME",
        "KIRO_HOME",
        "AGENT_CONTEXT_OUT",
        "KILO_RUN_ID",
        "KILO_PID",
        "ZED_TERM",
    ];
    DetectionEnvironment::new(
        SIGNALS
            .iter()
            .filter(|name| env::var_os(name).is_some())
            .map(|name| (*name).to_owned()),
    )
}

/// Whether `skilld add` may ask which Skills of a ref to install.
fn asks_which_skills(args: &[std::ffi::OsString]) -> bool {
    args.iter().skip(1).any(|arg| arg == "add")
        && !args
            .iter()
            .any(|arg| arg == "--all" || arg == "--json" || arg == "--plain")
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && !active_agent_detected()
        && !environment_enabled("CI")
}

fn active_agent_detected() -> bool {
    const SIGNALS: [&str; 28] = [
        "CLAUDE_CODE",
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
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
        "OPENCLAW_SHELL",
        "OPENCLAW_CLI",
        "OPENCLAW_STATE_DIR",
        "HERMES_AGENT",
        "HERMES_SESSION_ID",
        "HERMES_HOME",
        "KIRO_HOME",
        "AGENT_CONTEXT_OUT",
        "KILO_RUN_ID",
        "KILO_PID",
        "ZED_TERM",
    ];
    SIGNALS.iter().any(|name| environment_enabled(name))
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

fn environment_enabled(name: &str) -> bool {
    env::var(name).is_ok_and(|value| {
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

fn environment_present(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn terminal_width() -> u16 {
    terminal_size::terminal_size_of(std::io::stdout())
        .map(|(Width(width), _)| width)
        .filter(|width| (20..=240).contains(width))
        .or_else(|| {
            env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|width| (20..=240).contains(width))
        })
        .unwrap_or(80)
}

#[cfg(test)]
mod weekly_notice_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// An account store that counts every credential read.
    struct CountingAccount {
        reads: AtomicUsize,
        signed_in: AtomicBool,
    }

    impl CountingAccount {
        fn signed_out() -> Self {
            Self {
                reads: AtomicUsize::new(0),
                signed_in: AtomicBool::new(false),
            }
        }

        fn signed_in() -> Self {
            Self {
                reads: AtomicUsize::new(0),
                signed_in: AtomicBool::new(true),
            }
        }

        fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }
    }

    impl AccountProvider for CountingAccount {
        fn status(&self) -> Result<bool, CommandError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.signed_in.load(Ordering::SeqCst))
        }

        fn login(&self) -> Result<(), CommandError> {
            Ok(())
        }

        fn logout(&self) -> Result<(), CommandError> {
            Ok(())
        }
    }

    fn eligible() -> NoticeContext {
        NoticeContext {
            signed_in: false,
            auth_command: false,
            stderr_terminal: true,
            stdout_terminal: true,
            suppressed: false,
        }
    }

    #[test]
    fn a_suppressed_run_never_reads_credentials() {
        let root = tempfile::tempdir().expect("temp dir");
        let account = CountingAccount::signed_out();
        print_weekly_notice(
            root.path(),
            &account,
            NoticeContext {
                suppressed: true,
                ..eligible()
            },
        );
        assert_eq!(account.reads(), 0);
    }

    #[test]
    fn a_throttled_out_notice_never_reads_credentials() {
        let root = tempfile::tempdir().expect("temp dir");
        native_weekly::write_state(
            root.path(),
            &weekly::WeeklyNoticeState {
                shown_at: 1,
                shown_count: weekly::NOTICE_LIMIT,
            },
        )
        .expect("state write");
        let account = CountingAccount::signed_out();
        print_weekly_notice(root.path(), &account, eligible());
        assert_eq!(account.reads(), 0);
    }

    #[test]
    fn a_piped_stdout_run_never_reads_credentials() {
        let root = tempfile::tempdir().expect("temp dir");
        let account = CountingAccount::signed_out();
        print_weekly_notice(
            root.path(),
            &account,
            NoticeContext {
                stdout_terminal: false,
                ..eligible()
            },
        );
        assert_eq!(account.reads(), 0);
    }

    #[test]
    fn a_signed_out_person_gets_one_notice_and_a_record() {
        let root = tempfile::tempdir().expect("temp dir");
        let account = CountingAccount::signed_out();
        print_weekly_notice(root.path(), &account, eligible());
        assert_eq!(account.reads(), 1);
        let state = native_weekly::read_state(root.path());
        assert_eq!(state.shown_count, 1);
        assert!(state.shown_at > 0);
    }

    #[test]
    fn a_signed_in_person_gets_no_notice_and_no_record() {
        let root = tempfile::tempdir().expect("temp dir");
        let account = CountingAccount::signed_in();
        print_weekly_notice(root.path(), &account, eligible());
        assert_eq!(account.reads(), 1);
        assert_eq!(
            native_weekly::read_state(root.path()),
            weekly::WeeklyNoticeState::default()
        );
    }
}
