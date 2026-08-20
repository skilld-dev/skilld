use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroize;

use crate::callback::parse_callback_request;
use crate::model::{
    AuthDependencies, AuthError, AuthErrorKind, AuthStatus, BoundaryError, BoundaryErrorKind,
    CallbackReply, CancellationToken, HttpMethod, HttpRequest, HttpResponse, LoginOptions,
    SKILLD_ORIGIN, SecretString, SessionSummary, StoredCredential,
};

const AUTHORIZE_URL: &str = "https://skilld.dev/cli/authorize";
const TOKEN_URL: &str = "https://skilld.dev/api/cli/oauth/token";
const REFRESH_URL: &str = "https://skilld.dev/api/cli/oauth/refresh";
const LOGOUT_URL: &str = "https://skilld.dev/api/cli/logout";
const MAX_TOKEN_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_REDIRECTS: usize = 3;
const MAX_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenResponse {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: u64,
    login: String,
    scopes: Option<String>,
}

#[must_use = "the account login result must be handled"]
pub fn login(
    dependencies: &AuthDependencies<'_>,
    options: &LoginOptions,
) -> Result<SessionSummary, AuthError> {
    validate_login_options(options)?;
    check_cancelled(&options.cancellation)?;

    let mut verifier_bytes = [0_u8; 32];
    dependencies
        .random
        .fill(&mut verifier_bytes)
        .map_err(|error| {
            map_boundary(
                error,
                AuthErrorKind::CallbackFailed,
                "Account login could not create PKCE values.",
            )
        })?;
    let verifier = SecretString::new(URL_SAFE_NO_PAD.encode(verifier_bytes));
    verifier_bytes.zeroize();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose_secret().as_bytes()));

    let mut state_bytes = [0_u8; 32];
    dependencies
        .random
        .fill(&mut state_bytes)
        .map_err(|error| {
            map_boundary(
                error,
                AuthErrorKind::CallbackFailed,
                "Account login could not create PKCE values.",
            )
        })?;
    let state = SecretString::new(URL_SAFE_NO_PAD.encode(state_bytes));
    state_bytes.zeroize();

    let mut callback = dependencies.callbacks.bind().map_err(|error| {
        map_boundary(
            error,
            AuthErrorKind::CallbackFailed,
            "The account callback listener could not start.",
        )
    })?;
    let port = callback.port();
    let authorization_url = authorization_url(
        &challenge,
        state.expose_secret(),
        port,
        &options.cli_version,
    )?;
    dependencies
        .browser
        .open(&authorization_url)
        .map_err(|error| {
            map_boundary(
                error,
                AuthErrorKind::BrowserFailed,
                "The browser could not open the account login page.",
            )
        })?;

    let started_at = dependencies.clock.now_unix_seconds();
    let request = callback
        .receive(options.callback_timeout, &options.cancellation)
        .map_err(|error| {
            map_boundary(
                error,
                AuthErrorKind::CallbackFailed,
                "The account callback failed.",
            )
        })?;
    let code = match parse_callback_request(&request, port, state.expose_secret()) {
        Ok(code) => code,
        Err(error) => {
            // The callback already failed. A browser response cannot change that result.
            let _ = callback.respond(CallbackReply::Rejected);
            return Err(error);
        }
    };
    let expires_at = started_at.saturating_add(options.callback_timeout.as_secs());
    if dependencies.clock.now_unix_seconds() >= expires_at {
        let _ = callback.respond(CallbackReply::Rejected);
        return Err(AuthError::new(
            AuthErrorKind::ExpiredAuthorizationCode,
            "The account authorization code expired.",
        ));
    }
    let credential = match exchange_login_token(dependencies, options, port, &code, &verifier) {
        Ok(credential) => credential,
        Err(error) => {
            let _ = callback.respond(CallbackReply::Rejected);
            return Err(error);
        }
    };
    if let Err(error) = dependencies
        .credentials
        .save(&credential)
        .map_err(map_store_error)
    {
        let _ = callback.respond(CallbackReply::Rejected);
        return Err(error);
    }
    // Credential persistence is complete. Browser response failures are not account failures.
    let _ = callback.respond(CallbackReply::Accepted);
    Ok(summary(&credential))
}

