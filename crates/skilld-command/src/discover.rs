//! Discovery commands over the skilld.dev public API: `view` of a registry
//! ref, `browse`, `trending`, `tracks`, `curators`, and `index`.
//!
//! Every command builds both renderings from one typed answer. `--json`
//! carries the answer exactly as skilld.dev sent it. Human output shows the
//! provenance first, and the run command beside every Skill. Plain output is
//! one tab-separated record per row.
//!
//! Remote text never reaches the terminal raw: each string passes through
//! `screen_message` for Human output and `escape_plain` for Plain output. A
//! skilld.dev page URL prints only when it stays on the configured origin.

use serde_json::Value;
use skilld_core::api::{
    ApiList, CollectionDetail, CuratorDetail, CuratorSummary, IndexProgress, IndexRequest,
    RepositoryProfile, SkillDetail, SkillSummary, TrackDetail, TrackSummary, TrendingSignal,
    TrendingSkill,
};
use skilld_core::{MultiSkillRef, RemoteSelector, SkillRef, SourceSelector};
use skilld_ui::text::{grouped_number, is_unsafe_terminal};
use skilld_ui::{Detail, Line, Marker, Screen};
use url::Url;

use crate::output::{escape_plain, screen_message, shell_command};
use crate::provenance::RemoteProvenance;
use crate::remote::{ApiPage, BrowseQuery, INDEX_POLL_ATTEMPTS, SkilldApi, TrendingWindow};
use crate::{CommandError, CommandPlatform};

/// One API command result: the answer for `--json`, and both text renderings.
#[derive(Clone, Debug)]
pub(crate) struct ApiOutput {
    /// The `command` field of the JSON envelope.
    pub command: &'static str,
    /// The answer exactly as skilld.dev sent it.
    pub data: Value,
    pub human: Screen,
    pub plain: String,
}

impl ApiOutput {
    /// An output whose Plain rendering is the Human screen without glyphs.
    pub(crate) fn screen(command: &'static str, data: Value, human: Screen) -> Self {
        let plain = human.render_plain();
        Self {
            command,
            data,
            human,
            plain,
        }
    }

    /// An output whose Plain rendering is one record per row.
    pub(crate) fn records(
        command: &'static str,
        data: Value,
        human: Screen,
        plain: String,
    ) -> Self {
        Self {
            command,
            data,
            human,
            plain,
        }
    }
}

/// A registry ref `skilld view` reads from skilld.dev.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RegistryRef {
    Skill {
        owner: String,
        repository: String,
        name: String,
    },
    Repository {
        owner: String,
        repository: String,
    },
    Curator {
        login: String,
    },
    Collection {
        login: String,
        slug: String,
    },
}

/// Whether one `skilld view` argument reads the registry. An installed Skill
/// name never contains `/` or `@`.
pub(crate) fn is_registry_ref(value: &str) -> bool {
    value.contains('/') || value.starts_with('@')
}

/// Sort one `skilld view` argument into an installed Skill name or a
/// registry ref.
///
/// An installed Skill name never contains `/` or `@`, so any value with
/// either is a registry ref. `None` keeps the installed Skill path.
pub(crate) fn registry_ref(value: &str) -> Result<Option<RegistryRef>, CommandError> {
    if !is_registry_ref(value) {
        return Ok(None);
    }
    let guidance = "skilld view takes an installed Skill name, OWNER/REPOSITORY/SKILL, OWNER/REPOSITORY, @LOGIN, or @LOGIN/SLUG.";
    match SkillRef::parse(value).map_err(CommandError::remote)? {
        SkillRef::Many(MultiSkillRef::Repository { owner, repository }) => {
            Ok(Some(RegistryRef::Repository { owner, repository }))
        }
        SkillRef::Many(MultiSkillRef::Curator { login }) => {
            Ok(Some(RegistryRef::Curator { login }))
        }
        SkillRef::Many(MultiSkillRef::Collection { login, slug }) => {
            Ok(Some(RegistryRef::Collection { login, slug }))
        }
        SkillRef::Skill(source) => {
            let selector = RemoteSelector::parse(&source)
                .map_err(|error| CommandError::input(format!("{} {guidance}", error.message)))?;
            match selector {
                RemoteSelector::Skilld(request) if request.r#ref.is_none() => {
                    match request.selector {
                        SourceSelector::NamedSkill { name } => Ok(Some(RegistryRef::Skill {
                            owner: request.owner,
                            repository: request.repository,
                            name,
                        })),
                        SourceSelector::Path { .. } => Err(CommandError::input(format!(
                            "{source} names a path inside the Repository. {guidance}"
                        ))),
                    }
                }
                _ => Err(CommandError::input(format!(
                    "skilld view reads the registry, not a pinned or GitHub source. {guidance}"
                ))),
            }
        }
    }
}

