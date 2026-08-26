use clap::error::ErrorKind;
use serde::Serialize;
use skilld_core::UpdatePlanV1;
use skilld_ui::text::{grouped_number, sanitize, width, wrap};
use skilld_ui::{Role, paint};

use crate::run::{FileContent, PulledFile, RunOutcome, SkillOrigin, TransientSkill};
use crate::{CommandError, CommandErrorKind};

const JSON_SCHEMA_VERSION: u8 = 1;
const MIN_WIDTH: u16 = 20;
const MAX_WIDTH: u16 = 240;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputContext {
    HumanTerminal { width: u16, color: bool },
    Plain,
}

impl OutputContext {
    pub fn auto(
        stdout_is_terminal: bool,
        active_agent: bool,
        ci: bool,
        no_color: bool,
        term_is_dumb: bool,
        width: u16,
    ) -> Self {
        if active_agent || ci || !stdout_is_terminal {
            return Self::Plain;
        }
        Self::HumanTerminal {
            width: width.clamp(MIN_WIDTH, MAX_WIDTH),
            color: !no_color && !term_is_dumb,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutputMode {
    Human { width: u16, color: bool },
    Plain,
    JsonV1,
}

pub(crate) fn resolve_mode(json: bool, plain: bool, context: OutputContext) -> OutputMode {
    if json {
        OutputMode::JsonV1
    } else if plain {
        OutputMode::Plain
    } else {
        match context {
            OutputContext::HumanTerminal { width, color } => OutputMode::Human { width, color },
            OutputContext::Plain => OutputMode::Plain,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SearchOutcome {
    pub query: String,
    pub items: Vec<SearchItem>,
    pub total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchItem {
    pub name: String,
    pub selector: String,
    pub description: Option<String>,
    pub stargazer_count: u64,
}

pub(crate) fn render_search(
    outcome: &SearchOutcome,
    mode: OutputMode,
) -> Result<Vec<u8>, CommandError> {
    match mode {
        OutputMode::Human { width, color } => Ok(render_human(outcome, width, color).into_bytes()),
        OutputMode::Plain => Ok(render_plain(outcome).into_bytes()),
        OutputMode::JsonV1 => render_json_success(
            "search",
            JsonSearchData {
                query: &outcome.query,
                items: &outcome.items,
                total: outcome.total,
            },
            "Skill search output could not be encoded",
        ),
    }
}

pub(crate) fn render_display(kind: ErrorKind, path: &str, text: &str) -> Vec<u8> {
    let (command, data) = if kind == ErrorKind::DisplayVersion {
        (
            "version",
            JsonDisplayData::Version {
                name: "skilld",
                version: skilld_core::VERSION,
            },
        )
    } else {
        ("help", JsonDisplayData::Help { path, text })
    };
    render_json_success(command, data, "display output could not be encoded")
        .unwrap_or_else(|_| b"OUTPUT_RENDER_FAILED: display output could not be encoded\n".to_vec())
}

pub(crate) fn render_update_check(
    outcome: &UpdatePlanV1,
    mode: OutputMode,
) -> Result<Vec<u8>, CommandError> {
    if mode != OutputMode::JsonV1 {
        return Err(CommandError::service(
            "Skill update check output needs JSON mode",
        ));
    }
    render_json_success(
        "update",
        outcome,
        "Skill update check output could not be encoded",
    )
}

fn render_json_success<T: Serialize>(
    command: &'static str,
    data: T,
    encoding_error: &'static str,
) -> Result<Vec<u8>, CommandError> {
    serde_json::to_vec(&JsonSuccess {
        schema_version: JSON_SCHEMA_VERSION,
        tag: "Success",
        command,
        data,
        notices: Vec::new(),
    })
    .map(|mut bytes| {
        bytes.push(b'\n');
        bytes
    })
    .map_err(|_| CommandError::service(encoding_error))
}

pub(crate) fn render_error(error: &CommandError, mode: OutputMode) -> Vec<u8> {
    if mode == OutputMode::JsonV1 {
        return serde_json::to_vec(&JsonFailure {
            schema_version: JSON_SCHEMA_VERSION,
            tag: match error.kind {
                CommandErrorKind::Usage => "UsageError",
                CommandErrorKind::Operation => "OperationError",
            },
            error: JsonError {
                code: error.code,
                message: &error.message,
            },
        })
        .map(|mut bytes| {
            bytes.push(b'\n');
            bytes
        })
        .unwrap_or_else(|_| b"OUTPUT_RENDER_FAILED: error output could not be encoded\n".to_vec());
    }
    match mode {
        OutputMode::Human { color, .. } => format!(
            "{} {} {}\n",
            skilld_ui::paint("✗", skilld_ui::Role::Error, color),
            skilld_ui::paint(&error.message, skilld_ui::Role::Emphasis, color),
            skilld_ui::paint(&format!("({})", error.code), skilld_ui::Role::Dim, color),
        )
        .into_bytes(),
        OutputMode::Plain => format!("{error}\n").into_bytes(),
        OutputMode::JsonV1 => unreachable!("JSON errors return early"),
    }
}

fn render_plain(outcome: &SearchOutcome) -> String {
    let mut output = String::new();
    for item in &outcome.items {
        output.push_str(&escape_plain(&item.name));
        output.push('\t');
        output.push_str(&escape_plain(&item.selector));
        output.push('\t');
        output.push_str(&item.stargazer_count.to_string());
        output.push('\t');
        output.push_str(&escape_plain(
            item.description.as_deref().unwrap_or_default(),
        ));
        output.push('\n');
    }
    output
}

fn render_human(outcome: &SearchOutcome, terminal_width: u16, color: bool) -> String {
    let columns = usize::from(terminal_width);
    let mut output = String::new();
    let heading = format!("Skill search  {}", sanitize(&outcome.query));
    for line in wrap(&heading, columns) {
        output.push_str(&paint(&line, Role::Brand, color));
        output.push('\n');
    }
    let shown = outcome.items.len();
    output.push_str(&format!(
        "{} of {} {}\n",
        shown,
        outcome.total,
        if outcome.total == 1 {
            "Skill"
        } else {
            "Skills"
        }
    ));

    if outcome.items.is_empty() {
        output.push('\n');
        let empty = format!("No Skills found for {}.", sanitize(&outcome.query));
        for line in wrap(&empty, columns) {
            output.push_str(&line);
            output.push('\n');
        }
        output.push_str("Try a shorter search.\n");
        return output;
    }

    for item in &outcome.items {
        output.push('\n');
        let name = sanitize(&item.name);
        let stars = format!("{} stars", grouped_number(item.stargazer_count));
        if 2 + width(&name) + 2 + width(&stars) <= columns {
            let gap = columns - 2 - width(&name) - width(&stars);
            output.push_str("  ");
            output.push_str(&paint(&name, Role::Emphasis, color));
            output.push_str(&" ".repeat(gap));
            output.push_str(&paint(&stars, Role::Warn, color));
            output.push('\n');
        } else {
            for line in wrap(&name, columns.saturating_sub(2)) {
                output.push_str("  ");
                output.push_str(&paint(&line, Role::Emphasis, color));
                output.push('\n');
            }
            output.push_str("  ");
            output.push_str(&paint(&stars, Role::Warn, color));
            output.push('\n');
        }

        if let Some(description) = &item.description {
            for line in wrap(&sanitize(description), columns.saturating_sub(2)) {
                output.push_str("  ");
                output.push_str(&line);
                output.push('\n');
            }
        }
        let run = shell_command(&["skilld".to_owned(), "run".to_owned(), item.selector.clone()]);
        output.push_str("  ");
        output.push_str(&skilld_ui::paint_command(&run, color));
        output.push('\n');
    }
    output
}

fn escape_plain(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\t' => output.push_str("\\t"),
            '\r' => output.push_str("\\r"),
            '\n' => output.push_str("\\n"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{{{:04X}}}", u32::from(character)));
            }
            character => output.push(character),
        }
    }
    output
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSuccess<T> {
    schema_version: u8,
    #[serde(rename = "_tag")]
    tag: &'static str,
    command: &'static str,
    data: T,
    notices: Vec<JsonNotice>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum JsonDisplayData<'a> {
    Help {
        path: &'a str,
        text: &'a str,
    },
    Version {
        name: &'static str,
        version: &'static str,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSearchData<'a> {
    query: &'a str,
    items: &'a [SearchItem],
    total: u64,
}

#[derive(Serialize)]
struct JsonNotice;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonFailure<'a> {
    schema_version: u8,
    #[serde(rename = "_tag")]
    tag: &'static str,
    error: JsonError<'a>,
}

#[derive(Serialize)]
struct JsonError<'a> {
    code: &'a str,
    message: &'a str,
}

/// Render one transient Skill load, or the supporting files an Agent asked for.
pub(crate) fn render_run(outcome: &RunOutcome, mode: OutputMode) -> Result<Vec<u8>, CommandError> {
    match (outcome, mode) {
        (RunOutcome::Load(skill), OutputMode::JsonV1) => render_json_success(
            "run",
            load_json(skill),
            "Skill run output could not be encoded",
        ),
        (
            RunOutcome::Files {
                skill,
                origin,
                source_status,
                revision,
                files,
            },
            OutputMode::JsonV1,
        ) => render_json_success(
            "run",
            files_json(skill, origin, source_status, revision.as_deref(), files),
            "Skill run output could not be encoded",
        ),
        (RunOutcome::Load(skill), _) => Ok(render_load(skill, colored(mode)).into_bytes()),
        (
            RunOutcome::Files {
                skill,
                origin,
                source_status,
                revision,
                files,
            },
            _,
        ) => Ok(render_files(
            skill,
            origin,
            source_status,
            revision.as_deref(),
            files,
            colored(mode),
        )
        .into_bytes()),
    }
}

const fn colored(mode: OutputMode) -> bool {
    matches!(mode, OutputMode::Human { color: true, .. })
}

fn render_load(skill: &TransientSkill, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}\n",
        paint(
            &format!(
                "skilld loaded the transient Skill {} for this session.",
                sanitize(&skill.name)
            ),
            Role::Emphasis,
            color
        )
    ));

    match &skill.origin {
        SkillOrigin::Remote { source, .. } => {
            out.push_str("skilld retained no Skill files.\n");
            out.push_str("It created no lockfile entry, Agent target, or project file.\n");
            out.push_str(&field("Source", source, color));
        }
        SkillOrigin::Local { root } => {
            out.push_str("This Skill already sits on disk. skilld wrote no Skill files.\n");
            out.push_str(&field("Source", &root.display().to_string(), color));
        }
    }
    if let Some(revision) = &skill.revision {
        out.push_str(&field("Revision", revision, color));
    }
    out.push_str(&field("Source status", skill.source_status, color));
    out.push_str(source_status_caution(skill.source_status));

    out.push('\n');
    out.push_str(&paint("--- SKILL.md ---", Role::Dim, color));
    out.push('\n');
    let instructions = safe_terminal_text(&skill.instructions);
    out.push_str(&instructions);
    if !instructions.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&paint("--- end of SKILL.md ---", Role::Dim, color));
    out.push('\n');

    out.push('\n');
    out.push_str("Follow these instructions now.\n");
    out.push_str(&render_inventory(skill, color));
    out.push_str(&render_install_guidance(&skill.origin, color));
    out
}

fn render_inventory(skill: &TransientSkill, color: bool) -> String {
    if skill.files.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("The instructions may name a supporting file. skilld printed none of them.\n");
    if let SkillOrigin::Local { root } = &skill.origin {
        out.push_str(&format!(
            "Read one from {}, or use --file to print it here.\n",
            sanitize(&root.display().to_string())
        ));
    } else {
        out.push_str("Use --file to read the ones you need.\n");
    }
    out.push('\n');
    out.push_str(&paint(
        &format!("Supporting files ({}):", skill.files.len()),
        Role::Emphasis,
        color,
    ));
    out.push('\n');
    for file in &skill.files {
        out.push_str(&format!(
            "  {}  {} bytes  {}\n",
            sanitize(&file.path),
            grouped_number(file.size),
            file.kind.as_str()
        ));
        if file.kind.is_readable() {
            out.push_str(&format!(
                "    {}\n",
                paint(
                    &shell_command(&read_argv(
                        &skill.origin,
                        skill.revision.as_deref(),
                        &file.path,
                        false,
                    )),
                    Role::Brand,
                    color,
                )
            ));
        } else {
            out.push_str(&format!(
                "    skilld will not print this {} file. Install the Skill to use it.\n",
                file.kind.as_str()
            ));
        }
    }
    out.push('\n');
    out
}

fn render_files(
    skill: &str,
    origin: &SkillOrigin,
    source_status: &str,
    revision: Option<&str>,
    files: &[PulledFile],
    color: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}\n",
        paint(
            &format!(
                "skilld read supporting files from the transient Skill {}.",
                sanitize(skill)
            ),
            Role::Emphasis,
            color,
        )
    ));
    out.push_str(&origin_field(origin, color));
    if let Some(revision) = revision {
        out.push_str(&field("Revision", revision, color));
    }
    out.push_str(&field("Source status", source_status, color));
    out.push_str(source_status_caution(source_status));
    out.push('\n');
    for file in files {
        let path = sanitize(&file.path);
        out.push_str(&field("File", &path, color));
        out.push_str(&field(
            "Size",
            &format!("{} bytes", grouped_number(file.size)),
            color,
        ));
        out.push_str(&field("Kind", file.kind.as_str(), color));
        match &file.content {
            FileContent::Text(text) => {
                out.push('\n');
                out.push_str(&paint(&format!("--- {path} ---"), Role::Dim, color));
                out.push('\n');
                let text = safe_terminal_text(text);
                out.push_str(&text);
                if !text.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&paint(&format!("--- end of {path} ---"), Role::Dim, color));
                out.push('\n');
            }
            FileContent::Withheld { reason } => {
                out.push_str(&format!(
                    "skilld did not print this file, because {reason}.\n"
                ));
                out.push_str("Install the Skill to put this file on disk.\n");
            }
        }
        out.push('\n');
    }
    out
}

