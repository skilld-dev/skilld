//! Braille spinner frames shared by status lines and the TUI.

/// One full rotation of the skilld spinner.
pub const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The frame at `step` in the shared rotation.
pub const fn frame(step: usize) -> &'static str {
    FRAMES[step % FRAMES.len()]
}
