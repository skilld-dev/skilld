mod config;
mod local_store;
mod outdated;
pub use outdated::{NoOutdatedProgress, OutdatedProgress, ancestor_roots};
mod output;
mod remote;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};
pub use config::{ConfigStore, LocalConfig};
pub use local_store::{
    AllowTransaction, LocalStore, PreparedStoreUpdate, ResolvedTarget, SkillView, StoreError,
    TargetInstall, TransactionGate,
};
pub use output::OutputContext;
pub use remote::{
    Cancellation, HeaderValue, HttpAdapter, HttpHeader, HttpMethod, HttpRequest, HttpResponse,
    NativeRemoteConfig, NeverCancelled, NoTokenProvider, PreparedRemoteSkill,
    RemoteComparisonAccess, RemoteComparisonOutcome, RemoteComparisonRelation, RemoteLatestCommit,
    RemoteProvider, RemoteSourceState, RemoteUpdateComparison, RemoteUpdateResult, SecretValue,
    SkilldRemote, Sleeper, ThreadSleeper, TokenProvider,
};
use skilld_core::{
    AGENT_TARGETS, AgentTargetId, CommitHistory, CommitSha, DomainError, GlobalTargetPath,
    InstallMode, InstallOperation, InstallRequest, InstallScope, InstallSource, LockedSource,
    NotTrackedReason, SourceRef, UpdateFailure, UpdateLatestCommit, UpdateModelError, UpdatePlan,
    UpdatePlanItem, UpdatePlanV1, UpdateRelation, UpdateRetryAfter, VERSION,
    classify_update_comparison, select_target_ids,
};
use skilld_ui::{Line, Marker, Screen};

use output::{
    OutputMode, SearchItem, SearchOutcome, render_error, render_search, render_update_check,
    resolve_mode,
};

const DIRECT_SOURCE_GUIDANCE: &str = "--direct requires a github:OWNER/REPOSITORY/SKILL_PATH source or a GitHub tree URL. Remove --direct, then run the same command again.";

#[derive(Debug, Parser)]
#[command(
    name = "skilld",
    version = VERSION,
    about = "Search, install, and keep Skills current",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Output stable JSON for Agents and automation.
    #[arg(long, global = true, conflicts_with = "plain")]
    json: bool,
    /// Output stable text without terminal formatting.
    #[arg(long, global = true, conflicts_with = "json")]
    plain: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Search for Skills.
    Search { query: Vec<String> },
    /// Install a Skill, or restore the Skills recorded in your lockfile.
    #[command(
        long_about = "Install a Skill, or restore the Skills recorded in your lockfile.\n\nGive SOURCE as:\n  skilld:OWNER/REPOSITORY/SKILL\n      Install a hosted Artifact.\n  github:OWNER/REPOSITORY/SKILL_PATH\n  github:OWNER/REPOSITORY/SKILL_PATH#branch:BRANCH\n  github:OWNER/REPOSITORY/SKILL_PATH#tag:TAG\n  github:OWNER/REPOSITORY/SKILL_PATH#commit:SHA\n  https://github.com/OWNER/REPOSITORY/tree/REF/SKILL_PATH\n      Public GitHub Repository paths. Each one requires --direct.\n  ./RELATIVE_PATH or ABSOLUTE_PATH\n      Install a local Skill.\n  skilld\n      Install the skilld-maintained Skill with --global.\n\nRun skilld install without SOURCE to restore .skills/skilld-lock.yaml.\nVerified remote Skills restore the exact locked Git commit.",
        after_long_help = "Examples:\n  skilld install skilld:skilld-dev/skills/find-skill --agent codex\n  skilld install github:skilld-dev/skilld/skills/skilld --direct --agent codex\n  skilld install"
    )]
    Install {
        /// The Skill source to install. Omit SOURCE to restore .skills/skilld-lock.yaml.
        #[arg(value_name = "SOURCE")]
        source: Option<String>,
        #[arg(
            long,
            long_help = "Install to your account-level Agent targets. The default is the current project."
        )]
        global: bool,
        #[arg(
            long = "agent",
            value_name = "AGENT",
            long_help = "Select an Agent target. Repeat --agent to select several.\nValues: claude-code, cursor, windsurf, cline, codex, github-copilot,\n        gemini-cli, goose, amp, opencode, roo, antigravity.\nDefault: every Agent target skilld detects. If skilld detects none, it uses agent.targets."
        )]
        agents: Vec<String>,
        #[arg(
            long,
            value_name = "MODE",
            long_help = "Choose how each Agent target receives the Skill.\nValues: copy, symlink. The default comes from install.mode. A fresh configuration sets install.mode to copy."
        )]
        mode: Option<String>,
        #[arg(
            long,
            long_help = "Fetch a public GitHub Repository without going through skilld.dev.\nGive a github: source or a GitHub tree URL.\nA direct install records the unverified source status."
        )]
        direct: bool,
    },
    /// List installed Skills.
    List {
        #[arg(long)]
        global: bool,
    },
    /// View Skill details.
    View {
        skill: String,
        #[arg(long)]
        global: bool,
    },
    /// Remove an installed Skill.
    Remove {
        skill: String,
        #[arg(long)]
        global: bool,
    },
    /// Update installed Skills.
    Update {
        skill: Option<String>,
        /// Check update relations without changing files.
        #[arg(long)]
        check: bool,
        /// Select Skill updates in a terminal.
        #[arg(
            long,
            conflicts_with_all = ["skill", "check", "json", "plain"]
        )]
        interactive: bool,
        /// Update Skills in the global scope.
        #[arg(long)]
        global: bool,
    },
    /// Verify a Skill source.
    Verify { skill: Option<String> },
    /// Report outdated and unmanaged Skills.
    Outdated {
        /// Check both scopes and every Agent target directory.
        #[arg(long)]
        all: bool,
    },
    /// Manage account authentication.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Manage configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    Login,
    Status,
    Logout,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Get { key: String },
    Set { key: String, value: String },
    List,
}

pub trait Host {
    fn list(&self, scope: InstallScope) -> Result<Vec<String>, CommandError>;

    fn install(&self, source: InstallSource, scope: InstallScope) -> Result<String, CommandError>;

    fn install_request(&self, request: InstallRequest) -> Result<Vec<String>, CommandError> {
        if !request.targets.is_empty() || request.mode.is_some() {
            return Err(CommandError::unsupported_host(
                "Agent target selection is unavailable on this host",
            ));
        }
        let InstallOperation::Install(source) = request.operation else {
            return Err(CommandError::unsupported_host(
                "lockfile restore is unavailable",
            ));
        };
        self.install(source, request.scope).map(|name| vec![name])
    }

    fn view(&self, _name: &str, _scope: InstallScope) -> Result<SkillView, CommandError> {
        Err(CommandError::unsupported_host(
            "Skill details are unavailable on this host",
        ))
    }

    fn remove(&self, _name: &str, _scope: InstallScope) -> Result<(), CommandError> {
        Err(CommandError::unsupported_host(
            "Skill removal is unavailable on this host",
        ))
    }

    fn search(&self, _query: &str) -> Result<skilld_core::SearchResponse, CommandError> {
        Err(CommandError::unsupported_host(
            "Skill search is unavailable on this host",
        ))
    }

    fn verify(&self, _name: Option<&str>) -> Result<Vec<Line>, CommandError> {
        Err(CommandError::unsupported_host(
            "source verification is unavailable on this host",
        ))
    }

    fn update(&self, _name: Option<&str>, _scope: InstallScope) -> Result<Vec<Line>, CommandError> {
        Err(CommandError::unsupported_host(
            "Skill update is unavailable on this host",
        ))
    }

    fn update_selected(&self, items: &[UpdatePlanItem]) -> Result<Vec<Line>, CommandError> {
        validate_update_selection(items)?;
        Err(CommandError::unsupported_host(
            "Selected Skill updates are unavailable on this host",
        ))
    }

    fn update_check(&self, _name: Option<&str>) -> Result<UpdatePlanV1, CommandError> {
        Err(CommandError::unsupported_host(
            "Skill update checks are unavailable on this host",
        ))
    }

    fn outdated(&self, _all: bool) -> Result<Vec<Line>, CommandError> {
        Err(CommandError::unsupported_host(
            "Outdated Skill reports are unavailable on this host",
        ))
    }

    fn config_get(&self, _key: &str) -> Result<String, CommandError> {
        Err(CommandError::unsupported_host(
            "configuration is unavailable on this host",
        ))
    }

    fn config_set(&self, _key: &str, _value: &str) -> Result<(), CommandError> {
        Err(CommandError::unsupported_host(
            "configuration is unavailable on this host",
        ))
    }

    fn config_list(&self) -> Result<Vec<Line>, CommandError> {
        Err(CommandError::unsupported_host(
            "configuration is unavailable on this host",
        ))
    }

    fn auth_status(&self) -> Result<bool, CommandError> {
        Err(CommandError::unsupported_host(
            "credential access is unavailable on this host",
        ))
    }

    fn auth_login(&self) -> Result<(), CommandError> {
        Err(CommandError::unsupported_host(
            "browser access is unavailable on this host",
        ))
    }

