//! The skilld color theme.
//!
//! Roles map to fixed escape sequences so every surface (screens, the TUI,
//! status lines) shares one look. When color is off, painting is a no-op.

/// The escape sequence that clears every style.
pub const RESET: &str = "\u{1b}[0m";

/// A semantic slot in the skilld theme.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// Bold cyan. The skilld brand: headings and highlights.
    Brand,
    /// Bold. Emphasis inside body text.
    Emphasis,
    /// Green. Completed work.
    Success,
    /// Yellow. Attention: outdated or degraded results.
    Warn,
    /// Red. Failures and required action.
    Error,
    /// Dim. Hints, footers, and labels.
    Dim,
}

impl Role {
    /// The escape prefix for this role.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Brand => "\u{1b}[1m\u{1b}[36m",
            Self::Emphasis => "\u{1b}[1m",
            Self::Success => "\u{1b}[32m",
            Self::Warn => "\u{1b}[33m",
            Self::Error => "\u{1b}[31m",
            Self::Dim => "\u{1b}[2m",
        }
    }
}

/// Paint `value` with `role` when `color` is enabled, otherwise return it
/// unchanged.
pub fn paint(value: &str, role: Role, color: bool) -> String {
    if color {
        format!("{}{value}{RESET}", role.prefix())
    } else {
        value.to_owned()
    }
}
