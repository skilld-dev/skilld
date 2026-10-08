use crate::terminal_theme::{selection, tone};
use crate::update_ui::{InteractiveUpdateError, NativeTerminalLifecycle, with_restored_terminal};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use skilld_command::doctor::{
    DoctorOptions, DoctorOwner, DoctorReport, DoctorSkill, ScanPhase, ScanProgress,
};
use skilld_command::doctor_actions::{ActionPreview, DoctorAction, DoctorApplied, DoctorPlan};
use skilld_command::{CommandError, LocalHost};
use skilld_ui::text::sanitize;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::Reset;
fn in_group(skill: &DoctorSkill, owner: &str) -> bool {
    if owner == "Duplicates" {
        !skill.duplicates.is_empty()
    } else {
        skill.owner.label() == owner
    }
}

fn skill_count(count: usize) -> String {
    format!("{count} {}", if count == 1 { "Skill" } else { "Skills" })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkState {
    Scanning,
    Ready,
    Preparing,
    Applying,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Skills,
    Problems,
    Help,
    Notice,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Browse {
    #[default]
    Owners,
    Locations {
        owner: &'static str,
    },
    Skills {
        owner: &'static str,
        location: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathFilter {
    #[default]
    All,
    Symlinks,
    Directories,
}

impl PathFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All paths",
            Self::Symlinks => "Symlinks",
            Self::Directories => "Directories",
        }
    }
}

pub enum Entry {
    Group {
        label: String,
        skills: Vec<usize>,
        next: Browse,
    },
    Skill(usize),
}

fn locations(skill: &DoctorSkill, filter: PathFilter) -> BTreeSet<Option<PathBuf>> {
    skill
        .paths
        .iter()
        .filter(|p| match filter {
            PathFilter::All => true,
            PathFilter::Symlinks => p.symlink_target.is_some(),
            PathFilter::Directories => p.symlink_target.is_none(),
        })
        .map(|p| {
            p.project_root.clone().or_else(|| {
                if p.agent.is_some() {
                    None
                } else {
                    p.path.parent().map(PathBuf::from)
                }
            })
        })
        .collect()
}

pub struct Model {
    pub report: Option<DoctorReport>,
    pub selected: usize,
    pub browse: Browse,
    pub path_filter: PathFilter,
    pub preview: Option<ActionPreview>,
    pub message: String,
    pub work: WorkState,
    pub screen: Screen,
    pub detail_scroll: u16,
    pub scroll_limit: Cell<u16>,
    pub progress: Option<ScanProgress>,
    pub roots: Vec<PathBuf>,
    pub elapsed: Duration,
    pub color: bool,
    pub filter: String,
    pub editing_filter: bool,
    pub details_focused: bool,
    pub history: Vec<usize>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            report: None,
            selected: 0,
            browse: Browse::default(),
            path_filter: PathFilter::default(),
            preview: None,
            message: "Scanning Skill files. Press q to cancel.".into(),
            work: WorkState::Scanning,
            screen: Screen::Skills,
            detail_scroll: 0,
            scroll_limit: Cell::new(0),
            progress: None,
            roots: vec![],
            elapsed: Duration::ZERO,
            color: true,
            filter: String::new(),
            editing_filter: false,
            details_focused: false,
            history: vec![],
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
                        let scopes = locations(s, self.path_filter);
                        !scopes.is_empty()
                            && match &self.browse {
                                Browse::Owners => true,
                                Browse::Locations { owner } => in_group(s, owner),
                                Browse::Skills { owner, location } => {
                                    in_group(s, owner) && scopes.contains(location)
                                }
                            }
                            && (self.filter.is_empty()
                                || format!(
                                    "{} {} {} {}",
                                    s.name,
                                    s.owner.label(),
                                    s.canonical_path.display(),
                                    s.paths
                                        .iter()
                                        .map(|p| p.path.display().to_string())
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                )
                                .to_lowercase()
                                .contains(&self.filter.to_lowercase()))
                    })
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn entries(&self) -> Vec<Entry> {
        let visible = self.visible();
        let Some(report) = &self.report else {
            return vec![];
        };
        match &self.browse {
            Browse::Owners => [
                ("skills.sh", "Review skills.sh migration"),
                ("Unknown", "Inspect unknown installs"),
                ("Unavailable", "Resolve ownership problems"),
                ("Duplicates", "Review duplicate copies"),
                ("skilld", "Browse skilld installs"),
                ("Plugin", "Browse plugin Skills"),
                ("Source", "Browse source directories"),
            ]
            .into_iter()
            .filter_map(|(owner, label)| {
                let skills: Vec<_> = visible
                    .iter()
                    .copied()
                    .filter(|&i| in_group(&report.skills[i], owner))
                    .collect();
                (!skills.is_empty()).then(|| Entry::Group {
                    label: label.into(),
                    skills,
                    next: Browse::Locations { owner },
                })
            })
            .collect(),
            Browse::Locations { owner } => {
                let scopes: BTreeSet<_> = visible
                    .iter()
                    .flat_map(|&i| locations(&report.skills[i], self.path_filter))
                    .collect();
                scopes
                    .into_iter()
                    .map(|location| Entry::Group {
                        label: location.as_ref().map_or_else(
                            || "Global Agent targets".into(),
                            |p| p.display().to_string(),
                        ),
                        skills: visible
                            .iter()
                            .copied()
                            .filter(|&i| {
                                locations(&report.skills[i], self.path_filter).contains(&location)
                            })
                            .collect(),
                        next: Browse::Skills { owner, location },
                    })
                    .collect()
            }
            Browse::Skills { .. } => visible.into_iter().map(Entry::Skill).collect(),
        }
    }
    pub fn selected_skill(&self) -> Option<usize> {
        match self.entries().get(self.selected) {
            Some(Entry::Skill(i)) => Some(*i),
            _ => None,
        }
    }
    pub fn open_group(&mut self) {
        if self.work != WorkState::Ready || self.preview.is_some() {
            return;
        }
        if let Some(Entry::Group { next, .. }) = self.entries().into_iter().nth(self.selected) {
            self.history.push(self.selected);
            self.browse = next;
            self.reset_selection();
        }
    }
    pub fn back(&mut self) {
        if self.screen != Screen::Skills {
            self.screen = Screen::Skills;
        } else if self.preview.take().is_some() {
            // Keep the Skill selected after cancelling its action review.
        } else if !self.filter.is_empty() {
            self.filter.clear();
        } else {
            self.browse = match self.browse {
                Browse::Skills { owner, .. } => Browse::Locations { owner },
                _ => Browse::Owners,
            };
            self.reset_selection();
            self.selected = self
                .history
                .pop()
                .unwrap_or(0)
                .min(self.entries().len().saturating_sub(1));
        }
        self.detail_scroll = 0;
    }
    fn reset_selection(&mut self) {
        self.selected = 0;
        self.detail_scroll = 0;
        self.details_focused = false;
    }
    pub fn move_selection(&mut self, down: bool) {
        if self.preview.is_some() || self.details_focused || self.screen != Screen::Skills {
            self.detail_scroll = if down {
                self.detail_scroll
                    .saturating_add(1)
                    .min(self.scroll_limit.get())
            } else {
                self.detail_scroll.saturating_sub(1)
            };
            return;
        }
        let len = self.entries().len();
        if len > 0 {
            self.selected = if down {
                (self.selected + 1) % len
            } else {
                self.selected.checked_sub(1).unwrap_or(len - 1)
            };
        }
        self.detail_scroll = 0;
    }
    pub fn can_review_action(&self, width: u16, height: u16) -> bool {
        width >= 60
            && height >= 18
            && self.work == WorkState::Ready
            && self.screen == Screen::Skills
            && self.preview.is_none()
            && !self.editing_filter
            && self.selected_skill().is_some()
    }
    pub fn toggle_group(&mut self) {
        if self.work == WorkState::Ready && self.preview.is_none() {
            self.path_filter = match self.path_filter {
                PathFilter::All => PathFilter::Symlinks,
                PathFilter::Symlinks => PathFilter::Directories,
                PathFilter::Directories => PathFilter::All,
            };
            self.browse = Browse::Owners;
            self.history.clear();
            self.reset_selection();
        }
    }
}

