use std::io::Read;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use skilld_auth::{
    AuthDependencies, AuthError, AuthErrorKind, AuthStatus, BoundaryError, BoundaryErrorKind,
    BrowserLauncher, CancellationToken, Clock, CredentialStore, HttpClient, HttpRequest,
    HttpResponse, KeychainCredentialStore, LoginOptions, NativeLoopbackListener, OsRandom,
    ServiceOrigin, SystemClock, login, logout, refresh, status,
};
use skilld_command::{AccountProvider, CommandError, SecretValue, TokenProvider};
use skilld_core::{RemoteError, VERSION};
use skilld_native::auth_browser_command;

#[derive(Clone, Copy, Debug, Default)]
struct NativeAuthHttp;

impl HttpClient for NativeAuthHttp {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, BoundaryError> {
        if request.cancellation.is_cancelled() {
            return Err(boundary(BoundaryErrorKind::Cancelled));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(request.timeout))
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent("skilld/3")
            .build()
            .into();
        let mut builder = agent.post(&request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send(request.body.as_slice())
            .map_err(|_| boundary(BoundaryErrorKind::Failed))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect::<Vec<_>>();
        if headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-length")
                && value
                    .parse::<usize>()
                    .is_ok_and(|length| length > request.max_response_bytes)
        }) {
            return Err(boundary(BoundaryErrorKind::ResponseTooLarge));
        }
        let mut body = response.into_body();
        let mut reader = body.as_reader();
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            if request.cancellation.is_cancelled() {
                return Err(boundary(BoundaryErrorKind::Cancelled));
            }
            let read = reader
                .read(&mut buffer)
                .map_err(|_| boundary(BoundaryErrorKind::Failed))?;
            if read == 0 {
                break;
            }
            if bytes.len().saturating_add(read) > request.max_response_bytes {
                return Err(boundary(BoundaryErrorKind::ResponseTooLarge));
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        Ok(HttpResponse {
            status,
            headers,
            body: bytes,
        })
    }
}

#[derive(Clone, Debug, Default)]
struct NativeBrowser {
    origin: ServiceOrigin,
    manual: bool,
}

impl BrowserLauncher for NativeBrowser {
    fn open(&self, url: &str) -> Result<(), BoundaryError> {
        let launch =
            auth_browser_command(std::env::consts::OS, url, &self.origin).map_err(|error| {
                boundary(if error.code == "UNSUPPORTED_HOST" {
                    BoundaryErrorKind::Unsupported
                } else {
                    BoundaryErrorKind::Failed
                })
            })?;
        if self.manual {
            eprintln!("Open this authorization URL in your signed-in browser:\n{url}");
            return Ok(());
        }
        Command::new(launch.program)
            .args(launch.arguments)
            .status()
            .map_err(|_| boundary(BoundaryErrorKind::Failed))?
            .success()
            .then_some(())
            .ok_or_else(|| boundary(BoundaryErrorKind::Failed))
    }
}

pub struct NativeAccount {
    origin: ServiceOrigin,
    http: NativeAuthHttp,
    browser: NativeBrowser,
    clock: SystemClock,
    random: OsRandom,
    callbacks: NativeLoopbackListener,
    credentials: Arc<dyn CredentialStore>,
    /// `SKILLD_TOKEN`. When set, it is the only credential this run sends.
    token_override: Option<SecretValue>,
    keychain_unavailable: AtomicBool,
}

impl NativeAccount {
    pub fn new() -> Self {
        Self::with_credentials(Arc::new(KeychainCredentialStore::new()))
    }

    /// The account backed by the given credential store.
    pub fn with_credentials(credentials: Arc<dyn CredentialStore>) -> Self {
        Self {
            origin: ServiceOrigin::production(),
            http: NativeAuthHttp,
            browser: NativeBrowser::default(),
            clock: SystemClock,
            random: OsRandom,
            callbacks: NativeLoopbackListener,
            credentials,
            token_override: None,
            keychain_unavailable: AtomicBool::new(false),
        }
    }

    /// Send this token instead of the stored sign-in. See `token_override`.
    #[must_use]
    pub fn with_token_override(mut self, token: Option<SecretValue>) -> Self {
        self.token_override = token;
        self
    }

    /// The account on another skilld.dev origin. Its credential is separate
    /// from the production one.
    #[must_use]
    pub fn with_origin(mut self, origin: ServiceOrigin) -> Self {
        self.browser = NativeBrowser {
            origin: origin.clone(),
            manual: false,
        };
        self.origin = origin;
        self
    }

