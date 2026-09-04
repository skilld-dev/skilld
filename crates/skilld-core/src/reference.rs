//! Skill references for `skilld run` and `skilld add`.
//!
//! One argument names either one Skill or a set of Skills. The set forms are
//! the refs skilld.dev prints: a Repository, a curator, or a collection.
//! Parsing happens once here. Every later step trusts the tagged value.

use std::fmt;

use crate::remote::{RemoteError, valid_owner, valid_repository};

/// One parsed `skilld run` or `skilld add` argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillRef {
    /// One Skill, in any form `skilld install` accepts.
    Skill(String),
    /// A ref that names more than one Skill.
    Many(MultiSkillRef),
}

/// A ref that names a set of Skills.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MultiSkillRef {
    /// Every Skill one GitHub Repository carries: `gh:OWNER/REPOSITORY`.
    Repository { owner: String, repository: String },
    /// Every Skill one curator's collections name: `@LOGIN`.
    Curator { login: String },
    /// Every Skill one collection names: `@LOGIN/SLUG`.
    Collection { login: String, slug: String },
}

/// One Skill a multi-skill ref names, as skilld.dev lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListedSkill {
    pub name: String,
    pub owner: String,
    pub repository: String,
    pub description: Option<String>,
}

impl ListedSkill {
    /// The hosted selector `skilld run` and `skilld install` accept.
    pub fn selector(&self) -> String {
        format!("skilld:{}/{}/{}", self.owner, self.repository, self.name)
    }
}

/// Every Skill one multi-skill ref names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillListing {
    pub reference: MultiSkillRef,
    pub items: Vec<ListedSkill>,
}

impl SkillRef {
    /// Sort one argument into a single Skill source or a multi-skill ref.
    ///
    /// A single Skill source passes through untouched. `skilld install` parses
    /// it later, with its own rules for local paths and remote selectors.
    pub fn parse(value: &str) -> Result<Self, RemoteError> {
        let value = value.trim();
        if let Some(rest) = value.strip_prefix('@') {
            return parse_handle(rest).map(Self::Many);
        }
        if let Some(rest) = value.strip_prefix("gh:") {
            return parse_repository(rest, true).map(Self::Many);
        }
        if let Some(rest) = value.strip_prefix("github:")
            && rest.matches('/').count() == 1
        {
            return parse_repository(rest, false).map(Self::Many);
        }
        if value.starts_with("npm:") {
            return Err(invalid(
                "npm: references are not supported. Use gh:OWNER/REPOSITORY for a Repository, or skilld:OWNER/REPOSITORY/SKILL for one Skill.",
            ));
        }
        Ok(Self::Skill(value.to_owned()))
    }
}

impl MultiSkillRef {
    /// The kind word skilld prints and emits in JSON.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Repository { .. } => "repository",
            Self::Curator { .. } => "curator",
            Self::Collection { .. } => "collection",
        }
    }

    /// The shortest form that parses back to this ref.
    pub fn canonical(&self) -> String {
        match self {
            Self::Repository { owner, repository } => format!("gh:{owner}/{repository}"),
            Self::Curator { login } => format!("@{login}"),
            Self::Collection { login, slug } => format!("@{login}/{slug}"),
        }
    }
}

impl fmt::Display for MultiSkillRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

fn parse_handle(rest: &str) -> Result<MultiSkillRef, RemoteError> {
    const GUIDANCE: &str = "Use @LOGIN for a curator, or @LOGIN/SLUG for one collection.";
    let mut parts = rest.split('/');
    let login = parts.next().unwrap_or_default();
    if !valid_owner(login) {
        return Err(invalid(format!("the curator login is invalid. {GUIDANCE}")));
    }
    match (parts.next(), parts.next()) {
        (None, None) => Ok(MultiSkillRef::Curator {
            login: login.to_owned(),
        }),
        (Some(slug), None) if valid_slug(slug) => Ok(MultiSkillRef::Collection {
            login: login.to_owned(),
            slug: slug.to_owned(),
        }),
        (Some(_), None) => Err(invalid(format!(
            "the collection slug is invalid. {GUIDANCE}"
        ))),
        _ => Err(invalid(format!(
            "a collection reference has one slug. {GUIDANCE}"
        ))),
    }
}