/// A skilld.dev page URL the output may print: one on the configured origin,
/// with no query, fragment, or terminal control.
pub(crate) fn site_link(origin: &Url, value: &str) -> Option<String> {
    if value.chars().any(is_unsafe_terminal) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    (url.origin() == origin.origin() && url.query().is_none() && url.fragment().is_none())
        .then(|| url.into())
}

/// A GitHub URL the output may print and link.
pub(crate) fn github_link(value: &str) -> Option<String> {
    if value.chars().any(is_unsafe_terminal) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.fragment().is_none())
    .then(|| url.into())
}

/// Any HTTPS URL the output may print as text, such as a post on X.
pub(crate) fn text_link(value: &str) -> Option<String> {
    if value.chars().any(is_unsafe_terminal) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    (url.scheme() == "https" && url.username().is_empty() && url.password().is_none())
        .then(|| url.into())
}

/// The `OWNER/REPOSITORY/SKILL` selector of one registry Skill, when the
/// registry name is one `skilld run` accepts.
pub(crate) fn skill_selector(owner: &str, repository: &str, name: &str) -> Option<String> {
    let selector = format!("{owner}/{repository}/{name}");
    matches!(
        RemoteSelector::parse(&selector),
        Ok(RemoteSelector::Skilld(request))
            if matches!(request.selector, SourceSelector::NamedSkill { .. })
    )
    .then_some(selector)
}

/// `skilld run SELECTOR`, quoted for the platform.
pub(crate) fn run_command(selector: &str, platform: CommandPlatform) -> String {
    shell_command(
        &["skilld".to_owned(), "run".to_owned(), selector.to_owned()],
        platform,
    )
}

/// `skilld install SELECTOR`, quoted for the platform.
pub(crate) fn install_command(selector: &str, platform: CommandPlatform) -> String {
    shell_command(
        &[
            "skilld".to_owned(),
            "install".to_owned(),
            selector.to_owned(),
        ],
        platform,
    )
}

/// `2026-09-28` from an ISO 8601 timestamp, or the sanitized value.
pub(crate) fn day(value: &str) -> String {
    let date = value.get(..10).unwrap_or(value);
    let valid = date.len() == 10
        && date.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    if valid {
        date.to_owned()
    } else {
        screen_message(value)
    }
}

pub(crate) fn count(value: u64, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        grouped_number(value),
        if value == 1 { one } else { many }
    )
}

/// One Skill card as a Human record and a Plain record.
///
/// Plain: `name`, `selector`, `owner/repository`, stars, likes, description,
/// and Skill page, separated by tabs.
pub(crate) fn skill_row(
    skill: &SkillSummary,
    origin: &Url,
    platform: CommandPlatform,
    extra: Vec<Detail>,
) -> (Line, String) {
    let selector = skill_selector(&skill.owner, &skill.repository, &skill.name);
    let page = site_link(origin, &skill.page_url);
    let slug = format!("{}/{}", skill.owner, skill.repository);
    let status = format!(
        "{} · {} · {}",
        screen_message(&slug),
        count(skill.stars, "star", "stars"),
        count(skill.likes, "like", "likes")
    );
    let mut details = Vec::new();
    if let Some(description) = &skill.description {
        details.push(Detail::plain("About", screen_message(description)));
    }
    details.extend(extra);
    if let Some(selector) = &selector {
        details.push(Detail::command("Run", run_command(selector, platform)));
    }
    let title = screen_message(&skill.name);
    let plain = [
        escape_plain(&skill.name),
        escape_plain(selector.as_deref().unwrap_or_default()),
        escape_plain(&slug),
        skill.stars.to_string(),
        skill.likes.to_string(),
        escape_plain(skill.description.as_deref().unwrap_or_default()),
        escape_plain(page.as_deref().unwrap_or_default()),
    ]
    .join("\t");
    (
        Line::record(Marker::Note, plain.clone(), title, Some(status), details),
        plain,
    )
}

