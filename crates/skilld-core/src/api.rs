//! The skilld.dev public API, version 1.
//!
//! `packages/protocol/openapi/skilld-api-v1.json` vendors the contract that
//! skilld.dev generates. Each operation the CLI calls has one [`ApiOperation`]
//! here, and each answer parses into one wire type here.
//!
//! Answers are additive: skilld.dev may add a field to any object at any time.
//! So no answer type here denies unknown fields. Request bodies are the
//! opposite: the CLI writes them, so they deny unknown fields, and the contract
//! test fails when an example asks for a field the CLI cannot send.
//!
//! `tests/api_contract.rs` parses every response example in the vendored
//! contract into the type this module maps to that operation. A contract change
//! then fails a test before it fails a person.

use serde::{Deserialize, Serialize};

/// The HTTP method of one operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl ApiMethod {
    /// The method name as OpenAPI spells it, lowercase.
    pub const fn openapi_key(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Put => "put",
            Self::Patch => "patch",
            Self::Delete => "delete",
        }
    }
}

/// Who may call one operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiAccess {
    /// No credential. The answer never varies by caller.
    Public,
    /// A skilld token in `Authorization: Bearer`.
    Account,
}

/// What one successful answer carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiAnswerBody {
    /// A JSON body.
    Json,
    /// `204 No Content`.
    Empty,
}

/// One operation of the public API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApiOperation {
    /// The stable dotted identifier, such as `skills.get`.
    pub id: &'static str,
    pub method: ApiMethod,
    /// The path template, such as `/api/v1/skills/{owner}/{repository}/{name}`.
    pub path: &'static str,
    /// The query parameters the CLI may send.
    pub query: &'static [&'static str],
    pub access: ApiAccess,
    /// Whether a failed attempt may repeat. Queries and idempotent PUT and
    /// DELETE mutations may; POST and PATCH never do.
    pub retry: bool,
    pub answer: ApiAnswerBody,
}

const fn query(
    id: &'static str,
    path: &'static str,
    query: &'static [&'static str],
    access: ApiAccess,
) -> ApiOperation {
    ApiOperation {
        id,
        method: ApiMethod::Get,
        path,
        query,
        access,
        retry: true,
        answer: ApiAnswerBody::Json,
    }
}

const fn mutation(
    id: &'static str,
    method: ApiMethod,
    path: &'static str,
    access: ApiAccess,
    answer: ApiAnswerBody,
) -> ApiOperation {
    ApiOperation {
        id,
        method,
        path,
        query: &[],
        access,
        // PUT and DELETE name an end state, so a second send changes nothing.
        retry: matches!(method, ApiMethod::Put | ApiMethod::Delete),
        answer,
    }
}

/// Every operation the CLI maps.
pub mod operations {
    use super::ApiAccess::{Account, Public};
    use super::ApiAnswerBody::{Empty, Json};
    use super::ApiMethod::{Delete, Patch, Post, Put};
    use super::{ApiOperation, mutation, query};

    const PAGE: &[&str] = &["limit", "offset"];

    pub const SKILLS_SEARCH: ApiOperation =
        query("skills.search", "/api/v1/skills", &["q", "limit"], Public);
    pub const SKILLS_GET: ApiOperation = query(
        "skills.get",
        "/api/v1/skills/{owner}/{repository}/{name}",
        &[],
        Public,
    );
    pub const SKILLS_BROWSE: ApiOperation = query(
        "skills.browse",
        "/api/v1/browse",
        &["q", "owner", "tag", "sort", "limit", "offset"],
        Public,
    );
    pub const REPOSITORIES_GET: ApiOperation = query(
        "repositories.get",
        "/api/v1/repositories/{owner}/{repository}",
        &[],
        Public,
    );
    pub const INDEX_REQUESTS_CREATE: ApiOperation = mutation(
        "index_requests.create",
        Post,
        "/api/v1/index-requests",
        Public,
        Json,
    );
    pub const INDEX_REQUESTS_GET: ApiOperation = query(
        "index_requests.get",
        "/api/v1/index-requests/{id}",
        &[],
        Public,
    );
    pub const OWNERS_GET: ApiOperation = query("owners.get", "/api/v1/owners/{owner}", &[], Public);
    pub const TRACKS_LIST: ApiOperation = query("tracks.list", "/api/v1/tracks", &[], Public);
    pub const TRACKS_GET: ApiOperation = query("tracks.get", "/api/v1/tracks/{slug}", PAGE, Public);
    pub const TRENDING_LIST: ApiOperation = query(
        "trending.list",
        "/api/v1/trending",
        &["window", "limit"],
        Public,
    );

