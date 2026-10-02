//! The typed client for the skilld.dev public API, version 1.
//!
//! One method per operation the CLI uses. Every method shares one request
//! path: the route comes from the operation descriptor in
//! `skilld_core::api::operations`, an account operation carries the stored
//! skilld token, and a failure arrives as `application/problem+json`.
//!
//! Queries and idempotent PUT and DELETE mutations repeat on a 429, a 503, or
//! a transport failure. POST and PATCH never repeat, because a second send
//! could act twice.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use skilld_core::RemoteError;
use skilld_core::api::{
    Account, AccountChanges, AccountUpdateBody, AddCollectionSkillBody, ApiAccess, ApiAnswerBody,
    ApiList, ApiMethod, ApiOperation, CollectionDetail, CollectionSkill, CollectionWatch,
    CreateCollectionBody, CuratorDetail, CuratorSummary, IndexRequest, IndexRequestBody,
    IssuedToken, LikedSkill, RepositoryProfile, RepositoryScan, SkillDetail, SkillSummary,
    StarredRepository, StarsImport, StarsImportBody, Token, TokenCreateBody, TrackDetail,
    TrackSummary, TrendingSkill, Watch, operations,
};
use url::Url;

use super::{
    AllowedOrigin, HeaderValue, HttpHeader, HttpMethod, HttpRequest, HttpResponse, JSON_LIMIT,
    MAX_RETRIES, Problem, RemoteProgressStage, SkilldRemote, human_wait, retry_after_seconds,
    service_unavailable_error,
};

/// One parsed answer, and the JSON skilld.dev sent.
///
/// `--json` output carries `raw`, so a field this CLI does not know yet still
/// reaches the Agent that asked.
#[derive(Clone, Debug, PartialEq)]
pub struct ApiAnswer<T> {
    pub data: T,
    pub raw: Value,
}

/// `limit` and `offset` for one list request. `None` keeps the server default.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApiPage {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// How `skilld browse` orders Skills. Install counts never order anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowseSort {
    Stars,
    Likes,
    Updated,
}

impl BrowseSort {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stars => "stars",
            Self::Likes => "likes",
            Self::Updated => "updated",
        }
    }
}

/// The filters of one `skills.browse` request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BrowseQuery {
    pub text: Option<String>,
    pub owner: Option<String>,
    pub tag: Option<String>,
    pub sort: Option<BrowseSort>,
    pub page: ApiPage,
}

/// The trending board window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrendingWindow {
    Week,
    Month,
}

impl TrendingWindow {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

/// The skilld.dev public API, one method per operation the CLI uses.
pub trait SkilldApi: Send + Sync {
    /// The origin every answer belongs to, such as `https://skilld.dev`.
    /// Output prints a skilld.dev page URL only when it stays on this origin.
    fn site_origin(&self) -> Url;

    /// `skills.get`
    fn skill(
        &self,
        owner: &str,
        repository: &str,
        name: &str,
    ) -> Result<ApiAnswer<SkillDetail>, RemoteError>;

    /// `skills.browse`
    fn browse(&self, query: &BrowseQuery) -> Result<ApiAnswer<ApiList<SkillSummary>>, RemoteError>;