pub fn view(frame: &mut ratatui::Frame<'_>, model: &Model) {
    if frame.area().width < 60 || frame.area().height < 18 {
        frame.render_widget(
            Paragraph::new(if model.work == WorkState::Applying {
                "skilld doctor\n\nApplying action. Wait for the result.\nResize to at least 60 columns and 18 rows."
            } else {
                "skilld doctor\n\nResize to at least 60 columns and 18 rows.\nq quit"
            })
                .style(tone(model.color, ACCENT)),
            frame.area(),
        );
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(4),
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
    let scope = model
        .roots
        .iter()
        .map(|p| sanitize(&p.display().to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    "skilld doctor",
                    tone(model.color, ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("  {counts}")),
            ]),
            Line::styled(
                if model.work == WorkState::Scanning {
                    format!("Scan: {scope}")
                } else {
                    let trail = match &model.browse {
                        Browse::Owners => "Overview".into(),
                        Browse::Locations { owner } => format!("Overview > {owner}"),
                        Browse::Skills { owner, location } => format!(
                            "Overview > {owner} > {}",
                            location.as_ref().map_or_else(
                                || "Global Agent targets".into(),
                                |p| p.display().to_string()
                            )
                        ),
                    };
                    format!("{} | {trail}", model.path_filter.label())
                },
                tone(model.color, MUTED),
            ),
        ]),
        rows[0],
    );
    if model.work == WorkState::Scanning {
        loading(frame, model, rows[1]);
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw("Scanning reads files. Cleanup actions need your review."),
                Line::styled("q cancel", tone(model.color, ACCENT)),
            ]),
            rows[2],
        );
        return;
    }
    let cols = if model.preview.is_some() || model.screen != Screen::Skills {
        vec![Rect::new(rows[1].x, rows[1].y, 0, 0), rows[1]]
    } else if frame.area().width < 76 {
        Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(rows[1])
            .to_vec()
    } else {
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(rows[1])
            .to_vec()
    };
    let visible = model.visible();
    let entries = model.entries();
    let items = entries
        .iter()
        .map(|entry| match entry {
            Entry::Group {
                label,
                skills,
                next,
            } => {
                let short = match next {
                    Browse::Skills {
                        location: Some(p), ..
                    } => p
                        .file_name()
                        .map_or_else(|| label.clone(), |n| n.to_string_lossy().into_owned()),
                    _ => label.clone(),
                };
                ListItem::new(vec![
                    Line::styled(
                        sanitize(&short),
                        tone(model.color, ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Line::styled(
                        format!("  {} · Enter to review", skill_count(skills.len())),
                        tone(model.color, MUTED),
                    ),
                ])
            }
            Entry::Skill(i) => {
                let s = &model.report.as_ref().expect("visible report").skills[*i];
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if s.duplicates.is_empty() {
                            ""
                        } else {
                            "Duplicate · "
                        },
                        tone(model.color, Color::Yellow),
                    ),
                    Span::raw(sanitize(&s.name)),
                ]))
            }
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(if entries.is_empty() {
        None
    } else {
        Some(model.selected.min(entries.len() - 1))
    });
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(tone(
                        model.color,
                        if model.details_focused { MUTED } else { ACCENT },
                    ))
                    .title(format!(
                        " {}{} · {} ",
                        if !model.details_focused && model.preview.is_none() {
                            "> "
                        } else {
                            ""
                        },
                        match model.browse {
                            Browse::Owners => "Recommendations",
                            Browse::Locations { .. } => "Projects / folders",
                            Browse::Skills { .. } => "Skills",
                        },
                        entries.len()
                    )),
            )
            .highlight_symbol("> ")
            .highlight_style(selection()),
        cols[0],
        &mut state,
    );
    let (title, details) = if model.screen == Screen::Help {
        (
            "Help",
            "Open a recommendation, then a folder, then a Skill.

Up/down or j/k: select or scroll
Left/right: focus list or details
Enter: open group; Esc: back
Tab: All paths / Symlinks / Directories
/: filter Skills; Enter: finish; Esc: clear
PgUp/PgDn: scroll details
m: review migration; d: review removal
p: scan problems; n: full notice; ?: help; Esc: back
r: scan again; q or Ctrl-C: quit

Actions always require a separate review.
While applying, wait for the result."
                .into(),
        )
    } else if model.screen == Screen::Notice {
        ("Latest notice", model.message.clone())
    } else if model.screen == Screen::Problems {
        let text = model
            .report
            .as_ref()
            .map(|report| {
                report
                    .problems
                    .iter()
                    .map(|p| format!("{}\n{}\n", p.path.display(), p.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        (
            "Scan problems",
            if text.is_empty() {
                "No scan problems. Esc returns to Skills.".into()
            } else {
                text
            },
        )
    } else if let Some(p) = &model.preview {
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
    } else if let Some(Entry::Group {
        label,
        skills,
        next,
    }) = entries.get(model.selected)
    {
        let report = model.report.as_ref().expect("group report");
        let duplicate_count = skills
            .iter()
            .filter(|&&i| !report.skills[i].duplicates.is_empty())
            .count();
        let linked_count = skills
            .iter()
            .filter(|&&i| {
                report.skills[i]
                    .paths
                    .iter()
                    .any(|p| p.symlink_target.is_some())
            })
            .count();
        let description = match next {
            Browse::Locations { owner: "skills.sh" } => {
                "Installed through skills.sh. Review migration or removal for each Skill."
            }
            Browse::Locations { owner: "Unknown" } => {
                "No installation record found. Inspect each Skill before removing it."
            }
            Browse::Locations {
                owner: "Unavailable",
            } => "Ownership could not be checked. Press p to inspect scan problems.",
            Browse::Locations {
                owner: "Duplicates",
            } => {
                "These directories contain identical files. Compare paths, then use each owner's removal workflow. Doctor reviews all targets for an install."
            }
            Browse::Locations { owner: "skilld" } => {
                "Managed by skilld. Use skilld update or remove for these Skills."
            }
            Browse::Locations { owner: "Plugin" } => {
                "Managed by plugins. Use the plugin manager to make changes."
            }
            Browse::Locations { owner: "Source" } => {
                "Source directories. No cleanup action applies here."
            }
            _ => "Skills found in this project or folder. A Skill may appear in several locations.",
        };
        (
            "Recommended next step",
            format!(
                "{label}\n\n{description}\n\nEnter opens this group.\n\n{}\n{linked_count} with symlinks\n{duplicate_count} with identical copies\n\nFilters only change this view.\nActions review all targets for the install.\nRecommendations can include the same Skill.\n\nScan: {scope}",
                skill_count(skills.len())
            ),
        )
    } else if let Some(i) = model.selected_skill() {
        let s = &model.report.as_ref().expect("visible report").skills[i];
        let mut text = format!(
            "{}\nOwner: {}\n\nFilters only change this view.\nActions review all targets for the install.\n\n{}\n",
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
        text.push_str("\nDirectories:\n");
        for p in s.paths.iter().filter(|p| p.symlink_target.is_none()) {
            text.push_str(&format!("{}\n", p.path.display()));
        }
        text.push_str("\nSymlinks:\n");
        let mut links = 0;
        for p in &s.paths {
            if let Some(target) = &p.symlink_target {
                links += 1;
                text.push_str(&format!(
                    "{}\n  -> {}\n",
                    p.path.display(),
                    target.display()
                ));
            }
        }
        if links == 0 {
            text.push_str("None\n");
        }
        if !s.duplicates.is_empty() {
            text.push_str(&format!(
                "\nDuplicate: {} identical directory copies.\n",
                s.duplicates.len()
            ));
            for path in &s.duplicates {
                text.push_str(&format!("{}\n", path.display()));
            }
            text.push_str("Symlinks to this Skill are not duplicate copies.\n");
        }
        if let Some(m) = &s.source_match {
            text.push_str(&format!(
                "\n{}\nCommit: {}\n{}\n",
                m.source,
                m.commit.as_deref().unwrap_or("unknown"),
                m.result
            ));
        }
        text.push_str(match &s.owner {
            DoctorOwner::SkillsSh { .. } => {
                "\nActions: m migrate to skilld · d remove.\nReview affected paths before applying."
            }
            DoctorOwner::Unknown => "\nNo installation record found.\nAction: d review removal.",
            DoctorOwner::Skilld { .. } => "\nManaged by skilld. Use skilld update or remove.",
            DoctorOwner::Plugin => "\nManaged by a plugin. Change it through its plugin manager.",
            DoctorOwner::Source => "\nSource directory. No cleanup action applies here.",
            DoctorOwner::Unavailable { .. } => {
                "\nOwnership could not be checked. Resolve the scan problem first."
            }
        });
        ("Skill details", text)
    } else {
        (
            "Skill details",
            if model.filter.is_empty() {
                "No Skills in this group. Press Tab to switch groups."
            } else {
                "No Skills match this filter. Press / to change it."
            }
            .into(),
        )
    };
    let limit = bounded_scroll(&details, cols[1], u16::MAX);
    model.scroll_limit.set(limit);
    frame.render_widget(
        Paragraph::new(details.lines().map(sanitize).collect::<Vec<_>>().join("\n"))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " {}{title} ",
                        if model.details_focused
                            || model.preview.is_some()
                            || model.screen != Screen::Skills
                        {
                            "> "
                        } else {
                            ""
                        }
                    ))
                    .border_style(tone(
                        model.color,
                        if model.preview.is_some() {
                            Color::Yellow
                        } else if model.details_focused {
                            ACCENT
                        } else {
                            MUTED
                        },
                    )),
            )
            .wrap(Wrap { trim: false })
            .scroll((model.detail_scroll.min(limit), 0)),
        cols[1],
    );
    let actions = model
        .selected_skill()
        .map(|i| &model.report.as_ref().expect("visible report").skills[i].owner);
    let help = if model.editing_filter {
        "type to filter  enter done  esc clear"
    } else if model.work == WorkState::Applying {
        "Applying action. Wait for the result."
    } else if model.work == WorkState::Preparing {
        "Preparing action  q quit"
    } else if model.screen != Screen::Skills {
        "↑/↓ scroll  esc back  q quit"
    } else if model.preview.is_some() {
        "enter apply  esc cancel  ↑/↓ scroll"
    } else if !matches!(model.browse, Browse::Skills { .. }) {
        "enter open  esc back  / filter  tab paths  ? help  q quit"
    } else {
        match actions {
            Some(DoctorOwner::SkillsSh { .. }) => {
                "m migrate  d remove  esc back  / filter  ? help  q quit"
            }
            Some(DoctorOwner::Unknown) => "d remove  esc back  / filter  ? help  q quit",
            _ => "esc back  / filter  tab paths  ? help  q quit",
        }
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                sanitize(&model.message),
                tone(
                    model.color,
                    if model.editing_filter {
                        ACCENT
                    } else {
                        Color::Reset
                    },
                ),
            ),
            Line::styled(help, tone(model.color, ACCENT)),
            Line::styled(
                if model.work == WorkState::Applying || model.work == WorkState::Preparing {
                    format!("{}s elapsed", model.elapsed.as_secs())
                } else if model.preview.is_some() {
                    "PgUp/PgDn scroll  q quit".into()
                } else if model.screen != Screen::Skills {
                    "PgUp/PgDn scroll".into()
                } else if model.filter.is_empty() && !model.editing_filter {
                    "↑/↓ move  ←/→ focus  PgUp/PgDn scroll  p problems  n notice  r rescan".into()
                } else {
                    format!(
                        "Filter: {}   {} matching   / edit  esc clear",
                        sanitize(&model.filter),
                        visible.len()
                    )
                },
                tone(model.color, MUTED),
            ),
        ]),
        rows[2],
    );
}

