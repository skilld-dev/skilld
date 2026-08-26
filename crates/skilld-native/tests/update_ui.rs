use std::cell::RefCell;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use skilld_command::{CommandError, Host};
use skilld_core::{
    CommitAuthor, CommitHistory, CommitSha, CommitSummary as CoreCommitSummary, InstallScope,
    InstallSource, SkillName, UpdateFailure, UpdatePlan, UpdatePlanItem, UpdatePlanV1,
    UpdateRelation, UpdateRetryAfter,
};
use skilld_native::update_ui::{
    ApplyResult, CommandInteractiveUpdateHost, CommitLoadResult, CommitPage, CommitSummary,
    ComparisonId, Effect, InteractiveUpdateError, InteractiveUpdateHost, KeyInput, Message, Model,
    Tab, TerminalLifecycle, UpdateCandidate, initial_effect, render_snapshot,
    require_interactive_tty, resolve_effect, update, with_restored_terminal,
};
use unicode_width::UnicodeWidthStr;

fn comparison(value: &str) -> ComparisonId {
    ComparisonId::new(value)
}

fn candidate(name: &str, repository: &str, commits: u64) -> UpdateCandidate {
    UpdateCandidate::new(
        name,
        repository,
        "1111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222",
        commits,
        comparison(&format!("{repository}:111..222")),
    )
}

fn browsing_model(width: u16, height: u16) -> Model {
    let transition = update(
        Model::new(width, height),
        Message::CandidatesLoaded(Ok(vec![
            candidate("grill-me", "mattpocock/skills", 4),
            candidate("review-skill", "skilld-dev/skilld", 2),
            candidate("web-perf", "cloudflare/skills", 9),
        ])),
    );
    assert!(transition.effects.is_empty());
    transition.model
}

#[test]
fn model_selects_outdated_skills_and_generates_visible_key_help() {
    assert_eq!(initial_effect(), Effect::LoadCandidates);
    let transition = update(browsing_model(100, 20), Message::Key(KeyInput::Down));
    let transition = update(transition.model, Message::Key(KeyInput::Select));
    let snapshot = render_snapshot(&transition.model, false);

    assert!(transition.effects.is_empty());
    assert!(snapshot.contains("> ◯ review-skill"));
    assert!(snapshot.contains("2 selected"));
    assert!(snapshot.contains("↑/↓ move"));
    assert!(snapshot.contains("space select"));
    assert!(snapshot.contains("tab commits"));
    assert!(snapshot.contains("enter update"));
}

#[test]
fn commits_load_on_first_open_and_retry_only_failed_comparisons() {
    let model = browsing_model(100, 22);
    let expected = model.comparison_ids();
    let transition = update(model, Message::Key(KeyInput::NextTab));

    assert_eq!(transition.model.tab(), Tab::Commits);
    assert_eq!(transition.effects, [Effect::LoadCommits(expected.clone())]);

    let failed = expected[0].clone();
    let ready = expected[1].clone();
    let transition = update(
        transition.model,
        Message::CommitsLoaded(vec![
            CommitLoadResult::failed(
                failed.clone(),
                InteractiveUpdateError::new("GITHUB_RATE_LIMIT", "Try again after 10:42 UTC."),
            ),
            CommitLoadResult::ready(
                ready.clone(),
                CommitPage::new(
                    ready,
                    vec![CommitSummary::new(
                        "3333333333333333333333333333333333333333",
                        "fix: preserve accepted decisions\nignored body",
                        "2026-08-20T03:14:00Z",
                        "Harlan",
                    )],
                    1,
                    false,
                    "https://github.com/skilld-dev/skilld/compare/111...222",
                ),
            ),
        ]),
    );
    let snapshot = render_snapshot(&transition.model, false);

    assert!(snapshot.contains("GITHUB_RATE_LIMIT"));
    assert!(snapshot.contains("3333333"));
    assert!(snapshot.contains("fix: preserve accepted decisions"));
    assert!(!snapshot.contains("ignored body"));

    let transition = update(transition.model, Message::Key(KeyInput::Retry));
    assert_eq!(transition.effects, [Effect::LoadCommits(vec![failed])]);
}

#[test]
fn view_reflows_at_narrow_width_and_keeps_the_cursor_in_the_viewport() {
    let mut model = browsing_model(44, 11);
    for _ in 0..2 {
        model = update(model, Message::Key(KeyInput::Down)).model;
    }
    model = update(
        model,
        Message::Resized {
            width: 36,
            height: 10,
        },
    )
    .model;
    let snapshot = render_snapshot(&model, false);

    assert!(snapshot.contains("> ◉ web-perf"));
    assert!(!snapshot.contains("cloudflare/skills"));
    assert!(
        snapshot
            .lines()
            .all(|line| UnicodeWidthStr::width(line) <= 36)
    );
}

