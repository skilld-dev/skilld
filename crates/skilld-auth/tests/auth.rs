use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use skilld_auth::{
    AuthDependencies, AuthErrorKind, AuthStatus, BoundaryError, BoundaryErrorKind, BrowserLauncher,
    CallbackBinding, CallbackListener, CallbackReply, CallbackRequest, CancellationToken, Clock,
    CredentialStore, HttpClient, HttpRequest, HttpResponse, LoginOptions, NativeLoopbackListener,
    RandomSource, SKILLD_ORIGIN, SecretString, StoredCredential, UnsupportedCredentialStore, login,
    logout, parse_callback_request, refresh, status,
};
use url::Url;

const PORT: u16 = 49_152;
const NOW: u64 = 1_800_000_000;
const ACCESS_ONE: &str = "access-token-one-1234567890";
const ACCESS_TWO: &str = "access-token-two-1234567890";
const REFRESH_ONE: &str = "refresh-token-one-1234567890";
const REFRESH_TWO: &str = "refresh-token-two-1234567890";
const REFRESH_THREE: &str = "refresh-token-three-1234567890";

struct FakeHttp {
    responses: Mutex<VecDeque<Result<HttpResponse, BoundaryError>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FakeHttp {
    fn new(responses: Vec<Result<HttpResponse, BoundaryError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl HttpClient for FakeHttp {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, BoundaryError> {
        self.requests.lock().expect("requests lock").push(request);
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .unwrap_or_else(|| Err(BoundaryError::new(BoundaryErrorKind::Failed)))
    }
}

#[derive(Default)]
struct FakeBrowser {
    urls: Mutex<Vec<String>>,
    fail: AtomicBool,
}

impl BrowserLauncher for FakeBrowser {
    fn open(&self, url: &str) -> Result<(), BoundaryError> {
        self.urls.lock().expect("browser lock").push(url.to_owned());
        if self.fail.load(Ordering::Acquire) {
            Err(BoundaryError::new(BoundaryErrorKind::Failed))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn new(now: u64) -> Self {
        Self(Arc::new(AtomicU64::new(now)))
    }
}

impl Clock for FakeClock {
    fn now_unix_seconds(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Default)]
struct FakeRandom(AtomicU8);

impl RandomSource for FakeRandom {
    fn fill(&self, output: &mut [u8]) -> Result<(), BoundaryError> {
        let byte = self.0.fetch_add(1, Ordering::AcqRel) + 1;
        output.fill(byte);
        Ok(())
    }
}

struct FakeCallbacks {
    result: Mutex<Option<Result<CallbackRequest, BoundaryError>>>,
    replies: Arc<Mutex<Vec<CallbackReply>>>,
    clock: FakeClock,
    advance_to: AtomicU64,
    binds: AtomicU64,
}

impl FakeCallbacks {
    fn new(request: CallbackRequest, clock: FakeClock) -> Self {
        Self {
            result: Mutex::new(Some(Ok(request))),
            replies: Arc::new(Mutex::new(Vec::new())),
            clock,
            advance_to: AtomicU64::new(0),
            binds: AtomicU64::new(0),
        }
    }

    fn failing(error: BoundaryError, clock: FakeClock) -> Self {
        Self {
            result: Mutex::new(Some(Err(error))),
            replies: Arc::new(Mutex::new(Vec::new())),
            clock,
            advance_to: AtomicU64::new(0),
            binds: AtomicU64::new(0),
        }
    }
}

impl CallbackListener for FakeCallbacks {
    fn bind(&self) -> Result<Box<dyn CallbackBinding>, BoundaryError> {
        self.binds.fetch_add(1, Ordering::AcqRel);
        let result = self
            .result
            .lock()
            .expect("callback lock")
            .take()
            .unwrap_or_else(|| Err(BoundaryError::new(BoundaryErrorKind::Failed)));
        Ok(Box::new(FakeBinding {
            result: Some(result),
            replies: Arc::clone(&self.replies),
            clock: self.clock.clone(),
            advance_to: self.advance_to.load(Ordering::Acquire),
        }))
    }
}

struct FakeBinding {
    result: Option<Result<CallbackRequest, BoundaryError>>,
    replies: Arc<Mutex<Vec<CallbackReply>>>,
    clock: FakeClock,
    advance_to: u64,
}

impl CallbackBinding for FakeBinding {
    fn port(&self) -> u16 {
        PORT
    }

    fn receive(
        &mut self,
        _timeout: Duration,
        _cancellation: &CancellationToken,
    ) -> Result<CallbackRequest, BoundaryError> {
        if self.advance_to > 0 {
            self.clock.0.store(self.advance_to, Ordering::Release);
        }
        self.result
            .take()
            .unwrap_or_else(|| Err(BoundaryError::new(BoundaryErrorKind::Failed)))
    }

    fn respond(&mut self, reply: CallbackReply) -> Result<(), BoundaryError> {
        self.replies.lock().expect("replies lock").push(reply);
        Ok(())
    }
}

#[derive(Default)]
struct FakeCredentials {
    value: Mutex<Option<StoredCredential>>,
    fail_load: AtomicBool,
    fail_save: AtomicBool,
    fail_delete: AtomicBool,
    deletes: Mutex<Vec<(String, String)>>,
}

impl FakeCredentials {
    fn with(credential: StoredCredential) -> Self {
        Self {
            value: Mutex::new(Some(credential)),
            ..Self::default()
        }
    }

    fn current(&self) -> Option<StoredCredential> {
        self.value.lock().expect("credential lock").clone()
    }
}

impl CredentialStore for FakeCredentials {
    fn load(&self, _origin: &str) -> Result<Option<StoredCredential>, BoundaryError> {
        if self.fail_load.load(Ordering::Acquire) {
            return Err(BoundaryError::new(BoundaryErrorKind::Failed));
        }
        Ok(self.current())
    }

    fn save(&self, credential: &StoredCredential) -> Result<(), BoundaryError> {
        if self.fail_save.load(Ordering::Acquire) {
            return Err(BoundaryError::new(BoundaryErrorKind::Failed));
        }
        *self.value.lock().expect("credential lock") = Some(credential.clone());
        Ok(())
    }

    fn delete(&self, origin: &str, account: &str) -> Result<(), BoundaryError> {
        self.deletes
            .lock()
            .expect("deletes lock")
            .push((origin.to_owned(), account.to_owned()));
        if self.fail_delete.load(Ordering::Acquire) {
            return Err(BoundaryError::new(BoundaryErrorKind::Failed));
        }
        *self.value.lock().expect("credential lock") = None;
        Ok(())
    }
}

struct Fixture {
    http: FakeHttp,
    browser: FakeBrowser,
    clock: FakeClock,
    random: FakeRandom,
    callbacks: FakeCallbacks,
    credentials: FakeCredentials,
}

impl Fixture {
    fn login_with(responses: Vec<Result<HttpResponse, BoundaryError>>) -> Self {
        let clock = FakeClock::new(NOW);
        Self {
            http: FakeHttp::new(responses),
            browser: FakeBrowser::default(),
            callbacks: FakeCallbacks::new(
                callback_request(&expected_state(), PORT, PORT),
                clock.clone(),
            ),
            clock,
            random: FakeRandom::default(),
            credentials: FakeCredentials::default(),
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
}

#[test]
fn login_uses_the_site_pkce_contract_and_persists_the_account() {
    let fixture = Fixture::login_with(vec![Ok(token_response(
        ACCESS_ONE,
        REFRESH_ONE,
        NOW + 3600,
        "harlan",
    ))]);

    let result = login(&fixture.dependencies(), &LoginOptions::new("3.0.0-alpha.1"));

    assert_eq!(
        result,
        Ok(skilld_auth::SessionSummary {
            account: "harlan".to_owned(),
            expires_at: NOW + 3600,
            scopes: Some("cli".to_owned()),
        })
    );
    let authorization_url = fixture.browser.urls.lock().expect("browser lock")[0].clone();
    let authorization_url = Url::parse(&authorization_url).expect("authorization URL");
    assert_eq!(authorization_url.path(), "/cli/authorize");
    let expected_verifier = URL_SAFE_NO_PAD.encode([1_u8; 32]);
    let expected_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(expected_verifier.as_bytes()));
    assert_eq!(
        authorization_url
            .query_pairs()
            .find(|(key, _)| key == "challenge")
            .map(|(_, value)| value.into_owned()),
        Some(expected_challenge)
    );
    assert_eq!(
        authorization_url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned()),
        Some(expected_state())
    );
    assert_eq!(
        authorization_url
            .query_pairs()
            .find(|(key, _)| key == "port")
            .map(|(_, value)| value.into_owned()),
        Some(PORT.to_string())
    );
    let requests = fixture.http.requests();
    assert_eq!(requests[0].url, "https://skilld.dev/api/cli/oauth/token");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("token body");
    assert_eq!(body["redirect_uri"], format!("http://127.0.0.1:{PORT}/"));
    assert_eq!(
        fixture
            .callbacks
            .replies
            .lock()
            .expect("replies lock")
            .as_slice(),
        &[CallbackReply::Accepted]
    );
    assert_eq!(
        fixture.credentials.current().map(|value| value.account),
        Some("harlan".to_owned())
    );
}

#[test]
fn login_rejects_a_state_mismatch_before_token_exchange() {
    let mut fixture = Fixture::login_with(Vec::new());
    fixture.callbacks = FakeCallbacks::new(
        callback_request("wrong-state-value", PORT, PORT),
        fixture.clock.clone(),
    );

    let error =
        login(&fixture.dependencies(), &LoginOptions::new("3.0.0")).expect_err("state mismatch");

    assert_eq!(error.kind(), AuthErrorKind::StateMismatch);
    assert!(fixture.http.requests().is_empty());
    assert_eq!(
        fixture
            .callbacks
            .replies
            .lock()
            .expect("replies lock")
            .as_slice(),
        &[CallbackReply::Rejected]
    );
}

#[test]
fn login_rejects_duplicate_callback_values() {
    let mut fixture = Fixture::login_with(Vec::new());
    let raw = format!(
        "GET /?code=code-valid-1234567890&code=injected-code-1234&state={} HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\n\r\n",
        expected_state()
    );
    fixture.callbacks = FakeCallbacks::new(raw_callback(raw, PORT), fixture.clock.clone());

    let error = login(&fixture.dependencies(), &LoginOptions::new("3.0.0"))
        .expect_err("callback injection");

    assert_eq!(error.kind(), AuthErrorKind::CallbackInjection);
    assert!(fixture.http.requests().is_empty());
}

#[test]
fn login_rejects_a_callback_host_port_mismatch() {
    let mut fixture = Fixture::login_with(Vec::new());
    fixture.callbacks = FakeCallbacks::new(
        callback_request(&expected_state(), PORT + 1, PORT),
        fixture.clock.clone(),
    );

    let error =
        login(&fixture.dependencies(), &LoginOptions::new("3.0.0")).expect_err("port mismatch");

    assert_eq!(error.kind(), AuthErrorKind::CallbackPortMismatch);
    assert!(fixture.http.requests().is_empty());
}

#[test]
fn login_rejects_an_expired_authorization_code() {
    let fixture = Fixture::login_with(Vec::new());
    fixture
        .callbacks
        .advance_to
        .store(NOW + 300, Ordering::Release);

    let error =
        login(&fixture.dependencies(), &LoginOptions::new("3.0.0")).expect_err("expired code");

    assert_eq!(error.kind(), AuthErrorKind::ExpiredAuthorizationCode);
    assert!(fixture.http.requests().is_empty());
}

#[test]
fn login_rejects_an_expired_access_token() {
    let fixture = Fixture::login_with(vec![Ok(token_response(
        ACCESS_ONE,
        REFRESH_ONE,
        NOW,
        "harlan",
    ))]);

    let error =
        login(&fixture.dependencies(), &LoginOptions::new("3.0.0")).expect_err("expired token");

    assert_eq!(error.kind(), AuthErrorKind::ExpiredToken);
    assert_eq!(fixture.credentials.current(), None);
}

#[test]
fn login_maps_a_rejected_site_code_to_expiry() {
    let fixture = Fixture::login_with(vec![Ok(HttpResponse {
        status: 401,
        headers: Vec::new(),
        body: br#"{"message":"Invalid or expired code"}"#.to_vec(),
    })]);

    let error = login(&fixture.dependencies(), &LoginOptions::new("3.0.0"))
        .expect_err("site rejected code");

    assert_eq!(error.kind(), AuthErrorKind::ExpiredAuthorizationCode);
    assert_eq!(fixture.credentials.current(), None);
}

#[test]
fn refresh_uses_each_rotated_token_once() {
    let fixture = Fixture {
        http: FakeHttp::new(vec![
            Ok(token_response(
                ACCESS_TWO,
                REFRESH_TWO,
                NOW + 3600,
                "harlan",
            )),
            Ok(token_response(
                ACCESS_ONE,
                REFRESH_THREE,
                NOW + 7200,
                "harlan",
            )),
        ]),
        browser: FakeBrowser::default(),
        clock: FakeClock::new(NOW),
        random: FakeRandom::default(),
        callbacks: FakeCallbacks::failing(
            BoundaryError::new(BoundaryErrorKind::Failed),
            FakeClock::new(NOW),
        ),
        credentials: FakeCredentials::with(credential(ACCESS_ONE, REFRESH_ONE, NOW + 100)),
    };
    let cancellation = CancellationToken::new();

    refresh(
        &fixture.dependencies(),
        Duration::from_secs(30),
        &cancellation,
    )
    .expect("first refresh");
    refresh(
        &fixture.dependencies(),
        Duration::from_secs(30),
        &cancellation,
    )
    .expect("second refresh");

    let requests = fixture.http.requests();
    let first: Value = serde_json::from_slice(&requests[0].body).expect("first body");
    let second: Value = serde_json::from_slice(&requests[1].body).expect("second body");
    assert_eq!(first["refresh_token"], REFRESH_ONE);
    assert_eq!(second["refresh_token"], REFRESH_TWO);
    assert_eq!(
        fixture
            .credentials
            .current()
            .and_then(|value| value.refresh_token)
            .map(|value| value.expose_secret().to_owned()),
        Some(REFRESH_THREE.to_owned())
    );
}

#[test]
fn refresh_rejects_a_response_that_replays_the_same_token() {
    let fixture = Fixture {
        http: FakeHttp::new(vec![Ok(token_response(
            ACCESS_TWO,
            REFRESH_ONE,
            NOW + 3600,
            "harlan",
        ))]),
        browser: FakeBrowser::default(),
        clock: FakeClock::new(NOW),
        random: FakeRandom::default(),
        callbacks: FakeCallbacks::failing(
            BoundaryError::new(BoundaryErrorKind::Failed),
            FakeClock::new(NOW),
        ),
        credentials: FakeCredentials::with(credential(ACCESS_ONE, REFRESH_ONE, NOW + 100)),
    };

    let error = refresh(
        &fixture.dependencies(),
        Duration::from_secs(30),
        &CancellationToken::new(),
    )
    .expect_err("refresh replay");

    assert_eq!(error.kind(), AuthErrorKind::InvalidResponse);
    assert_eq!(
        fixture
            .credentials
            .current()
            .and_then(|value| value.refresh_token)
            .map(|value| value.expose_secret().to_owned()),
        Some(REFRESH_ONE.to_owned())
    );
}

#[test]
fn keychain_failure_returns_a_redacted_error() {
    let fixture = Fixture::login_with(vec![Ok(token_response(
        ACCESS_ONE,
        REFRESH_ONE,
        NOW + 3600,
        "harlan",
    ))]);
    fixture.credentials.fail_save.store(true, Ordering::Release);

    let error =
        login(&fixture.dependencies(), &LoginOptions::new("3.0.0")).expect_err("keychain failure");
    let rendered = format!("{error:?} {error}");

    assert_eq!(error.kind(), AuthErrorKind::CredentialStoreFailed);
    assert!(!rendered.contains(ACCESS_ONE));
    assert!(!rendered.contains(REFRESH_ONE));
}

#[test]
fn logout_revokes_then_deletes_the_bound_account() {
    let fixture = Fixture {
        http: FakeHttp::new(vec![Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: br#"{"ok":true}"#.to_vec(),
        })]),
        browser: FakeBrowser::default(),
        clock: FakeClock::new(NOW),
        random: FakeRandom::default(),
        callbacks: FakeCallbacks::failing(
            BoundaryError::new(BoundaryErrorKind::Failed),
            FakeClock::new(NOW),
        ),
        credentials: FakeCredentials::with(credential(ACCESS_ONE, REFRESH_ONE, NOW + 100)),
    };

    logout(
        &fixture.dependencies(),
        Duration::from_secs(30),
        &CancellationToken::new(),
    )
    .expect("logout");

    assert_eq!(fixture.credentials.current(), None);
    assert_eq!(
        fixture
            .credentials
            .deletes
            .lock()
            .expect("deletes lock")
            .as_slice(),
        &[(SKILLD_ORIGIN.to_owned(), "harlan".to_owned())]
    );
    let requests = fixture.http.requests();
    assert_eq!(requests[0].url, "https://skilld.dev/api/cli/logout");
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.as_str()),
        Some("Bearer access-token-one-1234567890")
    );
}

#[test]
fn logout_deletes_the_local_credential_when_revocation_fails() {
    let fixture = Fixture {
        http: FakeHttp::new(vec![Err(BoundaryError::new(BoundaryErrorKind::Failed))]),
        browser: FakeBrowser::default(),
        clock: FakeClock::new(NOW),
        random: FakeRandom::default(),
        callbacks: FakeCallbacks::failing(
            BoundaryError::new(BoundaryErrorKind::Failed),
            FakeClock::new(NOW),
        ),
        credentials: FakeCredentials::with(credential(ACCESS_ONE, REFRESH_ONE, NOW + 100)),
    };

    let error = logout(
        &fixture.dependencies(),
        Duration::from_secs(30),
        &CancellationToken::new(),
    )
    .expect_err("remote logout failure");

    assert_eq!(error.kind(), AuthErrorKind::HttpFailed);
    assert_eq!(fixture.credentials.current(), None);
}

#[test]
fn login_rejects_a_response_over_the_token_limit() {
    let fixture = Fixture::login_with(vec![Ok(HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: vec![b'x'; 64 * 1024 + 1],
    })]);

