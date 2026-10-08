//! Cached CLI upgrade prompt. Network checks never own the terminal.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::update_ui::{InteractiveUpdateError, NativeTerminalLifecycle, with_restored_terminal};

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
pub fn render_banner(current: &str, latest: &str, guidance: &str, width: u16) -> String {
    let width = width.clamp(20, 90);
    let lines = format!("{}\n\n{guidance}", banner(current, latest))
        .lines()
        .flat_map(|line| skilld_ui::text::wrap(line, usize::from(width - 2)))
        .collect::<Vec<_>>();
    let height = lines.len() as u16 + 2;
    let paragraph = Paragraph::new(lines.join("\n")).block(Block::bordered());
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("in-memory terminal");
    terminal
        .draw(|frame| frame.render_widget(paragraph, frame.area()))
        .expect("render banner");
    snapshot_text(&terminal)
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
    let accent = if color { Color::Cyan } else { Color::Reset };
    let header = Paragraph::new(format!("{}\n\n{action}", banner(current, latest)))
        .wrap(Wrap { trim: true })
        .block(Block::bordered().title(" skilld CLI upgrade "));
    frame.render_widget(header, areas[0]);
    let options = List::new([
        ListItem::new("Upgrade now"),
        ListItem::new("Not now"),
        ListItem::new("Don't remind me for this version"),
    ])
    .highlight_symbol("> ")
    .highlight_style(Style::default().fg(accent).add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(
        options,
        areas[1],
        &mut ListState::default().with_selected(Some(model.cursor)),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from("Up/Down to choose. Enter to confirm. Esc to continue."),
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
                false,
            )
        })
        .expect("render prompt");
    snapshot_text(&terminal)
}

fn snapshot_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width;
    let height = buffer.area.height;
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn terminal_error(error: io::Error) -> InteractiveUpdateError {
    InteractiveUpdateError::new(
        "UPGRADE_TERMINAL_FAILED",
        format!("Cannot read the upgrade choice: {error}"),
    )
}
