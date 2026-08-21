use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use skilld_command::{
    Cancellation, HeaderValue, Host, HttpAdapter, HttpRequest, HttpResponse, LocalHost,
    NativeRemoteConfig, NoTokenProvider, PreparedRemoteSkill, RemoteComparisonAccess,
    RemoteComparisonOutcome, RemoteComparisonRelation, RemoteProvider, RemoteSourceState,
    RemoteUpdateComparison, SecretValue, SkilldRemote, Sleeper, TokenProvider, run,
};
use skilld_core::{
    AgentTargetId, ArtifactAttestation, ArtifactFile, AttestationSignature, CheckOutcome,
    CheckResult, CommitAuthor, CommitSha, CommitSummary, InstallMode, InstallOperation,
    InstallRequest, InstallScope, InstallSource, LockedSource, PreparedFile, RemoteError,
    RemoteSelector, RepositoryVisibility, ResolvedSource, SearchResponse, SignatureAlgorithm,
    SourceProvider, SourceStatus, TrustedRootPin, UpdatePlanV1, UpdateRelation,
};

const ROOT_DOMAIN: &[u8] = b"skilld-trusted-key-v1\0";
const ATTESTATION_DOMAIN: &[u8] = b"skilld-attestation-v1\0";

#[derive(Default)]
struct FakeHttp {
    responses: Mutex<VecDeque<Result<HttpResponse, RemoteError>>>,
    requests: Mutex<Vec<HttpRequest>>,
    timeouts: Mutex<Vec<Option<Duration>>>,
}

impl FakeHttp {
    fn with(responses: impl IntoIterator<Item = HttpResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(vec![]),
            timeouts: Mutex::new(vec![]),
        }
    }
}

impl HttpAdapter for FakeHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
        timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        self.requests.lock().unwrap().push(request.clone());
        self.timeouts.lock().unwrap().push(timeout);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                Err(RemoteError::new(
                    "HTTP_TRANSPORT",
                    "the fake response queue is empty",
                ))
            })
    }
}

#[derive(Default)]
struct UpdatePlansHttp {
    requests: Mutex<Vec<HttpRequest>>,
    active: AtomicUsize,
    maximum_concurrency: AtomicUsize,
}

impl HttpAdapter for UpdatePlansHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
        _timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum_concurrency.fetch_max(active, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(5));
        self.requests.lock().unwrap().push(request.clone());
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let results = body["comparisons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|comparison| {
                json!({
                    "_tag": "ready",
                    "id": comparison["id"],
                    "owner": comparison["owner"],
                    "repository": comparison["repository"],
                    "baseSha": comparison["baseSha"],
                    "headSha": comparison["headSha"],
                    "relation": "ahead",
                    "aheadBy": 1,
                    "behindBy": 0,
                    "commits": [{
                        "sha": comparison["headSha"],
                        "subject": "Update Skill",
                        "timestamp": "2026-08-21T00:00:00Z",
                        "author": { "name": "Ada Lovelace", "login": "ada" }
                    }],
                    "total": 1,
                    "truncated": false,
                    "compareUrl": format!(
                        "https://github.com/{}/{}/compare/{}...{}",
                        comparison["owner"].as_str().unwrap(),
                        comparison["repository"].as_str().unwrap(),
                        comparison["baseSha"].as_str().unwrap(),
                        comparison["headSha"].as_str().unwrap(),
                    )
                })
            })
            .collect::<Vec<_>>();
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(response(
            200,
            serde_json::to_vec(&json!({ "results": results })).unwrap(),
        ))
    }
}

#[derive(Default)]
struct NoSleep;

