//! Account commands over the skilld.dev public API: the signed-in account and
//! its settings, likes, watches, changes, imported stars, collections, and
//! tokens.
//!
//! Every operation here sends the token `skilld auth login` stored. Without
//! one, the client stops before any request and names the login step.
//! Account deletion stays on skilld.dev, behind a browser sign-in.

use serde_json::Value;
use skilld_core::api::{
    Account, AccountChanges, AccountUpdateBody, AddCollectionSkillBody, ApiList,
    CreateCollectionBody, LikedSkill, RepositoryScan, StarredRepository, Token, TokenCreateBody,
    Watch,
};
use skilld_core::{MultiSkillRef, SkillRef};
use skilld_ui::text::grouped_number;
use skilld_ui::{Detail, Line, Marker, Screen};
use url::Url;

use crate::discover::{
    ApiOutput, Paging, RegistryRef, count, curator_name, day, registry_ref, site_link, skill_list,
    skill_row, text_link,
};
use crate::output::{escape_plain, screen_message, shell_command};
use crate::remote::{ApiPage, SkilldApi};
use crate::{CommandError, CommandPlatform};

/// The most `stars.import` pages one import reads. The contract stops at 10.
const MAX_STAR_PAGES: u32 = 10;

/// `OWNER/REPOSITORY/SKILL`, the only form a like or a collection entry takes.
fn skill_target(value: &str) -> Result<(String, String, String), CommandError> {
    match registry_ref(value) {
        Ok(Some(RegistryRef::Skill {
            owner,
            repository,
            name,
        })) => Ok((owner, repository, name)),
        _ => Err(CommandError::input(format!(
            "{} is not a Skill. Give it as OWNER/REPOSITORY/SKILL, the selector skilld search prints.",
            screen_message(value)
        ))),
    }
}

/// `OWNER/REPOSITORY`.
fn repository_target(value: &str) -> Result<(String, String), CommandError> {
    match SkillRef::parse(value) {
        Ok(SkillRef::Many(MultiSkillRef::Repository { owner, repository })) => {
            Ok((owner, repository))
        }
        _ => Err(CommandError::input(format!(
            "{} is not a Repository. Give it as OWNER/REPOSITORY.",
            screen_message(value)
        ))),
    }
}

/// `@LOGIN/SLUG`.
fn collection_target(value: &str) -> Result<(String, String), CommandError> {
    match SkillRef::parse(value) {
        Ok(SkillRef::Many(MultiSkillRef::Collection { login, slug })) => Ok((login, slug)),
        _ => Err(CommandError::input(format!(
            "{} is not a collection. Give it as @LOGIN/SLUG.",
            screen_message(value)
        ))),
    }
}

