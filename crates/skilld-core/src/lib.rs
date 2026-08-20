use std::fmt;
use std::path::{Path, PathBuf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SOURCE_STATUSES: [&str; 3] = ["verified", "local", "unverified"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceStatus {
    Verified,
    Local,
    Unverified,
}

impl SourceStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Local => "local",
            Self::Unverified => "unverified",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallScope {
    Project,
    Global,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallSource {
    Local(PathBuf),
    BundledSkilld,
    Remote(String),
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
pub enum DomainError {
    InvalidSkillName(String),
    InvalidSkillPath(PathBuf),
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSkillName(name) => write!(formatter, "invalid Skill name: {name}"),
            Self::InvalidSkillPath(path) => {
                write!(formatter, "invalid local Skill path: {}", path.display())
            }
        }
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_name_rejects_path_components() {
        assert_eq!(
            SkillName::parse("../outside"),
            Err(DomainError::InvalidSkillName("../outside".to_owned()))
        );
    }

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
    fn bundled_skilld_has_an_explicit_source() {
        assert_eq!(InstallSource::parse("skilld"), InstallSource::BundledSkilld);
    }

    #[test]
    fn source_status_values_match_the_v1_contract() {
        let fixture: Vec<String> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/v3-rust/v1/source-status.json"
        ))
        .unwrap();
        assert_eq!(
            [
                SourceStatus::Verified.as_str(),
                SourceStatus::Local.as_str(),
                SourceStatus::Unverified.as_str(),
            ],
            fixture.as_slice()
        );
        assert_eq!(fixture, SOURCE_STATUSES);
    }
}