#[test]
fn view_preserves_unicode_width_and_removes_terminal_formatting() {
    let model = update(
        Model::new(40, 12),
        Message::CandidatesLoaded(Ok(vec![candidate(
            "技能🙂 cafe\u{301}\u{1b}[31m\u{202e}forged",
            "skilld-dev/skills",
            3,
        )])),
    )
    .model;
    let plain = render_snapshot(&model, false);
    let colored = render_snapshot(&model, true);

    assert!(plain.contains("技能🙂 cafe\u{301}"), "{plain:?}");
    assert!(!plain.contains('\u{1b}'));
    assert!(!plain.contains('\u{202e}'));
    assert_eq!(plain, colored);
    assert!(plain.lines().all(|line| UnicodeWidthStr::width(line) <= 40));
}

#[test]
fn commits_render_newest_first_and_name_visible_truncation() {
    let mut model = update(
        Model::new(100, 18),
        Message::CandidatesLoaded(Ok(vec![candidate(
            "review-skill",
            "skilld-dev/skilld",
            550,
        )])),
    )
    .model;
    let id = model.comparison_ids()[0].clone();
    model = update(model, Message::Key(KeyInput::NextTab)).model;
    model = update(
        model,
        Message::CommitsLoaded(vec![CommitLoadResult::ready(
            id.clone(),
            CommitPage::new(
                id,
                vec![
                    CommitSummary::new(
                        "3333333333333333333333333333333333333333",
                        "older commit",
                        "2026-08-19T03:14:00Z",
                        "Harlan",
                    ),
                    CommitSummary::new(
                        "4444444444444444444444444444444444444444",
                        "newest commit",
                        "2026-08-21T03:14:00Z",
                        "Harlan",
                    ),
                ],
                550,
                true,
                "https://github.com/skilld-dev/skilld/compare/111...222",
            ),
        )]),
    )
    .model;
    let snapshot = render_snapshot(&model, false);

    assert!(snapshot.find("newest commit").unwrap() < snapshot.find("older commit").unwrap());
    assert!(snapshot.contains("Showing newest 2 of 550 commits."));
}

#[test]
fn loading_failure_can_retry_or_cancel() {
    let transition = update(
        Model::new(80, 20),
        Message::CandidatesLoaded(Err(InteractiveUpdateError::new(
            "SERVICE_UNAVAILABLE",
            "The update plan could not load.",
        ))),
    );
    let snapshot = render_snapshot(&transition.model, false);

    assert!(snapshot.contains("SERVICE_UNAVAILABLE"));
    assert!(snapshot.contains("r retry"));

    let retried = update(transition.model.clone(), Message::Key(KeyInput::Retry));
    assert_eq!(retried.effects, [Effect::LoadCandidates]);

    let cancelled = update(transition.model, Message::Key(KeyInput::Cancel));
    assert_eq!(cancelled.effects.len(), 1);
    assert!(cancelled.effects[0].exit_summary().unwrap().cancelled);
}

#[test]
fn one_unavailable_skill_keeps_other_updates_selectable() {
    let model = update(
        Model::new(120, 16),
        Message::CandidatesLoaded(Ok(vec![
            candidate("grill-me", "mattpocock/skills", 4),
            UpdateCandidate::unavailable(
                "review-skill",
                InteractiveUpdateError::new("RATE_LIMITED", "Try again in 60 seconds."),
            ),
        ])),
    )
    .model;
    let snapshot = render_snapshot(&model, false);

    assert!(snapshot.contains("⚠ review-skill: RATE_LIMITED"));
    assert!(snapshot.contains("1 selected. 1 unavailable"));
    assert!(snapshot.contains("r retry"));

    let retry = update(model.clone(), Message::Key(KeyInput::Retry));
    assert_eq!(retry.effects, [Effect::LoadCandidates]);

    let model = update(model, Message::Key(KeyInput::Down)).model;
    let model = update(model, Message::Key(KeyInput::Select)).model;
    let transition = update(model, Message::Key(KeyInput::Apply));

    assert_eq!(
        transition.effects,
        [Effect::Apply(vec!["grill-me".to_owned()])]
    );
}