    fn auth_logout(&self) -> Result<(), CommandError> {
        Err(CommandError::unsupported_host(
            "credential access is unavailable on this host",
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandErrorKind {
    Usage,
    Operation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub code: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn usage(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: CommandErrorKind::Usage,
            code,
            message: message.into(),
        }
    }

    pub fn operation(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: CommandErrorKind::Operation,
            code,
            message: message.into(),
        }
    }

    pub fn unsupported_host(message: impl Into<String>) -> Self {
        Self::operation("UNSUPPORTED_HOST", message)
    }

    pub fn service(message: impl Into<String>) -> Self {
        Self::operation("SERVICE_UNAVAILABLE", message)
    }

    pub fn not_implemented(message: impl Into<String>) -> Self {
        Self::operation("NOT_IMPLEMENTED", message)
    }

    pub fn input(message: impl Into<String>) -> Self {
        Self::usage("INVALID_SOURCE", message)
    }

    fn direct_local_source() -> Self {
        Self::usage(
            "DIRECT_SOURCE_REQUIRED",
            "--direct cannot install a local Skill. Remove --direct, then run the same command again.",
        )
    }

    fn direct_bundled_source() -> Self {
        Self::usage(
            "DIRECT_SOURCE_REQUIRED",
            "--direct cannot install the skilld-maintained Skill. Run skilld install skilld --global instead",
        )
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::usage("INVALID_CONFIG", message)
    }

    pub fn filesystem(message: impl Into<String>) -> Self {
        Self::operation("SERVICE_UNAVAILABLE", message)
    }

    pub fn domain(error: DomainError) -> Self {
        Self::usage(error.code(), error.to_string())
    }

    pub fn store(error: StoreError) -> Self {
        Self::operation(error.code(), error.to_string())
    }

    pub fn remote(error: skilld_core::RemoteError) -> Self {
        let kind = if matches!(
            error.code,
            "INVALID_SEARCH" | "INVALID_SOURCE" | "DIRECT_SOURCE_REQUIRED"
        ) {
            CommandErrorKind::Usage
        } else {
            CommandErrorKind::Operation
        };
        Self {
            kind,
            code: error.code,
            message: error.message,
        }
    }

    fn exit_code(&self) -> u8 {
        match self.kind {
            CommandErrorKind::Usage => 2,
            CommandErrorKind::Operation => 1,
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandResult {
    pub exit_code: u8,
}

pub fn interactive_update_requested<I, T>(args: I) -> Result<bool, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args).map(|cli| {
        matches!(
            cli.command,
            Command::Update {
                interactive: true,
                ..
            }
        )
    })
}

enum CommandOutput {
    Screen(Screen),
    Search(SearchOutcome),
    UpdateCheck(UpdatePlanV1),
}

pub fn run<I, T, H, O, E>(args: I, host: &H, stdout: &mut O, stderr: &mut E) -> CommandResult
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    H: Host,
    O: Write,
    E: Write,
{
    run_with_output(args, host, OutputContext::Plain, stdout, stderr)
}

pub fn run_with_output<I, T, H, O, E>(
    args: I,
    host: &H,
    context: OutputContext,
    stdout: &mut O,
    stderr: &mut E,
) -> CommandResult
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    H: Host,
    O: Write,
    E: Write,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    let (requested_json, requested_plain) = requested_output(&args);
    let requested_mode = if requested_json && requested_plain {
        OutputMode::Plain
    } else {
        resolve_mode(requested_json, requested_plain, context)
    };
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => {
            let display = matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            let target: &mut dyn Write = if display { stdout } else { stderr };
            let rendered = if requested_mode == OutputMode::JsonV1 {
                if display {
                    output::render_display(
                        error.kind(),
                        &display_path(&args),
                        error.to_string().trim(),
                    )
                } else {
                    let message = error
                        .to_string()
                        .lines()
                        .next()
                        .unwrap_or("invalid command arguments")
                        .trim_start_matches("error: ")
                        .to_owned();
                    render_error(
                        &CommandError::usage("INVALID_ARGUMENT", message),
                        requested_mode,
                    )
                }
            } else {
                error.to_string().into_bytes()
            };
            if target.write_all(&rendered).is_err() {
                return CommandResult {
                    exit_code: if display { 1 } else { 2 },
                };
            }
            return CommandResult {
                exit_code: if display { 0 } else { 2 },
            };
        }
    };

    let mode = resolve_mode(cli.json, cli.plain, context);
    if matches!(&cli.command, Command::Update { check: true, .. }) && mode != OutputMode::JsonV1 {
        let error = CommandError::usage("UNSUPPORTED_OUTPUT", "Skill update checks need --json");
        if stderr.write_all(&render_error(&error, mode)).is_err() {
            return CommandResult { exit_code: 2 };
        }
        return CommandResult { exit_code: 2 };
    }
    let supports_json = matches!(&cli.command, Command::Search { .. })
        || matches!(&cli.command, Command::Update { check: true, .. });
    if mode == OutputMode::JsonV1 && !supports_json {
        let error = CommandError::usage(
            "UNSUPPORTED_OUTPUT",
            "JSON output is available for Skill search and update checks",
        );
        if stderr.write_all(&render_error(&error, mode)).is_err() {
            return CommandResult { exit_code: 2 };
        }
        return CommandResult { exit_code: 2 };
    }

    match dispatch(cli.command, host) {
        Ok(CommandOutput::Screen(screen)) => {
            let bytes = match mode {
                OutputMode::Human { color, .. } => screen.render_human(color),
                OutputMode::Plain | OutputMode::JsonV1 => screen.render_plain(),
            };
            write_success(bytes.as_bytes(), mode, stdout, stderr)
        }
        Ok(CommandOutput::Search(outcome)) => match render_search(&outcome, mode) {
            Ok(bytes) => write_success(&bytes, mode, stdout, stderr),
            Err(error) => {
                if stderr.write_all(&render_error(&error, mode)).is_err() {
                    return CommandResult {
                        exit_code: error.exit_code(),
                    };
                }
                CommandResult {
                    exit_code: error.exit_code(),
                }
            }
        },
        Ok(CommandOutput::UpdateCheck(outcome)) => match render_update_check(&outcome, mode) {
            Ok(bytes) => {
                let exit_code = if outcome.is_incomplete() {
                    2
                } else if outcome.has_changes() {
                    1
                } else {
                    0
                };
                write_success_with_exit(&bytes, mode, stdout, stderr, exit_code)
            }
            Err(error) => {
                if stderr.write_all(&render_error(&error, mode)).is_err() {
                    return CommandResult {
                        exit_code: error.exit_code(),
                    };
                }
                CommandResult {
                    exit_code: error.exit_code(),
                }
            }
        },
        Err(error) => {
            if stderr.write_all(&render_error(&error, mode)).is_err() {
                return CommandResult {
                    exit_code: error.exit_code(),
                };
            }
            CommandResult {
                exit_code: error.exit_code(),
            }
        }
    }
}

fn requested_output(args: &[OsString]) -> (bool, bool) {
    let mut json = false;
    let mut plain = false;
    for argument in args.iter().skip(1) {
        if argument == "--" {
            break;
        }
        if argument == "--json" {
            json = true;
        } else if argument == "--plain" {
            plain = true;
        }
    }
    (json, plain)
}

fn display_path(args: &[OsString]) -> String {
    let commands = [
        "search", "install", "list", "view", "remove", "update", "verify", "auth", "config",
    ];
    let mut path = vec!["skilld"];
    if let Some(command) = args
        .iter()
        .skip(1)
        .filter_map(|argument| argument.to_str())
        .find(|argument| commands.contains(argument))
    {
        path.push(command);
    }
    path.join(" ")
}

fn write_success<O: Write, E: Write>(
    bytes: &[u8],
    mode: OutputMode,
    stdout: &mut O,
    stderr: &mut E,
) -> CommandResult {
    write_success_with_exit(bytes, mode, stdout, stderr, 0)
}

fn write_success_with_exit<O: Write, E: Write>(
    bytes: &[u8],
    mode: OutputMode,
    stdout: &mut O,
    stderr: &mut E,
    exit_code: u8,
) -> CommandResult {
    match stdout.write_all(bytes) {
        Ok(()) => CommandResult { exit_code },
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
            CommandResult { exit_code: 0 }
        }
        Err(_) => {
            let error =
                CommandError::operation("OUTPUT_WRITE_FAILED", "Skill output could not be written");
            let _ = stderr.write_all(&render_error(&error, mode));
            CommandResult { exit_code: 1 }
        }
    }
}

pub fn run_stdio_probe<R: Read, O: Write, E: Write>(
    stdin: &mut R,
    stdout: &mut O,
    stderr: &mut E,
) -> CommandResult {
    let mut input = String::new();
    match stdin.take(4096).read_to_string(&mut input) {
        Ok(_) => {
            if write!(stdout, "stdin:{input}").is_err() || writeln!(stderr, "stderr:probe").is_err()
            {
                return CommandResult { exit_code: 2 };
            }
            CommandResult { exit_code: 0 }
        }
        Err(error) => {
            if writeln!(stderr, "SERVICE_UNAVAILABLE: cannot read stdin: {error}").is_err() {
                return CommandResult { exit_code: 2 };
            }
            CommandResult { exit_code: 2 }
        }
    }
}