impl Sleeper for NoSleep {
    fn sleep(
        &self,
        _duration: Duration,
        cancellation: &dyn Cancellation,
    ) -> Result<(), RemoteError> {
        if cancellation.is_cancelled() {
            Err(RemoteError::new("CANCELLED", "cancelled"))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct RecordingSleeper {
    elapsed: Mutex<Duration>,
}

impl Sleeper for RecordingSleeper {
    fn sleep(
        &self,
        duration: Duration,
        cancellation: &dyn Cancellation,
    ) -> Result<(), RemoteError> {
        if cancellation.is_cancelled() {
            return Err(RemoteError::new("CANCELLED", "cancelled"));
        }
        *self.elapsed.lock().unwrap() += duration;
        Ok(())
    }
}

#[derive(Default)]
struct TestCancellation(AtomicBool);

impl Cancellation for TestCancellation {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

struct CancellingSleeper {
    cancellation: Arc<TestCancellation>,
}

impl Sleeper for CancellingSleeper {
    fn sleep(
        &self,
        _duration: Duration,
        _cancellation: &dyn Cancellation,
    ) -> Result<(), RemoteError> {
        self.cancellation.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct FixedToken;

impl TokenProvider for FixedToken {
    fn access_token(&self) -> Result<Option<SecretValue>, RemoteError> {
        Ok(Some(SecretValue::new("account-token").unwrap()))
    }
}

fn response(status: u16, body: impl Into<Vec<u8>>) -> HttpResponse {
    HttpResponse {
        status,
        headers: BTreeMap::new(),
        body: body.into(),
    }
}

fn signed_message(domain: &[u8], statement: &[u8]) -> Vec<u8> {
    let mut message = domain.to_vec();
    message.extend_from_slice(&Sha256::digest(statement));
    message
}

fn tar_skill(content: &[u8]) -> Vec<u8> {
    let mut header = [0_u8; 512];
    header[..8].copy_from_slice(b"SKILL.md");
    write_octal(&mut header[100..108], 0o644);
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    write_octal(&mut header[124..136], content.len() as u64);
    write_octal(&mut header[136..148], 0);
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum = header.iter().map(|byte| u64::from(*byte)).sum::<u64>();
    let checksum = format!("{checksum:06o}");
    header[148..154].copy_from_slice(checksum.as_bytes());
    header[154] = 0;
    header[155] = b' ';
    let mut archive = header.to_vec();
    archive.extend_from_slice(content);
    archive.resize(archive.len().div_ceil(512) * 512, 0);
    archive.extend([0_u8; 1024]);
    archive
}

fn write_octal(field: &mut [u8], value: u64) {
    field.fill(b'0');
    let value = format!("{value:o}");
    let start = field.len() - value.len() - 1;
    field[start..start + value.len()].copy_from_slice(value.as_bytes());
    field[field.len() - 1] = 0;
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn verified_remote_responses() -> (TrustedRootPin, Vec<HttpResponse>) {
    let skill = b"---\nname: example\ndescription: verified\n---\n";
    let archive = tar_skill(skill);
    let root_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
    let root_public_key = URL_SAFE_NO_PAD.encode(root_key.verifying_key().to_bytes());
    let signing_public_key = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes());
    let key_statement = serde_json::to_vec(&json!({
        "version": 1,
        "rootKeyId": "root-1",
        "keyId": "signer-1",
        "algorithm": "Ed25519",
        "publicKey": signing_public_key,
        "notBefore": "2026-01-01T00:00:00.000Z",
        "notAfter": "2027-01-01T00:00:00.000Z",
        "status": "active"
    }))
    .unwrap();
    let root_signature = root_key.sign(&signed_message(ROOT_DOMAIN, &key_statement));
    let source = ResolvedSource {
        provider: SourceProvider::Github,
        repository_id: 1,
        owner: "skilld-dev".to_owned(),
        repository: "skills".to_owned(),
        visibility: RepositoryVisibility::Public,
        commit_sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        tree_sha: "89abcdef0123456789abcdef0123456789abcdef".to_owned(),
        skill_path: "skills/example".to_owned(),
    };
    let file = ArtifactFile {
        path: "SKILL.md".to_owned(),
        mode: 0o644,
        size: skill.len() as u64,
        sha256: hex(&Sha256::digest(skill)),
    };
    let checks = vec![CheckResult {
        name: "path-policy".to_owned(),
        version: "1".to_owned(),
        outcome: CheckOutcome::Pass,
        required: true,
        summary: None,
        findings: vec![],
    }];
    let content_sha256 = hex(&Sha256::digest(&archive));
    let artifact_id = format!("sha256:{content_sha256}");
    let statement = serde_json::to_vec(&json!({
        "version": 1,
        "artifactId": artifact_id,
        "createdAt": "2026-08-20T00:00:00.000Z",
        "source": source,
        "sourceStatus": "verified",
        "format": "skilld-tar-v1",
        "contentSha256": content_sha256,
        "contentBytes": archive.len(),
        "policyVersion": "2026-08-20",
        "files": [file.clone()],
        "checkResults": checks
    }))
    .unwrap();
    let signature = signing_key.sign(&signed_message(ATTESTATION_DOMAIN, &statement));
    let attestation = ArtifactAttestation {
        version: 1,
        artifact_id: artifact_id.clone(),
        created_at: "2026-08-20T00:00:00.000Z".to_owned(),
        source,
        source_status: "verified".to_owned(),
        format: "skilld-tar-v1".to_owned(),
        content_sha256,
        content_bytes: archive.len() as u64,
        policy_version: "2026-08-20".to_owned(),
        files: vec![file],
        check_results: checks,
        statement: URL_SAFE_NO_PAD.encode(statement),
        signature: AttestationSignature {
            algorithm: SignatureAlgorithm::Ed25519,
            key_id: "signer-1".to_owned(),
            value: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        },
    };
    let root = json!({
        "version": 1,
        "rootKeyId": "root-1",
        "rootPublicKey": root_public_key,
        "keys": [{
            "keyId": "signer-1",
            "algorithm": "Ed25519",
            "publicKey": signing_public_key,
            "notBefore": "2026-01-01T00:00:00.000Z",
            "notAfter": "2027-01-01T00:00:00.000Z",
            "status": "active",
            "statement": URL_SAFE_NO_PAD.encode(key_statement),
            "rootSignature": URL_SAFE_NO_PAD.encode(root_signature.to_bytes())
        }],
        "fetchedAt": "2026-08-20T00:00:00.000Z"
    });
    let ready = json!({
        "state": "ready",
        "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
        "artifact": {
            "artifactId": artifact_id,
            "visibility": "public",
            "attestation": attestation
        }
    });
    let grant = json!({
        "kind": "public",
        "artifactId": artifact_id,
        "contentUrl": "http://127.0.0.1:8787/content",
        "expiresAt": "2026-08-20T00:05:00.000Z",
        "attestation": attestation
    });
    (
        TrustedRootPin {
            key_id: "root-1".to_owned(),
            public_key: root_public_key,
        },
        vec![
            response(200, serde_json::to_vec(&ready).unwrap()),
            response(200, serde_json::to_vec(&root).unwrap()),
            response(200, serde_json::to_vec(&grant).unwrap()),
            response(200, archive),
        ],
    )
}

fn search_remote(http: Arc<FakeHttp>) -> SkilldRemote {
    SkilldRemote::new(
        http,
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep))
}

fn skilld_selector() -> RemoteSelector {
    RemoteSelector::parse("skilld:skilld-dev/skilld/skilld").unwrap()
}

#[test]
fn search_uses_only_the_v1_skilld_route_and_retries_a_bounded_failure() {
    let http = Arc::new(FakeHttp::with([
        response(503, b"unavailable".to_vec()),
        response(
            200,
            include_bytes!("../../../contracts/fixtures/v1/skill-search.json").to_vec(),
        ),
    ]));
    let remote = search_remote(http.clone());

    let response = remote.search("vue testing", 20).unwrap();

    assert_eq!(response.items[0].name, "vue-testing");
    assert_eq!(response.total, 1);
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        request.url == "http://127.0.0.1:8787/api/v1/skills?q=vue+testing&limit=20"
    }));
}

#[test]
fn public_comparison_uses_anonymous_github_and_keeps_full_commit_details() {
    let base = CommitSha::parse("1".repeat(40)).unwrap();
    let head = CommitSha::parse("2".repeat(40)).unwrap();
    let commit = "3".repeat(40);
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&json!({
            "status": "ahead",
            "ahead_by": 1,
            "behind_by": 0,
            "total_commits": 1,
            "commits": [{
                "sha": commit,
                "commit": {
                    "message": "Add grill timers\u{1b}[31m\nHidden body",
                    "author": {
                        "name": "Ada Lovelace",
                        "date": "2026-08-21T00:00:00Z"
                    }
                },
                "author": { "login": "ada" }
            }]
        }))
        .unwrap(),
    )]));
    let remote = search_remote(http.clone());
    let comparison = RemoteUpdateComparison::new(
        "grill",
        "acme",
        "skills",
        base,
        head,
        RemoteComparisonAccess::PublicGithub,
    )
    .unwrap();

    let results = remote.compare_updates(&[comparison]).unwrap();

    assert_eq!(results.len(), 1);
    let RemoteComparisonOutcome::Ready {
        relation,
        commits,
        total,
        truncated,
        compare_url,
        ahead_by,
        behind_by,
    } = &results[0].outcome
    else {
        panic!("expected a ready comparison")
    };
    assert_eq!(*relation, RemoteComparisonRelation::Ahead);
    assert_eq!((*ahead_by, *behind_by), (1, 0));
    assert_eq!((*total, *truncated), (1, false));
    assert_eq!(
        commits[0].sha.as_str(),
        "3333333333333333333333333333333333333333"
    );
    assert_eq!(commits[0].subject, "Add grill timers [31m");
    assert_eq!(commits[0].author.name, "Ada Lovelace");
    assert_eq!(commits[0].author.login.as_deref(), Some("ada"));
    assert_eq!(
        commits[0].url,
        "https://github.com/acme/skills/commit/3333333333333333333333333333333333333333"
    );
    assert_eq!(
        compare_url,
        "https://github.com/acme/skills/compare/1111111111111111111111111111111111111111...2222222222222222222222222222222222222222"
    );
    let request = &http.requests.lock().unwrap()[0];
    assert!(
        request
            .url
            .starts_with("https://api.github.com/repos/acme/skills/compare/")
    );
    assert!(
        request
            .headers
            .iter()
            .all(|header| header.name != "authorization")
    );
    assert!(request.headers.iter().any(|header| {
        header.name == "x-github-api-version" && header.value.expose() == "2026-03-10"
    }));
}

