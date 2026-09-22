//! The Skill picker `skilld add` shows for a multi-skill ref.
//!
//! A ref such as `OWNER/REPOSITORY` names many Skills. A terminal asks which
//! of them to install. The model is pure: it takes one key and returns the
//! next model. The event loop and the terminal live at the edge.

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use skilld_command::{CommandError, SkillChooser};
use skilld_core::{ListedSkill, SkillListing};
use skilld_ui::text::sanitize;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    /// Choose or free every Skill the filter shows.
    ToggleAll,
    Confirm,
    Cancel,
    /// Start typing a filter.
    FilterStart,
    /// One character typed into the filter.
    FilterChar(char),
    /// Erase the last filter character.
    FilterBackspace,
    /// Leave the filter, keeping what it shows.
    FilterDone,
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
    reference: String,
    choices: Vec<SkillChoice>,
    /// The cursor position inside the filtered list, not the whole list.
    cursor: usize,
    filter: String,
    /// Whether typing goes to the filter or to the list.
    filtering: bool,
    outcome: Option<PickerOutcome>,
}

impl PickerModel {
    #[must_use]
    pub fn new(reference: impl Into<String>, choices: Vec<SkillChoice>) -> Self {
        Self {
            reference: reference.into(),
            choices,
            cursor: 0,
            filter: String::new(),
            filtering: false,
            outcome: None,
        }
    }

    /// The positions the filter shows, in list order.
    ///
    /// An empty filter shows every Skill. A filter matches the name or the
    /// description, ignoring case, so "form" finds both `vee-validate` and
    /// `formkit`.
    #[must_use]
    pub fn visible(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.choices.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        self.choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| {
                choice.label.to_lowercase().contains(&needle)
                    || choice
                        .description
                        .as_deref()
                        .is_some_and(|value| value.to_lowercase().contains(&needle))
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// What the person has typed into the filter.
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Whether the next character typed goes to the filter.
    #[must_use]
    pub const fn filtering(&self) -> bool {
        self.filtering
    }

    /// The Skills chosen so far, and how many the ref names.
    ///
    /// The chosen count covers every Skill, including ones the filter hides,
    /// because a Skill stays chosen while you look for the next one.
    #[must_use]
    pub fn counts(&self) -> (usize, usize) {
        (self.chosen().len(), self.choices.len())
    }

    /// The ref the picker is choosing from.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Apply one key.
    pub fn update(&mut self, key: PickerKey) {
        if self.choices.is_empty() {
            self.outcome = Some(PickerOutcome::Cancelled);
            return;
        }
        match key {
            PickerKey::Up => self.move_cursor(-1),
            PickerKey::Down => self.move_cursor(1),
            PickerKey::Toggle => {
                if let Some(position) = self.visible().get(self.cursor).copied() {
                    self.choices[position].selected = !self.choices[position].selected;
                }
            }
            PickerKey::ToggleAll => {
                // Filter, then press a: the pair is how you take a family of
                // Skills without walking the whole list.
                let visible = self.visible();
                let select = !visible
                    .iter()
                    .all(|position| self.choices[*position].selected);
                for position in visible {
                    self.choices[position].selected = select;
                }
            }
            PickerKey::Confirm => self.outcome = Some(PickerOutcome::Chose(self.chosen())),
            PickerKey::Cancel => {
                // Escape backs out of the filter first. It cancels the picker
                // only when there is nothing to back out of.
                if self.filtering || !self.filter.is_empty() {
                    self.filtering = false;
                    self.filter.clear();
                    self.clamp_cursor();
                } else {
                    self.outcome = Some(PickerOutcome::Cancelled);
                }
            }
            PickerKey::FilterStart => self.filtering = true,
            PickerKey::FilterChar(character) => {
                if self.filter.chars().count() < 64 {
                    self.filter.push(character);
                    self.cursor = 0;
                }
            }
            PickerKey::FilterBackspace => {
                self.filter.pop();
                self.cursor = 0;
            }
            PickerKey::FilterDone => self.filtering = false,
        }
    }

    /// Move the cursor inside the filtered list, wrapping at both ends.
    fn move_cursor(&mut self, step: isize) {
        let length = self.visible().len();
        if length == 0 {
            self.cursor = 0;
            return;
        }
        let position = self.cursor as isize + step;
        self.cursor = position.rem_euclid(length as isize) as usize;
    }

    /// Keep the cursor inside the filtered list after the filter changes.
    fn clamp_cursor(&mut self) {
        let length = self.visible().len();
        if self.cursor >= length {
            self.cursor = length.saturating_sub(1);
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

    /// The header line the picker prints.
    #[must_use]
    pub fn header(&self) -> String {
        let (chosen, total) = self.counts();
        format!("{} names {total} Skills. {chosen} chosen.", self.reference)
    }
}

/// Ask which Skills of one listing to install, on this terminal.
pub fn run_skill_picker(
    mut model: PickerModel,
    color: bool,
) -> Result<PickerOutcome, InteractiveUpdateError> {
    with_restored_terminal(NativeTerminalLifecycle, || {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).map_err(terminal_lost)?;
        loop {
            terminal
                .draw(|frame| view(frame, &model, color))
                .map_err(terminal_lost)?;
            if let Some(outcome) = model.outcome() {
                return Ok(outcome.clone());
            }
            if event::poll(Duration::from_millis(120)).map_err(terminal_lost)?
                && let Event::Key(event) = event::read().map_err(terminal_lost)?
                && let Some(key) = picker_key(event, model.filtering())
            {
                model.update(key);
            }
        }
    })
}

/// The chosen mark, the cursor, and the width the name column takes.
const MARK_CHOSEN: &str = "\u{25cf}";
const MARK_FREE: &str = "\u{25cb}";
const CURSOR: &str = "\u{276f} ";

fn view(frame: &mut ratatui::Frame<'_>, model: &PickerModel, color: bool) {
    let areas = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(frame.area());
    frame.render_widget(header(model, color), areas[0]);
    let width = areas[1].width as usize;
    let visible = model.visible();
    if visible.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "No Skill matches ",
                    Style::default().fg(theme(color, Color::DarkGray)),
                ),
                Span::styled(
                    sanitize(model.filter()),
                    Style::default().fg(theme(color, Color::Yellow)),
                ),
            ])),
            areas[1],
        );
        frame.render_widget(footer(model, color), areas[2]);
        return;
    }
    // The name column is as wide as the longest name, so every description
    // starts in the same place and the eye reads one column, not a ragged edge.
    let name_width = visible
        .iter()
        .map(|position| UnicodeWidthStr::width(model.choices()[*position].label.as_str()))
        .max()
        .unwrap_or(0);
    let items = visible
        .iter()
        .map(|position| row(&model.choices()[*position], name_width, width, color))
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(model.cursor()));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol(CURSOR)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD)),
        areas[1],
        &mut state,
    );
    frame.render_widget(footer(model, color), areas[2]);
}