    let error =
        login(&fixture.dependencies(), &LoginOptions::new("3.0.0")).expect_err("response limit");

    assert_eq!(error.kind(), AuthErrorKind::ResponseTooLarge);
    assert_eq!(fixture.http.requests()[0].max_response_bytes, 64 * 1024);
}

#[test]
fn login_rejects_more_than_three_same_origin_redirects() {
    let redirect = || {
        Ok(HttpResponse {
            status: 307,
            headers: vec![("location".to_owned(), "/api/cli/oauth/token".to_owned())],
            body: Vec::new(),
        })
    };
    let fixture = Fixture::login_with(vec![redirect(), redirect(), redirect(), redirect()]);

    let error =
        login(&fixture.dependencies(), &LoginOptions::new("3.0.0")).expect_err("redirect limit");

    assert_eq!(error.kind(), AuthErrorKind::RedirectRejected);
    assert_eq!(fixture.http.requests().len(), 4);
}

#[test]
fn login_rejects_a_redirect_that_leaves_skilld_dev() {
    let fixture = Fixture::login_with(vec![Ok(HttpResponse {
        status: 307,
        headers: vec![(
            "location".to_owned(),
            "https://example.com/token".to_owned(),
        )],
        body: Vec::new(),
    })]);

    let error = login(&fixture.dependencies(), &LoginOptions::new("3.0.0"))
        .expect_err("cross-origin redirect");

    assert_eq!(error.kind(), AuthErrorKind::RedirectRejected);
    assert_eq!(fixture.http.requests().len(), 1);
}