#[test]
fn public_comparison_keeps_the_newest_five_hundred_commits() {
    let responses = (0..6).map(|page| {
        let start = page * 100 + 1;
        let count = if page == 5 { 50 } else { 100 };
        response(
            200,
            serde_json::to_vec(&json!({
                "status": "ahead",
                "ahead_by": 550,
                "behind_by": 0,
                "total_commits": 550,
                "commits": (start..start + count).map(|number| json!({
                    "sha": format!("{number:040x}"),
                    "commit": {
                        "message": format!("Commit {number}"),
                        "author": {
                            "name": "Ada Lovelace",
                            "date": "2026-08-21T00:00:00Z"
                        }
                    },
                    "author": { "login": "ada" }
                })).collect::<Vec<_>>()
            }))
            .unwrap(),
        )
    });
    let http = Arc::new(FakeHttp::with(responses));
    let remote = search_remote(http.clone());
    let comparison = RemoteUpdateComparison::new(
        "grill",
        "acme",
        "skills",
        CommitSha::parse("1".repeat(40)).unwrap(),
        CommitSha::parse("2".repeat(40)).unwrap(),
        RemoteComparisonAccess::PublicGithub,
    )
    .unwrap();

    let results = remote.compare_updates(&[comparison]).unwrap();

    let RemoteComparisonOutcome::Ready {
        commits,
        total,
        truncated,
        ..
    } = &results[0].outcome
    else {
        panic!("expected a ready comparison")
    };
    assert_eq!(commits.len(), 500);
    assert_eq!(commits[0].sha.as_str(), format!("{:040x}", 51));
    assert_eq!(commits[499].sha.as_str(), format!("{:040x}", 550));
    assert_eq!((*total, *truncated), (550, true));
    assert_eq!(http.requests.lock().unwrap().len(), 6);
}

#[test]
fn public_comparison_rejects_an_incomplete_success_page() {
    let responses = (0..6).map(|page| {
        let start = page * 100 + 1;
        let count = if page == 2 {
            0
        } else if page == 5 {
            50
        } else {
            100
        };
        response(
            200,
            serde_json::to_vec(&json!({
                "status": "ahead",
                "ahead_by": 550,
                "behind_by": 0,
                "total_commits": 550,
                "commits": (start..start + count).map(|number| json!({
                    "sha": format!("{number:040x}"),
                    "commit": {
                        "message": format!("Commit {number}"),
                        "author": {
                            "name": "Ada Lovelace",
                            "date": "2026-08-21T00:00:00Z"
                        }
                    },
                    "author": { "login": "ada" }
                })).collect::<Vec<_>>()
            }))
            .unwrap(),
        )
    });
    let remote = search_remote(Arc::new(FakeHttp::with(responses)));
    let comparison = RemoteUpdateComparison::new(
        "grill",
        "acme",
        "skills",
        CommitSha::parse("1".repeat(40)).unwrap(),
        CommitSha::parse("2".repeat(40)).unwrap(),
        RemoteComparisonAccess::PublicGithub,
    )
    .unwrap();

    let results = remote.compare_updates(&[comparison]).unwrap();

    assert!(matches!(
        results[0].outcome,
        RemoteComparisonOutcome::RequestFailure {
            code: "INVALID_RESPONSE",
            ..
        }
    ));
}

#[test]
fn public_rate_limit_exposes_the_github_reset_wait() {
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let mut limited = response(403, vec![]);
    limited
        .headers
        .insert("x-ratelimit-remaining".to_owned(), "0".to_owned());
    limited
        .headers
        .insert("x-ratelimit-reset".to_owned(), reset.to_string());
    let remote = search_remote(Arc::new(FakeHttp::with([limited])));
    let comparison = RemoteUpdateComparison::new(
        "grill",
        "acme",
        "skills",
        CommitSha::parse("1".repeat(40)).unwrap(),
        CommitSha::parse("2".repeat(40)).unwrap(),
        RemoteComparisonAccess::PublicGithub,
    )
    .unwrap();

    let results = remote.compare_updates(&[comparison]).unwrap();

    assert!(matches!(
        results[0].outcome,
        RemoteComparisonOutcome::RateLimited {
            retry_after_seconds: Some(1..=300),
            reset_at: None,
        }
    ));
}