/// `@LOGIN`, or a bare login.
fn login_target(value: &str) -> Result<String, CommandError> {
    let handle = if value.starts_with('@') {
        value.to_owned()
    } else {
        format!("@{value}")
    };
    match SkillRef::parse(&handle) {
        Ok(SkillRef::Many(MultiSkillRef::Curator { login })) => Ok(login),
        _ => Err(CommandError::input(format!(
            "{} is not a login. Give it as @LOGIN.",
            screen_message(value)
        ))),
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn page_hint(page: ApiPage, shown: usize, total: u64) -> Option<Line> {
    Paging {
        offset: u64::from(page.offset.unwrap_or(0)),
        shown: shown as u64,
        total,
    }
    .hint()
    .map(Line::hint)
}

// ---------------------------------------------------------------------------
// account
// ---------------------------------------------------------------------------

/// The account fields as `key=value` records. Each settings key is the key
/// `skilld account set` takes.
fn account_lines(account: &Account, origin: &Url) -> Vec<Line> {
    let field = |key: &str, value: String| Line::field_plain(format!("{key}={value}"), key, value);
    let mut lines = vec![field("login", screen_message(&account.login))];
    if let Some(name) = &account.name {
        lines.push(field("name", screen_message(name)));
    }
    if let Some(page) = site_link(origin, &account.page_url) {
        lines.push(field("page", page));
    }
    lines.push(field(
        "email",
        account
            .email
            .as_deref()
            .map_or_else(|| "none".to_owned(), screen_message),
    ));
    lines.push(field("digest", on_off(account.digest).to_owned()));
    lines.push(field("weekly", on_off(account.weekly).to_owned()));
    lines.push(field(
        "likes-public",
        on_off(account.likes_public).to_owned(),
    ));
    lines.push(field(
        "repository-indexing",
        on_off(account.repository_indexing).to_owned(),
    ));
    lines.push(field(
        "stars-imported",
        account
            .stars_imported_at
            .as_deref()
            .map_or_else(|| "never".to_owned(), day),
    ));
    lines
}

/// `skilld account`
pub(crate) fn view(api: &dyn SkilldApi) -> Result<ApiOutput, CommandError> {
    let answer = api.account().map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let mut lines = account_lines(&answer.data, &origin);
    lines.push(Line::hint(
        "Change a setting with skilld account set KEY VALUE.",
    ));
    let plain = account_lines(&answer.data, &origin)
        .iter()
        .map(|line| format!("{}\n", line.plain_text()))
        .collect();
    Ok(ApiOutput::records(
        "account",
        answer.raw,
        Screen::with_header(
            curator_name(&answer.data.login, answer.data.name.as_deref()),
            lines,
        ),
        plain,
    ))
}

/// Parse one `skilld account set KEY VALUE` pair into the update body.
pub(crate) fn setting(key: &str, value: &str) -> Result<AccountUpdateBody, CommandError> {
    let switch = || match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(CommandError::usage(
            "INVALID_REQUEST",
            format!("{key} takes on or off"),
        )),
    };
    let mut body = AccountUpdateBody::default();
    match key {
        "email" => {
            let address = value.trim();
            if address.is_empty() || !address.contains('@') || address.len() > 254 {
                return Err(CommandError::usage(
                    "INVALID_REQUEST",
                    "email takes one address, such as you@example.com",
                ));
            }
            body.email = Some(address.to_owned());
        }
        "digest" => body.digest = Some(switch()?),
        "weekly" => body.weekly = Some(switch()?),
        "likes-public" => body.likes_public = Some(switch()?),
        "repository-indexing" => body.repository_indexing = Some(switch()?),
        _ => {
            return Err(CommandError::usage(
                "INVALID_REQUEST",
                format!(
                    "unknown account setting: {}. Use email, digest, weekly, likes-public, or repository-indexing.",
                    screen_message(key)
                ),
            ));
        }
    }
    Ok(body)
}

/// `skilld account set KEY VALUE`
pub(crate) fn set(api: &dyn SkilldApi, key: &str, value: &str) -> Result<ApiOutput, CommandError> {
    let body = setting(key, value)?;
    let answer = api.update_account(&body).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let mut lines = vec![Line::success(format!("Set {key}."))];
    lines.extend(account_lines(&answer.data, &origin));
    Ok(ApiOutput::screen(
        "account set",
        answer.raw,
        Screen::new(lines),
    ))
}

/// `skilld account scan`
pub(crate) fn scan(api: &dyn SkilldApi) -> Result<ApiOutput, CommandError> {
    let answer = api.scan_repositories().map_err(CommandError::remote)?;
    let scan: &RepositoryScan = &answer.data;
    let summary = format!(
        "{} with Skills, {} indexed, {} failed.",
        count(scan.repositories_found, "Repository", "Repositories"),
        grouped_number(scan.repositories_indexed),
        grouped_number(scan.repositories_failed),
    );
    // Every outcome is an answer, so the command exits 0. The first line
    // says whether the scan read everything.
    let lines = match scan.outcome.as_str() {
        "complete" => vec![Line::success(format!(
            "Scanned your Repositories. {summary}"
        ))],
        "partial" => vec![
            Line::warn(format!("Scanned some of your Repositories. {summary}")),
            Line::hint("Run skilld account scan again later to read the rest."),
        ],
        "github-rate-limited" => vec![
            Line::warn("GitHub rate limited the scan."),
            Line::hint("Run skilld account scan again later."),
        ],
        "github-auth-failure" => vec![
            Line::warn("GitHub refused the scan."),
            Line::hint(
                "Sign in on skilld.dev again to renew the GitHub access, then run the scan again.",
            ),
        ],
        _ => vec![Line::warn(format!(
            "The scan ended with {}. {summary}",
            screen_message(&scan.outcome)
        ))],
    };
    Ok(ApiOutput::screen(
        "account scan",
        answer.raw,
        Screen::new(lines),
    ))
}

/// `skilld account unpublish OWNER/REPOSITORY`
pub(crate) fn unpublish(api: &dyn SkilldApi, repository: &str) -> Result<ApiOutput, CommandError> {
    let (owner, repository) = repository_target(repository)?;
    api.unpublish_repository(&owner, &repository)
        .map_err(CommandError::remote)?;
    Ok(ApiOutput::screen(
        "account unpublish",
        Value::Null,
        Screen::new(vec![Line::success(format!(
            "Removed every Skill of {owner}/{repository} from the registry."
        ))]),
    ))
}

/// The signed-in login, for `skilld auth status`.
pub(crate) fn signed_in_login(api: &dyn SkilldApi) -> Result<String, CommandError> {
    api.account()
        .map(|answer| answer.data.login)
        .map_err(CommandError::remote)
}

// ---------------------------------------------------------------------------
// likes
// ---------------------------------------------------------------------------

/// `skilld like OWNER/REPOSITORY/SKILL`
pub(crate) fn like(api: &dyn SkilldApi, skill: &str) -> Result<ApiOutput, CommandError> {
    let (owner, repository, name) = skill_target(skill)?;
    api.like(&owner, &repository, &name)
        .map_err(CommandError::remote)?;
    Ok(ApiOutput::screen(
        "like",
        Value::Null,
        Screen::new(vec![
            Line::success(format!("Liked {owner}/{repository}/{name}.")),
            Line::hint(format!(
                "Your digest now reports changes to it. skilld.dev watches {owner}/{repository} for you."
            )),
        ]),
    ))
}

/// `skilld unlike OWNER/REPOSITORY/SKILL`
pub(crate) fn unlike(api: &dyn SkilldApi, skill: &str) -> Result<ApiOutput, CommandError> {
    let (owner, repository, name) = skill_target(skill)?;
    api.unlike(&owner, &repository, &name)
        .map_err(CommandError::remote)?;
    Ok(ApiOutput::screen(
        "unlike",
        Value::Null,
        Screen::new(vec![Line::success(format!(
            "Removed your like of {owner}/{repository}/{name}."
        ))]),
    ))
}

/// `skilld likes [@LOGIN]`: your likes, or a curator's public likes.
pub(crate) fn likes(
    api: &dyn SkilldApi,
    login: Option<&str>,
    page: ApiPage,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let origin = api.site_origin();
    match login {
        Some(login) => {
            let login = login_target(login)?;
            let answer = api
                .curator_likes(&login, page)
                .map_err(CommandError::remote)?;
            let rows = answer
                .data
                .items
                .iter()
                .map(|skill| skill_row(skill, &origin, platform, vec![]))
                .collect::<Vec<_>>();
            Ok(skill_list(
                "likes",
                answer.raw.clone(),
                format!(
                    "Liked by @{}  {}",
                    screen_message(&login),
                    count(answer.data.total, "Skill", "Skills")
                ),
                &rows,
                Some(Paging {
                    offset: u64::from(page.offset.unwrap_or(0)),
                    shown: answer.data.items.len() as u64,
                    total: answer.data.total,
                }),
            ))
        }
        None => {
            let answer = api.likes(page).map_err(CommandError::remote)?;
            let list: &ApiList<LikedSkill> = &answer.data;
            let rows = list
                .items
                .iter()
                .map(|skill| {
                    let (line, plain) = skill_row(
                        &skill.summary,
                        &origin,
                        platform,
                        vec![Detail::plain("Liked", day(&skill.liked_at))],
                    );
                    (line, format!("{plain}\t{}", day(&skill.liked_at)))
                })
                .collect::<Vec<_>>();
            Ok(skill_list(
                "likes",
                answer.raw.clone(),
                format!("Your likes  {}", count(list.total, "Skill", "Skills")),
                &rows,
                Some(Paging {
                    offset: u64::from(page.offset.unwrap_or(0)),
                    shown: list.items.len() as u64,
                    total: list.total,
                }),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// watches
// ---------------------------------------------------------------------------

/// `skilld watch OWNER/REPOSITORY` or `skilld watch @LOGIN/SLUG`
pub(crate) fn watch(api: &dyn SkilldApi, reference: &str) -> Result<ApiOutput, CommandError> {
    match SkillRef::parse(reference) {
        Ok(SkillRef::Many(MultiSkillRef::Repository { owner, repository })) => {
            api.watch(&owner, &repository)
                .map_err(CommandError::remote)?;
            Ok(ApiOutput::screen(
                "watch",
                Value::Null,
                Screen::new(vec![
                    Line::success(format!("Watching {owner}/{repository}.")),
                    Line::hint("Your digest reports changes to its Skills."),
                ]),
            ))
        }
        Ok(SkillRef::Many(MultiSkillRef::Collection { login, slug })) => {
            let answer = api
                .watch_collection(&login, &slug)
                .map_err(CommandError::remote)?;
            Ok(ApiOutput::screen(
                "watch",
                answer.raw,
                Screen::new(vec![
                    Line::success(format!(
                        "Watching {} of @{login}/{slug}.",
                        count(answer.data.watched, "Repository", "Repositories")
                    )),
                    Line::hint("A Repository the curator adds later needs this command again."),
                ]),
            ))
        }
        _ => Err(CommandError::input(format!(
            "{} cannot be watched. Give a Repository as OWNER/REPOSITORY, or a collection as @LOGIN/SLUG.",
            screen_message(reference)
        ))),
    }
}

/// `skilld unwatch OWNER/REPOSITORY`
pub(crate) fn unwatch(api: &dyn SkilldApi, reference: &str) -> Result<ApiOutput, CommandError> {
    if matches!(
        SkillRef::parse(reference),
        Ok(SkillRef::Many(MultiSkillRef::Collection { .. }))
    ) {
        return Err(CommandError::input(
            "skilld.dev watches the Repositories of a collection, not the collection. Run skilld watches, then skilld unwatch each OWNER/REPOSITORY.",
        ));
    }
    let (owner, repository) = repository_target(reference)?;
    api.unwatch(&owner, &repository)
        .map_err(CommandError::remote)?;
    Ok(ApiOutput::screen(
        "unwatch",
        Value::Null,
        Screen::new(vec![
            Line::success(format!("Stopped watching {owner}/{repository}.")),
            Line::hint("Your likes stay."),
        ]),
    ))
}

/// Why one watch exists, in words.
fn watch_reason(reason: &str) -> String {
    match reason {
        "direct" => "you watched it".to_owned(),
        "like" => "you liked one of its Skills".to_owned(),
        "star-import" => "you starred it on GitHub".to_owned(),
        "collection" => "you watched a collection that names it".to_owned(),
        other => screen_message(other),
    }
}

/// `skilld watches`
pub(crate) fn watches(
    api: &dyn SkilldApi,
    page: ApiPage,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api.watches(page).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let list: &ApiList<Watch> = &answer.data;
    let mut lines = Vec::new();
    let mut plain = String::new();
    for watch in &list.items {
        let reference = format!("{}/{}", watch.owner, watch.repository);
        let mut details = vec![Detail::plain("Why", watch_reason(&watch.reason))];
        if let Some(page) = site_link(&origin, &watch.page_url) {
            details.push(Detail::path("Page", page));
        }
        details.push(Detail::command(
            "Stop",
            shell_command(
                &["skilld".to_owned(), "unwatch".to_owned(), reference.clone()],
                platform,
            ),
        ));
        let record = [
            escape_plain(&reference),
            escape_plain(&watch.reason),
            day(&watch.watched_at),
        ]
        .join("\t");
        plain.push_str(&record);
        plain.push('\n');
        lines.push(Line::record(
            Marker::Note,
            record,
            screen_message(&reference),
            Some(format!("since {}", day(&watch.watched_at))),
            details,
        ));
    }
    if lines.is_empty() {
        lines.push(Line::plain(
            "You watch no Repositories. Run skilld watch OWNER/REPOSITORY to start.",
        ));
    }
    lines.extend(page_hint(page, list.items.len(), list.total));
    Ok(ApiOutput::records(
        "watches",
        answer.raw,
        Screen::with_header(
            format!(
                "Watching  {}",
                count(list.total, "Repository", "Repositories")
            ),
            lines,
        ),
        plain,
    ))
}

// ---------------------------------------------------------------------------
// changes
// ---------------------------------------------------------------------------

/// `YYYY-MM-DD` becomes the start of that UTC day. A full timestamp passes
/// through for skilld.dev to check.
pub(crate) fn since_value(value: &str) -> Result<String, CommandError> {
    let value = value.trim();
    let date = value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    if date {
        return Ok(format!("{value}T00:00:00Z"));
    }
    if value.len() >= 20 && value.as_bytes().get(10) == Some(&b'T') {
        return Ok(value.to_owned());
    }
    Err(CommandError::usage(
        "INVALID_REQUEST",
        "--since takes a date such as 2026-09-01, or a timestamp such as 2026-09-01T00:00:00Z",
    ))
}

/// `skilld changes [--since DATE]`
pub(crate) fn changes(
    api: &dyn SkilldApi,
    since: Option<&str>,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let since = since.map(since_value).transpose()?;
    let answer = api
        .changes(since.as_deref())
        .map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let window: &AccountChanges = &answer.data;
    let mut rows = Vec::with_capacity(window.items.len());
    for skill in &window.items {
        let mut details = vec![Detail::plain(
            "Changed",
            format!(
                "{} on {}",
                count(skill.change_count, "change", "changes"),
                day(&skill.changed_at)
            ),
        )];
        details.extend(
            skill
                .commit_messages
                .iter()
                .take(3)
                .map(|message| Detail::plain("Commit", screen_message(message))),
        );
        if let Some(url) = text_link(&skill.change_url) {
            details.push(Detail::path("Diff", url));
        }
        let (line, plain) = skill_row(&skill.summary, &origin, platform, details);
        rows.push((
            line,
            format!(
                "{plain}\t{}\t{}\t{}",
                day(&skill.changed_at),
                skill.change_count,
                escape_plain(&text_link(&skill.change_url).unwrap_or_default())
            ),
        ));
    }
    let mut output = skill_list(
        "changes",
        answer.raw.clone(),
        format!(
            "Changes since {}  {}",
            day(&window.since),
            count(window.items.len() as u64, "Skill", "Skills")
        ),
        &rows,
        None,
    );
    output.human.lines.push(Line::hint(format!(
        "Run skilld changes --since {} next time to read only newer changes.",
        screen_message(&window.until)
    )));
    Ok(output)
}

// ---------------------------------------------------------------------------
// stars
// ---------------------------------------------------------------------------

/// `skilld stars`: your starred Repositories that hold Skills.
pub(crate) fn stars(
    api: &dyn SkilldApi,
    page: ApiPage,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let answer = api.stars(page).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let list: &ApiList<StarredRepository> = &answer.data;
    let mut lines = Vec::new();
    let mut plain = String::new();
    for star in &list.items {
        let reference = format!("{}/{}", star.owner, star.repository);
        let mut details = Vec::new();
        if let Some(page) = site_link(&origin, &star.page_url) {
            details.push(Detail::path("Page", page));
        }
        details.push(Detail::command(
            "List",
            shell_command(
                &["skilld".to_owned(), "run".to_owned(), reference.clone()],
                platform,
            ),
        ));
        if !star.watching {
            details.push(Detail::command(
                "Watch",
                shell_command(
                    &["skilld".to_owned(), "watch".to_owned(), reference.clone()],
                    platform,
                ),
            ));
        }
        let record = [
            escape_plain(&reference),
            star.skill_count.to_string(),
            on_off(star.watching).to_owned(),
            day(&star.starred_at),
        ]
        .join("\t");
        plain.push_str(&record);
        plain.push('\n');
        lines.push(Line::record(
            Marker::Note,
            record,
            screen_message(&reference),
            Some(format!(
                "{}{}",
                count(star.skill_count, "Skill", "Skills"),
                if star.watching { " · watching" } else { "" }
            )),
            details,
        ));
    }
    if lines.is_empty() {
        lines.push(Line::plain(
            "No starred Repository holds Skills yet. Run skilld stars import to read GitHub again.",
        ));
    }
    lines.extend(page_hint(page, list.items.len(), list.total));
    Ok(ApiOutput::records(
        "stars",
        answer.raw,
        Screen::with_header(
            format!(
                "Starred  {}",
                count(list.total, "Repository", "Repositories")
            ),
            lines,
        ),
        plain,
    ))
}

/// `skilld stars import`: import every page of GitHub stars, one request at
/// a time, until skilld.dev names no next page.
pub(crate) fn import_stars(api: &dyn SkilldApi) -> Result<ApiOutput, CommandError> {
    let mut answer = api.import_stars(None).map_err(CommandError::remote)?;
    let mut pages = 1;
    while let Some(next) = answer.data.next_page {
        if pages == MAX_STAR_PAGES || next <= answer.data.page {
            break;
        }
        answer = api.import_stars(Some(next)).map_err(CommandError::remote)?;
        pages += 1;
    }
    let import = &answer.data;
    let mut lines = vec![Line::success(format!(
        "Imported {}. {} hold Skills.",
        count(
            import.imported,
            "starred Repository",
            "starred Repositories"
        ),
        grouped_number(import.with_skills)
    ))];
    if import.next_page.is_some() {
        lines.push(Line::warn(
            "skilld.dev stopped before the last page of your stars.",
        ));
    }
    lines.push(Line::hint("Run skilld stars to list them."));
    Ok(ApiOutput::screen(
        "stars import",
        answer.raw,
        Screen::new(lines),
    ))
}

// ---------------------------------------------------------------------------
// collections
// ---------------------------------------------------------------------------

/// `skilld collection create SLUG --title TITLE [--description TEXT]`
pub(crate) fn create_collection(
    api: &dyn SkilldApi,
    slug: &str,
    title: &str,
    description: Option<&str>,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let slug = slug.trim().trim_start_matches('@');
    let body = CreateCollectionBody {
        slug: slug.to_owned(),
        title: title.trim().to_owned(),
        description: description
            .map(str::trim)
            .filter(|description| !description.is_empty())
            .map(str::to_owned),
        skills: vec![],
    };
    let answer = api.create_collection(&body).map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let collection = &answer.data;
    let reference = format!("@{}/{}", collection.curator.login, collection.slug);
    let mut lines = vec![Line::success(format!(
        "Created collection {}.",
        screen_message(&reference)
    ))];
    if let Some(page) = site_link(&origin, &collection.page_url) {
        lines.push(Line::linked_field("Page", page.clone(), page));
    }
    lines.push(Line::hint(format!(
        "Add a Skill with {}.",
        shell_command(
            &[
                "skilld".to_owned(),
                "collection".to_owned(),
                "add".to_owned(),
                reference,
                "OWNER/REPOSITORY/SKILL".to_owned(),
            ],
            platform
        )
    )));
    Ok(ApiOutput::screen(
        "collection create",
        answer.raw,
        Screen::new(lines),
    ))
}

/// `skilld collection add @LOGIN/SLUG OWNER/REPOSITORY/SKILL [--reason TEXT]`
pub(crate) fn add_to_collection(
    api: &dyn SkilldApi,
    collection: &str,
    skill: &str,
    reason: Option<&str>,
    platform: CommandPlatform,
) -> Result<ApiOutput, CommandError> {
    let (login, slug) = collection_target(collection)?;
    let (owner, repository, name) = skill_target(skill)?;
    let body = AddCollectionSkillBody {
        reason: reason
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned),
    };
    let answer = api
        .add_collection_skill((&login, &slug), (&owner, &repository, &name), &body)
        .map_err(CommandError::remote)?;
    let origin = api.site_origin();
    let reason = answer
        .data
        .reason
        .as_deref()
        .map(|reason| vec![Detail::plain("Why", screen_message(reason))])
        .unwrap_or_default();
    let lines = vec![
        Line::success(format!(
            "Added {owner}/{repository}/{name} to @{login}/{slug}."
        )),
        skill_row(&answer.data.summary, &origin, platform, reason).0,
    ];
    Ok(ApiOutput::screen(
        "collection add",
        answer.raw,
        Screen::new(lines),
    ))
}

/// `skilld collection remove @LOGIN/SLUG OWNER/REPOSITORY/SKILL`
pub(crate) fn remove_from_collection(
    api: &dyn SkilldApi,
    collection: &str,
    skill: &str,
) -> Result<ApiOutput, CommandError> {
    let (login, slug) = collection_target(collection)?;
    let (owner, repository, name) = skill_target(skill)?;
    api.remove_collection_skill((&login, &slug), (&owner, &repository, &name))
        .map_err(CommandError::remote)?;
    Ok(ApiOutput::screen(
        "collection remove",
        Value::Null,
        Screen::new(vec![Line::success(format!(
            "Removed {owner}/{repository}/{name} from @{login}/{slug}."
        ))]),
    ))
}

// ---------------------------------------------------------------------------
// tokens
// ---------------------------------------------------------------------------

fn token_kind(kind: &str) -> String {
    match kind {
        "oauth" => "skilld auth login".to_owned(),
        "pat" => "created token".to_owned(),
        "oidc" => "GitHub Actions".to_owned(),
        other => screen_message(other),
    }
}

/// `skilld tokens`
pub(crate) fn tokens(api: &dyn SkilldApi, page: ApiPage) -> Result<ApiOutput, CommandError> {
    let answer = api.tokens(page).map_err(CommandError::remote)?;
    let list: &ApiList<Token> = &answer.data;
    let mut lines = Vec::new();
    let mut plain = String::new();
    for token in &list.items {
        let label = token
            .label
            .as_deref()
            .map_or_else(|| "no label".to_owned(), screen_message);
        let expires = token
            .expires_at
            .as_deref()
            .map_or_else(|| "never".to_owned(), day);
        let details = vec![
            Detail::plain("Kind", token_kind(&token.kind)),
            Detail::plain("Created", day(&token.created_at)),
            Detail::plain("Last used", day(&token.last_used_at)),
            Detail::plain("Expires", expires.clone()),
        ];
        let record = [
            token.id.to_string(),
            escape_plain(token.label.as_deref().unwrap_or_default()),
            escape_plain(&token.kind),
            day(&token.last_used_at),
            expires,
            if token.current { "current" } else { "" }.to_owned(),
        ]
        .join("\t");
        plain.push_str(&record);
        plain.push('\n');
        lines.push(Line::record(
            Marker::Note,
            record,
            format!("{}  {label}", token.id),
            token.current.then(|| "this sign-in".to_owned()),
            details,
        ));
    }
    if lines.is_empty() {
        lines.push(Line::plain("No tokens work for this account."));
    }
    lines.extend(page_hint(page, list.items.len(), list.total));
    Ok(ApiOutput::records(
        "tokens",
        answer.raw,
        Screen::with_header(format!("Tokens  {}", list.total), lines),
        plain,
    ))
}

/// `skilld tokens create --label LABEL [--ttl-days N]`. The answer holds the
/// only copy of the secret, so it prints once.
pub(crate) fn create_token(
    api: &dyn SkilldApi,
    label: &str,
    ttl_days: Option<u32>,
) -> Result<ApiOutput, CommandError> {
    let body = TokenCreateBody {
        label: label.trim().to_owned(),
        ttl_days,
    };
    let answer = api.create_token(&body).map_err(CommandError::remote)?;
    let issued = &answer.data;
    let expires = issued
        .expires_at
        .as_deref()
        .map_or_else(|| "never".to_owned(), day);
    let lines = vec![
        Line::success(format!(
            "Created token {} ({}).",
            issued.id,
            screen_message(&issued.label)
        )),
        Line::field("Token", screen_message(&issued.token)),
        Line::field("Expires", expires),
        Line::warn("Copy the token now. skilld.dev never shows it again."),
        Line::hint("Send it as Authorization: Bearer TOKEN."),
    ];
    Ok(ApiOutput::screen(
        "tokens create",
        answer.raw,
        Screen::new(lines),
    ))
}

/// `skilld tokens revoke ID`
pub(crate) fn revoke_token(api: &dyn SkilldApi, id: u64) -> Result<ApiOutput, CommandError> {
    api.revoke_token(id).map_err(CommandError::remote)?;
    Ok(ApiOutput::screen(
        "tokens revoke",
        Value::Null,
        Screen::new(vec![Line::success(format!(
            "Revoked token {id}. It stopped working at once."
        ))]),
    ))
}
