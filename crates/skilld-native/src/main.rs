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
use skilld_command::upgrade::InstallChannel;
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
use skilld_native::behavior_prompt::TtyBehaviorConfirmer;
use skilld_native::select_ui::TtySkillChooser;
use skilld_native::update_ui::{
    CommandInteractiveUpdateHost, require_interactive_tty, run_interactive_update,
    write_static_summary,
};
use skilld_native::upgrade::{
    self as cli_upgrade, InstallTarget, LAUNCHER_VARIABLE, NativeReleaseFetcher, WORKER_VARIABLE,
};
use skilld_native::weekly as native_weekly;
use skilld_native::{
    API_URL_VARIABLE, NativeHttpAdapter, TOKEN_VARIABLE, api_origin, token_override,
};
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
    let args = env::args_os().collect::<Vec<_>>();
    let upgrade_session = match start_upgrade(&args) {
        UpgradeStartup::Continue(session) => session,
        UpgradeStartup::Exit(exit) => return exit,
    };
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
    let origin = match api_origin(env::var_os(API_URL_VARIABLE).as_deref()) {
        Ok(origin) => origin,
        Err(error) => {
            eprintln!("{}: {}", error.code, error.message);
            return ExitCode::from(2);
        }
    };
    let token = match token_override(env::var_os(TOKEN_VARIABLE).as_deref()) {
        Ok(token) => token,
        Err(error) => {
            eprintln!("{}: {}", error.code, error.message);
            return ExitCode::from(2);
        }
    };
    let account = Arc::new(
        NativeAccount::new()
            .with_origin(origin.clone())
            .with_token_override(token),
    );
    let auth_command = is_auth_command(args.iter().map(|arg| arg.to_string_lossy()));
    let remote = match SkilldRemote::new(
        Arc::new(NativeHttpAdapter::new()),
        account.clone(),
        native_remote_config(),
    )
    .with_progress(remote_progress)
    .with_endpoint(origin.as_str())
    {
        Ok(remote) => Arc::new(remote),
        Err(error) => {
            eprintln!("{}: {}", error.code, error.message);
            return ExitCode::from(2);
        }
    };
    let host = LocalHost::new(project_root, global_root)
        .with_target_roots(target_roots())
        .with_detection_environment(detection.clone())
        .with_bundled_provider(Arc::new(EmbeddedSkilld::new()))
        .with_account_provider(account.clone())
        .with_api(remote.clone())
        .with_remote_provider(remote);
    // Only a person at a terminal can answer the Skill picker. An Agent, a
    // pipe, or CI requires explicit --all when several Skills are listed.
    let host = if asks_which_skills(&args) {
        host.with_skill_chooser(Arc::new(TtySkillChooser::new(!environment_present(
            "NO_COLOR",
        ))))
    } else {
        host
    };
    // Only a person at a terminal can approve Skill behaviors. An Agent, a
    // pipe, or CI gets the stopped run and its --allow command instead.
    // The interactive update screen owns the terminal, so it never asks here.
    let host = if !interactive && asks_for_behavior_approval(&args) {
        let spinner = status.stopper();
        host.with_behavior_confirmer(Arc::new(TtyBehaviorConfirmer::new(move || spinner.stop())))
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
                    if exit_code == 0
                        && let Some(notice) = account.public_access_notice()
                    {
                        eprintln!("{notice}");
                    }
                    print_upgrade_notice(upgrade_session.as_ref());
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
    if result.exit_code == 0
        && let Some(notice) = account.public_access_notice()
    {
        eprintln!("{notice}");
    }
    print_upgrade_notice(upgrade_session.as_ref());
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

struct UpgradeSession {
    root: PathBuf,
    channel: InstallChannel,
    executable: PathBuf,
}

enum UpgradeStartup {
    Continue(Option<UpgradeSession>),
    Exit(ExitCode),
}

/// Offer the cached release first, then check asynchronously for the next run.
fn start_upgrade(args: &[std::ffi::OsString]) -> UpgradeStartup {
    if environment_enabled("CI")
        || environment_present("SKILLD_NO_UPGRADE")
        || active_agent_detected()
        || !std::io::stdin().is_terminal()
        || !std::io::stdout().is_terminal()
        || !std::io::stderr().is_terminal()
        || args.iter().any(|arg| {
            matches!(
                arg.to_str(),
                Some("--json" | "--plain" | "--help" | "-h" | "--version" | "-V")
            )
        })
    {
        return UpgradeStartup::Continue(None);
    }
    let Ok(executable) = env::current_exe() else {
        return UpgradeStartup::Continue(None);
    };
    let channel = upgrade_channel(&executable);
    if channel == InstallChannel::Unmanaged {
        return UpgradeStartup::Continue(None);
    }
    let root = global_root();
    #[cfg(windows)]
    if channel == InstallChannel::Standalone {
        // An earlier upgrade left the replaced executable here; it is safe to lose.
        let _ = std::fs::remove_file(cli_upgrade::previous_executable(&executable));
    }
    if let Some(notice) = cli_upgrade::available_upgrade(&root, channel, VERSION)
        && !cli_upgrade::version_dismissed(&root, &notice.version)
    {
        let package = match channel {
            InstallChannel::Npm(runner) => {
                cli_upgrade::package_upgrade_args(runner, &executable, &notice.version)
            }
            _ => None,
        };
        let action = match (&channel, &package) {
            (InstallChannel::Standalone, _) => {
                Some("Download and verify the signed release, then upgrade skilld.".to_owned())
            }
            (_, Some((program, args))) => Some(format!("Run {} {}", program, args.join(" "))),
            _ => None,
        };
        if let Some(action) = action {
            use skilld_native::upgrade_ui::{self, UpgradeChoice};
            match upgrade_ui::ask(
                VERSION,
                &notice.version,
                &action,
                !environment_present("NO_COLOR"),
            ) {
                Ok(UpgradeChoice::Later) => {}
                Ok(UpgradeChoice::Dismiss) => {
                    if let Err(error) = cli_upgrade::dismiss_version(&root, &notice.version) {
                        eprintln!("UPGRADE_STATE_FAILED: Cannot save the upgrade choice: {error}");
                    }
                }
                Ok(UpgradeChoice::Upgrade) => {
                    eprintln!("Upgrading skilld to {}...", notice.version);
                    let result = if let Some((program, args)) = package {
                        eprintln!("Running {program} {}", args.join(" "));
                        cli_upgrade::run_package_upgrade(program, &args)
                    } else if let (Some(pin), Some(asset)) =
                        (release_pin(), cli_upgrade::current_release_asset())
                    {
                        cli_upgrade::install_release(
                            &InstallTarget {
                                executable: &executable,
                                current_version: VERSION,
                                asset,
                                pin: &pin,
                            },
                            &NativeReleaseFetcher::new(VERSION),
                            &notice.version,
                        )
                    } else {
                        unreachable!("standalone upgrades require a release key and asset")
                    };
                    return UpgradeStartup::Exit(match result {
                        Ok(()) => {
                            eprintln!("Upgrade complete. Run your command again.");
                            ExitCode::SUCCESS
                        }
                        Err(error) => {
                            eprintln!("{}: {}", error.code, error.message);
                            ExitCode::from(2)
                        }
                    });
                }
                Err(error) => {
                    eprintln!("{error}");
                    return UpgradeStartup::Exit(ExitCode::from(2));
                }
            }
        }
    }
    cli_upgrade::before_command(
        &root,
        &executable,
        channel,
        VERSION,
        cli_upgrade::unix_now(),
    );
    UpgradeStartup::Continue(Some(UpgradeSession {
        root,
        channel,
        executable,
    }))
}

fn run_upgrade_worker(value: &str) {
    let Ok(executable) = env::current_exe() else {
        return;
    };
    let channel = upgrade_channel(&executable);
    cli_upgrade::run_worker(
        value,
        &global_root(),
        channel,
        &NativeReleaseFetcher::new(VERSION),
        cli_upgrade::unix_now(),
    );
}

fn print_upgrade_notice(session: Option<&UpgradeSession>) {
    if let Some(session) = session
        && let Some(notice) =
            cli_upgrade::available_upgrade(&session.root, session.channel, VERSION)
    {
        let guidance = match session.channel {
            InstallChannel::Npm(runner)
                if cli_upgrade::package_upgrade_args(
                    runner,
                    &session.executable,
                    &notice.version,
                )
                .is_none() =>
            {
                if runner == skilld_command::upgrade::PackageRunner::Npx {
                    "Run npx skilld@latest to upgrade."
                } else {
                    "Use your package manager to upgrade this skilld installation."
                }
            }
            _ => "Restart skilld to upgrade.",
        };
        eprintln!(
            "\n{}",
            skilld_native::upgrade_ui::render_banner(
                VERSION,
                &notice.version,
                guidance,
                terminal_width(),
                !environment_present("NO_COLOR")
            )
        );
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
    // command, a throttled notice, and a recent check never pay for the
    // credential read.
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
    // An account holder already receives the weekly, so the notice only ever
    // speaks to a person with no account. A keychain read can fail on a
    // locked or absent store, which cannot tell those people apart, so an
    // unaskable person counts as an account holder too. The completed check
    // is recorded below either way.
    if account.has_account().unwrap_or(true) {
        // Record the completed check so later eligible runs short-circuit
        // before this read, and a failing store pays it at most once a week.
        // A notice that recorded nothing would send every run back to the
        // keychain, so a failed write is worth surfacing rather than
        // swallowing.
        if let Err(error) =
            native_weekly::write_state(data_root, &weekly::record_checked(&state, now))
        {
            eprintln!("SERVICE_UNAVAILABLE: the weekly notice state could not be stored: {error}");
        }
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
                page_url: None,
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

/// Whether `run`, `install`, `add`, or `update` may ask a person to approve
/// Skill behaviors.
fn asks_for_behavior_approval(args: &[std::ffi::OsString]) -> bool {
    args.iter().skip(1).any(|arg| {
        ["run", "install", "add", "update"]
            .iter()
            .any(|command| arg == command)
    }) && !args.iter().any(|arg| arg == "--json" || arg == "--plain")
        && std::io::stdin().is_terminal()
        && std::io::stderr().is_terminal()
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
    use skilld_auth::{
        BoundaryError, CredentialStore, SKILLD_ORIGIN, SecretString, StoredCredential,
    };
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

        fn has_account(&self) -> Result<bool, CommandError> {
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

    /// An account store whose check always fails, like a locked keychain.
    struct FailingAccount {
        checks: AtomicUsize,
    }

    impl FailingAccount {
        fn checks(&self) -> usize {
            self.checks.load(Ordering::SeqCst)
        }
    }

    impl AccountProvider for FailingAccount {
        fn status(&self) -> Result<bool, CommandError> {
            self.has_account()
        }

        fn has_account(&self) -> Result<bool, CommandError> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            Err(CommandError::operation(
                "SERVICE_UNAVAILABLE",
                "the account keychain failed",
            ))
        }

        fn login(&self) -> Result<(), CommandError> {
            Ok(())
        }

        fn logout(&self) -> Result<(), CommandError> {
            Ok(())
        }
    }

    /// A real credential store holding one fixed credential, so the account
    /// check runs the actual stored-credential path.
    struct FixedCredentialStore {
        credential: Option<StoredCredential>,
        loads: AtomicUsize,
    }

    impl FixedCredentialStore {
        fn loads(&self) -> usize {
            self.loads.load(Ordering::SeqCst)
        }
    }

    impl CredentialStore for FixedCredentialStore {
        fn load(&self, _origin: &str) -> Result<Option<StoredCredential>, BoundaryError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(self.credential.clone())
        }

        fn save(&self, _credential: &StoredCredential) -> Result<(), BoundaryError> {
            Ok(())
        }

        fn delete(&self, _origin: &str, _account: &str) -> Result<(), BoundaryError> {
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
                checked_at: 0,
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
    fn a_signed_in_person_reads_credentials_once_then_not_again() {
        let root = tempfile::tempdir().expect("temp dir");
        let account = CountingAccount::signed_in();
        print_weekly_notice(root.path(), &account, eligible());
        let reads_after_first = account.reads();
        let state = native_weekly::read_state(root.path());
        assert!(state.checked_at > 0, "the signed-in check must be recorded");
        assert_eq!(
            state.shown_count, 0,
            "the record must not consume the notice budget"
        );
        print_weekly_notice(root.path(), &account, eligible());
        assert_eq!(
            account.reads(),
            reads_after_first,
            "a recorded signed-in check must skip the credential read on the next run"
        );
    }

    #[test]
    fn a_failing_store_records_its_check_so_the_next_run_skips_the_read() {
        let root = tempfile::tempdir().expect("temp dir");
        let account = FailingAccount {
            checks: AtomicUsize::new(0),
        };
        print_weekly_notice(root.path(), &account, eligible());
        let state = native_weekly::read_state(root.path());
        assert!(
            state.checked_at > 0,
            "a check that could not ask must still be recorded"
        );
        assert_eq!(
            state.shown_count, 0,
            "an unaskable person must not be nagged"
        );
        print_weekly_notice(root.path(), &account, eligible());
        assert_eq!(
            account.checks(),
            1,
            "the recorded check must skip the read on the next run"
        );
    }

    #[test]
    fn an_account_with_an_expired_token_is_never_nagged() {
        let root = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(FixedCredentialStore {
            credential: Some(StoredCredential {
                origin: SKILLD_ORIGIN.to_owned(),
                account: "harlan".to_owned(),
                access_token: SecretString::new("expired"),
                refresh_token: None,
                expires_at: 1,
                scopes: None,
            }),
            loads: AtomicUsize::new(0),
        });
        let account = NativeAccount::with_credentials(store.clone());
        print_weekly_notice(root.path(), &account, eligible());
        let state = native_weekly::read_state(root.path());
        assert_eq!(
            state.shown_count, 0,
            "an account holder already gets the weekly, so nothing may print"
        );
        assert!(state.checked_at > 0, "the account check must be recorded");
        print_weekly_notice(root.path(), &account, eligible());
        assert_eq!(
            store.loads(),
            1,
            "the second run must skip the credential read"
        );
    }
}