#[test]
fn hosted_comparisons_are_authenticated_and_chunked_at_fifty() {
    let http = Arc::new(UpdatePlansHttp::default());
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(FixedToken),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));
    let comparisons = (0..51)
        .map(|index| {
            RemoteUpdateComparison::new(
                format!("skill-{index}"),
                "acme",
                "private-skills",
                CommitSha::parse("1".repeat(40)).unwrap(),
                CommitSha::parse("2".repeat(40)).unwrap(),
                RemoteComparisonAccess::Hosted,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();

    let results = remote.compare_updates(&comparisons).unwrap();

    assert_eq!(results.len(), 51);
    assert!(results.iter().all(|result| matches!(
        result.outcome,
        RemoteComparisonOutcome::Ready {
            relation: RemoteComparisonRelation::Ahead,
            ..
        }
    )));
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let mut sizes = requests
        .iter()
        .map(|request| {
            assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/update-plans");
            assert!(request.headers.iter().any(|header| {
                header.name == "authorization" && header.value.expose() == "Bearer account-token"
            }));
            serde_json::from_slice::<serde_json::Value>(&request.body).unwrap()["comparisons"]
                .as_array()
                .unwrap()
                .len()
        })
        .collect::<Vec<_>>();
    sizes.sort_unstable();
    assert_eq!(sizes, [1, 50]);
    assert!(http.maximum_concurrency.load(Ordering::SeqCst) <= 4);
}

#[test]
fn hosted_comparison_accepts_the_largest_valid_commit_batch() {
    let comparisons = (0..25)
        .map(|index| {
            RemoteUpdateComparison::new(
                format!("skill-{index}"),
                "acme",
                "private-skills",
                CommitSha::parse("1".repeat(40)).unwrap(),
                CommitSha::parse("2".repeat(40)).unwrap(),
                RemoteComparisonAccess::Hosted,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let commits = (0..500)
        .map(|index| {
            json!({
                "sha": format!("{index:040x}"),
                "subject": "s".repeat(500),
                "timestamp": "2026-08-21T00:00:00Z",
                "author": {
                    "name": "a".repeat(200),
                    "login": "l".repeat(100),
                }
            })
        })
        .collect::<Vec<_>>();
    let results = comparisons
        .iter()
        .map(|comparison| {
            json!({
                "_tag": "ready",
                "id": comparison.id,
                "owner": comparison.owner,
                "repository": comparison.repository,
                "baseSha": comparison.base_sha.as_str(),
                "headSha": comparison.head_sha.as_str(),
                "relation": "ahead",
                "aheadBy": 500,
                "behindBy": 0,
                "commits": commits,
                "total": 500,
                "truncated": false,
                "compareUrl": format!(
                    "https://github.com/acme/private-skills/compare/{}...{}",
                    comparison.base_sha.as_str(),
                    comparison.head_sha.as_str(),
                )
            })
        })
        .collect::<Vec<_>>();
    let body = serde_json::to_vec(&json!({ "results": results })).unwrap();
    assert!(body.len() > 8 * 1024 * 1024);
    let remote = SkilldRemote::new(
        Arc::new(FakeHttp::with([response(200, body)])),
        Arc::new(FixedToken),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap();

    let results = remote.compare_updates(&comparisons).unwrap();

    assert!(results.iter().all(|result| matches!(
        result.outcome,
        RemoteComparisonOutcome::Ready {
            total: 500,
            truncated: false,
            ..
        }
    )));
}

#[test]
fn hosted_comparison_keeps_per_item_failures_and_retry_after() {
    let inputs = ["ready", "limited"]
        .into_iter()
        .map(|id| {
            RemoteUpdateComparison::new(
                id,
                "acme",
                "private-skills",
                CommitSha::parse("1".repeat(40)).unwrap(),
                CommitSha::parse("2".repeat(40)).unwrap(),
                RemoteComparisonAccess::Hosted,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let identity = |id: &str| {
        json!({
            "id": id,
            "owner": "acme",
            "repository": "private-skills",
            "baseSha": "1".repeat(40),
            "headSha": "2".repeat(40),
        })
    };
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&json!({
            "results": [
                {
                    "_tag": "ready",
                    "id": identity("ready")["id"],
                    "owner": identity("ready")["owner"],
                    "repository": identity("ready")["repository"],
                    "baseSha": identity("ready")["baseSha"],
                    "headSha": identity("ready")["headSha"],
                    "relation": "ahead",
                    "aheadBy": 1,
                    "behindBy": 0,
                    "commits": [{
                        "sha": identity("ready")["headSha"],
                        "subject": "Update Skill",
                        "timestamp": "2026-08-21T00:00:00Z",
                        "author": { "name": "Ada Lovelace", "login": "ada" }
                    }],
                    "total": 1,
                    "truncated": false,
                    "compareUrl": format!(
                        "https://github.com/acme/private-skills/compare/{}...{}",
                        "1".repeat(40),
                        "2".repeat(40)
                    )
                },
                {
                    "_tag": "rate_limited",
                    "id": identity("limited")["id"],
                    "owner": identity("limited")["owner"],
                    "repository": identity("limited")["repository"],
                    "baseSha": identity("limited")["baseSha"],
                    "headSha": identity("limited")["headSha"],
                    "retryAfterSeconds": 12,
                    "resetAt": "2026-08-21T00:10:00.000Z"
                }
            ]
        }))
        .unwrap(),
    )]));
    let remote = SkilldRemote::new(http, Arc::new(FixedToken), NativeRemoteConfig::Unconfigured)
        .with_endpoint("http://127.0.0.1:8787")
        .unwrap()
        .with_sleeper(Arc::new(NoSleep));

    let results = remote.compare_updates(&inputs).unwrap();

    assert!(matches!(
        results[0].outcome,
        RemoteComparisonOutcome::Ready {
            relation: RemoteComparisonRelation::Ahead,
            ..
        }
    ));
    assert!(matches!(
        &results[1].outcome,
        RemoteComparisonOutcome::RateLimited {
            retry_after_seconds: Some(12),
            reset_at: Some(reset_at),
        } if reset_at == "2026-08-21T00:10:00.000Z"
    ));
}

#[test]
fn invalid_hosted_item_does_not_hide_a_ready_sibling() {
    let inputs = ["ready", "invalid"]
        .into_iter()
        .map(|id| {
            RemoteUpdateComparison::new(
                id,
                "acme",
                "private-skills",
                CommitSha::parse("1".repeat(40)).unwrap(),
                CommitSha::parse("2".repeat(40)).unwrap(),
                RemoteComparisonAccess::Hosted,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let ready = |id: &str, subject: &str| {
        json!({
            "_tag": "ready",
            "id": id,
            "owner": "acme",
            "repository": "private-skills",
            "baseSha": "1".repeat(40),
            "headSha": "2".repeat(40),
            "relation": "ahead",
            "aheadBy": 1,
            "behindBy": 0,
            "commits": [{
                "sha": "2".repeat(40),
                "subject": subject,
                "timestamp": "2026-08-21T00:00:00Z",
                "author": { "name": "Ada Lovelace", "login": "ada" }
            }],
            "total": 1,
            "truncated": false,
            "compareUrl": format!(
                "https://github.com/acme/private-skills/compare/{}...{}",
                "1".repeat(40),
                "2".repeat(40),
            )
        })
    };
    let body = serde_json::to_vec(&json!({
        "results": [ready("ready", "Valid commit"), ready("invalid", "")]
    }))
    .unwrap();
    let remote = SkilldRemote::new(
        Arc::new(FakeHttp::with([response(200, body)])),
        Arc::new(FixedToken),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap();

    let results = remote.compare_updates(&inputs).unwrap();

    assert!(matches!(
        results[0].outcome,
        RemoteComparisonOutcome::Ready { .. }
    ));
    assert!(matches!(
        results[1].outcome,
        RemoteComparisonOutcome::RequestFailure {
            code: "INVALID_RESPONSE",
            ..
        }
    ));
}

#[test]
fn hosted_batch_rate_limit_stays_visible_for_each_item() {
    let input = RemoteUpdateComparison::new(
        "private",
        "acme",
        "private-skills",
        CommitSha::parse("1".repeat(40)).unwrap(),
        CommitSha::parse("2".repeat(40)).unwrap(),
        RemoteComparisonAccess::Hosted,
    )
    .unwrap();
    let mut limited = response(429, vec![]);
    limited
        .headers
        .insert("retry-after".to_owned(), "120".to_owned());
    let remote = SkilldRemote::new(
        Arc::new(FakeHttp::with([limited])),
        Arc::new(FixedToken),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));

    let results = remote.compare_updates(&[input]).unwrap();

    assert!(matches!(
        results[0].outcome,
        RemoteComparisonOutcome::RateLimited {
            retry_after_seconds: Some(120),
            reset_at: None,
        }
    ));
}

#[test]
fn a_cross_origin_redirect_is_rejected_before_the_second_request() {
    let mut redirect = response(302, vec![]);
    redirect.headers.insert(
        "location".to_owned(),
        "https://example.com/steal".to_owned(),
    );
    let http = Arc::new(FakeHttp::with([redirect]));
    let remote = search_remote(http.clone());

    let error = remote.search("testing", 20).unwrap_err();

    assert_eq!(error.code, "REMOTE_ORIGIN_REJECTED");
    assert_eq!(http.requests.lock().unwrap().len(), 1);
}

#[test]
fn a_search_response_over_the_limit_is_rejected() {
    let http = Arc::new(FakeHttp::with([response(200, vec![b' '; 1024 * 1024 + 1])]));
    let remote = search_remote(http);

    let error = remote.search("testing", 20).unwrap_err();

    assert_eq!(error.code, "RESPONSE_TOO_LARGE");
}

#[test]
fn a_resolution_cannot_change_its_identity_while_polling() {
    let first_id = "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f";
    let second_id = "118f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f";
    let http = Arc::new(FakeHttp::with([
        response(
            200,
            serde_json::to_vec(&json!({
                "state": "pending",
                "resolutionId": first_id,
                "stage": "checking",
                "pollAfterMs": 250
            }))
            .unwrap(),
        ),
        response(
            200,
            serde_json::to_vec(&json!({
                "state": "blocked",
                "resolutionId": second_id,
                "checkResults": []
            }))
            .unwrap(),
        ),
    ]));
    let remote = search_remote(http);

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "INVALID_RESPONSE");
}

#[test]
fn a_pending_resolution_times_out_after_at_most_sixty_seconds() {
    let resolution_id = "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f";
    let pending = || {
        response(
            200,
            serde_json::to_vec(&json!({
                "state": "pending",
                "resolutionId": resolution_id,
                "stage": "checking",
                "pollAfterMs": 30_000
            }))
            .unwrap(),
        )
    };
    let http = Arc::new(FakeHttp::with([pending(), pending(), pending()]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(sleeper.clone());

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "RESOLUTION_TIMEOUT");
    assert_eq!(
        error.message,
        "Artifact creation stayed pending too long. Retry the same command."
    );
    assert_eq!(*sleeper.elapsed.lock().unwrap(), Duration::from_secs(60));
    assert_eq!(http.requests.lock().unwrap().len(), 2);
}

#[test]
fn resolution_retry_waits_count_toward_the_sixty_second_limit() {
    let resolution_id = "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f";
    let pending = || {
        response(
            200,
            serde_json::to_vec(&json!({
                "state": "pending",
                "resolutionId": resolution_id,
                "stage": "checking",
                "pollAfterMs": 30_000
            }))
            .unwrap(),
        )
    };
    let mut retry = response(503, b"unavailable".to_vec());
    retry
        .headers
        .insert("retry-after".to_owned(), "60".to_owned());
    let http = Arc::new(FakeHttp::with([pending(), retry, pending(), pending()]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(sleeper.clone());

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "RESOLUTION_TIMEOUT");
    assert_eq!(*sleeper.elapsed.lock().unwrap(), Duration::from_secs(60));
    assert_eq!(http.requests.lock().unwrap().len(), 2);
    let timeouts = http.timeouts.lock().unwrap();
    assert!(timeouts[0].is_some_and(|timeout| timeout <= Duration::from_secs(60)));
    assert!(timeouts[1].is_some_and(|timeout| timeout <= Duration::from_secs(30)));
}

#[test]
fn cancellation_wins_when_the_resolution_deadline_is_reached() {
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&json!({
            "state": "pending",
            "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
            "stage": "checking",
            "pollAfterMs": 60_000
        }))
        .unwrap(),
    )]));
    let cancellation = Arc::new(TestCancellation::default());
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_cancellation(cancellation.clone())
    .with_sleeper(Arc::new(CancellingSleeper { cancellation }));

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "CANCELLED");
    assert_eq!(http.requests.lock().unwrap().len(), 1);
}

#[test]
fn a_resolution_descriptor_must_match_its_attestation() {
    let attestation: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/fixtures/v1/artifact-attestation.json"
    ))
    .unwrap();
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&json!({
            "state": "ready",
            "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
            "artifact": {
                "artifactId": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "visibility": "public",
                "attestation": attestation
            }
        }))
        .unwrap(),
    )]));
    let remote = search_remote(http);

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "ATTESTATION_MISMATCH");
}

