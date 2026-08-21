use std::num::NonZeroU64;

use skilld_core::{
    CommitSha, SkillName, UpdateCheckV1, UpdateLatestCommit, UpdateModelError, UpdatePlan,
    UpdatePlanItem, UpdateRelation, classify_update_comparison,
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
            ahead_by: NonZeroU64::new(3).unwrap(),
        }
    );
    assert_eq!(
        classify_update_comparison(locked.clone(), latest.clone(), 0, 2).unwrap(),
        UpdateRelation::Behind {
            locked_commit_sha: locked.clone(),
            latest_commit_sha: latest.clone(),
            behind_by: NonZeroU64::new(2).unwrap(),
        }
    );
    assert_eq!(
        classify_update_comparison(locked.clone(), latest.clone(), 4, 2).unwrap(),
        UpdateRelation::Diverged {
            locked_commit_sha: locked,
            latest_commit_sha: latest,
            ahead_by: NonZeroU64::new(4).unwrap(),
            behind_by: NonZeroU64::new(2).unwrap(),
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
fn v1_check_fixture_covers_the_published_relations() {
    let bytes = include_bytes!("../../../tests/fixtures/v3-rust/v1/update-check.json");
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let check: UpdateCheckV1 = serde_json::from_value(fixture["data"].clone()).unwrap();

    assert_eq!(fixture["schemaVersion"], 1);
    assert_eq!(fixture["_tag"], "Success");
    assert_eq!(fixture["command"], "update");
    assert_eq!(check.items().len(), 7);
    assert!(matches!(
        check.items()[6].relation(),
        UpdateRelation::Unavailable {
            latest_commit: UpdateLatestCommit::Known { .. },
            ..
        }
    ));
    assert_eq!(serde_json::to_value(check).unwrap(), fixture["data"]);
}