#[test]
fn applying_selected_skills_produces_a_static_summary() {
    let model = update(browsing_model(80, 20), Message::Key(KeyInput::SelectAll)).model;
    let transition = update(model, Message::Key(KeyInput::Apply));

    assert_eq!(
        transition.effects,
        [Effect::Apply(vec![
            "grill-me".to_owned(),
            "review-skill".to_owned(),
            "web-perf".to_owned(),
        ])]
    );
    let cancelling = update(transition.model.clone(), Message::Key(KeyInput::Interrupt));
    assert!(cancelling.effects[0].exit_summary().unwrap().cancelled);
    assert!(render_snapshot(&cancelling.model, false).contains("Updating 3 Skills"));

    let transition = update(
        transition.model,
        Message::Applied(vec![
            ApplyResult::updated("grill-me"),
            ApplyResult::updated("review-skill"),
            ApplyResult::failed(
                "web-perf",
                InteractiveUpdateError::new("CHECK_BLOCKED", "Review the failed check."),
            ),
        ]),
    );
    let summary = transition.effects[0].exit_summary().unwrap();

    assert_eq!(summary.updated, 2);
    assert_eq!(summary.failed, 1);
    assert_eq!(
        summary.render(),
        concat!(
            "Updated 2 Skills.\n",
            "Failed Skill web-perf: CHECK_BLOCKED: Review the failed check.\n"
        )
    );
}

#[derive(Clone)]
struct RecordingTerminal {
    events: Rc<RefCell<Vec<&'static str>>>,
}

impl TerminalLifecycle for RecordingTerminal {
    fn enter(&mut self) -> Result<(), InteractiveUpdateError> {
        self.events.borrow_mut().push("enter");
        Ok(())
    }

    fn restore(&mut self) -> Result<(), InteractiveUpdateError> {
        self.events.borrow_mut().push("restore");
        Ok(())
    }
}

#[test]
fn terminal_restores_after_success_error_and_panic() {
    for outcome in ["success", "error", "panic"] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let terminal = RecordingTerminal {
            events: events.clone(),
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            with_restored_terminal(terminal, || match outcome {
                "success" => Ok(()),
                "error" => Err(InteractiveUpdateError::new("TEST", "failed")),
                _ => panic!("test panic"),
            })
        }));

        if outcome == "panic" {
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
        }
        assert_eq!(*events.borrow(), ["enter", "restore"]);
    }
}

#[test]
fn interactive_mode_needs_both_terminal_streams() {
    assert!(require_interactive_tty(true, true).is_ok());
    for streams in [(false, false), (false, true), (true, false)] {
        assert_eq!(
            require_interactive_tty(streams.0, streams.1)
                .unwrap_err()
                .code,
            "INTERACTIVE_TTY_REQUIRED"
        );
    }
}

#[derive(Clone)]
struct FixtureHost {
    candidates: Vec<UpdateCandidate>,
}

impl InteractiveUpdateHost for FixtureHost {
    fn load_candidates(&self) -> Result<Vec<UpdateCandidate>, InteractiveUpdateError> {
        Ok(self.candidates.clone())
    }

    fn load_commits(&self, comparisons: &[ComparisonId]) -> Vec<CommitLoadResult> {
        comparisons
            .iter()
            .cloned()
            .map(|id| {
                CommitLoadResult::ready(
                    id.clone(),
                    CommitPage::new(id, Vec::new(), 0, false, "https://github.com"),
                )
            })
            .collect()
    }

    fn apply(&self, names: &[String]) -> Vec<ApplyResult> {
        names.iter().map(ApplyResult::updated).collect()
    }
}

#[test]
fn fixture_host_resolves_model_effects_without_terminal_access() {
    let host = FixtureHost {
        candidates: vec![candidate("grill-me", "mattpocock/skills", 4)],
    };

    let loaded = resolve_effect(&host, Effect::LoadCandidates);
    let transition = update(Model::new(80, 20), loaded);
    let applied = resolve_effect(
        &host,
        update(transition.model, Message::Key(KeyInput::Apply)).effects[0].clone(),
    );

    assert!(
        matches!(applied, Message::Applied(results) if results == [ApplyResult::updated("grill-me")])
    );
}

struct PlanHost {
    plan: UpdatePlanV1,
    selections: Mutex<Vec<Vec<UpdatePlanItem>>>,
    apply_error: Option<CommandError>,
}

impl Host for PlanHost {
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

    fn update_check(&self, _name: Option<&str>) -> Result<UpdatePlanV1, CommandError> {
        Ok(self.plan.clone())
    }

    fn update_selected(
        &self,
        items: &[UpdatePlanItem],
    ) -> Result<Vec<skilld_ui::Line>, CommandError> {
        self.selections.lock().unwrap().push(items.to_vec());
        if let Some(error) = self.apply_error.clone() {
            return Err(error);
        }
        Ok(items
            .iter()
            .map(|item| {
                skilld_ui::Line::success(format!("Updated Skill {}.", item.name().as_str()))
            })
            .collect())
    }
}