#[test]
fn a_native_build_without_the_root_pin_fails_closed() {
    let attestation: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/fixtures/v1/artifact-attestation.json"
    ))
    .unwrap();
    let artifact_id = attestation["artifactId"].as_str().unwrap();
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&json!({
            "state": "ready",
            "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
            "artifact": {
                "artifactId": artifact_id,
                "visibility": "public",
                "attestation": attestation
            }
        }))
        .unwrap(),
    )]));
    let remote = search_remote(http);

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "TRUSTED_ROOT_UNCONFIGURED");
}

#[test]
fn a_verified_remote_install_uses_resolution_root_grant_and_content_in_order() {
    let (pin, responses) = verified_remote_responses();
    let http = Arc::new(FakeHttp::with(responses));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));
    let selector = RemoteSelector::parse("skilld:skilld-dev/skills/example").unwrap();

    let prepared = remote.prepare(&selector, false).unwrap();

    assert!(matches!(
        prepared.source_status,
        SourceStatus::Verified { .. }
    ));
    assert_eq!(prepared.files[0].path, "SKILL.md");
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].url.ends_with("/api/v1/resolutions"));
    assert!(requests[1].url.ends_with("/api/v1/trusted-root"));
    assert!(requests[2].url.contains("/api/v1/artifacts/"));
    assert!(requests[2].url.ends_with("/grants"));
    assert!(requests[3].url.ends_with("/content"));
}

#[test]
fn a_private_artifact_download_sends_the_account_and_one_time_grant() {
    let (pin, mut responses) = verified_remote_responses();
    let public_grant: serde_json::Value = serde_json::from_slice(&responses[2].body).unwrap();
    responses[2] = response(
        200,
        serde_json::to_vec(&json!({
            "kind": "private",
            "artifactId": public_grant["artifactId"],
            "contentUrl": public_grant["contentUrl"],
            "expiresAt": public_grant["expiresAt"],
            "downloadToken": "private-grant-token-with-enough-bytes",
            "attestation": public_grant["attestation"]
        }))
        .unwrap(),
    );
    let http = Arc::new(FakeHttp::with(responses));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(FixedToken),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));

    remote
        .prepare(
            &RemoteSelector::parse("skilld:skilld-dev/skills/example").unwrap(),
            false,
        )
        .unwrap();

    let requests = http.requests.lock().unwrap();
    let content = requests.last().unwrap();
    assert!(content.headers.iter().any(|header| {
        header.name == "authorization" && header.value.expose() == "Bearer account-token"
    }));
    assert!(content.headers.iter().any(|header| {
        header.name == "x-skilld-grant"
            && header.value.expose() == "private-grant-token-with-enough-bytes"
    }));
}

#[test]
fn direct_github_access_resolves_an_exact_public_commit_without_tokens() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let encoded = base64::engine::general_purpose::STANDARD.encode(skill);
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let tree = "89abcdef0123456789abcdef0123456789abcdef";
    let blob = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let http = Arc::new(FakeHttp::with([
        response(
            200,
            br#"{"private":false,"default_branch":"main"}"#.to_vec(),
        ),
        response(
            200,
            format!(r#"{{"sha":"{sha}","commit":{{"tree":{{"sha":"{tree}"}}}}}}"#),
        ),
        response(
            200,
            format!(
                r#"{{"truncated":false,"tree":[{{"path":"skills/example/SKILL.md","mode":"100644","type":"blob","sha":"{blob}","size":{}}}]}}"#,
                skill.len()
            ),
        ),
        response(
            200,
            format!(
                r#"{{"content":"{encoded}","encoding":"base64","size":{}}}"#,
                skill.len()
            ),
        ),
    ]));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_sleeper(Arc::new(NoSleep));
    let selector = RemoteSelector::parse("github:skilld-dev/skills/skills/example").unwrap();

    let prepared = remote.prepare(&selector, true).unwrap();

    assert!(matches!(
        prepared.source_status,
        SourceStatus::Unverified { .. }
    ));
    assert!(matches!(
        prepared.locked_source,
        LockedSource::Remote { ref commit_sha, .. } if commit_sha == sha
    ));
    assert!(http.requests.lock().unwrap().iter().all(|request| {
        request
            .headers
            .iter()
            .all(|header| header.name != "authorization")
    }));
}

#[test]
fn direct_github_access_rejects_private_repositories() {
    let http = Arc::new(FakeHttp::with([response(
        200,
        br#"{"private":true,"default_branch":"main"}"#.to_vec(),
    )]));
    let remote = SkilldRemote::new(
        http,
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    );
    let selector = RemoteSelector::parse("github:skilld-dev/skills/skills/example").unwrap();

    let error = remote.prepare(&selector, true).unwrap_err();

    assert_eq!(error.code, "DIRECT_PRIVATE_UNSUPPORTED");
}

#[test]
fn secret_headers_are_redacted_from_debug_output() {
    let secret = SecretValue::new("token-value").unwrap();
    let value = HeaderValue::Secret(secret);

    let debug = format!("{value:?}");

    assert!(!debug.contains("token-value"));
    assert!(debug.contains("REDACTED"));
}

struct FakeProvider {
    content: Mutex<Vec<u8>>,
    stale: Mutex<bool>,
    fail_prepare: Mutex<bool>,
    prepares: Mutex<Vec<(String, bool)>>,
}

impl FakeProvider {
    fn prepared(&self, selector: &RemoteSelector, direct: bool) -> PreparedRemoteSkill {
        let bytes = self.content.lock().unwrap().clone();
        let file = PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes,
        };
        let digest = installed_digest(std::slice::from_ref(&file));
        PreparedRemoteSkill {
            files: vec![file],
            locked_source: LockedSource::Remote {
                source: selector.canonical(),
                commit_sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                skill_path: "skills/example".to_owned(),
            },
            source_status: if direct {
                SourceStatus::Unverified {
                    content_sha256: digest.clone(),
                    installed_sha256: digest,
                }
            } else {
                SourceStatus::Verified {
                    artifact_id: format!("sha256:{digest}"),
                    content_sha256: digest.clone(),
                    installed_sha256: digest,
                    attestation_key_id: "test-key".to_owned(),
                }
            },
        }
    }
}

