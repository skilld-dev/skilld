use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use skilld_command::{
    Cancellation, HeaderValue, Host, HttpAdapter, HttpRequest, HttpResponse, LocalHost,
    NativeRemoteConfig, NoTokenProvider, PreparedRemoteSkill, RemoteComparisonAccess,
    RemoteComparisonOutcome, RemoteComparisonRelation, RemoteProgress, RemoteProgressStage,
    RemoteProvider, RemoteSourceState, RemoteUpdateComparison, SecretValue, SkilldRemote, Sleeper,
    TokenProvider, run,
};
use skilld_core::{
    AgentTargetId, ArtifactAttestation, ArtifactFile, AttestationSignature, CheckOutcome,
    CheckResult, CommitAuthor, CommitSha, CommitSummary, InstallMode, InstallOperation,
    InstallRequest, InstallScope, InstallSource, LinkedFile, ListedOrigin, ListedSkill,
    LockedSource, MultiSkillRef, PreparedFile, RemoteError, RemoteSelector, RepositoryVisibility,
    ResolvedSource, SearchResponse, SignatureAlgorithm, SourceProvider, SourceStatus,
    TrustedRootPin, UpdatePlanItem, UpdatePlanV1, UpdateRelation,
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
struct RecordingProgress(Mutex<Vec<RemoteProgressStage>>);

impl RemoteProgress for RecordingProgress {
    fn stage(&self, stage: RemoteProgressStage) {
        self.0.lock().unwrap().push(stage);
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
    verified_remote_responses_for(b"---\nname: example\ndescription: verified\n---\n")
}

fn passing_checks() -> Vec<CheckResult> {
    vec![CheckResult {
        name: "path-policy".to_owned(),
        version: "1".to_owned(),
        outcome: CheckOutcome::Pass,
        required: true,
        summary: None,
        findings: vec![],
    }]
}

fn verified_remote_responses_for(skill: &[u8]) -> (TrustedRootPin, Vec<HttpResponse>) {
    verified_remote_responses_with(skill, passing_checks())
}

fn verified_remote_responses_with(
    skill: &[u8],
    checks: Vec<CheckResult>,
) -> (TrustedRootPin, Vec<HttpResponse>) {
    verified_remote_responses_linking(skill, checks, &[])
}

/// A ready Resolution whose attestation lists these linked files. A response
/// for each one follows the Artifact, in order.
fn verified_remote_responses_linking(
    skill: &[u8],
    checks: Vec<CheckResult>,
    linked: &[(&str, &[u8])],
) -> (TrustedRootPin, Vec<HttpResponse>) {
    let archive = tar_skill(skill);
    let linked_files = linked
        .iter()
        .map(|(path, bytes)| LinkedFile {
            path: (*path).to_owned(),
            mode: 0o755,
            size: bytes.len() as u64,
            git_blob_sha: git_blob_sha(bytes),
        })
        .collect::<Vec<_>>();
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
    let content_sha256 = hex(&Sha256::digest(&archive));
    let artifact_id = format!("sha256:{content_sha256}");
    let mut statement = json!({
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
    });
    if !linked_files.is_empty() {
        statement["linkedFiles"] = json!(linked_files);
    }
    let statement = serde_json::to_vec(&statement).unwrap();
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
        linked_files,
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
        ]
        .into_iter()
        .chain(
            linked
                .iter()
                .map(|(_, bytes)| response(200, bytes.to_vec())),
        )
        .collect(),
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

fn listed(owner: &str, repository: &str, name: &str, description: Option<&str>) -> ListedSkill {
    ListedSkill {
        name: name.to_owned(),
        owner: owner.to_owned(),
        repository: repository.to_owned(),
        description: description.map(str::to_owned),
        origin: ListedOrigin::Registry { path: None },
    }
}

fn registry_page(rows: &[(&str, &str, &str, Option<&str>)]) -> HttpResponse {
    let items = rows
        .iter()
        .map(|(owner, repo, name, description)| {
            json!({
                "name": name,
                "owner": owner,
                "repo": repo,
                "description": description,
                "stars": 12,
                "registryPath": format!("/gh/{owner}/{repo}/{name}"),
            })
        })
        .collect::<Vec<_>>();
    response(
        200,
        serde_json::to_vec(&json!({ "items": items, "total": items.len(), "page": 1 })).unwrap(),
    )
}

fn request_paths(http: &FakeHttp) -> Vec<String> {
    http.requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| {
            request
                .url
                .trim_start_matches("http://127.0.0.1:8787")
                .to_owned()
        })
        .collect()
}

#[test]
fn a_repository_ref_lists_that_repository_from_the_owner_index() {
    let http = Arc::new(FakeHttp::with([registry_page(&[
        (
            "vuejs",
            "core",
            "vue",
            Some("Build Vue interfaces.\nSecond line."),
        ),
        ("vuejs", "core", "Not A Skill", Some("dropped: no selector")),
        ("vuejs", "router", "vue-router", None),
        ("vuejs", "core", "composition", None),
    ])]));
    let remote = search_remote(http.clone());

    let listing = remote
        .list_skills(&MultiSkillRef::Repository {
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
        })
        .unwrap();

    assert_eq!(
        listing.items,
        [
            listed("vuejs", "core", "composition", None),
            listed("vuejs", "core", "vue", Some("Build Vue interfaces.")),
        ]
    );
    assert_eq!(request_paths(&http), ["/api/skills?owner=vuejs&limit=200"]);
}

fn github_repository(default_branch: &str, private: bool) -> HttpResponse {
    response(
        200,
        serde_json::to_vec(&json!({
            "private": private,
            "default_branch": default_branch,
        }))
        .unwrap(),
    )
}

fn github_tree(paths: &[&str]) -> HttpResponse {
    let tree = paths
        .iter()
        .map(|path| {
            json!({
                "path": path,
                "mode": "100644",
                "type": if path.ends_with(".md") { "blob" } else { "tree" },
                "sha": "a".repeat(40),
                "size": 10,
            })
        })
        .collect::<Vec<_>>();
    response(
        200,
        serde_json::to_vec(&json!({ "truncated": false, "tree": tree })).unwrap(),
    )
}

fn direct_listed(owner: &str, repository: &str, name: &str, path: &str) -> ListedSkill {
    ListedSkill {
        name: name.to_owned(),
        owner: owner.to_owned(),
        repository: repository.to_owned(),
        description: None,
        origin: ListedOrigin::Direct {
            path: path.to_owned(),
        },
    }
}

#[test]
fn explicit_direct_listing_does_not_contact_the_registry() {
    let http = Arc::new(FakeHttp::with([
        github_repository("main", false),
        github_tree(&["skills/vue/SKILL.md"]),
    ]));
    let remote = search_remote(http.clone());
    let listing = remote
        .list_direct_skills(&MultiSkillRef::Repository {
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
        })
        .unwrap();
    assert_eq!(
        listing.items,
        [direct_listed("vuejs", "core", "vue", "skills/vue")]
    );
    assert_eq!(
        request_paths(&http),
        [
            "https://api.github.com/repos/vuejs/core",
            "https://api.github.com/repos/vuejs/core/git/trees/main?recursive=1",
        ]
    );
}

#[test]
fn github_discovery_does_not_select_direct_delivery_in_run_commands() {
    let http = Arc::new(FakeHttp::with([
        registry_page(&[]),
        github_repository("main", false),
        github_tree(&["skills/vue/SKILL.md"]),
    ]));
    let temporary = tempfile::tempdir().unwrap();
    let host = LocalHost::new(
        temporary.path().join("project"),
        temporary.path().join("global"),
    )
    .with_remote_provider(Arc::new(search_remote(http)));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run(
        ["skilld", "run", "vuejs/core", "--json"],
        &host,
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(
        output["data"]["items"][0]["runArgv"],
        json!(["skilld", "run", "github:vuejs/core/skills/vue", "--json"])
    );
}

#[test]
fn a_repository_the_registry_does_not_list_falls_back_to_its_github_tree() {
    let http = Arc::new(FakeHttp::with([
        registry_page(&[("vuejs", "router", "vue-router", None)]),
        github_repository("main", false),
        github_tree(&[
            "README.md",
            "skills",
            "skills/vue/SKILL.md",
            "skills/vue/references/api.md",
            "skills/nuxt/SKILL.md",
        ]),
    ]));
    let remote = search_remote(http.clone());

    let listing = remote
        .list_skills(&MultiSkillRef::Repository {
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
        })
        .unwrap();

    assert_eq!(
        listing.items,
        [
            direct_listed("vuejs", "core", "nuxt", "skills/nuxt"),
            direct_listed("vuejs", "core", "vue", "skills/vue"),
        ]
    );
    assert_eq!(
        listing.items[0].selector(),
        "github:vuejs/core/skills/nuxt".to_owned()
    );
    assert_eq!(
        request_paths(&http),
        [
            "/api/skills?owner=vuejs&limit=200",
            "https://api.github.com/repos/vuejs/core",
            "https://api.github.com/repos/vuejs/core/git/trees/main?recursive=1",
        ]
    );
}

#[test]
fn the_fallback_listing_drops_a_skill_file_at_the_repository_root() {
    let http = Arc::new(FakeHttp::with([
        registry_page(&[]),
        github_repository("trunk", false),
        github_tree(&["SKILL.md"]),
    ]));
    let remote = search_remote(http);

    let listing = remote
        .list_skills(&MultiSkillRef::Repository {
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
        })
        .unwrap();

    assert!(listing.items.is_empty());
}

#[test]
fn a_private_or_missing_github_repository_lists_no_skills() {
    for github in [
        github_repository("main", true),
        response(404, br#"{"message":"Not Found"}"#.to_vec()),
    ] {
        let http = Arc::new(FakeHttp::with([registry_page(&[]), github]));
        let remote = search_remote(http);

        let listing = remote
            .list_skills(&MultiSkillRef::Repository {
                owner: "vuejs".to_owned(),
                repository: "core".to_owned(),
            })
            .unwrap();

        assert!(listing.items.is_empty());
    }
}

#[test]
fn a_truncated_github_tree_stops_the_fallback_listing() {
    let http = Arc::new(FakeHttp::with([
        registry_page(&[]),
        github_repository("main", false),
        response(
            200,
            serde_json::to_vec(&json!({ "truncated": true, "tree": [] })).unwrap(),
        ),
    ]));
    let remote = search_remote(http);

    let error = remote
        .list_skills(&MultiSkillRef::Repository {
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "DIRECT_SOURCE_TOO_LARGE");
}

#[test]
fn a_repository_past_the_direct_listing_cap_is_a_too_large_error() {
    let paths = (0..201)
        .map(|index| format!("skills/skill-{index:03}/SKILL.md"))
        .collect::<Vec<_>>();
    let http = Arc::new(FakeHttp::with([
        registry_page(&[]),
        github_repository("main", false),
        github_tree(&paths.iter().map(String::as_str).collect::<Vec<_>>()),
    ]));
    let remote = search_remote(http);

    let error = remote
        .list_skills(&MultiSkillRef::Repository {
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "DIRECT_SOURCE_TOO_LARGE");
}

#[test]
fn an_owner_index_past_page_one_is_merged_into_the_listing() {
    let page_one = response(
        200,
        serde_json::to_vec(&json!({
            "items": [{
                "name": "padding",
                "owner": "big",
                "repo": "other",
                "description": null,
                "stars": 1,
                "registryPath": "/gh/big/other/padding",
            }],
            "total": 300,
            "page": 1,
            "pages": 2,
        }))
        .unwrap(),
    );
    let page_two = registry_page(&[
        ("big", "wanted", "zulu", None),
        ("big", "wanted", "alpha", Some("On page two.")),
    ]);
    let http = Arc::new(FakeHttp::with([page_one, page_two]));
    let remote = search_remote(http.clone());

    let listing = remote
        .list_skills(&MultiSkillRef::Repository {
            owner: "big".to_owned(),
            repository: "wanted".to_owned(),
        })
        .unwrap();

    assert_eq!(
        listing.items,
        [
            listed("big", "wanted", "alpha", Some("On page two.")),
            listed("big", "wanted", "zulu", None),
        ]
    );
    assert_eq!(
        request_paths(&http),
        [
            "/api/skills?owner=big&limit=200",
            "/api/skills?owner=big&limit=200&page=2",
        ]
    );
}

/// Serves the same full page for every owner index request, whatever `page`
/// names, so only a client side page cap can end the listing.
#[derive(Default)]
struct RunawayOwnerIndexHttp {
    requests: Mutex<Vec<String>>,
}

const RUNAWAY_PAGE_ROWS: usize = 200;
const RUNAWAY_REQUEST_LIMIT: usize = 25;

impl HttpAdapter for RunawayOwnerIndexHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
        _timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.url.to_string());
        assert!(
            requests.len() <= RUNAWAY_REQUEST_LIMIT,
            "the owner index kept paging: {} requests issued",
            requests.len()
        );
        let mut page = json!({
            "items": (0..RUNAWAY_PAGE_ROWS)
                .map(|_| json!({
                    "name": "wanted",
                    "owner": "big",
                    "repo": "wanted",
                    "description": null,
                    "stars": 1,
                    "registryPath": "/gh/big/wanted/wanted",
                }))
                .collect::<Vec<_>>(),
            "total": 20_000_000,
        });
        if requests.len() == 1 {
            page["pages"] = json!(u64::MAX);
        }
        Ok(response(200, serde_json::to_vec(&page).unwrap()))
    }
}

#[test]
fn a_runaway_owner_index_stops_paging_and_lists_each_skill_once() {
    let http = Arc::new(RunawayOwnerIndexHttp::default());
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));

    let listing = remote
        .list_skills(&MultiSkillRef::Repository {
            owner: "big".to_owned(),
            repository: "wanted".to_owned(),
        })
        .unwrap();

    assert_eq!(listing.items, [listed("big", "wanted", "wanted", None)]);
    assert_eq!(http.requests.lock().unwrap().len(), RUNAWAY_REQUEST_LIMIT);
}

