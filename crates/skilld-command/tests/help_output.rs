use skilld_command::{
    CommandError, CommandPlatform, Host, InstalledSkill, OutputContext, run_with_output,
};
use skilld_core::{InstallScope, InstallSource};

struct HelpHost;
impl Host for HelpHost {
    fn list(&self, _: InstallScope) -> Result<Vec<String>, CommandError> {
        unreachable!("help never calls the host")
    }
    fn install(&self, _: InstallSource, _: InstallScope) -> Result<InstalledSkill, CommandError> {
        unreachable!("help never calls the host")
    }
}

fn run(args: &[&str], context: OutputContext) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(args, &HelpHost, context, &mut stdout, &mut stderr);
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}
fn human(color: bool) -> OutputContext {
    OutputContext::HumanTerminal {
        width: 80,
        color,
        platform: CommandPlatform::Unix,
    }
}
fn strip_styles(text: &str) -> String {
    let mut characters = text.chars();
    let mut plain = String::new();
    while let Some(character) = characters.next() {
        if character == '\x1b' {
            assert_eq!(characters.next(), Some('['));
            for code in characters.by_ref() {
                if code == 'm' {
                    break;
                }
            }
        } else {
            plain.push(character);
        }
    }
    plain
}

#[test]
fn root_subcommand_and_nested_help_share_colour_without_changing_words() {
    for args in [
        vec!["skilld", "--help"],
        vec!["skilld", "doctor", "--help"],
        vec!["skilld", "auth", "login", "--help"],
    ] {
        let coloured = run(&args, human(true));
        let monochrome = run(&args, human(false));
        assert_eq!(coloured.0, 0);
        assert!(coloured.2.is_empty());
        assert!(coloured.1.contains('\x1b'), "{args:?}: {}", coloured.1);
        assert!(!monochrome.1.contains('\x1b'));
        assert_eq!(strip_styles(&coloured.1), monochrome.1);
    }
}

#[test]
fn usage_errors_are_styled_on_stderr_with_hostile_arguments_escaped() {
    let args = ["skilld", "search", "--evil\x1b[2J\nflag"];
    let coloured = run(&args, human(true));
    let monochrome = run(&args, human(false));
    assert_eq!(coloured.0, 2);
    assert!(coloured.1.is_empty());
    assert!(coloured.2.contains('\x1b'));
    assert!(!coloured.2.contains("\x1b[2J"));
    assert!(coloured.2.contains("\\u{001B}"));
    assert_eq!(strip_styles(&coloured.2), monochrome.2);
}

#[test]
fn plain_and_json_help_keep_their_machine_contracts() {
    let plain = run(
        &["skilld", "auth", "login", "--plain", "--help"],
        human(true),
    );
    assert_eq!(plain.0, 0);
    assert!(plain.2.is_empty());
    assert!(!plain.1.contains('\x1b'));
    let json = run(
        &["skilld", "auth", "login", "--json", "--help"],
        human(true),
    );
    assert_eq!(json.0, 0);
    assert!(json.2.is_empty());
    assert!(!json.1.contains('\x1b'));
    let envelope: serde_json::Value = serde_json::from_str(&json.1).unwrap();
    assert_eq!(envelope["command"], "help");
    assert_eq!(envelope["data"]["path"], "skilld auth");
}

#[test]
fn terminal_capabilities_disable_help_styles() {
    for (no_color, dumb) in [(true, false), (false, true)] {
        let context = OutputContext::auto(
            true,
            false,
            false,
            no_color,
            dumb,
            80,
            CommandPlatform::Unix,
        );
        let output = run(&["skilld", "--help"], context);
        assert_eq!(output.0, 0);
        assert!(!output.1.contains('\x1b'));
        assert!(output.1.contains("Usage:"));
    }
}
