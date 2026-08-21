use skilld_command::{CommandError, Host, OutputContext, run_with_output};
use skilld_core::{
    InstallScope, InstallSource, SearchResponse, SearchResult, SourceProvider, SourceRequest,
    SourceSelector,
};
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

#[derive(Clone)]
struct SearchHost {
    response: Result<SearchResponse, CommandError>,
}

impl Host for SearchHost {
    fn list(&self, _scope: InstallScope) -> Result<Vec<String>, CommandError> {
        unreachable!("list is outside this test")
    }

    fn install(
        &self,
        _source: InstallSource,
        _scope: InstallScope,
    ) -> Result<String, CommandError> {
        unreachable!("install is outside this test")
    }

    fn search(&self, _query: &str) -> Result<SearchResponse, CommandError> {
        self.response.clone()
    }
}

fn response() -> SearchResponse {
    SearchResponse {
        items: vec![SearchResult {
            name: "grill-me".to_owned(),
            description: Some(
                "A focused Skill description that wraps cleanly on a narrow terminal.".to_owned(),
            ),
            source: SourceRequest {
                provider: SourceProvider::Github,
                owner: "mattpocock".to_owned(),
                repository: "skills".to_owned(),
                selector: SourceSelector::NamedSkill {
                    name: "grill-me".to_owned(),
                },
                r#ref: None,
            },
            stargazer_count: 227_068,
        }],
        total: 14,
    }
}

fn run(args: &[&str], context: OutputContext) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(
        args,
        &SearchHost {
            response: Ok(response()),
        },
        context,
        &mut stdout,
        &mut stderr,
    );
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn json_search_returns_one_versioned_document() {
    let (exit, stdout, stderr) = run(
        &["skilld", "search", "grill", "--json"],
        OutputContext::auto(true, true, false, false, false, 80),
    );
    let global_form = run(
        &["skilld", "--json", "search", "grill"],
        OutputContext::auto(true, true, false, false, false, 80),
    );

    assert_eq!(exit, 0);
    assert!(stderr.is_empty());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout).unwrap(),
        serde_json::json!({
            "schemaVersion": 1,
            "_tag": "Success",
            "command": "search",
            "data": {
                "query": "grill",
                "items": [{
                    "name": "grill-me",
                    "selector": "skilld:mattpocock/skills/grill-me",
                    "description": "A focused Skill description that wraps cleanly on a narrow terminal.",
                    "stargazerCount": 227068
                }],
                "total": 14
            },
            "notices": []
        })
    );
    assert!(stdout.ends_with('\n'));
    assert_eq!(global_form, (exit, stdout, stderr));
}

#[test]
fn non_terminal_and_ci_output_are_stable_plain_records() {
    let expected = concat!(
        "grill-me\tskilld:mattpocock/skills/grill-me\t227068\t",
        "A focused Skill description that wraps cleanly on a narrow terminal.\n"
    );

    let non_terminal = run(
        &["skilld", "search", "grill"],
        OutputContext::auto(false, false, false, false, false, 80),
    );
    let ci_with_tty = run(
        &["skilld", "search", "grill"],
        OutputContext::auto(true, false, true, false, false, 120),
    );

    assert_eq!(non_terminal, (0, expected.to_owned(), String::new()));
    assert_eq!(ci_with_tty, non_terminal);
}

#[test]
fn detected_agent_output_is_plain_even_with_a_tty() {
    let agent_with_tty = run(
        &["skilld", "search", "grill"],
        OutputContext::auto(true, true, false, false, false, 120),
    );

    assert_eq!(
        agent_with_tty,
        (
            0,
            concat!(
                "grill-me\tskilld:mattpocock/skills/grill-me\t227068\t",
                "A focused Skill description that wraps cleanly on a narrow terminal.\n"
            )
            .to_owned(),
            String::new()
        )
    );
}

#[test]
fn explicit_plain_overrides_a_human_terminal() {
    let (_, stdout, stderr) = run(
        &["skilld", "search", "grill", "--plain"],
        OutputContext::auto(true, false, false, false, false, 120),
    );

    assert!(stderr.is_empty());
    assert_eq!(
        stdout,
        concat!(
            "grill-me\tskilld:mattpocock/skills/grill-me\t227068\t",
            "A focused Skill description that wraps cleanly on a narrow terminal.\n"
        )
    );
}