#[test]
fn login_surfaces_callback_timeout() {
    let mut fixture = Fixture::login_with(Vec::new());
    fixture.callbacks = FakeCallbacks::failing(
        BoundaryError::new(BoundaryErrorKind::Timeout),
        fixture.clock.clone(),
    );

    let error = login(&fixture.dependencies(), &LoginOptions::new("3.0.0")).expect_err("timeout");

    assert_eq!(error.kind(), AuthErrorKind::Timeout);
}

#[test]
fn login_stops_before_side_effects_when_cancelled() {
    let fixture = Fixture::login_with(Vec::new());
    let options = LoginOptions::new("3.0.0");
    options.cancellation.cancel();

    let error = login(&fixture.dependencies(), &options).expect_err("cancelled");

    assert_eq!(error.kind(), AuthErrorKind::Cancelled);
    assert_eq!(fixture.callbacks.binds.load(Ordering::Acquire), 0);
    assert!(
        fixture
            .browser
            .urls
            .lock()
            .expect("browser lock")
            .is_empty()
    );
}

#[test]
fn status_reports_expiry_without_returning_tokens() {
    let fixture = Fixture {
        http: FakeHttp::new(Vec::new()),
        browser: FakeBrowser::default(),
        clock: FakeClock::new(NOW),
        random: FakeRandom::default(),
        callbacks: FakeCallbacks::failing(
            BoundaryError::new(BoundaryErrorKind::Failed),
            FakeClock::new(NOW),
        ),
        credentials: FakeCredentials::with(credential(ACCESS_ONE, REFRESH_ONE, NOW - 1)),
    };

    let result = status(&fixture.dependencies());

    assert_eq!(
        result,
        Ok(AuthStatus::Expired {
            account: "harlan".to_owned(),
            expired_at: NOW - 1,
        })
    );
}

