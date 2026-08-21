//! Screens: command output as semantic lines with two renderings.
//!
//! A [`Line`] carries the exact Plain-mode text plus how it should look for
//! humans. Plain mode prints the text untouched for machines; Human mode adds
//! glyphs, theme roles, aligned fields, and optional hyperlinks.

use crate::text::{pad_to, width};
use crate::theme::{Role, paint};

/// The success glyph prefixing completed work.
pub const GLYPH_SUCCESS: &str = "✓";
/// The attention glyph prefixing degraded or outdated results.
pub const GLYPH_WARN: &str = "⚠";
/// The failure glyph prefixing errors and required action.
pub const GLYPH_ERROR: &str = "✗";
/// The neutral glyph prefixing informational rows.
pub const GLYPH_NOTE: &str = "•";

/// A marker selects the glyph and role for a row or group heading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Marker {
    /// Completed work: green check.
    Success,
    /// Attention: yellow warning.
    Warn,
    /// Failure: red cross.
    Error,
    /// Information: brand bullet.
    Note,
}

impl Marker {
    const fn glyph(self) -> &'static str {
        match self {
            Self::Success => GLYPH_SUCCESS,
            Self::Warn => GLYPH_WARN,
            Self::Error => GLYPH_ERROR,
            Self::Note => GLYPH_NOTE,
        }
    }

    const fn role(self) -> Role {
        match self {
            Self::Success => Role::Success,
            Self::Warn => Role::Warn,
            Self::Error => Role::Error,
            Self::Note => Role::Brand,
        }
    }

    fn paint_glyph(self, color: bool) -> String {
        paint(self.glyph(), self.role(), color)
    }
}

/// One rendered output document.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Screen {
    /// A Human-only heading, rendered with the brand role.
    pub header: Option<String>,
    /// The output body.
    pub lines: Vec<Line>,
}

impl Screen {
    /// A screen with no heading.
    pub fn new(lines: Vec<Line>) -> Self {
        Self {
            header: None,
            lines,
        }
    }

    /// A screen with a Human-only heading.
    pub fn with_header(header: impl Into<String>, lines: Vec<Line>) -> Self {
        Self {
            header: Some(header.into()),
            lines,
        }
    }

    /// The machine rendering: one exact line per record, heading excluded.
    pub fn render_plain(&self) -> String {
        let mut output = String::new();
        for line in &self.lines {
            output.push_str(line.plain_text());
            output.push('\n');
        }
        output
    }

    /// The terminal rendering with glyphs, theme roles, and aligned fields.
    pub fn render_human(&self, color: bool) -> String {
        let mut output = String::new();
        if let Some(header) = &self.header {
            output.push_str(&paint(header, Role::Brand, color));
            output.push('\n');
            if !self.lines.is_empty() {
                output.push('\n');
            }
        }
        let label_width = self
            .lines
            .iter()
            .filter_map(Line::field_label)
            .map(width)
            .max()
            .unwrap_or(0);
        for line in &self.lines {
            output.push_str(&line.render_human(color, label_width));
            output.push('\n');
        }
        output
    }
}

/// The plain text of every line, for tests and machine consumers.
pub fn plain_lines(lines: &[Line]) -> Vec<&str> {
    lines.iter().map(Line::plain_text).collect()
}

/// One semantic output line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    plain: String,
    kind: LineKind,
}

/// How a line renders for humans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineKind {
    /// Unstyled text.
    Plain,
    /// A highlighted entry, such as an installed Skill name.
    Item,
    /// Completed work, prefixed with the success glyph.
    Success,
    /// Attention, prefixed with the warn glyph.
    Warn,
    /// Failure, prefixed with the error glyph.
    Error,
    /// A dimmed hint or follow-up step.
    Hint,
    /// A label and value pair, aligned across the screen.
    Field {
        label: String,
        value: String,
        /// A terminal hyperlink target for the value, used when color is on.
        url: Option<String>,
    },
    /// One titled row with indented detail rows underneath. The Plain text
    /// may span several sentences joined by newlines; Human renders the
    /// title row once, then each detail on its own line.
    Record {
        marker: Marker,
        title: String,
        /// A dim badge after the title, such as an Agent list or state.
        status: Option<String>,
        /// Labelled detail rows shown under the title.
        details: Vec<Detail>,
    },
    /// A heading plus one row per item, so long name lists stay scannable.
    /// The Plain text is the full sentence with every name inline.
    Group {
        marker: Marker,
        heading: String,
        /// One (name, meta) pair per rendered row.
        items: Vec<(String, String)>,
    },
}

/// How a record detail value renders for humans. The class, not the text,
/// decides the look, the way syntax highlighting classes tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetailKind {
    /// Unstyled text.
    Plain,
    /// A command the user can type: highlighted as code.
    Command,
    /// A filesystem path: dimmed.
    Path,
}

/// One labelled row under a record title.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Detail {
    /// The dim label.
    pub label: &'static str,
    /// The row value.
    pub value: String,
    /// How the value renders.
    pub kind: DetailKind,
}

