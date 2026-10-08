use crate::update_ui::{InteractiveUpdateError, NativeTerminalLifecycle, with_restored_terminal};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use skilld_command::doctor::{DoctorOptions, DoctorOwner, DoctorReport};
use skilld_command::doctor_actions::{ActionPreview, DoctorAction, DoctorApplied, DoctorPlan};
use skilld_command::{CommandError, LocalHost};
use skilld_ui::text::sanitize;
use std::io;
use std::sync::{Arc, mpsc};
use std::time::Duration;

pub struct Model {
    pub report: Option<DoctorReport>,
    pub selected: usize,
    pub skills_sh_only: bool,
    pub preview: Option<ActionPreview>,
    pub message: String,
    pub busy: bool,
    pub detail_scroll: u16,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            report: None,
            selected: 0,
            skills_sh_only: false,
            preview: None,
            message: "Scanning Skill files. Press q to cancel.".into(),
            busy: true,
            detail_scroll: 0,
        }
    }
}

impl Model {
    pub fn visible(&self) -> Vec<usize> {
        self.report
            .as_ref()
            .map(|r| {
                r.skills
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        !self.skills_sh_only || matches!(s.owner, DoctorOwner::SkillsSh { .. })
                    })
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn move_selection(&mut self, down: bool) {
        if self.preview.is_some() {
            self.detail_scroll = if down {
                self.detail_scroll.saturating_add(1)
            } else {
                self.detail_scroll.saturating_sub(1)
            };
            return;
        }
        let len = self.visible().len();
        if len > 0 {
            self.selected = if down {
                (self.selected + 1) % len
            } else {
                self.selected.checked_sub(1).unwrap_or(len - 1)
            };
        }
        self.detail_scroll = 0;
    }
    pub fn toggle_group(&mut self) {
        if !self.busy && self.preview.is_none() {
            self.skills_sh_only = !self.skills_sh_only;
            self.selected = 0;
            self.detail_scroll = 0;
        }
    }
}

pub fn view(frame: &mut ratatui::Frame<'_>, model: &Model) {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(3),
    ])
    .split(frame.area());
    let counts = model
        .report
        .as_ref()
        .map(|r| {
            format!(
                "{} Skills · {} directories scanned · {} excluded · {} problems",
                r.skills.len(),
                r.visited_directories,
                r.skipped_directories,
                r.problems.len()
            )
        })
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(format!("skilld doctor  |  {counts}")),
        rows[0],
    );
    let cols =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(rows[1]);
    let visible = model.visible();
    let items = visible
        .iter()
        .map(|&i| {
            let s = &model.report.as_ref().expect("visible report").skills[i];
            ListItem::new(format!(
                "{}  {}{}",
                s.owner.label(),
                sanitize(&s.name),
                if s.duplicates.is_empty() { "" } else { "  =" }
            ))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(if visible.is_empty() {
        None
    } else {
        Some(model.selected.min(visible.len() - 1))
    });
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(if model.skills_sh_only {
                        "skills.sh installs"
                    } else {
                        "All Skills"
                    }),
            )
            .highlight_symbol("> ")
            .highlight_style(Style::default().add_modifier(Modifier::BOLD)),
        cols[0],
        &mut state,
    );
    let (title, details) = if let Some(p) = &model.preview {
        let mut text = format!("{}: {}\n\n", p.action, p.name);
        if let Some(source) = &p.source {
            text.push_str(&format!(
                "Source: {source}\nCommit: {}\n",
                p.commit.as_deref().unwrap_or("unknown")
            ));
        }
        if p.changes_content {
            text.push_str(
                "\nThis migration replaces contents with the source commit shown above.\n",
            );
        }
        if !p.behaviors.is_empty() {
            text.push_str("\nSkill behaviors needing approval:\n");
            for behavior in &p.behaviors {
                text.push_str(&format!("{behavior}\n"));
            }
            text.push_str("Enter approves these behaviors and applies the action.\n");
        }
        text.push_str("\nAffected Agent targets:\n");
        for path in &p.paths {
            text.push_str(&format!("{}\n", path.display()));
        }
        text.push_str("\nOriginal files and lock metadata will be backed up.\nEnter applies this action. Esc cancels.\nUp/down scroll all affected paths.");
        ("Review action", text)
    } else if let Some(&i) = visible.get(model.selected) {
        let s = &model.report.as_ref().expect("visible report").skills[i];
        let mut text = format!(
            "{}\nOwner: {}\n{}\n",
            s.name,
            s.owner.label(),
            s.canonical_path.display()
        );
        if let DoctorOwner::SkillsSh { record } = &s.owner {
            text.push_str(&format!(
                "\nSource: {}\nLockfile: {}\n",
                record.source,
                record.lockfile.display()
            ));
        }
        if let Some(f) = &s.fingerprint {
            text.push_str(&format!(
                "\n{} files · {} bytes\nGit tree: {}\n",
                f.files, f.bytes, f.git_tree
            ));
        }
        text.push_str("\nAgent paths:\n");
        for p in &s.paths {
            if p.agent.is_some() {
                text.push_str(&format!("{}\n", p.path.display()));
            }
        }
        if !s.duplicates.is_empty() {
            text.push_str(&format!(
                "\n{} identical directory copies.\n",
                s.duplicates.len()
            ));
        }
        if let Some(m) = &s.source_match {
            text.push_str(&format!(
                "\n{}\nCommit: {}\n{}\n",
                m.source,
                m.commit.as_deref().unwrap_or("unknown"),
                m.result
            ));
        }
        text.push_str("\nMigrate to skilld or remove from the skills.sh group.\nSource and plugin files remain owned by their source.");
        ("Skill details", text)
    } else {
        ("Skill details", "No Skills in this group.".into())
    };
    frame.render_widget(
        Paragraph::new(details.lines().map(sanitize).collect::<Vec<_>>().join("\n"))
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false })
            .scroll((model.detail_scroll, 0)),
        cols[1],
    );
    let help = if model.busy {
        "Working…"
    } else if model.preview.is_some() {
        "enter apply  esc cancel  ↑/↓ scroll"
    } else {
        "↑/↓ move  tab skills.sh/all  m migrate  d remove  r rescan  q quit"
    };
    frame.render_widget(
        Paragraph::new(format!("{}\n{}", sanitize(&model.message), help))
            .wrap(Wrap { trim: false }),
        rows[2],
    );
}