/// A list of Skill cards with a paging hint.
pub(crate) fn skill_list(
    command: &'static str,
    raw: Value,
    header: String,
    skills: &[(Line, String)],
    paging: Option<Paging>,
) -> ApiOutput {
    let mut lines = skills
        .iter()
        .map(|(line, _)| line.clone())
        .collect::<Vec<_>>();
    if skills.is_empty() {
        lines.push(Line::plain("No Skills found."));
    }
    lines.extend(paging.and_then(|paging| paging.hint()).map(Line::hint));
    let plain = skills
        .iter()
        .map(|(_, plain)| format!("{plain}\n"))
        .collect::<String>();
    ApiOutput::records(command, raw, Screen::with_header(header, lines), plain)
}

/// Where one page sits in the whole result.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Paging {
    pub offset: u64,
    pub shown: u64,
    pub total: u64,
}

impl Paging {
    pub(crate) fn hint(self) -> Option<String> {
        let end = self.offset + self.shown;
        (end < self.total && self.shown > 0).then(|| {
            format!(
                "Showing {} to {end} of {}. Add --offset {end} for the next page.",
                self.offset + 1,
                self.total
            )
        })
    }
}

/// `skilld view OWNER/REPOSITORY/SKILL`: one Skill with its provenance.
pub(crate) fn view_skill(
    api: &dyn SkilldApi,
    owner: &str,
    repository: &str,
    name: &str,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api
        .skill(owner, repository, name)
        .map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let lines = skill_detail_lines(&answer.data, &origin, platform)?;
    Ok(ApiOutput::screen(
        "view",
        answer.raw,
        Screen::with_header(screen_message(&answer.data.summary.name), lines),
    ))
}

fn skill_detail_lines(
    skill: &SkillDetail,
    origin: &Url,
    platform: CommandPlatform,
) -> Result<Vec<Line>, CommandError> {
    let summary = &skill.summary;
    let mut lines = vec![Line::field("Name", screen_message(&summary.name))];
    if summary.display_name != summary.name {
        lines.push(Line::field("Title", screen_message(&summary.display_name)));
    }
    let author = match &skill.author_name {
        Some(name) if name != &summary.owner => {
            format!(
                "{} ({})",
                screen_message(&summary.owner),
                screen_message(name)
            )
        }
        _ => screen_message(&summary.owner),
    };
    lines.push(Line::field("Author", author));
    let slug = screen_message(&format!("{}/{}", summary.owner, summary.repository));
    lines.push(match github_link(&skill.repository_url) {
        Some(url) => Line::linked_field("Repository", slug, url),
        None => Line::field("Repository", slug),
    });
    if let Some(source) = read_it_first(skill)? {
        lines.push(Line::linked_field("Read it first", source.clone(), source));
    }
    if let Some(commit) = skill
        .source_commit
        .as_deref()
        .filter(|commit| valid_commit(commit))
    {
        lines.push(Line::field("Commit", commit.to_owned()));
    }
    if skill.source_gone {
        lines.push(Line::warn(
            "The SKILL.md is gone upstream. skilld.dev keeps the last copy it read.",
        ));
    }
    if let Some(description) = &summary.description {
        lines.push(Line::field("Description", screen_message(description)));
    }
    if let Some(generated) = &skill.generated_summary {
        lines.push(Line::field("Generated summary", screen_message(generated)));
    }
    lines.push(Line::field("Stars", grouped_number(summary.stars)));
    lines.push(Line::field("Likes", grouped_number(summary.likes)));
    if let Some(updated) = &summary.updated_at {
        lines.push(Line::field("Updated", day(updated)));
    }
    if let Some(license) = &skill.license {
        lines.push(Line::field("License", screen_message(license)));
    }
    if !skill.tags.is_empty() {
        lines.push(Line::field("Tags", screen_message(&skill.tags.join(", "))));
    }
    if !skill.allowed_tools.is_empty() {
        lines.push(Line::field(
            "Allowed tools",
            screen_message(&skill.allowed_tools.join(", ")),
        ));
    }
    if !skill.files.is_empty() {
        let files = skill
            .files
            .iter()
            .map(|file| screen_message(&file.path))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::field("Files", files));
    }
    if let Some(page) = site_link(origin, &summary.page_url) {
        lines.push(Line::linked_field("Skill page", page.clone(), page));
    }
    if let Some(selector) = skill_selector(&summary.owner, &summary.repository, &summary.name) {
        lines.push(Line::command_field("Run", run_command(&selector, platform)));
        lines.push(Line::command_field(
            "Install",
            install_command(&selector, platform),
        ));
    }
    Ok(lines)
}