impl RemoteProvider for FakeProvider {
    fn search(&self, _query: &str, _limit: u8) -> Result<SearchResponse, RemoteError> {
        skilld_core::parse_search_response(include_bytes!(
            "../../../contracts/fixtures/v1/skill-search.json"
        ))
    }

    fn prepare(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        self.prepares
            .lock()
            .unwrap()
            .push((selector.canonical(), direct));
        if *self.fail_prepare.lock().unwrap() {
            Err(RemoteError::new("CHECK_BLOCKED", "a required check failed"))
        } else {
            Ok(self.prepared(selector, direct))
        }
    }

    fn prepare_exact(
        &self,
        selector: &RemoteSelector,
        expected_commit: &CommitSha,
        _direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        if *self.fail_prepare.lock().unwrap() {
            return Err(RemoteError::new("CHECK_BLOCKED", "a required check failed"));
        }
        let mut prepared = self.prepared(selector, _direct);
        let LockedSource::Remote { commit_sha, .. } = &mut prepared.locked_source else {
            unreachable!("the fixture uses a remote source")
        };
        *commit_sha = expected_commit.as_str().to_owned();
        Ok(prepared)
    }

    fn source_state(
        &self,
        _selector: &RemoteSelector,
        _artifact_id: &str,
        _commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError> {
        if *self.stale.lock().unwrap() {
            Ok(RemoteSourceState::Stale {
                current_artifact_id: "sha256:new".to_owned(),
                current_commit_sha: "ffffffffffffffffffffffffffffffffffffffff".to_owned(),
            })
        } else {
            Ok(RemoteSourceState::Current)
        }
    }

    fn compare_updates(
        &self,
        comparisons: &[RemoteUpdateComparison],
    ) -> Result<Vec<skilld_command::RemoteUpdateResult>, RemoteError> {
        let stale = *self.stale.lock().unwrap();
        Ok(comparisons
            .iter()
            .map(|comparison| skilld_command::RemoteUpdateResult {
                id: comparison.id.clone(),
                outcome: RemoteComparisonOutcome::Ready {
                    relation: if stale {
                        RemoteComparisonRelation::Ahead
                    } else {
                        RemoteComparisonRelation::Identical
                    },
                    ahead_by: u64::from(stale),
                    behind_by: 0,
                    commits: if stale {
                        vec![CommitSummary {
                            sha: comparison.head_sha.clone(),
                            subject: "Update example".to_owned(),
                            author: CommitAuthor {
                                name: "Ada Lovelace".to_owned(),
                                login: Some("ada".to_owned()),
                            },
                            timestamp: "2026-08-21T00:00:00Z".to_owned(),
                            url: format!(
                                "https://github.com/{}/{}/commit/{}",
                                comparison.owner,
                                comparison.repository,
                                comparison.head_sha.as_str(),
                            ),
                        }]
                    } else {
                        vec![]
                    },
                    total: u64::from(stale),
                    truncated: false,
                    compare_url: format!(
                        "https://github.com/{}/{}/compare/{}...{}",
                        comparison.owner,
                        comparison.repository,
                        comparison.base_sha.as_str(),
                        comparison.head_sha.as_str(),
                    ),
                },
            })
            .collect())
    }

    fn latest_commit(
        &self,
        _selector: &RemoteSelector,
        _direct: bool,
    ) -> Result<skilld_command::RemoteLatestCommit, RemoteError> {
        Ok(skilld_command::RemoteLatestCommit {
            commit_sha: CommitSha::parse(if *self.stale.lock().unwrap() {
                "ffffffffffffffffffffffffffffffffffffffff"
            } else {
                "0123456789abcdef0123456789abcdef01234567"
            })
            .unwrap(),
            access: RemoteComparisonAccess::PublicGithub,
        })
    }
}

fn installed_digest(files: &[PreparedFile]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update((file.path.len() as u64).to_be_bytes());
        hasher.update(file.path.as_bytes());
        hasher.update((file.bytes.len() as u64).to_be_bytes());
        hasher.update(&file.bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn provider(content: &str) -> Arc<FakeProvider> {
    Arc::new(FakeProvider {
        content: Mutex::new(content.as_bytes().to_vec()),
        stale: Mutex::new(false),
        fail_prepare: Mutex::new(false),
        prepares: Mutex::new(vec![]),
    })
}

struct BatchProvider {
    version: Mutex<&'static str>,
    prepared_names: Mutex<Vec<String>>,
    fail_name: Mutex<Option<&'static str>>,
    relation: Mutex<RemoteComparisonRelation>,
}

impl BatchProvider {
    fn prepared(&self, selector: &RemoteSelector) -> PreparedRemoteSkill {
        let skilld_core::SourceSelector::NamedSkill { name } = &selector.source().selector else {
            panic!("expected a named Skill selector")
        };
        self.prepared_names.lock().unwrap().push(name.clone());
        let bytes = format!(
            "---\nname: {name}\ndescription: {}\n---\n",
            *self.version.lock().unwrap()
        )
        .into_bytes();
        let file = PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes,
        };
        let digest = installed_digest(std::slice::from_ref(&file));
        PreparedRemoteSkill {
            files: vec![file],
            locked_source: LockedSource::Remote {
                source: selector.canonical(),
                commit_sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                skill_path: name.clone(),
            },
            source_status: SourceStatus::Verified {
                artifact_id: format!("sha256:{digest}"),
                content_sha256: digest.clone(),
                installed_sha256: digest,
                attestation_key_id: "test-key".to_owned(),
            },
        }
    }
}

impl RemoteProvider for BatchProvider {
    fn search(&self, _query: &str, _limit: u8) -> Result<SearchResponse, RemoteError> {
        unreachable!("search is outside this test")
    }

    fn prepare(
        &self,
        selector: &RemoteSelector,
        _direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        Ok(self.prepared(selector))
    }

    fn prepare_exact(
        &self,
        selector: &RemoteSelector,
        expected_commit: &CommitSha,
        _direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        let mut prepared = self.prepared(selector);
        let skilld_core::SourceSelector::NamedSkill { name } = &selector.source().selector else {
            panic!("expected a named Skill selector")
        };
        if self.fail_name.lock().unwrap().as_ref() == Some(&name.as_str()) {
            return Err(RemoteError::new("CHECK_BLOCKED", "a required check failed"));
        }
        let LockedSource::Remote { commit_sha, .. } = &mut prepared.locked_source else {
            unreachable!("the fixture uses a remote source")
        };
        *commit_sha = expected_commit.as_str().to_owned();
        Ok(prepared)
    }

    fn source_state(
        &self,
        _selector: &RemoteSelector,
        _artifact_id: &str,
        _commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError> {
        Ok(RemoteSourceState::Current)
    }

    fn latest_commit(
        &self,
        _selector: &RemoteSelector,
        _direct: bool,
    ) -> Result<skilld_command::RemoteLatestCommit, RemoteError> {
        Ok(skilld_command::RemoteLatestCommit {
            commit_sha: CommitSha::parse("f".repeat(40)).unwrap(),
            access: RemoteComparisonAccess::PublicGithub,
        })
    }

    fn compare_updates(
        &self,
        comparisons: &[RemoteUpdateComparison],
    ) -> Result<Vec<skilld_command::RemoteUpdateResult>, RemoteError> {
        Ok(comparisons
            .iter()
            .map(|comparison| {
                let relation = *self.relation.lock().unwrap();
                let (ahead_by, behind_by) = match relation {
                    RemoteComparisonRelation::Ahead => (1, 0),
                    RemoteComparisonRelation::Behind => (0, 1),
                    RemoteComparisonRelation::Diverged => (1, 1),
                    RemoteComparisonRelation::Identical => (0, 0),
                };
                skilld_command::RemoteUpdateResult {
                    id: comparison.id.clone(),
                    outcome: RemoteComparisonOutcome::Ready {
                        relation,
                        ahead_by,
                        behind_by,
                        commits: vec![CommitSummary {
                            sha: comparison.head_sha.clone(),
                            subject: "Update Skill".to_owned(),
                            author: CommitAuthor {
                                name: "Ada Lovelace".to_owned(),
                                login: Some("ada".to_owned()),
                            },
                            timestamp: "2026-08-21T00:00:00Z".to_owned(),
                            url: format!(
                                "https://github.com/{}/{}/commit/{}",
                                comparison.owner,
                                comparison.repository,
                                comparison.head_sha.as_str(),
                            ),
                        }],
                        total: 1,
                        truncated: false,
                        compare_url: format!(
                            "https://github.com/{}/{}/compare/{}...{}",
                            comparison.owner,
                            comparison.repository,
                            comparison.base_sha.as_str(),
                            comparison.head_sha.as_str(),
                        ),
                    },
                }
            })
            .collect())
    }
}

#[test]
fn multi_skill_update_prepares_then_commits_every_artifact() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    for name in ["alpha", "beta"] {
        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Remote(format!(
                "skilld:skilld-dev/skills/{name}"
            ))),
            scope: InstallScope::Project,
            targets: vec![AgentTargetId::Codex],
            mode: Some(InstallMode::Copy),
        })
        .unwrap();
    }
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";

    let lines = host.update(None).unwrap();

    assert_eq!(lines, ["Updated Skill alpha.", "Updated Skill beta."]);
    assert_eq!(*provider.prepared_names.lock().unwrap(), ["alpha", "beta"]);
    for name in ["alpha", "beta"] {
        assert_eq!(
            fs::read_to_string(project.join(format!(".skills/{name}/SKILL.md"))).unwrap(),
            format!("---\nname: {name}\ndescription: second\n---\n")
        );
        assert_eq!(
            fs::read_to_string(project.join(format!(".agents/skills/{name}/SKILL.md"))).unwrap(),
            format!("---\nname: {name}\ndescription: second\n---\n")
        );
    }
}