    pub const CURATORS_LIST: ApiOperation =
        query("curators.list", "/api/v1/curators", PAGE, Public);
    pub const CURATORS_GET: ApiOperation =
        query("curators.get", "/api/v1/curators/{login}", &[], Public);
    pub const CURATORS_LIKES: ApiOperation = query(
        "curators.likes",
        "/api/v1/curators/{login}/likes",
        PAGE,
        Public,
    );
    pub const COLLECTIONS_GET: ApiOperation = query(
        "collections.get",
        "/api/v1/collections/{login}/{slug}",
        PAGE,
        Public,
    );
    pub const COLLECTIONS_CREATE: ApiOperation = mutation(
        "collections.create",
        Post,
        "/api/v1/collections",
        Account,
        Json,
    );
    pub const COLLECTIONS_SKILLS_ADD: ApiOperation = mutation(
        "collections.skills.add",
        Put,
        "/api/v1/collections/{login}/{slug}/skills/{owner}/{repository}/{name}",
        Account,
        Json,
    );
    pub const COLLECTIONS_SKILLS_REMOVE: ApiOperation = mutation(
        "collections.skills.remove",
        Delete,
        "/api/v1/collections/{login}/{slug}/skills/{owner}/{repository}/{name}",
        Account,
        Empty,
    );
    pub const COLLECTIONS_WATCH: ApiOperation = mutation(
        "collections.watch",
        Put,
        "/api/v1/collections/{login}/{slug}/watch",
        Account,
        Json,
    );

    pub const ACCOUNT_GET: ApiOperation = query("account.get", "/api/v1/account", &[], Account);
    pub const ACCOUNT_UPDATE: ApiOperation =
        mutation("account.update", Patch, "/api/v1/account", Account, Json);
    pub const ACCOUNT_REPOSITORIES_SCAN: ApiOperation = mutation(
        "account.repositories.scan",
        Post,
        "/api/v1/account/repositories/scan",
        Account,
        Json,
    );
    pub const ACCOUNT_REPOSITORIES_UNPUBLISH: ApiOperation = mutation(
        "account.repositories.unpublish",
        Delete,
        "/api/v1/account/repositories/{owner}/{repository}",
        Account,
        Empty,
    );
    pub const LIKES_LIST: ApiOperation =
        query("likes.list", "/api/v1/account/likes", PAGE, Account);
    pub const LIKES_CREATE: ApiOperation = mutation(
        "likes.create",
        Put,
        "/api/v1/account/likes/{owner}/{repository}/{name}",
        Account,
        Empty,
    );
    pub const LIKES_DELETE: ApiOperation = mutation(
        "likes.delete",
        Delete,
        "/api/v1/account/likes/{owner}/{repository}/{name}",
        Account,
        Empty,
    );
    pub const WATCHES_LIST: ApiOperation =
        query("watches.list", "/api/v1/account/watches", PAGE, Account);
    pub const WATCHES_CREATE: ApiOperation = mutation(
        "watches.create",
        Put,
        "/api/v1/account/watches/{owner}/{repository}",
        Account,
        Empty,
    );
    pub const WATCHES_DELETE: ApiOperation = mutation(
        "watches.delete",
        Delete,
        "/api/v1/account/watches/{owner}/{repository}",
        Account,
        Empty,
    );
    pub const STARS_LIST: ApiOperation =
        query("stars.list", "/api/v1/account/stars", PAGE, Account);
    pub const STARS_IMPORT: ApiOperation = mutation(
        "stars.import",
        Post,
        "/api/v1/account/stars/import",
        Account,
        Json,
    );
    pub const CHANGES_LIST: ApiOperation = query(
        "changes.list",
        "/api/v1/account/changes",
        &["since"],
        Account,
    );
    pub const TOKENS_LIST: ApiOperation =
        query("tokens.list", "/api/v1/account/tokens", PAGE, Account);
    pub const TOKENS_CREATE: ApiOperation = mutation(
        "tokens.create",
        Post,
        "/api/v1/account/tokens",
        Account,
        Json,
    );
    pub const TOKENS_REVOKE: ApiOperation = mutation(
        "tokens.revoke",
        Delete,
        "/api/v1/account/tokens/{id}",
        Account,
        Empty,
    );