#[test]
fn a_collection_ref_lists_its_skills_in_order_and_expands_repository_entries() {
    let http = Arc::new(FakeHttp::with([
        response(
            200,
            serde_json::to_vec(&json!({
                "authorLogin": "harlan-zw",
                "slug": "nuxt",
                "skills": [
                    { "position": 0, "owner": "vuejs", "repo": "core", "name": "vue", "reason": "The reactive core." },
                    { "position": 1, "owner": "nuxt", "repo": "skills", "name": null, "reason": null },
                    { "position": 2, "owner": "vuejs", "repo": "core", "name": "vue", "reason": "Listed twice." },
                ]
            }))
            .unwrap(),
        ),
        // The named entry resolves through the rows of its Repository. No row
        // carries a path, so the named Skill keeps the hosted selector.
        registry_page(&[("vuejs", "core", "vue", None)]),
        registry_page(&[
            ("nuxt", "skills", "nuxt", Some("Build Nuxt apps.")),
            ("nuxt", "other", "ignored", None),
        ]),
    ]));
    let remote = search_remote(http.clone());

    let listing = remote
        .list_skills(&MultiSkillRef::Collection {
            login: "harlan-zw".to_owned(),
            slug: "nuxt".to_owned(),
        })
        .unwrap();

    assert_eq!(
        listing.items,
        [
            listed("vuejs", "core", "vue", Some("The reactive core.")),
            listed("nuxt", "skills", "nuxt", Some("Build Nuxt apps.")),
        ]
    );
    assert_eq!(
        request_paths(&http),
        [
            "/api/collections/by-author/harlan-zw/nuxt",
            "/api/skills?owner=vuejs&limit=200",
            "/api/skills?owner=nuxt&limit=200",
        ]
    );
}

#[test]
fn a_named_entry_and_a_repository_entry_list_one_skill_once() {
    let collection = response(
        200,
        serde_json::to_vec(&json!({
            "authorLogin": "harlan-zw",
            "slug": "nuxt",
            "skills": [
                { "position": 0, "owner": "vuejs", "repo": "core", "name": "vue", "reason": null },
                { "position": 1, "owner": "vuejs", "repo": "core", "name": null, "reason": null },
            ]
        }))
        .unwrap(),
    );
    let indexed = response(
        200,
        serde_json::to_vec(&json!({
            "items": [{
                "name": "vue",
                "owner": "vuejs",
                "repo": "core",
                "description": null,
                "stars": 12,
                "registryPath": "/gh/vuejs/core/vue",
                "skillFileUrl": format!(
                    "https://github.com/vuejs/core/blob/{}/skills/vue/SKILL.md",
                    "a".repeat(40)
                ),
            }],
            "total": 1,
            "page": 1,
        }))
        .unwrap(),
    );
    let http = Arc::new(FakeHttp::with([collection, indexed]));
    let remote = search_remote(http.clone());

    let listing = remote
        .list_skills(&MultiSkillRef::Collection {
            login: "harlan-zw".to_owned(),
            slug: "nuxt".to_owned(),
        })
        .unwrap();

    assert_eq!(
        listing.items,
        [ListedSkill {
            name: "vue".to_owned(),
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
            description: None,
            origin: ListedOrigin::Registry {
                path: Some("skills/vue".to_owned()),
            },
        }]
    );
    assert_eq!(listing.items[0].selector(), "vuejs/core/skills/vue");
    assert_eq!(
        request_paths(&http),
        [
            "/api/collections/by-author/harlan-zw/nuxt",
            "/api/skills?owner=vuejs&limit=200",
        ]
    );
}

/// Serves one collection whose entries name Repositories: `vuejs/core` is
/// indexed, `organizer/stalled` lists nothing and its index job stays queued.
#[derive(Default)]
struct CollectionExpansionHttp {
    polls: Mutex<usize>,
}

impl HttpAdapter for CollectionExpansionHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
        _timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        let url = request.url.as_str();
        if url.contains("/api/collections/by-author/") {
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "authorLogin": "harlan-zw",
                    "slug": "nuxt",
                    "skills": [
                        { "position": 0, "owner": "vuejs", "repo": "core", "name": null, "reason": null },
                        { "position": 1, "owner": "organizer", "repo": "stalled", "name": null, "reason": null },
                    ]
                }))
                .unwrap(),
            ));
        }
        if url.contains("/api/skills") {
            if url.contains("owner=vuejs") {
                return Ok(registry_page(&[("vuejs", "core", "vue", None)]));
            }
            return Ok(registry_page(&[]));
        }
        if url.ends_with("/api/repos") {
            panic!("Listing must not request registry indexing");
        }
        if url.contains("/api/repos/index/") {
            *self.polls.lock().unwrap() += 1;
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "_tag": "queued",
                    "repository": { "_tag": "repository", "owner": "organizer", "repo": "stalled", "url": "https://github.com/organizer/stalled" },
                    "progress": { "_tag": "checking" },
                }))
                .unwrap(),
            ));
        }
        if url.contains("/git/trees/") {
            return Ok(github_tree(&["skills/pinned/SKILL.md"]));
        }
        Ok(github_repository("main", false))
    }
}

#[test]
fn a_collection_expansion_does_not_wait_for_a_stalled_index_job() {
    let http = Arc::new(CollectionExpansionHttp::default());
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));

    let listing = remote
        .list_skills(&MultiSkillRef::Collection {
            login: "harlan-zw".to_owned(),
            slug: "nuxt".to_owned(),
        })
        .unwrap();

    assert_eq!(
        listing.items,
        [
            listed("vuejs", "core", "vue", None),
            direct_listed("organizer", "stalled", "pinned", "skills/pinned"),
        ]
    );
    assert_eq!(
        *http.polls.lock().unwrap(),
        0,
        "a collection entry must not poll an index job"
    );
}

/// Serves one collection: `vuejs/core` is indexed, and GitHub answers the
/// repository read for the unindexed `organizer/gone` with HTTP 500.
struct CollectionFallbackFailureHttp;

impl HttpAdapter for CollectionFallbackFailureHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
        _timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        let url = request.url.as_str();
        if url.contains("/api/collections/by-author/") {
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "authorLogin": "harlan-zw",
                    "slug": "nuxt",
                    "skills": [
                        { "position": 0, "owner": "vuejs", "repo": "core", "name": null, "reason": null },
                        { "position": 1, "owner": "organizer", "repo": "gone", "name": null, "reason": null },
                    ]
                }))
                .unwrap(),
            ));
        }
        if url.contains("/api/skills") {
            if url.contains("owner=vuejs") {
                return Ok(registry_page(&[("vuejs", "core", "vue", None)]));
            }
            return Ok(registry_page(&[]));
        }
        if url.contains("api.github.com/repos/organizer/gone") {
            return Ok(response(500, b"boom".to_vec()));
        }
        Ok(github_repository("main", false))
    }
}

#[test]
fn a_failing_github_read_lists_nothing_for_one_collection_entry() {
    let http = Arc::new(CollectionFallbackFailureHttp);
    let remote = SkilldRemote::new(
        http,
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));

    let listing = remote
        .list_skills(&MultiSkillRef::Collection {
            login: "harlan-zw".to_owned(),
            slug: "nuxt".to_owned(),
        })
        .unwrap();

    assert_eq!(listing.items, [listed("vuejs", "core", "vue", None)]);
}

/// Serves one collection naming `vue` of the unindexed `vuejs/core`. skilld.dev
/// fails every named-Skill Resolution, and GitHub serves the Repository, its
/// tree, and the Skill bytes.
struct LargeRepositoryHttp;

impl HttpAdapter for LargeRepositoryHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
        _timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        let url = request.url.as_str();
        if url.contains("/api/collections/by-author/") {
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "authorLogin": "harlan-zw",
                    "slug": "large",
                    "skills": [
                        { "position": 0, "owner": "vuejs", "repo": "core", "name": "vue", "reason": "The reactive core." },
                    ]
                }))
                .unwrap(),
            ));
        }
        if url.contains("/api/skills") {
            return Ok(registry_page(&[]));
        }
        if url.contains("/api/v1/resolutions") {
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "state": "failed",
                    "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
                    "code": "INVALID_SOURCE",
                    "retryable": false,
                }))
                .unwrap(),
            ));
        }
        if url.contains("/repos/vuejs/core/commits/") {
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "sha": "b".repeat(40),
                    "commit": { "tree": { "sha": "c".repeat(40) } },
                }))
                .unwrap(),
            ));
        }
        if url.contains("/git/trees/") {
            let skill = b"---\nname: vue\ndescription: The reactive core.\n---\n";
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "truncated": false,
                    "tree": [{
                        "path": "skills/vue/SKILL.md",
                        "mode": "100644",
                        "type": "blob",
                        "sha": "a".repeat(40),
                        "size": skill.len(),
                    }],
                }))
                .unwrap(),
            ));
        }
        if url.contains("/git/blobs/") {
            let skill = b"---\nname: vue\ndescription: The reactive core.\n---\n";
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "content": STANDARD.encode(skill),
                    "encoding": "base64",
                    "size": skill.len(),
                }))
                .unwrap(),
            ));
        }
        Ok(github_repository("main", false))
    }
}

#[test]
fn a_named_collection_entry_reports_delivery_failure_without_installing_directly() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host =
        LocalHost::new(project, temporary.path().join("data")).with_remote_provider(Arc::new(
            SkilldRemote::new(
                Arc::new(LargeRepositoryHttp),
                Arc::new(NoTokenProvider),
                NativeRemoteConfig::Unconfigured,
            )
            .with_endpoint("http://127.0.0.1:8787")
            .unwrap()
            .with_sleeper(Arc::new(NoSleep)),
        ));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        [
            "skilld",
            "add",
            "@harlan-zw/large",
            "--all",
            "--agent",
            "codex",
        ],
        &host,
        &mut stdout,
        &mut stderr,
    );

    let output = String::from_utf8(stdout).unwrap();
    assert_eq!(result.exit_code, 1, "{output}");
    assert!(
        output.contains("skilld install github:vuejs/core/skills/vue --direct --agent codex"),
        "{output}"
    );
    assert!(host.list(InstallScope::Project).unwrap().is_empty());
}

#[test]
fn a_curator_ref_lists_every_collection_once() {
    let collection = |name: &str| {
        response(
            200,
            serde_json::to_vec(&json!({
                "skills": [
                    { "position": 0, "owner": "vuejs", "repo": "core", "name": name, "reason": null },
                    { "position": 1, "owner": "vuejs", "repo": "core", "name": "shared", "reason": null },
                ]
            }))
            .unwrap(),
        )
    };
    let http = Arc::new(FakeHttp::with([
        response(
            200,
            serde_json::to_vec(&json!({
                "login": "harlan-zw",
                "collections": [
                    { "slug": "vue", "name": "Vue", "itemCount": 2 },
                    { "slug": "nuxt", "name": "Nuxt", "itemCount": 2 },
                ]
            }))
            .unwrap(),
        ),
        collection("vue"),
        collection("nuxt"),
        // The named entries resolve through the rows of their Repository once.
        // No row carries a path, so every named Skill keeps its selector.
        registry_page(&[
            ("vuejs", "core", "vue", None),
            ("vuejs", "core", "shared", None),
        ]),
    ]));
    let remote = search_remote(http.clone());

    let listing = remote
        .list_skills(&MultiSkillRef::Curator {
            login: "harlan-zw".to_owned(),
        })
        .unwrap();

    assert_eq!(
        listing.items,
        [
            listed("vuejs", "core", "vue", None),
            listed("vuejs", "core", "shared", None),
            listed("vuejs", "core", "nuxt", None),
        ]
    );
    assert_eq!(
        request_paths(&http),
        [
            "/api/curators/harlan-zw",
            "/api/collections/by-author/harlan-zw/vue",
            "/api/collections/by-author/harlan-zw/nuxt",
            "/api/skills?owner=vuejs&limit=200",
        ]
    );
}

/// Serves a curator payload with 100 collections, answers every collection
/// detail with one name-less Repository entry, and serves a runaway owner
/// index to every owner, so only client side caps and memoization can end
/// the listing.
#[derive(Default)]
struct RunawayCuratorHttp {
    requests: Mutex<Vec<String>>,
}

const RUNAWAY_CURATOR_COLLECTIONS: usize = 100;
const RUNAWAY_CURATOR_PAGE_CAP: usize = 25;