    fn dependencies(&self) -> AuthDependencies<'_> {
        AuthDependencies {
            origin: &self.origin,
            http: &self.http,
            browser: &self.browser,
            clock: &self.clock,
            random: &self.random,
            callbacks: &self.callbacks,
            credentials: self.credentials.as_ref(),
        }
    }

    /// Report public access after success, so JSON failures remain one document.
    pub fn public_access_notice(&self) -> Option<&'static str> {
        self.keychain_unavailable
            .load(Ordering::Relaxed)
            .then_some("The account keychain is unavailable. Using public access.")
    }

    fn current_token(&self) -> Result<Option<SecretValue>, RemoteError> {
        if let Some(token) = &self.token_override {
            return Ok(Some(token.clone()));
        }
        let mut credential = match self.credentials.load(self.origin.as_str()) {
            Ok(credential) => credential,
            Err(_) => {
                // Public delivery needs no account. Report the unavailable store;
                // account commands still surface its error, and private delivery
                // still requires a token. Never hide a failed credential refresh.
                self.keychain_unavailable.store(true, Ordering::Relaxed);
                return Ok(None);
            }
        };
        if credential
            .as_ref()
            .is_some_and(|credential| credential.expires_at <= self.clock.now_unix_seconds())
        {
            refresh(
                &self.dependencies(),
                Duration::from_secs(30),
                &CancellationToken::new(),
            )
            .map_err(remote_auth_error)?;
            credential = self.credentials.load(self.origin.as_str()).map_err(|_| {
                RemoteError::new("SERVICE_UNAVAILABLE", "the account keychain failed")
            })?;
        }
        credential
            .map(|credential| SecretValue::new(credential.access_token.expose_secret()))
            .transpose()
    }
}

impl Default for NativeAccount {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenProvider for NativeAccount {
    fn access_token(&self) -> Result<Option<SecretValue>, RemoteError> {
        self.current_token()
    }
}

impl AccountProvider for NativeAccount {
    /// What `skilld auth status` prints: only a fresh credential counts.
    fn status(&self) -> Result<bool, CommandError> {
        if self.token_override.is_some() {
            return Ok(true);
        }
        status(&self.dependencies())
            .map(|status| matches!(status, AuthStatus::Authenticated(_)))
            .map_err(command_auth_error)
    }

    /// Whether the person has an account at all. An expired token still
    /// belongs to an account that already receives the weekly, so it counts.
    fn has_account(&self) -> Result<bool, CommandError> {
        if self.token_override.is_some() {
            return Ok(true);
        }
        status(&self.dependencies())
            .map(|status| {
                matches!(
                    status,
                    AuthStatus::Authenticated(_) | AuthStatus::Expired { .. }
                )
            })
            .map_err(command_auth_error)
    }

    fn login(&self) -> Result<(), CommandError> {
        let options = LoginOptions {
            device_label: gethostname::gethostname().into_string().ok(),
            ..LoginOptions::new(VERSION)
        };
        login(&self.dependencies(), &options)
            .map(|_| ())
            .map_err(command_auth_error)
    }

    fn login_without_browser(&self) -> Result<(), CommandError> {
        let browser = NativeBrowser {
            origin: self.origin.clone(),
            manual: true,
        };
        let dependencies = AuthDependencies {
            browser: &browser,
            ..self.dependencies()
        };
        let options = LoginOptions {
            device_label: gethostname::gethostname().into_string().ok(),
            ..LoginOptions::new(VERSION)
        };
        login(&dependencies, &options)
            .map(|_| ())
            .map_err(command_auth_error)
    }

    fn logout(&self) -> Result<(), CommandError> {
        logout(
            &self.dependencies(),
            Duration::from_secs(30),
            &CancellationToken::new(),
        )
        .map_err(command_auth_error)
    }
}

fn command_auth_error(error: AuthError) -> CommandError {
    let code = match error.kind() {
        AuthErrorKind::NotAuthenticated
        | AuthErrorKind::ExpiredToken
        | AuthErrorKind::MissingRefreshToken
        | AuthErrorKind::RefreshRejected => "AUTH_REQUIRED",
        AuthErrorKind::UnsupportedCapability => "UNSUPPORTED_HOST",
        _ => "SERVICE_UNAVAILABLE",
    };
    CommandError::operation(code, error.message())
}

fn remote_auth_error(error: AuthError) -> RemoteError {
    let command = command_auth_error(error);
    RemoteError::new(command.code, command.message)
}

const fn boundary(kind: BoundaryErrorKind) -> BoundaryError {
    BoundaryError::new(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skilld_auth::StoredCredential;

    struct UnavailableStore;

    impl CredentialStore for UnavailableStore {
        fn load(&self, _: &str) -> Result<Option<StoredCredential>, BoundaryError> {
            Err(boundary(BoundaryErrorKind::Failed))
        }

        fn save(&self, _: &StoredCredential) -> Result<(), BoundaryError> {
            Err(boundary(BoundaryErrorKind::Failed))
        }

        fn delete(&self, _: &str, _: &str) -> Result<(), BoundaryError> {
            Err(boundary(BoundaryErrorKind::Failed))
        }
    }

    #[test]
    fn public_access_needs_no_keychain_but_account_status_reports_its_failure() {
        let account = NativeAccount::with_credentials(Arc::new(UnavailableStore));

        assert!(account.access_token().unwrap().is_none());
        assert!(account.status().is_err());
    }

    #[test]
    fn an_environment_token_does_not_read_an_unavailable_keychain() {
        let account = NativeAccount::with_credentials(Arc::new(UnavailableStore))
            .with_token_override(Some(SecretValue::new("explicit-token").unwrap()));

        assert_eq!(
            account.access_token().unwrap().unwrap().expose(),
            "explicit-token"
        );
        assert!(account.status().unwrap());
    }
}