    /// Every operation, for the contract test.
    pub const ALL: &[ApiOperation] = &[
        SKILLS_SEARCH,
        SKILLS_GET,
        SKILLS_BROWSE,
        REPOSITORIES_GET,
        INDEX_REQUESTS_CREATE,
        INDEX_REQUESTS_GET,
        OWNERS_GET,
        TRACKS_LIST,
        TRACKS_GET,
        TRENDING_LIST,
        CURATORS_LIST,
        CURATORS_GET,
        CURATORS_LIKES,
        COLLECTIONS_GET,
        COLLECTIONS_CREATE,
        COLLECTIONS_SKILLS_ADD,
        COLLECTIONS_SKILLS_REMOVE,
        COLLECTIONS_WATCH,
        ACCOUNT_GET,
        ACCOUNT_UPDATE,
        ACCOUNT_REPOSITORIES_SCAN,
        ACCOUNT_REPOSITORIES_UNPUBLISH,
        LIKES_LIST,
        LIKES_CREATE,
        LIKES_DELETE,
        WATCHES_LIST,
        WATCHES_CREATE,
        WATCHES_DELETE,
        STARS_LIST,
        STARS_IMPORT,
        CHANGES_LIST,
        TOKENS_LIST,
        TOKENS_CREATE,
        TOKENS_REVOKE,
    ];
}

/// One page of a list. `total` counts the whole result, not this page.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ApiList<T> {
    pub items: Vec<T>,
    pub total: u64,
}

/// The fields every Skill card carries, on every list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub owner: String,
    pub repository: String,
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub stars: u64,
    pub likes: u64,
    /// When the SKILL.md last changed upstream.
    pub updated_at: Option<String>,
    pub page_url: String,
    /// The SKILL.md in the author's Repository. `None` until the first sync.
    pub source_url: Option<String>,
    pub run_command: String,
    pub install_command: String,
}

/// One file beside a SKILL.md.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SkillFileEntry {
    pub path: String,
    pub size: u64,
}

/// `skills.get`: one Skill with its provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    #[serde(flatten)]
    pub summary: SkillSummary,
    /// The GitHub profile name of the Owner, when the registry synced it.
    pub author_name: Option<String>,
    pub license: Option<String>,
    pub repository_url: String,
    /// The SKILL.md path inside the Repository.
    pub skill_path: Option<String>,
    /// The commit the registry last read.
    pub source_commit: Option<String>,
    /// True when the SKILL.md is gone upstream.
    pub source_gone: bool,
    pub pushed_at: Option<String>,
    pub tags: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub files: Vec<SkillFileEntry>,
    /// Machine-generated from the SKILL.md. Never written by the author.
    pub generated_summary: Option<String>,
    pub markdown: Option<String>,
}