/// One curator payload, the capped collection details, and one memoized
/// owner index fetch for the repeated entry spanning the capped pages.
const RUNAWAY_CURATOR_REQUEST_LIMIT: usize = 1 + 2 * RUNAWAY_CURATOR_PAGE_CAP;

impl HttpAdapter for RunawayCuratorHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
        _timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.url.clone());
        assert!(
            requests.len() <= RUNAWAY_CURATOR_REQUEST_LIMIT,
            "the curator listing kept issuing requests: {} requests issued",
            requests.len()
        );
        if request.url.contains("/api/curators/") {
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "login": "curator",
                    "collections": (0..RUNAWAY_CURATOR_COLLECTIONS)
                        .map(|index| {
                            json!({
                                "slug": format!("collection-{index}"),
                                "name": format!("Collection {index}"),
                                "itemCount": 1,
                            })
                        })
                        .collect::<Vec<_>>(),
                }))
                .unwrap(),
            ));
        }
        if request.url.contains("/api/collections/by-author/") {
            return Ok(response(
                200,
                serde_json::to_vec(&json!({
                    "skills": [{
                        "position": 0,
                        "owner": "big",
                        "repo": "wanted",
                        "name": null,
                        "reason": null,
                    }]
                }))
                .unwrap(),
            ));
        }
        let page = json!({
            "items": (0..RUNAWAY_PAGE_ROWS)
                .map(|_| json!({
                    "name": "wanted",
                    "owner": "big",
                    "repo": "wanted",
                    "description": null,
                    "stars": 1,
                    "registryPath": "/gh/big/wanted/wanted",
                }))
                .collect::<Vec<_>>(),
            "total": 20_000_000,
            "pages": u64::MAX,
        });
        Ok(response(200, serde_json::to_vec(&page).unwrap()))
    }
}

#[test]
fn a_runaway_curator_stops_fanning_out_and_fetches_one_repository_once() {
    let http = Arc::new(RunawayCuratorHttp::default());
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));

    let listing = remote
        .list_skills(&MultiSkillRef::Curator {
            login: "curator".to_owned(),
        })
        .unwrap();

    assert_eq!(listing.items, [listed("big", "wanted", "wanted", None)]);
    assert_eq!(
        http.requests.lock().unwrap().len(),
        RUNAWAY_CURATOR_REQUEST_LIMIT
    );
}

#[test]
fn a_missing_collection_is_a_source_not_found_error() {
    let http = Arc::new(FakeHttp::with([response(
        404,
        br#"{"error":true,"statusCode":404,"message":"Collection not found"}"#.to_vec(),
    )]));
    let remote = search_remote(http);

    let error = remote
        .list_skills(&MultiSkillRef::Collection {
            login: "harlan-zw".to_owned(),
            slug: "missing".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "SOURCE_NOT_FOUND");
    assert_eq!(
        error.message,
        "skilld.dev has no collection @harlan-zw/missing"
    );
}

#[test]
fn a_rate_limited_resolution_says_how_long_to_wait() {
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&json!({
            "state": "failed",
            "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
            "code": "RATE_LIMITED",
            "retryable": true,
            "retryAfterSeconds": 1_500,
        }))
        .unwrap(),
    )]));
    let remote = search_remote(http);

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "RATE_LIMITED");
    assert_eq!(error.message, "the Resolution failed. Retry in 25 minutes.");
}

fn failed_resolution(code: &str, retryable: bool, retry_after: Option<u64>) -> HttpResponse {
    let mut body = json!({
        "state": "failed",
        "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
        "code": code,
        "retryable": retryable,
    });
    if let Some(seconds) = retry_after {
        body["retryAfterSeconds"] = json!(seconds);
    }
    response(200, serde_json::to_vec(&body).unwrap())
}

fn idempotency_keys(http: &FakeHttp) -> Vec<String> {
    http.requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| {
            request
                .headers
                .iter()
                .find(|header| header.name == "idempotency-key")
                .map(|header| header.value.expose().to_owned())
                .unwrap_or_default()
        })
        .collect()
}

fn resolution_remote(http: Arc<FakeHttp>, sleeper: Arc<RecordingSleeper>) -> SkilldRemote {
    SkilldRemote::new(
        http,
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(sleeper)
}

#[test]
fn a_retryable_resolution_says_it_may_be_retried_after_three_retries() {
    let http = Arc::new(FakeHttp::with([
        failed_resolution("SERVICE_UNAVAILABLE", true, None),
        failed_resolution("SERVICE_UNAVAILABLE", true, None),
        failed_resolution("SERVICE_UNAVAILABLE", true, None),
        failed_resolution("SERVICE_UNAVAILABLE", true, None),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let remote = resolution_remote(http.clone(), sleeper.clone());

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(
        error.message,
        "skilld.dev could not create the Artifact this time"
    );
    assert_eq!(
        error.next_step.as_deref(),
        Some("skilld requested the Skill 4 times. Run the same command again in a minute.")
    );
    let keys = idempotency_keys(&http);
    assert_eq!(keys.len(), 4);
    assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 4);
    let waited = *sleeper.elapsed.lock().unwrap();
    assert!(waited >= Duration::from_secs(7), "{waited:?}");
    assert!(waited < Duration::from_secs(11), "{waited:?}");
}

#[test]
fn a_rate_limited_resolution_is_requested_again_with_a_new_key_after_a_backoff() {
    let http = Arc::new(FakeHttp::with([
        failed_resolution("RATE_LIMITED", true, None),
        failed_resolution("INVALID_SOURCE", false, None),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let remote = resolution_remote(http.clone(), sleeper.clone());

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "INVALID_SOURCE");
    let keys = idempotency_keys(&http);
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
    let waited = *sleeper.elapsed.lock().unwrap();
    assert!(waited >= Duration::from_secs(1), "{waited:?}");
    assert!(waited < Duration::from_millis(1_500), "{waited:?}");
}

#[test]
fn a_short_retry_after_is_honored_before_the_next_resolution() {
    let http = Arc::new(FakeHttp::with([
        failed_resolution("RATE_LIMITED", true, Some(5)),
        failed_resolution("INVALID_SOURCE", false, None),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let remote = resolution_remote(http.clone(), sleeper.clone());

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "INVALID_SOURCE");
    let waited = *sleeper.elapsed.lock().unwrap();
    assert!(waited >= Duration::from_secs(5), "{waited:?}");
    assert!(waited < Duration::from_millis(5_250), "{waited:?}");
}

#[test]
fn a_failure_that_is_not_retryable_is_not_requested_again() {
    let http = Arc::new(FakeHttp::with([failed_resolution(
        "INVALID_SOURCE",
        false,
        None,
    )]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let remote = resolution_remote(http.clone(), sleeper.clone());

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "INVALID_SOURCE");
    assert_eq!(http.requests.lock().unwrap().len(), 1);
    assert_eq!(*sleeper.elapsed.lock().unwrap(), Duration::ZERO);
}

fn skilld_selector() -> RemoteSelector {
    RemoteSelector::parse("skilld-dev/skilld/skilld").unwrap()
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
fn a_blocked_resolution_names_the_check_that_failed() {
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&json!({
            "state": "blocked",
            "resolutionId": "0f9a4a44-27f9-4f6a-9a21-4d24d8ff2f60",
            "checkResults": [
                {
                    "name": "agent-skills-spec",
                    "version": "1",
                    "outcome": "pass",
                    "required": true,
                },
                {
                    "name": "path-policy",
                    "version": "1",
                    "outcome": "fail",
                    "required": true,
                    "summary": "The Skill has more than 256 files.",
                },
            ],
        }))
        .unwrap(),
    )]));
    let remote = search_remote(http);

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "CHECK_BLOCKED");
    assert_eq!(
        error.message,
        "the Resolution was blocked by check results. path-policy: The Skill has more than 256 files."
    );
}

#[test]
fn a_hosted_resolution_reports_each_service_stage() {
    let resolution_id = "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f";
    let stages = [
        "requested",
        "resolving",
        "fetching",
        "checking",
        "packaging",
        "encrypting",
        "signing",
        "publishing",
        "retry-wait",
    ];
    let mut responses = stages
        .iter()
        .map(|stage| {
            response(
                200,
                serde_json::to_vec(&json!({
                    "state": "pending",
                    "resolutionId": resolution_id,
                    "stage": stage,
                    "pollAfterMs": 250
                }))
                .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    responses.push(response(
        200,
        serde_json::to_vec(&json!({
            "state": "blocked",
            "resolutionId": resolution_id,
            "checkResults": []
        }))
        .unwrap(),
    ));
    let progress = Arc::new(RecordingProgress::default());
    let remote = search_remote(Arc::new(FakeHttp::with(responses))).with_progress(progress.clone());

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "CHECK_BLOCKED");
    assert_eq!(
        *progress.0.lock().unwrap(),
        [
            RemoteProgressStage::RequestingResolution,
            RemoteProgressStage::Requested,
            RemoteProgressStage::Resolving,
            RemoteProgressStage::Fetching,
            RemoteProgressStage::Checking,
            RemoteProgressStage::Packaging,
            RemoteProgressStage::Encrypting,
            RemoteProgressStage::Signing,
            RemoteProgressStage::Publishing,
            RemoteProgressStage::RetryWait,
        ]
    );
}

#[test]
fn an_unknown_pending_stage_does_not_abort_the_resolution() {
    let (pin, mut responses) = verified_remote_responses();
    let pending = response(
        200,
        serde_json::to_vec(&json!({
            "state": "pending",
            "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
            "stage": "queueing",
            "pollAfterMs": 250
        }))
        .unwrap(),
    );
    responses.insert(0, pending);
    let progress = Arc::new(RecordingProgress::default());
    let remote = SkilldRemote::new(
        Arc::new(FakeHttp::with(responses)),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep))
    .with_progress(progress.clone());
    let selector = RemoteSelector::parse("skilld-dev/skills/example").unwrap();

    let prepared = remote.prepare(&selector, false).unwrap();

    assert!(matches!(
        prepared.source_status,
        SourceStatus::Verified { .. }
    ));
    assert_eq!(
        *progress.0.lock().unwrap(),
        [
            RemoteProgressStage::RequestingResolution,
            RemoteProgressStage::VerifyingAttestation,
            RemoteProgressStage::RequestingDownload,
            RemoteProgressStage::DownloadingArtifact,
            RemoteProgressStage::VerifyingArtifact,
        ]
    );
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
        "skilld.dev did not finish the Artifact within 60 seconds"
    );
    assert_eq!(
        error.next_step.as_deref(),
        Some(
            "The Skill did not cause this failure. skilld requested the Skill once. Run the same command once more."
        )
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
    let selector = RemoteSelector::parse("skilld-dev/skills/example").unwrap();

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
    assert!(requests[2].headers.iter().any(|header| {
        header.name == "x-skilld-resolution-id"
            && header.value.expose() == "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f"
    }));
    assert!(requests[3].url.ends_with("/content"));
}

fn linking_remote(
    responses: Vec<HttpResponse>,
    pin: TrustedRootPin,
) -> (Arc<FakeHttp>, SkilldRemote) {
    let http = Arc::new(FakeHttp::with(responses));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));
    (http, remote)
}

#[test]
fn a_resolution_request_says_this_cli_reads_linked_files() {
    let (pin, responses) = verified_remote_responses();
    let (http, remote) = linking_remote(responses, pin);

    remote
        .prepare(
            &RemoteSelector::parse("skilld-dev/skills/example").unwrap(),
            false,
        )
        .unwrap();

    let requests = http.requests.lock().unwrap();
    assert!(requests[0].headers.iter().any(|header| {
        header.name == "skilld-capabilities" && header.value.expose() == "linked-files"
    }));
}

#[test]
fn a_linked_file_is_read_from_github_at_the_attested_commit_and_installed() {
    let tool = b"\x7fELF a binary too large to pack".as_slice();
    let (pin, responses) = verified_remote_responses_linking(
        b"---\nname: example\ndescription: verified\n---\n",
        passing_checks(),
        &[("scripts/my tool#1", tool)],
    );
    let (http, remote) = linking_remote(responses, pin);

    let prepared = remote
        .prepare(
            &RemoteSelector::parse("skilld-dev/skills/example").unwrap(),
            false,
        )
        .unwrap();

    let installed = prepared
        .files
        .iter()
        .find(|file| file.path == "scripts/my tool#1")
        .unwrap();
    assert_eq!(installed.bytes, tool);
    assert_eq!(installed.mode, 0o755);
    let requests = http.requests.lock().unwrap();
    assert_eq!(
        requests[4].url,
        "https://raw.githubusercontent.com/skilld-dev/skills/0123456789abcdef0123456789abcdef01234567/skills/example/scripts/my%20tool%231"
    );
}

#[test]
fn a_linked_file_with_other_bytes_installs_nothing() {
    let (pin, mut responses) = verified_remote_responses_linking(
        b"---\nname: example\ndescription: verified\n---\n",
        passing_checks(),
        &[("assets/track.mp3", b"the attested bytes")],
    );
    *responses.last_mut().unwrap() = response(200, b"the attested bytez".to_vec());
    let (_, remote) = linking_remote(responses, pin);

    let error = remote
        .prepare(
            &RemoteSelector::parse("skilld-dev/skills/example").unwrap(),
            false,
        )
        .unwrap_err();

    assert_eq!(error.code, "LINKED_FILE_DIGEST_MISMATCH");
}

fn prepared_with_page_header(header: Option<&str>) -> Option<String> {
    let (pin, mut responses) = verified_remote_responses();
    if let Some(value) = header {
        responses[0]
            .headers
            .insert("skilld-page-url".to_owned(), value.to_owned());
    }
    let remote = SkilldRemote::new(
        Arc::new(FakeHttp::with(responses)),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));
    let selector = RemoteSelector::parse("skilld-dev/skills/example").unwrap();
    remote.prepare(&selector, false).unwrap().page_url
}

#[test]
fn a_delivery_carries_the_page_url_the_server_names() {
    assert_eq!(
        prepared_with_page_header(Some("http://127.0.0.1:8787/gh/skilld-dev/skills")).as_deref(),
        Some("http://127.0.0.1:8787/gh/skilld-dev/skills")
    );
}

#[test]
fn a_delivery_from_an_older_server_carries_no_page_url() {
    assert_eq!(prepared_with_page_header(None), None);
}

#[test]
fn a_page_url_for_another_host_or_route_is_dropped() {
    for header in [
        "https://evil.example/gh/skilld-dev/skills",
        "http://127.0.0.1:8787/people/someone",
        "http://127.0.0.1:8787/gh/skilld-dev/skills?next=x",
        "http://127.0.0.1:8787/gh/skilld-dev/skills#top",
        "not a url",
    ] {
        assert_eq!(prepared_with_page_header(Some(header)), None, "{header}");
    }
}

fn searched_page_url(page_url: &str) -> Option<String> {
    let body = json!({
        "items": [{
            "name": "vue-testing",
            "description": null,
            "source": {
                "provider": "github",
                "owner": "skilld-dev",
                "repository": "skills",
                "selector": { "type": "named-skill", "name": "vue-testing" }
            },
            "stargazerCount": 120,
            "pageUrl": page_url,
        }],
        "total": 1,
    });
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&body).unwrap(),
    )]));
    search_remote(http)
        .search("vue testing", 20)
        .unwrap()
        .items
        .remove(0)
        .page_url
}

