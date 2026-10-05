//! The approval question `run`, `install`, `add`, and `update` ask before a
//! remote Skill brings a behavior that needs the user's approval.
//!
//! Only a person at a terminal sees it. An Agent, a pipe, or CI gets the
//! stopped run and its `--allow` command instead.

use std::io::{self, BufRead, Write};

use skilld_command::{
    BEHAVIOR_CAVEAT, BehaviorConfirmer, BehaviorDecision, CommandError, describe_behavior,
};
use skilld_core::Behavior;
use skilld_ui::text::sanitize;

/// Asks on standard error and reads one answer from standard input.
pub struct TtyBehaviorConfirmer {
    /// Frees the terminal line first, so a spinner never draws over the question.
    before_ask: Box<dyn Fn() + Send + Sync>,
}

impl TtyBehaviorConfirmer {
    pub fn new(before_ask: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            before_ask: Box::new(before_ask),
        }
    }
}

impl BehaviorConfirmer for TtyBehaviorConfirmer {
    fn confirm(
        &self,
        skill: &str,
        behaviors: &[&Behavior],
    ) -> Result<BehaviorDecision, CommandError> {
        (self.before_ask)();
        ask(
            &mut io::stdin().lock(),
            &mut io::stderr().lock(),
            skill,
            behaviors,
        )
    }
}

/// Write the question, then read one line. Anything but yes declines.
pub fn ask(
    input: &mut impl BufRead,
    output: &mut impl Write,
    skill: &str,
    behaviors: &[&Behavior],
) -> Result<BehaviorDecision, CommandError> {
    let mut question = format!(
        "The Skill {} needs your approval for these behaviors:\n",
        sanitize(skill)
    );
    for behavior in behaviors {
        question.push_str(&format!("  {}\n", sanitize(&describe_behavior(behavior))));
    }
    question.push_str(BEHAVIOR_CAVEAT);
    question.push_str("\nContinue with this Skill? [y/N] ");
    output
        .write_all(question.as_bytes())
        .and_then(|()| output.flush())
        .map_err(|error| CommandError::filesystem(format!("cannot ask for approval: {error}")))?;
    let mut answer = String::new();
    input
        .read_line(&mut answer)
        .map_err(|error| CommandError::filesystem(format!("cannot read the approval: {error}")))?;
    let answer = answer.trim();
    Ok(
        if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
            BehaviorDecision::Approved
        } else {
            BehaviorDecision::Declined
        },
    )
}

#[cfg(test)]
mod tests {
    use skilld_core::{BehaviorLocation, BehaviorTier};

    use super::*;

    fn remote_code() -> Behavior {
        Behavior {
            id: "remote-code",
            tier: BehaviorTier::Ask,
            label: "Runs code downloaded from the network",
            locations: vec![BehaviorLocation {
                path: "SKILL.md".to_owned(),
                line: Some(7),
            }],
            total: 1,
        }
    }

    fn answer(line: &str) -> (BehaviorDecision, String) {
        let behavior = remote_code();
        let mut output = Vec::new();
        let decision = ask(&mut line.as_bytes(), &mut output, "demo", &[&behavior]).unwrap();
        (decision, String::from_utf8(output).unwrap())
    }

    #[test]
    fn only_yes_approves() {
        assert_eq!(answer("y\n").0, BehaviorDecision::Approved);
        assert_eq!(answer("YES\n").0, BehaviorDecision::Approved);
        assert_eq!(answer("\n").0, BehaviorDecision::Declined);
        assert_eq!(answer("no\n").0, BehaviorDecision::Declined);
        assert_eq!(answer("").0, BehaviorDecision::Declined);
    }

    #[test]
    fn the_question_names_each_behavior_and_where_it_appears() {
        let (_, question) = answer("n\n");
        assert!(question.contains("Runs code downloaded from the network: SKILL.md:7"));
        assert!(question.ends_with("[y/N] "));
    }
}