fn render_install_guidance(origin: &SkillOrigin, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&paint("To keep this Skill:", Role::Emphasis, color));
    out.push('\n');
    out.push_str(&format!(
        "  {}\n",
        shell_command(&install_argv(origin, false))
    ));
    out.push_str(&format!(
        "  {}\n",
        shell_command(&install_argv(origin, true))
    ));
    out.push_str("The first writes the Skill into this project and records it in the lockfile.\n");
    out.push_str("The second keeps it for every project.\n");
    out.push_str(
        "Ask the user before you install. An install writes files they did not request.\n",
    );
    out.push('\n');
    out.push_str(&field("Find another Skill", "skilld search <query>", color));
    out.push_str(&field("List installed Skills", "skilld list", color));
    out.push_str(&field("Update installed Skills", "skilld update", color));
    out
}

fn read_argv(origin: &SkillOrigin, revision: Option<&str>, path: &str, json: bool) -> Vec<String> {
    let mut argv = vec![
        "skilld".to_owned(),
        "run".to_owned(),
        source_argument(origin),
    ];
    if matches!(origin, SkillOrigin::Remote { direct: true, .. }) {
        argv.push("--direct".to_owned());
    }
    if let Some(revision) = revision {
        argv.push("--revision".to_owned());
        argv.push(revision.to_owned());
    }
    argv.push(format!("--file={path}"));
    if json {
        argv.push("--json".to_owned());
    }
    argv
}

