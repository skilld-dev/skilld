use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use skilld_core::{
    AgentTargetId, CommitSha, InstallMode, InstallScope, InstallSource, LockedDeclarationSkill,
    LockedSource, LockedTarget, RemoteSelector, SkillName, SourceRef, SourceStatus,
};
use skilld_ui::Line;

use crate::{
    CommandError, LocalHost, PreparedStoreInstall, StoreError, TargetInstall, materialize_remote,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    name: SkillName,
    agents: Vec<AgentTargetId>,
    mode: InstallMode,
    skills: BTreeMap<SkillName, ManifestSkill>,
    #[serde(default)]
    requires: BTreeMap<SkillName, Vec<SkillName>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestSkill {
    source: String,
}

enum Source {
    Local(PathBuf),
    Remote {
        selector: RemoteSelector,
        commit: CommitSha,
    },
}

fn matches_source(source: &Source, locked: &LockedSource) -> bool {
    match (source, locked) {
        (Source::Local(path), LockedSource::Local { path: locked }) => path == Path::new(locked),
        (
            Source::Remote { selector, commit },
            LockedSource::Remote {
                source, commit_sha, ..
            },
        ) => source == &selector.canonical() && commit_sha == commit.as_str(),
        _ => false,
    }
}

fn manifest_error(message: impl Into<String>) -> CommandError {
    CommandError::operation("INVALID_MANIFEST", message)
}

fn read_manifest(path: &Path) -> Result<(Manifest, BTreeMap<SkillName, Source>), CommandError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| manifest_error(format!("cannot read the Skill manifest: {error}")))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(manifest_error("the Skill manifest must be a regular file"));
    }
    let bytes = fs::read(path).map_err(|error| CommandError::filesystem(error.to_string()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| manifest_error(format!("the Skill manifest is invalid: {error}")))?;
    if manifest.version != 1 || manifest.skills.is_empty() || manifest.agents.is_empty() {
        return Err(manifest_error(
            "the Skill manifest needs version 1, Skills, and Agent targets",
        ));
    }
    if manifest.agents.iter().collect::<BTreeSet<_>>().len() != manifest.agents.len() {
        return Err(manifest_error("the Skill manifest repeats an Agent target"));
    }
    for (consumer, dependencies) in &manifest.requires {
        let mut seen = BTreeSet::new();
        for dependency in dependencies {
            if !manifest.skills.contains_key(dependency) {
                return Err(manifest_error(format!(
                    "{consumer} requires {dependency}, but its source is missing"
                )));
            }
            if !seen.insert(dependency) {
                return Err(manifest_error(format!(
                    "{consumer} repeats required Skill {dependency}"
                )));
            }
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| manifest_error("the Skill manifest needs a parent directory"))?;
    let sources = manifest
        .skills
        .iter()
        .map(|(name, skill)| {
            let source = match InstallSource::parse(&skill.source) {
                InstallSource::Local(path) => Source::Local(
                    fs::canonicalize(if path.is_absolute() {
                        path
                    } else {
                        parent.join(path)
                    })
                    .map_err(|error| CommandError::filesystem(error.to_string()))?,
                ),
                InstallSource::Remote(source) => {
                    let selector = RemoteSelector::parse(&source).map_err(CommandError::remote)?;
                    let Some(SourceRef::Commit { value }) = &selector.source().r#ref else {
                        return Err(manifest_error(format!(
                            "Skill {name} needs an exact remote commit"
                        )));
                    };
                    let commit = CommitSha::parse(value.clone())
                        .map_err(|error| manifest_error(error.to_string()))?;
                    Source::Remote { selector, commit }
                }
                _ => {
                    return Err(manifest_error(
                        "the Skill manifest accepts local sources and exact hosted remote sources",
                    ));
                }
            };
            Ok((name.clone(), source))
        })
        .collect::<Result<_, CommandError>>()?;
    Ok((manifest, sources))
}

pub struct SyncRequest {
    pub manifest: PathBuf,
    pub scope: InstallScope,
    pub check: bool,
    pub adopt: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub current: bool,
    pub changed: Vec<String>,
    pub requirements_changed: bool,
    pub declarations_changed: bool,
}

