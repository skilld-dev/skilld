use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use skilld_command::{
    Cancellation, HeaderValue, Host, HttpAdapter, HttpRequest, HttpResponse, LocalHost,
    NativeRemoteConfig, NoTokenProvider, PreparedRemoteSkill, RemoteProvider, RemoteSourceState,
    SecretValue, SkilldRemote, Sleeper, TokenProvider, run,
};
use skilld_core::{
    AgentTargetId, ArtifactAttestation, ArtifactFile, AttestationSignature, CheckOutcome,
    CheckResult, InstallMode, InstallOperation, InstallRequest, InstallScope, InstallSource,
    LockedSource, PreparedFile, RemoteError, RemoteSelector, RepositoryVisibility, ResolvedSource,
    SearchResult, SignatureAlgorithm, SourceProvider, SourceStatus, TrustedRootPin,
};

const ROOT_DOMAIN: &[u8] = b"skilld-trusted-key-v1\0";
const ATTESTATION_DOMAIN: &[u8] = b"skilld-attestation-v1\0";

#[derive(Default)]
struct FakeHttp {
    responses: Mutex<VecDeque<Result<HttpResponse, RemoteError>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FakeHttp {
    fn with(responses: impl IntoIterator<Item = HttpResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(vec![]),
        }
    }
}

impl HttpAdapter for FakeHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
    ) -> Result<HttpResponse, RemoteError> {
        self.requests.lock().unwrap().push(request.clone());
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

    let results = remote.search("vue testing", 20).unwrap();

    assert_eq!(results[0].name, "vue-testing");
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        request.url == "http://127.0.0.1:8787/api/v1/skills?q=vue+testing&limit=20"
    }));
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
    fn search(&self, _query: &str, _limit: u8) -> Result<Vec<SearchResult>, RemoteError> {
        skilld_core::parse_search_response(include_bytes!(
            "../../../contracts/fixtures/v1/skill-search.json"
        ))
        .map(|response| response.items)
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
fn remote_install_verify_and_failed_upgrade_use_the_normal_transaction() {
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
    *provider.fail_prepare.lock().unwrap() = true;

    let error = host.upgrade(Some("example")).unwrap_err();

    assert_eq!(error.code, "CHECK_BLOCKED");
    assert_eq!(
        fs::read(project.join(".skills/example/SKILL.md")).unwrap(),
        before
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

    assert_eq!(restored.exit_code, 2);
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
