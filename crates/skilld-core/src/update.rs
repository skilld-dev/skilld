use std::fmt;
use std::num::NonZeroU64;

use serde::{Deserialize, Deserializer, Serialize};

use crate::SkillName;

const MAX_COMMIT_HISTORY: usize = 500;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommitSha(String);

impl CommitSha {
    pub fn parse(value: impl Into<String>) -> Result<Self, UpdateModelError> {
        let value = value.into();
        let valid = value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        valid
            .then_some(Self(value.clone()))
            .ok_or(UpdateModelError::InvalidCommitSha(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CommitSha {
    type Error = UpdateModelError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<CommitSha> for String {
    fn from(value: CommitSha) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CommitAuthor {
    pub name: String,
    pub login: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CommitSummary {
    pub sha: CommitSha,
    pub subject: String,
    pub author: CommitAuthor,
    pub timestamp: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "_tag",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommitHistory {
    Compared {
        items: Vec<CommitSummary>,
        total: u64,
        truncated: bool,
        compare_url: String,
    },
    #[default]
    NotNeeded,
}

impl CommitHistory {
    pub fn compared(
        items: Vec<CommitSummary>,
        total: u64,
        truncated: bool,
        compare_url: impl Into<String>,
    ) -> Result<Self, UpdateModelError> {
        let item_count = u64::try_from(items.len()).unwrap_or(u64::MAX);
        if items.len() > MAX_COMMIT_HISTORY
            || item_count > total
            || truncated != (item_count < total)
        {
            return Err(UpdateModelError::InvalidCommitHistory);
        }
        Ok(Self::Compared {
            items,
            total,
            truncated,
            compare_url: compare_url.into(),
        })
    }

    pub const fn is_not_needed(&self) -> bool {
        matches!(self, Self::NotNeeded)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateFailure {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<UpdateRetryAfter>,
}

impl UpdateFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn rate_limited(message: impl Into<String>, retry_after: UpdateRetryAfter) -> Self {
        Self {
            code: "RATE_LIMITED".to_owned(),
            message: message.into(),
            retry_after: Some(retry_after),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "_tag",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UpdateRetryAfter {
    Seconds { seconds: u64 },
    Reset { reset_at: String },
    SecondsAndReset { seconds: u64, reset_at: String },
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "_tag",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UpdateLatestCommit {
    Known { commit_sha: CommitSha },
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotTrackedReason {
    Local,
    Bundled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "_tag",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UpdateRelation {
    Current {
        commit_sha: CommitSha,
    },
    Available {
        locked_commit_sha: CommitSha,
        latest_commit_sha: CommitSha,
        ahead_by: NonZeroU64,
    },
    Behind {
        locked_commit_sha: CommitSha,
        latest_commit_sha: CommitSha,
        behind_by: NonZeroU64,
    },
    Diverged {
        locked_commit_sha: CommitSha,
        latest_commit_sha: CommitSha,
        ahead_by: NonZeroU64,
        behind_by: NonZeroU64,
    },
    Pinned {
        commit_sha: CommitSha,
    },
    NotTracked {
        reason: NotTrackedReason,
    },
    Unavailable {
        locked_commit_sha: CommitSha,
        latest_commit: UpdateLatestCommit,
        failure: UpdateFailure,
    },
}

pub fn classify_update_comparison(
    locked_commit_sha: CommitSha,
    latest_commit_sha: CommitSha,
    ahead_by: u64,
    behind_by: u64,
) -> Result<UpdateRelation, UpdateModelError> {
    match (locked_commit_sha == latest_commit_sha, ahead_by, behind_by) {
        (true, 0, 0) => Ok(UpdateRelation::Current {
            commit_sha: locked_commit_sha,
        }),
        (false, ahead_by, 0) if ahead_by > 0 => Ok(UpdateRelation::Available {
            locked_commit_sha,
            latest_commit_sha,
            ahead_by: NonZeroU64::new(ahead_by).expect("ahead count is non-zero"),
        }),
        (false, 0, behind_by) if behind_by > 0 => Ok(UpdateRelation::Behind {
            locked_commit_sha,
            latest_commit_sha,
            behind_by: NonZeroU64::new(behind_by).expect("behind count is non-zero"),
        }),
        (false, ahead_by, behind_by) if ahead_by > 0 && behind_by > 0 => {
            Ok(UpdateRelation::Diverged {
                locked_commit_sha,
                latest_commit_sha,
                ahead_by: NonZeroU64::new(ahead_by).expect("ahead count is non-zero"),
                behind_by: NonZeroU64::new(behind_by).expect("behind count is non-zero"),
            })
        }
        _ => Err(UpdateModelError::InvalidComparison),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdatePlanItem {
    name: SkillName,
    relation: UpdateRelation,
    #[serde(default, skip_serializing_if = "CommitHistory::is_not_needed")]
    history: CommitHistory,
}

impl UpdatePlanItem {
    pub fn new(name: SkillName, relation: UpdateRelation) -> Self {
        Self {
            name,
            relation,
            history: CommitHistory::NotNeeded,
        }
    }

    pub fn with_history(name: SkillName, relation: UpdateRelation, history: CommitHistory) -> Self {
        Self {
            name,
            relation,
            history,
        }
    }

    pub const fn name(&self) -> &SkillName {
        &self.name
    }

    pub const fn relation(&self) -> &UpdateRelation {
        &self.relation
    }

    pub const fn history(&self) -> &CommitHistory {
        &self.history
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdatePlan {
    items: Vec<UpdatePlanItem>,
}

impl UpdatePlan {
    pub fn new(mut items: Vec<UpdatePlanItem>) -> Result<Self, UpdateModelError> {
        items.sort_by(|left, right| left.name.cmp(&right.name));
        if let Some(duplicate) = items
            .windows(2)
            .find(|pair| pair[0].name == pair[1].name)
            .map(|pair| pair[0].name.to_string())
        {
            return Err(UpdateModelError::DuplicateSkill(duplicate));
        }
        Ok(Self { items })
    }

    pub fn items(&self) -> &[UpdatePlanItem] {
        &self.items
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdatePlanV1 {
    items: Vec<UpdatePlanItem>,
}

impl UpdatePlanV1 {
    pub fn new(plan: UpdatePlan) -> Self {
        Self { items: plan.items }
    }

    pub fn items(&self) -> &[UpdatePlanItem] {
        &self.items
    }

    pub fn has_changes(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(
                item.relation,
                UpdateRelation::Available { .. }
                    | UpdateRelation::Behind { .. }
                    | UpdateRelation::Diverged { .. }
            )
        })
    }

    pub fn is_incomplete(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item.relation, UpdateRelation::Unavailable { .. }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UpdatePlanV1Wire {
    items: Vec<UpdatePlanItem>,
}

impl<'de> Deserialize<'de> for UpdatePlanV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UpdatePlanV1Wire::deserialize(deserializer)?;
        UpdatePlan::new(wire.items)
            .map(Self::new)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateModelError {
    DuplicateSkill(String),
    InvalidCommitSha(String),
    InvalidCommitHistory,
    InvalidComparison,
}

impl fmt::Display for UpdateModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSkill(name) => {
                write!(formatter, "duplicate Skill in update plan: {name}")
            }
            Self::InvalidCommitSha(_) => formatter.write_str("invalid Git commit in update plan"),
            Self::InvalidCommitHistory => {
                formatter.write_str("invalid Git commit history in update plan")
            }
            Self::InvalidComparison => formatter.write_str("invalid Git update comparison"),
        }
    }
}

impl std::error::Error for UpdateModelError {}