/// The ref, the counts, and one blank line.
fn header(model: &PickerModel, color: bool) -> Paragraph<'static> {
    let (chosen, total) = model.counts();
    let chosen_style = if chosen == 0 {
        Style::default().fg(theme(color, Color::DarkGray))
    } else {
        Style::default()
            .fg(theme(color, Color::Green))
            .add_modifier(Modifier::BOLD)
    };
    let shown = model.visible().len();
    let mut counts = vec![
        Span::styled(
            sanitize(model.reference()),
            Style::default()
                .fg(theme(color, Color::Cyan))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" names {total} Skills. "),
            Style::default().fg(theme(color, Color::Reset)),
        ),
        Span::styled(format!("{chosen} chosen"), chosen_style),
    ];
    if shown != total {
        counts.push(Span::styled(
            format!(", {shown} shown"),
            Style::default().fg(theme(color, Color::DarkGray)),
        ));
    }
    Paragraph::new(vec![Line::from(counts), filter_line(model, color)])
}

/// The filter line. It carries a cursor while the person is typing.
fn filter_line(model: &PickerModel, color: bool) -> Line<'static> {
    if !model.filtering() && model.filter().is_empty() {
        return Line::from(String::new());
    }
    let mut spans = vec![
        Span::styled(
            "filter ",
            Style::default().fg(theme(color, Color::DarkGray)),
        ),
        Span::styled(
            sanitize(model.filter()),
            Style::default()
                .fg(theme(color, Color::Yellow))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if model.filtering() {
        spans.push(Span::styled(
            "\u{2588}",
            Style::default().fg(theme(color, Color::Yellow)),
        ));
    }
    Line::from(spans)
}

/// One Skill row: the mark, the name, then as much description as fits.
fn row(choice: &SkillChoice, name_width: usize, width: usize, color: bool) -> ListItem<'static> {
    let mark = if choice.selected {
        Span::styled(
            MARK_CHOSEN,
            Style::default()
                .fg(theme(color, Color::Green))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            MARK_FREE,
            Style::default().fg(theme(color, Color::DarkGray)),
        )
    };
    let name = sanitize(&choice.label);
    let name_style = if choice.selected {
        Style::default().fg(theme(color, Color::Reset))
    } else {
        Style::default().fg(theme(color, Color::Gray))
    };
    let padding = name_width.saturating_sub(UnicodeWidthStr::width(name.as_str()));
    let mut spans = vec![
        mark,
        Span::raw(" "),
        Span::styled(name.clone(), name_style),
        Span::raw(" ".repeat(padding + 2)),
    ];
    // The cursor, the mark, the name and the gap are already spent. A
    // description that does not fit is cut, never wrapped, so one Skill stays
    // on one line however narrow the terminal is.
    let spent =
        UnicodeWidthStr::width(CURSOR) + UnicodeWidthStr::width(MARK_CHOSEN) + 1 + name_width + 2;
    if let Some(description) = &choice.description {
        let room = width.saturating_sub(spent);
        if room > 1 {
            spans.push(Span::styled(
                truncate(&clean(description), room),
                Style::default().fg(theme(color, Color::DarkGray)),
            ));
        }
    }
    ListItem::new(Line::from(spans))
}