#[test]
fn plain_search_escapes_record_delimiters() {
    let mut response = response();
    response.items[0].description = Some("first\nsecond\tvalue\u{1b}".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run_with_output(
        ["skilld", "search", "grill", "--plain"],
        &SearchHost {
            response: Ok(response),
        },
        OutputContext::auto(true, false, false, false, false, 80),
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert!(stderr.is_empty());
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "grill-me\tskilld:mattpocock/skills/grill-me\t227068\tfirst\\nsecond\\tvalue\\u{001B}\n"
    );
}

#[test]
fn human_search_is_polished_and_respects_terminal_width() {
    let (_, stdout, stderr) = run(
        &["skilld", "search", "grill"],
        OutputContext::auto(true, false, false, true, false, 40),
    );

    assert!(stderr.is_empty());
    assert!(stdout.contains("Skill search"));
    assert!(stdout.contains("1 of 14 Skills"));
    assert!(stdout.contains("227,068 stars"));
    assert!(stdout.contains("skilld:mattpocock/skills/grill-me"));
    assert!(
        stdout
            .lines()
            .all(|line| UnicodeWidthStr::width(line) <= 40)
    );
    assert!(!stdout.contains('\u{1b}'));
}

#[test]
fn human_search_uses_display_cells_and_sanitizes_terminal_controls() {
    let mut response = response();
    response.items[0].name = "\u{6280}\u{80fd}\u{1f642}".to_owned();
    response.items[0].description =
        Some("\u{6f22}\u{5b57}\u{1f642} cafe\u{301} \u{1b}[31mred".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run_with_output(
        ["skilld", "search", "grill"],
        &SearchHost {
            response: Ok(response),
        },
        OutputContext::auto(true, false, false, true, false, 20),
        &mut stdout,
        &mut stderr,
    );
    let stdout = String::from_utf8(stdout).unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("\u{6280}\u{80fd}\u{1f642}"));
    assert!(stdout.contains("\u{6f22}\u{5b57}\u{1f642}"));
    assert!(stdout.contains("cafe\u{301}"));
    assert!(!stdout.contains('\u{1b}'));
    assert!(
        stdout
            .lines()
            .all(|line| UnicodeWidthStr::width(line) <= 20)
    );
}

#[test]
fn broken_pipe_is_a_successful_search_exit() {
    let mut stdout = BrokenPipeWriter;
    let mut stderr = Vec::new();

    let result = run_with_output(
        ["skilld", "search", "grill", "--plain"],
        &SearchHost {
            response: Ok(response()),
        },
        OutputContext::Plain,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert!(stderr.is_empty());
}

#[test]
fn color_capabilities_change_ansi_only() {
    let (_, colored, _) = run(
        &["skilld", "search", "grill"],
        OutputContext::auto(true, false, false, false, false, 100),
    );
    let (_, no_color, _) = run(
        &["skilld", "search", "grill"],
        OutputContext::auto(true, false, false, true, false, 100),
    );
    let (_, dumb_terminal, _) = run(
        &["skilld", "search", "grill"],
        OutputContext::auto(true, false, false, false, true, 100),
    );

    assert!(colored.contains('\u{1b}'));
    assert_eq!(strip_ansi(&colored), no_color);
    assert_eq!(no_color, dumb_terminal);
}

#[test]
fn conflicting_output_flags_fail_before_search() {
    let (exit, stdout, stderr) = run(
        &["skilld", "search", "grill", "--json", "--plain"],
        OutputContext::auto(false, false, false, false, false, 80),
    );

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn json_search_errors_are_tagged_and_written_to_stderr() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(
        ["skilld", "search", "grill", "--json"],
        &SearchHost {
            response: Err(CommandError::service("Skill search timed out")),
        },
        OutputContext::auto(false, false, false, false, false, 80),
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 2);
    assert!(stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&stderr).unwrap(),
        serde_json::json!({
            "schemaVersion": 1,
            "_tag": "OperationError",
            "error": {
                "code": "SERVICE_UNAVAILABLE",
                "message": "Skill search timed out"
            }
        })
    );
}

fn strip_ansi(value: &str) -> String {
    [
        "\u{1b}[1m",
        "\u{1b}[2m",
        "\u{1b}[36m",
        "\u{1b}[33m",
        "\u{1b}[0m",
    ]
    .into_iter()
    .fold(value.to_owned(), |value, code| value.replace(code, ""))
}

struct BrokenPipeWriter;

impl Write for BrokenPipeWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
