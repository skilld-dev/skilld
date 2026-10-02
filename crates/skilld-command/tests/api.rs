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
    AccountProvider, Cancellation, CommandError, CommandPlatform, HttpAdapter, HttpMethod,
    HttpRequest, HttpResponse, LocalHost, NativeRemoteConfig, OutputContext, SecretValue,
    SkilldRemote, Sleeper, TokenProvider, run_with_output,
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

// ---------------------------------------------------------------------------
// curators and collections
// ---------------------------------------------------------------------------

#[test]
fn curators_lists_each_curator_with_a_view_command() {
    let http = FakeHttp::with([json_response(200, &example("curators.list"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, stderr) = run(&host, &["skilld", "curators", "--limit", "1"]);

    assert_eq!((exit, stderr.as_str()), (0, ""));
    let request = http.only_request();
    assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/curators?limit=1");
    assert_eq!(header(&request, "authorization"), None);
    assert_eq!(stdout, "@harlan-zw\tHarlan Wilton\t2\n");
}

#[test]
fn view_of_a_curator_lists_their_collections() {
    let http = FakeHttp::with([json_response(200, &example("curators.get"))]);
    let host = host(http.clone(), None);

    let (exit, stdout, _) = human(&host, &["skilld", "view", "@harlan-zw"]);

    assert_eq!(exit, 0);
    assert_eq!(
        http.only_request().url,
        "http://127.0.0.1:8787/api/v1/curators/harlan-zw"
    );
    assert!(
        stdout.starts_with("@harlan-zw (Harlan Wilton)\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Install all     : skilld add @harlan-zw\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "• Design Engineering Essentials  @harlan-zw/design-engineering-essentials · 8 Skills"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("View   skilld view @harlan-zw/design-engineering-essentials"),
        "{stdout}"
    );
}

#[test]
fn view_of_a_collection_shows_the_reason_for_each_skill() {
    let answer = example("collections.get");
    let http = FakeHttp::with([json_response(200, &answer)]);
    let host = host(http.clone(), None);

    let (exit, stdout, _) = human(
        &host,
        &["skilld", "view", "@harlan-zw/design-engineering-essentials"],
    );

    assert_eq!(exit, 0);
    assert_eq!(
        http.only_request().url,
        "http://127.0.0.1:8787/api/v1/collections/harlan-zw/design-engineering-essentials"
    );
    assert!(
        stdout.contains("Curator    : @harlan-zw (Harlan Wilton)\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Watch      : skilld watch @harlan-zw/design-engineering-essentials\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "Why    Checks interface code against a published list of rules before review."
        ),
        "{stdout}"
    );
    let (_, stdout, _) = run(
        &host_with([json_response(200, &answer)]),
        &[
            "skilld",
            "view",
            "@harlan-zw/design-engineering-essentials",
            "--json",
        ],
    );
    assert_eq!(json_data(&stdout, "view"), answer);
}

fn host_with(responses: impl IntoIterator<Item = HttpResponse>) -> LocalHost {
    host(FakeHttp::with(responses), Some(TOKEN))
}

// ---------------------------------------------------------------------------
// account
// ---------------------------------------------------------------------------

fn assert_bearer(request: &HttpRequest) {
    assert_eq!(
        header(request, "authorization"),
        Some(format!("Bearer {TOKEN}").as_str()),
        "{} {}",
        request.url,
        "sends the stored token"
    );
}

#[test]
fn account_shows_every_setting_as_the_key_account_set_takes() {
    let http = FakeHttp::with([json_response(200, &example("account.get"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, stderr) = run(&host, &["skilld", "account"]);

    assert_eq!((exit, stderr.as_str()), (0, ""));
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Get);
    assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/account");
    assert_bearer(&request);
    assert_eq!(
        stdout,
        concat!(
            "login=harlan-zw\n",
            "name=Harlan Wilton\n",
            "email=harlan@example.com\n",
            "digest=on\n",
            "weekly=on\n",
            "likes-public=on\n",
            "repository-indexing=on\n",
            "stars-imported=2026-09-21\n",
        )
    );
}

#[test]
fn every_account_command_without_a_sign_in_stops_before_any_request() {
    let http = FakeHttp::with([]);
    let host = host(http.clone(), None);

    for command in [
        vec!["skilld", "account"],
        vec!["skilld", "account", "set", "digest", "off"],
        vec!["skilld", "account", "scan"],
        vec!["skilld", "account", "unpublish", "harlan-zw/skills"],
        vec![
            "skilld",
            "like",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
        vec![
            "skilld",
            "unlike",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
        vec!["skilld", "likes"],
        vec!["skilld", "watch", "vercel-labs/agent-skills"],
        vec![
            "skilld",
            "watch",
            "@harlan-zw/design-engineering-essentials",
        ],
        vec!["skilld", "unwatch", "vercel-labs/agent-skills"],
        vec!["skilld", "watches"],
        vec!["skilld", "changes"],
        vec!["skilld", "stars"],
        vec!["skilld", "stars", "import"],
        vec![
            "skilld",
            "collection",
            "create",
            "picks",
            "--title",
            "Picks",
        ],
        vec![
            "skilld",
            "collection",
            "add",
            "@harlan-zw/picks",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
        vec![
            "skilld",
            "collection",
            "remove",
            "@harlan-zw/picks",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
        vec!["skilld", "tokens"],
        vec!["skilld", "tokens", "create", "--label", "CI"],
        vec!["skilld", "tokens", "revoke", "412"],
    ] {
        let (exit, stdout, stderr) = run(&host, &command);
        assert_eq!((exit, stdout.as_str()), (1, ""), "{command:?}");
        assert_eq!(
            stderr,
            "AUTH_REQUIRED: This command needs a skilld.dev account. Run skilld auth login, then run the same command again.\n",
            "{command:?}"
        );
    }
    assert!(http.requests().is_empty());

    let (_, _, stderr) = run(&host, &["skilld", "account", "--json"]);
    let failure: Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(failure["_tag"], "OperationError");
    assert_eq!(failure["error"]["code"], "AUTH_REQUIRED");
}

#[test]
fn a_rejected_token_names_the_login_step() {
    let http = FakeHttp::with([problem(401, "AUTH_REQUIRED", "The token expired", &[])]);
    let host = host(http, Some(TOKEN));

    let (exit, _, stderr) = run(&host, &["skilld", "likes"]);

    assert_eq!(exit, 1);
    assert_eq!(
        stderr,
        "AUTH_REQUIRED: The token expired. Run skilld auth login, then run the same command again.\n"
    );
}

#[test]
fn account_set_patches_exactly_one_setting() {
    for (key, value, expected) in [
        ("digest", "off", json!({ "digest": false })),
        ("weekly", "on", json!({ "weekly": true })),
        ("likes-public", "off", json!({ "likesPublic": false })),
        (
            "repository-indexing",
            "on",
            json!({ "repositoryIndexing": true }),
        ),
        (
            "email",
            "you@example.com",
            json!({ "email": "you@example.com" }),
        ),
    ] {
        let http = FakeHttp::with([json_response(200, &example("account.update"))]);
        let host = host(http.clone(), Some(TOKEN));

        let (exit, stdout, stderr) = run(&host, &["skilld", "account", "set", key, value]);

        assert_eq!((exit, stderr.as_str()), (0, ""), "{key}");
        let request = http.only_request();
        assert_eq!(request.method, HttpMethod::Patch, "{key}");
        assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/account", "{key}");
        assert_bearer(&request);
        assert_eq!(body(&request), expected, "{key}");
        assert!(stdout.starts_with(&format!("Set {key}.\n")), "{stdout}");
    }
}

#[test]
fn account_set_refuses_an_unknown_key_or_value_before_any_request() {
    let http = FakeHttp::with([]);
    let host = host(http.clone(), Some(TOKEN));

    for (key, value, message) in [
        (
            "digest",
            "daily",
            "INVALID_REQUEST: digest takes on or off\n",
        ),
        (
            "repo-indexing",
            "on",
            "INVALID_REQUEST: unknown account setting: repo-indexing. Use email, digest, weekly, likes-public, or repository-indexing.\n",
        ),
        (
            "email",
            "nobody",
            "INVALID_REQUEST: email takes one address, such as you@example.com\n",
        ),
    ] {
        let (exit, _, stderr) = run(&host, &["skilld", "account", "set", key, value]);
        assert_eq!((exit, stderr.as_str()), (2, message), "{key}");
    }
    assert!(http.requests().is_empty());
}

#[test]
fn a_settings_patch_never_repeats() {
    let http = FakeHttp::with([problem(503, "SERVICE_UNAVAILABLE", "Try later", &[])]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, _, _) = run(&host, &["skilld", "account", "set", "weekly", "off"]);

    assert_eq!(exit, 1);
    assert_eq!(http.requests().len(), 1, "a PATCH never retries");
}

#[test]
fn account_scan_posts_once_and_reports_a_partial_scan() {
    let http = FakeHttp::with([json_response(
        200,
        &json!({
            "outcome": "partial",
            "repositoriesFound": 3,
            "repositoriesIndexed": 1,
            "repositoriesFailed": 0,
        }),
    )]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(&host, &["skilld", "account", "scan"]);

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Post);
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/account/repositories/scan"
    );
    assert!(request.body.is_empty());
    assert_bearer(&request);
    assert_eq!(
        stdout,
        concat!(
            "Scanned some of your Repositories. 3 Repositories with Skills, 1 indexed, 0 failed.\n",
            "Run skilld account scan again later to read the rest.\n",
        )
    );
}

#[test]
fn account_unpublish_deletes_one_repository() {
    let http = FakeHttp::with([empty_response()]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(
        &host,
        &[
            "skilld",
            "account",
            "unpublish",
            "harlan-zw/skills",
            "--json",
        ],
    );

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Delete);
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/account/repositories/harlan-zw/skills"
    );
    assert_bearer(&request);
    assert_eq!(json_data(&stdout, "account unpublish"), Value::Null);
}

#[test]
fn unpublishing_another_owner_keeps_the_forbidden_code() {
    let http = FakeHttp::with([problem(
        403,
        "FORBIDDEN",
        "The Owner must be your own login",
        &[],
    )]);
    let host = host(http, Some(TOKEN));

    let (exit, _, stderr) = run(
        &host,
        &["skilld", "account", "unpublish", "vercel-labs/agent-skills"],
    );

    assert_eq!(exit, 1);
    assert_eq!(
        stderr,
        "FORBIDDEN: The Owner must be your own login. Your account cannot do this.\n"
    );
}

// ---------------------------------------------------------------------------
// likes
// ---------------------------------------------------------------------------

#[test]
fn like_puts_the_skill_and_retries_an_unavailable_service() {
    let http = FakeHttp::with([
        problem(503, "SERVICE_UNAVAILABLE", "Busy", &[]),
        empty_response(),
    ]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, stderr) = run(
        &host,
        &[
            "skilld",
            "like",
            "vercel-labs/agent-skills/web-design-guidelines",
            "--json",
        ],
    );

    assert_eq!((exit, stderr.as_str()), (0, ""));
    let requests = http.requests();
    assert_eq!(requests.len(), 2, "an idempotent PUT retries");
    for request in &requests {
        assert_eq!(request.method, HttpMethod::Put);
        assert_eq!(
            request.url,
            "http://127.0.0.1:8787/api/v1/account/likes/vercel-labs/agent-skills/web-design-guidelines"
        );
        assert!(request.body.is_empty());
        assert_bearer(request);
    }
    assert_eq!(json_data(&stdout, "like"), Value::Null);
}

#[test]
fn unlike_deletes_the_like() {
    let http = FakeHttp::with([empty_response()]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(
        &host,
        &[
            "skilld",
            "unlike",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
    );

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Delete);
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/account/likes/vercel-labs/agent-skills/web-design-guidelines"
    );
    assert_bearer(&request);
    assert_eq!(
        stdout,
        "Removed your like of vercel-labs/agent-skills/web-design-guidelines.\n"
    );
}

#[test]
fn like_takes_only_one_skill_selector() {
    let http = FakeHttp::with([]);
    let host = host(http.clone(), Some(TOKEN));

    for value in [
        "vercel-labs/agent-skills",
        "@harlan-zw",
        "web-design-guidelines",
    ] {
        let (exit, _, stderr) = run(&host, &["skilld", "like", value]);
        assert_eq!(exit, 2, "{value}");
        assert!(
            stderr.contains("OWNER/REPOSITORY/SKILL"),
            "{value}: {stderr}"
        );
    }
    assert!(http.requests().is_empty());
}

#[test]
fn likes_lists_your_likes_with_the_day_you_liked_each() {
    let http = FakeHttp::with([json_response(200, &example("likes.list"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(&host, &["skilld", "likes", "--limit", "1"]);

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/account/likes?limit=1"
    );
    assert_bearer(&request);
    assert!(stdout.ends_with("\t2026-09-12\n"), "{stdout}");
}

#[test]
fn likes_of_a_curator_reads_their_public_list_without_a_token() {
    let http = FakeHttp::with([json_response(200, &example("curators.likes"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = human(&host, &["skilld", "likes", "@harlan-zw", "--offset", "0"]);

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/curators/harlan-zw/likes?offset=0"
    );
    assert_eq!(header(&request, "authorization"), None);
    assert!(
        stdout.starts_with("Liked by @harlan-zw  12 Skills\n"),
        "{stdout}"
    );
}

// ---------------------------------------------------------------------------
// watches
// ---------------------------------------------------------------------------

#[test]
fn watch_puts_a_repository_or_a_collection() {
    let http = FakeHttp::with([
        empty_response(),
        json_response(200, &example("collections.watch")),
    ]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, repository, _) = run(&host, &["skilld", "watch", "vercel-labs/agent-skills"]);
    assert_eq!(exit, 0);
    let (exit, collection, _) = run(
        &host,
        &[
            "skilld",
            "watch",
            "@harlan-zw/design-engineering-essentials",
        ],
    );
    assert_eq!(exit, 0);

    let requests = http.requests();
    assert_eq!(requests[0].method, HttpMethod::Put);
    assert_eq!(
        requests[0].url,
        "http://127.0.0.1:8787/api/v1/account/watches/vercel-labs/agent-skills"
    );
    assert_eq!(requests[1].method, HttpMethod::Put);
    assert_eq!(
        requests[1].url,
        "http://127.0.0.1:8787/api/v1/collections/harlan-zw/design-engineering-essentials/watch"
    );
    requests.iter().for_each(assert_bearer);
    assert!(
        repository.starts_with("Watching vercel-labs/agent-skills.\n"),
        "{repository}"
    );
    assert!(
        collection
            .starts_with("Watching 8 Repositories of @harlan-zw/design-engineering-essentials.\n"),
        "{collection}"
    );
}

#[test]
fn unwatch_deletes_a_repository_watch_and_explains_a_collection() {
    let http = FakeHttp::with([empty_response()]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, _, _) = run(&host, &["skilld", "unwatch", "vercel-labs/agent-skills"]);
    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Delete);
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/account/watches/vercel-labs/agent-skills"
    );
    assert_bearer(&request);

    let (exit, _, stderr) = run(&host, &["skilld", "unwatch", "@harlan-zw/picks"]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("Run skilld watches, then skilld unwatch each"),
        "{stderr}"
    );
    assert_eq!(http.requests().len(), 1);
}

#[test]
fn watches_lists_each_watch_with_its_reason() {
    let http = FakeHttp::with([json_response(200, &example("watches.list"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = human(&host, &["skilld", "watches", "--limit", "1"]);

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/account/watches?limit=1"
    );
    assert_bearer(&request);
    assert!(
        stdout.contains("• vercel-labs/agent-skills  since 2026-09-12"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Why   you liked one of its Skills"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Stop  skilld unwatch vercel-labs/agent-skills"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Showing 1 to 1 of 6. Add --offset 1 for the next page."),
        "{stdout}"
    );
}

// ---------------------------------------------------------------------------
// changes
// ---------------------------------------------------------------------------

#[test]
fn changes_sends_since_as_a_timestamp_and_names_the_next_since() {
    let http = FakeHttp::with([json_response(200, &example("changes.list"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = human(&host, &["skilld", "changes", "--since", "2026-09-01"]);

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/account/changes?since=2026-09-01T00%3A00%3A00Z"
    );
    assert_bearer(&request);
    assert!(
        stdout.contains("Changed  2 changes on 2026-09-28"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Commit   docs: add focus ring rules"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "Run skilld changes --since 2026-10-01T09:00:00.000Z next time to read only newer changes."
        ),
        "{stdout}"
    );
}

#[test]
fn changes_without_since_takes_the_server_window_and_rejects_a_bad_date() {
    let http = FakeHttp::with([json_response(200, &example("changes.list"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(&host, &["skilld", "changes", "--json"]);
    assert_eq!(exit, 0);
    assert_eq!(
        http.only_request().url,
        "http://127.0.0.1:8787/api/v1/account/changes"
    );
    assert_eq!(json_data(&stdout, "changes"), example("changes.list"));

    let (exit, _, stderr) = run(&host, &["skilld", "changes", "--since", "last week"]);
    assert_eq!(exit, 2);
    assert!(
        stderr.starts_with("INVALID_REQUEST: --since takes a date"),
        "{stderr}"
    );
    assert_eq!(http.requests().len(), 1);
}

// ---------------------------------------------------------------------------
// stars
// ---------------------------------------------------------------------------

#[test]
fn stars_lists_starred_repositories_with_skills() {
    let http = FakeHttp::with([json_response(200, &example("stars.list"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(&host, &["skilld", "stars"]);

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/account/stars");
    assert_bearer(&request);
    assert_eq!(stdout, "vercel-labs/agent-skills\t6\toff\t2026-08-30\n");
}

#[test]
fn stars_import_posts_every_page_until_none_is_next() {
    let http = FakeHttp::with([
        json_response(
            200,
            &json!({ "page": 1, "nextPage": 2, "imported": 4, "withSkills": 2, "importedAt": null }),
        ),
        json_response(
            200,
            &json!({ "page": 2, "nextPage": null, "imported": 5, "withSkills": 3, "importedAt": "2026-10-01T07:45:00.000Z" }),
        ),
    ]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(&host, &["skilld", "stars", "import"]);

    assert_eq!(exit, 0);
    let requests = http.requests();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.url,
            "http://127.0.0.1:8787/api/v1/account/stars/import"
        );
        assert_bearer(request);
    }
    assert_eq!(body(&requests[0]), json!({}));
    assert_eq!(body(&requests[1]), json!({ "page": 2 }));
    assert_eq!(
        stdout,
        "Imported 5 starred Repositories. 3 hold Skills.\nRun skilld stars to list them.\n"
    );
}

#[test]
fn stars_import_stops_at_the_page_limit() {
    let page = |page: u32| {
        json_response(
            200,
            &json!({ "page": page, "nextPage": page + 1, "imported": page, "withSkills": 0, "importedAt": null }),
        )
    };
    let http = FakeHttp::with((1..=12).map(page));
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(&host, &["skilld", "stars", "import"]);

    assert_eq!(exit, 0);
    assert_eq!(http.requests().len(), 10);
    assert!(
        stdout.contains("skilld.dev stopped before the last page of your stars."),
        "{stdout}"
    );
}

#[test]
fn stars_paging_flags_conflict_with_import() {
    let host = host(FakeHttp::with([]), Some(TOKEN));

    let (exit, _, _) = run(&host, &["skilld", "stars", "--limit", "5", "import"]);

    assert_eq!(exit, 2);
}

// ---------------------------------------------------------------------------
// collection
// ---------------------------------------------------------------------------

#[test]
fn collection_create_posts_the_slug_title_and_description() {
    let http = FakeHttp::with([json_response(201, &example("collections.create"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, stderr) = run(
        &host,
        &[
            "skilld",
            "collection",
            "create",
            "design-engineering-essentials",
            "--title",
            "Design Engineering Essentials",
            "--description",
            "The Skills I give an agent before it touches interface code.",
        ],
    );

    assert_eq!((exit, stderr.as_str()), (0, ""));
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Post);
    assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/collections");
    assert_bearer(&request);
    assert_eq!(
        body(&request),
        json!({
            "slug": "design-engineering-essentials",
            "title": "Design Engineering Essentials",
            "description": "The Skills I give an agent before it touches interface code.",
        })
    );
    assert_eq!(
        stdout,
        concat!(
            "Created collection @harlan-zw/design-engineering-essentials.\n",
            "Add a Skill with skilld collection add @harlan-zw/design-engineering-essentials OWNER/REPOSITORY/SKILL.\n",
        )
    );
}

#[test]
fn a_taken_collection_slug_keeps_the_conflict_code() {
    let http = FakeHttp::with([problem(
        409,
        "CONFLICT",
        "You already have a collection picks",
        &[],
    )]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, _, stderr) = run(
        &host,
        &[
            "skilld",
            "collection",
            "create",
            "picks",
            "--title",
            "Picks",
        ],
    );

    assert_eq!(exit, 1);
    assert_eq!(http.requests().len(), 1, "a POST never retries");
    assert_eq!(stderr, "CONFLICT: You already have a collection picks.\n");
}

#[test]
fn collection_add_puts_the_skill_with_its_reason() {
    let http = FakeHttp::with([
        json_response(200, &example("collections.skills.add")),
        json_response(200, &example("collections.skills.add")),
    ]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(
        &host,
        &[
            "skilld",
            "collection",
            "add",
            "@harlan-zw/design-engineering-essentials",
            "vercel-labs/agent-skills/web-design-guidelines",
            "--reason",
            "Checks interface code against a published list of rules before review.",
        ],
    );
    assert_eq!(exit, 0);
    let (exit, _, _) = run(
        &host,
        &[
            "skilld",
            "collection",
            "add",
            "@harlan-zw/design-engineering-essentials",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
    );
    assert_eq!(exit, 0);

    let requests = http.requests();
    for request in &requests {
        assert_eq!(request.method, HttpMethod::Put);
        assert_eq!(
            request.url,
            "http://127.0.0.1:8787/api/v1/collections/harlan-zw/design-engineering-essentials/skills/vercel-labs/agent-skills/web-design-guidelines"
        );
        assert_bearer(request);
    }
    assert_eq!(
        body(&requests[0]),
        json!({ "reason": "Checks interface code against a published list of rules before review." })
    );
    assert_eq!(
        body(&requests[1]),
        json!({}),
        "no --reason keeps the stored reason"
    );
    assert!(
        stdout.starts_with(
            "Added vercel-labs/agent-skills/web-design-guidelines to @harlan-zw/design-engineering-essentials.\n"
        ),
        "{stdout}"
    );
}

#[test]
fn collection_remove_deletes_the_entry() {
    let http = FakeHttp::with([empty_response()]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(
        &host,
        &[
            "skilld",
            "collection",
            "remove",
            "@harlan-zw/picks",
            "vercel-labs/agent-skills/web-design-guidelines",
            "--json",
        ],
    );

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Delete);
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/collections/harlan-zw/picks/skills/vercel-labs/agent-skills/web-design-guidelines"
    );
    assert_bearer(&request);
    assert_eq!(json_data(&stdout, "collection remove"), Value::Null);
}

#[test]
fn collection_commands_check_both_refs_first() {
    let http = FakeHttp::with([]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, _, stderr) = run(
        &host,
        &[
            "skilld",
            "collection",
            "add",
            "harlan-zw/picks",
            "vercel-labs/agent-skills/web-design-guidelines",
        ],
    );
    assert_eq!(exit, 2);
    assert!(stderr.contains("@LOGIN/SLUG"), "{stderr}");
    let (exit, _, stderr) = run(
        &host,
        &[
            "skilld",
            "collection",
            "remove",
            "@harlan-zw/picks",
            "vercel-labs/agent-skills",
        ],
    );
    assert_eq!(exit, 2);
    assert!(stderr.contains("OWNER/REPOSITORY/SKILL"), "{stderr}");
    assert!(http.requests().is_empty());
}

// ---------------------------------------------------------------------------
// tokens
// ---------------------------------------------------------------------------

#[test]
fn tokens_lists_each_token_without_a_secret() {
    let http = FakeHttp::with([json_response(200, &example("tokens.list"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(&host, &["skilld", "tokens"]);

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/account/tokens");
    assert_bearer(&request);
    assert_eq!(
        stdout,
        "412\tCI deploy\tpat\t2026-10-01\t2026-12-01\tcurrent\n"
    );
}

#[test]
fn tokens_create_prints_the_secret_once_with_a_warning() {
    let http = FakeHttp::with([json_response(201, &example("tokens.create"))]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(
        &host,
        &[
            "skilld",
            "tokens",
            "create",
            "--label",
            "CI deploy",
            "--ttl-days",
            "90",
        ],
    );

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Post);
    assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/account/tokens");
    assert_bearer(&request);
    assert_eq!(
        body(&request),
        json!({ "label": "CI deploy", "ttlDays": 90 })
    );
    assert_eq!(
        stdout,
        concat!(
            "Created token 412 (CI deploy).\n",
            "Token: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOjEsInRpZCI6NDEyfQ.c2lnbmF0dXJl\n",
            "Expires: 2026-12-31\n",
            "Copy the token now. skilld.dev never shows it again.\n",
            "Send it as Authorization: Bearer TOKEN.\n",
        )
    );
}

#[test]
fn tokens_revoke_deletes_one_token_by_id() {
    let http = FakeHttp::with([empty_response()]);
    let host = host(http.clone(), Some(TOKEN));

    let (exit, stdout, _) = run(&host, &["skilld", "tokens", "revoke", "412"]);

    assert_eq!(exit, 0);
    let request = http.only_request();
    assert_eq!(request.method, HttpMethod::Delete);
    assert_eq!(
        request.url,
        "http://127.0.0.1:8787/api/v1/account/tokens/412"
    );
    assert_bearer(&request);
    assert_eq!(stdout, "Revoked token 412. It stopped working at once.\n");

    let (exit, _, _) = run(&host, &["skilld", "tokens", "revoke", "not-a-number"]);
    assert_eq!(exit, 2);
    assert_eq!(http.requests().len(), 1);
}

// ---------------------------------------------------------------------------
// auth status
// ---------------------------------------------------------------------------

struct SignedIn(bool);

impl AccountProvider for SignedIn {
    fn status(&self) -> Result<bool, CommandError> {
        Ok(self.0)
    }

    fn has_account(&self) -> Result<bool, CommandError> {
        Ok(self.0)
    }

    fn login(&self) -> Result<(), CommandError> {
        Ok(())
    }

    fn logout(&self) -> Result<(), CommandError> {
        Ok(())
    }
}

#[test]
fn auth_status_names_the_signed_in_login() {
    let http = FakeHttp::with([json_response(200, &example("account.get"))]);
    let host = host(http.clone(), Some(TOKEN)).with_account_provider(Arc::new(SignedIn(true)));

    let (exit, stdout, _) = run(&host, &["skilld", "auth", "status"]);

    assert_eq!(
        (exit, stdout.as_str()),
        (0, "Authenticated as @harlan-zw.\n")
    );
    let request = http.only_request();
    assert_eq!(request.url, "http://127.0.0.1:8787/api/v1/account");
    assert_bearer(&request);
}

#[test]
fn auth_status_reports_a_rejected_sign_in_and_skips_the_api_when_signed_out() {
    let http = FakeHttp::with([problem(401, "AUTH_REQUIRED", "The token was revoked", &[])]);
    let host = host(http.clone(), Some(TOKEN)).with_account_provider(Arc::new(SignedIn(true)));

    let (exit, stdout, _) = run(&host, &["skilld", "auth", "status"]);
    assert_eq!(exit, 0);
    assert_eq!(
        stdout,
        "skilld.dev rejected the stored sign-in.\nRun skilld auth login to sign in again.\n"
    );

    let signed_out = FakeHttp::with([]);
    let host = host_from(signed_out.clone()).with_account_provider(Arc::new(SignedIn(false)));
    let (exit, stdout, _) = run(&host, &["skilld", "auth", "status"]);
    assert_eq!((exit, stdout.as_str()), (0, "Not authenticated.\n"));
    assert!(signed_out.requests().is_empty());
}

#[test]
fn auth_status_keeps_the_local_answer_when_skilld_dev_is_down() {
    let http = FakeHttp::with([]);
    let host = host(http, Some(TOKEN)).with_account_provider(Arc::new(SignedIn(true)));

    let (exit, stdout, _) = run(&host, &["skilld", "auth", "status"]);

    assert_eq!(exit, 0);
    assert!(
        stdout.starts_with("Authenticated.\nskilld.dev did not name the account: "),
        "{stdout}"
    );
}

fn host_from(http: Arc<FakeHttp>) -> LocalHost {
    host(http, None)
}
