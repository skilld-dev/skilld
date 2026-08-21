use clap::error::ErrorKind;
use serde::Serialize;
use skilld_core::UpdatePlanV1;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    format!("{error}\n").into_bytes()
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

fn render_human(outcome: &SearchOutcome, width: u16, color: bool) -> String {
    let width = usize::from(width);
    let mut output = String::new();
    let heading = format!("Skill search  {}", terminal_text(&outcome.query));
    for line in wrap(&heading, width) {
        output.push_str(&styled(&line, "\u{1b}[1m\u{1b}[36m", color));
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
        let empty = format!("No Skills found for {}.", terminal_text(&outcome.query));
        for line in wrap(&empty, width) {
            output.push_str(&line);
            output.push('\n');
        }
        output.push_str("Try a shorter search.\n");
        return output;
    }

    for item in &outcome.items {
        output.push('\n');
        let name = terminal_text(&item.name);
        let stars = format!("{} stars", grouped_number(item.stargazer_count));
        if 2 + display_width(&name) + 2 + display_width(&stars) <= width {
            let gap = width - 2 - display_width(&name) - display_width(&stars);
            output.push_str("  ");
            output.push_str(&styled(&name, "\u{1b}[1m", color));
            output.push_str(&" ".repeat(gap));
            output.push_str(&styled(&stars, "\u{1b}[33m", color));
            output.push('\n');
        } else {
            for line in wrap(&name, width.saturating_sub(2)) {
                output.push_str("  ");
                output.push_str(&styled(&line, "\u{1b}[1m", color));
                output.push('\n');
            }
            output.push_str("  ");
            output.push_str(&styled(&stars, "\u{1b}[33m", color));
            output.push('\n');
        }

        if let Some(description) = &item.description {
            for line in wrap(&terminal_text(description), width.saturating_sub(2)) {
                output.push_str("  ");
                output.push_str(&line);
                output.push('\n');
            }
        }
        let install = format!("Install: skilld install {}", terminal_text(&item.selector));
        for line in wrap(&install, width.saturating_sub(2)) {
            output.push_str("  ");
            output.push_str(&styled(&line, "\u{1b}[2m", color));
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

fn terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn wrap(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in value.split_whitespace() {
        let separator = usize::from(!current.is_empty());
        if display_width(&current) + separator + display_width(word) > width && !current.is_empty()
        {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        if display_width(word) <= width {
            current.push_str(word);
        } else {
            let mut chunk = String::new();
            for character in word.chars() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if !chunk.is_empty() && display_width(&chunk) + character_width > width {
                    lines.push(std::mem::take(&mut chunk));
                }
                chunk.push(character);
            }
            current = chunk;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

fn grouped_number(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    output
}

fn styled(value: &str, style: &str, color: bool) -> String {
    if color {
        format!("{style}{value}\u{1b}[0m")
    } else {
        value.to_owned()
    }
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