/// The exact SKILL.md the registry read: built from the commit and path when
/// both are known, else the GitHub URL skilld.dev sent.
fn read_it_first(skill: &SkillDetail) -> Result<Option<String>, CommandError> {
    let summary = &skill.summary;
    if let (Some(path), Some(commit)) = (&skill.skill_path, &skill.source_commit)
        && valid_commit(commit)
    {
        let directory = path
            .strip_suffix("SKILL.md")
            .unwrap_or(path)
            .trim_end_matches('/');
        return RemoteProvenance::new(
            summary.owner.as_str(),
            summary.repository.as_str(),
            directory,
            commit.as_str(),
        )
        .map(|provenance| Some(provenance.source_url));
    }
    Ok(summary.source_url.as_deref().and_then(github_link))
}

fn valid_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `skilld view OWNER/REPOSITORY`: one Repository and its Skills.
pub(crate) fn view_repository(
    api: &dyn SkilldApi,
    owner: &str,
    repository: &str,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api
        .repository(owner, repository)
        .map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let profile: &RepositoryProfile = &answer.data;
    let slug = screen_message(&format!("{}/{}", profile.owner, profile.repository));
    let mut lines = vec![match github_link(&profile.repository_url) {
        Some(url) => Line::linked_field("Repository", slug.clone(), url),
        None => Line::field("Repository", slug.clone()),
    }];
    lines.push(Line::field("Author", screen_message(&profile.owner)));
    if let Some(description) = &profile.description {
        lines.push(Line::field("Description", screen_message(description)));
    }
    lines.push(Line::field("Stars", grouped_number(profile.stars)));
    if let Some(pushed) = &profile.pushed_at {
        lines.push(Line::field("Last push", day(pushed)));
    }
    if let Some(page) = site_link(&origin, &profile.page_url) {
        lines.push(Line::linked_field("Page", page.clone(), page));
    }
    let reference = format!("{}/{}", profile.owner, profile.repository);
    lines.push(Line::field(
        "Install all",
        shell_command(
            &["skilld".to_owned(), "add".to_owned(), reference],
            platform,
        ),
    ));
    lines.push(Line::plain(""));
    lines.push(Line::item(count(
        profile.skills.len() as u64,
        "Skill",
        "Skills",
    )));
    for skill in &profile.skills {
        lines.push(skill_row(skill, &origin, platform, vec![]).0);
    }
    Ok(ApiOutput::screen(
        "view",
        answer.raw,
        Screen::with_header(slug, lines),
    ))
}

