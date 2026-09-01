use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
#[cfg(not(target_os = "wasi"))]
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use skilld_core::{
    ArtifactAttestation, CommitAuthor, CommitSha, CommitSummary, ListedSkill, LockedSource,
    MultiSkillRef, PreparedFile, RemoteError, RemoteSelector, RepositoryVisibility, SearchResponse,
    SkillListing, SourceRef, SourceRequest, SourceSelector, SourceStatus, TrustedRoot,
    TrustedRootPin, VerifiedTrustedRoot, parse_search_response, prepare_unverified_files,
    verify_artifact, verify_attestation, verify_trusted_root,
};
use skilld_ui::text::is_unsafe_terminal;
use url::Url;

const JSON_LIMIT: usize = 8 * 1024 * 1024;
const UPDATE_RESPONSE_LIMIT: usize = 64 * 1024 * 1024;
const SEARCH_LIMIT: usize = 1024 * 1024;
const LISTING_LIMIT: usize = 4 * 1024 * 1024;
const LISTING_PAGE: usize = 200;
const ARTIFACT_LIMIT: usize = 64 * 1024 * 1024;
const DIRECT_BLOB_LIMIT: usize = 12 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const MAX_RETRIES: usize = 2;
const MAX_POLLS: usize = 120;
const MAX_UPDATE_COMPARISONS: usize = 500;
const UPDATE_SERVICE_BATCH: usize = 50;
#[cfg(not(target_os = "wasi"))]
const UPDATE_CONCURRENCY: usize = 4;
const UPDATE_COMMITS_PER_PAGE: usize = 100;
const MAX_UPDATE_COMMITS: usize = 500;
const GITHUB_API_VERSION: &str = "2026-03-10";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const RESOLUTION_TIMEOUT_MS: u64 = 60 * 1_000;
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct ResolutionDeadline {
    expires_at: Instant,
    remaining_wait: Duration,
}

impl ResolutionDeadline {
    fn new() -> Self {
        let timeout = Duration::from_millis(RESOLUTION_TIMEOUT_MS);
        Self {
            expires_at: Instant::now()
                .checked_add(timeout)
                .expect("the fixed Resolution timeout is valid"),
            remaining_wait: timeout,
        }
    }

    fn remaining(&self) -> Result<Duration, RemoteError> {
        let remaining = self
            .expires_at
            .checked_duration_since(Instant::now())
            .unwrap_or_default()
            .min(self.remaining_wait);
        if remaining.is_zero() {
            Err(resolution_timeout())
        } else {
            Ok(remaining)
        }
    }