#[test]
fn a_search_result_carries_the_page_url_the_server_names() {
    assert_eq!(
        searched_page_url("http://127.0.0.1:8787/gh/skilld-dev/skills/vue-testing").as_deref(),
        Some("http://127.0.0.1:8787/gh/skilld-dev/skills/vue-testing")
    );
}

#[test]
fn a_search_page_url_for_another_host_or_with_terminal_controls_is_dropped() {
    for page_url in [
        "https://evil.example/gh/skilld-dev/skills/vue-testing",
        "http://127.0.0.1:8787/gh/skilld-dev/skills/vue-testing?next=x",
        "http://127.0.0.1:8787/gh/skilld-dev/skills/vue-testing\u{1b}]8;;https://evil.example\u{1b}\\",
    ] {
        assert_eq!(searched_page_url(page_url), None, "{page_url:?}");
    }
}

#[test]
fn a_public_grant_may_serve_content_from_a_service_subdomain() {
    let (pin, mut responses) = verified_remote_responses();
    let mut grant: serde_json::Value = serde_json::from_slice(&responses[2].body).unwrap();
    grant["contentUrl"] = json!("https://artifacts.skilld.dev/sha256/example");
    responses[2] = response(200, serde_json::to_vec(&grant).unwrap());
    let http = Arc::new(FakeHttp::with(responses));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("https://skilld.dev")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));
    let selector = RemoteSelector::parse("skilld-dev/skills/example").unwrap();

    let prepared = remote.prepare(&selector, false).unwrap();

    assert!(matches!(
        prepared.source_status,
        SourceStatus::Verified { .. }
    ));
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests[3].url,
        "https://artifacts.skilld.dev/sha256/example"
    );
}

#[test]
fn artifact_download_errors_preserve_problem_details_and_http_fallbacks() {
    for (body, code, message) in [
        (
            serde_json::to_vec(&json!({
                "type": "about:blank",
                "title": "Artifact revoked",
                "status": 410,
                "code": "ARTIFACT_REVOKED",
                "detail": "The artifact was revoked."
            }))
            .unwrap(),
            "ARTIFACT_REVOKED",
            "The artifact was revoked.",
        ),
        (
            b"Gone".to_vec(),
            "SERVICE_UNAVAILABLE",
            "the remote service returned HTTP 410",
        ),
    ] {
        let (pin, mut responses) = verified_remote_responses();
        let mut grant: serde_json::Value = serde_json::from_slice(&responses[2].body).unwrap();
        grant["contentUrl"] = json!("https://artifacts.skilld.dev/sha256/example");
        responses[2] = response(200, serde_json::to_vec(&grant).unwrap());
        responses[3] = response(410, body);
        let remote = SkilldRemote::new(
            Arc::new(FakeHttp::with(responses)),
            Arc::new(NoTokenProvider),
            NativeRemoteConfig::Pinned(pin),
        )
        .with_endpoint("https://skilld.dev")
        .unwrap()
        .with_sleeper(Arc::new(NoSleep));

        let selector = RemoteSelector::parse("skilld-dev/skills/example").unwrap();
        let error = remote.prepare(&selector, false).unwrap_err();

        assert_eq!(error.code, code);
        assert_eq!(error.message, message);
    }
}

#[test]
fn a_public_grant_on_an_unrelated_origin_is_rejected_before_download() {
    let (pin, mut responses) = verified_remote_responses();
    let mut grant: serde_json::Value = serde_json::from_slice(&responses[2].body).unwrap();
    grant["contentUrl"] = json!("https://example.com/sha256/example");
    responses[2] = response(200, serde_json::to_vec(&grant).unwrap());
    let http = Arc::new(FakeHttp::with(responses));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("https://skilld.dev")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));
    let selector = RemoteSelector::parse("skilld-dev/skills/example").unwrap();

    let error = remote.prepare(&selector, false).unwrap_err();

    assert_eq!(error.code, "REMOTE_ORIGIN_REJECTED");
    assert_eq!(http.requests.lock().unwrap().len(), 3);
}

#[test]
fn a_private_grant_refuses_public_access_without_an_account() {
    let (pin, mut responses) = verified_remote_responses();
    let grant: serde_json::Value = serde_json::from_slice(&responses[2].body).unwrap();
    responses[2] = response(
        200,
        serde_json::to_vec(&json!({
            "kind": "private",
            "artifactId": grant["artifactId"],
            "contentUrl": grant["contentUrl"],
            "expiresAt": grant["expiresAt"],
            "downloadToken": "private-grant-token-with-enough-bytes",
            "attestation": grant["attestation"]
        }))
        .unwrap(),
    );
    responses.truncate(3);
    let remote = SkilldRemote::new(
        Arc::new(FakeHttp::with(responses)),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("https://skilld.dev")
    .unwrap()
    .with_sleeper(Arc::new(NoSleep));

    let error = remote
        .prepare(
            &RemoteSelector::parse("skilld-dev/skills/example").unwrap(),
            false,
        )
        .unwrap_err();

    assert_eq!(error.code, "AUTH_REQUIRED");
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
            &RemoteSelector::parse("skilld-dev/skills/example").unwrap(),
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
fn a_second_direct_skill_reuses_the_repository_snapshot() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let encoded = base64::engine::general_purpose::STANDARD.encode(skill);
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let tree = "89abcdef0123456789abcdef0123456789abcdef";
    let blob = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let blob_response = || {
        response(
            200,
            format!(
                r#"{{"content":"{encoded}","encoding":"base64","size":{}}}"#,
                skill.len()
            ),
        )
    };
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
                r#"{{"truncated":false,"tree":[{{"path":"skills/one/SKILL.md","mode":"100644","type":"blob","sha":"{blob}","size":{size}}},{{"path":"skills/two/SKILL.md","mode":"100644","type":"blob","sha":"{blob}","size":{size}}}]}}"#,
                size = skill.len()
            ),
        ),
        // The tarball is tried first. This fixture predates it and carries
        // placeholder blob SHAs, so the archive is absent and the blob path
        // answers, which is exactly the fallback this Repository needs.
        response(404, br#"{"message":"Not Found"}"#.to_vec()),
        blob_response(),
        blob_response(),
    ]));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_sleeper(Arc::new(NoSleep));

    for name in ["one", "two"] {
        let selector =
            RemoteSelector::parse(&format!("github:skilld-dev/skills/skills/{name}")).unwrap();
        remote.prepare(&selector, true).unwrap();
    }

    // Two Skills, one Repository read, one commit read, one tree read, one
    // tarball attempt for the Repository, one blob each. GitHub rate limits an
    // unauthenticated caller at 60 an hour.
    assert_eq!(http.requests.lock().unwrap().len(), 6);
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
        // This fixture carries a placeholder blob SHA, so the tarball cannot
        // serve it and the blob path answers.
        response(404, br#"{"message":"Not Found"}"#.to_vec()),
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
fn a_direct_github_rate_limit_names_github_and_the_reset_wait() {
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let mut limited = response(403, br#"{"message":"API rate limit exceeded"}"#.to_vec());
    limited
        .headers
        .insert("x-ratelimit-remaining".to_owned(), "0".to_owned());
    limited
        .headers
        .insert("x-ratelimit-reset".to_owned(), reset.to_string());
    let http = Arc::new(FakeHttp::with([limited]));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_sleeper(Arc::new(NoSleep));
    let selector = RemoteSelector::parse("github:skilld-dev/skills/skills/example").unwrap();

    let error = remote.prepare(&selector, true).unwrap_err();

    assert_eq!(error.code, "RATE_LIMITED");
    assert!(
        error
            .message
            .starts_with("GitHub rate limited this request. Retry after ")
    );
    assert_eq!(http.requests.lock().unwrap().len(), 1);
}

#[test]
fn prepare_exact_rejects_a_conflicting_selector_commit_before_http() {
    let selector_commit = "0123456789abcdef0123456789abcdef01234567";
    let expected_commit = CommitSha::parse("ffffffffffffffffffffffffffffffffffffffff").unwrap();
    let http = Arc::new(FakeHttp::default());
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    );
    let selector = RemoteSelector::parse(&format!(
        "github:skilld-dev/skills/skills/example#commit:{selector_commit}"
    ))
    .unwrap();

    let error = remote
        .prepare_exact(&selector, &expected_commit, true)
        .unwrap_err();

    assert_eq!(error.code, "SOURCE_MISMATCH");
    assert!(http.requests.lock().unwrap().is_empty());
}

#[test]
fn prepare_exact_keeps_a_matching_selector_commit_as_provenance() {
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let expected_commit = CommitSha::parse(commit).unwrap();
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
    let selector = RemoteSelector::parse(&format!(
        "github:skilld-dev/skills/skills/example#commit:{commit}"
    ))
    .unwrap();

    let prepared = remote
        .prepare_exact(&selector, &expected_commit, false)
        .unwrap();

    assert!(matches!(
        prepared.locked_source,
        LockedSource::Remote {
            ref source,
            ref commit_sha,
            ..
        } if source == &selector.canonical() && commit_sha == commit
    ));
    assert_eq!(http.requests.lock().unwrap().len(), 4);
}

#[test]
fn direct_github_access_rejects_a_commit_response_that_changed_the_requested_commit() {
    let requested = "0123456789abcdef0123456789abcdef01234567";
    let returned = "ffffffffffffffffffffffffffffffffffffffff";
    let tree = "89abcdef0123456789abcdef0123456789abcdef";
    let http = Arc::new(FakeHttp::with([
        response(
            200,
            br#"{"private":false,"default_branch":"main"}"#.to_vec(),
        ),
        response(
            200,
            format!(r#"{{"sha":"{returned}","commit":{{"tree":{{"sha":"{tree}"}}}}}}"#),
        ),
    ]));
    let remote = SkilldRemote::new(
        http.clone(),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_sleeper(Arc::new(NoSleep));
    let selector = RemoteSelector::parse(&format!(
        "github:skilld-dev/skills/skills/example#commit:{requested}"
    ))
    .unwrap();

    let error = remote.prepare(&selector, true).unwrap_err();

    assert_eq!(error.code, "SOURCE_MISMATCH");
    assert_eq!(http.requests.lock().unwrap().len(), 2);
}

#[test]
fn direct_install_error_gives_an_agent_an_exact_recovery() {
    let remote = SkilldRemote::new(
        Arc::new(FakeHttp::default()),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    );
    let selector = RemoteSelector::parse("skilld-dev/skills/example").unwrap();

    let error = remote.prepare(&selector, true).unwrap_err();

    assert_eq!(error.code, "DIRECT_SOURCE_REQUIRED");
    assert_eq!(
        error.message,
        "--direct requires a github:OWNER/REPOSITORY/SKILL_PATH source or a GitHub tree URL. Remove --direct, then run the same command again."
    );
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
            // The server names a page only for a delivered Skill in its registry.
            page_url: (!direct
                && matches!(
                    &selector.source().selector,
                    skilld_core::SourceSelector::NamedSkill { name } if name == "example"
                ))
            .then(|| "https://skilld.dev/gh/skilld-dev/skills/example".to_owned()),
            omitted_files: Vec::new(),
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
    latest_commit: Mutex<char>,
    prepared_names: Mutex<Vec<String>>,
    fail_name: Mutex<Option<&'static str>>,
    relation: Mutex<RemoteComparisonRelation>,
    /// Instructions after the frontmatter of every prepared SKILL.md.
    body: Mutex<&'static str>,
}

impl BatchProvider {
    fn prepared(&self, selector: &RemoteSelector) -> PreparedRemoteSkill {
        let skilld_core::SourceSelector::NamedSkill { name } = &selector.source().selector else {
            panic!("expected a named Skill selector")
        };
        self.prepared_names.lock().unwrap().push(name.clone());
        let bytes = format!(
            "---\nname: {name}\ndescription: {}\n---\n{}",
            *self.version.lock().unwrap(),
            *self.body.lock().unwrap()
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
            page_url: None,
            omitted_files: Vec::new(),
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
            commit_sha: CommitSha::parse(self.latest_commit.lock().unwrap().to_string().repeat(40))
                .unwrap(),
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
        latest_commit: Mutex::new('f'),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
        body: Mutex::new(""),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    for name in ["alpha", "beta"] {
        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Remote(format!(
                "skilld-dev/skills/{name}"
            ))),
            scope: InstallScope::Project,
            targets: vec![AgentTargetId::Codex],
            mode: Some(InstallMode::Copy),
            allowed_behaviors: Vec::new(),
        })
        .unwrap();
    }
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";

    let lines = host.update(None, InstallScope::Project, &[]).unwrap();

    assert_eq!(
        lines
            .iter()
            .map(skilld_ui::Line::plain_text)
            .collect::<Vec<_>>(),
        ["Updated Skill alpha.", "Updated Skill beta."]
    );
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
        latest_commit: Mutex::new('f'),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
        body: Mutex::new(""),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    for name in ["alpha", "beta"] {
        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Remote(format!(
                "skilld-dev/skills/{name}"
            ))),
            scope: InstallScope::Project,
            targets: vec![AgentTargetId::Codex],
            mode: Some(InstallMode::Copy),
            allowed_behaviors: Vec::new(),
        })
        .unwrap();
    }
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";
    *provider.fail_name.lock().unwrap() = Some("beta");

    let error = host.update(None, InstallScope::Project, &[]).unwrap_err();

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
        latest_commit: Mutex::new('f'),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
        body: Mutex::new(""),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(
            "skilld-dev/skills/alpha".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
        allowed_behaviors: Vec::new(),
    })
    .unwrap();
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";
    *provider.relation.lock().unwrap() = RemoteComparisonRelation::Behind;

    let error = host.update(None, InstallScope::Project, &[]).unwrap_err();

    assert_eq!(error.code, "UPDATE_CONFIRMATION_REQUIRED");
    assert!(provider.prepared_names.lock().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(project.join(".skills/alpha/SKILL.md")).unwrap(),
        "---\nname: alpha\ndescription: first\n---\n"
    );
}