#[test]
fn multi_skill_update_changes_nothing_when_one_artifact_cannot_prepare() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    for name in ["alpha", "beta"] {
        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Remote(format!(
                "skilld:skilld-dev/skills/{name}"
            ))),
            scope: InstallScope::Project,
            targets: vec![AgentTargetId::Codex],
            mode: Some(InstallMode::Copy),
        })
        .unwrap();
    }
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";
    *provider.fail_name.lock().unwrap() = Some("beta");

    let error = host.update(None).unwrap_err();

    assert_eq!(error.code, "CHECK_BLOCKED");
    assert_eq!(*provider.prepared_names.lock().unwrap(), ["alpha", "beta"]);
    for name in ["alpha", "beta"] {
        let expected = format!("---\nname: {name}\ndescription: first\n---\n");
        assert_eq!(
            fs::read_to_string(project.join(format!(".skills/{name}/SKILL.md"))).unwrap(),
            expected
        );
        assert_eq!(
            fs::read_to_string(project.join(format!(".agents/skills/{name}/SKILL.md"))).unwrap(),
            expected
        );
    }
}

#[test]
fn plain_update_rejects_a_source_that_moved_behind() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(
            "skilld:skilld-dev/skills/alpha".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
    })
    .unwrap();
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";
    *provider.relation.lock().unwrap() = RemoteComparisonRelation::Behind;

    let error = host.update(None).unwrap_err();

    assert_eq!(error.code, "UPDATE_CONFIRMATION_REQUIRED");
    assert!(provider.prepared_names.lock().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(project.join(".skills/alpha/SKILL.md")).unwrap(),
        "---\nname: alpha\ndescription: first\n---\n"
    );
}

#[test]
fn selected_skill_update_commits_only_the_exact_subset() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    for name in ["alpha", "beta", "gamma"] {
        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Remote(format!(
                "skilld:skilld-dev/skills/{name}"
            ))),
            scope: InstallScope::Project,
            targets: vec![AgentTargetId::Codex],
            mode: Some(InstallMode::Copy),
        })
        .unwrap();
    }
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";

    let lines = host
        .update_selected(&["gamma".to_owned(), "alpha".to_owned()])
        .unwrap();

    assert_eq!(lines, ["Updated Skill gamma.", "Updated Skill alpha."]);
    assert_eq!(*provider.prepared_names.lock().unwrap(), ["gamma", "alpha"]);
    for (name, version) in [("alpha", "second"), ("beta", "first"), ("gamma", "second")] {
        assert_eq!(
            fs::read_to_string(project.join(format!(".skills/{name}/SKILL.md"))).unwrap(),
            format!("---\nname: {name}\ndescription: {version}\n---\n")
        );
    }
}

#[test]
fn selected_skill_update_rejects_empty_duplicate_and_invalid_names() {
    let temporary = tempfile::tempdir().unwrap();
    let host = LocalHost::new(
        temporary.path().join("project"),
        temporary.path().join("data"),
    );

    let empty = host.update_selected(&[]).unwrap_err();
    let duplicate = host
        .update_selected(&["alpha".to_owned(), "alpha".to_owned()])
        .unwrap_err();
    let invalid = host.update_selected(&["../alpha".to_owned()]).unwrap_err();

    assert_eq!(empty.code, "INVALID_SELECTION");
    assert_eq!(empty.message, "Select at least one Skill");
    assert_eq!(duplicate.code, "INVALID_SELECTION");
    assert_eq!(duplicate.message, "Select each Skill once");
    assert_eq!(invalid.code, "INVALID_SOURCE");
}

#[test]
fn selected_skill_update_changes_nothing_when_one_selected_artifact_fails() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    for name in ["alpha", "beta", "gamma"] {
        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Remote(format!(
                "skilld:skilld-dev/skills/{name}"
            ))),
            scope: InstallScope::Project,
            targets: vec![AgentTargetId::Codex],
            mode: Some(InstallMode::Copy),
        })
        .unwrap();
    }
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";
    *provider.fail_name.lock().unwrap() = Some("gamma");

    let error = host
        .update_selected(&["alpha".to_owned(), "gamma".to_owned()])
        .unwrap_err();

    assert_eq!(error.code, "CHECK_BLOCKED");
    assert_eq!(*provider.prepared_names.lock().unwrap(), ["alpha", "gamma"]);
    for name in ["alpha", "beta", "gamma"] {
        assert_eq!(
            fs::read_to_string(project.join(format!(".skills/{name}/SKILL.md"))).unwrap(),
            format!("---\nname: {name}\ndescription: first\n---\n")
        );
    }
}

#[test]
fn verify_reports_changed_bytes_and_stale_sources() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = provider("---\nname: example\ndescription: first\n---\n");
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(
            "skilld:skilld-dev/skills/example".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
    })
    .unwrap();
    *provider.stale.lock().unwrap() = true;

    let stale = host.verify(Some("example")).unwrap_err();
    assert_eq!(stale.code, "SOURCE_STALE");
    *provider.stale.lock().unwrap() = false;
    fs::write(
        project.join(".skills/example/SKILL.md"),
        "---\nname: example\ndescription: changed\n---\n",
    )
    .unwrap();

    let changed = host.verify(Some("example")).unwrap_err();
    assert_eq!(changed.code, "CONTENT_CHANGED");
}