#[test]
fn status_rejects_a_credential_bound_to_another_origin() {
    let mut wrong = credential(ACCESS_ONE, REFRESH_ONE, NOW + 100);
    wrong.origin = "https://example.com".to_owned();
    let fixture = Fixture {
        http: FakeHttp::new(Vec::new()),
        browser: FakeBrowser::default(),
        clock: FakeClock::new(NOW),
        random: FakeRandom::default(),
        callbacks: FakeCallbacks::failing(
            BoundaryError::new(BoundaryErrorKind::Failed),
            FakeClock::new(NOW),
        ),
        credentials: FakeCredentials::with(wrong),
    };

    let error = status(&fixture.dependencies()).expect_err("origin mismatch");

    assert_eq!(error.kind(), AuthErrorKind::AccountMismatch);
}

#[test]
fn status_surfaces_the_wasi_credential_capability_seam() {
    let http = FakeHttp::new(Vec::new());
    let browser = FakeBrowser::default();
    let clock = FakeClock::new(NOW);
    let random = FakeRandom::default();
    let callbacks = FakeCallbacks::failing(
        BoundaryError::new(BoundaryErrorKind::Unsupported),
        clock.clone(),
    );
    let credentials = UnsupportedCredentialStore;
    let dependencies = AuthDependencies {
        http: &http,
        browser: &browser,
        clock: &clock,
        random: &random,
        callbacks: &callbacks,
        credentials: &credentials,
    };

    let error = status(&dependencies).expect_err("unsupported credential capability");

    assert_eq!(error.kind(), AuthErrorKind::UnsupportedCapability);
}