fn reviewed_updates(host: &LocalHost, names: &[&str]) -> Vec<UpdatePlanItem> {
    let plan = host.update_check(None).unwrap();
    names
        .iter()
        .map(|name| {
            plan.items()
                .iter()
                .find(|item| item.name().as_str() == *name)
                .unwrap()
                .clone()
        })
        .collect()
}

#[test]
fn selected_skill_update_commits_only_the_exact_subset() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        latest_commit: Mutex::new('f'),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
        body: Mutex::new(""),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    for name in ["alpha", "beta", "gamma"] {
        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Remote(format!(
                "skilld-dev/skills/{name}"
            ))),
            scope: InstallScope::Project,
            targets: vec![AgentTargetId::Codex],
            mode: Some(InstallMode::Copy),
            allowed_behaviors: Vec::new(),
        })
        .unwrap();
    }
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";
    let reviewed = reviewed_updates(&host, &["gamma", "alpha"]);

    let lines = host.update_selected(&reviewed).unwrap();

    assert_eq!(
        lines
            .iter()
            .map(skilld_ui::Line::plain_text)
            .collect::<Vec<_>>(),
        ["Updated Skill gamma.", "Updated Skill alpha."]
    );
    assert_eq!(*provider.prepared_names.lock().unwrap(), ["gamma", "alpha"]);
    for (name, version) in [("alpha", "second"), ("beta", "first"), ("gamma", "second")] {
        assert_eq!(
            fs::read_to_string(project.join(format!(".skills/{name}/SKILL.md"))).unwrap(),
            format!("---\nname: {name}\ndescription: {version}\n---\n")
        );
    }
}

#[test]
fn selected_skill_update_rejects_empty_duplicate_and_unavailable_items() {
    let temporary = tempfile::tempdir().unwrap();
    let host = LocalHost::new(
        temporary.path().join("project"),
        temporary.path().join("data"),
    );

    let current = UpdatePlanItem::new(
        skilld_core::SkillName::parse("alpha").unwrap(),
        UpdateRelation::Current {
            commit_sha: CommitSha::parse("1".repeat(40)).unwrap(),
        },
    );
    let empty = host.update_selected(&[]).unwrap_err();
    let duplicate = host
        .update_selected(&[current.clone(), current.clone()])
        .unwrap_err();
    let invalid_relation = host.update_selected(&[current]).unwrap_err();

    assert_eq!(empty.code, "INVALID_SELECTION");
    assert_eq!(empty.message, "Select at least one Skill");
    assert_eq!(duplicate.code, "INVALID_SELECTION");
    assert_eq!(duplicate.message, "Select each Skill once");
    assert_eq!(invalid_relation.code, "INVALID_SELECTION");
    assert_eq!(
        invalid_relation.message,
        "Select only Skills with available updates"
    );
}

#[test]
fn selected_skill_update_changes_nothing_when_one_selected_artifact_fails() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        latest_commit: Mutex::new('f'),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
        body: Mutex::new(""),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    for name in ["alpha", "beta", "gamma"] {
        host.install_request(InstallRequest {
            operation: InstallOperation::Install(InstallSource::Remote(format!(
                "skilld-dev/skills/{name}"
            ))),
            scope: InstallScope::Project,
            targets: vec![AgentTargetId::Codex],
            mode: Some(InstallMode::Copy),
            allowed_behaviors: Vec::new(),
        })
        .unwrap();
    }
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";
    let reviewed = reviewed_updates(&host, &["alpha", "gamma"]);
    *provider.fail_name.lock().unwrap() = Some("gamma");

    let error = host.update_selected(&reviewed).unwrap_err();

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
fn selected_skill_update_rejects_a_head_that_changed_after_review() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        latest_commit: Mutex::new('f'),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
        body: Mutex::new(""),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(
            "skilld-dev/skills/alpha".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
        allowed_behaviors: Vec::new(),
    })
    .unwrap();
    provider.prepared_names.lock().unwrap().clear();
    *provider.version.lock().unwrap() = "second";
    let reviewed = reviewed_updates(&host, &["alpha"]);
    *provider.latest_commit.lock().unwrap() = 'e';

    let error = host.update_selected(&reviewed).unwrap_err();

    assert_eq!(error.code, "STALE_UPDATE_PLAN");
    assert!(provider.prepared_names.lock().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(project.join(".skills/alpha/SKILL.md")).unwrap(),
        "---\nname: alpha\ndescription: first\n---\n"
    );
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
            "skilld-dev/skills/example".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
        allowed_behaviors: Vec::new(),
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
            "skilld-dev/skills/example".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
        allowed_behaviors: Vec::new(),
    };

    let installed = host.install_request(request).unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].name, "example");
    assert_eq!(installed[0].source_status, "verified");
    assert_eq!(
        host.verify(Some("example"))
            .unwrap()
            .iter()
            .map(skilld_ui::Line::plain_text)
            .collect::<Vec<_>>(),
        ["Verified the source of Skill example."]
    );
    let before = fs::read(project.join(".skills/example/SKILL.md")).unwrap();
    *provider.content.lock().unwrap() = b"---\nname: example\ndescription: second\n---\n".to_vec();
    *provider.stale.lock().unwrap() = true;
    *provider.fail_prepare.lock().unwrap() = true;

    let error = host
        .update(Some("example"), InstallScope::Project, &[])
        .unwrap_err();

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
            "skilld-dev/skills/example".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
        allowed_behaviors: Vec::new(),
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
fn cli_install_shows_the_author_the_source_status_and_the_exact_skill_file() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider("---\nname: example\n---\n"));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        [
            "skilld",
            "install",
            "skilld-dev/skills/example",
            "--agent",
            "codex",
        ],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    let output = String::from_utf8(stdout).unwrap();
    assert!(
        output.contains(&format!(
            "Files: {}",
            project.join(".agents/skills/example").display()
        )),
        "{output}"
    );
    assert!(output.contains("Source status: verified"), "{output}");
    assert!(output.contains("Source: https://github.com/skilld-dev/skills/blob/0123456789abcdef0123456789abcdef01234567/skills/example/SKILL.md"), "{output}");
    assert!(
        output.contains("Read the instructions before using each Skill."),
        "{output}"
    );
    assert!(stderr.is_empty());
}

#[test]
fn cli_install_prints_no_page_when_the_server_names_none() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project, temporary.path().join("data"))
        .with_remote_provider(provider("---\nname: unlisted\n---\n"));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        [
            "skilld",
            "install",
            "skilld-dev/skills/unlisted",
            "--agent",
            "codex",
        ],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    let stdout = String::from_utf8(stdout).unwrap();
    assert!(stdout.contains("Source status: verified"), "{stdout}");
    assert!(!stdout.contains("Skill page"), "{stdout}");
    assert!(!stdout.contains("skilld.dev/gh"), "{stdout}");
}

