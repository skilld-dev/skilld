//! The skilld.dev public API commands, end to end through `run_with_output`.
//!
//! Each test drives the real `SkilldRemote` client over a fake HTTP adapter.
//! Answers come from the vendored contract's own examples, so a test reads
//! exactly what skilld.dev documents.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use skilld_command::{
    Cancellation, CommandPlatform, HttpAdapter, HttpMethod, HttpRequest, HttpResponse, LocalHost,
    NativeRemoteConfig, OutputContext, SecretValue, SkilldRemote, Sleeper, TokenProvider,
    run_with_output,
};
use skilld_core::RemoteError;

const SPEC: &str = include_str!("../../../packages/protocol/openapi/skilld-api-v1.json");
const ORIGIN: &str = "http://127.0.0.1:8787";
const TOKEN: &str = "skilld-test-token-1234567890";

#[derive(Default)]
struct FakeHttp {
    responses: Mutex<VecDeque<HttpResponse>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FakeHttp {
    fn with(responses: impl IntoIterator<Item = HttpResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(vec![]),
        })
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn only_request(&self) -> HttpRequest {
        let requests = self.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        requests.into_iter().next().unwrap()
    }
}

impl HttpAdapter for FakeHttp {
    fn send(
        &self,
        request: &HttpRequest,
        _cancellation: &dyn Cancellation,
        _timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        self.requests.lock().unwrap().push(request.clone());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| RemoteError::new("HTTP_TRANSPORT", "the fake response queue is empty"))
    }
}

