use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{AgentTargetId, InstallMode};

pub const SOURCE_STATUSES: [&str; 3] = ["verified", "local", "unverified"];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "camelCase")]
pub enum SourceStatus {
    Verified {
        artifact_id: String,
        content_sha256: String,
        installed_sha256: String,
        attestation_key_id: String,
    },
    Local {
        content_sha256: String,
    },
    Unverified {
        content_sha256: String,
        installed_sha256: String,
    },
}

impl SourceStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Verified { .. } => "verified",
            Self::Local { .. } => "local",
            Self::Unverified { .. } => "unverified",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "camelCase")]
pub enum LockedSource {
    Local {
        path: String,
    },
    BundledSkilld,
    Remote {
        source: String,
        commit_sha: String,
        skill_path: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LockedTarget {
    pub agent: AgentTargetId,
    pub mode: InstallMode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LockedSkill {
    pub source: LockedSource,
    pub source_status: SourceStatus,
    pub targets: Vec<LockedTarget>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LockDocument {
    pub version: u8,
    pub transaction_id: String,
    pub skills: BTreeMap<String, LockedSkill>,
}

impl Default for LockDocument {
    fn default() -> Self {
        Self {
            version: 1,
            transaction_id: "initial".to_owned(),
            skills: BTreeMap::new(),
        }
    }
}