fn loading(frame: &mut ratatui::Frame<'_>, model: &Model, area: Rect) {
    let block = Block::bordered()
        .title(" Scan progress ")
        .border_style(tone(model.color, ACCENT));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let phase = model
        .progress
        .as_ref()
        .map_or(ScanPhase::Discover, |p| p.phase);
    let (label, count) = match phase {
        ScanPhase::Discover => ("Finding Skill files", None),
        ScanPhase::Fingerprint { completed, total } => {
            ("Checking Skill contents", Some((completed, total)))
        }
        ScanPhase::Sources { completed, total } => {
            ("Checking source candidates", Some((completed, total)))
        }
    };
    let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        [(model.elapsed.as_millis() / 100 % 10) as usize];
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(3),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(format!(
            "{spinner} {label}   {}s elapsed",
            model.elapsed.as_secs()
        ))
        .style(tone(model.color, ACCENT).add_modifier(Modifier::BOLD)),
        rows[0],
    );
    if let Some((completed, total)) = count {
        frame.render_widget(
            Gauge::default()
                .ratio(if total == 0 {
                    1.0
                } else {
                    (completed as f64 / total as f64).min(1.0)
                })
                .label(format!("{completed} / {total} Skills"))
                .gauge_style(tone(model.color, ACCENT)),
            rows[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("Finding directories first. The total is not known yet.")
                .style(tone(model.color, MUTED)),
            rows[1],
        );
    }
    let mut lines = vec![];
    if let Some(p) = &model.progress {
        lines.extend([
            Line::from(vec![
                Span::styled(
                    format!("{} Skills found", p.found_skills),
                    tone(model.color, Color::Green),
                ),
                Span::raw(format!("   {} directories scanned", p.visited_directories)),
            ]),
            Line::styled(
                format!(
                    "{} excluded   {} problems",
                    p.skipped_directories, p.problems
                ),
                tone(
                    model.color,
                    if p.problems == 0 {
                        MUTED
                    } else {
                        Color::Yellow
                    },
                ),
            ),
            Line::raw(""),
            Line::styled("Current directory", tone(model.color, MUTED)),
            Line::raw(sanitize(&p.current_path.display().to_string())),
        ]);
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[2]);
}