    /// `repositories.get`
    fn repository(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<ApiAnswer<RepositoryProfile>, RemoteError>;

    /// `index_requests.create`
    fn create_index_request(
        &self,
        repository: &str,
    ) -> Result<ApiAnswer<IndexRequest>, RemoteError>;

    /// `index_requests.get`
    fn index_request(&self, id: &str) -> Result<ApiAnswer<IndexRequest>, RemoteError>;

    /// `tracks.list`
    fn tracks(&self) -> Result<ApiAnswer<ApiList<TrackSummary>>, RemoteError>;

    /// `tracks.get`
    fn track(&self, slug: &str, page: ApiPage) -> Result<ApiAnswer<TrackDetail>, RemoteError>;

    /// `trending.list`
    fn trending(
        &self,
        window: Option<TrendingWindow>,
        limit: Option<u32>,
    ) -> Result<ApiAnswer<ApiList<TrendingSkill>>, RemoteError>;

    /// Wait between two index request polls.
    fn wait_for_index(&self) -> Result<(), RemoteError>;

    /// `curators.list`
    fn curators(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<CuratorSummary>>, RemoteError>;

    /// `curators.get`
    fn curator(&self, login: &str) -> Result<ApiAnswer<CuratorDetail>, RemoteError>;

    /// `curators.likes`
    fn curator_likes(
        &self,
        login: &str,
        page: ApiPage,
    ) -> Result<ApiAnswer<ApiList<SkillSummary>>, RemoteError>;

    /// `collections.get`
    fn collection(
        &self,
        login: &str,
        slug: &str,
        page: ApiPage,
    ) -> Result<ApiAnswer<CollectionDetail>, RemoteError>;

    /// `collections.create`
    fn create_collection(
        &self,
        body: &CreateCollectionBody,
    ) -> Result<ApiAnswer<CollectionDetail>, RemoteError>;

    /// `collections.skills.add`
    fn add_collection_skill(
        &self,
        collection: (&str, &str),
        skill: (&str, &str, &str),
        body: &AddCollectionSkillBody,
    ) -> Result<ApiAnswer<CollectionSkill>, RemoteError>;

    /// `collections.skills.remove`
    fn remove_collection_skill(
        &self,
        collection: (&str, &str),
        skill: (&str, &str, &str),
    ) -> Result<(), RemoteError>;

    /// `collections.watch`
    fn watch_collection(
        &self,
        login: &str,
        slug: &str,
    ) -> Result<ApiAnswer<CollectionWatch>, RemoteError>;

    /// `account.get`
    fn account(&self) -> Result<ApiAnswer<Account>, RemoteError>;

    /// `account.update`
    fn update_account(&self, body: &AccountUpdateBody) -> Result<ApiAnswer<Account>, RemoteError>;

    /// `account.repositories.scan`
    fn scan_repositories(&self) -> Result<ApiAnswer<RepositoryScan>, RemoteError>;

    /// `account.repositories.unpublish`
    fn unpublish_repository(&self, owner: &str, repository: &str) -> Result<(), RemoteError>;

    /// `likes.list`
    fn likes(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<LikedSkill>>, RemoteError>;

    /// `likes.create`
    fn like(&self, owner: &str, repository: &str, name: &str) -> Result<(), RemoteError>;

    /// `likes.delete`
    fn unlike(&self, owner: &str, repository: &str, name: &str) -> Result<(), RemoteError>;

    /// `watches.list`
    fn watches(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<Watch>>, RemoteError>;

    /// `watches.create`
    fn watch(&self, owner: &str, repository: &str) -> Result<(), RemoteError>;

    /// `watches.delete`
    fn unwatch(&self, owner: &str, repository: &str) -> Result<(), RemoteError>;

    /// `stars.list`
    fn stars(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<StarredRepository>>, RemoteError>;

    /// `stars.import`
    fn import_stars(&self, page: Option<u32>) -> Result<ApiAnswer<StarsImport>, RemoteError>;

    /// `changes.list`
    fn changes(&self, since: Option<&str>) -> Result<ApiAnswer<AccountChanges>, RemoteError>;

    /// `tokens.list`
    fn tokens(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<Token>>, RemoteError>;

    /// `tokens.create`
    fn create_token(&self, body: &TokenCreateBody) -> Result<ApiAnswer<IssuedToken>, RemoteError>;

    /// `tokens.revoke`
    fn revoke_token(&self, id: u64) -> Result<(), RemoteError>;
}

/// The wait between two `index_requests.get` polls.
const INDEX_REQUEST_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// The message for an account operation without a stored sign-in.
pub(crate) const SIGN_IN_FIRST: &str = "This command needs a skilld.dev account. Run skilld auth login, then run the same command again.";

impl SkilldRemote {
    /// One absolute URL for an operation: the template with each `{param}`
    /// replaced by one percent-encoded path segment, plus the query pairs.
    fn api_url(
        &self,
        operation: &ApiOperation,
        params: &[&str],
        query: &[(&str, String)],
    ) -> Result<Url, RemoteError> {
        let invalid = || {
            RemoteError::new(
                "INVALID_REQUEST",
                format!("the {} request has an invalid path", operation.id),
            )
        };
        let mut url = self.service_url("/")?;
        let mut values = params.iter();
        {
            let mut segments = url.path_segments_mut().map_err(|_| invalid())?;
            segments.clear();
            for segment in operation.path.split('/').filter(|part| !part.is_empty()) {
                if segment.starts_with('{') {
                    let value = values.next().ok_or_else(invalid)?;
                    if value.is_empty() || *value == "." || *value == ".." {
                        return Err(invalid());
                    }
                    segments.push(value);
                } else {
                    segments.push(segment);
                }
            }
        }
        if values.next().is_some() {
            return Err(invalid());
        }
        let mut pairs = query.iter().peekable();
        if pairs.peek().is_some() {
            let mut serializer = url.query_pairs_mut();
            for (name, value) in pairs {
                debug_assert!(operation.query.contains(name), "{name}");
                serializer.append_pair(name, value);
            }
        }
        Ok(url)
    }

    /// Send one operation and return its JSON answer, or `None` for 204.
    fn api_send(
        &self,
        operation: &ApiOperation,
        params: &[&str],
        query: &[(&str, String)],
        body: Option<Vec<u8>>,
    ) -> Result<Option<Value>, RemoteError> {
        let mut headers = vec![HttpHeader {
            name: "accept".to_owned(),
            value: HeaderValue::Public("application/json".to_owned()),
        }];
        if body.is_some() {
            headers.push(HttpHeader {
                name: "content-type".to_owned(),
                value: HeaderValue::Public("application/json".to_owned()),
            });
        }
        if operation.access == ApiAccess::Account {
            // A stored sign-in that cannot refresh fails here, before any
            // request. It needs the same next step as no sign-in at all.
            let token = self
                .tokens
                .access_token()
                .map_err(|error| match error.code {
                    "AUTH_REQUIRED" => RemoteError::new(
                        "AUTH_REQUIRED",
                        format!(
                            "{} Run skilld auth login, then run the same command again.",
                            error.message.trim_end()
                        ),
                    ),
                    _ => error,
                })?
                .ok_or_else(|| RemoteError::new("AUTH_REQUIRED", SIGN_IN_FIRST))?;
            headers.push(HttpHeader {
                name: "authorization".to_owned(),
                value: HeaderValue::Secret(super::SecretValue::new(format!(
                    "Bearer {}",
                    token.expose()
                ))?),
            });
        }
        let request = HttpRequest {
            method: match operation.method {
                ApiMethod::Get => HttpMethod::Get,
                ApiMethod::Post => HttpMethod::Post,
                ApiMethod::Put => HttpMethod::Put,
                ApiMethod::Patch => HttpMethod::Patch,
                ApiMethod::Delete => HttpMethod::Delete,
            },
            url: self.api_url(operation, params, query)?.into(),
            headers,
            body: body.unwrap_or_default(),
            response_limit: JSON_LIMIT,
        };
        let retries = if operation.retry { MAX_RETRIES } else { 0 };
        let response = self.execute_with_retries(
            request,
            AllowedOrigin::Api(self.endpoint.clone()),
            None,
            retries,
        )?;
        match operation.answer {
            ApiAnswerBody::Empty => Ok(None),
            ApiAnswerBody::Json => serde_json::from_slice(&response.body)
                .map(Some)
                .map_err(|_| unreadable(operation)),
        }
    }

    fn api_json<T: DeserializeOwned>(
        &self,
        operation: &ApiOperation,
        params: &[&str],
        query: &[(&str, String)],
        body: Option<Vec<u8>>,
    ) -> Result<ApiAnswer<T>, RemoteError> {
        let raw = self
            .api_send(operation, params, query, body)?
            .ok_or_else(|| unreadable(operation))?;
        let data = T::deserialize(&raw).map_err(|_| unreadable(operation))?;
        Ok(ApiAnswer { data, raw })
    }
}

impl SkilldRemote {
    /// Send one operation that answers `204 No Content`.
    fn api_empty(
        &self,
        operation: &ApiOperation,
        params: &[&str],
        body: Option<Vec<u8>>,
    ) -> Result<(), RemoteError> {
        self.api_send(operation, params, &[], body).map(|_| ())
    }
}

/// An answer this CLI cannot read: skilld.dev changed a shape this release
/// depends on.
fn unreadable(operation: &ApiOperation) -> RemoteError {
    RemoteError::new(
        "INVALID_RESPONSE",
        format!(
            "skilld.dev answered {} in a shape this skilld release cannot read. Upgrade skilld, then run the same command again.",
            operation.id
        ),
    )
}

fn encode<T: Serialize>(operation: &ApiOperation, body: &T) -> Result<Vec<u8>, RemoteError> {
    serde_json::to_vec(body).map_err(|_| {
        RemoteError::new(
            "INVALID_REQUEST",
            format!("the {} request could not be encoded", operation.id),
        )
    })
}

fn page_query(page: ApiPage) -> Vec<(&'static str, String)> {
    page.limit
        .map(|limit| ("limit", limit.to_string()))
        .into_iter()
        .chain(page.offset.map(|offset| ("offset", offset.to_string())))
        .collect()
}

impl SkilldApi for SkilldRemote {
    fn site_origin(&self) -> Url {
        self.endpoint.clone()
    }

    fn skill(
        &self,
        owner: &str,
        repository: &str,
        name: &str,
    ) -> Result<ApiAnswer<SkillDetail>, RemoteError> {
        self.api_json(
            &operations::SKILLS_GET,
            &[owner, repository, name],
            &[],
            None,
        )
    }

    fn browse(&self, query: &BrowseQuery) -> Result<ApiAnswer<ApiList<SkillSummary>>, RemoteError> {
        let mut pairs = Vec::new();
        pairs.extend(query.text.clone().map(|text| ("q", text)));
        pairs.extend(query.owner.clone().map(|owner| ("owner", owner)));
        pairs.extend(query.tag.clone().map(|tag| ("tag", tag)));
        pairs.extend(query.sort.map(|sort| ("sort", sort.as_str().to_owned())));
        pairs.extend(page_query(query.page));
        self.api_json(&operations::SKILLS_BROWSE, &[], &pairs, None)
    }

    fn repository(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<ApiAnswer<RepositoryProfile>, RemoteError> {
        self.api_json(
            &operations::REPOSITORIES_GET,
            &[owner, repository],
            &[],
            None,
        )
    }

    fn create_index_request(
        &self,
        repository: &str,
    ) -> Result<ApiAnswer<IndexRequest>, RemoteError> {
        let operation = &operations::INDEX_REQUESTS_CREATE;
        let body = encode(
            operation,
            &IndexRequestBody {
                repository: repository.to_owned(),
            },
        )?;
        self.api_json(operation, &[], &[], Some(body))
    }

    fn index_request(&self, id: &str) -> Result<ApiAnswer<IndexRequest>, RemoteError> {
        self.api_json(&operations::INDEX_REQUESTS_GET, &[id], &[], None)
    }

    fn tracks(&self) -> Result<ApiAnswer<ApiList<TrackSummary>>, RemoteError> {
        self.api_json(&operations::TRACKS_LIST, &[], &[], None)
    }

    fn track(&self, slug: &str, page: ApiPage) -> Result<ApiAnswer<TrackDetail>, RemoteError> {
        self.api_json(&operations::TRACKS_GET, &[slug], &page_query(page), None)
    }

    fn trending(
        &self,
        window: Option<TrendingWindow>,
        limit: Option<u32>,
    ) -> Result<ApiAnswer<ApiList<TrendingSkill>>, RemoteError> {
        let mut pairs = Vec::new();
        pairs.extend(window.map(|window| ("window", window.as_str().to_owned())));
        pairs.extend(limit.map(|limit| ("limit", limit.to_string())));
        self.api_json(&operations::TRENDING_LIST, &[], &pairs, None)
    }

    fn wait_for_index(&self) -> Result<(), RemoteError> {
        self.progress.stage(RemoteProgressStage::Indexing);
        self.sleep(INDEX_REQUEST_POLL, None)
    }

    fn curators(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<CuratorSummary>>, RemoteError> {
        self.api_json(&operations::CURATORS_LIST, &[], &page_query(page), None)
    }

    fn curator(&self, login: &str) -> Result<ApiAnswer<CuratorDetail>, RemoteError> {
        self.api_json(&operations::CURATORS_GET, &[login], &[], None)
    }

    fn curator_likes(
        &self,
        login: &str,
        page: ApiPage,
    ) -> Result<ApiAnswer<ApiList<SkillSummary>>, RemoteError> {
        self.api_json(
            &operations::CURATORS_LIKES,
            &[login],
            &page_query(page),
            None,
        )
    }

    fn collection(
        &self,
        login: &str,
        slug: &str,
        page: ApiPage,
    ) -> Result<ApiAnswer<CollectionDetail>, RemoteError> {
        self.api_json(
            &operations::COLLECTIONS_GET,
            &[login, slug],
            &page_query(page),
            None,
        )
    }

    fn create_collection(
        &self,
        body: &CreateCollectionBody,
    ) -> Result<ApiAnswer<CollectionDetail>, RemoteError> {
        let operation = &operations::COLLECTIONS_CREATE;
        self.api_json(operation, &[], &[], Some(encode(operation, body)?))
    }

    fn add_collection_skill(
        &self,
        (login, slug): (&str, &str),
        (owner, repository, name): (&str, &str, &str),
        body: &AddCollectionSkillBody,
    ) -> Result<ApiAnswer<CollectionSkill>, RemoteError> {
        let operation = &operations::COLLECTIONS_SKILLS_ADD;
        self.api_json(
            operation,
            &[login, slug, owner, repository, name],
            &[],
            Some(encode(operation, body)?),
        )
    }

    fn remove_collection_skill(
        &self,
        (login, slug): (&str, &str),
        (owner, repository, name): (&str, &str, &str),
    ) -> Result<(), RemoteError> {
        self.api_empty(
            &operations::COLLECTIONS_SKILLS_REMOVE,
            &[login, slug, owner, repository, name],
            None,
        )
    }

    fn watch_collection(
        &self,
        login: &str,
        slug: &str,
    ) -> Result<ApiAnswer<CollectionWatch>, RemoteError> {
        self.api_json(&operations::COLLECTIONS_WATCH, &[login, slug], &[], None)
    }

    fn account(&self) -> Result<ApiAnswer<Account>, RemoteError> {
        self.api_json(&operations::ACCOUNT_GET, &[], &[], None)
    }

    fn update_account(&self, body: &AccountUpdateBody) -> Result<ApiAnswer<Account>, RemoteError> {
        let operation = &operations::ACCOUNT_UPDATE;
        self.api_json(operation, &[], &[], Some(encode(operation, body)?))
    }

    fn scan_repositories(&self) -> Result<ApiAnswer<RepositoryScan>, RemoteError> {
        self.api_json(&operations::ACCOUNT_REPOSITORIES_SCAN, &[], &[], None)
    }

    fn unpublish_repository(&self, owner: &str, repository: &str) -> Result<(), RemoteError> {
        self.api_empty(
            &operations::ACCOUNT_REPOSITORIES_UNPUBLISH,
            &[owner, repository],
            None,
        )
    }

    fn likes(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<LikedSkill>>, RemoteError> {
        self.api_json(&operations::LIKES_LIST, &[], &page_query(page), None)
    }

    fn like(&self, owner: &str, repository: &str, name: &str) -> Result<(), RemoteError> {
        self.api_empty(&operations::LIKES_CREATE, &[owner, repository, name], None)
    }

    fn unlike(&self, owner: &str, repository: &str, name: &str) -> Result<(), RemoteError> {
        self.api_empty(&operations::LIKES_DELETE, &[owner, repository, name], None)
    }

    fn watches(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<Watch>>, RemoteError> {
        self.api_json(&operations::WATCHES_LIST, &[], &page_query(page), None)
    }

    fn watch(&self, owner: &str, repository: &str) -> Result<(), RemoteError> {
        self.api_empty(&operations::WATCHES_CREATE, &[owner, repository], None)
    }

    fn unwatch(&self, owner: &str, repository: &str) -> Result<(), RemoteError> {
        self.api_empty(&operations::WATCHES_DELETE, &[owner, repository], None)
    }

    fn stars(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<StarredRepository>>, RemoteError> {
        self.api_json(&operations::STARS_LIST, &[], &page_query(page), None)
    }

    fn import_stars(&self, page: Option<u32>) -> Result<ApiAnswer<StarsImport>, RemoteError> {
        let operation = &operations::STARS_IMPORT;
        self.api_json(
            operation,
            &[],
            &[],
            Some(encode(operation, &StarsImportBody { page })?),
        )
    }

    fn changes(&self, since: Option<&str>) -> Result<ApiAnswer<AccountChanges>, RemoteError> {
        let query = since
            .map(|since| ("since", since.to_owned()))
            .into_iter()
            .collect::<Vec<_>>();
        self.api_json(&operations::CHANGES_LIST, &[], &query, None)
    }

    fn tokens(&self, page: ApiPage) -> Result<ApiAnswer<ApiList<Token>>, RemoteError> {
        self.api_json(&operations::TOKENS_LIST, &[], &page_query(page), None)
    }

    fn create_token(&self, body: &TokenCreateBody) -> Result<ApiAnswer<IssuedToken>, RemoteError> {
        let operation = &operations::TOKENS_CREATE;
        self.api_json(operation, &[], &[], Some(encode(operation, body)?))
    }

    fn revoke_token(&self, id: u64) -> Result<(), RemoteError> {
        self.api_empty(&operations::TOKENS_REVOKE, &[&id.to_string()], None)
    }
}

/// Map one public API problem to a code and a message a person can act on.
///
/// The problem `code` survives as the error code, so an Agent reads the same
/// code the contract names. The message keeps the server's detail and adds the
/// next step.
pub(super) fn api_problem_error(response: &HttpResponse) -> RemoteError {
    let Ok(problem) = serde_json::from_slice::<Problem>(&response.body) else {
        return service_unavailable_error(response);
    };
    let _ = (&problem.r#type, &problem.instance);
    if problem.status != response.status {
        return RemoteError::new(
            "INVALID_RESPONSE",
            "the remote problem status does not match HTTP",
        );
    }
    let detail = problem.detail.unwrap_or(problem.title);
    let detail = detail.trim().trim_end_matches('.');
    let request_id = response
        .header("x-request-id")
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
        .map(|value| format!(" If it keeps failing, report request ID {value}."))
        .unwrap_or_default();
    let (code, message) = match problem.code.as_str() {
        "INVALID_REQUEST" => ("INVALID_REQUEST", format!("{detail}.")),
        "AUTH_REQUIRED" => (
            "AUTH_REQUIRED",
            format!("{detail}. Run skilld auth login, then run the same command again."),
        ),
        "FORBIDDEN" => (
            "FORBIDDEN",
            format!("{detail}. Your account cannot do this."),
        ),
        "NOT_FOUND" => ("NOT_FOUND", format!("{detail}.")),
        "CONFLICT" => ("CONFLICT", format!("{detail}.")),
        "RATE_LIMITED" => (
            "RATE_LIMITED",
            match retry_after_seconds(response) {
                Some(seconds) => format!("{detail}. Retry in {}.", human_wait(seconds.max(1))),
                None => format!("{detail}. Retry in a minute."),
            },
        ),
        "INTERNAL_ERROR" => (
            "INTERNAL_ERROR",
            format!("{detail}. Retry in a minute.{request_id}"),
        ),
        "SERVICE_UNAVAILABLE" => (
            "SERVICE_UNAVAILABLE",
            format!("{detail}. Retry in a minute.{request_id}"),
        ),
        _ => return service_unavailable_error(response),
    };
    RemoteError::new(code, message)
}