#[test]
fn remote_install_verify_and_failed_update_use_the_normal_transaction() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let data = temporary.path().join("data");
    fs::create_dir_all(&project).unwrap();
    let provider = provider("---\nname: example\ndescription: first\n---\n");
    let host = LocalHost::new(project.clone(), data).with_remote_provider(provider.clone());
    let request = InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(
            "skilld:skilld-dev/skills/example".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
    };

    assert_eq!(host.install_request(request).unwrap(), ["example"]);
    assert_eq!(
        host.verify(Some("example")).unwrap(),
        ["Verified Skill example."]
    );
    let before = fs::read(project.join(".skills/example/SKILL.md")).unwrap();
    *provider.content.lock().unwrap() = b"---\nname: example\ndescription: second\n---\n".to_vec();
    *provider.stale.lock().unwrap() = true;
    *provider.fail_prepare.lock().unwrap() = true;

    let error = host.update(Some("example")).unwrap_err();

    assert_eq!(error.code, "CHECK_BLOCKED");
    assert_eq!(
        fs::read(project.join(".skills/example/SKILL.md")).unwrap(),
        before
    );
}

#[test]
fn update_check_carries_the_exact_comparison_and_commit_history() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = provider("---\nname: example\ndescription: first\n---\n");
    let host = LocalHost::new(project, temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(
            "skilld:skilld-dev/skills/example".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
    })
    .unwrap();
    *provider.stale.lock().unwrap() = true;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "update", "example", "--check", "--json"],
        &host,
        &mut stdout,
        &mut stderr,
    );
    let mut global_stdout = Vec::new();
    let mut global_stderr = Vec::new();
    let global_result = run(
        ["skilld", "--json", "update", "example", "--check"],
        &host,
        &mut global_stdout,
        &mut global_stderr,
    );
    let outcome: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    let check: UpdatePlanV1 = serde_json::from_value(outcome["data"].clone()).unwrap();

    assert_eq!(result.exit_code, 1);
    assert!(stderr.is_empty());
    assert_eq!(global_result.exit_code, 1);
    assert!(global_stderr.is_empty());
    assert_eq!(global_stdout, stdout);
    assert_eq!(outcome["schemaVersion"], 1);
    assert_eq!(outcome["_tag"], "Success");
    assert_eq!(outcome["command"], "update");
    assert_eq!(outcome["notices"], serde_json::json!([]));
    assert_eq!(outcome["data"]["items"][0]["relation"]["aheadBy"], 1);
    assert!(matches!(
        check.items()[0].relation(),
        UpdateRelation::Available {
            latest_commit_sha: commit_sha,
            ..
        } if commit_sha.as_str() == "ffffffffffffffffffffffffffffffffffffffff"
    ));
    assert_eq!(
        check.items()[0].history(),
        &skilld_core::CommitHistory::compared(
            vec![CommitSummary {
                sha: CommitSha::parse("f".repeat(40)).unwrap(),
                subject: "Update example".to_owned(),
                author: CommitAuthor {
                    name: "Ada Lovelace".to_owned(),
                    login: Some("ada".to_owned()),
                },
                timestamp: "2026-08-21T00:00:00Z".to_owned(),
                url: format!(
                    "https://github.com/skilld-dev/skills/commit/{}",
                    "f".repeat(40)
                ),
            }],
            1,
            false,
            format!(
                "https://github.com/skilld-dev/skills/compare/{}...{}",
                "0123456789abcdef0123456789abcdef01234567",
                "f".repeat(40)
            ),
        )
        .unwrap()
    );
}

#[test]
fn cli_direct_install_marks_review_as_required() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project, temporary.path().join("data"))
        .with_remote_provider(provider("---\nname: example\n---\n"));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        [
            "skilld",
            "install",
            "github:skilld-dev/skills/skills/example",
            "--direct",
            "--agent",
            "codex",
        ],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Installed Skill example.\nReview the unverified Skill before use.\n"
    );
    assert!(stderr.is_empty());
}

#[test]
fn cli_direct_restore_uses_the_locked_commit() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = provider("---\nname: example\ndescription: direct\n---\n");
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let installed = run(
        [
            "skilld",
            "install",
            "github:skilld-dev/skills/skills/example",
            "--direct",
            "--agent",
            "codex",
        ],
        &host,
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(installed.exit_code, 0);
    fs::remove_dir_all(project.join(".skills/example")).unwrap();
    fs::remove_dir_all(project.join(".agents")).unwrap();
    stdout.clear();
    stderr.clear();

    let restored = run(
        ["skilld", "install", "--direct"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(restored.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Installed Skill example.\nReview the unverified Skill before use.\n"
    );
    assert!(stderr.is_empty());
    assert_eq!(
        *provider.prepares.lock().unwrap(),
        [
            (
                "github:skilld-dev/skills/skills/example".to_owned(),
                true
            ),
            (
                "github:skilld-dev/skills/skills/example#commit:0123456789abcdef0123456789abcdef01234567"
                    .to_owned(),
                true
            )
        ]
    );
    let view = host.view("example", InstallScope::Project).unwrap();
    assert!(matches!(
        view.skill.source_status,
        SourceStatus::Unverified { .. }
    ));
    assert!(matches!(
        view.skill.source,
        LockedSource::Remote { ref commit_sha, .. }
            if commit_sha == "0123456789abcdef0123456789abcdef01234567"
    ));
    assert_eq!(view.skill.targets[0].agent, AgentTargetId::Codex);
    assert_eq!(view.skill.targets[0].mode, InstallMode::Copy);
    assert!(project.join(".agents/skills/example/SKILL.md").exists());
}

#[test]
fn cli_plain_restore_rejects_an_unverified_source_with_the_recovery_command() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = provider("---\nname: example\n---\n");
    let host = LocalHost::new(project, temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run(
            [
                "skilld",
                "install",
                "github:skilld-dev/skills/skills/example",
                "--direct",
                "--agent",
                "codex",
            ],
            &host,
            &mut stdout,
            &mut stderr,
        )
        .exit_code,
        0
    );
    stdout.clear();
    stderr.clear();

    let restored = run(
        ["skilld", "install", "--agent", "codex"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(restored.exit_code, 1);
    assert!(stdout.is_empty());
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "UNVERIFIED_SOURCE: run skilld install --direct to restore an unverified Skill\n"
    );
    assert_eq!(provider.prepares.lock().unwrap().len(), 1);
}

#[test]
fn cli_verified_restore_keeps_artifact_delivery() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = provider("---\nname: example\ndescription: verified\n---\n");
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run(
            [
                "skilld",
                "install",
                "skilld:skilld-dev/skills/example",
                "--agent",
                "codex",
            ],
            &host,
            &mut stdout,
            &mut stderr,
        )
        .exit_code,
        0
    );
    fs::remove_dir_all(project.join(".skills/example")).unwrap();
    fs::remove_dir_all(project.join(".agents")).unwrap();
    stdout.clear();
    stderr.clear();

    let restored = run(
        ["skilld", "install", "--agent", "codex"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(restored.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Installed Skill example.\n"
    );
    assert!(stderr.is_empty());
    assert_eq!(
        *provider.prepares.lock().unwrap(),
        [
            ("skilld:skilld-dev/skills/example".to_owned(), false),
            (
                "skilld:skilld-dev/skills/example#commit:0123456789abcdef0123456789abcdef01234567"
                    .to_owned(),
                false
            )
        ]
    );
    let view = host.view("example", InstallScope::Project).unwrap();
    assert!(matches!(
        view.skill.source_status,
        SourceStatus::Verified { .. }
    ));
}