fn bounded_scroll(text: &str, area: Rect, requested: u16) -> u16 {
    let width = area.width.saturating_sub(2).max(1) as usize;
    let height = area.height.saturating_sub(2) as usize;
    let lines: usize = text
        .lines()
        .map(|line| {
            unicode_width::UnicodeWidthStr::width(line)
                .max(1)
                .div_ceil(width)
        })
        .sum();
    requested.min(lines.saturating_sub(height).min(u16::MAX as usize) as u16)
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
    Progress(ScanProgress),
    Scan(Result<DoctorReport, CommandError>),
    Plan(Result<Box<DoctorPlan>, CommandError>),
    Applied(Result<DoctorApplied, CommandError>),
}

pub fn run_doctor(
    host: Arc<LocalHost>,
    options: DoctorOptions,
) -> Result<(), InteractiveUpdateError> {
    let roots = if options.roots.is_empty() {
        vec![
            host.doctor_context()
                .map_err(|e| InteractiveUpdateError::new(e.code, e.message))?
                .home,
        ]
    } else {
        options.roots.clone()
    };
    let summaries = with_restored_terminal(NativeTerminalLifecycle, || {
        let mut terminal =
            ratatui::Terminal::new(CrosstermBackend::new(io::stdout())).map_err(terminal_error)?;
        let (tx, rx) = mpsc::channel();
        start_scan(host.clone(), options.clone(), tx.clone());
        let mut model = Model {
            roots,
            color: std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").is_ok_and(|v| v != "dumb"),
            ..Model::default()
        };
        let mut started = Instant::now();
        let mut plan = None;
        let mut summaries = Vec::new();
        let mut previous_path = None;
        let mut completed_message = None;
        loop {
            while let Ok(job) = rx.try_recv() {
                model.work = WorkState::Ready;
                match job {
                    Job::Progress(progress) => {
                        model.work = WorkState::Scanning;
                        model.progress = Some(progress);
                    }
                    Job::Scan(result) => {
                        model.progress = None;
                        match result {
                            Ok(report) => {
                                model.message = if let Some(message) = completed_message.take() {
                                    message
                                } else if report.problems.is_empty() {
                                    "Choose a recommendation to review.".into()
                                } else {
                                    format!(
                                        "{} scan problems. Press p for paths and reasons.",
                                        report.problems.len()
                                    )
                                };
                                model.report = Some(report);
                                model.selected = previous_path
                                    .take()
                                    .and_then(|path| {
                                        model.visible().iter().position(|&index| {
                                            model.report.as_ref().unwrap().skills[index]
                                                .canonical_path
                                                == path
                                        })
                                    })
                                    .unwrap_or(0);
                            }
                            Err(e) => model.message = e.to_string(),
                        }
                    }
                    Job::Plan(result) => match result {
                        Ok(p) => {
                            let mut cancelled = false;
                            while event::poll(Duration::ZERO).map_err(terminal_error)? {
                                if let Event::Key(key) = event::read().map_err(terminal_error)? {
                                    if key.kind != KeyEventKind::Press {
                                        continue;
                                    }
                                    if key.code == KeyCode::Char('q')
                                        || (key.code == KeyCode::Char('c')
                                            && key.modifiers.contains(event::KeyModifiers::CONTROL))
                                    {
                                        return Ok(summaries);
                                    }
                                    cancelled |= key.code == KeyCode::Esc;
                                }
                            }
                            if cancelled {
                                model.message = "Action review cancelled. No files changed.".into();
                                continue;
                            }
                            model.preview = Some(p.preview.clone());
                            model.detail_scroll = 0;
                            model.message = "Review every affected target before applying.".into();
                            plan = Some(p);
                        }
                        Err(e) => model.message = e.to_string(),
                    },
                    Job::Applied(result) => {
                        model.work = WorkState::Ready;
                        model.preview = None;
                        match result {
                            Ok(done) => {
                                model.message =
                                    format!("{}. Backup: {}", done.message, done.backup.display());
                                summaries.push(model.message.clone());
                                completed_message = Some(model.message.clone());
                                model.work = WorkState::Scanning;
                                started = Instant::now();
                                start_scan(host.clone(), options.clone(), tx.clone());
                            }
                            Err(e) => {
                                model.message = e.to_string();
                                summaries.push(model.message.clone());
                            }
                        }
                    }
                }
            }
            if model.work != WorkState::Ready {
                model.elapsed = started.elapsed();
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
            if model.editing_filter && !key.modifiers.contains(event::KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Esc => {
                        model.filter.clear();
                        model.editing_filter = false;
                    }
                    KeyCode::Enter => model.editing_filter = false,
                    KeyCode::Backspace => {
                        model.filter.pop();
                    }
                    KeyCode::Char(c) => model.filter.push(c),
                    _ => {}
                }
                model.selected = 0;
                model.detail_scroll = 0;
                continue;
            }
            if (key.code == KeyCode::Char('q')
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(event::KeyModifiers::CONTROL)))
                && model.work != WorkState::Applying
            {
                return Ok(summaries);
            }
            if model.work != WorkState::Ready {
                continue;
            }
            let size = terminal.size().map_err(terminal_error)?;
            if size.width < 60 || size.height < 18 {
                continue;
            }
            if model.screen != Screen::Skills
                && !matches!(
                    key.code,
                    KeyCode::Esc
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                        | KeyCode::Char('j' | 'k' | '?' | 'p' | 'n')
                )
            {
                continue;
            }
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => model.move_selection(true),
                KeyCode::Up | KeyCode::Char('k') => model.move_selection(false),
                KeyCode::PageDown => {
                    model.detail_scroll = model
                        .detail_scroll
                        .saturating_add(size.height.saturating_sub(8))
                        .min(model.scroll_limit.get())
                }
                KeyCode::PageUp => {
                    model.detail_scroll = model
                        .detail_scroll
                        .min(model.scroll_limit.get())
                        .saturating_sub(size.height.saturating_sub(8))
                }
                KeyCode::Left => model.details_focused = false,
                KeyCode::Right => model.details_focused = true,
                KeyCode::Char('/') if model.preview.is_none() => model.editing_filter = true,
                KeyCode::Tab => model.toggle_group(),
                KeyCode::Char('?') if model.preview.is_none() => {
                    model.screen = Screen::Help;
                    model.detail_scroll = 0;
                }
                KeyCode::Char('p') if model.preview.is_none() => {
                    model.screen = Screen::Problems;
                    model.detail_scroll = 0;
                }
                KeyCode::Char('n') if model.preview.is_none() => {
                    model.screen = Screen::Notice;
                    model.detail_scroll = 0;
                }
                KeyCode::Esc => {
                    plan = None;
                    model.back();
                }
                KeyCode::Enter
                    if model.work == WorkState::Ready && model.screen == Screen::Skills =>
                {
                    if let Some(p) = plan.take() {
                        previous_path = model.selected_skill().map(|i| {
                            model.report.as_ref().unwrap().skills[i]
                                .canonical_path
                                .clone()
                        });
                        model.work = WorkState::Applying;
                        started = Instant::now();
                        model.message = "Applying reviewed action. Keep this terminal open.".into();
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(Job::Applied(p.apply()));
                        });
                    } else {
                        model.open_group();
                    }
                }
                KeyCode::Char('r') if model.preview.is_none() => {
                    previous_path = model.selected_skill().map(|i| {
                        model.report.as_ref().unwrap().skills[i]
                            .canonical_path
                            .clone()
                    });
                    model.work = WorkState::Scanning;
                    started = Instant::now();
                    start_scan(host.clone(), options.clone(), tx.clone());
                }
                KeyCode::Char('m' | 'd') if model.can_review_action(size.width, size.height) => {
                    if let (Some(report), Some(index)) =
                        (model.report.clone(), model.selected_skill())
                    {
                        let owner = &report.skills[index].owner;
                        if !matches!(owner, DoctorOwner::SkillsSh { .. } | DoctorOwner::Unknown)
                            || (key.code == KeyCode::Char('m')
                                && !matches!(owner, DoctorOwner::SkillsSh { .. }))
                        {
                            continue;
                        }
                        let action = if key.code == KeyCode::Char('m') {
                            DoctorAction::Migrate
                        } else {
                            DoctorAction::Remove
                        };
                        model.work = WorkState::Preparing;
                        started = Instant::now();
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
    })?;
    let mut output = io::stdout().lock();
    for summary in summaries {
        writeln!(output, "{}", sanitize(&summary)).map_err(terminal_error)?;
    }
    Ok(())
}

fn start_scan(host: Arc<LocalHost>, options: DoctorOptions, tx: mpsc::Sender<Job>) {
    std::thread::spawn(move || {
        let mut last = Instant::now();
        let mut phase = None;
        let result = host.doctor_scan_with_progress(&options, &mut |progress| {
            let current = std::mem::discriminant(&progress.phase);
            if phase != Some(current) || last.elapsed() >= Duration::from_millis(80) {
                // A closed receiver means the person quit the read-only scan.
                let _ = tx.send(Job::Progress(progress));
                last = Instant::now();
                phase = Some(current);
            }
        });
        let _ = tx.send(Job::Scan(result));
    });
}
fn terminal_error(error: io::Error) -> InteractiveUpdateError {
    InteractiveUpdateError::new("TERMINAL_UNAVAILABLE", error.to_string())
}
