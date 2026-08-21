mod config;
mod local_store;
mod output;
mod remote;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};
pub use config::{ConfigStore, LocalConfig};
pub use local_store::{
    AllowTransaction, LocalStore, ResolvedTarget, SkillView, StoreError, TargetInstall,
    TransactionGate,
};
pub use output::OutputContext;
pub use remote::{
    Cancellation, HeaderValue, HttpAdapter, HttpHeader, HttpMethod, HttpRequest, HttpResponse,
    NativeRemoteConfig, NeverCancelled, NoTokenProvider, PreparedRemoteSkill, RemoteProvider,
    RemoteSourceState, SecretValue, SkilldRemote, Sleeper, ThreadSleeper, TokenProvider,
};
use skilld_core::{
    AGENT_TARGETS, AgentTargetId, CommitSha, DomainError, GlobalTargetPath, InstallMode,
    InstallOperation, InstallRequest, InstallScope, InstallSource, LockedSource, NotTrackedReason,
    SourceRef, UpdateCheckV1, UpdateFailure, UpdateLatestCommit, UpdateModelError, UpdatePlan,
    UpdatePlanItem, UpdateRelation, VERSION, select_target_ids,
};

use output::{
    OutputMode, SearchItem, SearchOutcome, render_error, render_search, render_update_check,
    resolve_mode,
};

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
    /// Install a Skill or restore lockfile state.
    Install {
        source: Option<String>,
        #[arg(long)]
        global: bool,
        #[arg(long = "agent")]
        agents: Vec<String>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
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
        #[arg(long, requires = "json")]
        check: bool,
    },
    /// Verify a Skill source.
    Verify { skill: Option<String> },
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

    fn verify(&self, _name: Option<&str>) -> Result<Vec<String>, CommandError> {
        Err(CommandError::unsupported_host(
            "source verification is unavailable on this host",
        ))
    }

    fn update(&self, _name: Option<&str>) -> Result<Vec<String>, CommandError> {
        Err(CommandError::unsupported_host(
            "Skill update is unavailable on this host",
        ))
    }

    fn update_check(&self, _name: Option<&str>) -> Result<UpdateCheckV1, CommandError> {
        Err(CommandError::unsupported_host(
            "Skill update checks are unavailable on this host",
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

    fn config_list(&self) -> Result<Vec<String>, CommandError> {
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

enum CommandOutput {
    Lines(Vec<String>),
    Search(SearchOutcome),
    UpdateCheck(UpdateCheckV1),
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
        Ok(CommandOutput::Lines(lines)) => {
            let mut bytes = Vec::new();
            for line in lines {
                bytes.extend_from_slice(line.as_bytes());
                bytes.push(b'\n');
            }
            write_success(&bytes, mode, stdout, stderr)
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
    match stdout.write_all(bytes) {
        Ok(()) => CommandResult { exit_code: 0 },
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
                    (true, _) => {
                        return Err(CommandError::input(
                            "--direct needs an explicit public GitHub Repository selector",
                        ));
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
                .map(|name| format!("Installed Skill {name}."))
                .collect::<Vec<_>>();
            if direct {
                lines.push("Review the unverified Skill before use.".to_owned());
            }
            Ok(CommandOutput::Lines(lines))
        }
        Command::List { global } => host.list(scope(global)).map(CommandOutput::Lines),
        Command::View { skill, global } => {
            render_view(host.view(&skill, scope(global))?).map(CommandOutput::Lines)
        }
        Command::Remove { skill, global } => {
            host.remove(&skill, scope(global))?;
            Ok(CommandOutput::Lines(vec![format!(
                "Removed Skill {skill}."
            )]))
        }
        Command::Auth {
            command: AuthCommand::Status,
        } => Ok(CommandOutput::Lines(vec![if host.auth_status()? {
            "Authenticated.".to_owned()
        } else {
            "Not authenticated.".to_owned()
        }])),
        Command::Auth {
            command: AuthCommand::Login,
        } => {
            host.auth_login()?;
            Ok(CommandOutput::Lines(vec![
                "Authentication started.".to_owned(),
            ]))
        }
        Command::Auth {
            command: AuthCommand::Logout,
        } => {
            host.auth_logout()?;
            Ok(CommandOutput::Lines(vec!["Logged out.".to_owned()]))
        }
        Command::Config {
            command: ConfigCommand::Get { key },
        } => Ok(CommandOutput::Lines(vec![host.config_get(&key)?])),
        Command::Config {
            command: ConfigCommand::Set { key, value },
        } => {
            host.config_set(&key, &value)?;
            Ok(CommandOutput::Lines(vec![format!("Set {key}.")]))
        }
        Command::Config {
            command: ConfigCommand::List,
        } => host.config_list().map(CommandOutput::Lines),
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
        Command::Update { skill, check } => {
            if check {
                host.update_check(skill.as_deref())
                    .map(CommandOutput::UpdateCheck)
            } else {
                host.update(skill.as_deref()).map(CommandOutput::Lines)
            }
        }
        Command::Verify { skill } => host.verify(skill.as_deref()).map(CommandOutput::Lines),
    }
}

fn render_view(view: SkillView) -> Result<Vec<String>, CommandError> {
    let source = match view.skill.source {
        LockedSource::Local { path } => format!("local {path}"),
        LockedSource::BundledSkilld => "skilld-maintained Skill".to_owned(),
        LockedSource::Remote { source, .. } => source,
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
        format!("Name: {}", view.name),
        format!("Path: {}", view.canonical_path.display()),
        format!("Source: {source}"),
        format!("Source status: {}", view.skill.source_status.as_str()),
        format!("Agent targets: {targets}"),
    ])
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

    fn update_relation(
        &self,
        skill: &skilld_core::LockedSkill,
    ) -> Result<UpdateRelation, CommandError> {
        let (source, locked_commit_sha) = match &skill.source {
            LockedSource::Local { .. } => {
                return Ok(UpdateRelation::NotTracked {
                    reason: NotTrackedReason::Local,
                });
            }
            LockedSource::BundledSkilld => {
                return Ok(UpdateRelation::NotTracked {
                    reason: NotTrackedReason::Bundled,
                });
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
                return Ok(unavailable_update(
                    locked_commit_sha,
                    UpdateLatestCommit::Unknown,
                    error.code,
                    error.message,
                ));
            }
        };
        if let Some(SourceRef::Commit { value }) = &selector.source().r#ref {
            let pinned_commit_sha = match CommitSha::parse(value.clone()) {
                Ok(commit_sha) => commit_sha,
                Err(error) => {
                    return Ok(unavailable_update(
                        locked_commit_sha,
                        UpdateLatestCommit::Unknown,
                        "INVALID_SOURCE",
                        error.to_string(),
                    ));
                }
            };
            if pinned_commit_sha != locked_commit_sha {
                return Ok(unavailable_update(
                    locked_commit_sha,
                    UpdateLatestCommit::Known {
                        commit_sha: pinned_commit_sha,
                    },
                    "INVALID_LOCKFILE",
                    "the locked commit differs from its source selector",
                ));
            }
            return Ok(UpdateRelation::Pinned {
                commit_sha: locked_commit_sha,
            });
        }

        let artifact_id = match &skill.source_status {
            skilld_core::SourceStatus::Verified { artifact_id, .. } => artifact_id,
            skilld_core::SourceStatus::Unverified { .. } => {
                return Ok(unavailable_update(
                    locked_commit_sha,
                    UpdateLatestCommit::Unknown,
                    "UNVERIFIED_SOURCE",
                    "run an explicit --direct install to update this Skill",
                ));
            }
            skilld_core::SourceStatus::Local { .. } => {
                return Ok(unavailable_update(
                    locked_commit_sha,
                    UpdateLatestCommit::Unknown,
                    "INVALID_LOCKFILE",
                    "the remote Skill has a local source status",
                ));
            }
        };
        let provider = match self.remote_provider() {
            Ok(provider) => provider,
            Err(error) => {
                return Ok(unavailable_update(
                    locked_commit_sha,
                    UpdateLatestCommit::Unknown,
                    error.code,
                    error.message,
                ));
            }
        };
        let state = match provider.source_state(&selector, artifact_id, locked_commit_sha.as_str())
        {
            Ok(state) => state,
            Err(error) => {
                return Ok(unavailable_update(
                    locked_commit_sha,
                    UpdateLatestCommit::Unknown,
                    error.code,
                    error.message,
                ));
            }
        };
        match state {
            RemoteSourceState::Current => Ok(UpdateRelation::Current {
                commit_sha: locked_commit_sha,
            }),
            RemoteSourceState::Stale {
                current_commit_sha, ..
            } => {
                let latest_commit_sha = match CommitSha::parse(current_commit_sha) {
                    Ok(commit_sha) => commit_sha,
                    Err(error) => {
                        return Ok(unavailable_update(
                            locked_commit_sha,
                            UpdateLatestCommit::Unknown,
                            "INVALID_RESPONSE",
                            error.to_string(),
                        ));
                    }
                };
                if latest_commit_sha == locked_commit_sha {
                    return Ok(UpdateRelation::Current {
                        commit_sha: locked_commit_sha,
                    });
                }
                Ok(unavailable_update(
                    locked_commit_sha,
                    UpdateLatestCommit::Known {
                        commit_sha: latest_commit_sha,
                    },
                    "COMPARISON_UNAVAILABLE",
                    "skilld.dev does not provide Git comparison data",
                ))
            }
        }
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

    fn config_list(&self) -> Result<Vec<String>, CommandError> {
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

    fn verify(&self, requested: Option<&str>) -> Result<Vec<String>, CommandError> {
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
                            lines.push(format!("Verified Skill {}.", name.as_str()));
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
                _ => lines.push(format!("Checked local Skill {}.", name.as_str())),
            }
        }
        Ok(lines)
    }

    fn update(&self, requested: Option<&str>) -> Result<Vec<String>, CommandError> {
        let scope = InstallScope::Project;
        let known = self.known_targets(scope)?;
        let store = self.store(scope);
        let names = selected_names(&store, &known, requested)?;
        let mut updated = Vec::new();
        for name in names {
            let skill_name =
                skilld_core::SkillName::parse(name.clone()).map_err(CommandError::domain)?;
            let view = store
                .view(&skill_name, &known)
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
            let prepared = self
                .remote_provider()?
                .prepare(&selector, false)
                .map_err(CommandError::remote)?;
            let staged = materialize_remote(&prepared.files)?;
            let staged_name =
                skilld_core::SkillName::from_source(staged.path()).map_err(CommandError::domain)?;
            if staged_name != skill_name {
                return Err(CommandError::operation(
                    "SOURCE_MISMATCH",
                    format!("the updated Skill name changed from {name}"),
                ));
            }
            let targets = view
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
            store
                .install_from_with_status(
                    staged.path(),
                    prepared.locked_source,
                    prepared.source_status,
                    &targets,
                    &known,
                )
                .map_err(CommandError::store)?;
            updated.push(format!("Updated Skill {name}."));
        }
        Ok(updated)
    }

    fn update_check(&self, requested: Option<&str>) -> Result<UpdateCheckV1, CommandError> {
        let scope = InstallScope::Project;
        let known = self.known_targets(scope)?;
        let store = self.store(scope);
        let names = selected_names(&store, &known, requested)?;
        let mut items = Vec::with_capacity(names.len());
        for name in names {
            let skill_name = skilld_core::SkillName::parse(name).map_err(CommandError::domain)?;
            let view = store
                .view(&skill_name, &known)
                .map_err(CommandError::store)?;
            let relation = self.update_relation(&view.skill)?;
            items.push(UpdatePlanItem::new(skill_name, relation));
        }
        let plan = UpdatePlan::new(items).map_err(update_model_error)?;
        Ok(UpdateCheckV1::new(plan))
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
    CommandError {
        code: "INVALID_LOCKFILE",
        message: error.to_string(),
    }
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
                "search", "install", "list", "view", "remove", "update", "verify", "auth", "config"
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
