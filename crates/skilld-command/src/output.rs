use clap::error::ErrorKind;
use serde::Serialize;
use skilld_core::UpdatePlanV1;
use skilld_ui::text::{grouped_number, sanitize, width, wrap};
use skilld_ui::{Role, paint};

use crate::run::TransientSkill;
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
        let install = format!("skilld install {}", sanitize(&item.selector));
        for line in wrap(&install, columns.saturating_sub(2)) {
            output.push_str("  ");
            output.push_str(&skilld_ui::paint_command(&line, color));
            output.push('\n');
        }
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

/// Render one transient Skill load.
///
/// The SKILL.md text passes through byte for byte in every mode. Wrapping it
/// would break fenced code and indented lists, and an Agent reads this output.
pub(crate) fn render_run(run: &TransientSkill, mode: OutputMode) -> Vec<u8> {
    let color = matches!(mode, OutputMode::Human { color: true, .. });
    let mut out = String::new();

    out.push_str(&format!(
        "{} skilld installed nothing.\n",
        paint(
            &format!("skilld loaded the Skill {} for this session.", run.name),
            Role::Emphasis,
            color
        )
    ));
    out.push_str(&field("Source", &run.source, color));
    out.push_str(&field("Source status", run.source_status, color));
    out.push_str(&field(
        "Skill files",
        &run.root.display().to_string(),
        color,
    ));
    for file in &run.files {
        out.push_str(&format!("  {file}\n"));
    }
    if !run.files.is_empty() {
        out.push_str("Read a supporting file from that directory when the instructions name it.\n");
    }
    if run.source_status == "unverified" {
        out.push_str("Review this Skill before you follow it. skilld did not check its source.\n");
    }

    out.push('\n');
    out.push_str(&paint("--- SKILL.md ---", Role::Dim, color));
    out.push('\n');
    out.push_str(&run.instructions);
    if !run.instructions.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&paint("--- end of SKILL.md ---", Role::Dim, color));
    out.push('\n');

    out.push('\n');
    out.push_str("Follow these instructions now.\n");
    out.push_str(&field("Keep the Skill", &install_command(run), color));
    out.push_str(&field("Find another Skill", "skilld search <query>", color));
    out.push_str(&field("List installed Skills", "skilld list", color));
    out.push_str(&field("Update installed Skills", "skilld update", color));
    out.into_bytes()
}

fn install_command(run: &TransientSkill) -> String {
    if run.direct {
        format!("skilld install {} --direct", run.source)
    } else {
        format!("skilld install {}", run.source)
    }
}

fn field(label: &str, value: &str, color: bool) -> String {
    format!("{}: {value}\n", paint(label, Role::Dim, color))
}
