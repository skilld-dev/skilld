use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zeroize::Zeroize;

pub const SKILLD_ORIGIN: &str = "https://skilld.dev";

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthErrorKind {
    AccountMismatch,
    BrowserFailed,
    CallbackFailed,
    CallbackInjection,
    CallbackPortMismatch,
    Cancelled,
    CredentialStoreFailed,
    ExpiredAuthorizationCode,
    ExpiredToken,
    HttpFailed,
    InvalidAuthorizationCode,
    InvalidResponse,
    LogoutFailed,
    MissingRefreshToken,
    NotAuthenticated,
    RedirectRejected,
    RefreshRejected,
    ResponseTooLarge,
    StateMismatch,
    Timeout,
    UnsupportedCapability,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AuthError {
    kind: AuthErrorKind,
    message: &'static str,
}

impl AuthError {
    #[must_use]
    pub const fn new(kind: AuthErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    #[must_use]
    pub const fn kind(&self) -> AuthErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Debug for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthError")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .finish()
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for AuthError {}

pub struct SecretString(String);

impl SecretString {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl Clone for SecretString {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for SecretString {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationCode(SecretString);

impl AuthorizationCode {
    pub(crate) fn new(value: String) -> Self {
        Self(SecretString::new(value))
    }

    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct StoredCredential {
    pub origin: String,
    pub account: String,
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub expires_at: u64,
    pub scopes: Option<String>,
}

impl fmt::Debug for StoredCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredCredential")
            .field("origin", &self.origin)
            .field("account", &self.account)
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub account: String,
    pub expires_at: u64,
    pub scopes: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthStatus {
    NotAuthenticated,
    Authenticated(SessionSummary),
    Expired { account: String, expired_at: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryErrorKind {
    Cancelled,
    Failed,
    ResponseTooLarge,
    Timeout,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundaryError {
    kind: BoundaryErrorKind,
}

impl BoundaryError {
    #[must_use]
    pub const fn new(kind: BoundaryErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> BoundaryErrorKind {
        self.kind
    }
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("authentication boundary failed")
    }
}

impl std::error::Error for BoundaryError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpMethod {
    Post,
}

#[derive(Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
    pub max_response_bytes: usize,
    pub cancellation: CancellationToken,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

#[derive(Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body", &"[REDACTED]")
            .finish()
    }
}

pub trait HttpClient: Send + Sync {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, BoundaryError>;
}

pub trait BrowserLauncher: Send + Sync {
    fn open(&self, url: &str) -> Result<(), BoundaryError>;
}

pub trait Clock: Send + Sync {
    fn now_unix_seconds(&self) -> u64;
}

pub trait RandomSource: Send + Sync {
    fn fill(&self, output: &mut [u8]) -> Result<(), BoundaryError>;
}

#[derive(Clone, Debug)]
pub struct CallbackRequest {
    pub bytes: Vec<u8>,
    pub peer: SocketAddr,
    pub local_port: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallbackReply {
    Accepted,
    Rejected,
}

pub trait CallbackBinding: Send {
    fn port(&self) -> u16;

    fn receive(
        &mut self,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<CallbackRequest, BoundaryError>;

    fn respond(&mut self, reply: CallbackReply) -> Result<(), BoundaryError>;
}

pub trait CallbackListener: Send + Sync {
    fn bind(&self) -> Result<Box<dyn CallbackBinding>, BoundaryError>;
}

pub trait CredentialStore: Send + Sync {
    fn load(&self, origin: &str) -> Result<Option<StoredCredential>, BoundaryError>;

    fn save(&self, credential: &StoredCredential) -> Result<(), BoundaryError>;

    fn delete(&self, origin: &str, account: &str) -> Result<(), BoundaryError>;
}

pub struct AuthDependencies<'a> {
    pub http: &'a dyn HttpClient,
    pub browser: &'a dyn BrowserLauncher,
    pub clock: &'a dyn Clock,
    pub random: &'a dyn RandomSource,
    pub callbacks: &'a dyn CallbackListener,
    pub credentials: &'a dyn CredentialStore,
}

#[derive(Clone, Debug)]
pub struct LoginOptions {
    pub cli_version: String,
    pub callback_timeout: Duration,
    pub http_timeout: Duration,
    pub cancellation: CancellationToken,
}

impl LoginOptions {
    #[must_use]
    pub fn new(cli_version: impl Into<String>) -> Self {
        Self {
            cli_version: cli_version.into(),
            callback_timeout: Duration::from_secs(300),
            http_timeout: Duration::from_secs(30),
            cancellation: CancellationToken::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&self, output: &mut [u8]) -> Result<(), BoundaryError> {
        getrandom::fill(output).map_err(|_| BoundaryError::new(BoundaryErrorKind::Failed))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnsupportedCredentialStore;

impl CredentialStore for UnsupportedCredentialStore {
    fn load(&self, _origin: &str) -> Result<Option<StoredCredential>, BoundaryError> {
        Err(BoundaryError::new(BoundaryErrorKind::Unsupported))
    }

    fn save(&self, _credential: &StoredCredential) -> Result<(), BoundaryError> {
        Err(BoundaryError::new(BoundaryErrorKind::Unsupported))
    }

    fn delete(&self, _origin: &str, _account: &str) -> Result<(), BoundaryError> {
        Err(BoundaryError::new(BoundaryErrorKind::Unsupported))
    }
}
