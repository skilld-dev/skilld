mod lock;
mod remote;
mod target;
mod update;

use std::fmt;
use std::path::{Path, PathBuf};

pub use lock::{
    LockDocument, LockedSkill, LockedSource, LockedTarget, SOURCE_STATUSES, SourceStatus,
};
pub use remote::{
    ArtifactAttestation, ArtifactFile, AttestationSignature, CheckOutcome, CheckResult,
    PreparedFile, RemoteError, RemoteSelector, RepositoryVisibility, ResolvedSource,
    SearchResponse, SearchResult, SignatureAlgorithm, SourceProvider, SourceRef, SourceRequest,
    SourceSelector, TrustedKey, TrustedKeyStatus, TrustedRoot, TrustedRootPin, VerifiedArtifact,
    VerifiedTrustedRoot, parse_search_response, prepare_unverified_files, verify_artifact,
    verify_attestation, verify_trusted_root,
};
use serde::{Deserialize, Serialize};
pub use target::{
    AGENT_TARGETS, AgentTarget, AgentTargetId, GlobalTargetPath, TargetSelection, select_target_ids,
};
pub use update::{
    CommitAuthor, CommitHistory, CommitSha, CommitSummary, NotTrackedReason, UpdateFailure,
    UpdateLatestCommit, UpdateModelError, UpdatePlan, UpdatePlanItem, UpdatePlanV1, UpdateRelation,
    UpdateRetryAfter, classify_update_comparison,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    #[default]
    Copy,
    Symlink,
}

impl InstallMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Symlink => "symlink",
        }
    }

    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "copy" => Ok(Self::Copy),
            "symlink" => Ok(Self::Symlink),
            _ => Err(DomainError::InvalidInstallMode(value.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct SkillName(String);

impl SkillName {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !value.as_bytes().windows(2).any(|pair| pair == b"--");

        if valid {
            Ok(Self(value))
        } else {
            Err(DomainError::InvalidSkillName(value))
        }
    }

    pub fn from_source(source: &Path) -> Result<Self, DomainError> {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DomainError::InvalidSkillPath(source.to_path_buf()))?;
        Self::parse(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SkillName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for SkillName {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<SkillName> for String {
    fn from(value: SkillName) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallScope {
    Project,
    Global,
}

impl InstallScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallSource {
    Local(PathBuf),
    BundledSkilld,
    Remote(String),
    DirectRemote(String),
}

impl InstallSource {
    pub fn parse(value: &str) -> Self {
        if value == "skilld" {
            return Self::BundledSkilld;
        }

        let path = PathBuf::from(value);
        if path.is_absolute() || value.starts_with('.') {
            Self::Local(path)
        } else {
            Self::Remote(value.to_owned())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallPlan {
    pub source: PathBuf,
    pub store: PathBuf,
    pub name: SkillName,
}

impl InstallPlan {
    pub fn local(source: PathBuf, store: PathBuf) -> Result<Self, DomainError> {
        let name = SkillName::from_source(&source)?;
        Ok(Self {
            source,
            store,
            name,
        })
    }

    pub fn destination(&self) -> PathBuf {
        self.store.join(self.name.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallRequest {
    pub source: Option<InstallSource>,
    pub scope: InstallScope,
    pub targets: Vec<AgentTargetId>,
    pub mode: Option<InstallMode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainError {
    InvalidInstallMode(String),
    InvalidSkillName(String),
    InvalidSkillPath(PathBuf),
    InvalidTarget(String),
    TargetRequired,
}

impl DomainError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInstallMode(_) => "INVALID_INSTALL_MODE",
            Self::InvalidSkillName(_) | Self::InvalidSkillPath(_) => "INVALID_SOURCE",
            Self::InvalidTarget(_) => "INVALID_TARGET",
            Self::TargetRequired => "TARGET_REQUIRED",
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInstallMode(mode) => write!(formatter, "invalid install mode: {mode}"),
            Self::InvalidSkillName(name) => write!(formatter, "invalid Skill name: {name}"),
            Self::InvalidSkillPath(path) => {
                write!(formatter, "invalid local Skill path: {}", path.display())
            }
            Self::InvalidTarget(target) => write!(formatter, "unknown Agent target: {target}"),
            Self::TargetRequired => write!(
                formatter,
                "select an Agent target with --agent or configure agent.targets"
            ),
        }
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_name_matches_the_agent_skills_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/fixtures/skill-conformance/skill-name.json"
        ))
        .unwrap();

        for name in fixture["valid"].as_array().unwrap() {
            assert!(SkillName::parse(name.as_str().unwrap()).is_ok());
        }
        for name in fixture["invalid"].as_array().unwrap() {
            assert!(SkillName::parse(name.as_str().unwrap()).is_err());
        }
        let maximum = fixture["maximumLength"].as_u64().unwrap() as usize;
        assert!(SkillName::parse("a".repeat(maximum)).is_ok());
        assert!(SkillName::parse("a".repeat(maximum + 1)).is_err());
    }

    #[test]
    fn target_selection_uses_explicit_then_detected_then_configured() {
        let explicit = [AgentTargetId::Cursor];
        let detected = [AgentTargetId::Codex];
        let configured = [AgentTargetId::ClaudeCode];

        assert_eq!(
            select_target_ids(&explicit, &detected, &configured),
            Ok(TargetSelection::Explicit(explicit.to_vec()))
        );
        assert_eq!(
            select_target_ids(&[], &detected, &configured),
            Ok(TargetSelection::Detected(detected.to_vec()))
        );
        assert_eq!(
            select_target_ids(&[], &[], &configured),
            Ok(TargetSelection::Configured(configured.to_vec()))
        );
        assert_eq!(
            select_target_ids(&[], &[], &[]),
            Err(DomainError::TargetRequired)
        );
    }

    #[test]
    fn source_status_values_match_the_v1_contract() {
        let fixture: Vec<String> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/v3-rust/v1/source-status.json"
        ))
        .unwrap();

        assert_eq!(fixture, SOURCE_STATUSES);
    }

    #[test]
    fn bundled_skilld_has_an_explicit_source() {
        assert_eq!(InstallSource::parse("skilld"), InstallSource::BundledSkilld);
    }
}
