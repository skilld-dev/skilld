mod config;
mod local_store;

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
use skilld_core::{
    AGENT_TARGETS, AgentTargetId, DomainError, GlobalTargetPath, InstallMode, InstallRequest,
    InstallScope, InstallSource, LockedSource, VERSION, select_target_ids,
};

#[derive(Debug, Parser)]
#[command(
    name = "skilld",
    version = VERSION,
    about = "Search, install, and keep Skills current",
    disable_help_subcommand = true
)]
pub struct Cli {
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
    /// Upgrade installed Skills.
    Upgrade { skill: Option<String> },
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
        let source = request
            .source
            .ok_or_else(|| CommandError::unsupported_host("lockfile restore is unavailable"))?;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn unsupported_host(message: impl Into<String>) -> Self {
        Self {
            code: "UNSUPPORTED_HOST",
            message: message.into(),
        }
    }

    pub fn service(message: impl Into<String>) -> Self {
        Self {
            code: "SERVICE_UNAVAILABLE",
            message: message.into(),
        }
    }

    pub fn not_implemented(message: impl Into<String>) -> Self {
        Self {
            code: "NOT_IMPLEMENTED",
            message: message.into(),
        }
    }

    pub fn input(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_SOURCE",
            message: message.into(),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_CONFIG",
            message: message.into(),
        }
    }

    pub fn filesystem(message: impl Into<String>) -> Self {
        Self {
            code: "SERVICE_UNAVAILABLE",
            message: message.into(),
        }
    }

    pub fn domain(error: DomainError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }

    pub fn store(error: StoreError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
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

pub fn run<I, T, H, O, E>(args: I, host: &H, stdout: &mut O, stderr: &mut E) -> CommandResult
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    H: Host,
    O: Write,
    E: Write,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let display = matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            let target: &mut dyn Write = if display { stdout } else { stderr };
            if write!(target, "{error}").is_err() {
                return CommandResult { exit_code: 2 };
            }
            return CommandResult {
                exit_code: if display { 0 } else { 2 },
            };
        }
    };

    match dispatch(cli.command, host) {
        Ok(lines) => {
            for line in lines {
                if writeln!(stdout, "{line}").is_err() {
                    return CommandResult { exit_code: 2 };
                }
            }
            CommandResult { exit_code: 0 }
        }
        Err(error) => {
            if writeln!(stderr, "{error}").is_err() {
                return CommandResult { exit_code: 2 };
            }
            CommandResult { exit_code: 2 }
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

fn dispatch<H: Host>(command: Command, host: &H) -> Result<Vec<String>, CommandError> {
    match command {
        Command::Install {
            source,
            global,
            agents,
            mode,
        } => {
            let scope = scope(global);
            let source = source.map(|source| InstallSource::parse(&source));
            if source == Some(InstallSource::BundledSkilld) && scope != InstallScope::Global {
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
                source,
                scope,
                targets,
                mode,
            })?;
            Ok(names
                .into_iter()
                .map(|name| format!("Installed Skill {name}."))
                .collect())
        }
        Command::List { global } => host.list(scope(global)),
        Command::View { skill, global } => render_view(host.view(&skill, scope(global))?),
        Command::Remove { skill, global } => {
            host.remove(&skill, scope(global))?;
            Ok(vec![format!("Removed Skill {skill}.")])
        }
        Command::Auth {
            command: AuthCommand::Status,
        } => Ok(vec![if host.auth_status()? {
            "Authenticated.".to_owned()
        } else {
            "Not authenticated.".to_owned()
        }]),
        Command::Auth {
            command: AuthCommand::Login,
        } => {
            host.auth_login()?;
            Ok(vec!["Authentication started.".to_owned()])
        }
        Command::Auth {
            command: AuthCommand::Logout,
        } => {
            host.auth_logout()?;
            Ok(vec!["Logged out.".to_owned()])
        }
        Command::Config {
            command: ConfigCommand::Get { key },
        } => Ok(vec![host.config_get(&key)?]),
        Command::Config {
            command: ConfigCommand::Set { key, value },
        } => {
            host.config_set(&key, &value)?;
            Ok(vec![format!("Set {key}.")])
        }
        Command::Config {
            command: ConfigCommand::List,
        } => host.config_list(),
        Command::Search { query } => unavailable(format!(
            "Skill search is not implemented yet: {}",
            query.join(" ")
        )),
        Command::Upgrade { skill } => unavailable(format!(
            "Skill upgrade is not implemented yet: {}",
            skill.unwrap_or_else(|| "all installed Skills".to_owned())
        )),
        Command::Verify { skill } => unavailable(format!(
            "source verification is not implemented yet: {}",
            skill.unwrap_or_else(|| "all installed Skills".to_owned())
        )),
    }
}

fn render_view(view: SkillView) -> Result<Vec<String>, CommandError> {
    let source = match view.skill.source {
        LockedSource::Local { path } => format!("local {path}"),
        LockedSource::BundledSkilld => "skilld-maintained Skill".to_owned(),
        LockedSource::Remote { source } => source,
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

fn unavailable<T>(message: String) -> Result<T, CommandError> {
    Err(CommandError::not_implemented(message))
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
        }
    }

    fn restore(&self, request: &InstallRequest) -> Result<Vec<String>, CommandError> {
        let (targets, known) = self.select_installs(request)?;
        let store = self.store(request.scope);
        let names = store.list(&known).map_err(CommandError::store)?;
        if names.is_empty() {
            return Err(CommandError {
                code: "LOCKFILE_NOT_FOUND",
                message: format!(
                    "no installed Skills exist in {} scope",
                    request.scope.as_str()
                ),
            });
        }
        let mut restored = Vec::new();
        for name in names {
            let skill_name =
                skilld_core::SkillName::parse(name.clone()).map_err(CommandError::domain)?;
            let view = store
                .view(&skill_name, &known)
                .map_err(CommandError::store)?;
            let restored_targets = targets
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
                .collect::<Vec<_>>();
            let source = match view.skill.source {
                LockedSource::Local { path } => InstallSource::Local(PathBuf::from(path)),
                LockedSource::BundledSkilld => InstallSource::BundledSkilld,
                LockedSource::Remote { source } => InstallSource::Remote(source),
            };
            let (source, locked_source) = self.resolve_source(source)?;
            store
                .install_from(&source, locked_source, &restored_targets, &known)
                .map_err(CommandError::store)?;
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
            source: Some(source),
            scope,
            targets: vec![],
            mode: None,
        })?
        .into_iter()
        .next()
        .ok_or_else(|| CommandError::service("Skill install returned no result"))
    }

    fn install_request(&self, request: InstallRequest) -> Result<Vec<String>, CommandError> {
        let Some(source) = request.source.clone() else {
            return self.restore(&request);
        };
        let (targets, known) = self.select_installs(&request)?;
        let (source, locked_source) = self.resolve_source(source)?;
        let name = self
            .store(request.scope)
            .install_from(&source, locked_source, &targets, &known)
            .map_err(CommandError::store)?;
        Ok(vec![name.to_string()])
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
            assert_eq!(request.source, Some(InstallSource::BundledSkilld));
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
                "search", "install", "list", "view", "remove", "upgrade", "verify", "auth",
                "config"
            ]
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