/// `repositories.get`: one Repository and every Skill the registry holds from it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryProfile {
    pub owner: String,
    pub repository: String,
    pub description: Option<String>,
    pub stars: u64,
    pub pushed_at: Option<String>,
    pub repository_url: String,
    pub page_url: String,
    pub install_command: String,
    pub skills: Vec<SkillSummary>,
}

/// `owners.get`: one GitHub Owner and the Repositories it publishes Skills from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OwnerProfile {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: String,
    pub kind: String,
    pub page_url: String,
    pub repositories: Vec<OwnerRepository>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OwnerRepository {
    pub repository: String,
    pub description: Option<String>,
    pub stars: u64,
    pub skill_count: u64,
    pub page_url: String,
}

/// `index_requests.create` body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IndexRequestBody {
    /// `owner/repository`, or a github.com URL.
    pub repository: String,
}

/// `index_requests.create` and `index_requests.get`: one index request.
///
/// `create` answers `indexed` or `queued`. `get` can also answer `failed`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum IndexRequest {
    Queued {
        /// Poll `index_requests.get` with this until the status changes.
        id: String,
        owner: String,
        repository: String,
        progress: IndexProgress,
    },
    Indexed {
        owner: String,
        repository: String,
        skills: Vec<SkillSummary>,
    },
    Failed {
        owner: String,
        repository: String,
        /// Why the registry could not index the Repository.
        reason: String,
    },
}

/// How far a queued index request got.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "stage", rename_all = "lowercase")]
pub enum IndexProgress {
    Queued,
    Checking,
    Indexing {
        indexed: u64,
        total: u64,
    },
    /// A stage this CLI does not know yet.
    #[serde(other)]
    Unknown,
}

/// `tracks.list` row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrackSummary {
    pub slug: String,
    pub label: String,
    /// The second-person line a person wrote for the track.
    pub line: String,
    pub page_url: String,
    pub skill_count: u64,
}

/// `tracks.get`: one track and one page of its Skills.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrackDetail {
    pub slug: String,
    pub label: String,
    pub line: String,
    pub page_url: String,
    pub items: Vec<SkillSummary>,
    pub total: u64,
}

/// `trending.list` row: a Skill and why it is on the board.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct TrendingSkill {
    #[serde(flatten)]
    pub summary: SkillSummary,
    pub signal: TrendingSignal,
}

/// Why one Skill trends.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TrendingSignal {
    /// Devs posted about the Skill. `post` is one of those posts.
    #[serde(rename_all = "camelCase")]
    Social {
        author_count: u64,
        mention_count: u64,
        post: TrendingPost,
    },
    /// The Repository gained stars fast, and it holds only this Skill.
    #[serde(rename_all = "camelCase")]
    StarSurge { star_gain: u64, surged_on: String },
    /// Both of the above.
    #[serde(rename_all = "camelCase")]
    SocialAndStarSurge {
        author_count: u64,
        mention_count: u64,
        post: TrendingPost,
        star_gain: u64,
        surged_on: String,
    },
    /// The socials were quiet, so the board filled with a well-starred Skill.
    StarCount,
    /// A reason this CLI does not know yet.
    #[serde(other)]
    Unknown,
}

/// One post that talked about a trending Skill.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrendingPost {
    pub url: String,
    /// `x` or `bsky`.
    pub platform: String,
    pub author_handle: String,
    pub author_name: Option<String>,
    pub text: String,
    pub posted_at: String,
}

/// The curator fields every collection answer names.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CuratorIdentity {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

/// `curators.list` row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CuratorSummary {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub page_url: String,
    pub collection_count: u64,
}

/// `curators.get`: one curator and their collections.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CuratorDetail {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub page_url: String,
    /// Installs every Skill the curator's collections name.
    pub install_command: String,
    pub collections: Vec<CollectionSummary>,
}

/// One collection in a curator answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub page_url: String,
    pub install_command: String,
    pub skill_count: u64,
}