fn install_argv(origin: &SkillOrigin, global: bool) -> Vec<String> {
    let mut argv = vec![
        "skilld".to_owned(),
        "install".to_owned(),
        install_source_argument(origin),
    ];
    if matches!(origin, SkillOrigin::Remote { direct: true, .. }) {
        argv.push("--direct".to_owned());
    }
    if global {
        argv.push("--global".to_owned());
    }
    argv
}

fn source_argument(origin: &SkillOrigin) -> String {
    match origin {
        SkillOrigin::Remote { source, .. } => source.clone(),
        SkillOrigin::Local { root } => root.display().to_string(),
    }
}

fn install_source_argument(origin: &SkillOrigin) -> String {
    match origin {
        SkillOrigin::Remote { exact_source, .. } => exact_source.clone(),
        SkillOrigin::Local { root } => root.display().to_string(),
    }
}

fn shell_command(argv: &[String]) -> String {
    argv.iter()
        .map(|argument| shell_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(argument: &str) -> String {
    let portable = !argument.is_empty()
        && argument
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte));
    if portable {
        return argument.to_owned();
    }
    format!("'{}'", argument.replace('\'', "'\\''"))
}

/// State what the status covers, on every status.
///
/// A verified Artifact proves where the bytes came from. It says nothing about
/// what the instructions ask an Agent to do, and the output must not imply it.
fn source_status_caution(status: &str) -> &'static str {
    match status {
        "verified" => {
            "skilld checked where this Skill came from, not what it asks you to do.\nRead it before you follow it.\n"
        }
        "unverified" => "skilld did not check this source. Read this Skill before you follow it.\n",
        _ => "Read this Skill before you follow it.\n",
    }
}

