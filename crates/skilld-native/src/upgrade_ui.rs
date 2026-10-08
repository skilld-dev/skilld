//! Cached CLI upgrade prompt. Network checks never own the terminal.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::update_ui::{InteractiveUpdateError, NativeTerminalLifecycle, with_restored_terminal};

const ACCENT: Color = Color::Rgb(103, 232, 249);
const SUCCESS: Color = Color::Rgb(134, 239, 172);
const MUTED: Color = Color::Rgb(148, 163, 184);
const COMMAND: Color = Color::Rgb(253, 224, 71);
const SELECTED: Color = Color::Rgb(22, 78, 99);

fn tone(color: bool, foreground: Color) -> Style {
    if color {
        Style::default().fg(foreground)
    } else {
        Style::default()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpgradeChoice {
    Upgrade,
    Later,
    Dismiss,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpgradeKey {
    Up,
    Down,
    Confirm,
    Cancel,
}

#[derive(Default)]
pub struct UpgradePrompt {
    cursor: usize,
}

impl UpgradePrompt {
    pub fn update(&mut self, key: UpgradeKey) -> Option<UpgradeChoice> {
        match key {
            UpgradeKey::Up => self.cursor = (self.cursor + 2) % 3,
            UpgradeKey::Down => self.cursor = (self.cursor + 1) % 3,
            UpgradeKey::Cancel => return Some(UpgradeChoice::Later),
            UpgradeKey::Confirm => {
                return Some(
                    [
                        UpgradeChoice::Upgrade,
                        UpgradeChoice::Later,
                        UpgradeChoice::Dismiss,
                    ][self.cursor],
                );
            }
        }
        None
    }
}

pub fn banner(current: &str, latest: &str) -> String {
    format!(
        "Upgrade available: {current} -> {latest}\nhttps://github.com/skilld-dev/skilld/releases/latest"
    )
}

/// A static banner can be appended after the command without moving its cursor.
pub fn render_banner(
    current: &str,
    latest: &str,
    guidance: &str,
    width: u16,
    color: bool,
) -> String {
    let width = width.clamp(20, 90);
    let lines = format!("{}\n\n{guidance}", banner(current, latest))
        .lines()
        .enumerate()
        .flat_map(|(index, line)| {
            let style = tone(
                color,
                match index {
                    0 => SUCCESS,
                    1 => MUTED,
                    _ => ACCENT,
                },
            );
            skilld_ui::text::wrap(line, usize::from(width - 2))
                .into_iter()
                .map(move |line| Line::styled(line, style))
        })
        .collect::<Vec<_>>();
    let height = lines.len() as u16 + 2;
    let paragraph =
        Paragraph::new(lines).block(Block::bordered().border_style(tone(color, ACCENT)));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("in-memory terminal");
    terminal
        .draw(|frame| frame.render_widget(paragraph, frame.area()))
        .expect("render banner");
    snapshot_text(&terminal, color)
}

pub fn ask(
    current: &str,
    latest: &str,
    action: &str,
    color: bool,
) -> Result<UpgradeChoice, InteractiveUpdateError> {
    with_restored_terminal(NativeTerminalLifecycle, || {
        let mut terminal =
            Terminal::new(CrosstermBackend::new(io::stdout())).map_err(terminal_error)?;
        let mut model = UpgradePrompt::default();
        // An Enter already buffered for the original command is not upgrade consent.
        while event::poll(Duration::ZERO).map_err(terminal_error)? {
            let _ = event::read().map_err(terminal_error)?;
        }
        loop {
            terminal
                .draw(|frame| view(frame, &model, current, latest, action, color))
                .map_err(terminal_error)?;
            let Event::Key(key) = event::read().map_err(terminal_error)? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let key = match key.code {
                KeyCode::Up | KeyCode::Char('k') => UpgradeKey::Up,
                KeyCode::Down | KeyCode::Char('j') => UpgradeKey::Down,
                KeyCode::Enter => UpgradeKey::Confirm,
                KeyCode::Esc | KeyCode::Char('q') => UpgradeKey::Cancel,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    UpgradeKey::Cancel
                }
                _ => continue,
            };
            if let Some(choice) = model.update(key) {
                return Ok(choice);
            }
        }
    })
}

fn view(
    frame: &mut ratatui::Frame<'_>,
    model: &UpgradePrompt,
    current: &str,
    latest: &str,
    action: &str,
    color: bool,
) {
    let areas = Layout::vertical([
        Constraint::Length(7),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .split(frame.area());
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "Upgrade available: ",
                tone(color, ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(current, tone(color, MUTED)),
            Span::styled(" -> ", tone(color, MUTED)),
            Span::styled(latest, tone(color, SUCCESS).add_modifier(Modifier::BOLD)),
        ]),
        Line::styled(
            "https://github.com/skilld-dev/skilld/releases/latest",
            tone(color, MUTED),
        ),
        Line::from(""),
        Line::styled(action, tone(color, COMMAND)),
    ])
    .wrap(Wrap { trim: true })
    .block(
        Block::bordered()
            .border_style(tone(color, ACCENT))
            .title(Span::styled(
                " skilld CLI upgrade ",
                tone(color, ACCENT).add_modifier(Modifier::BOLD),
            )),
    );
    frame.render_widget(header, areas[0]);
    let options = List::new([
        ListItem::new("Upgrade now"),
        ListItem::new("Not now"),
        ListItem::new("Don't remind me for this version"),
    ])
    .highlight_symbol("> ")
    .highlight_style(if color {
        Style::default()
            .fg(ACCENT)
            .bg(SELECTED)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    });
    frame.render_stateful_widget(
        options,
        areas[1],
        &mut ListState::default().with_selected(Some(model.cursor)),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Up/Down", tone(color, ACCENT)),
                Span::styled(" to choose. ", tone(color, MUTED)),
                Span::styled("Enter", tone(color, ACCENT)),
                Span::styled(" to confirm. ", tone(color, MUTED)),
                Span::styled("Esc", tone(color, ACCENT)),
                Span::styled(" to continue.", tone(color, MUTED)),
            ]),
        ])
        .wrap(Wrap { trim: true }),
        areas[2],
    );
}

