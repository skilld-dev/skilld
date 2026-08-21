//! Shared terminal presentation primitives for the skilld CLI.
//!
//! Every command output is a [`Screen`] of semantic [`Line`]s. Plain mode
//! flattens to machine records, Human mode renders the same data with the
//! skilld theme.

pub mod screen;
pub mod spinner;
pub mod text;
pub mod theme;
pub mod time;

pub use screen::{Line, LineKind, Marker, Screen, plain_lines};
pub use theme::{RESET, Role, paint};
pub use time::relative_time;

#[cfg(test)]
mod tests {
    use super::{Line, Role, Screen, paint};

    #[test]
    fn paint_matches_legacy_escape_sequences() {
        assert_eq!(
            paint("x", Role::Brand, true),
            "\u{1b}[1m\u{1b}[36mx\u{1b}[0m"
        );
        assert_eq!(paint("x", Role::Emphasis, true), "\u{1b}[1mx\u{1b}[0m");
        assert_eq!(paint("x", Role::Warn, true), "\u{1b}[33mx\u{1b}[0m");
        assert_eq!(paint("x", Role::Dim, true), "\u{1b}[2mx\u{1b}[0m");
        assert_eq!(paint("x", Role::Brand, false), "x");
    }

    #[test]
    fn plain_render_keeps_the_exact_machine_text() {
        let screen = Screen::new(vec![
            Line::success("Installed Skill grill-me."),
            Line::hint("Review the unverified Skill before use."),
            Line::field("Name", "grill-me"),
        ]);

        assert_eq!(
            screen.render_plain(),
            concat!(
                "Installed Skill grill-me.\n",
                "Review the unverified Skill before use.\n",
                "Name: grill-me\n"
            )
        );
    }

    #[test]
    fn human_render_adds_glyphs_and_styles() {
        let screen = Screen::new(vec![
            Line::success("Installed Skill grill-me."),
            Line::warn("Outdated Skill grill-me."),
            Line::error("Unverified Skill grill-me."),
            Line::field_plain("agent.targets=claude-code", "agent.targets", "claude-code"),
        ]);

        assert_eq!(
            screen.render_human(true),
            concat!(
                "\u{1b}[32m✓\u{1b}[0m Installed Skill grill-me.\n",
                "\u{1b}[33m⚠\u{1b}[0m Outdated Skill grill-me.\n",
                "\u{1b}[31m✗\u{1b}[0m Unverified Skill grill-me.\n",
                "\u{1b}[2magent.targets\u{1b}[0m: claude-code\n"
            )
        );
    }

    #[test]
    fn human_without_color_keeps_glyphs_and_drops_escapes() {
        let screen = Screen::new(vec![Line::success("Installed Skill grill-me.")]);

        assert_eq!(screen.render_human(false), "✓ Installed Skill grill-me.\n");
    }

    #[test]
    fn fields_align_labels_in_human_mode() {
        let screen = Screen::new(vec![
            Line::field("Name", "grill-me"),
            Line::field("Source status", "verified"),
        ]);

        assert_eq!(
            screen.render_human(false),
            concat!("Name         : grill-me\n", "Source status: verified\n")
        );
    }
}