#[must_use = "the account refresh result must be handled"]
pub fn refresh(
    dependencies: &AuthDependencies<'_>,
    http_timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<SessionSummary, AuthError> {
    validate_http_timeout(http_timeout)?;
    check_cancelled(cancellation)?;
    let current = load_bound(dependencies)?.ok_or_else(|| {
        AuthError::new(
            AuthErrorKind::NotAuthenticated,
            "No account is authenticated.",
        )
    })?;
    let refresh_token = current.refresh_token.as_ref().ok_or_else(|| {
        AuthError::new(
            AuthErrorKind::MissingRefreshToken,
            "The account credential cannot refresh.",
        )
    })?;
    let body = serde_json::to_vec(&json!({
        "refresh_token": refresh_token.expose_secret(),
    }))
    .map_err(|_| {
        AuthError::new(
            AuthErrorKind::InvalidResponse,
            "The refresh request could not be encoded.",
        )
    })?;
    let response = send_json(
        dependencies.http,
        REFRESH_URL,
        Vec::new(),
        body,
        http_timeout,
        cancellation,
    )?;
    if response.status == 401 {
        return Err(AuthError::new(
            AuthErrorKind::RefreshRejected,
            "The account refresh token was rejected.",
        ));
    }
    if !(200..300).contains(&response.status) {
        return Err(AuthError::new(
            AuthErrorKind::HttpFailed,
            "The account refresh failed.",
        ));
    }
    let rotated = parse_token_response(&response, dependencies.clock.now_unix_seconds(), true)?;
    if rotated.account != current.account {
        return Err(AuthError::new(
            AuthErrorKind::AccountMismatch,
            "The refreshed credential belongs to another account.",
        ));
    }
    if rotated
        .refresh_token
        .as_ref()
        .is_some_and(|next| next == refresh_token)
    {
        return Err(AuthError::new(
            AuthErrorKind::InvalidResponse,
            "The refresh endpoint did not rotate its token.",
        ));
    }
    dependencies
        .credentials
        .save(&rotated)
        .map_err(map_store_error)?;
    Ok(summary(&rotated))
}

#[must_use = "the account status result must be handled"]
pub fn status(dependencies: &AuthDependencies<'_>) -> Result<AuthStatus, AuthError> {
    let Some(credential) = load_bound(dependencies)? else {
        return Ok(AuthStatus::NotAuthenticated);
    };
    if credential.expires_at <= dependencies.clock.now_unix_seconds() {
        return Ok(AuthStatus::Expired {
            account: credential.account,
            expired_at: credential.expires_at,
        });
    }
    Ok(AuthStatus::Authenticated(summary(&credential)))
}

#[must_use = "the account logout result must be handled"]
pub fn logout(
    dependencies: &AuthDependencies<'_>,
    http_timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<(), AuthError> {
    validate_http_timeout(http_timeout)?;
    check_cancelled(cancellation)?;
    let Some(credential) = load_bound(dependencies)? else {
        return Ok(());
    };
    let headers = vec![(
        "authorization".to_owned(),
        format!("Bearer {}", credential.access_token.expose_secret()),
    )];
    let remote = send_json(
        dependencies.http,
        LOGOUT_URL,
        headers,
        b"{}".to_vec(),
        http_timeout,
        cancellation,
    )
    .and_then(|response| {
        if (200..300).contains(&response.status) {
            Ok(response)
        } else {
            Err(AuthError::new(
                AuthErrorKind::LogoutFailed,
                "The account logout endpoint failed.",
            ))
        }
    });
    dependencies
        .credentials
        .delete(SKILLD_ORIGIN, &credential.account)
        .map_err(map_store_error)?;
    remote.map(|_| ())
}

fn validate_login_options(options: &LoginOptions) -> Result<(), AuthError> {
    if options.cli_version.is_empty() || options.cli_version.len() > 32 {
        return Err(AuthError::new(
            AuthErrorKind::CallbackFailed,
            "The CLI version is invalid.",
        ));
    }
    if options.callback_timeout < Duration::from_secs(1)
        || options.callback_timeout > MAX_CALLBACK_TIMEOUT
    {
        return Err(AuthError::new(
            AuthErrorKind::CallbackFailed,
            "The account callback timeout is invalid.",
        ));
    }
    validate_http_timeout(options.http_timeout)
}

fn validate_http_timeout(timeout: Duration) -> Result<(), AuthError> {
    if timeout < Duration::from_secs(1) || timeout > MAX_HTTP_TIMEOUT {
        return Err(AuthError::new(
            AuthErrorKind::HttpFailed,
            "The account request timeout is invalid.",
        ));
    }
    Ok(())
}

fn authorization_url(
    challenge: &str,
    state: &str,
    port: u16,
    cli_version: &str,
) -> Result<String, AuthError> {
    let mut url = Url::parse(AUTHORIZE_URL).map_err(|_| {
        AuthError::new(
            AuthErrorKind::BrowserFailed,
            "The account login URL is invalid.",
        )
    })?;
    url.query_pairs_mut()
        .append_pair("challenge", challenge)
        .append_pair("port", &port.to_string())
        .append_pair("state", state)
        .append_pair("v", cli_version);
    Ok(url.into())
}

fn exchange_login_token(
    dependencies: &AuthDependencies<'_>,
    options: &LoginOptions,
    port: u16,
    code: &crate::model::AuthorizationCode,
    verifier: &SecretString,
) -> Result<StoredCredential, AuthError> {
    check_cancelled(&options.cancellation)?;
    let redirect_uri = format!("http://127.0.0.1:{port}/");
    let body = serde_json::to_vec(&json!({
        "code": code.expose_secret(),
        "code_verifier": verifier.expose_secret(),
        "redirect_uri": redirect_uri,
    }))
    .map_err(|_| {
        AuthError::new(
            AuthErrorKind::InvalidResponse,
            "The token request could not be encoded.",
        )
    })?;
    let response = send_json(
        dependencies.http,
        TOKEN_URL,
        Vec::new(),
        body,
        options.http_timeout,
        &options.cancellation,
    )?;
    if response.status == 401 {
        return Err(AuthError::new(
            AuthErrorKind::ExpiredAuthorizationCode,
            "The account authorization code was invalid or expired.",
        ));
    }
    if !(200..300).contains(&response.status) {
        return Err(AuthError::new(
            AuthErrorKind::HttpFailed,
            "The token exchange failed.",
        ));
    }
    parse_token_response(&response, dependencies.clock.now_unix_seconds(), true)
}

fn parse_token_response(
    response: &HttpResponse,
    now: u64,
    require_refresh: bool,
) -> Result<StoredCredential, AuthError> {
    let parsed = serde_json::from_slice::<TokenResponse>(&response.body).map_err(|_| {
        AuthError::new(
            AuthErrorKind::InvalidResponse,
            "The token endpoint returned invalid JSON.",
        )
    })?;
    if !(16..=16 * 1024).contains(&parsed.access_token.len())
        || parsed
            .refresh_token
            .as_ref()
            .is_some_and(|token| !(16..=16 * 1024).contains(&token.len()))
        || parsed
            .scopes
            .as_ref()
            .is_some_and(|scopes| scopes.len() > 256)
        || !is_account(&parsed.login)
    {
        return Err(AuthError::new(
            AuthErrorKind::InvalidResponse,
            "The token endpoint returned invalid account data.",
        ));
    }
    if require_refresh && parsed.refresh_token.is_none() {
        return Err(AuthError::new(
            AuthErrorKind::InvalidResponse,
            "The token endpoint omitted its refresh token.",
        ));
    }
    if parsed.expires_at <= now {
        return Err(AuthError::new(
            AuthErrorKind::ExpiredToken,
            "The token endpoint returned an expired token.",
        ));
    }
    Ok(StoredCredential {
        origin: SKILLD_ORIGIN.to_owned(),
        account: parsed.login,
        access_token: SecretString::new(parsed.access_token),
        refresh_token: parsed.refresh_token.map(SecretString::new),
        expires_at: parsed.expires_at,
        scopes: parsed.scopes,
    })
}

fn is_account(value: &str) -> bool {
    if value.is_empty() || value.len() > 39 || value.starts_with('-') || value.ends_with('-') {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !value.contains("--")
}

fn load_bound(dependencies: &AuthDependencies<'_>) -> Result<Option<StoredCredential>, AuthError> {
    let credential = dependencies
        .credentials
        .load(SKILLD_ORIGIN)
        .map_err(map_store_error)?;
    if credential
        .as_ref()
        .is_some_and(|value| value.origin != SKILLD_ORIGIN || !is_account(&value.account))
    {
        return Err(AuthError::new(
            AuthErrorKind::AccountMismatch,
            "The stored credential has a different account binding.",
        ));
    }
    Ok(credential)
}

fn summary(credential: &StoredCredential) -> SessionSummary {
    SessionSummary {
        account: credential.account.clone(),
        expires_at: credential.expires_at,
        scopes: credential.scopes.clone(),
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), AuthError> {
    if cancellation.is_cancelled() {
        Err(AuthError::new(
            AuthErrorKind::Cancelled,
            "The account operation was cancelled.",
        ))
    } else {
        Ok(())
    }
}

fn send_json(
    http: &dyn crate::model::HttpClient,
    initial_url: &str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<HttpResponse, AuthError> {
    let mut url = Url::parse(initial_url).map_err(|_| {
        AuthError::new(
            AuthErrorKind::RedirectRejected,
            "The account endpoint URL is invalid.",
        )
    })?;
    for redirect_count in 0..=MAX_REDIRECTS {
        check_cancelled(cancellation)?;
        let mut request_headers = headers.clone();
        request_headers.push(("content-type".to_owned(), "application/json".to_owned()));
        let response = http
            .execute(HttpRequest {
                method: HttpMethod::Post,
                url: url.to_string(),
                headers: request_headers,
                body: body.clone(),
                timeout,
                max_response_bytes: MAX_TOKEN_RESPONSE_BYTES,
                cancellation: cancellation.clone(),
            })
            .map_err(|error| {
                map_boundary(
                    error,
                    AuthErrorKind::HttpFailed,
                    "The account endpoint request failed.",
                )
            })?;
        if response.body.len() > MAX_TOKEN_RESPONSE_BYTES {
            return Err(AuthError::new(
                AuthErrorKind::ResponseTooLarge,
                "The account endpoint response exceeded its byte limit.",
            ));
        }
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        if !matches!(response.status, 307 | 308) || redirect_count == MAX_REDIRECTS {
            return Err(AuthError::new(
                AuthErrorKind::RedirectRejected,
                "The account endpoint redirect was rejected.",
            ));
        }
        let location = single_header(&response, "location")?.ok_or_else(|| {
            AuthError::new(
                AuthErrorKind::RedirectRejected,
                "The account endpoint redirect omitted its location.",
            )
        })?;
        let next = url.join(location).map_err(|_| {
            AuthError::new(
                AuthErrorKind::RedirectRejected,
                "The account endpoint redirect URL was invalid.",
            )
        })?;
        if !is_skilld_origin(&next) {
            return Err(AuthError::new(
                AuthErrorKind::RedirectRejected,
                "The account endpoint redirect left skilld.dev.",
            ));
        }
        url = next;
    }
    Err(AuthError::new(
        AuthErrorKind::RedirectRejected,
        "The account endpoint used too many redirects.",
    ))
}

fn single_header<'a>(
    response: &'a HttpResponse,
    expected: &str,
) -> Result<Option<&'a str>, AuthError> {
    let values = response
        .headers
        .iter()
        .filter_map(|(name, value)| {
            name.eq_ignore_ascii_case(expected)
                .then_some(value.as_str())
        })
        .collect::<Vec<_>>();
    if values.len() > 1 {
        return Err(AuthError::new(
            AuthErrorKind::RedirectRejected,
            "The account endpoint returned duplicate redirect headers.",
        ));
    }
    Ok(values.first().copied())
}

fn is_skilld_origin(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("skilld.dev")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn map_boundary(
    error: BoundaryError,
    fallback_kind: AuthErrorKind,
    fallback_message: &'static str,
) -> AuthError {
    match error.kind() {
        BoundaryErrorKind::Cancelled => AuthError::new(
            AuthErrorKind::Cancelled,
            "The account operation was cancelled.",
        ),
        BoundaryErrorKind::Timeout => {
            AuthError::new(AuthErrorKind::Timeout, "The account operation timed out.")
        }
        BoundaryErrorKind::ResponseTooLarge => AuthError::new(
            AuthErrorKind::ResponseTooLarge,
            "The account response exceeded its byte limit.",
        ),
        BoundaryErrorKind::Unsupported => AuthError::new(
            AuthErrorKind::UnsupportedCapability,
            "The credential capability is unavailable on this host.",
        ),
        BoundaryErrorKind::Failed => AuthError::new(fallback_kind, fallback_message),
    }
}

fn map_store_error(error: BoundaryError) -> AuthError {
    map_boundary(
        error,
        AuthErrorKind::CredentialStoreFailed,
        "The OS keychain operation failed.",
    )
}
