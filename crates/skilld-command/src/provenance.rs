//! Where a remote Skill came from: the human, the Repository, and the exact file.
//!
//! Every surface that shows a remote Skill points at the SKILL.md the author
//! committed. The lockfile records the Repository, path, and commit; this module
//! turns those into one line a person can read and one URL they can open.

use skilld_core::{LockedSource, RemoteSelector};

use crate::CommandError;

/// The Repository, path, and commit that one remote Skill was read from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteProvenance {
    pub owner: String,
    pub repository: String,
    pub skill_path: String,
    pub commit_sha: String,
    /// The SKILL.md file at the exact commit, on github.com.
    pub source_url: String,
}

impl RemoteProvenance {
    pub fn new(
        owner: impl Into<String>,
        repository: impl Into<String>,
        skill_path: impl Into<String>,
        commit_sha: impl Into<String>,
    ) -> Result<Self, CommandError> {
        let owner = owner.into();
        let repository = repository.into();
        let skill_path = skill_path.into();
        let commit_sha = commit_sha.into();
        let source_url = source_url(&owner, &repository, &skill_path, &commit_sha)?;
        Ok(Self {
            owner,
            repository,
            skill_path,
            commit_sha,
            source_url,
        })
    }

    /// Read the provenance a lockfile entry recorded. Local and bundled
    /// Skills have no remote source, so they carry none.
    pub fn from_locked(source: &LockedSource) -> Result<Option<Self>, CommandError> {
        let LockedSource::Remote {
            source,
            commit_sha,
            skill_path,
        } = source
        else {
            return Ok(None);
        };
        let selector = RemoteSelector::parse(source).map_err(CommandError::remote)?;
        Self::new(
            selector.source().owner.as_str(),
            selector.source().repository.as_str(),
            skill_path.as_str(),
            commit_sha.as_str(),
        )
        .map(Some)
    }

    /// `owner/repository`, the way GitHub names it.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repository)
    }

    /// The first seven characters of the commit, for reading. The URL keeps the full commit.
    pub fn short_commit(&self) -> &str {
        self.commit_sha.get(..7).unwrap_or(&self.commit_sha)
    }

    /// One line: the Skill, the human who published it, and the commit.
    pub fn headline(&self, name: &str) -> String {
        format!("{name} · {} @ {}", self.slug(), self.short_commit())
    }
}

/// State what the status covers, on every status.
///
/// A verified Artifact proves where the bytes came from. It says nothing about
/// what the instructions ask an Agent to do, and the output must not imply it.
pub fn source_status_caution(status: &str) -> &'static str {
    match status {
        "verified" => {
            "skilld checked where this Skill came from, not what it asks you to do.\nRead it before you follow it.\n"
        }
        "unverified" => "skilld did not check this source. Read this Skill before you follow it.\n",
        _ => "Read this Skill before you follow it.\n",
    }
}

fn source_url(
    owner: &str,
    repository: &str,
    skill_path: &str,
    commit_sha: &str,
) -> Result<String, CommandError> {
    let failed = || CommandError::service("the GitHub Skill URL could not be built");
    let mut url = url::Url::parse("https://github.com/").map_err(|_| failed())?;
    {
        let mut segments = url.path_segments_mut().map_err(|_| failed())?;
        segments
            .push(owner)
            .push(repository)
            .push("blob")
            .push(commit_sha);
        for segment in skill_path.split('/').filter(|segment| !segment.is_empty()) {
            segments.push(segment);
        }
        segments.push(crate::run::INSTRUCTIONS_FILE);
    }
    Ok(url.into())
}
