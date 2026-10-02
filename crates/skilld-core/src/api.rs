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
) -> ApiOperation {
    ApiOperation {
        id,
        method: ApiMethod::Get,
        path,
        query,
        access: ApiAccess::Public,
        retry: true,
        answer: ApiAnswerBody::Json,
    }
}

/// Every operation the CLI maps, in contract order.
pub mod operations {
    use super::{ApiAccess, ApiAnswerBody, ApiMethod, ApiOperation, query};

    pub const SKILLS_SEARCH: ApiOperation =
        query("skills.search", "/api/v1/skills", &["q", "limit"]);
    pub const SKILLS_GET: ApiOperation = query(
        "skills.get",
        "/api/v1/skills/{owner}/{repository}/{name}",
        &[],
    );
    pub const SKILLS_BROWSE: ApiOperation = query(
        "skills.browse",
        "/api/v1/browse",
        &["q", "owner", "tag", "sort", "limit", "offset"],
    );
    pub const REPOSITORIES_GET: ApiOperation = query(
        "repositories.get",
        "/api/v1/repositories/{owner}/{repository}",
        &[],
    );
    pub const INDEX_REQUESTS_CREATE: ApiOperation = ApiOperation {
        id: "index_requests.create",
        method: ApiMethod::Post,
        path: "/api/v1/index-requests",
        query: &[],
        access: ApiAccess::Public,
        retry: false,
        answer: ApiAnswerBody::Json,
    };
    pub const INDEX_REQUESTS_GET: ApiOperation =
        query("index_requests.get", "/api/v1/index-requests/{id}", &[]);
    pub const OWNERS_GET: ApiOperation = query("owners.get", "/api/v1/owners/{owner}", &[]);
    pub const TRACKS_LIST: ApiOperation = query("tracks.list", "/api/v1/tracks", &[]);
    pub const TRACKS_GET: ApiOperation =
        query("tracks.get", "/api/v1/tracks/{slug}", &["limit", "offset"]);
    pub const TRENDING_LIST: ApiOperation =
        query("trending.list", "/api/v1/trending", &["window", "limit"]);

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
