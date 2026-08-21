//! Terminal text measurement and shaping, shared by every renderer.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Replace terminal control characters with spaces so untrusted text can
/// never move a cursor or forge an escape sequence.
pub fn sanitize(value: &str) -> String {
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

/// The display width of `value` in terminal cells.
pub fn width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

/// Pad `value` with spaces until it fills `columns` cells.
pub fn pad_to(value: &str, columns: usize) -> String {
    let used = width(value);
    if used >= columns {
        return value.to_owned();
    }
    format!("{value}{}", " ".repeat(columns - used))
}

/// Greedy word wrap that never splits below one column.
pub fn wrap(value: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in value.split_whitespace() {
        let separator = usize::from(!current.is_empty());
        if width(&current) + separator + width(word) > limit && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        if width(word) <= limit {
            current.push_str(word);
        } else {
            let mut chunk = String::new();
            for character in word.chars() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if !chunk.is_empty() && width(&chunk) + character_width > limit {
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

/// Truncate `value` to `limit` cells without an ellipsis.
pub fn truncate(value: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let mut used: usize = 0;
    let mut output = String::new();
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used.saturating_add(character_width) > limit {
            break;
        }
        output.push(character);
        used = used.saturating_add(character_width);
    }
    output
}

/// Truncate `value` to `limit` cells, marking the cut with an ellipsis.
pub fn truncate_ellipsis(value: &str, limit: usize) -> String {
    if width(value) <= limit {
        return value.to_owned();
    }
    let mut output = truncate(value, limit.saturating_sub(1));
    if limit > 1 {
        output.push('…');
    }
    output
}

/// Group digits with commas: `227068` becomes `227,068`.
pub fn grouped_number(value: u64) -> String {
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

/// The short form of a commit SHA, or the whole value when it is shorter.
pub fn short_sha(value: &str) -> &str {
    value.get(..7).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::{
        grouped_number, pad_to, sanitize, short_sha, truncate, truncate_ellipsis, width, wrap,
    };

    #[test]
    fn sanitize_replaces_control_characters() {
        assert_eq!(sanitize("a\u{1b}[31mb"), "a [31mb");
        assert_eq!(sanitize("a\tb"), "a b");
    }

    #[test]
    fn wrap_breaks_on_words_then_characters() {
        assert_eq!(wrap("aaa bbb", 5), vec!["aaa", "bbb"]);
        assert_eq!(wrap("aaaaaaaa", 3), vec!["aaa", "aaa", "aa"]);
        assert_eq!(wrap("", 5), vec![String::new()]);
    }

    #[test]
    fn truncate_respects_display_cells() {
        assert_eq!(truncate("漢字abc", 4), "漢字");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn truncate_ellipsis_marks_the_cut() {
        assert_eq!(truncate_ellipsis("abcdef", 4), "abc…");
        assert_eq!(truncate_ellipsis("ab", 4), "ab");
    }

    #[test]
    fn pad_to_fills_display_cells() {
        assert_eq!(pad_to("漢", 4), "漢  ");
        assert_eq!(pad_to("abc", 2), "abc");
    }

    #[test]
    fn numbers_group_by_thousands() {
        assert_eq!(grouped_number(227_068), "227,068");
        assert_eq!(grouped_number(999), "999");
    }

    #[test]
    fn shas_shorten_to_seven_characters() {
        assert_eq!(short_sha("0123456789abcdef"), "0123456");
        assert_eq!(short_sha("abc"), "abc");
        assert_eq!(width("漢"), 2);
    }
}