pub fn render_snapshot(
    current: &str,
    latest: &str,
    action: &str,
    width: u16,
    height: u16,
    color: bool,
) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("in-memory terminal");
    terminal
        .draw(|frame| {
            view(
                frame,
                &UpgradePrompt::default(),
                current,
                latest,
                action,
                color,
            )
        })
        .expect("render prompt");
    snapshot_text(&terminal, color)
}

fn snapshot_text(terminal: &Terminal<TestBackend>, color: bool) -> String {
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width;
    let height = buffer.area.height;
    (0..height)
        .map(|y| {
            if color {
                let mut line = String::new();
                let mut previous = None;
                for x in 0..width {
                    let cell = &buffer[(x, y)];
                    let style = cell.style();
                    if previous != Some(style) {
                        line.push_str("\x1b[0m");
                        line.push_str(&ansi_color(cell.fg, false));
                        line.push_str(&ansi_color(cell.bg, true));
                        if cell.modifier.contains(Modifier::BOLD) {
                            line.push_str("\x1b[1m");
                        }
                        previous = Some(style);
                    }
                    line.push_str(cell.symbol());
                }
                line.push_str("\x1b[0m");
                return line;
            }
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn ansi_color(color: Color, background: bool) -> String {
    let base = if background { 48 } else { 38 };
    let index = match color {
        Color::Reset => return format!("\x1b[{}m", if background { 49 } else { 39 }),
        Color::Rgb(red, green, blue) => return format!("\x1b[{base};2;{red};{green};{blue}m"),
        Color::Indexed(index) => index,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    format!("\x1b[{base};5;{index}m")
}

fn terminal_error(error: io::Error) -> InteractiveUpdateError {
    InteractiveUpdateError::new(
        "UPGRADE_TERMINAL_FAILED",
        format!("Cannot read the upgrade choice: {error}"),
    )
}