impl SyncReport {
    pub(crate) fn lines(&self) -> Vec<Line> {
        if self.current {
            vec![Line::success("Declared Skills match their Agent targets.")]
        } else {
            let mut lines: Vec<_> = self
                .changed
                .iter()
                .map(|name| Line::warn(format!("Skill {name} needs sync.")))
                .collect();
            if self.requirements_changed {
                lines.push(Line::warn("Declared Skill requirements need sync."));
            }
            if self.declarations_changed {
                lines.push(Line::warn("Skill declarations need sync."));
            }
            lines
        }
    }
}

pub(crate) fn sync(host: &LocalHost, request: SyncRequest) -> Result<SyncReport, CommandError> {
    let path = if request.manifest.is_absolute() {
        request.manifest
    } else {
        host.project_root.join(request.manifest)
    };
    let (manifest, sources) = read_manifest(&path)?;
    let known = host.known_targets(request.scope)?;
    let targets = manifest
        .agents
        .iter()
        .map(|agent| {
            let target = known
                .iter()
                .find(|target| target.agent == *agent)
                .expect("every Agent target has a known path");
            TargetInstall {
                target: target.clone(),
                mode: manifest.mode,
            }
        })
        .collect::<Vec<_>>();
    let store = host.store(request.scope);
    let snapshot = store.snapshot(&known).map_err(CommandError::store)?;
    let requirements = manifest
        .requires
        .iter()
        .map(|(consumer, dependencies)| {
            (
                consumer.to_string(),
                dependencies
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let requirements_changed =
        snapshot.requirements.get(manifest.name.as_str()) != Some(&requirements);
    let mut changed = Vec::new();
    let mut installs = Vec::new();
    // Keep every remote staging directory alive until the batch commits.
    let mut staging = Vec::new();
    let mut ownership = BTreeMap::new();
    let own_targets = targets
        .iter()
        .map(|target| LockedTarget {
            agent: target.target.agent,
            mode: target.mode,
        })
        .collect::<Vec<_>>();
    for (name, source) in sources {
        let mut targets = targets.clone();
        for (owner, skills) in &snapshot.declarations {
            if owner == manifest.name.as_str() {
                continue;
            }
            let Some(shared) = skills.get(name.as_str()) else {
                continue;
            };
            if !matches_source(&source, &shared.source) {
                return Err(CommandError::store(StoreError::Conflict(format!(
                    "Skill {name} has a different source in declaration {owner}"
                ))));
            }
            for locked in &shared.targets {
                if let Some(target) = targets
                    .iter()
                    .find(|target| target.target.agent == locked.agent)
                {
                    if target.mode != locked.mode {
                        return Err(CommandError::store(StoreError::Conflict(format!(
                            "Skill {name} has a different Agent target mode in declaration {owner}"
                        ))));
                    }
                } else {
                    let target = known
                        .iter()
                        .find(|target| target.agent == locked.agent)
                        .expect("every Agent target has a known path");
                    targets.push(TargetInstall {
                        target: target.clone(),
                        mode: locked.mode,
                    });
                }
            }
        }
        let old = snapshot.skills.get(name.as_str());
        let integrity_changed = match store.verify_content(&name, &known) {
            Ok(_) => false,
            Err(StoreError::NotFound(_) | StoreError::Conflict(_)) if request.check => true,
            Err(StoreError::NotFound(_)) if old.is_none() => true,
            Err(error) => return Err(CommandError::store(error)),
        } || !store.root().join(name.as_str()).join("SKILL.md").is_file();
        let source_changed = match &source {
            Source::Local(path) => {
                let path = fs::canonicalize(path).map_err(|error| CommandError::filesystem(error.to_string()))?;
                if SkillName::from_source(&path).map_err(CommandError::domain)? != name {
                    return Err(manifest_error(format!("the source directory does not match Skill {name}")));
                }
                let digest = store.source_digest(&path).map_err(CommandError::store)?;
                let locked = LockedSource::Local { path: path.to_str().ok_or_else(|| manifest_error("local Skill paths must use UTF-8"))?.to_owned() };
                old.is_none_or(|old| old.source != locked || old.source_status != SourceStatus::Local { content_sha256: digest.clone() })
                    || !store.source_permissions_match(&path, &name).map_err(CommandError::store)?
            }
            Source::Remote { selector, commit } => old.is_none_or(|old| {
                !matches!(&old.source, LockedSource::Remote { source, commit_sha, .. } if source == &selector.canonical() && commit_sha == commit.as_str())
                    || !matches!(old.source_status, SourceStatus::Verified { .. })
            }),
        };
        let targets_changed = old.is_none_or(|old| {
            old.targets.len() != targets.len()
                || targets.iter().any(|target| {
                    !target
                        .target
                        .root
                        .join(name.as_str())
                        .join("SKILL.md")
                        .is_file()
                        || !old.targets.iter().any(|locked| {
                            locked.agent == target.target.agent && locked.mode == target.mode
                        })
                })
        });
        if integrity_changed || source_changed || targets_changed {
            changed.push(name.to_string());
        }
        if request.check {
            let locked_source = match &source {
                Source::Local(path) => Some(LockedSource::Local {
                    path: path
                        .to_str()
                        .ok_or_else(|| manifest_error("local Skill paths must use UTF-8"))?
                        .to_owned(),
                }),
                Source::Remote { .. } => old
                    .filter(|old| matches_source(&source, &old.source))
                    .map(|old| old.source.clone()),
            };
            if let Some(locked_source) = locked_source {
                ownership.insert(
                    name.to_string(),
                    LockedDeclarationSkill {
                        source: locked_source,
                        targets: own_targets.clone(),
                    },
                );
            }
            continue;
        }
        let install = match source {
            Source::Local(path) => {
                let source = fs::canonicalize(path)
                    .map_err(|error| CommandError::filesystem(error.to_string()))?;
                let locked_source = LockedSource::Local {
                    path: source
                        .to_str()
                        .ok_or_else(|| manifest_error("local Skill paths must use UTF-8"))?
                        .to_owned(),
                };
                PreparedStoreInstall {
                    source,
                    locked_source,
                    source_status: None,
                    targets: targets.clone(),
                }
            }
            Source::Remote { selector, commit } => {
                if !source_changed && !integrity_changed {
                    let old = old.expect("a matching source is installed");
                    PreparedStoreInstall {
                        source: store.root().join(name.as_str()),
                        locked_source: old.source.clone(),
                        source_status: Some(old.source_status.clone()),
                        targets: targets.clone(),
                    }
                } else {
                    let prepared = host
                        .remote_provider()?
                        .prepare_exact(&selector, &commit, false)
                        .map_err(CommandError::remote)?;
                    if !matches!(&prepared.locked_source, LockedSource::Remote { source, commit_sha, .. }
                        if source == &selector.canonical() && commit_sha == commit.as_str())
                        || !matches!(prepared.source_status, SourceStatus::Verified { .. })
                    {
                        return Err(manifest_error(format!(
                            "Skill {name} needs verified delivery at its declared commit"
                        )));
                    }
                    let stage = materialize_remote(&prepared)?;
                    if SkillName::from_source(stage.path()).map_err(CommandError::domain)? != name {
                        return Err(manifest_error(format!(
                            "the delivered Skill does not match {name}"
                        )));
                    }
                    let install = PreparedStoreInstall {
                        source: stage.path().to_owned(),
                        locked_source: prepared.locked_source,
                        source_status: Some(prepared.source_status),
                        targets: targets.clone(),
                    };
                    staging.push(stage);
                    install
                }
            }
        };
        // An unchanged managed source needs a temporary copy because the store refuses overlap.
        let install = if install.source.starts_with(store.root()) {
            let stage =
                tempfile::tempdir().map_err(|error| CommandError::filesystem(error.to_string()))?;
            let source = stage.path().join(name.as_str());
            crate::local_store::copy_tree(&install.source, &source).map_err(CommandError::store)?;
            let install = PreparedStoreInstall { source, ..install };
            staging.push(crate::StagedRemote {
                _directory: stage,
                skill: install.source.clone(),
            });
            install
        } else {
            install
        };
        ownership.insert(
            name.to_string(),
            LockedDeclarationSkill {
                source: install.locked_source.clone(),
                targets: own_targets.clone(),
            },
        );
        installs.push(install);
    }
    let declarations_changed =
        snapshot.declarations.get(manifest.name.as_str()) != Some(&ownership);
    if request.check {
        return Ok(SyncReport {
            current: changed.is_empty() && !requirements_changed && !declarations_changed,
            changed,
            requirements_changed,
            declarations_changed,
        });
    }
    if !changed.is_empty() || requirements_changed || declarations_changed {
        store
            .apply_sync_batch(
                installs,
                &snapshot,
                crate::local_store::StoreDeclaration {
                    name: manifest.name.to_string(),
                    skills: ownership,
                    requirements,
                },
                request.adopt,
                &known,
            )
            .map_err(CommandError::store)?;
    }
    Ok(SyncReport {
        current: true,
        changed,
        requirements_changed: false,
        declarations_changed: false,
    })
}