fn command_plan_host(apply_error: Option<CommandError>) -> Arc<PlanHost> {
    let locked = CommitSha::parse("1".repeat(40)).unwrap();
    let latest = CommitSha::parse("2".repeat(40)).unwrap();
    let available = UpdatePlanItem::with_history(
        SkillName::parse("grill-me").unwrap(),
        UpdateRelation::Available {
            locked_commit_sha: locked.clone(),
            latest_commit_sha: latest.clone(),
            ahead_by: NonZeroU64::new(1).unwrap(),
        },
        CommitHistory::compared(
            vec![CoreCommitSummary {
                sha: latest.clone(),
                subject: "feat: sharpen design questions".to_owned(),
                author: CommitAuthor {
                    name: "Ada".to_owned(),
                    login: Some("ada".to_owned()),
                },
                timestamp: "2026-08-21T00:00:00Z".to_owned(),
                url: format!(
                    "https://github.com/mattpocock/skills/commit/{}",
                    latest.as_str()
                ),
            }],
            1,
            false,
            format!(
                "https://github.com/mattpocock/skills/compare/{}...{}",
                locked.as_str(),
                latest.as_str()
            ),
        )
        .unwrap(),
    );
    let second_available = UpdatePlanItem::new(
        SkillName::parse("smoke-skill").unwrap(),
        UpdateRelation::Available {
            locked_commit_sha: locked.clone(),
            latest_commit_sha: latest.clone(),
            ahead_by: NonZeroU64::new(1).unwrap(),
        },
    );
    let unavailable = UpdatePlanItem::new(
        SkillName::parse("review-skill").unwrap(),
        UpdateRelation::Unavailable {
            locked_commit_sha: locked.clone(),
            latest_commit: skilld_core::UpdateLatestCommit::Known { commit_sha: latest },
            failure: UpdateFailure::rate_limited(
                "GitHub rate limited the update check.",
                UpdateRetryAfter::Seconds { seconds: 60 },
            ),
        },
    );
    let current = UpdatePlanItem::new(
        SkillName::parse("current-skill").unwrap(),
        UpdateRelation::Current { commit_sha: locked },
    );
    Arc::new(PlanHost {
        plan: UpdatePlanV1::new(
            UpdatePlan::new(vec![available, second_available, unavailable, current]).unwrap(),
        ),
        selections: Mutex::new(Vec::new()),
        apply_error,
    })
}

#[test]
fn command_host_maps_update_plans_commits_and_exact_selection() {
    let host = command_plan_host(None);
    let interactive = CommandInteractiveUpdateHost::new(host.clone());
    let candidates = interactive.load_candidates().unwrap();
    let mut model = update(
        Model::new(100, 18),
        Message::CandidatesLoaded(Ok(candidates)),
    )
    .model;
    let outdated = render_snapshot(&model, false);

    assert!(outdated.contains("grill-me"));
    assert!(outdated.contains("review-skill: RATE_LIMITED"));
    assert!(outdated.contains("Try again in 60 seconds."));
    assert!(!outdated.contains("current-skill"));

    let ids = model.comparison_ids();
    model = update(model, Message::Key(KeyInput::NextTab)).model;
    model = update(
        model,
        Message::CommitsLoaded(interactive.load_commits(&ids)),
    )
    .model;
    assert!(render_snapshot(&model, false).contains("feat: sharpen design questions"));

    assert_eq!(
        interactive.apply(&["grill-me".to_owned()]),
        [ApplyResult::updated("grill-me")]
    );
    let selections = host.selections.lock().unwrap();
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0][0].name().as_str(), "grill-me");
    assert!(matches!(
        selections[0][0].relation(),
        UpdateRelation::Available {
            latest_commit_sha,
            ..
        } if latest_commit_sha.as_str() == "2222222222222222222222222222222222222222"
    ));
}

#[test]
fn command_host_reports_one_atomic_failure_for_every_selected_skill() {
    let host = command_plan_host(Some(CommandError::operation(
        "CHECK_BLOCKED",
        "A required check failed.",
    )));
    let interactive = CommandInteractiveUpdateHost::new(host.clone());
    interactive.load_candidates().unwrap();
    let names = ["grill-me".to_owned(), "smoke-skill".to_owned()];

    let results = interactive.apply(&names);

    assert_eq!(host.selections.lock().unwrap().len(), 1);
    assert_eq!(
        results,
        names
            .iter()
            .map(|name| ApplyResult::failed(
                name,
                InteractiveUpdateError::new("CHECK_BLOCKED", "A required check failed.")
            ))
            .collect::<Vec<_>>()
    );
}