fn parse_repository(rest: &str, short_prefix: bool) -> Result<MultiSkillRef, RemoteError> {
    const GUIDANCE: &str =
        "Use gh:OWNER/REPOSITORY for a Repository, or skilld:OWNER/REPOSITORY/SKILL for one Skill.";
    if rest.contains('#') {
        return Err(invalid(format!(
            "a Repository reference takes no #reference. {GUIDANCE}"
        )));
    }
    let mut parts = rest.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts
        .next()
        .map(|value| value.trim_end_matches(".git"))
        .unwrap_or_default();
    if parts.next().is_some() {
        let prefix = if short_prefix { "gh:" } else { "github:" };
        return Err(invalid(format!(
            "{prefix}OWNER/REPOSITORY names every Skill in a Repository. {GUIDANCE}"
        )));
    }
    if !valid_owner(owner) || !valid_repository(repository) {
        return Err(invalid(format!(
            "the GitHub Repository owner or name is invalid. {GUIDANCE}"
        )));
    }
    Ok(MultiSkillRef::Repository {
        owner: owner.to_owned(),
        repository: repository.to_owned(),
    })
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn invalid(message: impl Into<String>) -> RemoteError {
    RemoteError::new("INVALID_SOURCE", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn many(value: &str) -> MultiSkillRef {
        match SkillRef::parse(value).unwrap() {
            SkillRef::Many(reference) => reference,
            SkillRef::Skill(skill) => panic!("{value} parsed as one Skill: {skill}"),
        }
    }

    #[test]
    fn parses_every_multi_skill_form() {
        let repository = MultiSkillRef::Repository {
            owner: "skilld-dev".to_owned(),
            repository: "skills".to_owned(),
        };
        assert_eq!(many("gh:skilld-dev/skills"), repository);
        assert_eq!(many("gh:skilld-dev/skills.git"), repository);
        assert_eq!(many("github:skilld-dev/skills"), repository);
        assert_eq!(
            many("@harlan-zw"),
            MultiSkillRef::Curator {
                login: "harlan-zw".to_owned()
            }
        );
        assert_eq!(
            many("@harlan-zw/nuxt"),
            MultiSkillRef::Collection {
                login: "harlan-zw".to_owned(),
                slug: "nuxt".to_owned(),
            }
        );
    }

    #[test]
    fn single_skill_forms_pass_through_unchanged() {
        for value in [
            "skilld:skilld-dev/skills/vue",
            "github:skilld-dev/skilld/skills/skilld",
            "github:skilld-dev/skilld/skills/skilld#branch:main",
            "https://github.com/skilld-dev/skilld/tree/main/skills/skilld",
            "./skills/vue",
            "/srv/skills/vue",
            "skilld",
        ] {
            assert_eq!(
                SkillRef::parse(value).unwrap(),
                SkillRef::Skill(value.to_owned())
            );
        }
    }

    #[test]
    fn rejects_ambiguous_and_unsupported_forms() {
        let cases = [
            (
                "gh:skilld-dev/skills/vue",
                "names every Skill in a Repository",
            ),
            ("gh:skilld-dev", "owner or name is invalid"),
            ("gh:skilld-dev/skills#branch:main", "takes no #reference"),
            (
                "github:skilld-dev/skills#branch:main",
                "takes no #reference",
            ),
            ("@", "curator login is invalid"),
            ("@harlan-zw/nuxt/extra", "one slug"),
            ("@harlan-zw/", "collection slug is invalid"),
            ("@-bad", "curator login is invalid"),
            ("npm:vue", "npm: references are not supported"),
            ("gh:skilld-dev/skills\u{1b}[0m", "owner or name is invalid"),
            ("@harlan\u{202e}zw", "curator login is invalid"),
        ];
        for (value, expected) in cases {
            let error = SkillRef::parse(value).unwrap_err();
            assert_eq!(error.code, "INVALID_SOURCE", "{value}");
            assert!(
                error.message.contains(expected),
                "{value}: {}",
                error.message
            );
        }
    }

    #[test]
    fn canonical_forms_round_trip() {
        for value in ["gh:skilld-dev/skills", "@harlan-zw", "@harlan-zw/nuxt"] {
            assert_eq!(many(value).canonical(), value);
        }
        assert_eq!(many("github:a/b").canonical(), "gh:a/b");
    }
}