    fn sleep(
        &mut self,
        duration: Duration,
        sleeper: &dyn Sleeper,
        cancellation: &dyn Cancellation,
    ) -> Result<(), RemoteError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let remaining = self.remaining()?;
        if duration >= remaining {
            sleeper.sleep(remaining, cancellation)?;
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            self.remaining_wait = Duration::ZERO;
            return Err(resolution_timeout());
        }
        sleeper.sleep(duration, cancellation)?;
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        self.remaining_wait = self.remaining_wait.saturating_sub(duration);
        self.remaining().map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone, Eq, PartialEq)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Result<Self, RemoteError> {
        let value = value.into();
        if value.is_empty() || value.contains(['\r', '\n']) {
            return Err(RemoteError::new(
                "INVALID_CREDENTIAL",
                "a credential value is invalid",
            ));
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HeaderValue {
    Public(String),
    Secret(SecretValue),
}

impl HeaderValue {
    pub fn expose(&self) -> &str {
        match self {
            Self::Public(value) => value,
            Self::Secret(value) => value.expose(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpHeader {
    pub name: String,
    pub value: HeaderValue,
}

#[derive(Clone, Eq, PartialEq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<HttpHeader>,
    pub body: Vec<u8>,
    pub response_limit: usize,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("body_bytes", &self.body.len())
            .field("response_limit", &self.response_limit)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

pub trait Cancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub trait Sleeper: Send + Sync {
    fn sleep(&self, duration: Duration, cancellation: &dyn Cancellation)
    -> Result<(), RemoteError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep(
        &self,
        duration: Duration,
        cancellation: &dyn Cancellation,
    ) -> Result<(), RemoteError> {
        let mut remaining = duration;
        while !remaining.is_zero() {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let part = remaining.min(Duration::from_millis(100));
            std::thread::sleep(part);
            remaining = remaining.saturating_sub(part);
        }
        Ok(())
    }
}

pub trait HttpAdapter: Send + Sync {
    fn send(
        &self,
        request: &HttpRequest,
        cancellation: &dyn Cancellation,
        timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError>;
}

pub trait TokenProvider: Send + Sync {
    fn access_token(&self) -> Result<Option<SecretValue>, RemoteError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoTokenProvider;

impl TokenProvider for NoTokenProvider {
    fn access_token(&self) -> Result<Option<SecretValue>, RemoteError> {
        Ok(None)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedRemoteSkill {
    pub files: Vec<PreparedFile>,
    pub locked_source: LockedSource,
    pub source_status: SourceStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteComparisonAccess {
    PublicGithub,
    Hosted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteUpdateComparison {
    pub id: String,
    pub owner: String,
    pub repository: String,
    pub base_sha: CommitSha,
    pub head_sha: CommitSha,
    pub access: RemoteComparisonAccess,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteLatestCommit {
    pub commit_sha: CommitSha,
    pub access: RemoteComparisonAccess,
}

impl RemoteUpdateComparison {
    pub fn new(
        id: impl Into<String>,
        owner: impl Into<String>,
        repository: impl Into<String>,
        base_sha: CommitSha,
        head_sha: CommitSha,
        access: RemoteComparisonAccess,
    ) -> Result<Self, RemoteError> {
        let comparison = Self {
            id: id.into(),
            owner: owner.into(),
            repository: repository.into(),
            base_sha,
            head_sha,
            access,
        };
        validate_update_comparison(&comparison)?;
        Ok(comparison)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RemoteComparisonRelation {
    Ahead,
    Behind,
    Diverged,
    Identical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteComparisonOutcome {
    Ready {
        relation: RemoteComparisonRelation,
        ahead_by: u64,
        behind_by: u64,
        commits: Vec<CommitSummary>,
        total: u64,
        truncated: bool,
        compare_url: String,
    },
    NotFound,
    InvalidComparison,
    RateLimited {
        retry_after_seconds: Option<u64>,
        reset_at: Option<String>,
    },
    ProviderFailure {
        status: Option<u16>,
    },
    RequestFailure {
        code: &'static str,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteUpdateResult {
    pub id: String,
    pub outcome: RemoteComparisonOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteSourceState {
    Current,
    Stale {
        current_artifact_id: String,
        current_commit_sha: String,
    },
}

pub trait RemoteProvider: Send + Sync {
    fn search(&self, query: &str, limit: u8) -> Result<SearchResponse, RemoteError>;

    fn prepare(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError>;

    fn prepare_exact(
        &self,
        selector: &RemoteSelector,
        expected_commit: &CommitSha,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError>;

    fn source_state(
        &self,
        selector: &RemoteSelector,
        artifact_id: &str,
        commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError>;

    fn latest_commit(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<RemoteLatestCommit, RemoteError>;

    fn compare_updates(
        &self,
        comparisons: &[RemoteUpdateComparison],
    ) -> Result<Vec<RemoteUpdateResult>, RemoteError>;

    /// List every Skill a Repository, curator, or collection names.
    fn list_skills(&self, reference: &MultiSkillRef) -> Result<SkillListing, RemoteError> {
        let _ = reference;
        Err(RemoteError::new(
            "NOT_IMPLEMENTED",
            "this remote provider lists no Skills",
        ))
    }
}

#[derive(Clone)]
enum UpdateComparisonTask {
    Public {
        index: usize,
        input: RemoteUpdateComparison,
    },
    Hosted {
        inputs: Vec<(usize, RemoteUpdateComparison)>,
    },
}

impl UpdateComparisonTask {
    fn indexed_inputs(&self) -> Vec<(usize, &RemoteUpdateComparison)> {
        match self {
            Self::Public { index, input } => vec![(*index, input)],
            Self::Hosted { inputs } => inputs
                .iter()
                .map(|(index, input)| (*index, input))
                .collect(),
        }
    }
}

#[derive(Clone)]
struct ComparisonPageReady {
    relation: RemoteComparisonRelation,
    ahead_by: u64,
    behind_by: u64,
    commits: Vec<CommitSummary>,
    total: u64,
    page: usize,
}

enum ComparisonPage {
    Ready(ComparisonPageReady),
    NotFound,
    InvalidComparison,
    RateLimited {
        retry_after_seconds: Option<u64>,
        reset_at: Option<String>,
    },
    ProviderFailure {
        status: Option<u16>,
    },
}

impl ComparisonPage {
    fn into_outcome(self) -> RemoteComparisonOutcome {
        match self {
            Self::Ready(_) => unreachable!("ready pages are handled before conversion"),
            Self::NotFound => RemoteComparisonOutcome::NotFound,
            Self::InvalidComparison => RemoteComparisonOutcome::InvalidComparison,
            Self::RateLimited {
                retry_after_seconds,
                reset_at,
            } => RemoteComparisonOutcome::RateLimited {
                retry_after_seconds,
                reset_at,
            },
            Self::ProviderFailure { status } => RemoteComparisonOutcome::ProviderFailure { status },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HostedComparisonsRequest<'a> {
    comparisons: Vec<HostedComparison<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HostedComparison<'a> {
    id: &'a str,
    owner: &'a str,
    repository: &'a str,
    base_sha: &'a str,
    head_sha: &'a str,
}

impl<'a> From<&'a RemoteUpdateComparison> for HostedComparison<'a> {
    fn from(input: &'a RemoteUpdateComparison) -> Self {
        Self {
            id: &input.id,
            owner: &input.owner,
            repository: &input.repository,
            base_sha: input.base_sha.as_str(),
            head_sha: input.head_sha.as_str(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostedComparisonsResponse {
    results: Vec<HostedComparisonResult>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, tag = "_tag", rename_all = "snake_case")]
enum HostedComparisonResult {
    Ready {
        id: String,
        owner: String,
        repository: String,
        #[serde(rename = "baseSha")]
        base_sha: String,
        #[serde(rename = "headSha")]
        head_sha: String,
        relation: RemoteComparisonRelation,
        #[serde(rename = "aheadBy")]
        ahead_by: u64,
        #[serde(rename = "behindBy")]
        behind_by: u64,
        commits: Vec<HostedCommit>,
        total: u64,
        truncated: bool,
        #[serde(rename = "compareUrl")]
        compare_url: String,
    },
    NotFound {
        id: String,
        owner: String,
        repository: String,
        #[serde(rename = "baseSha")]
        base_sha: String,
        #[serde(rename = "headSha")]
        head_sha: String,
    },
    InvalidComparison {
        id: String,
        owner: String,
        repository: String,
        #[serde(rename = "baseSha")]
        base_sha: String,
        #[serde(rename = "headSha")]
        head_sha: String,
    },
    RateLimited {
        id: String,
        owner: String,
        repository: String,
        #[serde(rename = "baseSha")]
        base_sha: String,
        #[serde(rename = "headSha")]
        head_sha: String,
        #[serde(rename = "retryAfterSeconds")]
        retry_after_seconds: Option<u64>,
        #[serde(rename = "resetAt")]
        reset_at: Option<String>,
    },
    ProviderFailure {
        id: String,
        owner: String,
        repository: String,
        #[serde(rename = "baseSha")]
        base_sha: String,
        #[serde(rename = "headSha")]
        head_sha: String,
        status: Option<u16>,
    },
}

impl HostedComparisonResult {
    fn identity(&self) -> (&str, &str, &str, &str, &str) {
        match self {
            Self::Ready {
                id,
                owner,
                repository,
                base_sha,
                head_sha,
                ..
            }
            | Self::NotFound {
                id,
                owner,
                repository,
                base_sha,
                head_sha,
            }
            | Self::InvalidComparison {
                id,
                owner,
                repository,
                base_sha,
                head_sha,
            }
            | Self::RateLimited {
                id,
                owner,
                repository,
                base_sha,
                head_sha,
                ..
            }
            | Self::ProviderFailure {
                id,
                owner,
                repository,
                base_sha,
                head_sha,
                ..
            } => (id, owner, repository, base_sha, head_sha),
        }
    }

    fn id(&self) -> &str {
        self.identity().0
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct HostedCommit {
    sha: String,
    subject: String,
    timestamp: String,
    author: HostedCommitAuthor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostedCommitAuthor {
    name: String,
    login: Option<String>,
}

#[derive(Deserialize)]
struct GithubComparisonWire {
    status: RemoteComparisonRelation,
    ahead_by: u64,
    behind_by: u64,
    total_commits: u64,
    commits: Vec<GithubComparisonCommit>,
}

#[derive(Deserialize)]
struct GithubComparisonCommit {
    sha: String,
    commit: GithubCommitDetails,
    author: Option<GithubCommitAuthor>,
}

#[derive(Deserialize)]
struct GithubCommitDetails {
    message: String,
    author: GithubGitAuthor,
}

#[derive(Deserialize)]
struct GithubGitAuthor {
    name: String,
    date: String,
}

#[derive(Deserialize)]
struct GithubCommitAuthor {
    login: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeRemoteConfig {
    Pinned(TrustedRootPin),
    Unconfigured,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteProgressStage {
    RequestingResolution,
    Requested,
    Resolving,
    Fetching,
    Checking,
    Packaging,
    Encrypting,
    Signing,
    Publishing,
    RetryWait,
    VerifyingAttestation,
    RequestingDownload,
    DownloadingArtifact,
    VerifyingArtifact,
}

pub trait RemoteProgress: Send + Sync {
    fn stage(&self, stage: RemoteProgressStage);
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RemotePendingStage {
    Known(RemoteProgressStage),
    #[allow(dead_code)]
    Unknown(String),
}

#[derive(Default)]
pub struct NoRemoteProgress;

impl RemoteProgress for NoRemoteProgress {
    fn stage(&self, _stage: RemoteProgressStage) {}
}

pub struct SkilldRemote {
    adapter: Arc<dyn HttpAdapter>,
    tokens: Arc<dyn TokenProvider>,
    cancellation: Arc<dyn Cancellation>,
    sleeper: Arc<dyn Sleeper>,
    progress: Arc<dyn RemoteProgress>,
    endpoint: Url,
    root_pin: NativeRemoteConfig,
}

impl SkilldRemote {
    pub fn new(
        adapter: Arc<dyn HttpAdapter>,
        tokens: Arc<dyn TokenProvider>,
        root_pin: NativeRemoteConfig,
    ) -> Self {
        Self {
            adapter,
            tokens,
            cancellation: Arc::new(NeverCancelled),
            sleeper: Arc::new(ThreadSleeper),
            progress: Arc::new(NoRemoteProgress),
            endpoint: Url::parse("https://skilld.dev").expect("the fixed endpoint is valid"),
            root_pin,
        }
    }

    pub fn with_cancellation(mut self, cancellation: Arc<dyn Cancellation>) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn with_sleeper(mut self, sleeper: Arc<dyn Sleeper>) -> Self {
        self.sleeper = sleeper;
        self
    }

    pub fn with_progress(mut self, progress: Arc<dyn RemoteProgress>) -> Self {
        self.progress = progress;
        self
    }

    pub fn with_endpoint(mut self, endpoint: &str) -> Result<Self, RemoteError> {
        let endpoint = Url::parse(endpoint)
            .map_err(|_| RemoteError::new("INVALID_ENDPOINT", "the API endpoint is invalid"))?;
        let production = endpoint.scheme() == "https" && endpoint.host_str().is_some();
        let local_test = endpoint.scheme() == "http"
            && matches!(endpoint.host_str(), Some("127.0.0.1" | "localhost"));
        if !production && !local_test {
            return Err(RemoteError::new(
                "INVALID_ENDPOINT",
                "the API endpoint must use HTTPS",
            ));
        }
        self.endpoint = endpoint;
        Ok(self)
    }

    fn service_url(&self, path: &str) -> Result<Url, RemoteError> {
        self.endpoint
            .join(path)
            .map_err(|_| RemoteError::new("INVALID_ENDPOINT", "the API route is invalid"))
    }

    fn execute(
        &self,
        request: HttpRequest,
        allowed: AllowedOrigin,
    ) -> Result<HttpResponse, RemoteError> {
        self.execute_with_deadline(request, allowed, None)
    }

    fn execute_with_deadline(
        &self,
        mut request: HttpRequest,
        allowed: AllowedOrigin,
        mut deadline: Option<&mut ResolutionDeadline>,
    ) -> Result<HttpResponse, RemoteError> {
        let mut redirects = 0_usize;
        loop {
            validate_request_url(&request.url, &allowed)?;
            let mut retry = 0_usize;
            let response = loop {
                if self.cancellation.is_cancelled() {
                    return Err(cancelled());
                }
                let timeout = deadline
                    .as_deref()
                    .map(ResolutionDeadline::remaining)
                    .transpose()?;
                let response = self
                    .adapter
                    .send(&request, self.cancellation.as_ref(), timeout);
                if self.cancellation.is_cancelled() {
                    return Err(cancelled());
                }
                if let Some(deadline) = deadline.as_deref() {
                    deadline.remaining()?;
                }
                match response {
                    Ok(response) => {
                        if response.body.len() > request.response_limit {
                            return Err(RemoteError::new(
                                "RESPONSE_TOO_LARGE",
                                "a remote response exceeded its limit",
                            ));
                        }
                        if matches!(response.status, 429 | 503) && retry < MAX_RETRIES {
                            retry += 1;
                            self.sleep(retry_delay(&response, retry), deadline.as_deref_mut())?;
                            continue;
                        }
                        break response;
                    }
                    Err(error) if error.code == "HTTP_TRANSPORT" && retry < MAX_RETRIES => {
                        retry += 1;
                        self.sleep(
                            Duration::from_millis(100 * retry as u64),
                            deadline.as_deref_mut(),
                        )?;
                    }
                    Err(error) => return Err(error),
                }
            };
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                if request.method != HttpMethod::Get || redirects == MAX_REDIRECTS {
                    return Err(RemoteError::new(
                        "REDIRECT_REJECTED",
                        "the remote response used a rejected redirect",
                    ));
                }
                let location = response.header("location").ok_or_else(|| {
                    RemoteError::new("REDIRECT_REJECTED", "the redirect has no location")
                })?;
                let current = Url::parse(&request.url).map_err(|_| {
                    RemoteError::new("REDIRECT_REJECTED", "the request URL is invalid")
                })?;
                let next = current.join(location).map_err(|_| {
                    RemoteError::new("REDIRECT_REJECTED", "the redirect URL is invalid")
                })?;
                validate_url(&next, &allowed)?;
                request.url = next.into();
                redirects += 1;
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(problem_error(&response));
            }
            return Ok(response);
        }
    }

    fn sleep(
        &self,
        duration: Duration,
        deadline: Option<&mut ResolutionDeadline>,
    ) -> Result<(), RemoteError> {
        match deadline {
            Some(deadline) => {
                deadline.sleep(duration, self.sleeper.as_ref(), self.cancellation.as_ref())
            }
            None => self.sleeper.sleep(duration, self.cancellation.as_ref()),
        }
    }

    fn authenticated_headers(&self) -> Result<Vec<HttpHeader>, RemoteError> {
        self.tokens.access_token().map(|token| {
            token
                .map(|token| HttpHeader {
                    name: "authorization".to_owned(),
                    value: HeaderValue::Secret(
                        SecretValue::new(format!("Bearer {}", token.expose()))
                            .expect("a parsed token cannot contain a newline"),
                    ),
                })
                .into_iter()
                .collect()
        })
    }

    fn verified_root(&self) -> Result<VerifiedTrustedRoot, RemoteError> {
        let NativeRemoteConfig::Pinned(pin) = &self.root_pin else {
            return Err(RemoteError::new(
                "TRUSTED_ROOT_UNCONFIGURED",
                "this build has no trusted root pin",
            ));
        };
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: self.service_url("/api/v1/trusted-root")?.into(),
            headers: vec![],
            body: vec![],
            response_limit: JSON_LIMIT,
        };
        let response = self.execute(request, AllowedOrigin::Service(self.endpoint.clone()))?;
        let root: TrustedRoot = parse_json(&response.body)?;
        verify_trusted_root(root, pin)
    }

    fn resolve(&self, source: &SourceRequest) -> Result<ArtifactDescriptor, RemoteError> {
        let mut deadline = ResolutionDeadline::new();
        self.progress
            .stage(RemoteProgressStage::RequestingResolution);
        let body = serde_json::to_vec(&json!({ "source": source })).map_err(|_| {
            RemoteError::new("INVALID_SOURCE", "the source request cannot be encoded")
        })?;
        let mut headers = self.authenticated_headers()?;
        headers.extend(json_headers());
        headers.push(idempotency_header());
        let request = HttpRequest {
            method: HttpMethod::Post,
            url: self.service_url("/api/v1/resolutions")?.into(),
            headers,
            body,
            response_limit: JSON_LIMIT,
        };
        let response = self.execute_with_deadline(
            request,
            AllowedOrigin::Service(self.endpoint.clone()),
            Some(&mut deadline),
        )?;
        let mut resolution: Resolution = parse_json(&response.body)?;
        let resolution_id = resolution.resolution_id().to_owned();
        if !valid_resolution_id(&resolution_id) {
            return Err(RemoteError::new(
                "INVALID_RESPONSE",
                "the Resolution identifier is invalid",
            ));
        }
        for _ in 0..MAX_POLLS {
            if resolution.resolution_id() != resolution_id {
                return Err(RemoteError::new(
                    "INVALID_RESPONSE",
                    "the Resolution identifier changed",
                ));
            }
            match resolution {
                Resolution::Ready { artifact, .. } => {
                    if artifact.artifact_id != artifact.attestation.artifact_id
                        || artifact.visibility != artifact.attestation.source.visibility
                    {
                        return Err(RemoteError::new(
                            "ATTESTATION_MISMATCH",
                            "the Resolution Artifact does not match its attestation",
                        ));
                    }
                    validate_resolved_source(source, &artifact.attestation)?;
                    return Ok(*artifact);
                }
                Resolution::Pending {
                    resolution_id,
                    stage,
                    poll_after_ms,
                } => {
                    if let RemotePendingStage::Known(stage) = stage {
                        self.progress.stage(stage);
                    }
                    if !(250..=60_000).contains(&poll_after_ms) {
                        return Err(RemoteError::new(
                            "INVALID_RESPONSE",
                            "the Resolution poll interval is invalid",
                        ));
                    }
                    deadline.sleep(
                        Duration::from_millis(poll_after_ms),
                        self.sleeper.as_ref(),
                        self.cancellation.as_ref(),
                    )?;
                    let path = format!("/api/v1/resolutions/{}", path_segment(&resolution_id));
                    let request = HttpRequest {
                        method: HttpMethod::Get,
                        url: self.service_url(&path)?.into(),
                        headers: self.authenticated_headers()?,
                        body: vec![],
                        response_limit: JSON_LIMIT,
                    };
                    let response = self.execute_with_deadline(
                        request,
                        AllowedOrigin::Service(self.endpoint.clone()),
                        Some(&mut deadline),
                    )?;
                    resolution = parse_json(&response.body)?;
                }
                Resolution::Blocked { .. } => {
                    return Err(RemoteError::new(
                        "CHECK_BLOCKED",
                        "the Resolution was blocked by check results",
                    ));
                }
                Resolution::Failed {
                    code, retryable, ..
                } => {
                    return Err(RemoteError::new(
                        problem_code(&code),
                        if retryable {
                            "the Resolution failed and may be retried"
                        } else {
                            "the Resolution failed"
                        },
                    ));
                }
                Resolution::Revoked { .. } => {
                    return Err(RemoteError::new(
                        "ARTIFACT_REVOKED",
                        "the Resolution was revoked",
                    ));
                }
            }
        }
        Err(resolution_timeout())
    }

    fn grant(&self, artifact_id: &str) -> Result<ArtifactGrant, RemoteError> {
        let path = format!("/api/v1/artifacts/{}/grants", path_segment(artifact_id));
        let mut headers = self.authenticated_headers()?;
        headers.push(idempotency_header());
        let request = HttpRequest {
            method: HttpMethod::Post,
            url: self.service_url(&path)?.into(),
            headers,
            body: vec![],
            response_limit: JSON_LIMIT,
        };
        let response = self.execute(request, AllowedOrigin::Service(self.endpoint.clone()))?;
        parse_json(&response.body)
    }

    fn download_grant(
        &self,
        descriptor: &ArtifactDescriptor,
        grant: ArtifactGrant,
    ) -> Result<Vec<u8>, RemoteError> {
        let (artifact_id, content_url, attestation, headers) = match grant {
            ArtifactGrant::Public {
                artifact_id,
                content_url,
                attestation,
                ..
            } => (artifact_id, content_url, attestation, vec![]),
            ArtifactGrant::Private {
                artifact_id,
                content_url,
                download_token,
                attestation,
                ..
            } => {
                let account = self.tokens.access_token()?.ok_or_else(|| {
                    RemoteError::new(
                        "AUTH_REQUIRED",
                        "private Artifact delivery needs an account",
                    )
                })?;
                (
                    artifact_id,
                    content_url,
                    attestation,
                    vec![
                        HttpHeader {
                            name: "authorization".to_owned(),
                            value: HeaderValue::Secret(SecretValue::new(format!(
                                "Bearer {}",
                                account.expose()
                            ))?),
                        },
                        HttpHeader {
                            name: "x-skilld-grant".to_owned(),
                            value: HeaderValue::Secret(SecretValue::new(download_token)?),
                        },
                    ],
                )
            }
        };
        if artifact_id != descriptor.artifact_id || attestation != descriptor.attestation {
            return Err(RemoteError::new(
                "ATTESTATION_MISMATCH",
                "the Artifact grant does not match its Resolution",
            ));
        }
        let limit = usize::try_from(attestation.content_bytes)
            .ok()
            .and_then(|value| value.checked_add(1))
            .filter(|value| *value <= ARTIFACT_LIMIT + 1)
            .ok_or_else(|| {
                RemoteError::new("RESPONSE_TOO_LARGE", "the Artifact exceeds the size limit")
            })?;
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: content_url,
            headers,
            body: vec![],
            response_limit: limit,
        };
        self.execute(request, AllowedOrigin::Service(self.endpoint.clone()))
            .map(|response| response.body)
    }

    fn github_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        limit: usize,
    ) -> Result<T, RemoteError> {
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: url.to_owned(),
            headers: github_headers(),
            body: vec![],
            response_limit: limit,
        };
        let response = self.execute(request, AllowedOrigin::Github)?;
        parse_json(&response.body)
    }

    fn direct_commit(
        &self,
        selector: &RemoteSelector,
    ) -> Result<(String, String, GithubCommit), RemoteError> {
        if !selector.is_explicit_github() {
            return Err(RemoteError::new(
                "DIRECT_SOURCE_REQUIRED",
                crate::DIRECT_SOURCE_GUIDANCE,
            ));
        }
        let source = selector.source();
        let SourceSelector::Path { path: skill_path } = &source.selector else {
            return Err(RemoteError::new(
                "DIRECT_SOURCE_REQUIRED",
                crate::DIRECT_SOURCE_GUIDANCE,
            ));
        };
        let repository_url = format!(
            "https://api.github.com/repos/{}/{}",
            path_segment(&source.owner),
            path_segment(&source.repository)
        );
        let repository: GithubRepository = self.github_json(&repository_url, JSON_LIMIT)?;
        if repository.private {
            return Err(RemoteError::new(
                "DIRECT_PRIVATE_UNSUPPORTED",
                "--direct cannot use a private Repository",
            ));
        }
        let reference = source.r#ref.as_ref().map_or_else(
            || repository.default_branch.clone(),
            |reference| match reference {
                SourceRef::Branch { value }
                | SourceRef::Tag { value }
                | SourceRef::Commit { value } => value.clone(),
            },
        );
        let commit_url = format!("{repository_url}/commits/{}", path_segment(&reference));
        let commit: GithubCommit = self.github_json(&commit_url, JSON_LIMIT)?;
        if !valid_sha(&commit.sha) || !valid_sha(&commit.commit.tree.sha) {
            return Err(invalid_github());
        }
        if matches!(
            &source.r#ref,
            Some(SourceRef::Commit { value }) if value != &commit.sha
        ) {
            return Err(RemoteError::new(
                "SOURCE_MISMATCH",
                "GitHub resolved a different commit than requested",
            ));
        }
        Ok((repository_url, skill_path.clone(), commit))
    }

    fn public_comparison(
        &self,
        input: &RemoteUpdateComparison,
    ) -> Result<RemoteComparisonOutcome, RemoteError> {
        let first = self.github_comparison_page(input, 1)?;
        let ComparisonPage::Ready(first) = first else {
            return Ok(first.into_outcome());
        };
        let relation = first.relation;
        let ahead_by = first.ahead_by;
        let behind_by = first.behind_by;
        let total = first.total;
        let first_included_index = total.saturating_sub(MAX_UPDATE_COMMITS as u64);
        let first_included_page =
            usize::try_from(first_included_index / UPDATE_COMMITS_PER_PAGE as u64 + 1)
                .map_err(|_| invalid_update_response())?;
        let first_page_offset =
            usize::try_from(first_included_index % UPDATE_COMMITS_PER_PAGE as u64)
                .map_err(|_| invalid_update_response())?;
        let last_included_page =
            usize::try_from(total.div_ceil(UPDATE_COMMITS_PER_PAGE as u64).max(1))
                .map_err(|_| invalid_update_response())?;
        let mut commits = Vec::new();
        for page in first_included_page..=last_included_page {
            let page = if page == 1 {
                ComparisonPage::Ready(first.clone())
            } else {
                self.github_comparison_page(input, page)?
            };
            let ComparisonPage::Ready(page) = page else {
                return Ok(page.into_outcome());
            };
            if page.relation != relation
                || page.ahead_by != ahead_by
                || page.behind_by != behind_by
                || page.total != total
            {
                return Err(invalid_update_response());
            }
            let offset = if page.page == first_included_page {
                first_page_offset
            } else {
                0
            };
            commits.extend(page.commits.into_iter().skip(offset));
        }
        if commits.len() > MAX_UPDATE_COMMITS {
            commits.drain(..commits.len() - MAX_UPDATE_COMMITS);
        }
        let expected_count = usize::try_from(total.min(MAX_UPDATE_COMMITS as u64))
            .map_err(|_| invalid_update_response())?;
        let unique_count = commits
            .iter()
            .map(|commit| commit.sha.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        if commits.len() != expected_count || unique_count != commits.len() {
            return Err(invalid_update_response());
        }
        let compare_url = github_compare_url(input);
        Ok(RemoteComparisonOutcome::Ready {
            relation,
            ahead_by,
            behind_by,
            truncated: u64::try_from(commits.len()).unwrap_or(u64::MAX) < total,
            commits,
            total,
            compare_url,
        })
    }

    fn github_comparison_page(
        &self,
        input: &RemoteUpdateComparison,
        page: usize,
    ) -> Result<ComparisonPage, RemoteError> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/compare/{}...{}?per_page={UPDATE_COMMITS_PER_PAGE}&page={page}",
            path_segment(&input.owner),
            path_segment(&input.repository),
            input.base_sha.as_str(),
            input.head_sha.as_str(),
        );
        let request = HttpRequest {
            method: HttpMethod::Get,
            url,
            headers: github_headers(),
            body: vec![],
            response_limit: JSON_LIMIT,
        };
        let response = self.execute_comparison_request(request, AllowedOrigin::Github)?;
        if is_rate_limited(&response) {
            return Ok(ComparisonPage::RateLimited {
                retry_after_seconds: bounded_rate_limit_wait(&response),
                reset_at: None,
            });
        }
        if response.status == 404 {
            return Ok(ComparisonPage::NotFound);
        }
        if matches!(response.status, 409 | 422) {
            return Ok(ComparisonPage::InvalidComparison);
        }
        if !(200..300).contains(&response.status) {
            return Ok(ComparisonPage::ProviderFailure {
                status: Some(response.status),
            });
        }
        let wire: GithubComparisonWire = parse_json(&response.body)?;
        if wire.commits.len() > UPDATE_COMMITS_PER_PAGE
            || wire.total_commits > MAX_SAFE_INTEGER
            || wire.ahead_by > MAX_SAFE_INTEGER
            || wire.behind_by > MAX_SAFE_INTEGER
            || wire.total_commits != wire.ahead_by
            || !comparison_counts_match(
                wire.status,
                input.base_sha == input.head_sha,
                wire.ahead_by,
                wire.behind_by,
            )
        {
            return Err(invalid_update_response());
        }
        let commits = wire
            .commits
            .into_iter()
            .map(|commit| present_commit(input, commit))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ComparisonPage::Ready(ComparisonPageReady {
            relation: wire.status,
            ahead_by: wire.ahead_by,
            behind_by: wire.behind_by,
            total: wire.total_commits,
            commits,
            page,
        }))
    }

    fn hosted_comparisons(
        &self,
        inputs: &[RemoteUpdateComparison],
    ) -> Result<Vec<RemoteUpdateResult>, RemoteError> {
        let mut headers = self.authenticated_headers()?;
        if !headers.iter().any(|header| header.name == "authorization") {
            return Err(RemoteError::new(
                "AUTH_REQUIRED",
                "Hosted update checks need an account",
            ));
        }
        headers.extend(json_headers());
        let body = serde_json::to_vec(&HostedComparisonsRequest {
            comparisons: inputs.iter().map(HostedComparison::from).collect(),
        })
        .map_err(|_| {
            RemoteError::new("INVALID_SOURCE", "Update comparisons could not be encoded")
        })?;
        let request = HttpRequest {
            method: HttpMethod::Post,
            url: self.service_url("/api/v1/update-plans")?.into(),
            headers,
            body,
            response_limit: UPDATE_RESPONSE_LIMIT,
        };
        let response = self
            .execute_comparison_request(request, AllowedOrigin::Service(self.endpoint.clone()))?;
        if is_rate_limited(&response) {
            let retry_after_seconds = bounded_rate_limit_wait(&response);
            return Ok(inputs
                .iter()
                .map(|input| RemoteUpdateResult {
                    id: input.id.clone(),
                    outcome: RemoteComparisonOutcome::RateLimited {
                        retry_after_seconds,
                        reset_at: None,
                    },
                })
                .collect());
        }
        if !(200..300).contains(&response.status) {
            return Err(problem_error(&response));
        }
        let wire: HostedComparisonsResponse = parse_json(&response.body)?;
        if wire.results.len() != inputs.len() || wire.results.len() > UPDATE_SERVICE_BATCH {
            return Err(invalid_update_response());
        }
        let mut by_id = BTreeMap::new();
        for result in wire.results {
            let id = result.id().to_owned();
            if by_id.contains_key(&id) {
                return Err(invalid_update_response());
            }
            let input = inputs
                .iter()
                .find(|input| input.id == id)
                .ok_or_else(invalid_update_response)?;
            let outcome = validate_hosted_identity(input, &result)
                .and_then(|()| present_hosted_outcome(input, result))
                .unwrap_or_else(|error| RemoteComparisonOutcome::RequestFailure {
                    code: error.code,
                    message: error.message,
                });
            by_id.insert(id, outcome);
        }
        if by_id.len() != inputs.len() {
            return Err(invalid_update_response());
        }
        inputs
            .iter()
            .map(|input| {
                by_id
                    .remove(&input.id)
                    .map(|outcome| RemoteUpdateResult {
                        id: input.id.clone(),
                        outcome,
                    })
                    .ok_or_else(invalid_update_response)
            })
            .collect()
    }

    fn execute_comparison_request(
        &self,
        request: HttpRequest,
        allowed: AllowedOrigin,
    ) -> Result<HttpResponse, RemoteError> {
        validate_request_url(&request.url, &allowed)?;
        let mut attempts = 0;
        loop {
            if self.cancellation.is_cancelled() {
                return Err(cancelled());
            }
            match self
                .adapter
                .send(&request, self.cancellation.as_ref(), None)
            {
                Ok(response) => {
                    if response.body.len() > request.response_limit {
                        return Err(RemoteError::new(
                            "RESPONSE_TOO_LARGE",
                            "A comparison response exceeded its limit",
                        ));
                    }
                    if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                        return Err(RemoteError::new(
                            "REDIRECT_REJECTED",
                            "A comparison response used a redirect",
                        ));
                    }
                    return Ok(response);
                }
                Err(error) if error.code == "HTTP_TRANSPORT" && attempts < MAX_RETRIES => {
                    attempts += 1;
                    self.sleep(Duration::from_millis(100 * attempts as u64), None)?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn direct(&self, selector: &RemoteSelector) -> Result<PreparedRemoteSkill, RemoteError> {
        let (repository_url, skill_path, commit) = self.direct_commit(selector)?;
        let tree_url = format!(
            "{repository_url}/git/trees/{}?recursive=1",
            path_segment(&commit.commit.tree.sha)
        );
        let tree: GithubTree = self.github_json(&tree_url, JSON_LIMIT)?;
        if tree.truncated || tree.tree.len() > 20_000 {
            return Err(RemoteError::new(
                "DIRECT_SOURCE_TOO_LARGE",
                "the GitHub Repository tree exceeds the direct access limit",
            ));
        }
        let prefix = skill_path.trim_end_matches('/');
        let prefix_with_slash = format!("{prefix}/");
        let mut files = Vec::new();
        let mut total = 0_u64;
        for entry in tree.tree {
            if entry.path == prefix && entry.kind != "tree" {
                return Err(invalid_github());
            }
            let Some(relative) = entry.path.strip_prefix(&prefix_with_slash) else {
                continue;
            };
            if entry.kind == "tree" {
                continue;
            }
            if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
                return Err(RemoteError::new(
                    "DIRECT_SOURCE_UNSUPPORTED",
                    "direct GitHub access rejects links and submodules",
                ));
            }
            let size = entry.size.ok_or_else(invalid_github)?;
            total = total.checked_add(size).ok_or_else(invalid_github)?;
            if size > 8 * 1024 * 1024 || total > ARTIFACT_LIMIT as u64 || files.len() == 2_000 {
                return Err(RemoteError::new(
                    "DIRECT_SOURCE_TOO_LARGE",
                    "the direct Skill exceeds its content limit",
                ));
            }
            let blob_url = format!("{repository_url}/git/blobs/{}", path_segment(&entry.sha));
            let blob: GithubBlob = self.github_json(&blob_url, DIRECT_BLOB_LIMIT)?;
            if blob.encoding != "base64" || blob.size != size {
                return Err(invalid_github());
            }
            let encoded = blob.content.replace(['\r', '\n'], "");
            let bytes = STANDARD.decode(encoded).map_err(|_| invalid_github())?;
            if bytes.len() as u64 != size {
                return Err(invalid_github());
            }
            files.push(PreparedFile {
                path: relative.to_owned(),
                mode: if entry.mode == "100755" { 0o755 } else { 0o644 },
                bytes,
            });
        }
        let (_name, installed_sha256, files) = prepare_unverified_files(files)?;
        Ok(PreparedRemoteSkill {
            locked_source: LockedSource::Remote {
                source: selector.canonical(),
                commit_sha: commit.sha,
                skill_path,
            },
            source_status: SourceStatus::Unverified {
                content_sha256: installed_sha256.clone(),
                installed_sha256,
            },
            files,
        })
    }
}

/// One entry a collection names, before the Skills behind it are known.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CollectionEntry {
    owner: String,
    repository: String,
    /// `None` names every Skill the Repository carries.
    name: Option<String>,
    /// The curator's one-line reason for including it.
    reason: Option<String>,
}

/// `GET /api/skills?owner=` rows. The site returns more fields; skilld reads these.
#[derive(Deserialize)]
struct RegistrySkillPage {
    items: Vec<RegistrySkillRow>,
    total: Option<usize>,
    pages: Option<usize>,
}

#[derive(Deserialize)]
struct RegistrySkillRow {
    name: String,
    owner: String,
    repo: String,
    description: Option<String>,
}

/// `GET /api/curators/{login}`: the protocol `CuratorPayload` shape.
#[derive(Deserialize)]
struct CuratorPayload {
    collections: Vec<CollectionSummary>,
}

#[derive(Deserialize)]
struct CollectionSummary {
    slug: String,
}

/// `GET /api/collections/by-author/{login}/{slug}`. The site returns more
/// fields; skilld reads the resolved Skill rows.
#[derive(Deserialize)]
struct CollectionDetail {
    skills: Vec<CollectionSkillRow>,
}

#[derive(Deserialize)]
struct CollectionSkillRow {
    owner: String,
    repo: String,
    name: Option<String>,
    reason: Option<String>,
}

impl SkilldRemote {
    fn service_json<T: for<'de> Deserialize<'de>>(&self, url: Url) -> Result<T, RemoteError> {
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: url.into(),
            headers: vec![],
            body: vec![],
            response_limit: LISTING_LIMIT,
        };
        let response = self.execute(request, AllowedOrigin::Service(self.endpoint.clone()))?;
        parse_json(&response.body)
    }

    /// Every indexed Skill one GitHub account owns.
    fn owner_skills(&self, owner: &str) -> Result<Vec<ListedSkill>, RemoteError> {
        let first = self.owner_skills_page(owner, 1)?;
        let pages = first
            .pages
            .or_else(|| first.total.map(|total| total.div_ceil(LISTING_PAGE)))
            .unwrap_or(1);
        let mut rows = first.items;
        for page in 2..=pages {
            let next = self.owner_skills_page(owner, page)?;
            let received = next.items.len();
            rows.extend(next.items);
            if received < LISTING_PAGE {
                break;
            }
        }
        Ok(rows
            .into_iter()
            .filter(|row| row.owner.eq_ignore_ascii_case(owner))
            .filter_map(|row| {
                listed_skill(row.owner, row.repo, row.name, row.description.as_deref())
            })
            .collect())
    }

    fn owner_skills_page(
        &self,
        owner: &str,
        page: usize,
    ) -> Result<RegistrySkillPage, RemoteError> {
        let mut url = self.service_url("/api/skills")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("owner", owner);
            pairs.append_pair("limit", &LISTING_PAGE.to_string());
            if page > 1 {
                pairs.append_pair("page", &page.to_string());
            }
        }
        self.service_json(url)
    }

    /// Every Skill one Repository carries, by name.
    fn repository_skills(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<Vec<ListedSkill>, RemoteError> {
        let mut items = self
            .owner_skills(owner)?
            .into_iter()
            .filter(|skill| skill.repository.eq_ignore_ascii_case(repository))
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(items)
    }

    fn collection_entries(
        &self,
        login: &str,
        slug: &str,
    ) -> Result<Vec<CollectionEntry>, RemoteError> {
        let path = format!(
            "/api/collections/by-author/{}/{}",
            path_segment(login),
            path_segment(slug)
        );
        let detail: CollectionDetail =
            self.service_json(self.service_url(&path)?)
                .map_err(|error| {
                    not_found_as_source(
                        error,
                        format!("skilld.dev has no collection @{login}/{slug}"),
                    )
                })?;
        Ok(detail
            .skills
            .into_iter()
            .map(|row| CollectionEntry {
                owner: row.owner,
                repository: row.repo,
                name: row.name,
                reason: row.reason,
            })
            .collect())
    }

    fn curator_slugs(&self, login: &str) -> Result<Vec<String>, RemoteError> {
        let path = format!("/api/curators/{}", path_segment(login));
        let curator: CuratorPayload =
            self.service_json(self.service_url(&path)?)
                .map_err(|error| {
                    not_found_as_source(error, format!("skilld.dev has no curator @{login}"))
                })?;
        Ok(curator
            .collections
            .into_iter()
            .map(|collection| collection.slug)
            .collect())
    }

    /// Turn collection entries into listed Skills, in collection order.
    /// An entry that names one Skill lists it with the curator's reason.
    /// An entry that names a Repository lists every Skill it carries.
    fn expand_entries(
        &self,
        entries: Vec<CollectionEntry>,
    ) -> Result<Vec<ListedSkill>, RemoteError> {
        let mut seen = BTreeSet::new();
        let mut items = Vec::new();
        for entry in entries {
            let expanded = match entry.name {
                Some(name) => {
                    listed_skill(entry.owner, entry.repository, name, entry.reason.as_deref())
                        .into_iter()
                        .collect()
                }
                None => self.repository_skills(&entry.owner, &entry.repository)?,
            };
            for skill in expanded {
                if seen.insert(skill.selector()) {
                    items.push(skill);
                }
            }
        }
        Ok(items)
    }
}

/// Build one listed Skill from untrusted registry fields.
///
/// A row outside the selector contract has no `skilld:` selector, so skilld
/// could not run or install it. Listing it would print a command that fails,
/// so the row is dropped.
fn listed_skill(
    owner: String,
    repository: String,
    name: String,
    description: Option<&str>,
) -> Option<ListedSkill> {
    let skill = ListedSkill {
        owner,
        repository,
        name,
        description: description
            .map(|value| sanitize_line(value, 500, ""))
            .filter(|value| !value.is_empty()),
    };
    RemoteSelector::parse(&skill.selector())
        .is_ok()
        .then_some(skill)
}

fn not_found_as_source(error: RemoteError, message: String) -> RemoteError {
    if error.code == "SERVICE_UNAVAILABLE" && error.message.ends_with("HTTP 404") {
        RemoteError::new("SOURCE_NOT_FOUND", message)
    } else {
        error
    }
}

impl RemoteProvider for SkilldRemote {
    fn list_skills(&self, reference: &MultiSkillRef) -> Result<SkillListing, RemoteError> {
        let items = match reference {
            MultiSkillRef::Repository { owner, repository } => {
                self.repository_skills(owner, repository)?
            }
            MultiSkillRef::Collection { login, slug } => {
                self.expand_entries(self.collection_entries(login, slug)?)?
            }
            MultiSkillRef::Curator { login } => {
                let mut entries = Vec::new();
                for slug in self.curator_slugs(login)? {
                    entries.extend(self.collection_entries(login, &slug)?);
                }
                self.expand_entries(entries)?
            }
        };
        Ok(SkillListing {
            reference: reference.clone(),
            items,
        })
    }

    fn search(&self, query: &str, limit: u8) -> Result<SearchResponse, RemoteError> {
        let query = query.trim();
        if query.is_empty() || query.len() > 200 || !(1..=50).contains(&limit) {
            return Err(RemoteError::new(
                "INVALID_SEARCH",
                "Skill search needs a query and a limit from 1 to 50",
            ));
        }
        let mut url = self.service_url("/api/v1/skills")?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("limit", &limit.to_string());
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: url.into(),
            headers: vec![],
            body: vec![],
            response_limit: SEARCH_LIMIT,
        };
        let response = self.execute(request, AllowedOrigin::Service(self.endpoint.clone()))?;
        parse_search_response(&response.body)
    }

    fn prepare(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        if direct {
            return self.direct(selector);
        }
        let descriptor = self.resolve(selector.source())?;
        self.progress
            .stage(RemoteProgressStage::VerifyingAttestation);
        let root = self.verified_root()?;
        verify_attestation(&descriptor.attestation, &root)?;
        self.progress.stage(RemoteProgressStage::RequestingDownload);
        let grant = self.grant(&descriptor.artifact_id)?;
        self.progress
            .stage(RemoteProgressStage::DownloadingArtifact);
        let archive = self.download_grant(&descriptor, grant)?;
        self.progress.stage(RemoteProgressStage::VerifyingArtifact);
        let verified = verify_artifact(descriptor.attestation, &root, &archive)?;
        if matches!(
            &selector.source().selector,
            SourceSelector::NamedSkill { name } if name != verified.name.as_str()
        ) {
            return Err(RemoteError::new(
                "SOURCE_MISMATCH",
                "the resolved Skill name does not match the selector",
            ));
        }
        Ok(PreparedRemoteSkill {
            locked_source: LockedSource::Remote {
                source: selector.canonical(),
                commit_sha: verified.attestation.source.commit_sha.clone(),
                skill_path: verified.attestation.source.skill_path.clone(),
            },
            source_status: SourceStatus::Verified {
                artifact_id: verified.attestation.artifact_id.clone(),
                content_sha256: verified.attestation.content_sha256.clone(),
                installed_sha256: verified.installed_sha256,
                attestation_key_id: verified.attestation.signature.key_id.clone(),
            },
            files: verified.files,
        })
    }

    fn prepare_exact(
        &self,
        selector: &RemoteSelector,
        expected_commit: &CommitSha,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        if let Some(SourceRef::Commit { value }) = &selector.source().r#ref
            && value != expected_commit.as_str()
        {
            return Err(RemoteError::new(
                "SOURCE_MISMATCH",
                "the source commit does not match the expected commit",
            ));
        }
        let mut source = selector.source().clone();
        source.r#ref = Some(SourceRef::Commit {
            value: expected_commit.as_str().to_owned(),
        });
        let exact = match selector {
            RemoteSelector::Skilld(_) => RemoteSelector::Skilld(source),
            RemoteSelector::Github(_) => RemoteSelector::Github(source),
        };
        let mut prepared = self.prepare(&exact, direct)?;
        let LockedSource::Remote {
            source, commit_sha, ..
        } = &mut prepared.locked_source
        else {
            return Err(RemoteError::new(
                "SOURCE_MISMATCH",
                "The prepared update has no remote source",
            ));
        };
        if commit_sha != expected_commit.as_str() {
            return Err(RemoteError::new(
                "SOURCE_MISMATCH",
                "The prepared update changed its exact commit",
            ));
        }
        *source = selector.canonical();
        Ok(prepared)
    }

    fn source_state(
        &self,
        selector: &RemoteSelector,
        artifact_id: &str,
        commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError> {
        let descriptor = self.resolve(selector.source())?;
        let root = self.verified_root()?;
        verify_attestation(&descriptor.attestation, &root)?;
        if descriptor.artifact_id == artifact_id
            && descriptor.attestation.source.commit_sha == commit_sha
        {
            Ok(RemoteSourceState::Current)
        } else {
            Ok(RemoteSourceState::Stale {
                current_artifact_id: descriptor.artifact_id,
                current_commit_sha: descriptor.attestation.source.commit_sha,
            })
        }
    }

    fn latest_commit(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<RemoteLatestCommit, RemoteError> {
        if direct {
            let commit_sha = self.direct_commit(selector).and_then(|(_, _, commit)| {
                CommitSha::parse(commit.sha).map_err(|_| invalid_github())
            })?;
            return Ok(RemoteLatestCommit {
                commit_sha,
                access: RemoteComparisonAccess::PublicGithub,
            });
        }
        let descriptor = self.resolve(selector.source())?;
        let root = self.verified_root()?;
        verify_attestation(&descriptor.attestation, &root)?;
        let access = match descriptor.visibility {
            RepositoryVisibility::Public => RemoteComparisonAccess::PublicGithub,
            RepositoryVisibility::Private => RemoteComparisonAccess::Hosted,
        };
        let commit_sha = CommitSha::parse(descriptor.attestation.source.commit_sha)
            .map_err(|_| invalid_update_response())?;
        Ok(RemoteLatestCommit { commit_sha, access })
    }

    fn compare_updates(
        &self,
        comparisons: &[RemoteUpdateComparison],
    ) -> Result<Vec<RemoteUpdateResult>, RemoteError> {
        if comparisons.is_empty() || comparisons.len() > MAX_UPDATE_COMPARISONS {
            return Err(RemoteError::new(
                "INVALID_UPDATE_COMPARISON",
                "Update checks need from 1 to 500 comparisons",
            ));
        }
        let mut identifiers = BTreeSet::new();
        for comparison in comparisons {
            validate_update_comparison(comparison)?;
            if !identifiers.insert(comparison.id.as_str()) {
                return Err(RemoteError::new(
                    "INVALID_UPDATE_COMPARISON",
                    "Update comparison identifiers must be unique",
                ));
            }
        }
        let mut tasks = comparisons
            .iter()
            .enumerate()
            .filter(|(_, input)| input.access == RemoteComparisonAccess::PublicGithub)
            .map(|(index, input)| UpdateComparisonTask::Public {
                index,
                input: input.clone(),
            })
            .collect::<Vec<_>>();
        let hosted = comparisons
            .iter()
            .enumerate()
            .filter(|(_, input)| input.access == RemoteComparisonAccess::Hosted)
            .map(|(index, input)| (index, input.clone()))
            .collect::<Vec<_>>();
        tasks.extend(hosted.chunks(UPDATE_SERVICE_BATCH).map(|chunk| {
            UpdateComparisonTask::Hosted {
                inputs: chunk.to_vec(),
            }
        }));
        let mut indexed = run_update_comparison_tasks(self, &tasks);
        indexed.sort_by_key(|(index, _)| *index);
        if indexed.len() != comparisons.len()
            || indexed
                .iter()
                .enumerate()
                .any(|(expected, (actual, _))| expected != *actual)
        {
            return Err(RemoteError::new(
                "INVALID_RESPONSE",
                "Update comparison results were incomplete",
            ));
        }
        Ok(indexed.into_iter().map(|(_, result)| result).collect())
    }
}

fn run_update_comparison_tasks(
    remote: &SkilldRemote,
    tasks: &[UpdateComparisonTask],
) -> Vec<(usize, RemoteUpdateResult)> {
    #[cfg(target_os = "wasi")]
    {
        return tasks
            .iter()
            .flat_map(|task| execute_update_comparison_task(remote, task))
            .collect();
    }
    #[cfg(not(target_os = "wasi"))]
    {
        let next = AtomicU64::new(0);
        let results = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..UPDATE_CONCURRENCY.min(tasks.len()) {
                scope.spawn(|| {
                    loop {
                        let index = usize::try_from(next.fetch_add(1, Ordering::Relaxed))
                            .unwrap_or(usize::MAX);
                        let Some(task) = tasks.get(index) else {
                            break;
                        };
                        results
                            .lock()
                            .expect("comparison result lock is available")
                            .extend(execute_update_comparison_task(remote, task));
                    }
                });
            }
        });
        results
            .into_inner()
            .expect("comparison result lock is available")
    }
}

fn execute_update_comparison_task(
    remote: &SkilldRemote,
    task: &UpdateComparisonTask,
) -> Vec<(usize, RemoteUpdateResult)> {
    let outcome = match task {
        UpdateComparisonTask::Public { index, input } => {
            remote.public_comparison(input).map(|outcome| {
                vec![(
                    *index,
                    RemoteUpdateResult {
                        id: input.id.clone(),
                        outcome,
                    },
                )]
            })
        }
        UpdateComparisonTask::Hosted { inputs } => remote
            .hosted_comparisons(
                &inputs
                    .iter()
                    .map(|(_, input)| input.clone())
                    .collect::<Vec<_>>(),
            )
            .map(|results| {
                inputs
                    .iter()
                    .zip(results)
                    .map(|((index, _), result)| (*index, result))
                    .collect()
            }),
    };
    outcome.unwrap_or_else(|error| {
        task.indexed_inputs()
            .into_iter()
            .map(|(index, input)| {
                (
                    index,
                    RemoteUpdateResult {
                        id: input.id.clone(),
                        outcome: RemoteComparisonOutcome::RequestFailure {
                            code: error.code,
                            message: error.message.clone(),
                        },
                    },
                )
            })
            .collect()
    })
}

fn validate_update_comparison(input: &RemoteUpdateComparison) -> Result<(), RemoteError> {
    let valid_id = !input.id.is_empty()
        && input.id.len() <= 200
        && input.id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b'@' | b':')
        });
    let valid_owner = !input.owner.is_empty()
        && input.owner.len() <= 39
        && input
            .owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    let valid_repository = !input.repository.is_empty()
        && input.repository.len() <= 100
        && !matches!(input.repository.as_str(), "." | "..")
        && !input.repository.ends_with(".git")
        && input
            .repository
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid_id && valid_owner && valid_repository {
        Ok(())
    } else {
        Err(RemoteError::new(
            "INVALID_UPDATE_COMPARISON",
            "An update comparison identity is invalid",
        ))
    }
}

fn validate_hosted_identity(
    input: &RemoteUpdateComparison,
    result: &HostedComparisonResult,
) -> Result<(), RemoteError> {
    let (id, owner, repository, base_sha, head_sha) = result.identity();
    if id == input.id
        && owner.eq_ignore_ascii_case(&input.owner)
        && repository.eq_ignore_ascii_case(&input.repository)
        && base_sha == input.base_sha.as_str()
        && head_sha == input.head_sha.as_str()
    {
        Ok(())
    } else {
        Err(invalid_update_response())
    }
}

fn present_hosted_outcome(
    input: &RemoteUpdateComparison,
    result: HostedComparisonResult,
) -> Result<RemoteComparisonOutcome, RemoteError> {
    match result {
        HostedComparisonResult::Ready {
            relation,
            ahead_by,
            behind_by,
            commits,
            total,
            truncated,
            compare_url,
            ..
        } => {
            if commits.len() > MAX_UPDATE_COMMITS
                || total > MAX_SAFE_INTEGER
                || ahead_by > MAX_SAFE_INTEGER
                || behind_by > MAX_SAFE_INTEGER
                || total != ahead_by
                || !comparison_counts_match(
                    relation,
                    input.base_sha == input.head_sha,
                    ahead_by,
                    behind_by,
                )
                || u64::try_from(commits.len()).unwrap_or(u64::MAX) > total
                || truncated != (u64::try_from(commits.len()).unwrap_or(u64::MAX) < total)
                || compare_url != github_compare_url(input)
            {
                return Err(invalid_update_response());
            }
            Ok(RemoteComparisonOutcome::Ready {
                relation,
                ahead_by,
                behind_by,
                commits: commits
                    .into_iter()
                    .map(|commit| present_hosted_commit(input, commit))
                    .collect::<Result<_, _>>()?,
                total,
                truncated,
                compare_url,
            })
        }
        HostedComparisonResult::NotFound { .. } => Ok(RemoteComparisonOutcome::NotFound),
        HostedComparisonResult::InvalidComparison { .. } => {
            Ok(RemoteComparisonOutcome::InvalidComparison)
        }
        HostedComparisonResult::RateLimited {
            retry_after_seconds,
            reset_at,
            ..
        } => {
            if retry_after_seconds.is_some_and(|seconds| seconds > 604_800)
                || reset_at
                    .as_deref()
                    .is_some_and(|value| value.len() > 64 || value.chars().any(is_unsafe_terminal))
            {
                return Err(invalid_update_response());
            }
            Ok(RemoteComparisonOutcome::RateLimited {
                retry_after_seconds,
                reset_at,
            })
        }
        HostedComparisonResult::ProviderFailure { status, .. } => {
            if status.is_some_and(|status| !(100..=599).contains(&status)) {
                return Err(invalid_update_response());
            }
            Ok(RemoteComparisonOutcome::ProviderFailure { status })
        }
    }
}

fn present_hosted_commit(
    input: &RemoteUpdateComparison,
    commit: HostedCommit,
) -> Result<CommitSummary, RemoteError> {
    if commit.subject.is_empty()
        || commit.subject.chars().count() > 500
        || commit.author.name.is_empty()
        || commit.author.name.chars().count() > 200
        || commit
            .author
            .login
            .as_deref()
            .is_some_and(|login| login.is_empty() || login.chars().count() > 100)
        || !valid_timestamp(&commit.timestamp)
    {
        return Err(invalid_update_response());
    }
    let sha = CommitSha::parse(commit.sha).map_err(|_| invalid_update_response())?;
    Ok(CommitSummary {
        url: github_commit_url(input, sha.as_str()),
        sha,
        subject: sanitize_line(&commit.subject, 500, "No commit subject"),
        author: CommitAuthor {
            name: sanitize_line(&commit.author.name, 200, "Unknown"),
            login: commit.author.login,
        },
        timestamp: commit.timestamp,
    })
}

fn present_commit(
    input: &RemoteUpdateComparison,
    commit: GithubComparisonCommit,
) -> Result<CommitSummary, RemoteError> {
    let sha = CommitSha::parse(commit.sha).map_err(|_| invalid_update_response())?;
    if !valid_timestamp(&commit.commit.author.date)
        || commit
            .author
            .as_ref()
            .is_some_and(|author| author.login.is_empty() || author.login.chars().count() > 100)
    {
        return Err(invalid_update_response());
    }
    Ok(CommitSummary {
        url: github_commit_url(input, sha.as_str()),
        sha,
        subject: sanitize_line(&commit.commit.message, 500, "No commit subject"),
        author: CommitAuthor {
            name: sanitize_line(
                &commit.commit.author.name,
                200,
                commit
                    .author
                    .as_ref()
                    .map_or("Unknown", |author| author.login.as_str()),
            ),
            login: commit.author.map(|author| author.login),
        },
        timestamp: commit.commit.author.date,
    })
}

fn sanitize_line(value: &str, maximum: usize, fallback: &str) -> String {
    let line = value.split(['\r', '\n']).next().unwrap_or_default();
    let sanitized = line
        .chars()
        .map(|character| {
            if is_unsafe_terminal(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let value = if sanitized.is_empty() {
        fallback
    } else {
        &sanitized
    };
    value.chars().take(maximum).collect()
}

fn valid_timestamp(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.contains('T')
        && value.ends_with('Z')
        && !value.chars().any(is_unsafe_terminal)
}

fn github_headers() -> Vec<HttpHeader> {
    vec![
        HttpHeader {
            name: "accept".to_owned(),
            value: HeaderValue::Public("application/vnd.github+json".to_owned()),
        },
        HttpHeader {
            name: "user-agent".to_owned(),
            value: HeaderValue::Public("skilld".to_owned()),
        },
        HttpHeader {
            name: "x-github-api-version".to_owned(),
            value: HeaderValue::Public(GITHUB_API_VERSION.to_owned()),
        },
    ]
}

fn github_compare_url(input: &RemoteUpdateComparison) -> String {
    format!(
        "https://github.com/{}/{}/compare/{}...{}",
        path_segment(&input.owner),
        path_segment(&input.repository),
        input.base_sha.as_str(),
        input.head_sha.as_str(),
    )
}

fn github_commit_url(input: &RemoteUpdateComparison, sha: &str) -> String {
    format!(
        "https://github.com/{}/{}/commit/{sha}",
        path_segment(&input.owner),
        path_segment(&input.repository),
    )
}

fn is_rate_limited(response: &HttpResponse) -> bool {
    response.status == 429
        || (response.status == 403
            && (response.header("x-ratelimit-remaining") == Some("0")
                || response.header("retry-after").is_some()))
}

fn comparison_counts_match(
    relation: RemoteComparisonRelation,
    same_commit: bool,
    ahead_by: u64,
    behind_by: u64,
) -> bool {
    matches!(
        (relation, same_commit, ahead_by, behind_by),
        (RemoteComparisonRelation::Identical, true, 0, 0)
            | (RemoteComparisonRelation::Ahead, false, 1.., 0)
            | (RemoteComparisonRelation::Behind, false, 0, 1..)
            | (RemoteComparisonRelation::Diverged, false, 1.., 1..)
    )
}

fn bounded_rate_limit_wait(response: &HttpResponse) -> Option<u64> {
    let retry_after = response
        .header("retry-after")
        .and_then(|value| value.parse().ok())
        .map(|seconds: u64| seconds.min(604_800));
    retry_after.or_else(|| {
        let reset = response.header("x-ratelimit-reset")?.parse::<u64>().ok()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Some(reset.saturating_sub(now).min(604_800)).filter(|seconds| *seconds > 0)
    })
}

fn invalid_update_response() -> RemoteError {
    RemoteError::new(
        "INVALID_RESPONSE",
        "A comparison response violated its contract",
    )
}

#[derive(Clone)]
enum AllowedOrigin {
    Service(Url),
    Github,
}

fn validate_request_url(value: &str, allowed: &AllowedOrigin) -> Result<(), RemoteError> {
    let url = Url::parse(value)
        .map_err(|_| RemoteError::new("INVALID_REMOTE_URL", "a remote URL is invalid"))?;
    validate_url(&url, allowed)
}

fn validate_url(url: &Url, allowed: &AllowedOrigin) -> Result<(), RemoteError> {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(RemoteError::new(
            "REMOTE_ORIGIN_REJECTED",
            "the remote URL contains rejected credentials or a fragment",
        ));
    }
    let allowed = match allowed {
        AllowedOrigin::Service(base) => {
            url.scheme() == base.scheme()
                && url.host_str() == base.host_str()
                && url.port_or_known_default() == base.port_or_known_default()
        }
        AllowedOrigin::Github => {
            url.scheme() == "https"
                && url.host_str() == Some("api.github.com")
                && url.port().is_none()
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(RemoteError::new(
            "REMOTE_ORIGIN_REJECTED",
            "the remote URL uses an unapproved origin",
        ))
    }
}

fn json_headers() -> Vec<HttpHeader> {
    vec![
        HttpHeader {
            name: "accept".to_owned(),
            value: HeaderValue::Public("application/json".to_owned()),
        },
        HttpHeader {
            name: "content-type".to_owned(),
            value: HeaderValue::Public("application/json".to_owned()),
        },
    ]
}

fn idempotency_header() -> HttpHeader {
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    HttpHeader {
        name: "idempotency-key".to_owned(),
        value: HeaderValue::Public(format!("skilld-{time:032x}-{sequence:016x}")),
    }
}

fn retry_delay(response: &HttpResponse, attempt: usize) -> Duration {
    response
        .header("retry-after")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value <= 60)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_millis(100 * attempt as u64))
}

fn parse_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, RemoteError> {
    serde_json::from_slice(bytes).map_err(|_| {
        RemoteError::new(
            "INVALID_RESPONSE",
            "the remote service returned invalid JSON",
        )
    })
}

fn problem_error(response: &HttpResponse) -> RemoteError {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Problem {
        code: String,
        detail: Option<String>,
        title: String,
        status: u16,
        r#type: String,
        instance: Option<String>,
    }
    serde_json::from_slice::<Problem>(&response.body).map_or_else(
        |_| service_unavailable_error(response),
        |problem| {
            let _ = (&problem.r#type, &problem.instance);
            if problem.status != response.status {
                return RemoteError::new(
                    "INVALID_RESPONSE",
                    "the remote problem status does not match HTTP",
                );
            }
            let detail = match (response.status, retry_after_seconds(response)) {
                (429, Some(seconds)) => {
                    let detail = problem.detail.unwrap_or(problem.title);
                    let detail = detail.trim_end_matches('.');
                    format!("{detail}. Retry in {seconds}s.")
                }
                _ => problem.detail.unwrap_or(problem.title),
            };
            RemoteError::new(problem_code(&problem.code), detail)
        },
    )
}

fn service_unavailable_error(response: &HttpResponse) -> RemoteError {
    match (response.status, retry_after_seconds(response)) {
        (429, Some(seconds)) => RemoteError::new(
            "SERVICE_UNAVAILABLE",
            format!("the remote service rate limited the request. Retry in {seconds}s."),
        ),
        (status, _) if (500..600).contains(&status) => RemoteError::new(
            "SERVICE_UNAVAILABLE",
            format!(
                "the remote service returned HTTP {status}. Retry in a minute. If it keeps failing, the service may be down."
            ),
        ),
        (status, _) => RemoteError::new(
            "SERVICE_UNAVAILABLE",
            format!("the remote service returned HTTP {status}"),
        ),
    }
}

fn retry_after_seconds(response: &HttpResponse) -> Option<u64> {
    response
        .header("retry-after")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value <= 3600)
}

fn problem_code(value: &str) -> &'static str {
    match value {
        "AUTH_REQUIRED" => "AUTH_REQUIRED",
        "INVALID_SOURCE" => "INVALID_SOURCE",
        "SOURCE_NOT_FOUND" => "SOURCE_NOT_FOUND",
        "SOURCE_ACCESS_DENIED" => "SOURCE_ACCESS_DENIED",
        "SOURCE_UNAVAILABLE" => "SOURCE_UNAVAILABLE",
        "RATE_LIMITED" => "RATE_LIMITED",
        "CHECK_BLOCKED" => "CHECK_BLOCKED",
        "CHECK_UNAVAILABLE" => "CHECK_UNAVAILABLE",
        "ARTIFACT_REVOKED" => "ARTIFACT_REVOKED",
        "ARTIFACT_EXPIRED" => "ARTIFACT_EXPIRED",
        "ATTESTATION_EXPIRED" => "ATTESTATION_EXPIRED",
        "SIGNER_UNAVAILABLE" => "SIGNER_UNAVAILABLE",
        _ => "SERVICE_UNAVAILABLE",
    }
}

fn validate_resolved_source(
    request: &SourceRequest,
    attestation: &ArtifactAttestation,
) -> Result<(), RemoteError> {
    let resolved = &attestation.source;
    if !resolved.owner.eq_ignore_ascii_case(&request.owner)
        || !resolved
            .repository
            .eq_ignore_ascii_case(&request.repository)
        || matches!(&request.selector, SourceSelector::Path { path } if path != &resolved.skill_path)
    {
        return Err(RemoteError::new(
            "SOURCE_MISMATCH",
            "the Resolution source does not match the request",
        ));
    }
    if let Some(SourceRef::Commit { value }) = &request.r#ref
        && value != &resolved.commit_sha
    {
        return Err(RemoteError::new(
            "SOURCE_MISMATCH",
            "the Resolution commit does not match the request",
        ));
    }
    Ok(())
}

fn path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn valid_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_resolution_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn cancelled() -> RemoteError {
    RemoteError::new("CANCELLED", "the remote operation was cancelled")
}

fn resolution_timeout() -> RemoteError {
    RemoteError::new(
        "RESOLUTION_TIMEOUT",
        "Artifact creation stayed pending too long. Retry the same command.",
    )
}

fn invalid_github() -> RemoteError {
    RemoteError::new(
        "INVALID_GITHUB_RESPONSE",
        "GitHub returned an invalid Repository response",
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, tag = "state", rename_all = "lowercase")]
enum Resolution {
    Pending {
        #[serde(rename = "resolutionId")]
        resolution_id: String,
        stage: RemotePendingStage,
        #[serde(rename = "pollAfterMs")]
        poll_after_ms: u64,
    },
    Ready {
        #[serde(rename = "resolutionId")]
        resolution_id: String,
        artifact: Box<ArtifactDescriptor>,
    },
    Blocked {
        #[serde(rename = "resolutionId")]
        resolution_id: String,
        #[serde(rename = "checkResults")]
        _check_results: Vec<skilld_core::CheckResult>,
    },
    Failed {
        #[serde(rename = "resolutionId")]
        resolution_id: String,
        code: String,
        retryable: bool,
    },
    Revoked {
        #[serde(rename = "resolutionId")]
        resolution_id: String,
        #[serde(rename = "reasonCode")]
        _reason_code: String,
    },
}

impl Resolution {
    fn resolution_id(&self) -> &str {
        match self {
            Self::Pending { resolution_id, .. }
            | Self::Ready { resolution_id, .. }
            | Self::Blocked { resolution_id, .. }
            | Self::Failed { resolution_id, .. }
            | Self::Revoked { resolution_id, .. } => resolution_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ArtifactDescriptor {
    artifact_id: String,
    visibility: RepositoryVisibility,
    attestation: ArtifactAttestation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "lowercase")]
enum ArtifactGrant {
    Public {
        #[serde(rename = "artifactId")]
        artifact_id: String,
        #[serde(rename = "contentUrl")]
        content_url: String,
        #[serde(rename = "expiresAt")]
        _expires_at: String,
        attestation: ArtifactAttestation,
    },
    Private {
        #[serde(rename = "artifactId")]
        artifact_id: String,
        #[serde(rename = "contentUrl")]
        content_url: String,
        #[serde(rename = "downloadToken")]
        download_token: String,
        #[serde(rename = "expiresAt")]
        _expires_at: String,
        attestation: ArtifactAttestation,
    },
}

#[derive(Deserialize)]
struct GithubRepository {
    private: bool,
    default_branch: String,
}

#[derive(Deserialize)]
struct GithubCommit {
    sha: String,
    commit: GithubCommitData,
}

#[derive(Deserialize)]
struct GithubCommitData {
    tree: GithubTreeIdentity,
}

#[derive(Deserialize)]
struct GithubTreeIdentity {
    sha: String,
}

#[derive(Deserialize)]
struct GithubTree {
    truncated: bool,
    tree: Vec<GithubTreeEntry>,
}

#[derive(Deserialize)]
struct GithubTreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    size: Option<u64>,
}

#[derive(Deserialize)]
struct GithubBlob {
    content: String,
    encoding: String,
    size: u64,
}

#[cfg(test)]
mod problem_tests {
    use super::{HttpResponse, problem_error};

    fn response(status: u16, headers: &[(&str, &str)], body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn server_errors_state_the_next_step() {
        let error = problem_error(&response(500, &[], "not json"));
        assert_eq!(error.code, "SERVICE_UNAVAILABLE");
        assert_eq!(
            error.message,
            "the remote service returned HTTP 500. Retry in a minute. If it keeps failing, the service may be down."
        );
    }

    #[test]
    fn rate_limits_without_a_body_state_the_wait() {
        let error = problem_error(&response(429, &[("retry-after", "7")], "not json"));
        assert_eq!(error.code, "SERVICE_UNAVAILABLE");
        assert_eq!(
            error.message,
            "the remote service rate limited the request. Retry in 7s."
        );
    }

    #[test]
    fn rate_limits_with_a_body_append_the_wait() {
        let body = r#"{"code":"RATE_LIMITED","title":"Too many requests","status":429,"type":"about:blank"}"#;
        let error = problem_error(&response(429, &[("retry-after", "12")], body));
        assert_eq!(error.code, "RATE_LIMITED");
        assert_eq!(error.message, "Too many requests. Retry in 12s.");
    }

    #[test]
    fn client_errors_keep_the_plain_message() {
        let error = problem_error(&response(404, &[], "not json"));
        assert_eq!(error.code, "SERVICE_UNAVAILABLE");
        assert_eq!(error.message, "the remote service returned HTTP 404");
    }
}