#[test]
fn cli_direct_install_marks_review_as_required() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
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
    let output = String::from_utf8(stdout).unwrap();
    assert!(
        output.contains(&format!(
            "Files: {}",
            project.join(".agents/skills/example").display()
        )),
        "{output}"
    );
    assert!(output.contains("Source status: unverified"), "{output}");
    assert!(output.contains("Source: https://github.com/skilld-dev/skills/blob/0123456789abcdef0123456789abcdef01234567/skills/example/SKILL.md"), "{output}");
    assert!(
        output.contains("Read the instructions before using each Skill."),
        "{output}"
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
    let output = String::from_utf8(stdout).unwrap();
    assert!(
        output.contains(&format!(
            "Files: {}",
            project.join(".agents/skills/example").display()
        )),
        "{output}"
    );
    assert!(output.contains("Source status: unverified"), "{output}");
    assert!(output.contains("Source: https://github.com/skilld-dev/skills/blob/0123456789abcdef0123456789abcdef01234567/skills/example/SKILL.md"), "{output}");
    assert!(
        output.contains("Read the instructions before using each Skill."),
        "{output}"
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
                "skilld-dev/skills/example",
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
    let output = String::from_utf8(stdout).unwrap();
    assert!(
        output.contains(&format!(
            "Files: {}",
            project.join(".agents/skills/example").display()
        )),
        "{output}"
    );
    assert!(output.contains("Source status: verified"), "{output}");
    assert!(output.contains("Source: https://github.com/skilld-dev/skills/blob/0123456789abcdef0123456789abcdef01234567/skills/example/SKILL.md"), "{output}");
    assert!(
        output.contains("Read the instructions before using each Skill."),
        "{output}"
    );
    assert!(stderr.is_empty());
    assert_eq!(
        *provider.prepares.lock().unwrap(),
        [
            ("skilld-dev/skills/example".to_owned(), false),
            (
                "skilld-dev/skills/example#commit:0123456789abcdef0123456789abcdef01234567"
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

#[test]
fn cli_restores_a_lockfile_that_records_the_legacy_skilld_prefix() {
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
                "skilld-dev/skills/example",
                "--agent",
                "codex"
            ],
            &host,
            &mut stdout,
            &mut stderr,
        )
        .exit_code,
        0
    );
    // A 3.0 lockfile spells the same source with the skilld: prefix.
    let lockfile = project.join(".skills/skilld-lock.yaml");
    let legacy = fs::read_to_string(&lockfile).unwrap().replace(
        "\"skilld-dev/skills/example",
        "\"skilld:skilld-dev/skills/example",
    );
    assert!(legacy.contains("\"skilld:skilld-dev/skills/example"));
    fs::write(&lockfile, legacy).unwrap();
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

    assert_eq!(
        restored.exit_code,
        0,
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(project.join(".skills/example/SKILL.md").is_file());
    assert!(
        String::from_utf8(stdout)
            .unwrap()
            .contains("Commit: 0123456789abcdef0123456789abcdef01234567")
    );
}

// ---------------------------------------------------------------------------
// Direct mode tarball delivery.
//
// Direct mode reads one GitHub request per file, against an unauthenticated
// ceiling of 60 an hour. The Repository tarball is one request for every file
// of every Skill, and it costs no REST quota. These tests build the archive
// here, so they state the exact bytes the CLI must accept and reject.
// ---------------------------------------------------------------------------

const TAR_TOP: &str = "skilld-dev-skills-0123456";

fn git_blob_sha(bytes: &[u8]) -> String {
    let mut hasher = sha1::Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One ustar header block. `mode` is written the way GitHub writes it, which
/// is not the mode the Git tree carries.
fn tar_header(name: &str, size: usize, kind: u8, mode: u32) -> Vec<u8> {
    let mut header = vec![0_u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
    header[108..115].copy_from_slice(b"0000000");
    header[116..123].copy_from_slice(b"0000000");
    header[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
    header[136..147].copy_from_slice(b"00000000000");
    header[156] = kind;
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[148..156].copy_from_slice(b"        ");
    let checksum = header.iter().map(|byte| u32::from(*byte)).sum::<u32>();
    header[148..154].copy_from_slice(format!("{checksum:06o}").as_bytes());
    header[154] = 0;
    header[155] = b' ';
    header
}

fn tar_body(bytes: &[u8]) -> Vec<u8> {
    let mut body = bytes.to_vec();
    body.resize(bytes.len().div_ceil(512) * 512, 0);
    body
}

/// One regular file entry, named inside GitHub's top level directory.
fn tar_entry(path: &str, bytes: &[u8], mode: u32) -> Vec<u8> {
    let name = format!("{TAR_TOP}/{path}");
    let mut entry = tar_header(&name, bytes.len(), b'0', mode);
    entry.extend(tar_body(bytes));
    entry
}

/// One pax `x` extended header carrying the path, then the file itself under a
/// truncated name. Git writes this shape for a path over 100 bytes.
fn tar_pax_entry(path: &str, bytes: &[u8], mode: u32) -> Vec<u8> {
    let name = format!("{TAR_TOP}/{path}");
    let record_body = format!(" path={name}\n");
    let mut length = record_body.len() + 2;
    loop {
        let record = format!("{length}{record_body}");
        if record.len() == length {
            let mut entry = tar_header("pax_global_header", record.len(), b'x', 0o664);
            entry.extend(tar_body(record.as_bytes()));
            entry.extend(tar_entry(&path[..40], bytes, mode));
            return entry;
        }
        length = record.len();
    }
}

fn tar_archive(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut tar = tar_header(&format!("{TAR_TOP}/"), 0, b'5', 0o775);
    for (path, bytes, mode) in files {
        if path.len() > 90 {
            tar.extend(tar_pax_entry(path, bytes, *mode));
        } else {
            tar.extend(tar_entry(path, bytes, *mode));
        }
    }
    tar.extend(vec![0_u8; 1024]);
    tar
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let carry = crc & 1;
            crc >>= 1;
            if carry == 1 {
                crc ^= 0xedb8_8320;
            }
        }
    }
    !crc
}

/// One gzip member whose deflate stream is stored blocks. Storing keeps the
/// fixture readable; the CLI inflates it through the same path as a compressed
/// archive.
fn gzip(payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xff];
    let mut chunks = payload.chunks(0xffff).peekable();
    if chunks.peek().is_none() {
        out.extend([0x01, 0x00, 0x00, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        let last = u8::from(chunks.peek().is_none());
        out.push(last);
        out.extend((chunk.len() as u16).to_le_bytes());
        out.extend((!(chunk.len() as u16)).to_le_bytes());
        out.extend(chunk);
    }
    out.extend(crc32(payload).to_le_bytes());
    out.extend((payload.len() as u32).to_le_bytes());
    out
}

fn tarball_response(files: &[(&str, &[u8], u32)]) -> HttpResponse {
    response(200, gzip(&tar_archive(files)))
}

fn direct_repository_response() -> HttpResponse {
    response(
        200,
        br#"{"private":false,"default_branch":"main"}"#.to_vec(),
    )
}

fn direct_commit_response(sha: &str) -> HttpResponse {
    response(
        200,
        format!(
            r#"{{"sha":"{sha}","commit":{{"tree":{{"sha":"89abcdef0123456789abcdef0123456789abcdef"}}}}}}"#
        ),
    )
}

fn tree_blob(path: &str, bytes: &[u8], mode: &str) -> serde_json::Value {
    json!({
        "path": path,
        "mode": mode,
        "type": "blob",
        "sha": git_blob_sha(bytes),
        "size": bytes.len(),
    })
}

fn direct_tree_response(entries: Vec<serde_json::Value>) -> HttpResponse {
    response(
        200,
        serde_json::to_vec(&json!({ "truncated": false, "tree": entries })).unwrap(),
    )
}

fn blob_response(bytes: &[u8]) -> HttpResponse {
    response(
        200,
        serde_json::to_vec(&json!({
            "content": STANDARD.encode(bytes),
            "encoding": "base64",
            "size": bytes.len(),
        }))
        .unwrap(),
    )
}

fn direct_remote(http: Arc<FakeHttp>) -> SkilldRemote {
    SkilldRemote::new(
        http,
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_sleeper(Arc::new(NoSleep))
}

fn direct_selector(name: &str) -> RemoteSelector {
    RemoteSelector::parse(&format!("github:skilld-dev/skills/skills/{name}")).unwrap()
}

fn file_bytes<'a>(prepared: &'a PreparedRemoteSkill, path: &str) -> &'a [u8] {
    prepared
        .files
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.bytes.as_slice())
        .unwrap_or_else(|| panic!("the prepared Skill has no {path}"))
}

const DIRECT_SKILL: &[u8] = b"---\nname: example\ndescription: fixture\n---\n";
const DIRECT_REFERENCE: &[u8] = b"# reference\n";
const DIRECT_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn a_direct_install_reads_the_repository_tarball_once_instead_of_one_blob_per_file() {
    let http = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        direct_tree_response(vec![
            tree_blob("skills/example/SKILL.md", DIRECT_SKILL, "100644"),
            tree_blob("skills/example/reference.md", DIRECT_REFERENCE, "100644"),
        ]),
        tarball_response(&[
            ("skills/example/SKILL.md", DIRECT_SKILL, 0o664),
            ("skills/example/reference.md", DIRECT_REFERENCE, 0o664),
        ]),
    ]));
    let remote = direct_remote(http.clone());

    let prepared = remote.prepare(&direct_selector("example"), true).unwrap();

    assert_eq!(file_bytes(&prepared, "SKILL.md"), DIRECT_SKILL);
    assert_eq!(file_bytes(&prepared, "reference.md"), DIRECT_REFERENCE);
    // Repository, commit, tree, tarball. No request per file.
    assert_eq!(request_paths(&http).len(), 4);
    assert!(request_paths(&http)[3].contains(&format!("/tarball/{DIRECT_COMMIT}")));
}

#[test]
fn a_file_the_tarball_does_not_carry_falls_back_to_the_blob_path() {
    // `.gitattributes export-ignore` drops files from the archive silently.
    let http = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        direct_tree_response(vec![
            tree_blob("skills/example/SKILL.md", DIRECT_SKILL, "100644"),
            tree_blob("skills/example/reference.md", DIRECT_REFERENCE, "100644"),
        ]),
        tarball_response(&[("skills/example/SKILL.md", DIRECT_SKILL, 0o664)]),
        blob_response(DIRECT_SKILL),
        blob_response(DIRECT_REFERENCE),
    ]));
    let remote = direct_remote(http.clone());

    let prepared = remote.prepare(&direct_selector("example"), true).unwrap();

    assert_eq!(prepared.files.len(), 2);
    assert_eq!(file_bytes(&prepared, "reference.md"), DIRECT_REFERENCE);
    assert_eq!(request_paths(&http).len(), 6);
}

#[test]
fn a_tarball_file_that_misses_its_blob_sha_falls_back_to_the_blob_path() {
    let http = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        direct_tree_response(vec![tree_blob(
            "skills/example/SKILL.md",
            DIRECT_SKILL,
            "100644",
        )]),
        tarball_response(&[(
            "skills/example/SKILL.md",
            b"---\nname: example\ndescription: altered\n--\n".as_slice(),
            0o664,
        )]),
        blob_response(DIRECT_SKILL),
    ]));
    let remote = direct_remote(http.clone());

    let prepared = remote.prepare(&direct_selector("example"), true).unwrap();

    assert_eq!(file_bytes(&prepared, "SKILL.md"), DIRECT_SKILL);
    assert_eq!(request_paths(&http).len(), 5);
}

#[test]
fn a_repository_past_the_tarball_size_gate_never_downloads_the_archive() {
    let big = json!({
        "path": "media/build.bin",
        "mode": "100644",
        "type": "blob",
        "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "size": 200_u64 * 1024 * 1024,
    });
    let http = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        direct_tree_response(vec![
            tree_blob("skills/example/SKILL.md", DIRECT_SKILL, "100644"),
            big,
        ]),
        blob_response(DIRECT_SKILL),
    ]));
    let remote = direct_remote(http.clone());

    let prepared = remote.prepare(&direct_selector("example"), true).unwrap();

    assert_eq!(file_bytes(&prepared, "SKILL.md"), DIRECT_SKILL);
    assert!(
        request_paths(&http)
            .iter()
            .all(|path| !path.contains("/tarball/")),
        "a 200 MiB Repository must not be downloaded for one Skill file"
    );
}

#[test]
fn a_pax_long_path_is_read_out_of_the_tarball() {
    let long = format!("skills/example/{}/note.md", "a".repeat(120));
    let http = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        direct_tree_response(vec![
            tree_blob("skills/example/SKILL.md", DIRECT_SKILL, "100644"),
            tree_blob(&long, DIRECT_REFERENCE, "100644"),
        ]),
        tarball_response(&[
            ("skills/example/SKILL.md", DIRECT_SKILL, 0o664),
            (long.as_str(), DIRECT_REFERENCE, 0o664),
        ]),
    ]));
    let remote = direct_remote(http.clone());

    let prepared = remote.prepare(&direct_selector("example"), true).unwrap();

    assert_eq!(
        file_bytes(&prepared, &format!("{}/note.md", "a".repeat(120))),
        DIRECT_REFERENCE
    );
    assert_eq!(request_paths(&http).len(), 4);
}

#[test]
fn the_file_mode_comes_from_the_git_tree_not_the_tar_header() {
    // GitHub tarballs report 0664 and 0775, which no Git tree ever carries.
    let script = b"#!/bin/sh\necho skilld\n";
    let http = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        direct_tree_response(vec![
            tree_blob("skills/example/SKILL.md", DIRECT_SKILL, "100644"),
            tree_blob("skills/example/run.sh", script, "100755"),
        ]),
        tarball_response(&[
            ("skills/example/SKILL.md", DIRECT_SKILL, 0o775),
            ("skills/example/run.sh", script, 0o664),
        ]),
    ]));
    let remote = direct_remote(http.clone());

    let prepared = remote.prepare(&direct_selector("example"), true).unwrap();

    let mode = |path: &str| {
        prepared
            .files
            .iter()
            .find(|file| file.path == path)
            .unwrap()
            .mode
    };
    assert_eq!(mode("run.sh"), 0o755);
    assert_eq!(mode("SKILL.md"), 0o644);
}

#[test]
fn a_tarball_redirect_to_codeload_is_followed_exactly_once() {
    let mut redirect = response(302, Vec::new());
    redirect.headers.insert(
        "location".to_owned(),
        format!("https://codeload.github.com/skilld-dev/skills/legacy.tar.gz/{DIRECT_COMMIT}"),
    );
    let http = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        direct_tree_response(vec![tree_blob(
            "skills/example/SKILL.md",
            DIRECT_SKILL,
            "100644",
        )]),
        redirect,
        tarball_response(&[("skills/example/SKILL.md", DIRECT_SKILL, 0o664)]),
    ]));
    let remote = direct_remote(http.clone());

    let prepared = remote.prepare(&direct_selector("example"), true).unwrap();

    assert_eq!(file_bytes(&prepared, "SKILL.md"), DIRECT_SKILL);
    let paths = request_paths(&http);
    assert_eq!(paths.len(), 5);
    assert!(paths[4].starts_with("https://codeload.github.com/"));
}

#[test]
fn a_tarball_redirect_away_from_codeload_is_rejected_and_falls_back() {
    let mut redirect = response(302, Vec::new());
    redirect.headers.insert(
        "location".to_owned(),
        "https://codeload.github.com.example.com/skilld-dev/skills/legacy.tar.gz".to_owned(),
    );
    let http = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        direct_tree_response(vec![tree_blob(
            "skills/example/SKILL.md",
            DIRECT_SKILL,
            "100644",
        )]),
        redirect,
        blob_response(DIRECT_SKILL),
    ]));
    let remote = direct_remote(http.clone());

    let prepared = remote.prepare(&direct_selector("example"), true).unwrap();

    assert_eq!(file_bytes(&prepared, "SKILL.md"), DIRECT_SKILL);
    let paths = request_paths(&http);
    assert_eq!(paths.len(), 5);
    assert!(
        paths
            .iter()
            .all(|path| !path.contains("codeload.github.com.example.com")),
        "the rejected redirect host was requested: {paths:?}"
    );
}

