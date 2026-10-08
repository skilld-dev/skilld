//! Terminal palette styles for screens that inherit the user's background.

use ratatui::style::{Color, Modifier, Style};

pub(crate) fn tone(color: bool, foreground: Color) -> Style {
    if !color {
        return Style::default();
    }
    match foreground {
        Color::Gray | Color::DarkGray => Style::default().add_modifier(Modifier::DIM),
        foreground => Style::default().fg(foreground),
    }
}

pub(crate) fn selection() -> Style {
    Style::default()
        .fg(Color::Reset)
        .bg(Color::Reset)
        .remove_modifier(Modifier::DIM)
        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
}