/// `skilld browse`
pub(crate) fn browse(
    api: &dyn SkilldApi,
    query: &BrowseQuery,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api.browse(query).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let list: &ApiList<SkillSummary> = &answer.data;
    let rows = list
        .items
        .iter()
        .map(|skill| skill_row(skill, &origin, platform, vec![]))
        .collect::<Vec<_>>();
    let header = match &query.text {
        Some(text) => format!("Browse  {}", screen_message(text)),
        None => "Browse".to_owned(),
    };
    Ok(skill_list(
        "browse",
        answer.raw,
        format!("{header}  {}", count(list.total, "Skill", "Skills")),
        &rows,
        Some(Paging {
            offset: u64::from(query.page.offset.unwrap_or(0)),
            shown: list.items.len() as u64,
            total: list.total,
        }),
    ))
}

/// `skilld trending`: each row says why the Skill trends.
pub(crate) fn trending(
    api: &dyn SkilldApi,
    window: Option<TrendingWindow>,
    limit: Option<u32>,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api.trending(window, limit).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let board: &ApiList<TrendingSkill> = &answer.data;
    let mut rows = Vec::with_capacity(board.items.len());
    for skill in &board.items {
        let (line, plain) = skill_row(
            &skill.summary,
            &origin,
            platform,
            trending_reason(&skill.signal),
        );
        rows.push((
            line,
            format!("{plain}\t{}", escape_plain(&signal_record(&skill.signal))),
        ));
    }
    let window = window.unwrap_or(TrendingWindow::Week);
    Ok(skill_list(
        "trending",
        answer.raw,
        format!("Trending this {}", window.as_str()),
        &rows,
        None,
    ))
}

/// The Human lines that say why one Skill trends.
fn trending_reason(signal: &TrendingSignal) -> Vec<Detail> {
    let post = |post: &skilld_core::api::TrendingPost, authors: u64| {
        let network = match post.platform.as_str() {
            "x" => "X",
            "bsky" => "Bluesky",
            other => other,
        };
        let mut details = vec![Detail::plain(
            "Why",
            format!(
                "@{} on {} ({} talked about it)",
                screen_message(&post.author_handle),
                screen_message(network),
                count(authors, "account", "accounts")
            ),
        )];
        details.push(Detail::plain(
            "Post",
            format!("\u{201c}{}\u{201d}", screen_message(&post.text)),
        ));
        if let Some(url) = text_link(&post.url) {
            details.push(Detail::path("Link", url));
        }
        details
    };
    let surge = |gain: u64, on: &str| {
        Detail::plain(
            "Why",
            format!("+{} on {}", count(gain, "star", "stars"), day(on)),
        )
    };
    match signal {
        TrendingSignal::Social {
            author_count,
            post: value,
            ..
        } => post(value, *author_count),
        TrendingSignal::StarSurge {
            star_gain,
            surged_on,
        } => vec![surge(*star_gain, surged_on)],
        TrendingSignal::SocialAndStarSurge {
            author_count,
            post: value,
            star_gain,
            surged_on,
            ..
        } => {
            let mut details = post(value, *author_count);
            details.insert(1, surge(*star_gain, surged_on));
            details
        }
        TrendingSignal::StarCount => vec![Detail::plain(
            "Why",
            "Well starred. Nobody posted about it in this window.",
        )],
        TrendingSignal::Unknown => vec![],
    }
}

/// The Plain reason column: `social:@handle:URL`, `star-surge:+N:DAY`,
/// `star-count`, or `unknown`.
fn signal_record(signal: &TrendingSignal) -> String {
    let social = |post: &skilld_core::api::TrendingPost| {
        format!(
            "social:@{}:{}",
            post.author_handle,
            text_link(&post.url).unwrap_or_default()
        )
    };
    match signal {
        TrendingSignal::Social { post, .. } => social(post),
        TrendingSignal::StarSurge {
            star_gain,
            surged_on,
        } => format!("star-surge:+{star_gain}:{}", day(surged_on)),
        TrendingSignal::SocialAndStarSurge {
            post,
            star_gain,
            surged_on,
            ..
        } => format!(
            "{} star-surge:+{star_gain}:{}",
            social(post),
            day(surged_on)
        ),
        TrendingSignal::StarCount => "star-count".to_owned(),
        TrendingSignal::Unknown => "unknown".to_owned(),
    }
}

