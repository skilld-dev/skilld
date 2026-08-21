use std::io::Read;
use std::process::Command;
use std::time::Duration;

use skilld_auth::{
    AuthDependencies, AuthError, AuthErrorKind, AuthStatus, BoundaryError, BoundaryErrorKind,
    BrowserLauncher, CancellationToken, Clock, CredentialStore, HttpClient, HttpRequest,
    HttpResponse, KeychainCredentialStore, LoginOptions, NativeLoopbackListener, OsRandom,
    SKILLD_ORIGIN, SystemClock, login, logout, refresh, status,
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

#[derive(Clone, Copy, Debug, Default)]
struct NativeBrowser;

impl BrowserLauncher for NativeBrowser {
    fn open(&self, url: &str) -> Result<(), BoundaryError> {
        let launch = auth_browser_command(std::env::consts::OS, url).map_err(|error| {
            boundary(if error.code == "UNSUPPORTED_HOST" {
                BoundaryErrorKind::Unsupported
            } else {
                BoundaryErrorKind::Failed
            })
        })?;
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
    http: NativeAuthHttp,
    browser: NativeBrowser,
    clock: SystemClock,
    random: OsRandom,
    callbacks: NativeLoopbackListener,
    credentials: KeychainCredentialStore,
}

impl NativeAccount {
    pub fn new() -> Self {
        Self {
            http: NativeAuthHttp,
            browser: NativeBrowser,
            clock: SystemClock,
            random: OsRandom,
            callbacks: NativeLoopbackListener,
            credentials: KeychainCredentialStore::new(),
        }
    }

    fn dependencies(&self) -> AuthDependencies<'_> {
        AuthDependencies {
            http: &self.http,
            browser: &self.browser,
            clock: &self.clock,
            random: &self.random,
            callbacks: &self.callbacks,
            credentials: &self.credentials,
        }
    }

    fn current_token(&self) -> Result<Option<SecretValue>, RemoteError> {
        let mut credential = self
            .credentials
            .load(SKILLD_ORIGIN)
            .map_err(|_| RemoteError::new("SERVICE_UNAVAILABLE", "the account keychain failed"))?;
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
            credential = self.credentials.load(SKILLD_ORIGIN).map_err(|_| {
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
    fn status(&self) -> Result<bool, CommandError> {
        status(&self.dependencies())
            .map(|status| matches!(status, AuthStatus::Authenticated(_)))
            .map_err(command_auth_error)
    }

    fn login(&self) -> Result<(), CommandError> {
        login(&self.dependencies(), &LoginOptions::new(VERSION))
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