/// The key hints. Each key is lit, each verb stays dim.
///
/// Typing a filter takes the letter keys, so the hints say what is left.
fn footer(model: &PickerModel, color: bool) -> Paragraph<'static> {
    let key = Style::default()
        .fg(theme(color, Color::Cyan))
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(theme(color, Color::DarkGray));
    let hints = if model.filtering() {
        vec![
            ("type", " to filter   "),
            ("enter", " keep it   "),
            ("esc", " clear"),
        ]
    } else if model.filter().is_empty() {
        vec![
            ("space", " choose   "),
            ("a", " all   "),
            ("/", " filter   "),
            ("enter", " install   "),
            ("esc", " cancel"),
        ]
    } else {
        vec![
            ("space", " choose   "),
            ("a", " all shown   "),
            ("/", " edit filter   "),
            ("enter", " install   "),
            ("esc", " clear filter"),
        ]
    };
    let mut spans = Vec::with_capacity(hints.len() * 2);
    for (name, verb) in hints {
        spans.push(Span::styled(name, key));
        spans.push(Span::styled(verb, text));
    }
    Paragraph::new(vec![Line::from(String::new()), Line::from(spans)])
}

/// Colors are off under NO_COLOR, so every style falls back to the default.
const fn theme(color: bool, value: Color) -> Color {
    if color { value } else { Color::Reset }
}

/// One line of text, with runs of whitespace collapsed.
fn clean(value: &str) -> String {
    sanitize(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Cut `value` to `width` columns. A cut description ends in an ellipsis.
fn truncate(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    let mut used = 0;
    let mut output = String::new();
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > width.saturating_sub(1) {
            break;
        }
        output.push(character);
        used += character_width;
    }
    output.push('\u{2026}');
    output
}

/// Draw the picker into an in-memory terminal, for tests and review.
#[must_use]
pub fn render_snapshot(model: &PickerModel, width: u16, height: u16, color: bool) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("the in-memory terminal is valid");
    terminal
        .draw(|frame| view(frame, model, color))
        .expect("the in-memory terminal can draw");
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::with_capacity(usize::from(height));
    for y in 0..height {
        let mut line = String::new();
        let mut x = 0;
        while x < width {
            if let Some(cell) = buffer.cell((x, y)) {
                line.push_str(cell.symbol());
                x = x.saturating_add(UnicodeWidthStr::width(cell.symbol()).max(1) as u16);
            } else {
                x = x.saturating_add(1);
            }
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

/// Read one key. What it means depends on whether a filter is being typed.
fn picker_key(event: KeyEvent, filtering: bool) -> Option<PickerKey> {
    if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('c') {
        return Some(PickerKey::Cancel);
    }
    if filtering {
        return match event.code {
            KeyCode::Up => Some(PickerKey::Up),
            KeyCode::Down => Some(PickerKey::Down),
            KeyCode::Backspace => Some(PickerKey::FilterBackspace),
            // Enter keeps the filter and hands the letter keys back, so the
            // next space chooses rather than typing a space.
            KeyCode::Enter => Some(PickerKey::FilterDone),
            KeyCode::Esc => Some(PickerKey::Cancel),
            KeyCode::Char(character) if !character.is_control() => {
                Some(PickerKey::FilterChar(character))
            }
            _ => None,
        };
    }
    match event.code {
        KeyCode::Up | KeyCode::Char('k') => Some(PickerKey::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(PickerKey::Down),
        KeyCode::Char(' ') => Some(PickerKey::Toggle),
        KeyCode::Char('a') => Some(PickerKey::ToggleAll),
        KeyCode::Char('/') => Some(PickerKey::FilterStart),
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

/// Build one picker row per listed Skill, with nothing chosen.
///
/// Installing every Skill of a Repository is the rarer intent, and `--all`
/// already says it. An empty list also makes the count in the header mean
/// something as the person works down the list.
#[must_use]
pub fn choices_for(listing: &SkillListing) -> Vec<SkillChoice> {
    listing
        .items
        .iter()
        .map(|item| SkillChoice {
            label: item.name.clone(),
            description: item.description.clone(),
            selected: false,
        })
        .collect()
}

/// The chooser that asks on this terminal.
pub struct TtySkillChooser {
    color: bool,
}

impl TtySkillChooser {
    #[must_use]
    pub const fn new(color: bool) -> Self {
        Self { color }
    }
}

impl SkillChooser for TtySkillChooser {
    fn choose(&self, listing: &SkillListing) -> Result<Vec<ListedSkill>, CommandError> {
        if listing.items.len() < 2 {
            return Ok(listing.items.clone());
        }
        let model = PickerModel::new(listing.reference.to_string(), choices_for(listing));
        let outcome = run_skill_picker(model, self.color).map_err(|error| {
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