struct Tokens(Option<&'static str>);

impl TokenProvider for Tokens {
    fn access_token(&self) -> Result<Option<SecretValue>, RemoteError> {
        self.0.map(SecretValue::new).transpose()
    }
}

struct NoSleep;

impl Sleeper for NoSleep {
    fn sleep(
        &self,
        _duration: Duration,
        _cancellation: &dyn Cancellation,
    ) -> Result<(), RemoteError> {
        Ok(())
    }
}

/// The response example the vendored contract documents for one operation.
fn example(id: &str) -> Value {
    let spec: Value = serde_json::from_str(SPEC).unwrap();
    for item in spec["paths"].as_object().unwrap().values() {
        for operation in item.as_object().unwrap().values() {
            if operation["operationId"] == id {
                let responses = operation["responses"].as_object().unwrap();
                let (_, success) = responses
                    .iter()
                    .find(|(status, _)| status.starts_with('2'))
                    .unwrap();
                return success["content"]["application/json"]["example"].clone();
            }
        }
    }
    panic!("the contract has no operation {id}")
}

fn json_response(status: u16, body: &Value) -> HttpResponse {
    HttpResponse {
        status,
        headers: BTreeMap::new(),
        body: serde_json::to_vec(body).unwrap(),
    }
}

fn empty_response() -> HttpResponse {
    HttpResponse {
        status: 204,
        headers: BTreeMap::new(),
        body: vec![],
    }
}

fn problem(status: u16, code: &str, detail: &str, headers: &[(&str, &str)]) -> HttpResponse {
    HttpResponse {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: serde_json::to_vec(&json!({
            "type": format!("https://skilld.dev/problems/{}", code.to_lowercase().replace('_', "-")),
            "title": "Problem",
            "status": status,
            "detail": detail,
            "code": code,
        }))
        .unwrap(),
    }
}

fn host(http: Arc<FakeHttp>, token: Option<&'static str>) -> LocalHost {
    let temporary = std::env::temp_dir();
    let remote = Arc::new(
        SkilldRemote::new(
            http,
            Arc::new(Tokens(token)),
            NativeRemoteConfig::Unconfigured,
        )
        .with_endpoint(ORIGIN)
        .unwrap()
        .with_sleeper(Arc::new(NoSleep)),
    );
    LocalHost::new(
        temporary.join("skilld-api-project"),
        temporary.join("skilld-api-data"),
    )
    .with_api(remote.clone())
    .with_remote_provider(remote)
}

const PLAIN: OutputContext = OutputContext::Plain {
    platform: CommandPlatform::Unix,
};

fn run(host: &LocalHost, args: &[&str]) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(args, host, PLAIN, &mut stdout, &mut stderr);
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn human(host: &LocalHost, args: &[&str]) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(
        args,
        host,
        OutputContext::HumanTerminal {
            width: 100,
            color: false,
            platform: CommandPlatform::Unix,
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

fn json_data(stdout: &str, command: &str) -> Value {
    let envelope: Value = serde_json::from_str(stdout).unwrap();
    assert_eq!(envelope["schemaVersion"], 1);
    assert_eq!(envelope["_tag"], "Success");
    assert_eq!(envelope["command"], command);
    assert_eq!(envelope["notices"], json!([]));
    envelope["data"].clone()
}

fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|header| header.name == name)
        .map(|header| header.value.expose())
}

fn body(request: &HttpRequest) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

// ---------------------------------------------------------------------------
// view
// ---------------------------------------------------------------------------

#[test]
fn view_of_a_registry_skill_shows_its_provenance_and_commands() {
    let http = FakeHttp::with([json_response(200, &example("skills.get"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, stderr) = run(
        &host,
        &[
            "skilld",
            "view",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
    );

    assert_eq!((exit, stderr.as_str()), (0, ""));
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Get);
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/skills/vercel-labs/agent-skills/web-design-guidelines"
    );
    assert_eq!(
        header(&request, "authorization"),
        None,
        "a public read sends no token"
    );
    assert_eq!(
        stdout,
        concat!(
            "Name: web-design-guidelines\n",
            "Title: Web Design Guidelines\n",
            "Author: vercel-labs (Vercel Labs)\n",
            "Repository: vercel-labs/agent-skills\n",
            "Read it first: https://github.com/vercel-labs/agent-skills/blob/4f1c2a9e0b7d3c5a8e6f1b2d4c7a9e0f3b5d8c1a/skills/web-design-guidelines/SKILL.md\n",
            "Commit: 4f1c2a9e0b7d3c5a8e6f1b2d4c7a9e0f3b5d8c1a\n",
            "Description: Review UI code for compliance with web interface guidelines.\n",
            "Generated summary: Checks interface code against a published list of web design rules.\n",
            "Stars: 18,204\n",
            "Likes: 41\n",
            "Updated: 2026-09-28\n",
            "License: MIT\n",
            "Tags: design, accessibility\n",
            "Files: references/checklist.md\n",
            "Run: skilld run vercel-labs/agent-skills/web-design-guidelines\n",
            "Install: skilld install vercel-labs/agent-skills/web-design-guidelines\n",
        ),
        "the production Skill page is off the configured origin, so it never prints"
    );
}

#[test]
fn view_json_carries_the_answer_exactly_as_skilld_dev_sent_it() {
    let mut answer = example("skills.get");
    answer["fieldFromTheFuture"] = json!({ "kept": true });
    let http = FakeHttp::with([json_response(200, &answer)]);
    let host = host(http, None);

    let (exit, stdout, stderr) = run(
        &host,
        &[
            "skilld",
            "view",
            "vercel-labs/agent-skills/web-design-guidelines",
            "--json",
        ],
    );

    assert_eq!((exit, stderr.as_str()), (0, ""));
    assert_eq!(json_data(&stdout, "view"), answer);
}

#[test]
fn view_prints_a_skill_page_on_the_configured_origin() {
    let mut answer = example("skills.get");
    answer["pageUrl"] =
        json!("http://127.0.0.1:8787/gh/vercel-labs/agent-skills/web-design-guidelines");
    let host = host(FakeHttp::with([json_response(200, &answer)]), None);

    let (_, stdout, _) = run(
        &host,
        &[
            "skilld",
            "view",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
    );

    assert!(
        stdout.contains(
            "Skill page: http://127.0.0.1:8787/gh/vercel-labs/agent-skills/web-design-guidelines\n"
        ),
        "{stdout}"
    );
}

#[test]
fn view_of_a_gone_skill_says_so() {
    let mut answer = example("skills.get");
    answer["sourceGone"] = json!(true);
    let host = host(FakeHttp::with([json_response(200, &answer)]), None);

    let (_, stdout, _) = human(
        &host,
        &[
            "skilld",
            "view",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
    );

    assert!(
        stdout.contains("⚠ The SKILL.md is gone upstream. skilld.dev keeps the last copy it read."),
        "{stdout}"
    );
}

#[test]
fn view_of_a_repository_lists_its_skills_and_the_install_all_command() {
    let http = FakeHttp::with([json_response(200, &example("repositories.get"))]);
    let host = host(http.clone(), None);

    let (exit, stdout, stderr) = run(&host, &["skilld", "view", "vercel-labs/agent-skills"]);

    assert_eq!((exit, stderr.as_str()), (0, ""));
    assert_eq!(
        http.only_request().url,
        "http://127.0.0.1:8787/api/v1/repositories/vercel-labs/agent-skills"
    );
    assert!(
        stdout.contains("Install all: skilld add vercel-labs/agent-skills\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "web-design-guidelines\tvercel-labs/agent-skills/web-design-guidelines\tvercel-labs/agent-skills\t18204\t41\t"
        ),
        "{stdout}"
    );
}

#[test]
fn view_of_an_installed_name_stays_local_and_sends_nothing() {
    let http = FakeHttp::with([]);
    let host = host(http.clone(), None);

    let (exit, _, stderr) = run(&host, &["skilld", "view", "not-installed"]);

    assert_eq!(exit, 1, "{stderr}");
    assert!(http.requests().is_empty());
    let (exit, _, stderr) = run(&host, &["skilld", "view", "not-installed", "--json"]);
    assert_eq!(exit, 2);
    assert!(stderr.contains("UNSUPPORTED_OUTPUT"), "{stderr}");
}

#[test]
fn view_rejects_global_and_skill_paths_for_a_registry_ref() {
    let http = FakeHttp::with([]);
    let host = host(http.clone(), None);

    let (exit, _, stderr) = run(
        &host,
        &[
            "skilld",
            "view",
            "vercel-labs/agent-skills/web-design-guidelines",
            "--global",
        ],
    );
    assert_eq!(exit, 2);
    assert!(
        stderr.starts_with("INVALID_SOURCE: --global applies"),
        "{stderr}"
    );

    let (exit, _, stderr) = run(
        &host,
        &["skilld", "view", "vercel-labs/agent-skills/skills/vue"],
    );
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("names a path inside the Repository"),
        "{stderr}"
    );
    assert!(http.requests().is_empty());
}

#[test]
fn a_missing_registry_skill_keeps_the_problem_code_and_detail() {
    let http = FakeHttp::with([problem(
        404,
        "NOT_FOUND",
        "No Skill vercel-labs/agent-skills/nope",
        &[],
    )]);
    let host = host(http, None);

    let (exit, stdout, stderr) = run(&host, &["skilld", "view", "vercel-labs/agent-skills/nope"]);

    assert_eq!((exit, stdout.as_str()), (1, ""));
    assert_eq!(
        stderr,
        "NOT_FOUND: No Skill vercel-labs/agent-skills/nope.\n"
    );
}

// ---------------------------------------------------------------------------
// browse
// ---------------------------------------------------------------------------

#[test]
fn browse_sends_every_filter_and_prints_one_record_per_skill() {
    let http = FakeHttp::with([json_response(200, &example("skills.browse"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, stderr) = run(
        &host,
        &[
            "skilld",
            "browse",
            "web",
            "design",
            "--owner",
            "vercel-labs",
            "--tag",
            "design",
            "--sort",
            "likes",
            "--limit",
            "1",
            "--offset",
            "0",
        ],
    );

    assert_eq!((exit, stderr.as_str()), (0, ""));
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Get);
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/browse?q=web+design&owner=vercel-labs&tag=design&sort=likes&limit=1&offset=0"
    );
    assert_eq!(header(&request, "authorization"), None);
    assert_eq!(
        stdout,
        "web-design-guidelines\tvercel-labs/agent-skills/web-design-guidelines\tvercel-labs/agent-skills\t18204\t41\tReview UI code for compliance with web interface guidelines.\t\n"
    );
}

#[test]
fn browse_without_filters_sends_no_query_and_pages_in_human_output() {
    let http = FakeHttp::with([json_response(200, &example("skills.browse"))]);
    let host = host(http.clone(), None);

    let (exit, stdout, _) = human(&host, &["skilld", "browse"]);

    assert_eq!(exit, 0);
    assert_eq!(
        http.only_request().url,
        "http://127.0.0.1:8787/api/v1/browse"
    );
    assert!(stdout.starts_with("Browse  6 Skills\n"), "{stdout}");
    assert!(
        stdout.contains(
            "• web-design-guidelines  vercel-labs/agent-skills · 18,204 stars · 41 likes"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("Run    skilld run vercel-labs/agent-skills/web-design-guidelines"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Showing 1 to 1 of 6. Add --offset 1 for the next page."),
        "{stdout}"
    );
}

#[test]
fn browse_json_is_the_list_answer() {
    let answer = example("skills.browse");
    let host = host(FakeHttp::with([json_response(200, &answer)]), None);

    let (exit, stdout, _) = run(&host, &["skilld", "browse", "--json"]);

    assert_eq!(exit, 0);
    assert_eq!(json_data(&stdout, "browse"), answer);
}

#[test]
fn an_invalid_browse_filter_is_a_usage_error() {
    let http = FakeHttp::with([problem(
        400,
        "INVALID_REQUEST",
        "tag must be a tag slug",
        &[],
    )]);
    let host = host(http, None);

    let (exit, _, stderr) = run(
        &host,
        &["skilld", "browse", "--tag", "Not A Slug", "--json"],
    );

    assert_eq!(exit, 2);
    let failure: Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(failure["_tag"], "UsageError");
    assert_eq!(failure["error"]["code"], "INVALID_REQUEST");
    assert_eq!(failure["error"]["message"], "tag must be a tag slug.");
}

#[test]
fn a_rate_limited_read_retries_then_states_the_wait() {
    let limited = || {
        problem(
            429,
            "RATE_LIMITED",
            "Too many requests",
            &[("retry-after", "30")],
        )
    };
    let http = FakeHttp::with([limited(), limited(), limited()]);
    let host = host(http.clone(), None);

    let (exit, _, stderr) = run(&host, &["skilld", "browse"]);

    assert_eq!(exit, 1);
    assert_eq!(http.requests().len(), 3, "a query retries twice");
    assert_eq!(stderr, "RATE_LIMITED: Too many requests. Retry in 30s.\n");
}

#[test]
fn a_server_failure_names_the_request_id_to_report() {
    let failure = || {
        problem(
            500,
            "INTERNAL_ERROR",
            "Something broke",
            &[("x-request-id", "req-123")],
        )
    };
    let host = host(FakeHttp::with([failure()]), None);

    let (exit, _, stderr) = run(&host, &["skilld", "trending"]);

    assert_eq!(exit, 1);
    assert_eq!(
        stderr,
        "INTERNAL_ERROR: Something broke. Retry in a minute. If it keeps failing, report request ID req-123.\n"
    );
}

#[test]
fn a_non_problem_failure_falls_back_to_the_http_status() {
    let host = host(
        FakeHttp::with([HttpResponse {
            status: 502,
            headers: BTreeMap::new(),
            body: b"bad gateway".to_vec(),
        }]),
        None,
    );

    let (exit, _, stderr) = run(&host, &["skilld", "tracks"]);

    assert_eq!(exit, 1);
    assert!(
        stderr.starts_with("SERVICE_UNAVAILABLE: the remote service returned HTTP 502."),
        "{stderr}"
    );
}

#[test]
fn an_answer_in_an_unknown_shape_asks_for_an_upgrade() {
    let host = host(
        FakeHttp::with([json_response(200, &json!({ "rows": [] }))]),
        None,
    );

    let (exit, _, stderr) = run(&host, &["skilld", "browse"]);

    assert_eq!(exit, 1);
    assert_eq!(
        stderr,
        "INVALID_RESPONSE: skilld.dev answered skills.browse in a shape this skilld release cannot read. Upgrade skilld, then run the same command again.\n"
    );
}

// ---------------------------------------------------------------------------
// trending
// ---------------------------------------------------------------------------

#[test]
fn trending_says_why_each_skill_trends() {
    let http = FakeHttp::with([json_response(200, &example("trending.list"))]);
    let host = host(http.clone(), None);

    let (exit, stdout, _) = human(
        &host,
        &["skilld", "trending", "--window", "month", "--limit", "5"],
    );

    assert_eq!(exit, 0);
    assert_eq!(
        http.only_request().url,
        "http://127.0.0.1:8787/api/v1/trending?window=month&limit=5"
    );
    assert!(stdout.starts_with("Trending this month\n"), "{stdout}");
    assert!(
        stdout.contains("Why    @ada_ships on X (3 accounts talked about it)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Post   \u{201c}web-design-guidelines catches the UI mistakes I used to catch in review.\u{201d}"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Link   https://x.com/ada_ships/status/1972000000000000000"),
        "{stdout}"
    );
}

#[test]
fn trending_plain_ends_each_record_with_its_reason() {
    let mut answer = example("trending.list");
    answer["items"][0]["signal"] = json!({
        "kind": "star-surge",
        "starGain": 312,
        "surgedOn": "2026-09-29T00:00:00.000Z",
    });
    let host = host(FakeHttp::with([json_response(200, &answer)]), None);

    let (_, stdout, _) = run(&host, &["skilld", "trending"]);

    assert!(
        stdout.ends_with("\tstar-surge:+312:2026-09-29\n"),
        "{stdout}"
    );
}

// ---------------------------------------------------------------------------
// tracks
// ---------------------------------------------------------------------------

#[test]
fn tracks_lists_every_track_with_its_command() {
    let http = FakeHttp::with([json_response(200, &example("tracks.list"))]);
    let host = host(http.clone(), None);

    let (exit, stdout, _) = run(&host, &["skilld", "tracks"]);

    assert_eq!(exit, 0);
    assert_eq!(
        http.only_request().url,
        "http://127.0.0.1:8787/api/v1/tracks"
    );
    assert_eq!(
        stdout,
        "design\tDesign and interface work\t41\tYou care how the interface looks, moves, and reads.\n"
    );
}

#[test]
fn one_track_pages_through_its_skills() {
    let http = FakeHttp::with([json_response(200, &example("tracks.get"))]);
    let host = host(http.clone(), None);

    let (exit, stdout, _) = human(
        &host,
        &[
            "skilld", "tracks", "design", "--limit", "1", "--offset", "3",
        ],
    );

    assert_eq!(exit, 0);
    assert_eq!(
        http.only_request().url,
        "http://127.0.0.1:8787/api/v1/tracks/design?limit=1&offset=3"
    );
    assert!(
        stdout.starts_with("Design and interface work  41 Skills\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Showing 4 to 4 of 41. Add --offset 4 for the next page."),
        "{stdout}"
    );
}

#[test]
fn track_paging_needs_a_track() {
    let host = host(FakeHttp::with([]), None);

    let (exit, _, stderr) = run(&host, &["skilld", "tracks", "--limit", "5"]);

    assert_eq!(exit, 2, "{stderr}");
}

// ---------------------------------------------------------------------------
// index
// ---------------------------------------------------------------------------

fn queued(stage: Value) -> Value {
    json!({
        "status": "queued",
        "id": "0f8b5c1e-3d4a-4f6b-9a2c-7e1d5b8c9a04",
        "owner": "vercel-labs",
        "repository": "agent-skills",
        "progress": stage,
    })
}

fn indexed() -> Value {
    json!({
        "status": "indexed",
        "owner": "vercel-labs",
        "repository": "agent-skills",
        "skills": example("skills.browse")["items"],
    })
}

#[test]
fn index_posts_the_repository_then_polls_until_it_is_indexed() {
    let http = FakeHttp::with([
        json_response(201, &queued(json!({ "stage": "queued" }))),
        json_response(
            200,
            &queued(json!({ "stage": "indexing", "indexed": 1, "total": 2 })),
        ),
        json_response(200, &indexed()),
    ]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, stderr) = run(
        &host,
        &[
            "skilld",
            "index",
            "https://github.com/vercel-labs/agent-skills",
        ],
    );

    assert_eq!((exit, stderr.as_str()), (0, ""));
    let requests = http.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, HttpMethod::Post);
    assert_eq!(
        requests[0].url,
        "http://127.0.0.1:8787/api/v1/index-requests"
    );
    assert_eq!(
        header(&requests[0], "content-type"),
        Some("application/json")
    );
    assert_eq!(header(&requests[0], "authorization"), None);
    assert_eq!(
        body(&requests[0]),
        json!({ "repository": "https://github.com/vercel-labs/agent-skills" })
    );
    for poll in &requests[1..] {
        assert_eq!(poll.method, HttpMethod::Get);
        assert_eq!(
            poll.url,
            "http://127.0.0.1:8787/api/v1/index-requests/0f8b5c1e-3d4a-4f6b-9a2c-7e1d5b8c9a04"
        );
    }
    assert_eq!(
        stdout,
        "web-design-guidelines\tvercel-labs/agent-skills/web-design-guidelines\tvercel-labs/agent-skills\t18204\t41\tReview UI code for compliance with web interface guidelines.\t\n"
    );
}

#[test]
fn index_json_ends_on_the_last_answer() {
    let http = FakeHttp::with([json_response(201, &indexed())]);
    let host = host(http.clone(), None);

    let (exit, stdout, _) = run(
        &host,
        &["skilld", "index", "vercel-labs/agent-skills", "--json"],
    );

    assert_eq!(exit, 0);
    assert_eq!(
        body(&http.only_request()),
        json!({ "repository": "vercel-labs/agent-skills" })
    );
    assert_eq!(json_data(&stdout, "index"), indexed());
}

#[test]
fn a_failed_index_request_is_an_operation_error_with_the_reason() {
    let http = FakeHttp::with([
        json_response(201, &queued(json!({ "stage": "checking" }))),
        json_response(
            200,
            &json!({
                "status": "failed",
                "owner": "vercel-labs",
                "repository": "agent-skills",
                "reason": "No supported SKILL.md files were found.",
            }),
        ),
    ]);
    let host = host(http, None);

    let (exit, stdout, stderr) = run(&host, &["skilld", "index", "vercel-labs/agent-skills"]);

    assert_eq!((exit, stdout.as_str()), (1, ""));
    assert_eq!(
        stderr,
        "INDEX_FAILED: skilld.dev could not index vercel-labs/agent-skills: No supported SKILL.md files were found\n"
    );
}

#[test]
fn an_index_request_that_outlasts_the_wait_says_how_to_check_again() {
    let mut responses = vec![json_response(201, &queued(json!({ "stage": "queued" })))];
    responses.extend((0..skilld_command::INDEX_POLL_ATTEMPTS).map(|_| {
        json_response(
            200,
            &queued(json!({ "stage": "indexing", "indexed": 3, "total": 6 })),
        )
    }));
    let http = FakeHttp::with(responses);
    let host = host(http.clone(), None);

    let (exit, stdout, _) = run(&host, &["skilld", "index", "vercel-labs/agent-skills"]);

    assert_eq!(exit, 0);
    assert_eq!(
        http.requests().len(),
        1 + skilld_command::INDEX_POLL_ATTEMPTS
    );
    assert_eq!(
        stdout,
        concat!(
            "skilld.dev is still indexing vercel-labs/agent-skills. It has indexed 3 of 6 Skills so far.\n",
            "Run skilld index vercel-labs/agent-skills again to check.\n",
        )
    );
}

#[test]
fn an_index_post_never_repeats_after_a_failure() {
    let http = FakeHttp::with([problem(
        503,
        "SERVICE_UNAVAILABLE",
        "Index queue paused",
        &[],
    )]);
    let host = host(http.clone(), None);

    let (exit, _, stderr) = run(&host, &["skilld", "index", "vercel-labs/agent-skills"]);

    assert_eq!(exit, 1);
    assert_eq!(http.requests().len(), 1, "a POST never retries");
    assert_eq!(
        stderr,
        "SERVICE_UNAVAILABLE: Index queue paused. Retry in a minute.\n"
    );
}

#[test]
fn index_rejects_a_value_that_names_no_repository() {
    let http = FakeHttp::with([]);
    let host = host(http.clone(), None);

    for value in [
        "vercel-labs",
        "@harlan-zw",
        "https://gitlab.com/a/b",
        "a/b/c",
    ] {
        let (exit, _, stderr) = run(&host, &["skilld", "index", value]);
        assert_eq!(exit, 2, "{value}: {stderr}");
        assert!(stderr.contains("OWNER/REPOSITORY"), "{value}: {stderr}");
    }
    assert!(http.requests().is_empty());
}

#[allow(dead_code)]
fn unused_helpers() {
    let _ = empty_response();
}
