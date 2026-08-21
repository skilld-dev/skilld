//! Inline token coloring, modelled on syntax highlighting.
//!
//! A command the user should type is a code literal: it must scan as one
//! visual object, not as prose. [`command_spans`] splits a command into
//! spans the way a highlighter splits a line of code: the binary is the
//! brand, the subcommand is the emphasis, flags dim, values plain.

use crate::theme::{Role, paint};

/// One run of text with one role, or unstyled text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Span {
    /// Unstyled text.
    Text(String),
    /// Text painted with a role.
    Styled(String, Role),
}

/// Split a skilld command into highlighted spans.
///
/// `skilld` is Brand, the first bare word after it is Emphasis (the
/// subcommand), flags starting with `-` are Dim, and everything else stays
/// plain so selectors and Skill names carry the weight.
pub fn command_spans(command: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut subcommand_used = false;
    for word in command.split_whitespace() {
        if spans.is_empty() && word == "skilld" {
            spans.push(Span::Styled(word.to_owned(), Role::Brand));
            continue;
        }
        if !subcommand_used && !word.starts_with('-') && spans.len() == 1 {
            subcommand_used = true;
            spans.push(Span::Styled(word.to_owned(), Role::Emphasis));
            continue;
        }
        if word.starts_with('-') && word.chars().any(|c| c.is_ascii_alphabetic()) {
            spans.push(Span::Styled(word.to_owned(), Role::Dim));
            continue;
        }
        spans.push(Span::Text(word.to_owned()));
    }
    spans
}

/// Join spans into one colored string. Without color, the plain text.
pub fn paint_spans(spans: &[Span], color: bool) -> String {
    if !color {
        let mut output = String::new();
        for span in spans {
            output.push_str(span_text(span));
            output.push(' ');
        }
        return output.trim_end().to_owned();
    }
    let mut output = String::new();
    for span in spans {
        match span {
            Span::Text(text) => {
                output.push_str(text);
                output.push(' ');
            }
            Span::Styled(text, role) => {
                output.push_str(&paint(text, *role, true));
                output.push(' ');
            }
        }
    }
    output.pop();
    output
}

/// The span text with any styling stripped. One trailing space per span
/// mirrors the colored join; callers trim.
fn span_text(span: &Span) -> &str {
    match span {
        Span::Text(text) | Span::Styled(text, _) => text,
    }
}

/// Paint a command string in one step.
pub fn paint_command(command: &str, color: bool) -> String {
    paint_spans(&command_spans(command), color)
}

#[cfg(test)]
mod tests {
    use super::{Span, command_spans, paint_command, paint_spans};
    use crate::theme::Role;

    #[test]
    fn commands_tokenize_like_code() {
        assert_eq!(
            command_spans("skilld install skilld:owner/repo/x --agent codex"),
            vec![
                Span::Styled("skilld".to_owned(), Role::Brand),
                Span::Styled("install".to_owned(), Role::Emphasis),
                Span::Text("skilld:owner/repo/x".to_owned()),
                Span::Styled("--agent".to_owned(), Role::Dim),
                Span::Text("codex".to_owned()),
            ]
        );
    }

    #[test]
    fn update_commands_highlight_their_subcommand() {
        assert_eq!(
            command_spans("skilld update nuxt-seo --global")[1],
            Span::Styled("update".to_owned(), Role::Emphasis)
        );
    }

    #[test]
    fn plain_commands_render_verbatim() {
        assert_eq!(
            paint_command("skilld install foo --agent codex", false),
            "skilld install foo --agent codex"
        );
    }

    #[test]
    fn colored_commands_carry_brand_and_dim() {
        let colored = paint_command("skilld install foo --agent codex", true);
        assert!(colored.starts_with("\u{1b}[1m\u{1b}[36mskilld\u{1b}[0m"));
        assert!(colored.contains("\u{1b}[1minstall\u{1b}[0m"));
        assert!(colored.contains("\u{1b}[2m--agent\u{1b}[0m"));
        assert!(colored.ends_with("codex"));
    }

    #[test]
    fn empty_input_renders_empty() {
        assert_eq!(paint_spans(&command_spans(""), false), "");
    }
}