fn dispatch<H: Host>(command: Command, host: &H) -> Result<CommandOutput, CommandError> {
    match command {
        Command::Install {
            source,
            global,
            agents,
            mode,
            direct,
        } => {
            let scope = scope(global);
            let operation = match source {
                Some(source) => match (direct, InstallSource::parse(&source)) {
                    (true, InstallSource::Remote(source)) => {
                        InstallOperation::Install(InstallSource::DirectRemote(source))
                    }
                    (true, InstallSource::DirectRemote(source)) => {
                        InstallOperation::Install(InstallSource::DirectRemote(source))
                    }
                    (true, InstallSource::Local(_)) => {
                        return Err(CommandError::direct_local_source());
                    }
                    (true, InstallSource::BundledSkilld) => {
                        return Err(CommandError::direct_bundled_source());
                    }
                    (false, source) => InstallOperation::Install(source),
                },
                None if direct => InstallOperation::DirectRestore,
                None => InstallOperation::Restore,
            };
            if operation == InstallOperation::Install(InstallSource::BundledSkilld)
                && scope != InstallScope::Global
            {
                return Err(CommandError::input(
                    "install the skilld-maintained Skill with --global",
                ));
            }
            let targets = agents
                .iter()
                .map(|agent| AgentTargetId::parse(agent).map_err(CommandError::domain))
                .collect::<Result<Vec<_>, _>>()?;
            let mode = mode
                .as_deref()
                .map(InstallMode::parse)
                .transpose()
                .map_err(CommandError::domain)?;
            let names = host.install_request(InstallRequest {
                operation,
                scope,
                targets,
                mode,
            })?;
            let mut lines = names
                .into_iter()
                .map(|name| Line::success(format!("Installed Skill {name}.")))
                .collect::<Vec<_>>();
            if direct {
                lines.push(Line::hint("Review the unverified Skill before use."));
            }
            Ok(CommandOutput::Screen(Screen::new(lines)))
        }
        Command::List { global } => host.list(scope(global)).map(|names| {
            CommandOutput::Screen(Screen::new(names.into_iter().map(Line::item).collect()))
        }),
        Command::View { skill, global } => render_view(host.view(&skill, scope(global))?)
            .map(|lines| CommandOutput::Screen(Screen::new(lines))),
        Command::Remove { skill, global } => {
            host.remove(&skill, scope(global))?;
            Ok(CommandOutput::Screen(Screen::new(vec![Line::success(
                format!("Removed Skill {skill}."),
            )])))
        }
        Command::Auth {
            command: AuthCommand::Status,
        } => Ok(CommandOutput::Screen(Screen::new(vec![
            if host.auth_status()? {
                Line::success("Authenticated.")
            } else {
                Line::plain("Not authenticated.")
            },
        ]))),
        Command::Auth {
            command: AuthCommand::Login,
        } => {
            host.auth_login()?;
            Ok(CommandOutput::Screen(Screen::new(vec![Line::plain(
                "Authentication started.",
            )])))
        }
        Command::Auth {
            command: AuthCommand::Logout,
        } => {
            host.auth_logout()?;
            Ok(CommandOutput::Screen(Screen::new(vec![Line::success(
                "Logged out.",
            )])))
        }
        Command::Config {
            command: ConfigCommand::Get { key },
        } => Ok(CommandOutput::Screen(Screen::new(vec![Line::plain(
            host.config_get(&key)?,
        )]))),
        Command::Config {
            command: ConfigCommand::Set { key, value },
        } => {
            host.config_set(&key, &value)?;
            Ok(CommandOutput::Screen(Screen::new(vec![Line::success(
                format!("Set {key}."),
            )])))
        }
        Command::Config {
            command: ConfigCommand::List,
        } => host
            .config_list()
            .map(|lines| CommandOutput::Screen(Screen::new(lines))),
        Command::Search { query } => {
            let query = query.join(" ").trim().to_owned();
            if query.is_empty() || query.len() > 200 {
                return Err(CommandError::usage(
                    "INVALID_SEARCH",
                    "Skill search needs a query up to 200 bytes",
                ));
            }
            let response = host.search(&query)?;
            let items = response
                .items
                .into_iter()
                .map(|result| {
                    let selector = result.selector().map_err(CommandError::remote)?;
                    Ok(SearchItem {
                        name: result.name,
                        selector: selector.to_string(),
                        description: result.description,
                        stargazer_count: result.stargazer_count,
                    })
                })
                .collect::<Result<Vec<_>, CommandError>>()?;
            Ok(CommandOutput::Search(SearchOutcome {
                query,
                items,
                total: response.total,
            }))
        }
        Command::Update {
            skill,
            check,
            interactive,
            global,
        } => {
            if interactive {
                Err(CommandError::unsupported_host(
                    "Interactive Skill update needs a native terminal host",
                ))
            } else if check {
                host.update_check(skill.as_deref())
                    .map(CommandOutput::UpdateCheck)
            } else {
                host.update(skill.as_deref(), scope(global))
                    .map(|lines| CommandOutput::Screen(Screen::new(lines)))
            }
        }
        Command::Verify { skill } => host
            .verify(skill.as_deref())
            .map(|lines| CommandOutput::Screen(Screen::new(lines))),
        Command::Outdated { all } => host
            .outdated(all)
            .map(|lines| CommandOutput::Screen(Screen::new(lines))),
    }
}

