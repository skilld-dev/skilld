//! The Skill picker `skilld add` shows for a multi-skill ref.
//!
//! A ref such as `OWNER/REPOSITORY` names many Skills. A terminal asks which
//! of them to install. The model is pure: it takes one key and returns the
//! next model. The event loop and the terminal live at the edge.

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use skilld_command::{CommandError, SkillChooser};
use skilld_core::{ListedSkill, SkillListing};
use skilld_ui::text::sanitize;

use crate::update_ui::{InteractiveUpdateError, NativeTerminalLifecycle, with_restored_terminal};

use std::time::Duration;

/// One row of the picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillChoice {
    pub label: String,
    pub description: Option<String>,
    pub selected: bool,
}

/// The keys the picker answers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerKey {
    Up,
    Down,
    Toggle,
    ToggleAll,
    Confirm,
    Cancel,
}

/// What the picker returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PickerOutcome {
    /// The person confirmed these row positions.
    Chose(Vec<usize>),
    /// The person cancelled. skilld installs nothing.
    Cancelled,
}

/// The picker state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerModel {
    title: String,
    choices: Vec<SkillChoice>,
    cursor: usize,
    outcome: Option<PickerOutcome>,
}

impl PickerModel {
    #[must_use]
    pub fn new(title: impl Into<String>, choices: Vec<SkillChoice>) -> Self {
        Self {
            title: title.into(),
            choices,
            cursor: 0,
            outcome: None,
        }
    }

    /// Apply one key.
    pub fn update(&mut self, key: PickerKey) {
        if self.choices.is_empty() {
            self.outcome = Some(PickerOutcome::Cancelled);
            return;
        }
        match key {
            PickerKey::Up => {
                self.cursor = if self.cursor == 0 {
                    self.choices.len() - 1
                } else {
                    self.cursor - 1
                };
            }
            PickerKey::Down => self.cursor = (self.cursor + 1) % self.choices.len(),
            PickerKey::Toggle => {
                let choice = &mut self.choices[self.cursor];
                choice.selected = !choice.selected;
            }
            PickerKey::ToggleAll => {
                let select = !self.choices.iter().all(|choice| choice.selected);
                for choice in &mut self.choices {
                    choice.selected = select;
                }
            }
            PickerKey::Confirm => self.outcome = Some(PickerOutcome::Chose(self.chosen())),
            PickerKey::Cancel => self.outcome = Some(PickerOutcome::Cancelled),
        }
    }

    #[must_use]
    pub fn outcome(&self) -> Option<&PickerOutcome> {
        self.outcome.as_ref()
    }

    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn choices(&self) -> &[SkillChoice] {
        &self.choices
    }

    fn chosen(&self) -> Vec<usize> {
        self.choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| choice.selected)
            .map(|(index, _)| index)
            .collect()
    }

    /// The header line, with the count already chosen.
    #[must_use]
    pub fn header(&self) -> String {
        format!(
            "{} Chosen {} of {}.",
            self.title,
            self.chosen().len(),
            self.choices.len()
        )
    }
}

const FOOTER: &str = "Space toggles. a toggles all. Enter installs. Esc cancels.";

/// Ask which Skills of one listing to install, on this terminal.
pub fn run_skill_picker(mut model: PickerModel) -> Result<PickerOutcome, InteractiveUpdateError> {
    with_restored_terminal(NativeTerminalLifecycle, || {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).map_err(terminal_lost)?;
        loop {
            terminal
                .draw(|frame| view(frame, &model))
                .map_err(terminal_lost)?;
            if let Some(outcome) = model.outcome() {
                return Ok(outcome.clone());
            }
            if event::poll(Duration::from_millis(120)).map_err(terminal_lost)?
                && let Event::Key(event) = event::read().map_err(terminal_lost)?
                && let Some(key) = picker_key(event)
            {
                model.update(key);
            }
        }
    })
}

fn view(frame: &mut ratatui::Frame<'_>, model: &PickerModel) {
    let areas = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(frame.area());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(sanitize(&model.header())),
            Line::from(String::new()),
        ]),
        areas[0],
    );
    let items = model
        .choices()
        .iter()
        .map(|choice| {
            let mark = if choice.selected { "[x]" } else { "[ ]" };
            let description = choice
                .description
                .as_deref()
                .map(|value| format!("  {}", sanitize(value)))
                .unwrap_or_default();
            ListItem::new(format!("{mark} {}{description}", sanitize(&choice.label)))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(model.cursor()));
    frame.render_stateful_widget(
        List::new(items).highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        areas[1],
        &mut state,
    );
    frame.render_widget(
        Paragraph::new(Line::from(FOOTER)).style(Style::default().fg(Color::DarkGray)),
        areas[2],
    );
}

fn picker_key(event: KeyEvent) -> Option<PickerKey> {
    if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('c') {
        return Some(PickerKey::Cancel);
    }
    match event.code {
        KeyCode::Up | KeyCode::Char('k') => Some(PickerKey::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(PickerKey::Down),
        KeyCode::Char(' ') => Some(PickerKey::Toggle),
        KeyCode::Char('a') => Some(PickerKey::ToggleAll),
        KeyCode::Enter => Some(PickerKey::Confirm),
        KeyCode::Char('q') | KeyCode::Esc => Some(PickerKey::Cancel),
        _ => None,
    }
}

fn terminal_lost(_error: io::Error) -> InteractiveUpdateError {
    InteractiveUpdateError::new(
        "INTERACTIVE_TTY_LOST",
        "The interactive terminal stopped responding.",
    )
}

/// Build one picker row per listed Skill, every row chosen.
#[must_use]
pub fn choices_for(listing: &SkillListing) -> Vec<SkillChoice> {
    listing
        .items
        .iter()
        .map(|item| SkillChoice {
            label: item.name.clone(),
            description: item.description.clone(),
            selected: true,
        })
        .collect()
}

/// The chooser that asks on this terminal.
pub struct TtySkillChooser;

impl SkillChooser for TtySkillChooser {
    fn choose(&self, listing: &SkillListing) -> Result<Vec<ListedSkill>, CommandError> {
        if listing.items.len() < 2 {
            return Ok(listing.items.clone());
        }
        let title = format!(
            "{} names {} Skills. Choose which ones to install.",
            listing.reference,
            listing.items.len()
        );
        let model = PickerModel::new(title, choices_for(listing));
        let outcome = run_skill_picker(model).map_err(|error| {
            CommandError::operation("TERMINAL_UNAVAILABLE", error.message.clone())
        })?;
        match outcome {
            PickerOutcome::Chose(positions) => Ok(positions
                .into_iter()
                .filter_map(|index| listing.items.get(index).cloned())
                .collect()),
            PickerOutcome::Cancelled => Ok(Vec::new()),
        }
    }
}