/// `skilld tracks`: every track.
pub(crate) fn tracks(api: &dyn SkilldApi) -> Result<ApiOutput, CommandError> {
    let answer = api.tracks().map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let list: &ApiList<TrackSummary> = &answer.data;
    let mut lines = Vec::new();
    let mut plain = String::new();
    for track in &list.items {
        let mut details = vec![Detail::plain("About", screen_message(&track.line))];
        if let Some(page) = site_link(&origin, &track.page_url) {
            details.push(Detail::path("Page", page));
        }
        details.push(Detail::command(
            "List",
            format!("skilld tracks {}", screen_message(&track.slug)),
        ));
        let record = [
            escape_plain(&track.slug),
            escape_plain(&track.label),
            track.skill_count.to_string(),
            escape_plain(&track.line),
        ]
        .join("\t");
        plain.push_str(&record);
        plain.push('\n');
        lines.push(Line::record(
            Marker::Note,
            record,
            screen_message(&track.label),
            Some(format!(
                "{} · {}",
                screen_message(&track.slug),
                count(track.skill_count, "Skill", "Skills")
            )),
            details,
        ));
    }
    if lines.is_empty() {
        lines.push(Line::plain("No tracks found."));
    }
    Ok(ApiOutput::records(
        "tracks",
        answer.raw,
        Screen::with_header("Tracks", lines),
        plain,
    ))
}

/// `skilld tracks SLUG`: one track and one page of its Skills.
pub(crate) fn track(
    api: &dyn SkilldApi,
    slug: &str,
    page: ApiPage,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api.track(slug, page).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let detail: &TrackDetail = &answer.data;
    let rows = detail
        .items
        .iter()
        .map(|skill| skill_row(skill, &origin, platform, vec![]))
        .collect::<Vec<_>>();
    let mut output = skill_list(
        "tracks",
        answer.raw.clone(),
        format!(
            "{}  {}",
            screen_message(&detail.label),
            count(detail.total, "Skill", "Skills")
        ),
        &rows,
        Some(Paging {
            offset: u64::from(page.offset.unwrap_or(0)),
            shown: detail.items.len() as u64,
            total: detail.total,
        }),
    );
    output
        .human
        .lines
        .insert(0, Line::hint(screen_message(&detail.line)));
    Ok(output)
}

/// `skilld index REPOSITORY`: ask skilld.dev to index one Repository and
/// wait, bounded, until it lists the Skills.
pub(crate) fn index(
    api: &dyn SkilldApi,
    repository: &str,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let repository = repository_reference(repository)?;
    let mut answer = api
        .create_index_request(&repository)
        .map_err(CommandError::remote)?;
    let mut polls = 0;
    while let IndexRequest::Queued { id, .. } = &answer.data {
        if polls == INDEX_POLL_ATTEMPTS {
            break;
        }
        let id = id.clone();
        api.wait_for_index().map_err(CommandError::remote)?;
        answer = api.index_request(&id).map_err(CommandError::remote)?;
        polls += 1;
    }
    let origin = api.site_origin();
    match &answer.data {
        IndexRequest::Indexed {
            owner,
            repository,
            skills,
        } => {
            let reference = format!("{owner}/{repository}");
            let mut lines = vec![Line::success(format!(
                "skilld.dev indexed {} Skills from {}.",
                skills.len(),
                screen_message(&reference)
            ))];
            let mut plain = String::new();
            for skill in skills {
                let (line, record) = skill_row(skill, &origin, platform, vec![]);
                lines.push(line);
                plain.push_str(&record);
                plain.push('\n');
            }
            Ok(ApiOutput::records(
                "index",
                answer.raw,
                Screen::new(lines),
                plain,
            ))
        }
        IndexRequest::Queued {
            owner,
            repository,
            progress,
            ..
        } => {
            let reference = format!("{owner}/{repository}");
            let stage = match progress {
                IndexProgress::Indexing { indexed, total } => {
                    format!("It has indexed {indexed} of {total} Skills so far.")
                }
                IndexProgress::Checking => "It is reading the Repository.".to_owned(),
                IndexProgress::Queued | IndexProgress::Unknown => {
                    "The request waits for a worker.".to_owned()
                }
            };
            let lines = vec![
                Line::warn(format!(
                    "skilld.dev is still indexing {}. {stage}",
                    screen_message(&reference)
                )),
                Line::hint(format!(
                    "Run {} again to check.",
                    shell_command(
                        &["skilld".to_owned(), "index".to_owned(), reference],
                        platform
                    )
                )),
            ];
            Ok(ApiOutput::screen("index", answer.raw, Screen::new(lines)))
        }
        IndexRequest::Failed {
            owner,
            repository,
            reason,
        } => Err(CommandError::operation(
            "INDEX_FAILED",
            format!(
                "skilld.dev could not index {owner}/{repository}: {}",
                reason.trim_end_matches('.')
            ),
        )),
    }
}