pub fn render_snapshot(model: &Model, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(|f| view(f, model)).expect("render");
    let buffer = terminal.backend().buffer();
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

enum Job {
    Scan(Result<DoctorReport, CommandError>),
    Plan(Result<Box<DoctorPlan>, CommandError>),
    Applied(Result<DoctorApplied, CommandError>),
}

pub fn run_doctor(
    host: Arc<LocalHost>,
    options: DoctorOptions,
) -> Result<(), InteractiveUpdateError> {
    with_restored_terminal(NativeTerminalLifecycle, || {
        let mut terminal =
            ratatui::Terminal::new(CrosstermBackend::new(io::stdout())).map_err(terminal_error)?;
        let (tx, rx) = mpsc::channel();
        start_scan(host.clone(), options.clone(), tx.clone());
        let mut model = Model::default();
        let mut plan = None;
        let mut applying = false;
        let mut completed_message = None;
        loop {
            while let Ok(job) = rx.try_recv() {
                model.busy = false;
                match job {
                    Job::Scan(result) => match result {
                        Ok(report) => {
                            model.message = if let Some(message) = completed_message.take() {
                                message
                            } else if report.problems.is_empty() {
                                "Choose a Skill to review.".into()
                            } else {
                                format!(
                                    "{} scan problems. Use --json for their paths and reasons.",
                                    report.problems.len()
                                )
                            };
                            model.report = Some(report);
                            model.selected = 0;
                        }
                        Err(e) => model.message = e.to_string(),
                    },
                    Job::Plan(result) => match result {
                        Ok(p) => {
                            model.preview = Some(p.preview.clone());
                            model.detail_scroll = 0;
                            model.message = "Review every affected target before applying.".into();
                            plan = Some(p);
                        }
                        Err(e) => model.message = e.to_string(),
                    },
                    Job::Applied(result) => {
                        applying = false;
                        model.preview = None;
                        match result {
                            Ok(done) => {
                                model.message =
                                    format!("{}. Backup: {}", done.message, done.backup.display());
                                completed_message = Some(model.message.clone());
                                model.busy = true;
                                start_scan(host.clone(), options.clone(), tx.clone());
                            }
                            Err(e) => model.message = e.to_string(),
                        }
                    }
                }
            }
            terminal.draw(|f| view(f, &model)).map_err(terminal_error)?;
            if !event::poll(Duration::from_millis(100)).map_err(terminal_error)? {
                continue;
            }
            let Event::Key(key) = event::read().map_err(terminal_error)? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if (key.code == KeyCode::Char('q')
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(event::KeyModifiers::CONTROL)))
                && !applying
            {
                return Ok(());
            }
            if model.busy {
                continue;
            }
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => model.move_selection(true),
                KeyCode::Up | KeyCode::Char('k') => model.move_selection(false),
                KeyCode::PageDown => model.detail_scroll = model.detail_scroll.saturating_add(10),
                KeyCode::PageUp => model.detail_scroll = model.detail_scroll.saturating_sub(10),
                KeyCode::Tab => model.toggle_group(),
                KeyCode::Esc => {
                    plan = None;
                    model.preview = None;
                    model.detail_scroll = 0;
                }
                KeyCode::Enter => {
                    if let Some(p) = plan.take() {
                        applying = true;
                        model.busy = true;
                        model.message = "Applying reviewed action. Keep this terminal open.".into();
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(Job::Applied(p.apply()));
                        });
                    }
                }
                KeyCode::Char('r') if model.preview.is_none() => {
                    model.busy = true;
                    start_scan(host.clone(), options.clone(), tx.clone());
                }
                KeyCode::Char('m' | 'd') if model.preview.is_none() => {
                    if let (Some(report), Some(&index)) =
                        (model.report.clone(), model.visible().get(model.selected))
                    {
                        let action = if key.code == KeyCode::Char('m') {
                            DoctorAction::Migrate
                        } else {
                            DoctorAction::Remove
                        };
                        model.busy = true;
                        model.message = "Preparing action and checking source contents.".into();
                        let host = host.clone();
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(Job::Plan(
                                host.doctor_plan(&report, index, action).map(Box::new),
                            ));
                        });
                    }
                }
                _ => {}
            }
        }
    })
}

fn start_scan(host: Arc<LocalHost>, options: DoctorOptions, tx: mpsc::Sender<Job>) {
    std::thread::spawn(move || {
        let _ = tx.send(Job::Scan(host.doctor_scan(&options)));
    });
}
fn terminal_error(error: io::Error) -> InteractiveUpdateError {
    InteractiveUpdateError::new("TERMINAL_UNAVAILABLE", error.to_string())
}
