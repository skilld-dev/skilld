//! Reviewed, reversible changes. Discovery never calls this module's apply path.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::doctor::{
    DoctorOptions, DoctorOwner, DoctorReport, DoctorSkill, ScanContext, ScanPhase, ScanProgress,
    SourceMatch,
};
use crate::doctor_metadata::{ForeignRecord, fingerprint, foreign_lock_without};
use crate::{
    CommandError, LocalHost, LocalStore, PreparedRemoteSkill, ResolvedTarget, TargetInstall,
};
use serde::Serialize;
use skilld_core::{InstallMode, InstallScope, LockedSource, RemoteSelector, SkillName};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoctorAction {
    Migrate,
    Remove,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionPreview {
    pub action: String,
    pub name: String,
    pub paths: Vec<PathBuf>,
    pub source: Option<String>,
    pub commit: Option<String>,
    pub changes_content: bool,
    pub behaviors: Vec<String>,
}

pub struct DoctorPlan {
    pub preview: ActionPreview,
    action: DoctorAction,
    copies: Vec<DoctorSkill>,
    paths: Vec<PathBuf>,
    observed_paths: Vec<(PathBuf, PathBuf, Option<PathBuf>)>,
    absent_targets: Vec<PathBuf>,
    record: Option<ForeignRecord>,
    store: LocalStore,
    targets: Vec<TargetInstall>,
    prepared: Option<(PreparedRemoteSkill, crate::StagedRemote)>,
    allowed_behaviors: Vec<String>,
}

#[derive(Debug)]
pub struct DoctorApplied {
    pub backup: PathBuf,
    pub message: String,
}

fn failure(message: impl Into<String>) -> CommandError {
    CommandError::config(message)
}

fn same_record(a: &ForeignRecord, b: &ForeignRecord) -> bool {
    a.lockfile == b.lockfile && a.name == b.name
}

fn source_selector(record: &ForeignRecord) -> Result<RemoteSelector, CommandError> {
    if record.source_type != "github" {
        return Err(failure(
            "Migration requires a GitHub source. Keep or remove this Skill.",
        ));
    }
    let mut source = if let Some(path) = &record.skill_path {
        let path = path.strip_suffix("/SKILL.md").unwrap_or(path);
        let path = if path == "SKILL.md" { "." } else { path };
        format!("github:{}/{}", record.source, path)
    } else {
        format!("{}/{}", record.source, record.name)
    };
    if let Some(reference) = &record.revision {
        source.push('#');
        source.push_str(reference);
    }
    RemoteSelector::parse(&source).map_err(CommandError::remote)
}

impl LocalHost {
    pub fn doctor_context(&self) -> Result<ScanContext, CommandError> {
        let lock = std::env::var_os("XDG_STATE_HOME")
            .filter(|s| !s.is_empty())
            .map(|p| PathBuf::from(p).join("skills/.skill-lock.json"))
            .unwrap_or_else(|| self.target_roots.home.join(".agents/.skill-lock.json"));
        let mut global_targets = self.known_targets(InstallScope::Global)?;
        global_targets.push(
            ResolvedTarget::new(
                skilld_core::AgentTargetId::Codex,
                self.target_roots.home.join(".codex/skills"),
            )
            .map_err(CommandError::store)?,
        );
        Ok(ScanContext {
            home: self.target_roots.home.clone(),
            global_store: self.global_root.join("skills"),
            global_targets,
            skills_sh_global_lock: lock,
        })
    }

    pub fn doctor_scan(&self, options: &DoctorOptions) -> Result<DoctorReport, CommandError> {
        self.doctor_scan_with_progress(options, &mut |_| {})
    }

    pub fn doctor_scan_with_progress(
        &self,
        options: &DoctorOptions,
        progress: &mut impl FnMut(ScanProgress),
    ) -> Result<DoctorReport, CommandError> {
        let mut report =
            crate::doctor::scan_with_progress(options, &self.doctor_context()?, progress)?;
        if options.check_sources {
            let mut checked = 0;
            let total = report
                .skills
                .iter()
                .filter(|s| matches!(s.owner, DoctorOwner::SkillsSh { .. } | DoctorOwner::Unknown))
                .count()
                .min(20);
            let mut status = ScanProgress::from_report(
                &report,
                ScanPhase::Sources {
                    completed: 0,
                    total,
                },
                PathBuf::new(),
                report.skills.len(),
            );
            for skill in &mut report.skills {
                if checked >= 20 {
                    break;
                }
                if !matches!(
                    skill.owner,
                    DoctorOwner::SkillsSh { .. } | DoctorOwner::Unknown
                ) {
                    continue;
                }
                status.phase = ScanPhase::Sources {
                    completed: checked,
                    total,
                };
                status.current_path = skill.canonical_path.clone();
                progress(status.clone());
                checked += 1;
                let selector = match &skill.owner {
                    DoctorOwner::SkillsSh { record } => source_selector(record),
                    DoctorOwner::Unknown => {
                        let result = self
                            .remote_provider()?
                            .search(&skill.name, 5)
                            .map_err(CommandError::remote);
                        match result {
                            Ok(results) => {
                                let matches = results
                                    .items
                                    .into_iter()
                                    .filter(|r| r.name == skill.name)
                                    .collect::<Vec<_>>();
                                if matches.len() != 1 {
                                    skill.source_match = Some(SourceMatch {
                                        source: skill.name.clone(),
                                        commit: None,
                                        result: "No unique repository candidate.".into(),
                                    });
                                    continue;
                                }
                                matches[0].selector().map_err(CommandError::remote)
                            }
                            Err(e) => {
                                skill.source_match = Some(SourceMatch {
                                    source: skill.name.clone(),
                                    commit: None,
                                    result: e.to_string(),
                                });
                                continue;
                            }
                        }
                    }
                    _ => continue,
                };
                let result = selector.and_then(|selector| {
                    let prepared = self
                        .remote_provider()?
                        .prepare(&selector, false)
                        .map_err(CommandError::remote)?;
                    let staged = crate::materialize_remote(&prepared)?;
                    let current = fingerprint(staged.path())?;
                    Ok((prepared, current))
                });
                skill.source_match = Some(match result {
                    Ok((prepared, current)) => {
                        let (source, commit) = match prepared.locked_source {
                            LockedSource::Remote {
                                source, commit_sha, ..
                            } => (source, Some(commit_sha)),
                            _ => (skill.name.clone(), None),
                        };
                        SourceMatch {
                            source,
                            commit,
                            result: if skill
                                .fingerprint
                                .as_ref()
                                .is_some_and(|f| f.git_tree == current.git_tree)
                            {
                                "Exact directory match.".into()
                            } else {
                                "Source contents differ. Review before replacement.".into()
                            },
                        }
                    }
                    Err(e) => SourceMatch {
                        source: skill.name.clone(),
                        commit: None,
                        result: e.to_string(),
                    },
                });
            }
        }
        Ok(report)
    }

    pub fn doctor_plan(
        &self,
        report: &DoctorReport,
        index: usize,
        action: DoctorAction,
    ) -> Result<DoctorPlan, CommandError> {
        let selected = report
            .skills
            .get(index)
            .ok_or_else(|| failure("Select a Skill first."))?;
        let record = match &selected.owner {
            DoctorOwner::Pnpm { .. } => {
                return Err(failure(
                    "pnpm owns this Skill. Update or remove its package with pnpm.",
                ));
            }
            DoctorOwner::SkillsSh { record } => Some(record.clone()),
            DoctorOwner::Unknown if action == DoctorAction::Remove => None,
            _ => {
                return Err(failure(
                    "This action requires a skills.sh install or an unknown Agent target.",
                ));
            }
        };
        let copies = report
            .skills
            .iter()
            .filter(|s| match (&record, &s.owner) {
                (Some(a), DoctorOwner::SkillsSh { record: b }) => same_record(a, b),
                (None, _) => s.canonical_path == selected.canonical_path,
                _ => false,
            })
            .cloned()
            .collect::<Vec<_>>();
        let locations = copies
            .iter()
            .flat_map(|s| s.paths.iter())
            .filter(|p| p.agent.is_some())
            .collect::<Vec<_>>();
        let first = locations
            .first()
            .ok_or_else(|| failure("The Skill has no Agent target to change."))?;
        let scope_root = first.project_root.clone();
        if locations.iter().any(|p| p.project_root != scope_root) {
            return Err(failure(
                "These paths span different scopes. Scan and change one scope at a time.",
            ));
        }
        let store = LocalStore::new(
            scope_root
                .as_ref()
                .map_or_else(|| self.global_root.join("skills"), |p| p.join(".skills")),
        );
        if fs::symlink_metadata(store.root().join(&selected.name)).is_ok() {
            return Err(failure(
                "skilld already contains this name. Resolve that installation first.",
            ));
        }
        // A metadata entry cannot justify changing an Agent target outside the scan.
        let all_targets = if let Some(root) = &scope_root {
            skilld_core::AGENT_TARGETS
                .iter()
                .map(|t| ResolvedTarget::new(t.id, root.join(t.project_skills_dir)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommandError::store)?
        } else {
            self.doctor_context()?.global_targets
        };
        let mut paths = BTreeSet::new();
        let mut observed_paths = vec![];
        let mut targets = BTreeMap::new();
        let managed_targets = if scope_root.is_some() {
            all_targets.clone()
        } else {
            self.known_targets(InstallScope::Global)?
        };
        for location in &locations {
            let parent = location
                .path
                .parent()
                .ok_or_else(|| failure("The Agent target has no parent."))?;
            let real_parent =
                fs::canonicalize(parent).map_err(|e| CommandError::filesystem(e.to_string()))?;
            let filename = location
                .path
                .file_name()
                .ok_or_else(|| failure("The Agent target has no Skill name."))?;
            if action == DoctorAction::Migrate && filename != selected.name.as_str() {
                return Err(failure(
                    "An Agent target uses another name. Migration cannot rename it.",
                ));
            }
            let path = real_parent.join(filename);
            let agent = location.agent.expect("filtered Agent target");
            if action == DoctorAction::Migrate
                && !managed_targets
                    .iter()
                    .any(|t| t.agent == agent && t.root == parent && t.root == real_parent)
            {
                return Err(failure(
                    "Migration requires a standard Agent target root. Keep or remove this installation.",
                ));
            }
            if let Some(previous) = targets.get(&agent) {
                let previous: &TargetInstall = previous;
                if action == DoctorAction::Migrate && previous.target.root != real_parent {
                    return Err(failure(
                        "One Agent has multiple target roots. Resolve them separately.",
                    ));
                }
            }
            let mode = if fs::symlink_metadata(&path)
                .map_err(|e| CommandError::filesystem(e.to_string()))?
                .file_type()
                .is_symlink()
            {
                InstallMode::Symlink
            } else {
                InstallMode::Copy
            };
            let link = if mode == InstallMode::Symlink {
                Some(fs::read_link(&path).map_err(|e| CommandError::filesystem(e.to_string()))?)
            } else {
                None
            };
            observed_paths.push((location.path.clone(), path.clone(), link));
            targets.insert(
                agent,
                TargetInstall {
                    target: ResolvedTarget::new(agent, real_parent).map_err(CommandError::store)?,
                    mode,
                },
            );
            paths.insert(path);
        }
        let mut absent_targets = vec![store.root().join(&selected.name)];
        for target in all_targets {
            let path = target.root.join(&selected.name);
            if fs::symlink_metadata(&path).is_ok() {
                let real = fs::canonicalize(&target.root)
                    .map_err(|e| CommandError::filesystem(e.to_string()))?
                    .join(&selected.name);
                if !paths.contains(&real) {
                    return Err(failure(format!(
                        "Another Agent target was outside this scan: {}. Scan the complete scope first.",
                        path.display()
                    )));
                }
            } else {
                absent_targets.push(path);
            }
        }
        if let Some(record) = &record {
            foreign_lock_without(&record.lockfile, record)?;
            let document: serde_json::Value = serde_json::from_slice(
                &fs::read(&record.lockfile).map_err(|e| CommandError::filesystem(e.to_string()))?,
            )
            .map_err(|e| failure(e.to_string()))?;
            if document["skills"][&record.name]["subagents"]
                .as_array()
                .is_some_and(|v| !v.is_empty())
            {
                return Err(failure(
                    "This Skill has subagent targets. Use skills.sh to change those targets first.",
                ));
            }
        }
        let mut preview = ActionPreview {
            action: if action == DoctorAction::Migrate {
                "Migrate to skilld"
            } else {
                "Remove"
            }
            .into(),
            name: selected.name.clone(),
            paths: paths.iter().cloned().collect(),
            source: None,
            commit: None,
            changes_content: false,
            behaviors: vec![],
        };
        let mut allowed_behaviors = vec![];
        let prepared = if action == DoctorAction::Migrate {
            let record = record.as_ref().expect("migration has a record");
            let selected_hash = selected
                .fingerprint
                .as_ref()
                .ok_or_else(|| failure("The Skill directory could not be read."))?;
            if copies.iter().any(|s| {
                s.fingerprint
                    .as_ref()
                    .is_none_or(|f| f.git_tree != selected_hash.git_tree)
            }) {
                return Err(failure(
                    "Installed copies differ. Keep them or remove selected copies before migration.",
                ));
            }
            let selector = source_selector(record)?;
            let prepared = self
                .remote_provider()?
                .prepare(&selector, false)
                .map_err(CommandError::remote)?;
            if prepared
                .skill_name()
                .map_err(CommandError::remote)?
                .as_str()
                != selected.name
            {
                return Err(failure(
                    "The source uses another Skill name. Migration cannot rename Agent targets.",
                ));
            }
            let staged = crate::materialize_remote(&prepared)?;
            let current = fingerprint(staged.path())?;
            preview.changes_content = selected_hash.git_tree != current.git_tree;
            // Project computedHash uses locale-specific JS ordering. Exact remote bytes
            // establish identity without pretending that hash has Git tree semantics.
            if preview.changes_content
                && record.expected_tree.as_ref() != Some(&selected_hash.git_tree)
            {
                return Err(failure(
                    "Source contents differ and the installed directory does not match a recorded Git tree. Migration stopped.",
                ));
            }
            let behaviors = skilld_core::detect_behaviors(&prepared.files);
            let held = crate::held_behaviors(&behaviors, &[], &[]);
            preview.behaviors = held.iter().map(|b| crate::describe_behavior(b)).collect();
            allowed_behaviors = held.iter().map(|b| b.id.to_owned()).collect();
            if let LockedSource::Remote {
                source, commit_sha, ..
            } = &prepared.locked_source
            {
                preview.source = Some(source.clone());
                preview.commit = Some(commit_sha.clone());
            }
            Some((prepared, staged))
        } else {
            None
        };
        Ok(DoctorPlan {
            preview,
            action,
            copies,
            paths: paths.into_iter().collect(),
            observed_paths,
            absent_targets,
            record,
            store,
            targets: targets.into_values().collect(),
            prepared,
            allowed_behaviors,
        })
    }
}

impl DoctorPlan {
    pub fn apply(self) -> Result<DoctorApplied, CommandError> {
        for path in &self.absent_targets {
            match fs::symlink_metadata(path) {
                Ok(_) => {
                    return Err(failure(
                        "An installation appeared after review. Scan it again.",
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(CommandError::filesystem(e.to_string())),
            }
        }
        if let Some((prepared, _)) = &self.prepared {
            crate::gate_new_behaviors(
                |_| Ok(crate::BehaviorDecision::Unavailable),
                &self.preview.name,
                crate::BehaviorChange::Install,
                &skilld_core::detect_behaviors(&prepared.files),
                &[],
                &self.allowed_behaviors,
                &prepared.behavior_readings,
            )?;
        }
        for (observed, physical, link) in &self.observed_paths {
            let parent = fs::canonicalize(observed.parent().expect("planned parent"))
                .map_err(|e| CommandError::filesystem(e.to_string()))?;
            if parent.join(observed.file_name().expect("planned name")) != *physical {
                return Err(failure(
                    "An Agent target root changed after review. Scan it again.",
                ));
            }
            let metadata = fs::symlink_metadata(physical)
                .map_err(|e| CommandError::filesystem(e.to_string()))?;
            let current_link = if metadata.file_type().is_symlink() {
                Some(fs::read_link(physical).map_err(|e| CommandError::filesystem(e.to_string()))?)
            } else {
                None
            };
            if current_link != *link {
                return Err(failure(
                    "An Agent target link changed after review. Scan it again.",
                ));
            }
        }
        for skill in &self.copies {
            let expected = skill
                .fingerprint
                .as_ref()
                .ok_or_else(|| failure("The Skill directory could not be read."))?;
            for location in skill.paths.iter().filter(|p| p.agent.is_some()) {
                let real = fs::canonicalize(&location.path)
                    .map_err(|e| CommandError::filesystem(e.to_string()))?;
                if real != skill.canonical_path || fingerprint(&real)? != *expected {
                    return Err(failure("The Skill changed after review. Scan it again."));
                }
            }
        }
        let foreign_new = if let Some(r) = &self.record {
            Some(foreign_lock_without(&r.lockfile, r)?)
        } else {
            None
        };
        let parent = self
            .store
            .root()
            .parent()
            .ok_or_else(|| failure("The Skill store has no parent."))?;
        let backup_root = parent.join(".skilld-doctor-backups");
        fs::create_dir_all(&backup_root).map_err(|e| CommandError::filesystem(e.to_string()))?;
        let backup = tempfile::Builder::new()
            .prefix("action-")
            .tempdir_in(&backup_root)
            .map_err(|e| CommandError::filesystem(e.to_string()))?
            .keep();
        let moves = self
            .paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p.clone(), backup.join(format!("target-{i}"))))
            .collect::<Vec<_>>();
        let receipt = serde_json::json!({"version":1,"action":self.preview.action,"name":self.preview.name,"paths":moves,"foreignLock":self.record.as_ref().map(|r|&r.lockfile)});
        fs::write(
            backup.join("recovery.json"),
            serde_json::to_vec_pretty(&receipt)
                .map_err(|e| CommandError::filesystem(e.to_string()))?,
        )
        .map_err(|e| CommandError::filesystem(e.to_string()))?;
        if let Some(r) = &self.record {
            fs::copy(&r.lockfile, backup.join("skills-lock.json"))
                .map_err(|e| CommandError::filesystem(e.to_string()))?;
        }
        let known = self
            .targets
            .iter()
            .map(|t| t.target.clone())
            .collect::<Vec<_>>();
        let mut moved = vec![];
        let mut installed = false;
        let result = (|| {
            for (original, saved) in &moves {
                fs::rename(original, saved).map_err(|e| {
                    CommandError::filesystem(format!("Cannot back up {}: {e}", original.display()))
                })?;
                moved.push((original, saved));
            }
            if let Some((prepared, staged)) = &self.prepared {
                let outcome = self.store.install_from_with_status(
                    staged.path(),
                    prepared.locked_source.clone(),
                    prepared.source_status.clone(),
                    &self.targets,
                    &known,
                );
                // A cleanup error can follow a committed installation. Roll it back too.
                installed = outcome.is_ok()
                    || outcome
                        .as_ref()
                        .is_err_and(|e| e.code() == "COMMITTED_CLEANUP_PENDING");
                outcome.map_err(CommandError::store)?;
                installed = true;
                let name =
                    SkillName::parse(self.preview.name.clone()).map_err(CommandError::domain)?;
                self.store
                    .verify_content(&name, &known)
                    .map_err(CommandError::store)?;
            }
            if let (Some(record), Some(bytes)) = (&self.record, &foreign_new) {
                foreign_lock_without(&record.lockfile, record)?;
                atomic_write(&record.lockfile, bytes)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            if installed {
                let name =
                    SkillName::parse(self.preview.name.clone()).map_err(CommandError::domain)?;
                self.store.remove(&name, &known).map_err(|rollback| {
                    CommandError::filesystem(format!(
                        "{error}. Rollback failed: {rollback}. Recovery files: {}",
                        backup.display()
                    ))
                })?;
            }
            for (original, saved) in moved.into_iter().rev() {
                if fs::symlink_metadata(original).is_ok() {
                    return Err(CommandError::filesystem(format!(
                        "{error}. A target changed during rollback. Recovery files: {}",
                        backup.display()
                    )));
                }
                fs::rename(saved, original).map_err(|rollback| {
                    CommandError::filesystem(format!(
                        "{error}. Rollback failed: {rollback}. Recovery files: {}",
                        backup.display()
                    ))
                })?;
            }
            return Err(error);
        }
        Ok(DoctorApplied {
            backup,
            message: format!(
                "{}: {}",
                if self.action == DoctorAction::Migrate {
                    "Migrated"
                } else {
                    "Removed"
                },
                self.preview.name
            ),
        })
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CommandError> {
    let parent = path
        .parent()
        .ok_or_else(|| failure("The lockfile has no parent."))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| CommandError::filesystem(e.to_string()))?;
    staged
        .as_file()
        .set_permissions(
            fs::metadata(path)
                .map_err(|e| CommandError::filesystem(e.to_string()))?
                .permissions(),
        )
        .map_err(|e| CommandError::filesystem(e.to_string()))?;
    staged
        .write_all(bytes)
        .map_err(|e| CommandError::filesystem(e.to_string()))?;
    staged
        .as_file()
        .sync_all()
        .map_err(|e| CommandError::filesystem(e.to_string()))?;
    staged
        .persist(path)
        .map_err(|e| CommandError::filesystem(e.to_string()))?;
    Ok(())
}