impl Detail {
    /// An unstyled detail row.
    pub fn plain(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            kind: DetailKind::Plain,
        }
    }

    /// A command the user can type. Rendered as highlighted code.
    pub fn command(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            kind: DetailKind::Command,
        }
    }

    /// A filesystem path. Rendered dim.
    pub fn path(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            kind: DetailKind::Path,
        }
    }
}

impl Line {
    /// Unstyled text whose Plain and Human renderings match.
    pub fn plain(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            plain: text,
            kind: LineKind::Plain,
        }
    }

    /// A highlighted entry.
    pub fn item(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            plain: text,
            kind: LineKind::Item,
        }
    }

    /// Completed work; Plain text is the sentence without the glyph.
    pub fn success(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            plain: text,
            kind: LineKind::Success,
        }
    }

    /// Attention; Plain text is the sentence without the glyph.
    pub fn warn(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            plain: text,
            kind: LineKind::Warn,
        }
    }

    /// Failure; Plain text is the sentence without the glyph.
    pub fn error(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            plain: text,
            kind: LineKind::Error,
        }
    }

    /// A dimmed hint.
    pub fn hint(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            plain: text,
            kind: LineKind::Hint,
        }
    }

    /// A label and value pair; Plain text is `label: value`.
    pub fn field(label: impl Into<String>, value: impl Into<String>) -> Self {
        let label = label.into();
        let value = value.into();
        Self::field_plain(format!("{label}: {value}"), label, value)
    }

    /// A label and value pair with an explicit Plain rendering, for records
    /// like `key=value`.
    pub fn field_plain(
        plain: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            plain: plain.into(),
            kind: LineKind::Field {
                label: label.into(),
                value: value.into(),
                url: None,
            },
        }
    }

    /// A label and value pair whose value links to `url` in Human mode.
    pub fn linked_field(
        label: impl Into<String>,
        value: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        let label = label.into();
        let value = value.into();
        Self {
            plain: format!("{label}: {value}"),
            kind: LineKind::Field {
                label,
                value,
                url: Some(url.into()),
            },
        }
    }

    /// The exact Plain-mode text.
    pub fn plain_text(&self) -> &str {
        &self.plain
    }

    /// A record row: `glyph title  status` with labelled detail rows
    /// underneath. `plain` is the exact machine sentence, which may join
    /// several sentences with newlines.
    pub fn record(
        marker: Marker,
        plain: impl Into<String>,
        title: impl Into<String>,
        status: Option<String>,
        details: Vec<Detail>,
    ) -> Self {
        Self {
            plain: plain.into(),
            kind: LineKind::Record {
                marker,
                title: title.into(),
                status,
                details,
            },
        }
    }

    /// A group heading with one row per item. `plain` is the exact machine
    /// sentence with every name inline.
    pub fn group(
        marker: Marker,
        plain: impl Into<String>,
        heading: impl Into<String>,
        items: Vec<(String, String)>,
    ) -> Self {
        Self {
            plain: plain.into(),
            kind: LineKind::Group {
                marker,
                heading: heading.into(),
                items,
            },
        }
    }

    fn field_label(&self) -> Option<&str> {
        match &self.kind {
            LineKind::Field { label, .. } => Some(label),
            _ => None,
        }
    }

    fn render_human(&self, color: bool, label_width: usize) -> String {
        match &self.kind {
            LineKind::Plain => self.plain.clone(),
            LineKind::Item => paint(&self.plain, Role::Emphasis, color),
            LineKind::Success => glyphed(GLYPH_SUCCESS, Role::Success, &self.plain, color),
            LineKind::Warn => glyphed(GLYPH_WARN, Role::Warn, &self.plain, color),
            LineKind::Error => glyphed(GLYPH_ERROR, Role::Error, &self.plain, color),
            LineKind::Hint => paint(&self.plain, Role::Dim, color),
            LineKind::Field { label, value, url } => {
                let label = pad_to(label, label_width);
                let value = match url {
                    Some(url) if color => hyperlink(value, url),
                    _ => value.clone(),
                };
                format!("{}: {value}", paint(&label, Role::Dim, color))
            }
            LineKind::Record {
                marker,
                title,
                status,
                details,
            } => {
                let mut output = format!("{} {title}", marker.paint_glyph(color));
                if let Some(status) = status {
                    output.push_str(&format!("  {}", paint(status, Role::Dim, color)));
                }
                let label_width = details
                    .iter()
                    .map(|detail| width(detail.label))
                    .max()
                    .unwrap_or(0);
                for detail in details {
                    let label = pad_to(detail.label, label_width);
                    let value = match detail.kind {
                        DetailKind::Plain => detail.value.clone(),
                        DetailKind::Command => crate::spans::paint_command(&detail.value, color),
                        DetailKind::Path => paint(&detail.value, Role::Dim, color),
                    };
                    output.push('\n');
                    output.push_str(&format!("  {label}  {value}"));
                }
                output
            }
            LineKind::Group {
                marker,
                heading,
                items,
            } => {
                let mut output = format!("{} {heading}", marker.paint_glyph(color));
                let name_width = items.iter().map(|(name, _)| width(name)).max().unwrap_or(0);
                for (name, meta) in items {
                    output.push('\n');
                    let name = pad_to(name, name_width);
                    output.push_str(&format!("  {name}"));
                    if !meta.is_empty() {
                        output.push_str(&format!("  {}", paint(meta, Role::Dim, color)));
                    }
                }
                output
            }
        }
    }
}

