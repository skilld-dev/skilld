//! The sign-in notice that points a signed-out person at the weekly.
//!
//! skilld.dev sends one weekly email to an account: Skills you liked that
//! changed, plus what trended. It is on by default and can be turned off.
//! A person with no account cannot receive it and has no way to learn it
//! exists, so the CLI says so a few times and then stops.
//!
//! This module decides. It reads no file, reads no clock, and prints nothing.

use serde::{Deserialize, Serialize};

/// The shortest gap between two notices.
pub const NOTICE_INTERVAL_SECONDS: u64 = 7 * 24 * 60 * 60;

/// How many notices one installation ever prints.
pub const NOTICE_LIMIT: u32 = 3;

/// What the CLI prints. Two lines, no color, stderr only.
pub const NOTICE_MESSAGE: &str = "The weekly email covers Skills you liked that changed, plus what trended.\nRun skilld auth login to get it. Turn it off at any time.";

/// How many notices this installation has printed, and when the last one was.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeeklyNoticeState {
    /// Unix seconds of the last notice. Zero means none yet.
    #[serde(default)]
    pub shown_at: u64,
    #[serde(default)]
    pub shown_count: u32,
}

/// Everything outside this module that the decision depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoticeContext {
    /// The person already has credentials, so they already get the weekly.
    pub signed_in: bool,
    /// The command is an `auth` command, which says this already.
    pub auth_command: bool,
    /// stderr is a terminal a person is reading.
    pub stderr_terminal: bool,
    /// stdout is a terminal too. A run piped into an Agent stays quiet.
    pub stdout_terminal: bool,
    /// An Agent, CI, or an explicit opt-out is present.
    pub suppressed: bool,
}

/// Whether to print the notice now.
pub fn should_show(state: &WeeklyNoticeState, context: NoticeContext, now: u64) -> bool {
    if context.signed_in
        || context.auth_command
        || context.suppressed
        || !context.stderr_terminal
        || !context.stdout_terminal
    {
        return false;
    }
    if state.shown_count >= NOTICE_LIMIT {
        return false;
    }
    if state.shown_at == 0 {
        return true;
    }
    now.saturating_sub(state.shown_at) >= NOTICE_INTERVAL_SECONDS
}

/// The state to store after printing a notice.
pub fn record_shown(state: &WeeklyNoticeState, now: u64) -> WeeklyNoticeState {
    WeeklyNoticeState {
        shown_at: now,
        shown_count: state.shown_count.saturating_add(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEEK: u64 = NOTICE_INTERVAL_SECONDS;

    fn eligible() -> NoticeContext {
        NoticeContext {
            signed_in: false,
            auth_command: false,
            stderr_terminal: true,
            stdout_terminal: true,
            suppressed: false,
        }
    }

    #[test]
    fn shows_on_a_first_run() {
        assert!(should_show(
            &WeeklyNoticeState::default(),
            eligible(),
            1_000
        ));
    }

    #[test]
    fn stays_quiet_for_a_signed_in_person() {
        let context = NoticeContext {
            signed_in: true,
            ..eligible()
        };
        assert!(!should_show(&WeeklyNoticeState::default(), context, 1_000));
    }

    #[test]
    fn stays_quiet_during_an_auth_command() {
        let context = NoticeContext {
            auth_command: true,
            ..eligible()
        };
        assert!(!should_show(&WeeklyNoticeState::default(), context, 1_000));
    }

    #[test]
    fn stays_quiet_when_suppressed_or_piped() {
        let suppressed = NoticeContext {
            suppressed: true,
            ..eligible()
        };
        let piped = NoticeContext {
            stderr_terminal: false,
            ..eligible()
        };
        assert!(!should_show(
            &WeeklyNoticeState::default(),
            suppressed,
            1_000
        ));
        assert!(!should_show(&WeeklyNoticeState::default(), piped, 1_000));
    }

    #[test]
    fn stays_quiet_when_stdout_is_piped_but_stderr_is_a_terminal() {
        let context = NoticeContext {
            stdout_terminal: false,
            ..eligible()
        };
        assert!(!should_show(&WeeklyNoticeState::default(), context, 1_000));
    }

    #[test]
    fn waits_a_week_between_notices() {
        let state = WeeklyNoticeState {
            shown_at: 1_000,
            shown_count: 1,
        };
        assert!(!should_show(&state, eligible(), 1_000 + WEEK - 1));
        assert!(should_show(&state, eligible(), 1_000 + WEEK));
    }

    #[test]
    fn stops_for_good_at_the_limit() {
        let state = WeeklyNoticeState {
            shown_at: 1_000,
            shown_count: NOTICE_LIMIT,
        };
        assert!(!should_show(&state, eligible(), 1_000 + WEEK * 520));
    }

    #[test]
    fn recording_advances_the_count_and_the_clock() {
        let first = record_shown(&WeeklyNoticeState::default(), 1_000);
        assert_eq!(
            first,
            WeeklyNoticeState {
                shown_at: 1_000,
                shown_count: 1,
            }
        );
        let second = record_shown(&first, 1_000 + WEEK);
        assert_eq!(second.shown_count, 2);
        assert_eq!(second.shown_at, 1_000 + WEEK);
    }

    #[test]
    fn a_backwards_clock_does_not_show_early() {
        let state = WeeklyNoticeState {
            shown_at: 10_000,
            shown_count: 1,
        };
        assert!(!should_show(&state, eligible(), 500));
    }
}