/// `collections.get` and `collections.create`: one collection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDetail {
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub page_url: String,
    pub install_command: String,
    pub curator: CuratorIdentity,
    pub skills: ApiList<CollectionSkill>,
}

/// One Skill in a collection, with the curator's reason for it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct CollectionSkill {
    #[serde(flatten)]
    pub summary: SkillSummary,
    pub reason: Option<String>,
}

/// `collections.create` body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCollectionBody {
    pub slug: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<CollectionSkillEntry>,
}

/// One Skill a new collection names.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionSkillEntry {
    pub owner: String,
    pub repository: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `collections.skills.add` body. Without `reason`, the stored reason stays.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddCollectionSkillBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `collections.watch`
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct CollectionWatch {
    /// The Repositories of the collection, all of which you now watch.
    pub watched: u64,
}

/// `account.get` and `account.update`: the signed-in account.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub page_url: String,
    /// The address skilld emails.
    pub email: Option<String>,
    /// Whether the digest of watched Repositories goes to `email`.
    pub digest: bool,
    /// Whether the weekly goes to `email`.
    pub weekly: bool,
    /// Whether anyone can read the liked Skills at `/@login/liked`.
    pub likes_public: bool,
    /// Whether skilld may scan the account's public Repositories for Skills.
    pub repository_indexing: bool,
    pub stars_imported_at: Option<String>,
}

/// `account.update` body. Only the settings to change.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AccountUpdateBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub likes_public: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_indexing: Option<bool>,
}

/// `account.repositories.scan`
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryScan {
    /// `complete`, `partial`, or a `github-*` failure.
    pub outcome: String,
    pub repositories_found: u64,
    pub repositories_indexed: u64,
    pub repositories_failed: u64,
}

/// `likes.list` row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LikedSkill {
    #[serde(flatten)]
    pub summary: SkillSummary,
    pub liked_at: String,
}

/// `watches.list` row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub owner: String,
    pub repository: String,
    pub page_url: String,
    /// `direct`, `like`, `star-import`, or `collection`.
    pub reason: String,
    pub watched_at: String,
}

/// `stars.list` row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StarredRepository {
    pub owner: String,
    pub repository: String,
    pub page_url: String,
    pub starred_at: String,
    pub watching: bool,
    pub skill_count: u64,
}

/// `stars.import` body.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StarsImportBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

/// `stars.import`: one imported page.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StarsImport {
    pub page: u32,
    /// The page to import next, or `None` when the import is finished.
    pub next_page: Option<u32>,
    pub imported: u64,
    pub with_skills: u64,
    pub imported_at: Option<String>,
}

/// `changes.list`: what changed in the watched Repositories.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct AccountChanges {
    pub since: String,
    /// Send this as `since` next time to read only newer changes.
    pub until: String,
    pub items: Vec<ChangedSkill>,
}

/// One changed Skill.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedSkill {
    #[serde(flatten)]
    pub summary: SkillSummary,
    pub changed_at: String,
    pub change_count: u64,
    /// Commit messages from the author's Repository, newest first.
    pub commit_messages: Vec<String>,
    pub change_url: String,
}

/// `tokens.list` row. It never holds a secret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub id: u64,
    pub label: Option<String>,
    /// `oauth`, `pat`, or `oidc`.
    pub kind: String,
    pub created_at: String,
    pub last_used_at: String,
    pub expires_at: Option<String>,
    /// True for the token that sent this request.
    pub current: bool,
}

/// `tokens.create` body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TokenCreateBody {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_days: Option<u32>,
}

/// `tokens.create`: the new token, with the only copy of its secret.
#[derive(Clone, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IssuedToken {
    pub id: u64,
    pub label: String,
    pub expires_at: Option<String>,
    pub token: String,
}

impl std::fmt::Debug for IssuedToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedToken")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("expires_at", &self.expires_at)
            .field("token", &"[REDACTED]")
            .finish()
    }
}