fn field(label: &str, value: &str, color: bool) -> String {
    format!("{}: {}\n", paint(label, Role::Dim, color), sanitize(value))
}

fn origin_field(origin: &SkillOrigin, color: bool) -> String {
    match origin {
        SkillOrigin::Remote { source, .. } => field("Source", source, color),
        SkillOrigin::Local { root } => field("Source", &root.display().to_string(), color),
    }
}

fn safe_terminal_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect()
}

#[derive(Serialize)]
#[serde(tag = "_tag", rename_all = "lowercase")]
enum JsonOrigin {
    Remote { source: String, direct: bool },
    Local { root: String },
}

fn origin_json(origin: &SkillOrigin) -> JsonOrigin {
    match origin {
        SkillOrigin::Remote { source, direct, .. } => JsonOrigin::Remote {
            source: source.clone(),
            direct: *direct,
        },
        SkillOrigin::Local { root } => JsonOrigin::Local {
            root: root.display().to_string(),
        },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSupportingFile {
    path: String,
    kind: &'static str,
    size: u64,
    readable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    read_argv: Option<Vec<String>>,
}

#[derive(Serialize)]
struct JsonInstallArgv {
    project: Vec<String>,
    global: Vec<String>,
}

#[derive(Serialize)]
#[serde(
    tag = "_tag",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
enum JsonRunData {
    Load {
        name: String,
        origin: JsonOrigin,
        source_status: &'static str,
        source_caution: &'static str,
        revision: Option<String>,
        wrote_skill_files: bool,
        instructions: String,
        files: Vec<JsonSupportingFile>,
        install_argv: JsonInstallArgv,
    },
    Files {
        name: String,
        origin: JsonOrigin,
        source_status: &'static str,
        source_caution: &'static str,
        revision: Option<String>,
        wrote_skill_files: bool,
        files: Vec<JsonPulledFile>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonPulledFile {
    path: String,
    kind: &'static str,
    size: u64,
    content: JsonFileContent,
}

#[derive(Serialize)]
#[serde(tag = "_tag", rename_all = "lowercase")]
enum JsonFileContent {
    Text { value: String },
    Withheld { reason: &'static str },
}

fn load_json(skill: &TransientSkill) -> JsonRunData {
    JsonRunData::Load {
        name: skill.name.clone(),
        origin: origin_json(&skill.origin),
        source_status: skill.source_status,
        source_caution: source_status_caution(skill.source_status).trim_end(),
        revision: skill.revision.clone(),
        wrote_skill_files: false,
        instructions: skill.instructions.clone(),
        files: skill
            .files
            .iter()
            .map(|file| JsonSupportingFile {
                path: file.path.clone(),
                kind: file.kind.as_str(),
                size: file.size,
                readable: file.kind.is_readable(),
                read_argv: file
                    .kind
                    .is_readable()
                    .then(|| read_argv(&skill.origin, skill.revision.as_deref(), &file.path, true)),
            })
            .collect(),
        install_argv: JsonInstallArgv {
            project: install_argv(&skill.origin, false),
            global: install_argv(&skill.origin, true),
        },
    }
}

fn files_json(
    skill: &str,
    origin: &SkillOrigin,
    source_status: &'static str,
    revision: Option<&str>,
    files: &[PulledFile],
) -> JsonRunData {
    JsonRunData::Files {
        name: skill.to_owned(),
        origin: origin_json(origin),
        source_status,
        source_caution: source_status_caution(source_status).trim_end(),
        revision: revision.map(str::to_owned),
        wrote_skill_files: false,
        files: files
            .iter()
            .map(|file| JsonPulledFile {
                path: file.path.clone(),
                kind: file.kind.as_str(),
                size: file.size,
                content: match &file.content {
                    FileContent::Text(text) => JsonFileContent::Text {
                        value: text.clone(),
                    },
                    FileContent::Withheld { reason } => JsonFileContent::Withheld { reason },
                },
            })
            .collect(),
    }
}