fn glyphed(glyph: &str, role: Role, text: &str, color: bool) -> String {
    format!("{} {text}", paint(glyph, role, color))
}

/// Wrap `value` in an OSC 8 terminal hyperlink.
fn hyperlink(value: &str, url: &str) -> String {
    format!("\u{1b}]8;;{url}\u{1b}\\{value}\u{1b}]8;;\u{1b}\\")
}

/// True when the value contains an OSC 8 hyperlink.
#[cfg(test)]
pub(crate) fn has_hyperlink(value: &str) -> bool {
    value.contains("\u{1b}]8;;")
}

#[cfg(test)]
mod tests {
    use super::{Detail, Line, Marker, Screen, has_hyperlink};

    #[test]
    fn plain_rendering_matches_the_machine_contract() {
        let screen = Screen::new(vec![
            Line::field_plain("agent.targets=codex", "agent.targets", "codex"),
            Line::success("Installed Skill grill-me."),
        ]);

        assert_eq!(
            screen.render_plain(),
            "agent.targets=codex\nInstalled Skill grill-me.\n"
        );
    }

    #[test]
    fn human_header_is_brand_colored_and_excluded_from_plain() {
        let screen = Screen::with_header("Installed Skills", vec![Line::item("grill-me")]);

        assert_eq!(screen.render_plain(), "grill-me\n");
        assert_eq!(
            screen.render_human(true),
            "\u{1b}[1m\u{1b}[36mInstalled Skills\u{1b}[0m\n\n\u{1b}[1mgrill-me\u{1b}[0m\n"
        );
        assert_eq!(screen.render_human(false), "Installed Skills\n\ngrill-me\n");
    }

    #[test]
    fn linked_fields_hyperlink_only_with_color() {
        let screen = Screen::new(vec![Line::linked_field(
            "Source",
            "skilld-dev/skilld",
            "https://github.com/skilld-dev/skilld",
        )]);

        let colored = screen.render_human(true);
        let mono = screen.render_human(false);

        assert_eq!(screen.render_plain(), "Source: skilld-dev/skilld\n");
        assert!(has_hyperlink(&colored));
        assert!(!has_hyperlink(&mono));
        assert!(colored.contains("https://github.com/skilld-dev/skilld"));
        assert_eq!(mono, "Source: skilld-dev/skilld\n");
    }

    #[test]
    fn records_render_a_title_row_with_aligned_details() {
        let line = Line::record(
            Marker::Warn,
            "Unmanaged Skill vue-testing (claude-code). Candidate source sel, 0 stars.\nDelete /tmp/x, then run skilld install sel.",
            "vue-testing",
            Some("claude-code · unmanaged".to_owned()),
            vec![
                Detail::plain("candidate", "sel"),
                Detail::command("install", "skilld install sel"),
            ],
        );

        assert_eq!(
            line.render_human(false, 0),
            concat!(
                "⚠ vue-testing  claude-code · unmanaged\n",
                "  candidate  sel\n",
                "  install    skilld install sel"
            )
        );
    }

    #[test]
    fn groups_render_one_row_per_item_with_aligned_names() {
        let line = Line::group(
            Marker::Warn,
            "No Repository match for 2 Skills (b (codex), longer-name (amp)).",
            "No Repository match for 2 Skills",
            vec![
                ("b".to_owned(), "codex".to_owned()),
                ("longer-name".to_owned(), "amp".to_owned()),
            ],
        );

        assert_eq!(
            line.render_human(false, 0),
            concat!(
                "⚠ No Repository match for 2 Skills\n",
                "  b            codex\n",
                "  longer-name  amp"
            )
        );
    }

    #[test]
    fn records_and_groups_keep_their_plain_sentences() {
        let screen = Screen::new(vec![
            Line::record(
                Marker::Note,
                "Local Skill example.",
                "example",
                Some("local".to_owned()),
                Vec::new(),
            ),
            Line::group(
                Marker::Warn,
                "No Repository match for 1 Skill (b (codex)).",
                "No Repository match for 1 Skill",
                vec![("b".to_owned(), "codex".to_owned())],
            ),
        ]);

        assert_eq!(
            screen.render_plain(),
            concat!(
                "Local Skill example.\n",
                "No Repository match for 1 Skill (b (codex)).\n"
            )
        );
    }

    #[test]
    fn empty_screens_render_nothing() {
        let screen = Screen::new(vec![]);

        assert_eq!(screen.render_plain(), "");
        assert_eq!(screen.render_human(true), "");
    }
}