/// `OWNER/REPOSITORY` or a github.com URL, the forms skilld.dev accepts.
fn repository_reference(value: &str) -> Result<String, CommandError> {
    let value = value.trim();
    let invalid = || {
        CommandError::input(
            "Give the Repository as OWNER/REPOSITORY or a https://github.com/OWNER/REPOSITORY URL.",
        )
    };
    if value.chars().any(is_unsafe_terminal) || value.len() > 2048 {
        return Err(invalid());
    }
    if let Some(rest) = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("https://www.github.com/"))
    {
        return if rest.split('/').filter(|part| !part.is_empty()).count() >= 2 {
            Ok(value.to_owned())
        } else {
            Err(invalid())
        };
    }
    match SkillRef::parse(value) {
        Ok(SkillRef::Many(MultiSkillRef::Repository { owner, repository })) => {
            Ok(format!("{owner}/{repository}"))
        }
        _ => Err(invalid()),
    }
}

/// The curator name a person reads: `@login (Name)`.
pub(crate) fn curator_name(login: &str, name: Option<&str>) -> String {
    match name {
        Some(name) if !name.trim().is_empty() && name != login => {
            format!("@{} ({})", screen_message(login), screen_message(name))
        }
        _ => format!("@{}", screen_message(login)),
    }
}

/// `skilld view @LOGIN`: one curator and their collections.
pub(crate) fn view_curator(
    api: &dyn SkilldApi,
    login: &str,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api.curator(login).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let curator: &CuratorDetail = &answer.data;
    let mut lines = vec![Line::field(
        "Curator",
        curator_name(&curator.login, curator.name.as_deref()),
    )];
    if let Some(page) = site_link(&origin, &curator.page_url) {
        lines.push(Line::linked_field("Page", page.clone(), page));
    }
    let handle = format!("@{}", curator.login);
    lines.push(Line::field(
        "List every Skill",
        shell_command(
            &["skilld".to_owned(), "run".to_owned(), handle.clone()],
            platform,
        ),
    ));
    lines.push(Line::field(
        "Install all",
        shell_command(&["skilld".to_owned(), "add".to_owned(), handle], platform),
    ));
    lines.push(Line::plain(""));
    lines.push(Line::item(count(
        curator.collections.len() as u64,
        "collection",
        "collections",
    )));
    if curator.collections.is_empty() {
        lines.push(Line::plain("No collections yet."));
    }
    for collection in &curator.collections {
        let reference = format!("@{}/{}", curator.login, collection.slug);
        let mut details = Vec::new();
        if let Some(description) = &collection.description {
            details.push(Detail::plain("About", screen_message(description)));
        }
        details.push(Detail::command(
            "View",
            shell_command(
                &["skilld".to_owned(), "view".to_owned(), reference.clone()],
                platform,
            ),
        ));
        let plain = [
            escape_plain(&reference),
            escape_plain(&collection.title),
            collection.skill_count.to_string(),
            escape_plain(collection.description.as_deref().unwrap_or_default()),
        ]
        .join("\t");
        lines.push(Line::record(
            Marker::Note,
            plain,
            screen_message(&collection.title),
            Some(format!(
                "{} · {}",
                screen_message(&reference),
                count(collection.skill_count, "Skill", "Skills")
            )),
            details,
        ));
    }
    Ok(ApiOutput::screen(
        "view",
        answer.raw,
        Screen::with_header(curator_name(&curator.login, curator.name.as_deref()), lines),
    ))
}

