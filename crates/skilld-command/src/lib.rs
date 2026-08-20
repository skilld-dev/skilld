mod local_store;

use std::ffi::OsString;
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};
pub use local_store::{AllowPromotion, LocalStore, PromotionGate};
use skilld_core::{InstallScope, InstallSource, VERSION};

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
    /// Install a Skill.
    Install {
        source: String,
        #[arg(long)]
        global: bool,
    },
    /// List installed Skills.
    List {
        #[arg(long)]
        global: bool,
    },
    /// View Skill details.
    View { skill: String },
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

    pub fn input(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_SOURCE",
            message: message.into(),
        }
    }

    pub fn filesystem(message: impl Into<String>) -> Self {
        Self {
            code: "SERVICE_UNAVAILABLE",
            message: message.into(),
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
        Command::Install { source, global } => {
            let scope = scope(global);
            let source = InstallSource::parse(&source);
            if source == InstallSource::BundledSkilld && scope != InstallScope::Global {
                return Err(CommandError::input(
                    "install the skilld-maintained Skill with --global",
                ));
            }
            let name = host.install(source, scope)?;
            Ok(vec![format!("Installed Skill {name}.")])
        }
        Command::List { global } => host.list(scope(global)),
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
        Command::Search { query } => unavailable(format!(
            "Skill search is unavailable in this proof build: {}",
            query.join(" ")
        )),
        Command::View { skill } => unavailable(format!(
            "Skill details are unavailable in this proof build: {skill}"
        )),
        Command::Remove { skill, global } => unavailable(format!(
            "Skill removal is unavailable in this proof build: {skill} ({})",
            scope_name(global)
        )),
        Command::Upgrade { skill } => unavailable(format!(
            "Skill upgrade is unavailable in this proof build: {}",
            skill.unwrap_or_else(|| "all installed Skills".to_owned())
        )),
        Command::Verify { skill } => unavailable(format!(
            "source verification is unavailable in this proof build: {}",
            skill.unwrap_or_else(|| "all installed Skills".to_owned())
        )),
        Command::Config {
            command: ConfigCommand::Get { key },
        } => unavailable(format!(
            "configuration read is unavailable in this proof build: {key}"
        )),
        Command::Config {
            command: ConfigCommand::Set { key, value },
        } => unavailable(format!(
            "configuration write is unavailable in this proof build: {key}={value}"
        )),
        Command::Config {
            command: ConfigCommand::List,
        } => unavailable("configuration is unavailable in this proof build".to_owned()),
    }
}

fn unavailable<T>(message: String) -> Result<T, CommandError> {
    Err(CommandError::service(message))
}

fn scope(global: bool) -> InstallScope {
    if global {
        InstallScope::Global
    } else {
        InstallScope::Project
    }
}

fn scope_name(global: bool) -> &'static str {
    if global { "global" } else { "project" }
}

#[derive(Clone, Debug)]
pub struct LocalHost {
    project_root: PathBuf,
    global_root: PathBuf,
    bundled_skill: Option<PathBuf>,
}

impl LocalHost {
    pub fn new(project_root: PathBuf, global_root: PathBuf) -> Self {
        Self {
            project_root,
            global_root,
            bundled_skill: None,
        }
    }

    pub fn with_bundled_skill(mut self, path: PathBuf) -> Self {
        self.bundled_skill = Some(path);
        self
    }

    fn store(&self, scope: InstallScope) -> LocalStore {
        match scope {
            InstallScope::Project => LocalStore::new(self.project_root.join(".skills")),
            InstallScope::Global => LocalStore::new(self.global_root.join("skills")),
        }
    }
}

impl Host for LocalHost {
    fn list(&self, scope: InstallScope) -> Result<Vec<String>, CommandError> {
        self.store(scope)
            .list()
            .map_err(|error| CommandError::filesystem(error.to_string()))
    }

    fn install(&self, source: InstallSource, scope: InstallScope) -> Result<String, CommandError> {
        let source = match source {
            InstallSource::Local(path) if path.is_absolute() => path,
            InstallSource::Local(path) => self.project_root.join(path),
            InstallSource::BundledSkilld => self.bundled_skill.clone().ok_or_else(|| {
                CommandError::service(
                    "the bundled search and install Skill is unavailable in this build",
                )
            })?,
            InstallSource::Remote(source) => {
                return Err(CommandError::service(format!(
                    "Artifact delivery is unavailable in this proof build: {source}"
                )));
            }
        };

        self.store(scope)
            .install_from(&source)
            .map(|name| name.to_string())
            .map_err(|error| CommandError::filesystem(error.to_string()))
    }
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
    fn global_skilld_install_uses_the_bundled_skill_contract() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = run(
            ["skilld", "install", "skilld", "--global"],
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

    #[test]
    fn bundled_skilld_uses_the_global_skill_store() {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let data = temporary.path().join("data");
        let bundled = temporary.path().join("skilld");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&bundled).unwrap();
        std::fs::write(bundled.join("SKILL.md"), "fixture").unwrap();
        let host = LocalHost::new(project, data.clone()).with_bundled_skill(bundled);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = run(
            ["skilld", "install", "skilld", "--global"],
            &host,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(result.exit_code, 0);
        assert_eq!(
            std::fs::read_to_string(data.join("skills/skilld/SKILL.md")).unwrap(),
            "fixture"
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn version_uses_the_shared_rust_command() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = run(
            ["skilld", "--version"],
            &RecordingHost,
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(result.exit_code, 0);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!("skilld {VERSION}\n")
        );
        assert!(stderr.is_empty());
    }
}