/// The whole point of the change, measured rather than argued: every Skill of
/// one Repository, through both paths, with the requests counted.
#[test]
fn installing_every_skill_of_a_repository_downloads_one_tarball() {
    const SKILLS: usize = 33;
    const FILES_PER_SKILL: usize = 5;

    let names = (0..SKILLS)
        .map(|index| format!("skill-{index:02}"))
        .collect::<Vec<_>>();
    let contents = names
        .iter()
        .map(|name| {
            (0..FILES_PER_SKILL)
                .map(|index| {
                    let path = if index == 0 {
                        format!("skills/{name}/SKILL.md")
                    } else {
                        format!("skills/{name}/reference-{index}.md")
                    };
                    let bytes = format!("---\nname: {name}\ndescription: fixture {index}\n---\n")
                        .into_bytes();
                    (path, bytes)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let flat = contents.iter().flatten().collect::<Vec<_>>();
    let tree = direct_tree_response(
        flat.iter()
            .map(|(path, bytes)| tree_blob(path, bytes, "100644"))
            .collect(),
    );
    let archive = flat
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice(), 0o664))
        .collect::<Vec<_>>();

    let with_tarball = Arc::new(FakeHttp::with([
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        tree.clone(),
        tarball_response(&archive),
    ]));
    let remote = direct_remote(with_tarball.clone());
    for name in &names {
        remote.prepare(&direct_selector(name), true).unwrap();
    }

    // The per-blob path, measured on the same Repository: the tarball 404s, so
    // every file is read one at a time.
    let mut per_blob = vec![
        direct_repository_response(),
        direct_commit_response(DIRECT_COMMIT),
        tree,
        response(404, br#"{"message":"Not Found"}"#.to_vec()),
    ];
    per_blob.extend(flat.iter().map(|(_path, bytes)| blob_response(bytes)));
    let without_tarball = Arc::new(FakeHttp::with(per_blob));
    let remote = direct_remote(without_tarball.clone());
    for name in &names {
        remote.prepare(&direct_selector(name), true).unwrap();
    }

    // 33 Skills, 165 files: 169 requests become 4, and GitHub rate limits an
    // unauthenticated caller at 60 an hour.
    assert_eq!(without_tarball.requests.lock().unwrap().len(), 169);
    assert_eq!(with_tarball.requests.lock().unwrap().len(), 4);
}

#[test]
fn an_endpoint_override_must_be_one_https_or_loopback_origin() {
    let remote = || {
        SkilldRemote::new(
            Arc::new(FakeHttp::default()),
            Arc::new(NoTokenProvider),
            NativeRemoteConfig::Unconfigured,
        )
    };
    for accepted in [
        "https://preview.skilld.dev",
        "https://preview.skilld.dev/",
        "http://localhost:3000",
        "http://127.0.0.1:8787/",
    ] {
        assert!(remote().with_endpoint(accepted).is_ok(), "{accepted}");
    }
    for rejected in [
        "http://preview.skilld.dev",
        "https://skilld.dev/api",
        "https://skilld.dev/?next=x",
        "https://user:secret@skilld.dev",
        "not a url",
    ] {
        let Err(error) = remote().with_endpoint(rejected) else {
            panic!("{rejected} must be refused");
        };
        assert_eq!(error.code, "INVALID_ENDPOINT", "{rejected}");
    }
}

const SUDO_BODY: &str = "\n```sh\nsudo true\n```\n";

fn installed_alpha(
    body: &'static str,
    allowed: &[&str],
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Arc<BatchProvider>,
    LocalHost,
) {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(BatchProvider {
        version: Mutex::new("first"),
        latest_commit: Mutex::new('f'),
        prepared_names: Mutex::new(vec![]),
        fail_name: Mutex::new(None),
        relation: Mutex::new(RemoteComparisonRelation::Ahead),
        body: Mutex::new(body),
    });
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(
            "skilld-dev/skills/alpha".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
        allowed_behaviors: allowed.iter().map(|id| (*id).to_owned()).collect(),
    })
    .unwrap();
    *provider.version.lock().unwrap() = "second";
    (temporary, project, provider, host)
}

fn update_cli(host: &LocalHost, args: &[&str]) -> (u8, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = skilld_command::run_with_output(
        args,
        host,
        skilld_command::OutputContext::Plain {
            platform: skilld_command::CommandPlatform::Unix,
        },
        &mut stdout,
        &mut stderr,
    );
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap() + &String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn an_update_that_adds_an_ask_behavior_changes_nothing_until_approved() {
    let (_temporary, project, provider, host) = installed_alpha("", &[]);
    *provider.body.lock().unwrap() = SUDO_BODY;

    let (exit, output) = update_cli(&host, &["skilld", "update"]);

    assert_eq!(exit, 1, "{output}");
    assert!(
        output.starts_with("BEHAVIOR_CONFIRMATION_REQUIRED:"),
        "{output}"
    );
    assert!(
        output.contains("The update of Skill alpha adds behaviors"),
        "{output}"
    );
    assert!(
        output.contains("Runs commands as root: SKILL.md:7"),
        "{output}"
    );
    assert!(output.trim_end().ends_with("--allow privilege"), "{output}");
    assert_eq!(
        fs::read_to_string(project.join(".skills/alpha/SKILL.md")).unwrap(),
        "---\nname: alpha\ndescription: first\n---\n"
    );

    let (exit, output) = update_cli(&host, &["skilld", "update", "--allow", "privilege"]);

    assert_eq!(exit, 0, "{output}");
    assert!(
        fs::read_to_string(project.join(".skills/alpha/SKILL.md"))
            .unwrap()
            .contains("sudo true")
    );
}

#[test]
fn an_update_keeps_an_ask_behavior_the_installed_copy_already_had() {
    let (_temporary, project, _provider, host) = installed_alpha(SUDO_BODY, &["privilege"]);

    let (exit, output) = update_cli(&host, &["skilld", "update"]);

    assert_eq!(exit, 0, "{output}");
    assert!(
        fs::read_to_string(project.join(".skills/alpha/SKILL.md"))
            .unwrap()
            .contains("description: second")
    );
}

#[test]
fn a_blocked_resolution_prints_the_first_findings_of_each_failed_check() {
    let http = Arc::new(FakeHttp::with([response(
        200,
        serde_json::to_vec(&json!({
            "state": "blocked",
            "resolutionId": "0f9a4a44-27f9-4f6a-9a21-4d24d8ff2f60",
            "checkResults": [
                {
                    "name": "agent-skills-spec",
                    "version": "2026-08-20",
                    "outcome": "fail",
                    "required": true,
                    "summary": "The Skill does not match the Agent Skills specification.",
                    "findings": ["The Skill name must match its directory name."],
                },
                {
                    "name": "source-policy",
                    "version": "1",
                    "outcome": "fail",
                    "required": true,
                    "summary": "A Skill file exceeds 2097152 bytes.",
                    "findings": ["a.webp", "b.gif", "c\u{1b}[2J.mp4", "d.png", "e.mov"],
                },
            ],
        }))
        .unwrap(),
    )]));
    let remote = search_remote(http);

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "CHECK_BLOCKED");
    assert_eq!(
        error.message,
        "the Resolution was blocked by check results. \
agent-skills-spec: The Skill does not match the Agent Skills specification. \
Findings: The Skill name must match its directory name. \
source-policy: A Skill file exceeds 2097152 bytes. \
Findings: a.webp; b.gif; c [2J.mp4; and 2 more."
    );
}

fn pinned_remote(
    pin: TrustedRootPin,
    http: Arc<FakeHttp>,
    sleeper: Arc<RecordingSleeper>,
) -> SkilldRemote {
    SkilldRemote::new(
        http,
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(pin),
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(sleeper)
}

fn example_selector() -> RemoteSelector {
    RemoteSelector::parse("skilld-dev/skills/example").unwrap()
}

fn pending_at(stage: &str, poll_after_ms: u64) -> HttpResponse {
    response(
        200,
        serde_json::to_vec(&json!({
            "state": "pending",
            "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
            "stage": stage,
            "pollAfterMs": poll_after_ms
        }))
        .unwrap(),
    )
}

fn request_urls(http: &FakeHttp) -> Vec<String> {
    http.requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.url.clone())
        .collect()
}

#[test]
fn a_bad_gateway_during_a_deploy_repeats_the_same_resolution_request() {
    for status in [500, 502, 504] {
        let (pin, mut responses) = verified_remote_responses();
        responses.insert(0, response(status, b"<html>Bad gateway</html>".to_vec()));
        let http = Arc::new(FakeHttp::with(responses));
        let remote = pinned_remote(pin, http.clone(), Arc::new(RecordingSleeper::default()));

        let prepared = remote.prepare(&example_selector(), false);

        assert!(prepared.is_ok(), "HTTP {status}: {prepared:?}");
        let keys = idempotency_keys(&http);
        assert!(request_urls(&http)[1].ends_with("/api/v1/resolutions"));
        assert_eq!(
            keys[0], keys[1],
            "HTTP {status} must replay the same Resolution"
        );
    }
}

#[test]
fn a_truncated_artifact_download_is_downloaded_again() {
    let (pin, mut responses) = verified_remote_responses();
    let archive = responses[3].body.clone();
    responses[3] = response(200, archive[..512].to_vec());
    responses.push(response(200, archive));
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http.clone(), Arc::new(RecordingSleeper::default()));

    let prepared = remote.prepare(&example_selector(), false).unwrap();

    assert_eq!(prepared.files[0].path, "SKILL.md");
    let urls = request_urls(&http);
    assert_eq!(urls.len(), 5);
    assert!(urls[3].ends_with("/content"));
    assert!(urls[4].ends_with("/content"));
}

#[test]
fn a_corrupt_artifact_download_that_repeats_says_what_to_do_next() {
    let (pin, mut responses) = verified_remote_responses();
    let mut corrupt = responses[3].body.clone();
    corrupt[600] ^= 1;
    responses[3] = response(200, corrupt.clone());
    responses.push(response(200, corrupt));
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http.clone(), Arc::new(RecordingSleeper::default()));

    let error = remote.prepare(&example_selector(), false).unwrap_err();

    assert_eq!(error.code, "ARTIFACT_DIGEST_MISMATCH");
    assert_eq!(
        error.message,
        "the downloaded Artifact does not match its attestation. skilld downloaded it twice and loaded nothing"
    );
    assert_eq!(
        error.next_step.as_deref(),
        Some(
            "The Skill did not cause this failure. The download changed on its way. Run the same command once more."
        )
    );
    assert_eq!(http.requests.lock().unwrap().len(), 5);
}

#[test]
fn a_resolution_stalled_at_signing_is_requested_again() {
    let (pin, ready) = verified_remote_responses();
    let mut responses = vec![
        pending_at("signing", 5_000),
        pending_at("signing", 5_000),
        pending_at("signing", 5_000),
        pending_at("signing", 5_000),
    ];
    responses.extend(ready);
    let http = Arc::new(FakeHttp::with(responses));
    let sleeper = Arc::new(RecordingSleeper::default());
    let remote = pinned_remote(pin, http.clone(), sleeper.clone());

    let prepared = remote.prepare(&example_selector(), false);

    assert!(prepared.is_ok(), "{prepared:?}");
    let urls = request_urls(&http);
    let posts = urls
        .iter()
        .enumerate()
        .filter(|(_, url)| url.ends_with("/api/v1/resolutions"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(posts, [0, 4]);
    let keys = idempotency_keys(&http);
    assert_ne!(keys[0], keys[4]);
    assert!(*sleeper.elapsed.lock().unwrap() < Duration::from_secs(20));
}

#[test]
fn a_slow_github_fetch_keeps_its_resolution() {
    let (pin, ready) = verified_remote_responses();
    let mut responses = (0..6)
        .map(|_| pending_at("fetching", 5_000))
        .collect::<Vec<_>>();
    responses.extend(ready);
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http.clone(), Arc::new(RecordingSleeper::default()));

    let prepared = remote.prepare(&example_selector(), false);

    assert!(prepared.is_ok(), "{prepared:?}");
    let posts = request_urls(&http)
        .iter()
        .filter(|url| url.ends_with("/api/v1/resolutions"))
        .count();
    assert_eq!(posts, 1);
}

#[test]
fn a_missing_source_says_what_to_check() {
    let http = Arc::new(FakeHttp::with([failed_resolution(
        "SOURCE_NOT_FOUND",
        false,
        None,
    )]));
    let remote = resolution_remote(http, Arc::new(RecordingSleeper::default()));

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "SOURCE_NOT_FOUND");
    assert_eq!(
        error.message,
        "skilld.dev found no Skill at this source. Check the owner, Repository, and Skill name."
    );
}

#[test]
fn a_large_artifact_download_gets_time_for_its_size() {
    let mut skill = b"---\nname: example\ndescription: large\n---\n".to_vec();
    skill.resize(6 * 1024 * 1024, b'a');
    let (pin, responses) = verified_remote_responses_for(&skill);
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http.clone(), Arc::new(RecordingSleeper::default()));

    let prepared = remote.prepare(&example_selector(), false);

    assert!(prepared.is_ok(), "{prepared:?}");
    let timeouts = http.timeouts.lock().unwrap();
    assert!(
        timeouts[3].is_some_and(|timeout| timeout > Duration::from_secs(60)),
        "{timeouts:?}"
    );
}