/// `skilld view @LOGIN/SLUG`: one collection and the Skills it names, each
/// with the curator's reason.
pub(crate) fn view_collection(
    api: &dyn SkilldApi,
    login: &str,
    slug: &str,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api
        .collection(login, slug, ApiPage::default())
        .map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let collection: &CollectionDetail = &answer.data;
    let reference = format!("@{}/{}", collection.curator.login, collection.slug);
    let mut lines = vec![
        Line::field("Collection", screen_message(&collection.title)),
        Line::field(
            "Curator",
            curator_name(
                &collection.curator.login,
                collection.curator.name.as_deref(),
            ),
        ),
    ];
    if let Some(description) = &collection.description {
        lines.push(Line::field("Description", screen_message(description)));
    }
    if let Some(page) = site_link(&origin, &collection.page_url) {
        lines.push(Line::linked_field("Page", page.clone(), page));
    }
    lines.push(Line::field(
        "Install all",
        shell_command(
            &["skilld".to_owned(), "add".to_owned(), reference.clone()],
            platform,
        ),
    ));
    lines.push(Line::field(
        "Watch",
        shell_command(
            &["skilld".to_owned(), "watch".to_owned(), reference],
            platform,
        ),
    ));
    lines.push(Line::plain(""));
    lines.push(Line::item(count(
        collection.skills.total,
        "Skill",
        "Skills",
    )));
    for skill in &collection.skills.items {
        let reason = skill
            .reason
            .as_deref()
            .map(|reason| vec![Detail::plain("Why", screen_message(reason))])
            .unwrap_or_default();
        lines.push(skill_row(&skill.summary, &origin, platform, reason).0);
    }
    let shown = collection.skills.items.len() as u64;
    if shown < collection.skills.total {
        lines.push(Line::hint(format!(
            "skilld.dev shows the first {shown} of {} Skills here.",
            collection.skills.total
        )));
    }
    Ok(ApiOutput::screen(
        "view",
        answer.raw,
        Screen::with_header(screen_message(&collection.title), lines),
    ))
}

/// `skilld curators`
pub(crate) fn curators(
    api: &dyn SkilldApi,
    page: ApiPage,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api.curators(page).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let list: &ApiList<CuratorSummary> = &answer.data;
    let mut lines = Vec::new();
    let mut plain = String::new();
    for curator in &list.items {
        let handle = format!("@{}", curator.login);
        let mut details = Vec::new();
        if let Some(page) = site_link(&origin, &curator.page_url) {
            details.push(Detail::path("Page", page));
        }
        details.push(Detail::command(
            "View",
            shell_command(
                &["skilld".to_owned(), "view".to_owned(), handle.clone()],
                platform,
            ),
        ));
        let record = [
            escape_plain(&handle),
            escape_plain(curator.name.as_deref().unwrap_or_default()),
            curator.collection_count.to_string(),
        ]
        .join("\t");
        plain.push_str(&record);
        plain.push('\n');
        lines.push(Line::record(
            Marker::Note,
            record,
            curator_name(&curator.login, curator.name.as_deref()),
            Some(count(curator.collection_count, "collection", "collections")),
            details,
        ));
    }
    if lines.is_empty() {
        lines.push(Line::plain("No curators found."));
    }
    lines.extend(
        Paging {
            offset: u64::from(page.offset.unwrap_or(0)),
            shown: list.items.len() as u64,
            total: list.total,
        }
        .hint()
        .map(Line::hint),
    );
    Ok(ApiOutput::records(
        "curators",
        answer.raw,
        Screen::with_header(format!("Curators  {}", list.total), lines),
        plain,
    ))
}