#[test]
fn native_loopback_listener_accepts_one_strict_callback() {
    let listener = NativeLoopbackListener;
    let mut binding = listener.bind().expect("loopback listener");
    let port = binding.port();
    let state = "state-value-1234567890";
    let request = format!(
        "GET /?code=code-valid-1234567890&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    );
    let browser = thread::spawn(move || {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("callback connect");
        stream
            .write_all(request.as_bytes())
            .expect("callback write");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("callback response");
        response
    });

    let received = binding
        .receive(Duration::from_secs(2), &CancellationToken::new())
        .expect("callback request");
    let code = parse_callback_request(&received, port, state).expect("strict callback");
    binding
        .respond(CallbackReply::Accepted)
        .expect("callback reply");

    assert_eq!(code.expose_secret(), "code-valid-1234567890");
    assert!(
        browser
            .join()
            .expect("browser thread")
            .starts_with("HTTP/1.1 200 OK")
    );
}

fn expected_state() -> String {
    URL_SAFE_NO_PAD.encode([2_u8; 32])
}

fn callback_request(state: &str, host_port: u16, local_port: u16) -> CallbackRequest {
    raw_callback(
        format!(
            "GET /?code=code-valid-1234567890&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{host_port}\r\n\r\n"
        ),
        local_port,
    )
}

fn raw_callback(value: String, local_port: u16) -> CallbackRequest {
    CallbackRequest {
        bytes: value.into_bytes(),
        peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 53_000),
        local_port,
    }
}

fn token_response(access: &str, refresh: &str, expires_at: u64, login: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&json!({
            "accessToken": access,
            "refreshToken": refresh,
            "expiresAt": expires_at,
            "login": login,
            "scopes": "cli",
        }))
        .expect("token response"),
    }
}

fn credential(access: &str, refresh: &str, expires_at: u64) -> StoredCredential {
    StoredCredential {
        origin: SKILLD_ORIGIN.to_owned(),
        account: "harlan".to_owned(),
        access_token: SecretString::new(access),
        refresh_token: Some(SecretString::new(refresh)),
        expires_at,
        scopes: Some("cli".to_owned()),
    }
}
