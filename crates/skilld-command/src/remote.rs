use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::json;
use skilld_core::{
    ArtifactAttestation, LockedSource, PreparedFile, RemoteError, RemoteSelector,
    RepositoryVisibility, SearchResponse, SourceRef, SourceRequest, SourceSelector, SourceStatus,
    TrustedRoot, TrustedRootPin, VerifiedTrustedRoot, parse_search_response,
    prepare_unverified_files, verify_artifact, verify_attestation, verify_trusted_root,
};
use url::Url;

const JSON_LIMIT: usize = 8 * 1024 * 1024;
const SEARCH_LIMIT: usize = 1024 * 1024;
const ARTIFACT_LIMIT: usize = 64 * 1024 * 1024;
const DIRECT_BLOB_LIMIT: usize = 12 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const MAX_RETRIES: usize = 2;
const MAX_POLLS: usize = 120;
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

    fn source_state(
        &self,
        selector: &RemoteSelector,
        artifact_id: &str,
        commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeRemoteConfig {
    Pinned(TrustedRootPin),
    Unconfigured,
}

pub struct SkilldRemote {
    adapter: Arc<dyn HttpAdapter>,
    tokens: Arc<dyn TokenProvider>,
    cancellation: Arc<dyn Cancellation>,
    sleeper: Arc<dyn Sleeper>,
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
                    poll_after_ms,
                    ..
                } => {
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
            headers: vec![HttpHeader {
                name: "accept".to_owned(),
                value: HeaderValue::Public("application/vnd.github+json".to_owned()),
            }],
            body: vec![],
            response_limit: limit,
        };
        let response = self.execute(request, AllowedOrigin::Github)?;
        parse_json(&response.body)
    }

    fn direct(&self, selector: &RemoteSelector) -> Result<PreparedRemoteSkill, RemoteError> {
        if !selector.is_explicit_github() {
            return Err(RemoteError::new(
                "DIRECT_SOURCE_REQUIRED",
                "--direct needs an explicit public GitHub Repository selector",
            ));
        }
        let source = selector.source();
        let SourceSelector::Path { path: skill_path } = &source.selector else {
            return Err(RemoteError::new(
                "DIRECT_SOURCE_REQUIRED",
                "--direct needs an explicit GitHub Skill path",
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
                "--direct cannot install from a private Repository",
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
                skill_path: skill_path.clone(),
            },
            source_status: SourceStatus::Unverified {
                content_sha256: installed_sha256.clone(),
                installed_sha256,
            },
            files,
        })
    }
}

impl RemoteProvider for SkilldRemote {
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
        let root = self.verified_root()?;
        verify_attestation(&descriptor.attestation, &root)?;
        let grant = self.grant(&descriptor.artifact_id)?;
        let archive = self.download_grant(&descriptor, grant)?;
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
        |_| {
            RemoteError::new(
                "SERVICE_UNAVAILABLE",
                format!("the remote service returned HTTP {}", response.status),
            )
        },
        |problem| {
            let _ = (&problem.r#type, &problem.instance);
            if problem.status != response.status {
                return RemoteError::new(
                    "INVALID_RESPONSE",
                    "the remote problem status does not match HTTP",
                );
            }
            RemoteError::new(
                problem_code(&problem.code),
                problem.detail.unwrap_or(problem.title),
            )
        },
    )
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
        #[serde(rename = "stage")]
        _stage: String,
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