/// Run `skilld run` against a remote served by `http`, and return the exit
/// code, stdout, and stderr.
fn run_remote_cli(remote: SkilldRemote, args: &[&str]) -> (u8, String, String) {
    let temporary = tempfile::tempdir().unwrap();
    let host = LocalHost::new(
        temporary.path().join("project"),
        temporary.path().join("data"),
    )
    .with_remote_provider(Arc::new(remote));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = skilld_command::run_with_output(
        args,
        &host,
        skilld_command::OutputContext::Plain {
            platform: skilld_command::CommandPlatform::Unix,
        },
        &mut stdout,
        &mut stderr,
    );
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn unpinned(responses: Vec<HttpResponse>) -> SkilldRemote {
    resolution_remote(
        Arc::new(FakeHttp::with(responses)),
        Arc::new(RecordingSleeper::default()),
    )
}

const RUN: [&str; 3] = ["skilld", "run", "skilld-dev/skilld/skilld"];

#[test]
fn a_retryable_failure_says_how_often_skilld_asked_and_exits_with_75() {
    let failed = || failed_resolution("SERVICE_UNAVAILABLE", true, None);
    let remote = unpinned(vec![failed(), failed(), failed(), failed()]);

    let (exit, stdout, stderr) = run_remote_cli(remote, &RUN);

    assert_eq!((exit, stdout.as_str()), (75, ""));
    assert_eq!(
        stderr,
        "SERVICE_UNAVAILABLE: skilld.dev could not create the Artifact this time\n\
Next step: skilld requested the Skill 4 times. Run the same command again in a minute.\n"
    );
}

fn signed_in(responses: Vec<HttpResponse>) -> SkilldRemote {
    SkilldRemote::new(
        Arc::new(FakeHttp::with(responses)),
        Arc::new(FixedToken),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint("http://127.0.0.1:8787")
    .unwrap()
    .with_sleeper(Arc::new(RecordingSleeper::default()))
}

#[test]
fn a_rate_limited_run_without_a_sign_in_offers_the_own_quota_first() {
    let remote = unpinned(vec![failed_resolution("RATE_LIMITED", true, Some(1_500))]);

    let (exit, _, stderr) = run_remote_cli(remote, &RUN);

    assert_eq!(exit, 75);
    assert_eq!(
        stderr,
        "RATE_LIMITED: the Resolution failed. Retry in 25 minutes.\n\
Next step: To use your own GitHub quota, run skilld auth login. Then run the same command again. \
Without a sign-in, run the same command again in 25 minutes. skilld requested the Skill once.\n"
    );
}

#[test]
fn a_rate_limited_run_that_is_signed_in_names_the_wait_skilld_dev_asked_for() {
    let remote = signed_in(vec![failed_resolution("RATE_LIMITED", true, Some(1_500))]);

    let (exit, _, stderr) = run_remote_cli(remote, &RUN);

    assert_eq!(exit, 75);
    assert_eq!(
        stderr,
        "RATE_LIMITED: the Resolution failed. Retry in 25 minutes.\n\
Next step: skilld requested the Skill once. Run the same command again in 25 minutes.\n"
    );
}

#[test]
fn a_blocked_skill_says_not_to_retry_and_names_its_source() {
    let blocked = response(
        200,
        serde_json::to_vec(&json!({
            "state": "blocked",
            "resolutionId": "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f",
            "checkResults": [{
                "name": "credential-material",
                "version": "1",
                "outcome": "fail",
                "required": true,
                "summary": "The Skill contains private key material.",
                "findings": ["examples/key.pem"],
            }],
        }))
        .unwrap(),
    );

    let (exit, _, stderr) = run_remote_cli(unpinned(vec![blocked]), &RUN);

    assert_eq!(exit, 1);
    assert_eq!(
        stderr,
        "CHECK_BLOCKED: the Resolution was blocked by check results. \
credential-material: The Skill contains private key material. Findings: examples/key.pem.\n\
Next step: Do not retry. The checks give the same result each time. \
Tell the user why skilld.dev blocked the Skill, from the message above. \
If the user wants to read the Skill, its source is at https://github.com/skilld-dev/skilld.\n"
    );
}

#[test]
fn a_missing_skill_lists_the_nearest_names_its_repository_holds() {
    let remote = unpinned(vec![
        failed_resolution("SOURCE_NOT_FOUND", false, None),
        registry_page(&[
            ("skilld-dev", "skilld", "skilld-maintainer", None),
            ("skilld-dev", "skilld", "generate-package-skill", None),
            ("skilld-dev", "skilld", "skills", None),
            ("skilld-dev", "skilld", "skilld-cli", None),
            ("skilld-dev", "other", "skilld", None),
        ]),
    ]);

    let (exit, _, stderr) =
        run_remote_cli(remote, &["skilld", "run", "skilld-dev/skilld/skilld-cli"]);

    assert_eq!(exit, 1);
    assert_eq!(
        stderr,
        "SOURCE_NOT_FOUND: skilld.dev found no Skill at this source. Check the owner, Repository, and Skill name.\n\
Next step: Do not retry this ref. Did you mean one of these? skilld-dev/skilld/skills, \
skilld-dev/skilld/skilld-maintainer, skilld-dev/skilld/generate-package-skill. \
To find more Skills, run skilld search skilld cli.\n"
    );
}

#[test]
fn a_missing_skill_without_a_near_name_suggests_a_search() {
    let remote = unpinned(vec![
        failed_resolution("SOURCE_NOT_FOUND", false, None),
        registry_page(&[]),
        response(200, br#"{"items":[],"total":0}"#.to_vec()),
    ]);

    let (exit, _, stderr) = run_remote_cli(remote, &RUN);

    assert_eq!(exit, 1);
    assert_eq!(
        stderr,
        "SOURCE_NOT_FOUND: skilld.dev found no Skill at this source. Check the owner, Repository, and Skill name.\n\
Next step: Do not retry this ref. Check the owner, Repository, and Skill name. \
To find more Skills, run skilld search skilld.\n"
    );
}

#[test]
fn a_network_failure_says_the_skill_did_not_cause_it() {
    let remote = unpinned(vec![]);

    let (exit, _, stderr) = run_remote_cli(remote, &RUN);

    assert_eq!(exit, 75);
    assert_eq!(
        stderr,
        "HTTP_TRANSPORT: the fake response queue is empty\n\
Next step: The Skill did not cause this failure. Run the same command once more. \
If it fails again, tell the user to check the network connection.\n"
    );
}

#[test]
fn a_resolution_that_outlasts_sixty_seconds_says_to_run_once_more() {
    let pending = || pending_at("checking", 30_000);
    let remote = unpinned(vec![pending(), pending(), pending()]);

    let (exit, _, stderr) = run_remote_cli(remote, &RUN);

    assert_eq!(exit, 75);
    assert_eq!(
        stderr,
        "RESOLUTION_TIMEOUT: skilld.dev did not finish the Artifact within 60 seconds\n\
Next step: The Skill did not cause this failure. skilld requested the Skill once. Run the same command once more.\n"
    );
}

#[test]
fn a_json_failure_carries_the_next_step_and_marks_a_retryable_one() {
    let failed = || failed_resolution("SIGNER_UNAVAILABLE", true, None);
    let remote = unpinned(vec![failed(), failed(), failed(), failed()]);

    let (exit, _, stderr) = run_remote_cli(
        remote,
        &["skilld", "run", "skilld-dev/skilld/skilld", "--json"],
    );

    assert_eq!(exit, 75);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stderr).unwrap(),
        json!({
            "schemaVersion": 1,
            "_tag": "OperationError",
            "error": {
                "code": "SIGNER_UNAVAILABLE",
                "message": "skilld.dev could not create the Artifact this time",
                "retryable": true,
                "nextStep": "skilld requested the Skill 4 times. Run the same command again in a minute.",
            },
        })
    );
}

fn omitted_files_check(findings: &[&str]) -> CheckResult {
    CheckResult {
        name: "omitted-files".to_owned(),
        version: "1".to_owned(),
        outcome: CheckOutcome::Warn,
        required: false,
        summary: Some("skilld.dev left out files over the size limits.".to_owned()),
        findings: findings
            .iter()
            .map(|finding| (*finding).to_owned())
            .collect(),
    }
}

const SKILL_BYTES: &[u8] = b"---\nname: example\ndescription: verified\n---\n";

#[test]
fn a_skill_without_its_oversized_files_names_each_one() {
    let mut checks = passing_checks();
    checks.push(omitted_files_check(&[
        "assets/demo.mp4: 9,311,232 bytes, https://github.com/skilld-dev/skills/blob/0123456789abcdef0123456789abcdef01234567/skills/example/assets/demo.mp4",
        "assets/large.bin: 3000000 bytes, https://example.com/large.bin",
        "a finding in another shape",
    ]));
    let (pin, responses) = verified_remote_responses_with(SKILL_BYTES, checks);
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http, Arc::new(RecordingSleeper::default()));

    let (exit, stdout, stderr) =
        run_remote_cli(remote, &["skilld", "run", "skilld-dev/skills/example"]);

    assert_eq!(exit, 0, "{stderr}");
    assert!(
        stdout.contains(
            "The Skill loaded without 3 files over the skilld.dev size limits:\n  \
assets/demo.mp4 (8.88 MiB) https://github.com/skilld-dev/skills/blob/0123456789abcdef0123456789abcdef01234567/skills/example/assets/demo.mp4\n  \
assets/large.bin (2.87 MiB)\n  \
a finding in another shape\n\
If the instructions need one of these files, tell the user it is missing.\n"
        ),
        "{stdout}"
    );
    let skill = stdout
        .split("--- SKILL.md ---\n")
        .nth(1)
        .and_then(|rest| rest.split("--- end of SKILL.md ---").next())
        .unwrap();
    assert_eq!(skill.as_bytes(), SKILL_BYTES);
}

#[test]
fn a_json_run_lists_the_omitted_files() {
    let mut checks = passing_checks();
    checks.push(omitted_files_check(&["assets/demo.mp4: 9311232 bytes"]));
    let (pin, responses) = verified_remote_responses_with(SKILL_BYTES, checks);
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http, Arc::new(RecordingSleeper::default()));

    let (exit, stdout, _) = run_remote_cli(
        remote,
        &["skilld", "run", "skilld-dev/skills/example", "--json"],
    );

    assert_eq!(exit, 0);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        json["data"]["omittedFiles"],
        json!([{ "path": "assets/demo.mp4", "bytes": 9_311_232, "url": null }])
    );
}

#[test]
fn a_run_without_omitted_files_adds_no_field() {
    let (pin, responses) = verified_remote_responses();
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http, Arc::new(RecordingSleeper::default()));

    let (exit, stdout, _) = run_remote_cli(
        remote,
        &["skilld", "run", "skilld-dev/skills/example", "--json"],
    );

    assert_eq!(exit, 0);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(json["data"].get("omittedFiles").is_none());
}

#[test]
fn answers_with_fields_this_release_does_not_know_still_parse() {
    let (pin, mut responses) = verified_remote_responses();
    for index in [0, 1, 2] {
        let mut body: serde_json::Value = serde_json::from_slice(&responses[index].body).unwrap();
        body["laterField"] = json!({ "added": "after this release" });
        if index == 0 {
            body["artifact"]["laterField"] = json!(true);
        }
        responses[index] = response(200, serde_json::to_vec(&body).unwrap());
    }
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http, Arc::new(RecordingSleeper::default()));

    let prepared = remote.prepare(&example_selector(), false);

    assert!(prepared.is_ok(), "{prepared:?}");
}

#[test]
fn a_problem_with_a_later_field_keeps_its_code() {
    let mut problem = response(
        404,
        serde_json::to_vec(&json!({
            "type": "about:blank",
            "title": "Not found",
            "status": 404,
            "code": "SOURCE_NOT_FOUND",
            "detail": "No Skill skilld-dev/skilld/skilld.",
            "laterField": 1,
        }))
        .unwrap(),
    );
    problem.headers.insert(
        "content-type".to_owned(),
        "application/problem+json".to_owned(),
    );
    let remote = unpinned(vec![problem]);

    let error = remote.prepare(&skilld_selector(), false).unwrap_err();

    assert_eq!(error.code, "SOURCE_NOT_FOUND");
}

#[test]
fn a_skill_with_spec_findings_loads_with_a_warning() {
    let checks = vec![
        passing_checks().remove(0),
        CheckResult {
            name: "agent-skills-spec".to_owned(),
            version: "2".to_owned(),
            outcome: CheckOutcome::Warn,
            required: false,
            summary: Some("The Skill does not match the Agent Skills specification.".to_owned()),
            findings: vec!["SKILL.md has no frontmatter.".to_owned()],
        },
    ];
    let (pin, responses) =
        verified_remote_responses_with(b"# Example\n\nNo frontmatter.\n", checks);
    let http = Arc::new(FakeHttp::with(responses));
    let remote = pinned_remote(pin, http, Arc::new(RecordingSleeper::default()));

    let (exit, stdout, stderr) =
        run_remote_cli(remote, &["skilld", "run", "skilld-dev/skills/example"]);

    assert_eq!(exit, 0, "{stderr}");
    assert!(
        stdout.contains("Warning: SKILL.md declares no name. skilld uses the folder name example."),
        "{stdout}"
    );
}
