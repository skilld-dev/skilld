use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Tabs};
use skilld_command::{CommandError, Host};
use skilld_core::{CommitHistory, UpdatePlanItem, UpdatePlanV1, UpdateRelation, UpdateRetryAfter};
use skilld_ui::spinner;
use skilld_ui::time::relative_time;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use url::Url;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ComparisonId(String);

impl ComparisonId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(clean_text(&value.into()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateCandidate {
    Available {
        name: String,
        repository: String,
        locked_commit_sha: String,
        latest_commit_sha: String,
        commit_count: u64,
        comparison_id: ComparisonId,
    },
    Unavailable {
        name: String,
        error: InteractiveUpdateError,
    },
}

impl UpdateCandidate {
    pub fn new(
        name: impl Into<String>,
        repository: impl Into<String>,
        locked_commit_sha: impl Into<String>,
        latest_commit_sha: impl Into<String>,
        commit_count: u64,
        comparison_id: ComparisonId,
    ) -> Self {
        Self::Available {
            name: clean_text(&name.into()),
            repository: clean_text(&repository.into()),
            locked_commit_sha: clean_text(&locked_commit_sha.into()),
            latest_commit_sha: clean_text(&latest_commit_sha.into()),
            commit_count,
            comparison_id,
        }
    }

    pub fn unavailable(name: impl Into<String>, error: InteractiveUpdateError) -> Self {
        Self::Unavailable {
            name: clean_text(&name.into()),
            error,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Available { name, .. } | Self::Unavailable { name, .. } => name,
        }
    }

    fn comparison_id(&self) -> Option<&ComparisonId> {
        match self {
            Self::Available { comparison_id, .. } => Some(comparison_id),
            Self::Unavailable { .. } => None,
        }
    }

    const fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitSummary {
    commit_sha: String,
    subject: String,
    timestamp: String,
    author: String,
}

impl CommitSummary {
    pub fn new(
        commit_sha: impl Into<String>,
        subject: impl Into<String>,
        timestamp: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        let subject = subject.into();
        Self {
            commit_sha: clean_text(&commit_sha.into()),
            subject: clean_text(subject.lines().next().unwrap_or_default()),
            timestamp: clean_text(&timestamp.into()),
            author: clean_text(&author.into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitPage {
    comparison_id: ComparisonId,
    commits: Vec<CommitSummary>,
    total: u64,
    truncated: bool,
    compare_url: String,
}

impl CommitPage {
    pub fn new(
        comparison_id: ComparisonId,
        mut commits: Vec<CommitSummary>,
        total: u64,
        truncated: bool,
        compare_url: impl Into<String>,
    ) -> Self {
        commits.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));
        Self {
            comparison_id,
            commits,
            total,
            truncated,
            compare_url: clean_text(&compare_url.into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveUpdateError {
    pub code: String,
    pub message: String,
}

impl InteractiveUpdateError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: clean_text(&code.into()),
            message: clean_text(&message.into()),
        }
    }

    fn terminal(code: &'static str, message: &'static str) -> Self {
        Self::new(code, message)
    }
}

impl fmt::Display for InteractiveUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for InteractiveUpdateError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitLoadResult {
    comparison_id: ComparisonId,
    result: Result<CommitPage, InteractiveUpdateError>,
}

impl CommitLoadResult {
    pub fn ready(comparison_id: ComparisonId, page: CommitPage) -> Self {
        Self {
            comparison_id,
            result: Ok(page),
        }
    }

    pub fn failed(comparison_id: ComparisonId, error: InteractiveUpdateError) -> Self {
        Self {
            comparison_id,
            result: Err(error),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyResult {
    name: String,
    result: Result<(), InteractiveUpdateError>,
}

impl ApplyResult {
    pub fn updated(name: impl Into<String>) -> Self {
        Self {
            name: clean_text(&name.into()),
            result: Ok(()),
        }
    }

    pub fn failed(name: impl Into<String>, error: InteractiveUpdateError) -> Self {
        Self {
            name: clean_text(&name.into()),
            result: Err(error),
        }
    }
}

pub trait InteractiveUpdateHost: Send + Sync + 'static {
    fn load_candidates(&self) -> Result<Vec<UpdateCandidate>, InteractiveUpdateError>;

    fn load_commits(&self, comparisons: &[ComparisonId]) -> Vec<CommitLoadResult>;

    fn apply(&self, names: &[String]) -> Vec<ApplyResult>;
}

pub struct CommandInteractiveUpdateHost<H: Host> {
    host: Arc<H>,
    plan: Mutex<CachedInteractivePlan>,
}

#[derive(Default)]
struct CachedInteractivePlan {
    commits: BTreeMap<ComparisonId, CommitPage>,
    items: BTreeMap<String, UpdatePlanItem>,
}

impl<H: Host> CommandInteractiveUpdateHost<H> {
    pub fn new(host: Arc<H>) -> Self {
        Self {
            host,
            plan: Mutex::new(CachedInteractivePlan::default()),
        }
    }
}

impl<H: Host + Send + Sync + 'static> InteractiveUpdateHost for CommandInteractiveUpdateHost<H> {
    fn load_candidates(&self) -> Result<Vec<UpdateCandidate>, InteractiveUpdateError> {
        let plan = self.host.update_check(None).map_err(command_error)?;
        let (candidates, commits) = prepare_interactive_plan(&plan);
        let items = plan
            .items()
            .iter()
            .filter(|item| matches!(item.relation(), UpdateRelation::Available { .. }))
            .map(|item| (item.name().as_str().to_owned(), item.clone()))
            .collect();
        *self.plan.lock().map_err(|_| {
            InteractiveUpdateError::new(
                "SERVICE_UNAVAILABLE",
                "The reviewed update plan could not be saved.",
            )
        })? = CachedInteractivePlan { commits, items };
        Ok(candidates)
    }

    fn load_commits(&self, comparisons: &[ComparisonId]) -> Vec<CommitLoadResult> {
        let plan = match self.plan.lock() {
            Ok(plan) => plan,
            Err(_) => {
                return comparisons
                    .iter()
                    .cloned()
                    .map(|comparison| {
                        CommitLoadResult::failed(
                            comparison,
                            InteractiveUpdateError::new(
                                "SERVICE_UNAVAILABLE",
                                "Repository commits could not be read.",
                            ),
                        )
                    })
                    .collect();
            }
        };
        comparisons
            .iter()
            .cloned()
            .map(|comparison| match plan.commits.get(&comparison) {
                Some(page) => CommitLoadResult::ready(comparison, page.clone()),
                None => CommitLoadResult::failed(
                    comparison,
                    InteractiveUpdateError::new(
                        "COMPARISON_UNAVAILABLE",
                        "The repository commit range is unavailable.",
                    ),
                ),
            })
            .collect()
    }

    fn apply(&self, names: &[String]) -> Vec<ApplyResult> {
        let items = self
            .plan
            .lock()
            .map_err(|_| {
                InteractiveUpdateError::new(
                    "SERVICE_UNAVAILABLE",
                    "The reviewed update plan could not be read.",
                )
            })
            .and_then(|plan| {
                names
                    .iter()
                    .map(|name| {
                        plan.items.get(name).cloned().ok_or_else(|| {
                            InteractiveUpdateError::new(
                                "STALE_UPDATE_PLAN",
                                format!("Skill {name} needs another commit review."),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            });
        let items = match items {
            Ok(items) => items,
            Err(error) => {
                return names
                    .iter()
                    .map(|name| ApplyResult::failed(name, error.clone()))
                    .collect();
            }
        };
        match self.host.update_selected(&items) {
            Ok(_) => names.iter().map(ApplyResult::updated).collect(),
            Err(error) => {
                let error = command_error(error);
                names
                    .iter()
                    .map(|name| ApplyResult::failed(name, error.clone()))
                    .collect()
            }
        }
    }
}

fn prepare_interactive_plan(
    plan: &UpdatePlanV1,
) -> (Vec<UpdateCandidate>, BTreeMap<ComparisonId, CommitPage>) {
    let mut candidates = Vec::new();
    let mut pages = BTreeMap::new();
    for item in plan.items() {
        let name = item.name().as_str();
        match item.relation() {
            UpdateRelation::Available {
                locked_commit_sha,
                latest_commit_sha,
                ahead_by,
            } => {
                let CommitHistory::Compared {
                    items,
                    total,
                    truncated,
                    compare_url,
                } = item.history()
                else {
                    candidates.push(UpdateCandidate::unavailable(
                        name,
                        InteractiveUpdateError::new(
                            "INVALID_UPDATE_PLAN",
                            "The Skill has no repository commit range.",
                        ),
                    ));
                    continue;
                };
                let Some(repository) = repository_from_compare_url(compare_url) else {
                    candidates.push(UpdateCandidate::unavailable(
                        name,
                        InteractiveUpdateError::new(
                            "INVALID_UPDATE_PLAN",
                            "The repository compare URL is invalid.",
                        ),
                    ));
                    continue;
                };
                let comparison = ComparisonId::new(format!(
                    "{repository}:{}..{}",
                    locked_commit_sha.as_str(),
                    latest_commit_sha.as_str()
                ));
                let commits = items
                    .iter()
                    .map(|commit| {
                        CommitSummary::new(
                            commit.sha.as_str(),
                            &commit.subject,
                            &commit.timestamp,
                            &commit.author.name,
                        )
                    })
                    .collect();
                pages.insert(
                    comparison.clone(),
                    CommitPage::new(comparison.clone(), commits, *total, *truncated, compare_url),
                );
                candidates.push(UpdateCandidate::new(
                    name,
                    repository,
                    locked_commit_sha.as_str(),
                    latest_commit_sha.as_str(),
                    ahead_by.get(),
                    comparison,
                ));
            }
            UpdateRelation::Unavailable { failure, .. } => {
                candidates.push(UpdateCandidate::unavailable(
                    name,
                    InteractiveUpdateError::new(&failure.code, update_failure_message(failure)),
                ));
            }
            UpdateRelation::Behind { .. } => {
                candidates.push(UpdateCandidate::unavailable(
                    name,
                    InteractiveUpdateError::new(
                        "SOURCE_BEHIND",
                        "The installed Skill is ahead of its source.",
                    ),
                ));
            }
            UpdateRelation::Diverged { .. } => {
                candidates.push(UpdateCandidate::unavailable(
                    name,
                    InteractiveUpdateError::new(
                        "SOURCE_DIVERGED",
                        "The installed Skill and its source have diverged.",
                    ),
                ));
            }
            UpdateRelation::Current { .. }
            | UpdateRelation::Pinned { .. }
            | UpdateRelation::NotTracked { .. } => {}
        }
    }
    (candidates, pages)
}

fn repository_from_compare_url(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let mut segments = url.path_segments()?;
    let owner = segments.next()?;
    let repository = segments.next()?;
    let kind = segments.next()?;
    let range = segments.next()?;
    (kind == "compare"
        && !owner.is_empty()
        && !repository.is_empty()
        && !range.is_empty()
        && segments.next().is_none())
    .then(|| format!("{owner}/{repository}"))
}

fn update_failure_message(failure: &skilld_core::UpdateFailure) -> String {
    let retry = match failure.retry_after.as_ref() {
        Some(UpdateRetryAfter::Seconds { seconds }) => {
            format!(" Try again in {seconds} seconds.")
        }
        Some(UpdateRetryAfter::Reset { reset_at }) => {
            format!(" Try again after {reset_at}.")
        }
        Some(UpdateRetryAfter::SecondsAndReset { seconds, reset_at }) => {
            format!(" Try again in {seconds} seconds, after {reset_at}.")
        }
        Some(UpdateRetryAfter::Unknown) | None => String::new(),
    };
    format!("{}{retry}", failure.message)
}

fn command_error(error: CommandError) -> InteractiveUpdateError {
    InteractiveUpdateError::new(error.code, error.message)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tab {
    Outdated,
    Commits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyInput {
    Up,
    Down,
    PageUp,
    PageDown,
    NextTab,
    PreviousTab,
    Select,
    SelectAll,
    Apply,
    Retry,
    Help,
    Cancel,
    Interrupt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    Key(KeyInput),
    Resized { width: u16, height: u16 },
    CandidatesLoaded(Result<Vec<UpdateCandidate>, InteractiveUpdateError>),
    CommitsLoaded(Vec<CommitLoadResult>),
    Applied(Vec<ApplyResult>),
    Tick,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    LoadCandidates,
    LoadCommits(Vec<ComparisonId>),
    Apply(Vec<String>),
    Exit(InteractiveUpdateSummary),
}

impl Effect {
    pub fn exit_summary(&self) -> Option<&InteractiveUpdateSummary> {
        match self {
            Self::Exit(summary) => Some(summary),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveUpdateSummary {
    pub updated: usize,
    pub failed: usize,
    pub cancelled: bool,
    failures: Vec<(String, InteractiveUpdateError)>,
    interrupted: bool,
    all_current: bool,
}

impl InteractiveUpdateSummary {
    fn current() -> Self {
        Self {
            updated: 0,
            failed: 0,
            cancelled: false,
            failures: Vec::new(),
            interrupted: false,
            all_current: true,
        }
    }

    fn cancelled(interrupted: bool) -> Self {
        Self {
            updated: 0,
            failed: 0,
            cancelled: true,
            failures: Vec::new(),
            interrupted,
            all_current: false,
        }
    }

    fn applied(results: &[ApplyResult]) -> Self {
        Self {
            updated: results
                .iter()
                .filter(|result| result.result.is_ok())
                .count(),
            failed: results
                .iter()
                .filter(|result| result.result.is_err())
                .count(),
            cancelled: false,
            failures: results
                .iter()
                .filter_map(|result| {
                    result
                        .result
                        .as_ref()
                        .err()
                        .map(|error| (result.name.clone(), error.clone()))
                })
                .collect(),
            interrupted: false,
            all_current: false,
        }
    }

    pub fn render(&self) -> String {
        if self.all_current {
            return "All installed Skills are current.\n".to_owned();
        }
        if self.cancelled {
            return "Skill update cancelled.\n".to_owned();
        }
        let updated = skill_count(self.updated);
        if self.failed == 0 {
            return format!("Updated {updated}.\n");
        }
        let mut summary = format!("Updated {updated}.\n");
        for (name, error) in &self.failures {
            summary.push_str(&format!(
                "Failed Skill {name}: {}: {}\n",
                error.code, error.message
            ));
        }
        summary
    }

    /// The exit summary with the terminal theme; `render` stays plain for
    /// tests and non-terminal hosts.
    pub fn render_styled(&self, color: bool) -> String {
        let glyph =
            |symbol: &str, role: skilld_ui::Role| skilld_ui::theme::paint(symbol, role, color);
        if self.all_current {
            return format!(
                "{} All installed Skills are current.\n",
                glyph("✓", skilld_ui::Role::Success)
            );
        }
        if self.cancelled {
            return "Skill update cancelled.\n".to_owned();
        }
        let updated = skill_count(self.updated);
        let mut summary = format!(
            "{} Updated {updated}.\n",
            glyph("✓", skilld_ui::Role::Success)
        );
        for (name, error) in &self.failures {
            summary.push_str(&format!(
                "{} Failed Skill {name}: {}\n",
                glyph("✗", skilld_ui::Role::Error),
                error.message
            ));
            summary.push_str(&format!(
                "  {}\n",
                skilld_ui::theme::paint(&error.code, skilld_ui::Role::Dim, color)
            ));
        }
        summary
    }

    pub fn exit_code(&self) -> u8 {
        if self.interrupted {
            130
        } else if self.failed > 0 {
            2
        } else {
            0
        }
    }
}

fn skill_count(count: usize) -> String {
    if count == 1 {
        "1 Skill".to_owned()
    } else {
        format!("{count} Skills")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Phase {
    LoadingCandidates,
    Browsing,
    Applying,
    FailedLoad(InteractiveUpdateError),
    Finished(InteractiveUpdateSummary),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CommitState {
    NotRequested,
    Loading,
    Ready(CommitPage),
    Failed(InteractiveUpdateError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UpdateRow {
    candidate: UpdateCandidate,
    selected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Model {
    tab: Tab,
    phase: Phase,
    rows: Vec<UpdateRow>,
    commits: BTreeMap<ComparisonId, CommitState>,
    cursor: usize,
    viewport: usize,
    width: u16,
    height: u16,
    expanded_help: bool,
    spinner: usize,
}

impl Model {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            tab: Tab::Outdated,
            phase: Phase::LoadingCandidates,
            rows: Vec::new(),
            commits: BTreeMap::new(),
            cursor: 0,
            viewport: 0,
            width: width.max(1),
            height: height.max(1),
            expanded_help: false,
            spinner: 0,
        }
    }

    pub const fn tab(&self) -> Tab {
        self.tab
    }

    pub fn comparison_ids(&self) -> Vec<ComparisonId> {
        self.rows
            .iter()
            .filter_map(|row| row.candidate.comparison_id().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn selected_names(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| row.selected)
            .map(|row| row.candidate.name().to_owned())
            .collect()
    }

    fn visible_outdated_rows(&self) -> usize {
        let body_height = usize::from(self.height.saturating_sub(7).max(1));
        if self.width >= 60 {
            (body_height / 2).max(1)
        } else {
            body_height
        }
    }

    fn keep_cursor_visible(&mut self) {
        let visible = self.visible_outdated_rows();
        if self.cursor < self.viewport {
            self.viewport = self.cursor;
        } else if self.cursor >= self.viewport.saturating_add(visible) {
            self.viewport = self.cursor.saturating_add(1).saturating_sub(visible);
        }
    }

    fn keep_commit_viewport_valid(&mut self) {
        let visible = usize::from(self.height.saturating_sub(9).max(1));
        let maximum = self.commit_line_count().saturating_sub(visible);
        self.viewport = self.viewport.min(maximum);
    }

    fn commit_line_count(&self) -> usize {
        let mut rendered = BTreeSet::new();
        self.rows
            .iter()
            .filter_map(|row| {
                row.candidate
                    .comparison_id()
                    .filter(|id| rendered.insert((*id).clone()))
                    .map(|id| (row, id))
            })
            .map(|(_row, id)| {
                let state_lines = match self.commits.get(id) {
                    Some(CommitState::Ready(page)) => {
                        page.commits.len() + usize::from(page.truncated)
                    }
                    _ => 1,
                };
                2 + state_lines
            })
            .sum()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition {
    pub model: Model,
    pub effects: Vec<Effect>,
}

pub const fn initial_effect() -> Effect {
    Effect::LoadCandidates
}

pub fn update(mut model: Model, message: Message) -> Transition {
    let mut effects = Vec::new();
    match message {
        Message::CandidatesLoaded(Ok(mut candidates)) => {
            candidates.sort_by(|left, right| left.name().cmp(right.name()));
            model.rows = candidates
                .into_iter()
                .map(|candidate| UpdateRow {
                    selected: candidate.is_available(),
                    candidate,
                })
                .collect();
            model.commits = model
                .comparison_ids()
                .into_iter()
                .map(|id| (id, CommitState::NotRequested))
                .collect();
            model.cursor = 0;
            model.viewport = 0;
            if model.rows.is_empty() {
                let summary = InteractiveUpdateSummary::current();
                model.phase = Phase::Finished(summary.clone());
                effects.push(Effect::Exit(summary));
            } else {
                model.phase = Phase::Browsing;
            }
        }
        Message::CandidatesLoaded(Err(error)) => {
            model.phase = Phase::FailedLoad(error);
        }
        Message::CommitsLoaded(results) => {
            for result in results {
                let CommitLoadResult {
                    comparison_id,
                    result,
                } = result;
                if let std::collections::btree_map::Entry::Occupied(mut entry) =
                    model.commits.entry(comparison_id)
                {
                    let state = match result {
                        Ok(page) if page.comparison_id == *entry.key() => CommitState::Ready(page),
                        Ok(_) => CommitState::Failed(InteractiveUpdateError::new(
                            "INVALID_UPDATE_PLAN",
                            "The commit range did not match its request.",
                        )),
                        Err(error) => CommitState::Failed(error),
                    };
                    entry.insert(state);
                }
            }
            model.keep_commit_viewport_valid();
        }
        Message::Applied(results) => {
            let summary = InteractiveUpdateSummary::applied(&results);
            model.phase = Phase::Finished(summary.clone());
            effects.push(Effect::Exit(summary));
        }
        Message::Resized { width, height } => {
            model.width = width.max(1);
            model.height = height.max(1);
            match model.tab {
                Tab::Outdated => model.keep_cursor_visible(),
                Tab::Commits => model.keep_commit_viewport_valid(),
            }
        }
        Message::Tick => {
            model.spinner = model.spinner.wrapping_add(1);
        }
        Message::Key(key) => update_key(&mut model, key, &mut effects),
    }
    Transition { model, effects }
}

fn update_key(model: &mut Model, key: KeyInput, effects: &mut Vec<Effect>) {
    if matches!(key, KeyInput::Cancel | KeyInput::Interrupt)
        && !matches!(model.phase, Phase::Finished(_))
    {
        let summary = InteractiveUpdateSummary::cancelled(key == KeyInput::Interrupt);
        if !matches!(model.phase, Phase::Applying) {
            model.phase = Phase::Finished(summary.clone());
        }
        effects.push(Effect::Exit(summary));
        return;
    }
    if key == KeyInput::Help {
        model.expanded_help = !model.expanded_help;
        return;
    }
    if let Phase::FailedLoad(_) = &model.phase {
        if key == KeyInput::Retry {
            model.phase = Phase::LoadingCandidates;
            effects.push(Effect::LoadCandidates);
        }
        return;
    }
    if !matches!(model.phase, Phase::Browsing) {
        return;
    }

    match key {
        KeyInput::NextTab | KeyInput::PreviousTab => {
            model.tab = match model.tab {
                Tab::Outdated => Tab::Commits,
                Tab::Commits => Tab::Outdated,
            };
            model.viewport = 0;
            if model.tab == Tab::Commits {
                let pending = model
                    .commits
                    .iter()
                    .filter_map(|(id, state)| {
                        matches!(state, CommitState::NotRequested).then_some(id.clone())
                    })
                    .collect::<Vec<_>>();
                for id in &pending {
                    model.commits.insert(id.clone(), CommitState::Loading);
                }
                if !pending.is_empty() {
                    effects.push(Effect::LoadCommits(pending));
                }
            }
        }
        KeyInput::Up if model.tab == Tab::Outdated => {
            model.cursor = model.cursor.saturating_sub(1);
            model.keep_cursor_visible();
        }
        KeyInput::Down if model.tab == Tab::Outdated => {
            model.cursor = model
                .cursor
                .saturating_add(1)
                .min(model.rows.len().saturating_sub(1));
            model.keep_cursor_visible();
        }
        KeyInput::Up if model.tab == Tab::Commits => {
            model.viewport = model.viewport.saturating_sub(1);
        }
        KeyInput::Down if model.tab == Tab::Commits => {
            model.viewport = model.viewport.saturating_add(1);
            model.keep_commit_viewport_valid();
        }
        KeyInput::PageUp if model.tab == Tab::Commits => {
            model.viewport = model
                .viewport
                .saturating_sub(usize::from(model.height.max(1)));
        }
        KeyInput::PageDown if model.tab == Tab::Commits => {
            model.viewport = model
                .viewport
                .saturating_add(usize::from(model.height.max(1)));
            model.keep_commit_viewport_valid();
        }
        KeyInput::Select if model.tab == Tab::Outdated => {
            if let Some(row) = model
                .rows
                .get_mut(model.cursor)
                .filter(|row| row.candidate.is_available())
            {
                row.selected = !row.selected;
            }
        }
        KeyInput::SelectAll if model.tab == Tab::Outdated => {
            for row in &mut model.rows {
                row.selected = row.candidate.is_available();
            }
        }
        KeyInput::Apply if model.tab == Tab::Outdated => {
            let names = model.selected_names();
            if !names.is_empty() {
                model.phase = Phase::Applying;
                effects.push(Effect::Apply(names));
            }
        }
        KeyInput::Retry
            if model.tab == Tab::Outdated
                && model.rows.iter().any(|row| !row.candidate.is_available()) =>
        {
            model.phase = Phase::LoadingCandidates;
            effects.push(Effect::LoadCandidates);
        }
        KeyInput::Retry if model.tab == Tab::Commits => {
            let failed = model
                .commits
                .iter()
                .filter_map(|(id, state)| {
                    matches!(state, CommitState::Failed(_)).then_some(id.clone())
                })
                .collect::<Vec<_>>();
            for id in &failed {
                model.commits.insert(id.clone(), CommitState::Loading);
            }
            if !failed.is_empty() {
                effects.push(Effect::LoadCommits(failed));
            }
        }
        _ => {}
    }
}

pub fn view(frame: &mut ratatui::Frame<'_>, model: &Model, color: bool) {
    let help_height = if model.expanded_help { 3 } else { 1 };
    let areas = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(help_height),
    ])
    .split(frame.area());

    frame.render_widget(
        Paragraph::new("skilld update").style(
            Style::default()
                .fg(theme(color, Color::Cyan))
                .add_modifier(Modifier::BOLD),
        ),
        areas[0],
    );
    render_tabs(frame, model, color, areas[1]);
    render_body(frame, model, color, areas[2]);
    frame.render_widget(Paragraph::new(status_text(model)), areas[3]);
    frame.render_widget(
        Paragraph::new(help_text(model)).style(Style::default().fg(theme(color, Color::DarkGray))),
        areas[4],
    );
}

fn render_tabs(frame: &mut ratatui::Frame<'_>, model: &Model, color: bool, area: Rect) {
    let outdated = format!(
        "Outdated {}",
        model
            .rows
            .iter()
            .filter(|row| row.candidate.is_available())
            .count()
    );
    let titles = match model.tab {
        Tab::Outdated => vec![format!("[ {outdated} ]"), "Commits".to_owned()],
        Tab::Commits => vec![outdated, "[ Commits ]".to_owned()],
    };
    let tabs = Tabs::new(titles)
        .divider("   ")
        .select(match model.tab {
            Tab::Outdated => 0,
            Tab::Commits => 1,
        })
        .highlight_style(
            Style::default()
                .fg(theme(color, Color::Cyan))
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, area);
}

fn render_body(frame: &mut ratatui::Frame<'_>, model: &Model, color: bool, area: Rect) {
    match &model.phase {
        Phase::LoadingCandidates => {
            frame.render_widget(
                Paragraph::new(format!(
                    "{} Loading outdated Skills…",
                    spinner::frame(model.spinner)
                )),
                area,
            );
        }
        Phase::FailedLoad(error) => {
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::from("The update plan could not load."),
                    Line::from(format!("{}: {}", error.code, error.message)),
                ]))
                .style(Style::default().fg(theme(color, Color::Red))),
                area,
            );
        }
        Phase::Finished(summary) => {
            frame.render_widget(Paragraph::new(summary.render()), area);
        }
        Phase::Browsing | Phase::Applying => match model.tab {
            Tab::Outdated => render_outdated(frame, model, color, area),
            Tab::Commits => render_commits(frame, model, color, area),
        },
    }
}

fn render_outdated(frame: &mut ratatui::Frame<'_>, model: &Model, color: bool, area: Rect) {
    let visible = model.visible_outdated_rows();
    let compact = model.width < 60;
    let width = usize::from(area.width.saturating_sub(4));
    let items = model
        .rows
        .iter()
        .skip(model.viewport)
        .take(visible)
        .map(|row| match &row.candidate {
            UpdateCandidate::Available {
                name,
                repository,
                locked_commit_sha,
                latest_commit_sha,
                commit_count,
                ..
            } => {
                let checked = if row.selected { "◉" } else { "◯" };
                if compact {
                    ListItem::new(truncate(&format!("{checked} {name}"), width))
                } else {
                    let first = format!("{checked} {name:<28} {repository}");
                    let commits = if *commit_count == 1 {
                        "1 commit".to_owned()
                    } else {
                        format!("{commit_count} commits")
                    };
                    let second = format!(
                        "    {} → {}  {commits}",
                        short_sha(locked_commit_sha),
                        short_sha(latest_commit_sha)
                    );
                    ListItem::new(Text::from(vec![
                        Line::from(truncate(&first, width)),
                        Line::from(truncate(&second, width)),
                    ]))
                }
            }
            UpdateCandidate::Unavailable { name, error } => {
                if compact {
                    ListItem::new(truncate(&format!("⚠ {name}: {}", error.code), width))
                } else {
                    ListItem::new(Text::from(vec![
                        Line::from(truncate(&format!("⚠ {name}: {}", error.code), width)),
                        Line::from(truncate(&format!("    {}", error.message), width)),
                    ]))
                    .style(Style::default().fg(theme(color, Color::Red)))
                }
            }
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(Block::bordered().title("Select Skills to update"))
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(theme(color, Color::Cyan))
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default().with_selected(
        (!model.rows.is_empty()).then_some(model.cursor.saturating_sub(model.viewport)),
    );
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_commits(frame: &mut ratatui::Frame<'_>, model: &Model, color: bool, area: Rect) {
    let width = usize::from(area.width.saturating_sub(4));
    let mut lines = Vec::new();
    let mut rendered = BTreeSet::new();
    for row in &model.rows {
        let UpdateCandidate::Available {
            repository,
            locked_commit_sha,
            latest_commit_sha,
            comparison_id: id,
            ..
        } = &row.candidate
        else {
            continue;
        };
        if !rendered.insert(id.clone()) {
            continue;
        }
        lines.push(Line::styled(
            truncate(
                &format!(
                    "{}  {}..{}",
                    repository,
                    short_sha(locked_commit_sha),
                    short_sha(latest_commit_sha)
                ),
                width,
            ),
            Style::default()
                .fg(theme(color, Color::Cyan))
                .add_modifier(Modifier::BOLD),
        ));
        match model.commits.get(id) {
            Some(CommitState::NotRequested) => {
                lines.push(Line::from("  Open this tab to load commits."));
            }
            Some(CommitState::Loading) => {
                lines.push(Line::from(format!(
                    "  {} Loading commits…",
                    spinner::frame(model.spinner)
                )));
            }
            Some(CommitState::Failed(error)) => {
                lines.push(Line::styled(
                    truncate(&format!("  {}: {}", error.code, error.message), width),
                    Style::default().fg(theme(color, Color::Red)),
                ));
            }
            Some(CommitState::Ready(page)) => {
                for commit in &page.commits {
                    let date = relative_time(&commit.timestamp, std::time::SystemTime::now());
                    let details = if model.width >= 100 {
                        format!("{}  {date}", commit.author)
                    } else {
                        date
                    };
                    lines.push(Line::from(truncate(
                        &format!(
                            "  {}  {}  {}",
                            short_sha(&commit.commit_sha),
                            commit.subject,
                            details
                        ),
                        width,
                    )));
                }
                if page.truncated {
                    lines.push(Line::styled(
                        truncate(
                            &format!(
                                "  Showing newest {} of {} commits. {}",
                                page.commits.len(),
                                page.total,
                                page.compare_url
                            ),
                            width,
                        ),
                        Style::default().fg(theme(color, Color::Yellow)),
                    ));
                }
            }
            None => {}
        }
        lines.push(Line::from(""));
    }
    let lines = lines.into_iter().skip(model.viewport).collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title("Repository commits")),
        area,
    );
}

fn status_text(model: &Model) -> String {
    match model.phase {
        Phase::LoadingCandidates => "Loading update plan".to_owned(),
        Phase::Applying => format!(
            "{} Updating {}…",
            spinner::frame(model.spinner),
            skill_count(model.selected_names().len())
        ),
        Phase::FailedLoad(_) => "Update plan failed".to_owned(),
        Phase::Finished(_) => String::new(),
        Phase::Browsing if model.tab == Tab::Outdated => {
            let unavailable = model
                .rows
                .iter()
                .filter(|row| !row.candidate.is_available())
                .count();
            if unavailable == 0 {
                format!("{} selected", model.selected_names().len())
            } else {
                format!(
                    "{} selected. {} unavailable",
                    model.selected_names().len(),
                    unavailable
                )
            }
        }
        Phase::Browsing => {
            let loading = model
                .commits
                .values()
                .filter(|state| matches!(state, CommitState::Loading))
                .count();
            let failed = model
                .commits
                .values()
                .filter(|state| matches!(state, CommitState::Failed(_)))
                .count();
            match (loading, failed) {
                (0, 0) => "Repository commits loaded".to_owned(),
                (_, 0) => format!("Loading {loading} commit ranges"),
                (0, _) => format!("{failed} commit ranges failed"),
                _ => format!("Loading {loading}. {failed} failed"),
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Binding {
    keys: &'static str,
    action: &'static str,
}

fn bindings(model: &Model) -> Vec<Binding> {
    let mut bindings = match (&model.phase, model.tab) {
        (Phase::FailedLoad(_), _) => vec![Binding {
            keys: "r",
            action: "retry",
        }],
        (Phase::Browsing, Tab::Outdated) => vec![
            Binding {
                keys: "↑/↓",
                action: "move",
            },
            Binding {
                keys: "space",
                action: "select",
            },
            Binding {
                keys: "a",
                action: "select all",
            },
            Binding {
                keys: "tab",
                action: "commits",
            },
            Binding {
                keys: "enter",
                action: "update",
            },
        ]
        .into_iter()
        .chain(
            model
                .rows
                .iter()
                .any(|row| !row.candidate.is_available())
                .then_some(Binding {
                    keys: "r",
                    action: "retry",
                }),
        )
        .collect(),
        (Phase::Browsing, Tab::Commits) => {
            let mut values = vec![
                Binding {
                    keys: "↑/↓",
                    action: "scroll",
                },
                Binding {
                    keys: "tab",
                    action: "outdated",
                },
            ];
            if model
                .commits
                .values()
                .any(|state| matches!(state, CommitState::Failed(_)))
            {
                values.push(Binding {
                    keys: "r",
                    action: "retry",
                });
            }
            values
        }
        _ => Vec::new(),
    };
    bindings.extend([
        Binding {
            keys: "?",
            action: "help",
        },
        Binding {
            keys: "q",
            action: "cancel",
        },
    ]);
    bindings
}

fn help_text(model: &Model) -> String {
    let concise = bindings(model)
        .into_iter()
        .map(|binding| format!("{} {}", binding.keys, binding.action))
        .collect::<Vec<_>>()
        .join("  ");
    if model.expanded_help {
        format!("{concise}\nVim keys: j/k move. Arrow keys change tabs.\nEsc and Ctrl+C cancel.")
    } else {
        concise
    }
}

fn theme(color: bool, value: Color) -> Color {
    if color { value } else { Color::Reset }
}

fn short_sha(value: &str) -> &str {
    value.get(..7).unwrap_or(value)
}

fn clean_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut used: usize = 0;
    let mut output = String::new();
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used.saturating_add(character_width) > width {
            break;
        }
        output.push(character);
        used = used.saturating_add(character_width);
    }
    output
}

pub fn render_snapshot(model: &Model, color: bool) -> String {
    let backend = TestBackend::new(model.width, model.height);
    let mut terminal = Terminal::new(backend).expect("the in-memory terminal is valid");
    terminal
        .draw(|frame| view(frame, model, color))
        .expect("the in-memory terminal can draw");
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::with_capacity(usize::from(model.height));
    for y in 0..model.height {
        let mut line = String::new();
        let mut x = 0;
        while x < model.width {
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

pub trait TerminalLifecycle {
    fn enter(&mut self) -> Result<(), InteractiveUpdateError>;

    fn restore(&mut self) -> Result<(), InteractiveUpdateError>;
}

struct RestoreGuard<L: TerminalLifecycle> {
    lifecycle: Option<L>,
}

impl<L: TerminalLifecycle> RestoreGuard<L> {
    fn enter(mut lifecycle: L) -> Result<Self, InteractiveUpdateError> {
        lifecycle.enter()?;
        Ok(Self {
            lifecycle: Some(lifecycle),
        })
    }

    fn restore(mut self) -> Result<(), InteractiveUpdateError> {
        let result = self
            .lifecycle
            .as_mut()
            .expect("the active terminal has a lifecycle")
            .restore();
        self.lifecycle = None;
        result
    }
}

impl<L: TerminalLifecycle> Drop for RestoreGuard<L> {
    fn drop(&mut self) {
        if let Some(lifecycle) = self.lifecycle.as_mut() {
            let _ = lifecycle.restore();
        }
    }
}

pub fn with_restored_terminal<L, Operation, Outcome>(
    lifecycle: L,
    operation: Operation,
) -> Result<Outcome, InteractiveUpdateError>
where
    L: TerminalLifecycle,
    Operation: FnOnce() -> Result<Outcome, InteractiveUpdateError>,
{
    let guard = RestoreGuard::enter(lifecycle)?;
    let outcome = operation();
    guard.restore()?;
    outcome
}

struct NativeTerminalLifecycle;

impl TerminalLifecycle for NativeTerminalLifecycle {
    fn enter(&mut self) -> Result<(), InteractiveUpdateError> {
        enable_raw_mode().map_err(|_| {
            InteractiveUpdateError::terminal(
                "TERMINAL_UNAVAILABLE",
                "The terminal could not enter interactive mode.",
            )
        })?;
        let mut stdout = io::stdout();
        if execute!(stdout, EnterAlternateScreen, Hide).is_err() {
            let _ = disable_raw_mode();
            return Err(InteractiveUpdateError::terminal(
                "TERMINAL_UNAVAILABLE",
                "The terminal could not enter interactive mode.",
            ));
        }
        Ok(())
    }

    fn restore(&mut self) -> Result<(), InteractiveUpdateError> {
        let mut stdout = io::stdout();
        let screen = execute!(stdout, Show, LeaveAlternateScreen);
        let raw = disable_raw_mode();
        if screen.is_err() || raw.is_err() {
            return Err(InteractiveUpdateError::terminal(
                "TERMINAL_RESTORE_FAILED",
                "Run reset if the terminal still looks incorrect.",
            ));
        }
        Ok(())
    }
}

pub fn run_interactive_update<H: InteractiveUpdateHost>(
    host: Arc<H>,
    color: bool,
) -> Result<InteractiveUpdateSummary, InteractiveUpdateError> {
    with_restored_terminal(NativeTerminalLifecycle, || {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).map_err(terminal_lost)?;
        terminal.clear().map_err(terminal_lost)?;
        run_event_loop(&mut terminal, host, color)
    })
}

pub fn require_interactive_tty(
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
) -> Result<(), InteractiveUpdateError> {
    if stdin_is_terminal && stdout_is_terminal {
        return Ok(());
    }
    Err(InteractiveUpdateError::new(
        "INTERACTIVE_TTY_REQUIRED",
        "Interactive Skill update needs terminal input and output.",
    ))
}

fn run_event_loop<H: InteractiveUpdateHost>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    host: Arc<H>,
    color: bool,
) -> Result<InteractiveUpdateSummary, InteractiveUpdateError> {
    let size = terminal.size().map_err(terminal_lost)?;
    let mut model = Model::new(size.width, size.height);
    let mut effects = VecDeque::from([initial_effect()]);
    let (sender, receiver) = mpsc::channel();
    let mut running = None;
    let mut interrupt_after_apply = false;
    loop {
        terminal
            .draw(|frame| view(frame, &model, color))
            .map_err(terminal_lost)?;

        if matches!(effects.front(), Some(Effect::Exit(_))) {
            let Some(Effect::Exit(summary)) = effects.pop_front() else {
                unreachable!("the front effect is an exit")
            };
            if running == Some(RunningEffect::Apply) {
                interrupt_after_apply |= summary.interrupted;
                continue;
            }
            return Ok(summary);
        }

        match receiver.try_recv() {
            Ok(message) => {
                running = None;
                let mut transition = update(model, message);
                if interrupt_after_apply {
                    mark_interrupted(&mut transition);
                    interrupt_after_apply = false;
                }
                model = transition.model;
                enqueue_effects(&mut effects, transition.effects);
                continue;
            }
            Err(TryRecvError::Disconnected) if running.is_some() => {
                return Err(InteractiveUpdateError::new(
                    "INTERACTIVE_WORKER_FAILED",
                    "The interactive update worker stopped.",
                ));
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }

        if running.is_none()
            && let Some(effect) = effects.pop_front()
        {
            let kind = match effect {
                Effect::Apply(_) => RunningEffect::Apply,
                Effect::LoadCandidates | Effect::LoadCommits(_) => RunningEffect::Load,
                Effect::Exit(_) => unreachable!("exit effects are handled first"),
            };
            let fallback = failed_effect(&effect);
            let host = host.clone();
            let sender = sender.clone();
            thread::spawn(move || {
                let message =
                    catch_unwind(AssertUnwindSafe(|| resolve_effect(host.as_ref(), effect)))
                        .unwrap_or(fallback);
                let _ = sender.send(message);
            });
            running = Some(kind);
            continue;
        }

        let message = if event::poll(Duration::from_millis(120)).map_err(terminal_lost)? {
            match event::read().map_err(terminal_lost)? {
                Event::Key(key) => key_input(key).map(Message::Key),
                Event::Resize(width, height) => Some(Message::Resized { width, height }),
                _ => None,
            }
        } else {
            Some(Message::Tick)
        };
        if let Some(message) = message {
            let transition = update(model, message);
            model = transition.model;
            enqueue_effects(&mut effects, transition.effects);
        }
    }
}

fn mark_interrupted(transition: &mut Transition) {
    if let Phase::Finished(summary) = &mut transition.model.phase {
        summary.interrupted = true;
    }
    for effect in &mut transition.effects {
        if let Effect::Exit(summary) = effect {
            summary.interrupted = true;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunningEffect {
    Load,
    Apply,
}

fn enqueue_effects(queue: &mut VecDeque<Effect>, effects: Vec<Effect>) {
    if effects
        .iter()
        .any(|effect| matches!(effect, Effect::Exit(_)))
    {
        queue.clear();
    }
    queue.extend(effects);
}

fn failed_effect(effect: &Effect) -> Message {
    let error = InteractiveUpdateError::new(
        "INTERACTIVE_WORKER_FAILED",
        "The interactive update worker stopped.",
    );
    match effect {
        Effect::LoadCandidates => Message::CandidatesLoaded(Err(error)),
        Effect::LoadCommits(comparisons) => Message::CommitsLoaded(
            comparisons
                .iter()
                .cloned()
                .map(|comparison| CommitLoadResult::failed(comparison, error.clone()))
                .collect(),
        ),
        Effect::Apply(names) => Message::Applied(
            names
                .iter()
                .map(|name| ApplyResult::failed(name, error.clone()))
                .collect(),
        ),
        Effect::Exit(_) => unreachable!("exit effects do not run in a worker"),
    }
}

pub fn resolve_effect<H: InteractiveUpdateHost>(host: &H, effect: Effect) -> Message {
    match effect {
        Effect::LoadCandidates => Message::CandidatesLoaded(host.load_candidates()),
        Effect::LoadCommits(ids) => {
            let mut results = host
                .load_commits(&ids)
                .into_iter()
                .map(|result| (result.comparison_id.clone(), result))
                .collect::<BTreeMap<_, _>>();
            Message::CommitsLoaded(
                ids.into_iter()
                    .map(|id| {
                        results.remove(&id).unwrap_or_else(|| {
                            CommitLoadResult::failed(
                                id.clone(),
                                InteractiveUpdateError::new(
                                    "COMPARISON_UNAVAILABLE",
                                    "The commit range returned no result.",
                                ),
                            )
                        })
                    })
                    .collect(),
            )
        }
        Effect::Apply(names) => {
            let mut results = host
                .apply(&names)
                .into_iter()
                .map(|result| (result.name.clone(), result))
                .collect::<BTreeMap<_, _>>();
            Message::Applied(
                names
                    .into_iter()
                    .map(|name| {
                        results.remove(&name).unwrap_or_else(|| {
                            ApplyResult::failed(
                                name.clone(),
                                InteractiveUpdateError::new(
                                    "UPDATE_FAILED",
                                    "The Skill update returned no result.",
                                ),
                            )
                        })
                    })
                    .collect(),
            )
        }
        Effect::Exit(_) => unreachable!("exit effects end the event loop"),
    }
}

fn key_input(event: KeyEvent) -> Option<KeyInput> {
    if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('c') {
        return Some(KeyInput::Interrupt);
    }
    match event.code {
        KeyCode::Up | KeyCode::Char('k') => Some(KeyInput::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(KeyInput::Down),
        KeyCode::PageUp => Some(KeyInput::PageUp),
        KeyCode::PageDown => Some(KeyInput::PageDown),
        KeyCode::Tab | KeyCode::Right => Some(KeyInput::NextTab),
        KeyCode::BackTab | KeyCode::Left => Some(KeyInput::PreviousTab),
        KeyCode::Char(' ') => Some(KeyInput::Select),
        KeyCode::Char('a') => Some(KeyInput::SelectAll),
        KeyCode::Enter => Some(KeyInput::Apply),
        KeyCode::Char('r') => Some(KeyInput::Retry),
        KeyCode::Char('?') => Some(KeyInput::Help),
        KeyCode::Char('q') | KeyCode::Esc => Some(KeyInput::Cancel),
        _ => None,
    }
}

fn terminal_lost(_error: io::Error) -> InteractiveUpdateError {
    InteractiveUpdateError::terminal(
        "INTERACTIVE_TTY_LOST",
        "The interactive terminal stopped responding.",
    )
}

pub fn write_static_summary(
    summary: &InteractiveUpdateSummary,
    color: bool,
    output: &mut impl Write,
) -> Result<(), InteractiveUpdateError> {
    output
        .write_all(summary.render_styled(color).as_bytes())
        .map_err(|_| {
            InteractiveUpdateError::terminal(
                "TERMINAL_UNAVAILABLE",
                "The Skill update summary could not be written.",
            )
        })
}
