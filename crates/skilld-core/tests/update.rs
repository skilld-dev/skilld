use skilld_core::{
    CommitAuthor, CommitHistory, CommitSha, CommitSummary, SkillName, UpdateLatestCommit,
    UpdateModelError, UpdatePlan, UpdatePlanItem, UpdatePlanV1, UpdateRelation,
    classify_update_comparison,
};

fn sha(value: char) -> CommitSha {
    CommitSha::parse(value.to_string().repeat(40)).unwrap()
}

#[test]
fn comparison_counts_classify_every_git_relation() {
    let locked = sha('1');
    let latest = sha('2');

    assert_eq!(
        classify_update_comparison(locked.clone(), locked.clone(), 0, 0).unwrap(),
        UpdateRelation::Current {
            commit_sha: locked.clone(),
        }
    );
    assert_eq!(
        classify_update_comparison(locked.clone(), latest.clone(), 3, 0).unwrap(),
        UpdateRelation::Available {
            locked_commit_sha: locked.clone(),
            latest_commit_sha: latest.clone(),
        }
    );
    assert_eq!(
        classify_update_comparison(locked.clone(), latest.clone(), 0, 2).unwrap(),
        UpdateRelation::Behind {
            locked_commit_sha: locked.clone(),
            latest_commit_sha: latest.clone(),
        }
    );
    assert_eq!(
        classify_update_comparison(locked.clone(), latest.clone(), 4, 2).unwrap(),
        UpdateRelation::Diverged {
            locked_commit_sha: locked,
            latest_commit_sha: latest,
        }
    );
}

#[test]
fn impossible_comparison_counts_are_rejected() {
    assert_eq!(
        classify_update_comparison(sha('1'), sha('2'), 0, 0),
        Err(UpdateModelError::InvalidComparison)
    );
    assert_eq!(
        classify_update_comparison(sha('1'), sha('1'), 1, 0),
        Err(UpdateModelError::InvalidComparison)
    );
}

#[test]
fn update_plan_sorts_skills_and_rejects_duplicates() {
    let first = UpdatePlanItem::new(
        SkillName::parse("zeta").unwrap(),
        UpdateRelation::Pinned {
            commit_sha: sha('1'),
        },
    );
    let second = UpdatePlanItem::new(
        SkillName::parse("alpha").unwrap(),
        UpdateRelation::Current {
            commit_sha: sha('2'),
        },
    );
    let plan = UpdatePlan::new(vec![first, second]).unwrap();

    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.name().as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert_eq!(
        UpdatePlan::new(vec![
            UpdatePlanItem::new(
                SkillName::parse("alpha").unwrap(),
                UpdateRelation::Pinned {
                    commit_sha: sha('1'),
                },
            ),
            UpdatePlanItem::new(
                SkillName::parse("alpha").unwrap(),
                UpdateRelation::Current {
                    commit_sha: sha('2'),
                },
            ),
        ]),
        Err(UpdateModelError::DuplicateSkill("alpha".to_owned()))
    );
}

#[test]
fn update_plan_carries_full_bounded_commit_history() {
    let item = UpdatePlanItem::with_history(
        SkillName::parse("grill").unwrap(),
        UpdateRelation::Available {
            locked_commit_sha: sha('1'),
            latest_commit_sha: sha('2'),
        },
        CommitHistory::compared(
            vec![CommitSummary {
                sha: sha('2'),
                subject: "Add charcoal timing".to_owned(),
                author: CommitAuthor {
                    name: "Ada Lovelace".to_owned(),
                    login: Some("ada".to_owned()),
                },
                timestamp: "2026-08-21T00:00:00Z".to_owned(),
                url: format!(
                    "https://github.com/acme/skills/commit/{}",
                    sha('2').as_str()
                ),
            }],
            501,
            true,
            format!(
                "https://github.com/acme/skills/compare/{}...{}",
                sha('1').as_str(),
                sha('2').as_str()
            ),
        )
        .unwrap(),
    );
    let plan = UpdatePlanV1::new(UpdatePlan::new(vec![item]).unwrap());
    let value = serde_json::to_value(plan).unwrap();

    assert_eq!(value["items"][0]["history"]["total"], 501);
    assert_eq!(value["items"][0]["history"]["truncated"], true);
    assert_eq!(
        value["items"][0]["history"]["items"][0]["sha"],
        sha('2').as_str()
    );
    assert_eq!(
        value["items"][0]["history"]["compareUrl"],
        format!(
            "https://github.com/acme/skills/compare/{}...{}",
            sha('1').as_str(),
            sha('2').as_str()
        )
    );
}

#[test]
fn v1_check_fixture_covers_the_published_relations() {
    let bytes = include_bytes!("../../../tests/fixtures/v3-rust/v1/update-check.json");
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let check: UpdatePlanV1 = serde_json::from_value(fixture["data"].clone()).unwrap();

    assert_eq!(fixture["schemaVersion"], 1);
    assert_eq!(fixture["_tag"], "Success");
    assert_eq!(fixture["command"], "update");
    assert_eq!(check.items().len(), 7);
    assert!(check.has_changes());
    assert!(check.is_incomplete());
    assert!(matches!(
        check.items()[6].relation(),
        UpdateRelation::Unavailable {
            latest_commit: UpdateLatestCommit::Known { .. },
            ..
        }
    ));
    assert_eq!(serde_json::to_value(check).unwrap(), fixture["data"]);
}
