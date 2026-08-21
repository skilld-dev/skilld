use std::fmt;
use std::num::NonZeroU64;

use serde::{Deserialize, Deserializer, Serialize};

use crate::SkillName;

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
pub struct UpdateFailure {
    pub code: String,
    pub message: String,
}

impl UpdateFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
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
}

impl UpdatePlanItem {
    pub fn new(name: SkillName, relation: UpdateRelation) -> Self {
        Self { name, relation }
    }

    pub const fn name(&self) -> &SkillName {
        &self.name
    }

    pub const fn relation(&self) -> &UpdateRelation {
        &self.relation
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
pub struct UpdateCheckV1 {
    items: Vec<UpdatePlanItem>,
}

impl UpdateCheckV1 {
    pub fn new(plan: UpdatePlan) -> Self {
        Self { items: plan.items }
    }

    pub fn items(&self) -> &[UpdatePlanItem] {
        &self.items
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UpdateCheckV1Wire {
    items: Vec<UpdatePlanItem>,
}

impl<'de> Deserialize<'de> for UpdateCheckV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UpdateCheckV1Wire::deserialize(deserializer)?;
        UpdatePlan::new(wire.items)
            .map(Self::new)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateModelError {
    DuplicateSkill(String),
    InvalidCommitSha(String),
    InvalidComparison,
}

impl fmt::Display for UpdateModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSkill(name) => {
                write!(formatter, "duplicate Skill in update plan: {name}")
            }
            Self::InvalidCommitSha(_) => formatter.write_str("invalid Git commit in update plan"),
            Self::InvalidComparison => formatter.write_str("invalid Git update comparison"),
        }
    }
}

impl std::error::Error for UpdateModelError {}