fn render_view(view: SkillView) -> Result<Vec<Line>, CommandError> {
    let source = match view.skill.source {
        LockedSource::Local { path } => Line::field("Source", format!("local {path}")),
        LockedSource::BundledSkilld => Line::field("Source", "skilld-maintained Skill"),
        LockedSource::Remote { source, .. } => match github_url(&source) {
            Some(url) => Line::linked_field("Source", source, url),
            None => Line::field("Source", source),
        },
    };
    let targets = if view.skill.targets.is_empty() {
        "none".to_owned()
    } else {
        view.skill
            .targets
            .iter()
            .map(|target| format!("{} ({})", target.agent, target.mode.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(vec![
        Line::field("Name", view.name),
        Line::field("Path", view.canonical_path.display().to_string()),
        source,
        Line::field("Source status", view.skill.source_status.as_str()),
        Line::field("Agent targets", targets),
    ])
}

/// A GitHub repository URL for a remote Skill source, when the source names
/// one.
fn github_url(source: &str) -> Option<String> {
    let body = source.split_once(':')?.1;
    let mut segments = body.split('/');
    let owner = segments.next()?;
    let repository = segments.next()?;
    (!owner.is_empty() && !repository.is_empty())
        .then(|| format!("https://github.com/{owner}/{repository}"))
}

fn scope(global: bool) -> InstallScope {
    if global {
        InstallScope::Global
    } else {
        InstallScope::Project
    }
}

#[derive(Clone, Debug, Default)]
pub struct DetectionEnvironment {
    variables: BTreeSet<String>,
}

impl DetectionEnvironment {
    pub fn new(variables: impl IntoIterator<Item = String>) -> Self {
        Self {
            variables: variables.into_iter().collect(),
        }
    }

    fn has(&self, name: &str) -> bool {
        self.variables.contains(name)
    }
}

#[derive(Clone, Debug)]
pub struct TargetRoots {
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub claude_home: PathBuf,
}

impl TargetRoots {
    pub fn new(home: PathBuf, config_home: PathBuf, claude_home: PathBuf) -> Self {
        Self {
            home,
            config_home,
            claude_home,
        }
    }
}

pub trait BundledSkillProvider: Send + Sync {
    fn skilld_source(&self) -> Result<PathBuf, CommandError>;
}

pub trait AccountProvider: Send + Sync {
    fn status(&self) -> Result<bool, CommandError>;
    fn login(&self) -> Result<(), CommandError>;
    fn logout(&self) -> Result<(), CommandError>;
}

#[derive(Clone, Debug)]
struct DirectoryBundledSkillProvider {
    path: PathBuf,
}

impl BundledSkillProvider for DirectoryBundledSkillProvider {
    fn skilld_source(&self) -> Result<PathBuf, CommandError> {
        Ok(self.path.clone())
    }
}

pub struct LocalHost {
    project_root: PathBuf,
    global_root: PathBuf,
    target_roots: TargetRoots,
    detection: DetectionEnvironment,
    bundled_skill: Option<Arc<dyn BundledSkillProvider>>,
    remote: Option<Arc<dyn RemoteProvider>>,
    account: Option<Arc<dyn AccountProvider>>,
    outdated_progress: Arc<dyn outdated::OutdatedProgress>,
}

impl LocalHost {
    pub fn new(project_root: PathBuf, global_root: PathBuf) -> Self {
        let home = global_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| global_root.clone());
        Self {
            project_root,
            global_root,
            target_roots: TargetRoots::new(
                home.clone(),
                home.join(".config"),
                home.join(".claude"),
            ),
            detection: DetectionEnvironment::default(),
            bundled_skill: None,
            remote: None,
            account: None,
            outdated_progress: Arc::new(outdated::NoOutdatedProgress),
        }
    }

    pub fn with_target_roots(mut self, roots: TargetRoots) -> Self {
        self.target_roots = roots;
        self
    }

    pub fn with_detection_environment(mut self, environment: DetectionEnvironment) -> Self {
        self.detection = environment;
        self
    }

    pub fn with_bundled_provider(mut self, provider: Arc<dyn BundledSkillProvider>) -> Self {
        self.bundled_skill = Some(provider);
        self
    }

    pub fn with_bundled_skill(self, path: PathBuf) -> Self {
        self.with_bundled_provider(Arc::new(DirectoryBundledSkillProvider { path }))
    }

    pub fn with_remote_provider(mut self, provider: Arc<dyn RemoteProvider>) -> Self {
        self.remote = Some(provider);
        self
    }

    pub fn with_account_provider(mut self, provider: Arc<dyn AccountProvider>) -> Self {
        self.account = Some(provider);
        self
    }

    pub fn with_outdated_progress(mut self, progress: Arc<dyn outdated::OutdatedProgress>) -> Self {
        self.outdated_progress = progress;
        self
    }

    fn store(&self, scope: InstallScope) -> LocalStore {
        match scope {
            InstallScope::Project => LocalStore::new(self.project_root.join(".skills")),
            InstallScope::Global => LocalStore::new(self.global_root.join("skills")),
        }
    }

    fn config_store(&self) -> ConfigStore {
        ConfigStore::new(&self.global_root)
    }

    fn known_targets(&self, scope: InstallScope) -> Result<Vec<ResolvedTarget>, CommandError> {
        AGENT_TARGETS
            .iter()
            .map(|target| {
                let root = match scope {
                    InstallScope::Project => self.project_root.join(target.project_skills_dir),
                    InstallScope::Global => match target.global_skills_dir {
                        GlobalTargetPath::Home(path) => self.target_roots.home.join(path),
                        GlobalTargetPath::ConfigHome(path) => {
                            self.target_roots.config_home.join(path)
                        }
                        GlobalTargetPath::ClaudeHome(path) => {
                            self.target_roots.claude_home.join(path)
                        }
                    },
                };
                let root = absolute(&root)?;
                ResolvedTarget::new(target.id, root).map_err(CommandError::store)
            })
            .collect()
    }

    fn detected_targets(&self, scope: InstallScope) -> Vec<AgentTargetId> {
        AGENT_TARGETS
            .iter()
            .filter(|target| match scope {
                InstallScope::Project => {
                    self.project_root.join(target.project_skills_dir).exists()
                        || detects_environment(target.id, &self.detection)
                        || detects_project(target.id, &self.project_root)
                }
                InstallScope::Global => {
                    global_target_root(target.global_skills_dir, &self.target_roots).exists()
                        || detects_environment(target.id, &self.detection)
                        || detects_installed(target.id, &self.target_roots)
                }
            })
            .map(|target| target.id)
            .collect()
    }

    fn select_installs(
        &self,
        request: &InstallRequest,
    ) -> Result<(Vec<TargetInstall>, Vec<ResolvedTarget>), CommandError> {
        let config = self.config_store().read()?;
        let detected = self.detected_targets(request.scope);
        let selected = select_target_ids(&request.targets, &detected, &config.agent_targets)
            .map_err(CommandError::domain)?
            .into_targets();
        let mode = request.mode.unwrap_or(config.install_mode);
        let known = self.known_targets(request.scope)?;
        let installs = selected
            .iter()
            .map(|agent| {
                known
                    .iter()
                    .find(|target| target.agent == *agent)
                    .cloned()
                    .map(|target| TargetInstall { target, mode })
                    .ok_or_else(|| {
                        CommandError::domain(DomainError::InvalidTarget(agent.to_string()))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((installs, known))
    }

    fn resolve_source(
        &self,
        source: InstallSource,
    ) -> Result<(PathBuf, LockedSource), CommandError> {
        match source {
            InstallSource::Local(path) => {
                let path = if path.is_absolute() {
                    path
                } else {
                    self.project_root.join(path)
                };
                let recorded = path
                    .to_str()
                    .ok_or_else(|| CommandError::input("local Skill paths must use UTF-8"))?;
                Ok((
                    path.clone(),
                    LockedSource::Local {
                        path: recorded.to_owned(),
                    },
                ))
            }
            InstallSource::BundledSkilld => {
                let provider = self.bundled_skill.as_ref().ok_or_else(|| {
                    CommandError::service(
                        "the bundled search and install Skill is unavailable in this build",
                    )
                })?;
                Ok((provider.skilld_source()?, LockedSource::BundledSkilld))
            }
            InstallSource::Remote(source) => Err(CommandError::not_implemented(format!(
                "Artifact delivery is not implemented yet: {source}"
            ))),
            InstallSource::DirectRemote(source) => Err(CommandError::not_implemented(format!(
                "direct Repository access is not implemented yet: {source}"
            ))),
        }
    }

    fn remote_provider(&self) -> Result<&dyn RemoteProvider, CommandError> {
        self.remote.as_deref().ok_or_else(|| {
            CommandError::service("remote Artifact delivery is unavailable in this build")
        })
    }

    fn install_remote(
        &self,
        source: &str,
        direct: bool,
        scope: InstallScope,
        targets: &[TargetInstall],
        known: &[ResolvedTarget],
    ) -> Result<String, CommandError> {
        let selector = skilld_core::RemoteSelector::parse(source).map_err(CommandError::remote)?;
        let prepared = self
            .remote_provider()?
            .prepare(&selector, direct)
            .map_err(CommandError::remote)?;
        let staged = materialize_remote(&prepared.files)?;
        let name = self
            .store(scope)
            .install_from_with_status(
                staged.path(),
                prepared.locked_source,
                prepared.source_status,
                targets,
                known,
            )
            .map_err(CommandError::store)?;
        Ok(name.to_string())
    }

    fn restore(&self, request: &InstallRequest, direct: bool) -> Result<Vec<String>, CommandError> {
        let (targets, known) = if request.targets.is_empty() {
            (None, self.known_targets(request.scope)?)
        } else {
            let (targets, known) = self.select_installs(request)?;
            (Some(targets), known)
        };
        let store = self.store(request.scope);
        let names = store.list(&known).map_err(CommandError::store)?;
        if names.is_empty() {
            return Err(CommandError::operation(
                "LOCKFILE_NOT_FOUND",
                format!(
                    "no installed Skills exist in {} scope",
                    request.scope.as_str()
                ),
            ));
        }
        let mut restored = Vec::new();
        for name in names {
            let skill_name =
                skilld_core::SkillName::parse(name.clone()).map_err(CommandError::domain)?;
            let view = store
                .view(&skill_name, &known)
                .map_err(CommandError::store)?;
            let restored_targets = if let Some(targets) = &targets {
                targets
                    .iter()
                    .map(|target| {
                        let mode = if request.mode.is_some() {
                            target.mode
                        } else {
                            view.skill
                                .targets
                                .iter()
                                .find(|locked| locked.agent == target.target.agent)
                                .map_or(target.mode, |locked| locked.mode)
                        };
                        TargetInstall {
                            target: target.target.clone(),
                            mode,
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
                view.skill
                    .targets
                    .iter()
                    .map(|locked| {
                        known
                            .iter()
                            .find(|target| target.agent == locked.agent)
                            .cloned()
                            .map(|target| TargetInstall {
                                target,
                                mode: request.mode.unwrap_or(locked.mode),
                            })
                            .ok_or_else(|| {
                                CommandError::domain(DomainError::InvalidTarget(
                                    locked.agent.to_string(),
                                ))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            match view.skill.source {
                LockedSource::Local { path } => {
                    let (source, locked_source) =
                        self.resolve_source(InstallSource::Local(PathBuf::from(path)))?;
                    store
                        .install_from(&source, locked_source, &restored_targets, &known)
                        .map_err(CommandError::store)?;
                }
                LockedSource::BundledSkilld => {
                    let (source, locked_source) =
                        self.resolve_source(InstallSource::BundledSkilld)?;
                    store
                        .install_from(&source, locked_source, &restored_targets, &known)
                        .map_err(CommandError::store)?;
                }
                LockedSource::Remote {
                    source, commit_sha, ..
                } => {
                    let direct = match view.skill.source_status {
                        skilld_core::SourceStatus::Verified { .. } => false,
                        skilld_core::SourceStatus::Unverified { .. } if direct => true,
                        _ => {
                            return Err(CommandError::operation(
                                "UNVERIFIED_SOURCE",
                                "run skilld install --direct to restore an unverified Skill"
                                    .to_owned(),
                            ));
                        }
                    };
                    let selector = skilld_core::RemoteSelector::parse(&source)
                        .map_err(CommandError::remote)?;
                    let mut exact_source = selector.source().clone();
                    exact_source.r#ref = Some(skilld_core::SourceRef::Commit { value: commit_sha });
                    let exact = match selector {
                        skilld_core::RemoteSelector::Skilld(_) => {
                            skilld_core::RemoteSelector::Skilld(exact_source)
                        }
                        skilld_core::RemoteSelector::Github(_) => {
                            skilld_core::RemoteSelector::Github(exact_source)
                        }
                    };
                    let prepared = self
                        .remote_provider()?
                        .prepare(&exact, direct)
                        .map_err(CommandError::remote)?;
                    let staged = materialize_remote(&prepared.files)?;
                    store
                        .install_from_with_status(
                            staged.path(),
                            prepared.locked_source,
                            prepared.source_status,
                            &restored_targets,
                            &known,
                        )
                        .map_err(CommandError::store)?;
                }
            }
            restored.push(name);
        }
        Ok(restored)
    }
}

impl Host for LocalHost {
    fn list(&self, scope: InstallScope) -> Result<Vec<String>, CommandError> {
        let known = self.known_targets(scope)?;
        self.store(scope).list(&known).map_err(CommandError::store)
    }

    fn install(&self, source: InstallSource, scope: InstallScope) -> Result<String, CommandError> {
        self.install_request(InstallRequest {
            operation: InstallOperation::Install(source),
            scope,
            targets: vec![],
            mode: None,
        })?
        .into_iter()
        .next()
        .ok_or_else(|| CommandError::service("Skill install returned no result"))
    }

    fn install_request(&self, request: InstallRequest) -> Result<Vec<String>, CommandError> {
        let source = match request.operation.clone() {
            InstallOperation::Restore => return self.restore(&request, false),
            InstallOperation::DirectRestore => return self.restore(&request, true),
            InstallOperation::Install(source) => source,
        };
        let (targets, known) = self.select_installs(&request)?;
        match source {
            InstallSource::Remote(source) => self
                .install_remote(&source, false, request.scope, &targets, &known)
                .map(|name| vec![name]),
            InstallSource::DirectRemote(source) => self
                .install_remote(&source, true, request.scope, &targets, &known)
                .map(|name| vec![name]),
            source => {
                let (source, locked_source) = self.resolve_source(source)?;
                let name = self
                    .store(request.scope)
                    .install_from(&source, locked_source, &targets, &known)
                    .map_err(CommandError::store)?;
                Ok(vec![name.to_string()])
            }
        }
    }

    fn view(&self, name: &str, scope: InstallScope) -> Result<SkillView, CommandError> {
        let known = self.known_targets(scope)?;
        let name = skilld_core::SkillName::parse(name.to_owned()).map_err(CommandError::domain)?;
        self.store(scope)
            .view(&name, &known)
            .map_err(CommandError::store)
    }

    fn remove(&self, name: &str, scope: InstallScope) -> Result<(), CommandError> {
        let known = self.known_targets(scope)?;
        let name = skilld_core::SkillName::parse(name.to_owned()).map_err(CommandError::domain)?;
        self.store(scope)
            .remove(&name, &known)
            .map_err(CommandError::store)
    }

    fn config_get(&self, key: &str) -> Result<String, CommandError> {
        self.config_store().read()?.get(key)
    }

    fn config_set(&self, key: &str, value: &str) -> Result<(), CommandError> {
        let store = self.config_store();
        let mut config = store.read()?;
        config.set(key, value)?;
        store.write(&config)
    }

    fn config_list(&self) -> Result<Vec<Line>, CommandError> {
        Ok(self.config_store().read()?.entries())
    }

    fn auth_status(&self) -> Result<bool, CommandError> {
        self.account
            .as_deref()
            .ok_or_else(|| CommandError::unsupported_host("credential access is unavailable"))?
            .status()
    }

    fn auth_login(&self) -> Result<(), CommandError> {
        self.account
            .as_deref()
            .ok_or_else(|| CommandError::unsupported_host("browser access is unavailable"))?
            .login()
    }

    fn auth_logout(&self) -> Result<(), CommandError> {
        self.account
            .as_deref()
            .ok_or_else(|| CommandError::unsupported_host("credential access is unavailable"))?
            .logout()
    }

    fn search(&self, query: &str) -> Result<skilld_core::SearchResponse, CommandError> {
        self.remote_provider()?
            .search(query, 20)
            .map_err(CommandError::remote)
    }

    fn verify(&self, requested: Option<&str>) -> Result<Vec<Line>, CommandError> {
        let scope = InstallScope::Project;
        let known = self.known_targets(scope)?;
        let store = self.store(scope);
        let names = selected_names(&store, &known, requested)?;
        let mut lines = Vec::new();
        for name in names {
            let name = skilld_core::SkillName::parse(name).map_err(CommandError::domain)?;
            let view = store
                .verify_content(&name, &known)
                .map_err(|error| match error {
                    StoreError::Conflict(message) => {
                        CommandError::operation("CONTENT_CHANGED", message)
                    }
                    error => CommandError::store(error),
                })?;
            match (&view.skill.source, &view.skill.source_status) {
                (
                    LockedSource::Remote {
                        source, commit_sha, ..
                    },
                    skilld_core::SourceStatus::Verified { artifact_id, .. },
                ) => {
                    let selector =
                        skilld_core::RemoteSelector::parse(source).map_err(CommandError::remote)?;
                    match self
                        .remote_provider()?
                        .source_state(&selector, artifact_id, commit_sha)
                        .map_err(CommandError::remote)?
                    {
                        RemoteSourceState::Current => {
                            lines.push(Line::success(format!("Verified Skill {}.", name.as_str())));
                        }
                        RemoteSourceState::Stale { .. } => {
                            return Err(CommandError::operation(
                                "SOURCE_STALE",
                                format!("Skill {} has a newer or changed source", name.as_str()),
                            ));
                        }
                    }
                }
                (_, skilld_core::SourceStatus::Unverified { .. }) => {
                    return Err(CommandError::operation(
                        "UNVERIFIED_SOURCE",
                        format!("Skill {} has an unverified source", name.as_str()),
                    ));
                }
                _ => lines.push(Line::plain(format!(
                    "Checked local Skill {}.",
                    name.as_str()
                ))),
            }
        }
        Ok(lines)
    }

    fn update(
        &self,
        requested: Option<&str>,
        scope: InstallScope,
    ) -> Result<Vec<Line>, CommandError> {
        let known = self.known_targets(scope)?;
        let store = self.store(scope);
        let names = selected_names(&store, &known, requested)?;
        let provider = self.remote_provider()?;
        let mut pending = Vec::new();
        for name in names {
            let skill_name =
                skilld_core::SkillName::parse(name.clone()).map_err(CommandError::domain)?;
            let view = store
                .verify_content(&skill_name, &known)
                .map_err(CommandError::store)?;
            let LockedSource::Remote { source, .. } = &view.skill.source else {
                continue;
            };
            if !matches!(
                view.skill.source_status,
                skilld_core::SourceStatus::Verified { .. }
            ) {
                return Err(CommandError::operation(
                    "UNVERIFIED_SOURCE",
                    format!("Skill {name} needs another explicit --direct install"),
                ));
            }
            let selector =
                skilld_core::RemoteSelector::parse(source).map_err(CommandError::remote)?;
            if matches!(selector.source().r#ref, Some(SourceRef::Commit { .. })) {
                continue;
            }
            let latest_commit = provider
                .latest_commit(&selector, false)
                .map_err(CommandError::remote)?;
            let LockedSource::Remote { commit_sha, .. } = &view.skill.source else {
                unreachable!("the update candidate has a remote source")
            };
            let locked_commit_sha =
                CommitSha::parse(commit_sha.clone()).map_err(update_model_error)?;
            if latest_commit.commit_sha == locked_commit_sha {
                continue;
            }
            let comparison = RemoteUpdateComparison::new(
                skill_name.as_str(),
                &selector.source().owner,
                &selector.source().repository,
                locked_commit_sha,
                latest_commit.commit_sha.clone(),
                latest_commit.access,
            )
            .map_err(CommandError::remote)?;
            pending.push(PendingUpdateApply {
                name,
                skill_name,
                view,
                selector,
                expected_commit: latest_commit.commit_sha,
                comparison,
            });
        }
        if pending.is_empty() {
            return Ok(vec![]);
        }
        let comparisons = pending
            .iter()
            .map(|pending| pending.comparison.clone())
            .collect::<Vec<_>>();
        let results = provider
            .compare_updates(&comparisons)
            .map_err(CommandError::remote)?;
        if results.len() != pending.len() {
            return Err(CommandError::service(
                "Update comparison results were incomplete",
            ));
        }
        let mut selected = Vec::new();
        for (pending, result) in pending.into_iter().zip(results) {
            if pending.comparison.id != result.id {
                return Err(CommandError::service(
                    "Update comparison results changed order",
                ));
            }
            match result.outcome {
                RemoteComparisonOutcome::Ready {
                    relation: RemoteComparisonRelation::Ahead,
                    total,
                    ..
                } if total > 0 => {}
                RemoteComparisonOutcome::Ready {
                    relation: RemoteComparisonRelation::Behind,
                    ..
                } => {
                    return Err(CommandError::operation(
                        "UPDATE_CONFIRMATION_REQUIRED",
                        format!(
                            "Skill {} needs interactive confirmation because its source moved behind",
                            pending.name
                        ),
                    ));
                }
                RemoteComparisonOutcome::Ready {
                    relation: RemoteComparisonRelation::Diverged,
                    ..
                } => {
                    return Err(CommandError::operation(
                        "UPDATE_CONFIRMATION_REQUIRED",
                        format!(
                            "Skill {} needs interactive confirmation because its source diverged",
                            pending.name
                        ),
                    ));
                }
                RemoteComparisonOutcome::Ready { .. } => {
                    return Err(CommandError::operation(
                        "INVALID_RESPONSE",
                        "GitHub returned an impossible update relation",
                    ));
                }
                outcome => return Err(update_apply_failure(&pending.name, outcome)),
            }
            let prepared = provider
                .prepare_exact(&pending.selector, &pending.expected_commit, false)
                .map_err(CommandError::remote)?;
            let staged = materialize_remote(&prepared.files)?;
            let staged_name =
                skilld_core::SkillName::from_source(staged.path()).map_err(CommandError::domain)?;
            if staged_name != pending.skill_name {
                return Err(CommandError::operation(
                    "SOURCE_MISMATCH",
                    format!("the updated Skill name changed from {}", pending.name),
                ));
            }
            let targets = pending
                .view
                .skill
                .targets
                .iter()
                .map(|locked| {
                    known
                        .iter()
                        .find(|target| target.agent == locked.agent)
                        .cloned()
                        .map(|target| TargetInstall {
                            target,
                            mode: locked.mode,
                        })
                        .ok_or_else(|| {
                            CommandError::domain(DomainError::InvalidTarget(
                                locked.agent.to_string(),
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            selected.push(PreparedUpdateSelection {
                name: pending.name,
                staged,
                prepared,
                targets,
                expected_transaction_id: pending.view.transaction_id,
                expected_skill: pending.view.skill,
            });
        }
        let updates = selected
            .iter()
            .map(|selection| PreparedStoreUpdate {
                source: selection.staged.path().to_owned(),
                locked_source: selection.prepared.locked_source.clone(),
                source_status: Some(selection.prepared.source_status.clone()),
                targets: selection.targets.clone(),
                expected_transaction_id: selection.expected_transaction_id.clone(),
                expected_skill: selection.expected_skill.clone(),
            })
            .collect();
        store
            .apply_update_batch(updates, &known)
            .map_err(CommandError::store)?;
        Ok(selected
            .into_iter()
            .map(|selection| Line::success(format!("Updated Skill {}.", selection.name)))
            .collect())
    }

    fn update_selected(&self, items: &[UpdatePlanItem]) -> Result<Vec<Line>, CommandError> {
        validate_update_selection(items)?;
        let scope = InstallScope::Project;
        let known = self.known_targets(scope)?;
        let store = self.store(scope);
        apply_update_selection(self, items, store, known)
    }

    fn update_check(&self, requested: Option<&str>) -> Result<UpdatePlanV1, CommandError> {
        let scope = InstallScope::Project;
        let known = self.known_targets(scope)?;
        let store = self.store(scope);
        let names = selected_names(&store, &known, requested)?;
        let mut items = Vec::with_capacity(names.len());
        let mut pending = Vec::new();
        for name in names {
            let skill_name = skilld_core::SkillName::parse(name).map_err(CommandError::domain)?;
            let view = store
                .verify_content(&skill_name, &known)
                .map_err(CommandError::store)?;
            let (source, locked_commit_sha) = match &view.skill.source {
                LockedSource::Local { .. } => {
                    items.push(UpdatePlanItem::new(
                        skill_name,
                        UpdateRelation::NotTracked {
                            reason: NotTrackedReason::Local,
                        },
                    ));
                    continue;
                }
                LockedSource::BundledSkilld => {
                    items.push(UpdatePlanItem::new(
                        skill_name,
                        UpdateRelation::NotTracked {
                            reason: NotTrackedReason::Bundled,
                        },
                    ));
                    continue;
                }
                LockedSource::Remote {
                    source, commit_sha, ..
                } => (
                    source,
                    CommitSha::parse(commit_sha.clone()).map_err(update_model_error)?,
                ),
            };
            let selector = match skilld_core::RemoteSelector::parse(source) {
                Ok(selector) => selector,
                Err(error) => {
                    items.push(UpdatePlanItem::new(
                        skill_name,
                        unavailable_update(
                            locked_commit_sha,
                            UpdateLatestCommit::Unknown,
                            error.code,
                            error.message,
                        ),
                    ));
                    continue;
                }
            };
            if let Some(SourceRef::Commit { value }) = &selector.source().r#ref {
                let pinned = CommitSha::parse(value.clone()).map_err(update_model_error)?;
                let relation = if pinned == locked_commit_sha {
                    UpdateRelation::Pinned {
                        commit_sha: locked_commit_sha,
                    }
                } else {
                    unavailable_update(
                        locked_commit_sha,
                        UpdateLatestCommit::Known { commit_sha: pinned },
                        "INVALID_LOCKFILE",
                        "The locked commit differs from its source selector",
                    )
                };
                items.push(UpdatePlanItem::new(skill_name, relation));
                continue;
            }
            let direct = match &view.skill.source_status {
                skilld_core::SourceStatus::Verified { .. } => false,
                skilld_core::SourceStatus::Unverified { .. } => true,
                skilld_core::SourceStatus::Local { .. } => {
                    items.push(UpdatePlanItem::new(
                        skill_name,
                        unavailable_update(
                            locked_commit_sha,
                            UpdateLatestCommit::Unknown,
                            "INVALID_LOCKFILE",
                            "A remote Skill has a local source status",
                        ),
                    ));
                    continue;
                }
            };
            let latest = match self.remote_provider().and_then(|provider| {
                provider
                    .latest_commit(&selector, direct)
                    .map_err(CommandError::remote)
            }) {
                Ok(latest) => latest,
                Err(error) => {
                    items.push(UpdatePlanItem::new(
                        skill_name,
                        unavailable_update(
                            locked_commit_sha,
                            UpdateLatestCommit::Unknown,
                            error.code,
                            error.message,
                        ),
                    ));
                    continue;
                }
            };
            if latest.commit_sha == locked_commit_sha {
                items.push(UpdatePlanItem::new(
                    skill_name,
                    UpdateRelation::Current {
                        commit_sha: locked_commit_sha,
                    },
                ));
                continue;
            }
            let comparison = RemoteUpdateComparison::new(
                skill_name.as_str(),
                &selector.source().owner,
                &selector.source().repository,
                locked_commit_sha.clone(),
                latest.commit_sha.clone(),
                latest.access,
            )
            .map_err(CommandError::remote)?;
            pending.push(PendingUpdateComparison {
                name: skill_name,
                locked_commit_sha,
                latest_commit_sha: latest.commit_sha,
                comparison,
            });
        }
        if !pending.is_empty() {
            let comparisons = pending
                .iter()
                .map(|pending| pending.comparison.clone())
                .collect::<Vec<_>>();
            let results = self
                .remote_provider()?
                .compare_updates(&comparisons)
                .map_err(CommandError::remote)?;
            if results.len() != pending.len() {
                return Err(CommandError::service(
                    "Update comparison results were incomplete",
                ));
            }
            for (pending, result) in pending.into_iter().zip(results) {
                if pending.comparison.id != result.id {
                    return Err(CommandError::service(
                        "Update comparison results changed order",
                    ));
                }
                items.push(update_plan_item(pending, result.outcome));
            }
        }
        let plan = UpdatePlan::new(items).map_err(update_model_error)?;
        Ok(UpdatePlanV1::new(plan))
    }

    fn outdated(&self, all: bool) -> Result<Vec<Line>, CommandError> {
        let scopes = if all {
            vec![InstallScope::Project, InstallScope::Global]
        } else {
            vec![InstallScope::Project]
        };
        let progress = self.outdated_progress.as_ref();
        let mut lines: Vec<Line> = Vec::new();
        let mut managed = BTreeMap::<String, Vec<PathBuf>>::new();
        let mut store_roots = Vec::new();
        let mut scan = Vec::new();
        let mut suppressed_roots = BTreeSet::new();
        let mut views = Vec::new();
        for scope in scopes {
            let known = self.known_targets(scope)?;
            let store = self.store(scope);
            let names = match store.list(&known) {
                Ok(names) => names,
                Err(error) => {
                    // Without a readable lockfile, managed copies cannot be told from unmanaged ones.
                    lines.push(Line::error(format!(
                        "Skill store unavailable in {} scope: {}",
                        scope.as_str(),
                        CommandError::store(error).message
                    )));
                    if all {
                        // The ancestor scan must not report Skills this scope cannot verify.
                        suppressed_roots.extend(known.iter().map(|target| target.root.clone()));
                    }
                    continue;
                }
            };
            for name in names {
                let skill_name =
                    skilld_core::SkillName::parse(name.clone()).map_err(CommandError::domain)?;
                let view = match store.view(&skill_name, &known) {
                    Ok(view) => view,
                    Err(error) => {
                        lines.push(Line::error(format!(
                            "Skill {name} details unavailable: {}",
                            CommandError::store(error).message
                        )));
                        continue;
                    }
                };
                let mut paths = vec![view.canonical_path.clone()];
                for locked in &view.skill.targets {
                    if let Some(target) = known.iter().find(|target| target.agent == locked.agent) {
                        paths.push(target.root.join(name.as_str()));
                    }
                }
                managed
                    .entry(name.clone())
                    .or_default()
                    .extend(paths.iter().cloned());
                progress.found(&format!("{name} ({} scope)", scope.as_str()));
                views.push((view, scope));
            }
            if all {
                store_roots.push(store.root().to_path_buf());
                scan.push((scope, known));
            }
        }
        if all {
            // Global Agent directories keep the global scope, so ancestor roots
            // scan after them and skip roots another scope already claimed.
            let mut claimed = scan
                .iter()
                .flat_map(|(_, targets)| targets.iter().map(|target| target.root.clone()))
                .collect::<BTreeSet<_>>();
            claimed.extend(suppressed_roots);
            for root in outdated::ancestor_roots(&self.project_root, &self.target_roots.home) {
                for target in skilld_core::AGENT_TARGETS {
                    if let Ok(resolved) =
                        ResolvedTarget::new(target.id, root.join(target.project_skills_dir))
                        && !claimed.contains(&resolved.root)
                    {
                        scan.push((InstallScope::Project, vec![resolved]));
                    }
                }
            }
        }
        let unmanaged = if all {
            outdated::scan_unmanaged(&scan, &store_roots, &managed)
        } else {
            Vec::new()
        };
        for skill in &unmanaged {
            progress.found(&outdated::found_line(skill));
        }
        for (view, scope) in &views {
            progress.checking(&view.name);
            lines.extend(self.report_outdated_view(view, *scope));
        }
        if all {
            #[cfg(not(target_os = "wasi"))]
            let results = search_candidates_parallel(self, &unmanaged);
            #[cfg(target_os = "wasi")]
            let results = unmanaged
                .iter()
                .map(|skill| self.search_candidate(&skill.name))
                .collect::<Vec<_>>();
            let mut no_match = Vec::new();
            let mut failures = BTreeMap::<String, Vec<&outdated::UnmanagedSkill>>::new();
            for (skill, result) in unmanaged.iter().zip(results) {
                match result {
                    Ok(Some(candidate)) => {
                        lines.extend(outdated::render_unmanaged(skill, Some(&candidate)))
                    }
                    Ok(None) => no_match.push(skill),
                    Err(error) => failures.entry(error.message).or_default().push(skill),
                }
            }
            lines.extend(outdated::render_no_match(&no_match));
            lines.extend(outdated::render_search_failures(&failures));
        }
        progress.finish();
        if lines.is_empty() {
            lines.push(Line::plain("No installed Skills found."));
        }
        Ok(lines)
    }
}

#[cfg(not(target_os = "wasi"))]
fn search_candidates_parallel(
    host: &LocalHost,
    skills: &[outdated::UnmanagedSkill],
) -> Vec<Result<Option<outdated::SkillCandidate>, CommandError>> {
    use std::sync::Mutex;

    const MAX_CONCURRENT_SEARCHES: usize = 8;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots = Mutex::new(
        skills
            .iter()
            .map(|_| None)
            .collect::<Vec<Option<Result<Option<outdated::SkillCandidate>, CommandError>>>>(),
    );
    if !skills.is_empty() {
        std::thread::scope(|scope| {
            let workers = skills.len().min(MAX_CONCURRENT_SEARCHES);
            let mut handles = Vec::with_capacity(workers);
            for _ in 0..workers {
                handles.push(scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if index >= skills.len() {
                            break;
                        }
                        host.outdated_progress.checking(&skills[index].name);
                        let result = host.search_candidate(&skills[index].name);
                        slots.lock().unwrap()[index] = Some(result);
                    }
                }));
            }
            for handle in handles {
                let _ = handle.join();
            }
        });
    }
    let filled = slots.into_inner().unwrap();
    filled
        .into_iter()
        .map(|slot| slot.expect("every search slot is filled"))
        .collect()
}

struct PendingUpdateComparison {
    name: skilld_core::SkillName,
    locked_commit_sha: CommitSha,
    latest_commit_sha: CommitSha,
    comparison: RemoteUpdateComparison,
}

struct PendingUpdateApply {
    name: String,
    skill_name: skilld_core::SkillName,
    view: SkillView,
    selector: skilld_core::RemoteSelector,
    expected_commit: CommitSha,
    comparison: RemoteUpdateComparison,
}

struct PreparedUpdateSelection {
    name: String,
    staged: StagedRemote,
    prepared: PreparedRemoteSkill,
    targets: Vec<TargetInstall>,
    expected_transaction_id: String,
    expected_skill: skilld_core::LockedSkill,
}

fn apply_update_selection(
    host: &LocalHost,
    items: &[UpdatePlanItem],
    store: LocalStore,
    known: Vec<ResolvedTarget>,
) -> Result<Vec<Line>, CommandError> {
    let provider = host.remote_provider()?;
    let mut pending = Vec::new();
    for item in items {
        let name = item.name().as_str().to_owned();
        let UpdateRelation::Available {
            locked_commit_sha,
            latest_commit_sha,
            ..
        } = item.relation()
        else {
            unreachable!("the update selection was validated")
        };
        let skill_name = item.name().clone();
        let view = store
            .verify_content(&skill_name, &known)
            .map_err(CommandError::store)?;
        let LockedSource::Remote {
            source, commit_sha, ..
        } = &view.skill.source
        else {
            return Err(stale_update_plan(&name));
        };
        if !matches!(
            view.skill.source_status,
            skilld_core::SourceStatus::Verified { .. }
        ) {
            return Err(CommandError::operation(
                "UNVERIFIED_SOURCE",
                format!("Skill {name} needs another explicit --direct install"),
            ));
        }
        let selector = skilld_core::RemoteSelector::parse(source).map_err(CommandError::remote)?;
        if matches!(selector.source().r#ref, Some(SourceRef::Commit { .. })) {
            return Err(stale_update_plan(&name));
        }
        let installed_commit_sha =
            CommitSha::parse(commit_sha.clone()).map_err(update_model_error)?;
        if &installed_commit_sha != locked_commit_sha {
            return Err(stale_update_plan(&name));
        }
        let latest_commit = provider
            .latest_commit(&selector, false)
            .map_err(CommandError::remote)?;
        if &latest_commit.commit_sha != latest_commit_sha {
            return Err(stale_update_plan(&name));
        }
        let comparison = RemoteUpdateComparison::new(
            skill_name.as_str(),
            &selector.source().owner,
            &selector.source().repository,
            locked_commit_sha.clone(),
            latest_commit_sha.clone(),
            latest_commit.access,
        )
        .map_err(CommandError::remote)?;
        pending.push(PendingUpdateApply {
            name,
            skill_name,
            view,
            selector,
            expected_commit: latest_commit_sha.clone(),
            comparison,
        });
    }
    if pending.is_empty() {
        return Ok(vec![]);
    }
    let comparisons = pending
        .iter()
        .map(|pending| pending.comparison.clone())
        .collect::<Vec<_>>();
    let results = provider
        .compare_updates(&comparisons)
        .map_err(CommandError::remote)?;
    if results.len() != pending.len() {
        return Err(CommandError::service(
            "Update comparison results were incomplete",
        ));
    }
    let mut selected = Vec::new();
    for (pending, result) in pending.into_iter().zip(results) {
        if pending.comparison.id != result.id {
            return Err(CommandError::service(
                "Update comparison results changed order",
            ));
        }
        match result.outcome {
            RemoteComparisonOutcome::Ready {
                relation: RemoteComparisonRelation::Ahead,
                total,
                ..
            } if total > 0 => {}
            RemoteComparisonOutcome::Ready {
                relation: RemoteComparisonRelation::Behind,
                ..
            } => {
                return Err(CommandError::operation(
                    "UPDATE_CONFIRMATION_REQUIRED",
                    format!(
                        "Skill {} needs interactive confirmation because its source moved behind",
                        pending.name
                    ),
                ));
            }
            RemoteComparisonOutcome::Ready {
                relation: RemoteComparisonRelation::Diverged,
                ..
            } => {
                return Err(CommandError::operation(
                    "UPDATE_CONFIRMATION_REQUIRED",
                    format!(
                        "Skill {} needs interactive confirmation because its source diverged",
                        pending.name
                    ),
                ));
            }
            RemoteComparisonOutcome::Ready { .. } => {
                return Err(CommandError::operation(
                    "INVALID_RESPONSE",
                    "GitHub returned an impossible update relation",
                ));
            }
            outcome => return Err(update_apply_failure(&pending.name, outcome)),
        }
        let prepared = provider
            .prepare_exact(&pending.selector, &pending.expected_commit, false)
            .map_err(CommandError::remote)?;
        let staged = materialize_remote(&prepared.files)?;
        let staged_name =
            skilld_core::SkillName::from_source(staged.path()).map_err(CommandError::domain)?;
        if staged_name != pending.skill_name {
            return Err(CommandError::operation(
                "SOURCE_MISMATCH",
                format!("the updated Skill name changed from {}", pending.name),
            ));
        }
        let targets = pending
            .view
            .skill
            .targets
            .iter()
            .map(|locked| {
                known
                    .iter()
                    .find(|target| target.agent == locked.agent)
                    .cloned()
                    .map(|target| TargetInstall {
                        target,
                        mode: locked.mode,
                    })
                    .ok_or_else(|| {
                        CommandError::domain(DomainError::InvalidTarget(locked.agent.to_string()))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        selected.push(PreparedUpdateSelection {
            name: pending.name,
            staged,
            prepared,
            targets,
            expected_transaction_id: pending.view.transaction_id,
            expected_skill: pending.view.skill,
        });
    }
    let updates = selected
        .iter()
        .map(|selection| PreparedStoreUpdate {
            source: selection.staged.path().to_owned(),
            locked_source: selection.prepared.locked_source.clone(),
            source_status: Some(selection.prepared.source_status.clone()),
            targets: selection.targets.clone(),
            expected_transaction_id: selection.expected_transaction_id.clone(),
            expected_skill: selection.expected_skill.clone(),
        })
        .collect();
    store
        .apply_update_batch(updates, &known)
        .map_err(CommandError::store)?;
    Ok(selected
        .into_iter()
        .map(|selection| Line::success(format!("Updated Skill {}.", selection.name)))
        .collect())
}

fn update_plan_item(
    pending: PendingUpdateComparison,
    outcome: RemoteComparisonOutcome,
) -> UpdatePlanItem {
    let unavailable = |code: &'static str, message: String| {
        UpdatePlanItem::new(
            pending.name.clone(),
            unavailable_update(
                pending.locked_commit_sha.clone(),
                UpdateLatestCommit::Known {
                    commit_sha: pending.latest_commit_sha.clone(),
                },
                code,
                message,
            ),
        )
    };
    match outcome {
        RemoteComparisonOutcome::Ready {
            relation,
            ahead_by,
            behind_by,
            commits,
            total,
            truncated,
            compare_url,
        } => {
            let Ok(classified) = classify_update_comparison(
                pending.locked_commit_sha.clone(),
                pending.latest_commit_sha.clone(),
                ahead_by,
                behind_by,
            ) else {
                return unavailable(
                    "INVALID_RESPONSE",
                    "GitHub returned impossible update counts".to_owned(),
                );
            };
            let relation_matches = matches!(
                (&relation, &classified),
                (
                    RemoteComparisonRelation::Identical,
                    UpdateRelation::Current { .. }
                ) | (
                    RemoteComparisonRelation::Ahead,
                    UpdateRelation::Available { .. }
                ) | (
                    RemoteComparisonRelation::Behind,
                    UpdateRelation::Behind { .. }
                ) | (
                    RemoteComparisonRelation::Diverged,
                    UpdateRelation::Diverged { .. }
                )
            );
            if !relation_matches {
                return unavailable(
                    "INVALID_RESPONSE",
                    "GitHub returned an impossible update relation".to_owned(),
                );
            }
            match CommitHistory::compared(commits, total, truncated, compare_url) {
                Ok(history) => UpdatePlanItem::with_history(pending.name, classified, history),
                Err(error) => unavailable("INVALID_RESPONSE", error.to_string()),
            }
        }
        RemoteComparisonOutcome::NotFound => unavailable(
            "SOURCE_NOT_FOUND",
            "The Repository or commit is unavailable".to_owned(),
        ),
        RemoteComparisonOutcome::InvalidComparison => unavailable(
            "COMMIT_NOT_FOUND",
            "GitHub could not compare the installed commit".to_owned(),
        ),
        RemoteComparisonOutcome::RateLimited {
            retry_after_seconds,
            reset_at,
        } => {
            let retry_after = match (retry_after_seconds, reset_at) {
                (Some(seconds), Some(reset_at)) => {
                    UpdateRetryAfter::SecondsAndReset { seconds, reset_at }
                }
                (Some(seconds), None) => UpdateRetryAfter::Seconds { seconds },
                (None, Some(reset_at)) => UpdateRetryAfter::Reset { reset_at },
                (None, None) => UpdateRetryAfter::Unknown,
            };
            let failure =
                UpdateFailure::rate_limited("GitHub rate limited the update check.", retry_after);
            UpdatePlanItem::new(
                pending.name,
                UpdateRelation::Unavailable {
                    locked_commit_sha: pending.locked_commit_sha,
                    latest_commit: UpdateLatestCommit::Known {
                        commit_sha: pending.latest_commit_sha,
                    },
                    failure,
                },
            )
        }
        RemoteComparisonOutcome::ProviderFailure { status } => unavailable(
            "SERVICE_UNAVAILABLE",
            status.map_or_else(
                || "GitHub comparison failed".to_owned(),
                |status| format!("GitHub comparison returned HTTP {status}"),
            ),
        ),
        RemoteComparisonOutcome::RequestFailure { code, message } => unavailable(code, message),
    }
}

fn update_apply_failure(name: &str, outcome: RemoteComparisonOutcome) -> CommandError {
    match outcome {
        RemoteComparisonOutcome::NotFound => CommandError::operation(
            "SOURCE_NOT_FOUND",
            format!("The Repository or commit for Skill {name} is unavailable"),
        ),
        RemoteComparisonOutcome::InvalidComparison => CommandError::operation(
            "COMMIT_NOT_FOUND",
            format!("GitHub could not compare the installed commit for Skill {name}"),
        ),
        RemoteComparisonOutcome::RateLimited {
            retry_after_seconds,
            ..
        } => CommandError::operation(
            "RATE_LIMITED",
            retry_after_seconds.map_or_else(
                || "GitHub rate limited the Skill update".to_owned(),
                |seconds| {
                    format!("GitHub rate limited the Skill update. Retry after {seconds} seconds")
                },
            ),
        ),
        RemoteComparisonOutcome::ProviderFailure { status } => CommandError::operation(
            "SERVICE_UNAVAILABLE",
            status.map_or_else(
                || "GitHub comparison failed".to_owned(),
                |status| format!("GitHub comparison returned HTTP {status}"),
            ),
        ),
        RemoteComparisonOutcome::RequestFailure { code, message } => {
            CommandError::operation(code, message)
        }
        RemoteComparisonOutcome::Ready { .. } => CommandError::operation(
            "INVALID_RESPONSE",
            "GitHub returned an impossible update relation",
        ),
    }
}

impl LocalHost {
    fn report_outdated_view(&self, view: &SkillView, scope: InstallScope) -> Vec<Line> {
        let name = &view.name;
        let global = if scope == InstallScope::Global {
            " --global"
        } else {
            ""
        };
        match (&view.skill.source, &view.skill.source_status) {
            (
                LockedSource::Remote {
                    source, commit_sha, ..
                },
                skilld_core::SourceStatus::Verified { artifact_id, .. },
            ) => {
                let state = skilld_core::RemoteSelector::parse(source)
                    .map_err(CommandError::remote)
                    .and_then(|selector| {
                        self.remote_provider()?
                            .source_state(&selector, artifact_id, commit_sha)
                            .map_err(CommandError::remote)
                    });
                match state {
                    Ok(RemoteSourceState::Current) => {
                        vec![Line::record(
                            Marker::Success,
                            format!("Current Skill {name}."),
                            name,
                            Some("current".to_owned()),
                            Vec::new(),
                        )]
                    }
                    Ok(RemoteSourceState::Stale { .. }) => {
                        let update = format!("skilld update {name}{global}");
                        vec![Line::record(
                            Marker::Warn,
                            format!("Outdated Skill {name}. Run {update}."),
                            name,
                            Some("outdated".to_owned()),
                            vec![("update", update)],
                        )]
                    }
                    Err(error) => {
                        vec![Line::record(
                            Marker::Error,
                            format!(
                                "Source state unavailable for Skill {name}: {}.",
                                error.message
                            ),
                            name,
                            Some("source unavailable".to_owned()),
                            vec![("error", error.message.clone())],
                        )]
                    }
                }
            }
            (LockedSource::Remote { source, .. }, skilld_core::SourceStatus::Unverified { .. }) => {
                let agents = view
                    .skill
                    .targets
                    .iter()
                    .map(|locked| locked.agent)
                    .collect::<Vec<_>>();
                let agent_flags = outdated::agent_flags(&agents);
                let install = format!("skilld install {source} --direct{global}{agent_flags}");
                vec![Line::record(
                    Marker::Warn,
                    format!("Unverified Skill {name}. Run {install} to update it."),
                    name,
                    Some("unverified".to_owned()),
                    vec![("install", install)],
                )]
            }
            (LockedSource::BundledSkilld, _) => vec![Line::record(
                Marker::Note,
                format!("skilld-maintained Skill {name}."),
                name,
                Some("skilld-maintained".to_owned()),
                Vec::new(),
            )],
            _ => vec![Line::record(
                Marker::Note,
                format!("Local Skill {name}."),
                name,
                Some("local".to_owned()),
                Vec::new(),
            )],
        }
    }

    fn search_candidate(
        &self,
        name: &str,
    ) -> Result<Option<outdated::SkillCandidate>, CommandError> {
        let results = self
            .remote_provider()?
            .search(name, 5)
            .map_err(CommandError::remote)?;
        let Some(result) = results.items.into_iter().find(|result| result.name == name) else {
            return Ok(None);
        };
        let selector = result.selector().map_err(CommandError::remote)?;
        Ok(Some(outdated::SkillCandidate {
            selector: selector.canonical(),
            stargazer_count: result.stargazer_count,
        }))
    }
}
struct StagedRemote {
    _directory: tempfile::TempDir,
    skill: PathBuf,
}

impl StagedRemote {
    fn path(&self) -> &Path {
        &self.skill
    }
}

fn materialize_remote(files: &[skilld_core::PreparedFile]) -> Result<StagedRemote, CommandError> {
    let (name, _, files) =
        skilld_core::prepare_unverified_files(files.to_vec()).map_err(CommandError::remote)?;
    let directory = tempfile::Builder::new()
        .prefix("skilld-remote-")
        .tempdir()
        .map_err(|error| CommandError::filesystem(format!("cannot stage the Skill: {error}")))?;
    let skill = directory.path().join(name.as_str());
    fs::create_dir(&skill)
        .map_err(|error| CommandError::filesystem(format!("cannot stage the Skill: {error}")))?;
    for file in files {
        let path = skill.join(&file.path);
        let parent = path
            .parent()
            .ok_or_else(|| CommandError::filesystem("cannot resolve a staged Skill file parent"))?;
        fs::create_dir_all(parent).map_err(|error| {
            CommandError::filesystem(format!("cannot stage a Skill directory: {error}"))
        })?;
        let mut destination = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                CommandError::filesystem(format!("cannot stage a Skill file: {error}"))
            })?;
        destination.write_all(&file.bytes).map_err(|error| {
            CommandError::filesystem(format!("cannot write a staged Skill file: {error}"))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(file.mode)).map_err(|error| {
                CommandError::filesystem(format!("cannot set a staged Skill mode: {error}"))
            })?;
        }
    }
    Ok(StagedRemote {
        _directory: directory,
        skill,
    })
}

fn selected_names(
    store: &LocalStore,
    known: &[ResolvedTarget],
    requested: Option<&str>,
) -> Result<Vec<String>, CommandError> {
    if let Some(name) = requested {
        skilld_core::SkillName::parse(name.to_owned()).map_err(CommandError::domain)?;
        Ok(vec![name.to_owned()])
    } else {
        store.list(known).map_err(CommandError::store)
    }
}

fn validate_update_selection(items: &[UpdatePlanItem]) -> Result<(), CommandError> {
    if items.is_empty() {
        return Err(CommandError::usage(
            "INVALID_SELECTION",
            "Select at least one Skill",
        ));
    }
    let mut unique = BTreeSet::new();
    for item in items {
        if !unique.insert(item.name()) {
            return Err(CommandError::usage(
                "INVALID_SELECTION",
                "Select each Skill once",
            ));
        }
    }
    for item in items {
        if !matches!(item.relation(), UpdateRelation::Available { .. }) {
            return Err(CommandError::usage(
                "INVALID_SELECTION",
                "Select only Skills with available updates",
            ));
        }
    }
    Ok(())
}

fn stale_update_plan(name: &str) -> CommandError {
    CommandError::operation(
        "STALE_UPDATE_PLAN",
        format!("Skill {name} changed after review. Review its commits again"),
    )
}

fn unavailable_update(
    locked_commit_sha: CommitSha,
    latest_commit: UpdateLatestCommit,
    code: impl Into<String>,
    message: impl Into<String>,
) -> UpdateRelation {
    UpdateRelation::Unavailable {
        locked_commit_sha,
        latest_commit,
        failure: UpdateFailure::new(code, message),
    }
}

fn update_model_error(error: UpdateModelError) -> CommandError {
    CommandError::operation("INVALID_LOCKFILE", error.to_string())
}

fn detects_environment(agent: AgentTargetId, environment: &DetectionEnvironment) -> bool {
    match agent {
        AgentTargetId::ClaudeCode => [
            "CLAUDE_CODE",
            "CLAUDECODE",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CONFIG_DIR",
        ]
        .iter()
        .any(|name| environment.has(name)),
        AgentTargetId::Cursor => ["CURSOR_SESSION", "CURSOR_TRACE_ID"]
            .iter()
            .any(|name| environment.has(name)),
        AgentTargetId::Windsurf => environment.has("WINDSURF_SESSION"),
        AgentTargetId::Cline => ["CLINE_TASK_ID", "CLINE_ACTIVE"]
            .iter()
            .any(|name| environment.has(name)),
        AgentTargetId::Codex => false,
        AgentTargetId::GithubCopilot => environment.has("COPILOT_RUN_APP"),
        AgentTargetId::GeminiCli => environment.has("GEMINI_CLI"),
        AgentTargetId::Goose => ["GOOSE_SESSION", "AGENT_SESSION_ID"]
            .iter()
            .any(|name| environment.has(name)),
        AgentTargetId::Amp => environment.has("AMP_SESSION"),
        AgentTargetId::Opencode => ["OPENCODE_SESSION", "OPENCODE_SESSION_ID"]
            .iter()
            .any(|name| environment.has(name)),
        AgentTargetId::Roo => environment.has("ROO_SESSION"),
        AgentTargetId::Antigravity => environment.has("ANTIGRAVITY_CLI_ALIAS"),
    }
}

fn detects_project(agent: AgentTargetId, root: &Path) -> bool {
    let exists = |path: &str| root.join(path).exists();
    match agent {
        AgentTargetId::ClaudeCode => exists(".claude"),
        AgentTargetId::Cursor => exists(".cursor") || exists(".cursorrules"),
        AgentTargetId::Windsurf => exists(".windsurf") || exists(".windsurfrules"),
        AgentTargetId::Cline => exists(".cline"),
        AgentTargetId::Codex => {
            exists(".codex")
                || exists("AGENTS.md")
                || exists("AGENTS.override.md")
                || exists(".agents/skills")
        }
        AgentTargetId::GithubCopilot => {
            exists(".github/copilot-instructions.md")
                || exists(".github/skills")
                || exists("AGENTS.md")
                || exists(".github/instructions")
        }
        AgentTargetId::GeminiCli => exists(".gemini"),
        AgentTargetId::Goose => exists(".goose"),
        AgentTargetId::Amp => exists(".agents/AGENTS.md"),
        AgentTargetId::Opencode => exists(".opencode"),
        AgentTargetId::Roo => exists(".roo"),
        AgentTargetId::Antigravity => exists(".agent"),
    }
}

fn detects_installed(agent: AgentTargetId, roots: &TargetRoots) -> bool {
    match agent {
        AgentTargetId::ClaudeCode => roots.claude_home.exists(),
        AgentTargetId::Cursor => roots.home.join(".cursor").exists(),
        AgentTargetId::Windsurf => roots.home.join(".codeium/windsurf").exists(),
        AgentTargetId::Cline => roots.home.join(".cline").exists(),
        AgentTargetId::Codex => roots.home.join(".codex").exists(),
        AgentTargetId::GithubCopilot => {
            roots.home.join(".copilot").exists() || has_copilot_extension(&roots.home)
        }
        AgentTargetId::GeminiCli => roots.home.join(".gemini").exists(),
        AgentTargetId::Goose => roots.config_home.join("goose").exists(),
        AgentTargetId::Amp => roots.config_home.join("amp").exists(),
        AgentTargetId::Opencode => roots.config_home.join("opencode").exists(),
        AgentTargetId::Roo => roots.home.join(".roo").exists(),
        AgentTargetId::Antigravity => roots.home.join(".gemini/antigravity").exists(),
    }
}

fn global_target_root(path: GlobalTargetPath, roots: &TargetRoots) -> PathBuf {
    match path {
        GlobalTargetPath::Home(path) => roots.home.join(path),
        GlobalTargetPath::ConfigHome(path) => roots.config_home.join(path),
        GlobalTargetPath::ClaudeHome(path) => roots.claude_home.join(path),
    }
}

fn has_copilot_extension(home: &Path) -> bool {
    [
        home.join(".vscode/extensions"),
        home.join(".vscode-server/extensions"),
        home.join(".cursor/extensions"),
    ]
    .iter()
    .any(|directory| {
        fs::read_dir(directory).is_ok_and(|entries| {
            // One unreadable extension entry cannot prove Copilot is absent or present.
            entries.filter_map(Result::ok).any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("github.copilot-"))
            })
        })
    })
}

fn absolute(path: &Path) -> Result<PathBuf, CommandError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| CommandError::filesystem(error.to_string()))
}

pub fn command_names() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .map(|command| command.get_name().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingHost;

    impl Host for RecordingHost {
        fn list(&self, _scope: InstallScope) -> Result<Vec<String>, CommandError> {
            Ok(vec![])
        }

        fn install(
            &self,
            source: InstallSource,
            scope: InstallScope,
        ) -> Result<String, CommandError> {
            assert_eq!(source, InstallSource::BundledSkilld);
            assert_eq!(scope, InstallScope::Global);
            Ok("skilld".to_owned())
        }

        fn install_request(&self, request: InstallRequest) -> Result<Vec<String>, CommandError> {
            assert_eq!(
                request.operation,
                InstallOperation::Install(InstallSource::BundledSkilld)
            );
            assert_eq!(request.scope, InstallScope::Global);
            assert_eq!(request.targets, [AgentTargetId::Codex]);
            Ok(vec!["skilld".to_owned()])
        }
    }

    #[test]
    fn public_command_vocabulary_matches_v3() {
        assert_eq!(
            command_names(),
            [
                "search", "install", "list", "view", "remove", "update", "verify", "outdated",
                "auth", "config"
            ]
        );
    }

    #[test]
    fn upgrade_is_not_a_command_alias() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = run(
            ["skilld", "upgrade"],
            &RecordingHost,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(result.exit_code, 2);
        assert!(stdout.is_empty());
        assert!(
            String::from_utf8(stderr)
                .unwrap()
                .contains("unrecognized subcommand 'upgrade'")
        );
    }

    #[test]
    fn global_skilld_install_uses_the_target_contract() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = run(
            [
                "skilld", "install", "skilld", "--global", "--agent", "codex",
            ],
            &RecordingHost,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(result.exit_code, 0);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "Installed Skill skilld.\n"
        );
        assert!(stderr.is_empty());
    }
}
