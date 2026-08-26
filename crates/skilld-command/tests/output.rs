use skilld_command::{CommandError, CommandPlatform, Host, OutputContext, run_with_output};
use skilld_core::{
    InstallScope, InstallSource, RemoteError, SearchResponse, SearchResult, SourceProvider,
    SourceRequest, SourceSelector,
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

const PLAIN: OutputContext = OutputContext::Plain {
    platform: CommandPlatform::Unix,
};

fn auto(
    stdout_is_terminal: bool,
    active_agent: bool,
    ci: bool,
    no_color: bool,
    term_is_dumb: bool,
    width: u16,
) -> OutputContext {
    OutputContext::auto(
        stdout_is_terminal,
        active_agent,
        ci,
        no_color,
        term_is_dumb,
        width,
        CommandPlatform::Unix,
    )
}

#[test]
fn json_search_returns_one_versioned_document() {
    let (exit, stdout, stderr) = run(
        &["skilld", "search", "grill", "--json"],
        auto(true, true, false, false, false, 80),
    );
    let global_form = run(
        &["skilld", "--json", "search", "grill"],
        auto(true, true, false, false, false, 80),
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
fn json_help_and_version_return_versioned_documents() {
    let root_help = run(&["skilld", "--json", "--help"], PLAIN);
    let search_help = run(&["skilld", "search", "--json", "--help"], PLAIN);
    let run_help = run(&["skilld", "run", "--json", "--help"], PLAIN);
    let version = run(&["skilld", "--json", "--version"], PLAIN);

    assert_eq!(root_help.0, 0);
    assert!(root_help.2.is_empty());
    let root = serde_json::from_str::<serde_json::Value>(&root_help.1).unwrap();
    assert_eq!(root["command"], "help");
    assert_eq!(root["data"]["path"], "skilld");
    let search = serde_json::from_str::<serde_json::Value>(&search_help.1).unwrap();
    assert_eq!(search["command"], "help");
    assert_eq!(search["data"]["path"], "skilld search");
    let run = serde_json::from_str::<serde_json::Value>(&run_help.1).unwrap();
    assert_eq!(run["command"], "help");
    assert_eq!(run["data"]["path"], "skilld run");
    let version = serde_json::from_str::<serde_json::Value>(&version.1).unwrap();
    assert_eq!(version["command"], "version");
    assert_eq!(version["data"]["name"], "skilld");
    assert_eq!(version["data"]["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn help_explains_primary_flow_remote_file_revisions_and_direct_delivery() {
    let root = run(&["skilld", "--help"], PLAIN);
    let install = run(&["skilld", "install", "--help"], PLAIN);
    let run_help = run(&["skilld", "run", "--help"], PLAIN);

    assert_eq!(root.0, 0);
    assert!(root.2.is_empty());
    assert!(
        root.1
            .contains("Search, run, install, and keep Skills current")
    );
    assert_eq!(install.0, 0);
    assert!(install.2.is_empty());
    assert!(
        install
            .1
            .contains("Install a hosted Artifact from an explicit GitHub selector.")
    );
    assert!(
        install
            .1
            .contains("Add --direct to fetch a public GitHub Repository instead.")
    );
    assert_eq!(run_help.0, 0);
    assert!(run_help.2.is_empty());
    assert!(
        run_help
            .1
            .contains("Remote file reads also require the returned --revision.")
    );
}

#[test]
fn non_terminal_and_ci_output_are_stable_plain_records() {
    let expected = concat!(
        "grill-me\tskilld:mattpocock/skills/grill-me\t227068\t",
        "A focused Skill description that wraps cleanly on a narrow terminal.\n"
    );

    let non_terminal = run(
        &["skilld", "search", "grill"],
        auto(false, false, false, false, false, 80),
    );
    let ci_with_tty = run(
        &["skilld", "search", "grill"],
        auto(true, false, true, false, false, 120),
    );

    assert_eq!(non_terminal, (0, expected.to_owned(), String::new()));
    assert_eq!(ci_with_tty, non_terminal);
}

#[test]
fn human_terminal_is_formatted_without_an_explicit_machine_flag() {
    let agent_with_tty = run(
        &["skilld", "search", "grill"],
        auto(true, false, false, false, false, 120),
    );

    assert_eq!(agent_with_tty.0, 0);
    assert!(agent_with_tty.1.contains("Skill search"));
    assert!(agent_with_tty.1.contains('\u{1b}'));
    assert!(agent_with_tty.2.is_empty());
}

#[test]
fn active_agent_terminal_is_plain_without_an_explicit_machine_flag() {
    let result = run(
        &["skilld", "search", "grill"],
        auto(true, true, false, false, false, 120),
    );

    assert_eq!(
        result,
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
        auto(true, false, false, false, false, 120),
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
    response.items[0].description = Some("first\nsecond\tvalue\u{1b}\u{202e}".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run_with_output(
        ["skilld", "search", "grill", "--plain"],
        &SearchHost {
            response: Ok(response),
        },
        auto(true, false, false, false, false, 80),
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert!(stderr.is_empty());
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "grill-me\tskilld:mattpocock/skills/grill-me\t227068\tfirst\\nsecond\\tvalue\\u{001B}\\u{202E}\n"
    );
}

#[test]
fn human_search_is_polished_and_respects_terminal_width() {
    let (_, stdout, stderr) = run(
        &["skilld", "search", "grill"],
        auto(true, false, false, true, false, 40),
    );

    assert!(stderr.is_empty());
    assert!(stdout.contains("Skill search"));
    assert!(stdout.contains("1 of 14 Skills"));
    assert!(stdout.contains("227,068 stars"));
    assert!(stdout.contains("skilld:mattpocock/skills/grill-me"));
    assert!(stdout.contains("skilld run"));
    assert!(stdout.contains("skilld:mattpocock/skills/grill-me"));
    assert!(!stdout.contains("skilld install"));
    assert!(
        stdout
            .lines()
            .filter(|line| !line.trim_start().starts_with("skilld run "))
            .all(|line| UnicodeWidthStr::width(line) <= 40)
    );
    assert!(!stdout.contains('\u{1b}'));
}

#[test]
fn human_search_keeps_the_run_command_on_one_line() {
    let mut response = response();
    let name = "a-very-long-skill-name".to_owned();
    response.items[0].name.clone_from(&name);
    response.items[0].source.selector = SourceSelector::NamedSkill { name };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run_with_output(
        ["skilld", "search", "grill"],
        &SearchHost {
            response: Ok(response),
        },
        OutputContext::HumanTerminal {
            width: 20,
            color: false,
            platform: CommandPlatform::Unix,
        },
        &mut stdout,
        &mut stderr,
    );
    let stdout = String::from_utf8(stdout).unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(stderr.is_empty());
    assert!(
        stdout
            .lines()
            .any(|line| line == "  skilld run skilld:mattpocock/skills/a-very-long-skill-name")
    );
}

#[test]
fn human_empty_search_names_the_query_and_suggests_a_next_step() {
    let mut response = response();
    response.items.clear();
    response.total = 0;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run_with_output(
        ["skilld", "search", "grill"],
        &SearchHost {
            response: Ok(response),
        },
        auto(true, false, false, true, false, 40),
        &mut stdout,
        &mut stderr,
    );
    let stdout = String::from_utf8(stdout).unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(stderr.is_empty());
    assert!(stdout.contains("No Skills found for grill."));
    assert!(stdout.contains("Try a shorter search."));
}

#[test]
fn human_search_uses_display_cells_and_sanitizes_terminal_formatting() {
    let mut response = response();
    response.items[0].name = "\u{6280}\u{80fd}\u{1f642}".to_owned();
    response.items[0].description =
        Some("\u{6f22}\u{5b57}\u{1f642} cafe\u{301} \u{1b}[31mred\u{202e}forged".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run_with_output(
        ["skilld", "search", "grill"],
        &SearchHost {
            response: Ok(response),
        },
        auto(true, false, false, true, false, 20),
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
    assert!(!stdout.contains('\u{202e}'));
    assert!(
        stdout
            .lines()
            .filter(|line| !line.trim_start().starts_with("skilld run "))
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
        PLAIN,
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
        auto(true, false, false, false, false, 100),
    );
    let (_, no_color, _) = run(
        &["skilld", "search", "grill"],
        auto(true, false, false, true, false, 100),
    );
    let (_, dumb_terminal, _) = run(
        &["skilld", "search", "grill"],
        auto(true, false, false, false, true, 100),
    );

    assert!(colored.contains('\u{1b}'));
    assert_eq!(strip_ansi(&colored), no_color);
    assert_eq!(no_color, dumb_terminal);
}

#[test]
fn conflicting_output_flags_fail_before_search() {
    let (exit, stdout, stderr) = run(
        &["skilld", "search", "grill", "--json", "--plain"],
        auto(false, false, false, false, false, 80),
    );

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn json_search_parse_errors_are_tagged_usage_errors() {
    let after = run(&["skilld", "search", "grill", "--json", "--unknown"], PLAIN);
    let before = run(&["skilld", "--json", "search", "grill", "--unknown"], PLAIN);

    assert_eq!(after.0, 2);
    assert!(after.1.is_empty());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&after.2).unwrap()["_tag"],
        "UsageError"
    );
    assert_eq!(before, after);
}

#[test]
fn human_parse_errors_keep_clap_guidance_without_untrusted_terminal_formatting() {
    let hostile = "--unknown\u{1b}[31m\u{0085}\u{202e}\rforged\nline";
    let (exit, stdout, stderr) = run(
        &["skilld", "search", "grill", hostile],
        OutputContext::HumanTerminal {
            width: 80,
            color: false,
            platform: CommandPlatform::Unix,
        },
    );

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert!(stderr.contains("unexpected argument"));
    assert!(stderr.contains("Usage:"));
    assert!(stderr.contains("\\u{001B}"));
    assert!(stderr.contains("\\u{0085}"));
    assert!(stderr.contains("\\u{202E}"));
    assert!(stderr.contains("\\rforged\\nline"));
    assert!(!stderr.contains('\u{1b}'));
    assert!(!stderr.contains('\u{0085}'));
    assert!(!stderr.contains('\u{202e}'));
    assert!(!stderr.contains('\r'));
}

#[test]
fn plain_parse_errors_escape_record_delimiters_and_terminal_formatting() {
    let hostile = "--unknown\u{1b}[31m\u{0085}\u{202e}\rforged\nline";
    let (exit, stdout, stderr) = run(&["skilld", "search", "grill", hostile, "--plain"], PLAIN);

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(stderr.lines().count(), 1);
    assert!(stderr.contains("INVALID_ARGUMENT:"));
    assert!(stderr.contains("\\u{0085}"));
    assert!(stderr.contains("\\u{202E}"));
    assert!(stderr.contains("\\rforged\\nline"));
    assert!(!stderr.contains('\u{1b}'));
    assert!(!stderr.contains('\u{0085}'));
    assert!(!stderr.contains('\u{202e}'));
    assert!(!stderr.contains('\r'));
}

#[test]
fn json_parse_errors_keep_the_typed_error_contract_for_untrusted_arguments() {
    let hostile = "--unknown\u{1b}[31m\u{0085}\u{202e}\rforged\nline";
    let (exit, stdout, stderr) = run(&["skilld", "search", "grill", hostile, "--json"], PLAIN);

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(stderr.lines().count(), 1);
    let error = serde_json::from_str::<serde_json::Value>(&stderr).unwrap();
    assert_eq!(error["schemaVersion"], 1);
    assert_eq!(error["_tag"], "UsageError");
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        error["error"]["message"],
        "unexpected argument '--unknown\u{0085}\u{202e}\rforged"
    );
}

#[test]
fn empty_json_search_is_a_tagged_usage_error() {
    let (exit, stdout, stderr) = run(&["skilld", "search", "--json"], PLAIN);

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    let error = serde_json::from_str::<serde_json::Value>(&stderr).unwrap();
    assert_eq!(error["_tag"], "UsageError");
    assert_eq!(error["error"]["code"], "INVALID_SEARCH");
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
        auto(false, false, false, false, false, 80),
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 1);
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

#[test]
fn remote_errors_are_terminal_safe_and_json_preserves_the_original_text() {
    let code = "REMOTE\nCODE\u{202e}";
    let message = "line one\u{1b}[31m\u{0085}\u{202e}\rforged\nline two";
    let host = SearchHost {
        response: Err(CommandError::remote(RemoteError::new(code, message))),
    };
    let mut human_stdout = Vec::new();
    let mut human_stderr = Vec::new();
    let mut plain_stdout = Vec::new();
    let mut plain_stderr = Vec::new();
    let mut json_stdout = Vec::new();
    let mut json_stderr = Vec::new();

    let human = run_with_output(
        ["skilld", "search", "grill"],
        &host,
        OutputContext::HumanTerminal {
            width: 80,
            color: false,
            platform: CommandPlatform::Unix,
        },
        &mut human_stdout,
        &mut human_stderr,
    );
    let plain = run_with_output(
        ["skilld", "search", "grill", "--plain"],
        &host,
        PLAIN,
        &mut plain_stdout,
        &mut plain_stderr,
    );
    let json = run_with_output(
        ["skilld", "search", "grill", "--json"],
        &host,
        PLAIN,
        &mut json_stdout,
        &mut json_stderr,
    );

    assert_eq!(human.exit_code, 1);
    assert!(human_stdout.is_empty());
    assert_eq!(
        String::from_utf8(human_stderr).unwrap(),
        "✗ line one [31m   forged line two (REMOTE CODE )\n"
    );
    assert_eq!(plain.exit_code, 1);
    assert!(plain_stdout.is_empty());
    assert_eq!(
        String::from_utf8(plain_stderr).unwrap(),
        "REMOTE\\nCODE\\u{202E}: line one\\u{001B}[31m\\u{0085}\\u{202E}\\rforged\\nline two\n"
    );
    assert_eq!(json.exit_code, 1);
    assert!(json_stdout.is_empty());
    let json_error = serde_json::from_slice::<serde_json::Value>(&json_stderr).unwrap();
    assert_eq!(json_error["error"]["code"], code);
    assert_eq!(json_error["error"]["message"], message);
}

#[test]
fn stdout_failures_report_an_operation_error() {
    let mut stdout = WriteErrorWriter;
    let mut stderr = Vec::new();

    let result = run_with_output(
        ["skilld", "search", "grill", "--json"],
        &SearchHost {
            response: Ok(response()),
        },
        PLAIN,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 1);
    let error = serde_json::from_slice::<serde_json::Value>(&stderr).unwrap();
    assert_eq!(error["_tag"], "OperationError");
    assert_eq!(error["error"]["code"], "OUTPUT_WRITE_FAILED");
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

struct WriteErrorWriter;

impl Write for WriteErrorWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("write failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
