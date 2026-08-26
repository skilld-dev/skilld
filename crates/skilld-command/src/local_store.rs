use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skilld_core::{
    AgentTargetId, InstallMode, LockDocument, LockedSkill, LockedSource, LockedTarget, SkillName,
    SourceStatus,
};

const JOURNAL_NAME: &str = ".skilld-transaction";
#[cfg(not(target_os = "wasi"))]
const LOCK_NAME: &str = ".skilld-store-lock";
const LOCKFILE_NAME: &str = "skilld-lock.yaml";
static TRANSACTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(not(target_os = "wasi"))]
struct StoreLock {
    _file: File,
}

#[cfg(target_os = "wasi")]
struct StoreLock;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTarget {
    pub agent: AgentTargetId,
    pub root: PathBuf,
}

impl ResolvedTarget {
    pub fn new(agent: AgentTargetId, root: PathBuf) -> Result<Self, StoreError> {
        if !root.is_absolute() || root.components().any(|part| part == Component::ParentDir) {
            return Err(StoreError::InvalidTargetPath(root));
        }
        Ok(Self {
            agent,
            root: normalize_path(&root),
        })
    }

    fn destination(&self, name: &SkillName) -> PathBuf {
        self.root.join(name.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetInstall {
    pub target: ResolvedTarget,
    pub mode: InstallMode,
}

#[derive(Clone, Debug)]
pub struct PreparedStoreUpdate {
    pub source: PathBuf,
    pub locked_source: LockedSource,
    pub source_status: Option<SourceStatus>,
    pub targets: Vec<TargetInstall>,
    pub expected_transaction_id: String,
    pub expected_skill: LockedSkill,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillView {
    pub name: String,
    pub canonical_path: PathBuf,
    pub skill: LockedSkill,
    pub transaction_id: String,
}

#[derive(Debug)]
pub enum StoreError {
    CommittedCleanupPending(String),
    Conflict(String),
    Filesystem(String),
    InvalidLockfile(String),
    InvalidSource(String),
    InvalidTargetPath(PathBuf),
    NotFound(String),
    StalePlan(String),
    Unsupported(String),
}

impl StoreError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CommittedCleanupPending(_) => "COMMITTED_CLEANUP_PENDING",
            Self::Conflict(_) => "TARGET_CONFLICT",
            Self::Filesystem(_) => "SERVICE_UNAVAILABLE",
            Self::InvalidLockfile(_) => "INVALID_LOCKFILE",
            Self::InvalidSource(_) => "INVALID_SOURCE",
            Self::InvalidTargetPath(_) => "INVALID_TARGET",
            Self::NotFound(_) => "SKILL_NOT_FOUND",
            Self::StalePlan(_) => "PLAN_STALE",
            Self::Unsupported(_) => "UNSUPPORTED_HOST",
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommittedCleanupPending(message)
            | Self::Conflict(message)
            | Self::Filesystem(message)
            | Self::InvalidLockfile(message)
            | Self::InvalidSource(message)
            | Self::NotFound(message)
            | Self::StalePlan(message)
            | Self::Unsupported(message) => formatter.write_str(message),
            Self::InvalidTargetPath(path) => {
                write!(
                    formatter,
                    "Agent target path is not confined: {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for StoreError {}

pub trait TransactionGate {
    fn before_lock_commit(&self, _lockfile: &Path) -> Result<(), StoreError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AllowTransaction;

impl TransactionGate for AllowTransaction {}

#[derive(Clone, Debug)]
pub struct LocalStore {
    root: PathBuf,
}

impl LocalStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: normalize_path(&root),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn list(&self, known_targets: &[ResolvedTarget]) -> Result<Vec<String>, StoreError> {
        if !self.root.exists() {
            return Ok(vec![]);
        }
        let _lock = self.lock_existing()?;
        self.recover_locked(known_targets)?;
        Ok(self.read_lock()?.skills.into_keys().collect())
    }

    pub fn view(
        &self,
        name: &SkillName,
        known_targets: &[ResolvedTarget],
    ) -> Result<SkillView, StoreError> {
        if !self.root.exists() {
            return Err(StoreError::NotFound(format!(
                "Skill {} is not installed",
                name.as_str()
            )));
        }
        let _lock = self.lock_existing()?;
        self.recover_locked(known_targets)?;
        let document = self.read_lock()?;
        let skill = document
            .skills
            .get(name.as_str())
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("Skill {name} is not installed")))?;
        Ok(SkillView {
            name: name.to_string(),
            canonical_path: self.root.join(name.as_str()),
            skill,
            transaction_id: document.transaction_id,
        })
    }

    pub fn verify_content(
        &self,
        name: &SkillName,
        known_targets: &[ResolvedTarget],
    ) -> Result<SkillView, StoreError> {
        if !self.root.exists() {
            return Err(StoreError::NotFound(format!(
                "Skill {} is not installed",
                name.as_str()
            )));
        }
        let _lock = self.lock_existing()?;
        self.recover_locked(known_targets)?;
        let document = self.read_lock()?;
        let skill = document
            .skills
            .get(name.as_str())
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("Skill {name} is not installed")))?;
        self.verify_managed_state(name, Some(&skill), known_targets)?;
        Ok(SkillView {
            name: name.to_string(),
            canonical_path: self.root.join(name.as_str()),
            skill,
            transaction_id: document.transaction_id,
        })
    }

    pub fn install_from(
        &self,
        source: &Path,
        locked_source: LockedSource,
        targets: &[TargetInstall],
        known_targets: &[ResolvedTarget],
    ) -> Result<SkillName, StoreError> {
        self.install_from_with_gate_and_status(
            source,
            locked_source,
            targets,
            known_targets,
            &AllowTransaction,
            None,
        )
    }

    pub fn install_from_with_status(
        &self,
        source: &Path,
        locked_source: LockedSource,
        source_status: SourceStatus,
        targets: &[TargetInstall],
        known_targets: &[ResolvedTarget],
    ) -> Result<SkillName, StoreError> {
        self.install_from_with_gate_and_status(
            source,
            locked_source,
            targets,
            known_targets,
            &AllowTransaction,
            Some(source_status),
        )
    }

    pub fn install_from_with_gate<G: TransactionGate>(
        &self,
        source: &Path,
        locked_source: LockedSource,
        targets: &[TargetInstall],
        known_targets: &[ResolvedTarget],
        gate: &G,
    ) -> Result<SkillName, StoreError> {
        self.install_from_with_gate_and_status(
            source,
            locked_source,
            targets,
            known_targets,
            gate,
            None,
        )
    }

    fn install_from_with_gate_and_status<G: TransactionGate>(
        &self,
        source: &Path,
        locked_source: LockedSource,
        targets: &[TargetInstall],
        known_targets: &[ResolvedTarget],
        gate: &G,
        source_status: Option<SourceStatus>,
    ) -> Result<SkillName, StoreError> {
        ensure_write_capability()?;
        let source = absolute_normalized(source).map_err(fs_error)?;
        validate_skill_source(&source)?;
        let digest = hash_skill_tree(&source)?;
        let source = resolve_path(&source).map_err(fs_error)?;
        let name = SkillName::from_source(&source)
            .map_err(|error| StoreError::InvalidSource(error.to_string()))?;
        self.reject_overlap(&source)?;
        self.prepare_root()?;
        let _lock = acquire_store_lock(&self.root)?;
        self.recover_locked(known_targets)?;

        let old_lock = self.read_lock()?;
        let old_skill = old_lock.skills.get(name.as_str());
        self.verify_managed_state(&name, old_skill, known_targets)?;
        let selected_targets = unique_target_installs(targets);
        self.validate_target_changes(&name, old_skill, &selected_targets, known_targets)?;

        let transaction = transaction_id();
        let canonical = self.root.join(name.as_str());
        let canonical_had_existing = path_exists(&canonical)?;
        let changes = target_changes(&name, old_skill, &selected_targets, known_targets)?;
        let journal = Journal {
            version: 1,
            transaction_id: transaction.clone(),
            skills: vec![JournalSkill {
                operation: JournalOperation::Install,
                skill: name.to_string(),
                canonical_had_existing,
                targets: changes
                    .iter()
                    .map(|change| JournalTarget {
                        agent: change.agent,
                        had_existing: change.had_existing,
                        promote: change.install.is_some(),
                    })
                    .collect(),
            }],
        };
        self.write_journal(&journal)?;

        let canonical_stage = stage_path(&canonical, &transaction)?;
        let canonical_backup = backup_path(&canonical, &transaction)?;
        if let Err(error) = copy_tree(&source, &canonical_stage) {
            return self.rollback_error(error, known_targets);
        }
        for change in &changes {
            if let Some(install) = &change.install {
                if let Err(error) =
                    stage_target(install, &canonical, &canonical_stage, &transaction, &digest)
                {
                    return self.rollback_error(error, known_targets);
                }
            }
        }
        if hash_skill_tree(&canonical_stage)? != digest {
            return self.rollback_error(
                StoreError::InvalidSource("the local Skill changed while copying".to_owned()),
                known_targets,
            );
        }

        if canonical_had_existing {
            if let Err(error) = fs::rename(&canonical, &canonical_backup).map_err(fs_error) {
                return self.rollback_error(error, known_targets);
            }
        }
        if let Err(error) = fs::rename(&canonical_stage, &canonical).map_err(fs_error) {
            return self.rollback_error(error, known_targets);
        }
        for change in &changes {
            let destination = change.target.destination(&name);
            let backup = backup_path(&destination, &transaction)?;
            if change.had_existing {
                if let Err(error) = fs::rename(&destination, &backup).map_err(fs_error) {
                    return self.rollback_error(error, known_targets);
                }
            }
            if change.install.is_some() {
                let stage = stage_path(&destination, &transaction)?;
                if let Err(error) = fs::rename(&stage, &destination).map_err(fs_error) {
                    return self.rollback_error(error, known_targets);
                }
            }
        }

        if let Err(error) = gate.before_lock_commit(&self.lockfile_path()) {
            return self.rollback_error(error, known_targets);
        }
        let source_status = source_status.unwrap_or_else(|| SourceStatus::Local {
            content_sha256: digest.clone(),
        });
        if source_digest(&source_status) != digest {
            return self.rollback_error(
                StoreError::InvalidSource(
                    "the installed Skill does not match its source status".to_owned(),
                ),
                known_targets,
            );
        }
        let mut new_lock = old_lock;
        new_lock.transaction_id.clone_from(&transaction);
        new_lock.skills.insert(
            name.to_string(),
            LockedSkill {
                source: locked_source,
                source_status,
                targets: targets
                    .iter()
                    .map(|target| LockedTarget {
                        agent: target.target.agent,
                        mode: target.mode,
                    })
                    .collect(),
            },
        );
        if let Err(error) = self.write_lock_atomic(&new_lock, &transaction) {
            return self.rollback_error(error, known_targets);
        }
        self.cleanup_committed(&journal, known_targets)
            .map_err(|error| StoreError::CommittedCleanupPending(error.to_string()))?;
        Ok(name)
    }

    pub fn apply_update_batch(
        &self,
        updates: Vec<PreparedStoreUpdate>,
        known_targets: &[ResolvedTarget],
    ) -> Result<Vec<SkillName>, StoreError> {
        self.apply_update_batch_with_gate(updates, known_targets, &AllowTransaction)
    }

    pub fn apply_update_batch_with_gate<G: TransactionGate>(
        &self,
        updates: Vec<PreparedStoreUpdate>,
        known_targets: &[ResolvedTarget],
        gate: &G,
    ) -> Result<Vec<SkillName>, StoreError> {
        ensure_write_capability()?;
        if updates.is_empty() {
            return Ok(vec![]);
        }

        let mut names = BTreeSet::new();
        let mut validated = Vec::with_capacity(updates.len());
        for update in updates {
            let source = absolute_normalized(&update.source).map_err(fs_error)?;
            validate_skill_source(&source)?;
            let digest = hash_skill_tree(&source)?;
            let source = resolve_path(&source).map_err(fs_error)?;
            let name = SkillName::from_source(&source)
                .map_err(|error| StoreError::InvalidSource(error.to_string()))?;
            if !names.insert(name.clone()) {
                return Err(StoreError::InvalidSource(format!(
                    "Skill {name} appears more than once in the update"
                )));
            }
            self.reject_overlap(&source)?;
            let source_status = update.source_status.unwrap_or_else(|| SourceStatus::Local {
                content_sha256: digest.clone(),
            });
            if source_digest(&source_status) != digest {
                return Err(StoreError::InvalidSource(
                    "the updated Skill does not match its source status".to_owned(),
                ));
            }
            validated.push(ValidatedStoreUpdate {
                name,
                source,
                digest,
                locked_source: update.locked_source,
                source_status,
                targets: update.targets,
                expected_transaction_id: update.expected_transaction_id,
                expected_skill: update.expected_skill,
            });
        }
        let expected_transaction_id = validated[0].expected_transaction_id.clone();
        if validated
            .iter()
            .any(|update| update.expected_transaction_id != expected_transaction_id)
        {
            return Err(stale_update_plan());
        }

        self.prepare_root()?;
        let _lock = acquire_store_lock(&self.root)?;
        self.recover_locked(known_targets)?;
        let old_lock = self.read_lock()?;
        if old_lock.transaction_id != expected_transaction_id {
            return Err(stale_update_plan());
        }
        let mut prepared = Vec::with_capacity(validated.len());
        for update in validated {
            let Some(old_skill) = old_lock.skills.get(update.name.as_str()) else {
                return Err(stale_update_plan());
            };
            if old_skill != &update.expected_skill {
                return Err(stale_update_plan());
            }
            self.verify_managed_state(&update.name, Some(old_skill), known_targets)?;
            let selected_targets = unique_target_installs(&update.targets);
            self.validate_target_changes(
                &update.name,
                Some(old_skill),
                &selected_targets,
                known_targets,
            )?;
            let canonical = self.root.join(update.name.as_str());
            prepared.push(BatchStoreUpdate {
                canonical_had_existing: path_exists(&canonical)?,
                changes: target_changes(
                    &update.name,
                    Some(old_skill),
                    &selected_targets,
                    known_targets,
                )?,
                canonical,
                update,
            });
        }

        let transaction = transaction_id();
        let journal = Journal {
            version: 1,
            transaction_id: transaction.clone(),
            skills: prepared
                .iter()
                .map(|prepared| JournalSkill {
                    operation: JournalOperation::Install,
                    skill: prepared.update.name.to_string(),
                    canonical_had_existing: prepared.canonical_had_existing,
                    targets: prepared
                        .changes
                        .iter()
                        .map(|change| JournalTarget {
                            agent: change.agent,
                            had_existing: change.had_existing,
                            promote: change.install.is_some(),
                        })
                        .collect(),
                })
                .collect(),
        };
        self.write_journal(&journal)?;

        let commit_result = (|| -> Result<(), StoreError> {
            for prepared in &prepared {
                let canonical_stage = stage_path(&prepared.canonical, &transaction)?;
                copy_tree(&prepared.update.source, &canonical_stage)?;
                for change in &prepared.changes {
                    if let Some(install) = &change.install {
                        stage_target(
                            install,
                            &prepared.canonical,
                            &canonical_stage,
                            &transaction,
                            &prepared.update.digest,
                        )?;
                    }
                }
                if hash_skill_tree(&canonical_stage)? != prepared.update.digest {
                    return Err(StoreError::InvalidSource(
                        "the local Skill changed while copying".to_owned(),
                    ));
                }
            }

            for prepared in &prepared {
                let canonical_backup = backup_path(&prepared.canonical, &transaction)?;
                if prepared.canonical_had_existing {
                    fs::rename(&prepared.canonical, canonical_backup).map_err(fs_error)?;
                }
                fs::rename(
                    stage_path(&prepared.canonical, &transaction)?,
                    &prepared.canonical,
                )
                .map_err(fs_error)?;
                for change in &prepared.changes {
                    let destination = change.target.destination(&prepared.update.name);
                    if change.had_existing {
                        fs::rename(&destination, backup_path(&destination, &transaction)?)
                            .map_err(fs_error)?;
                    }
                    if change.install.is_some() {
                        fs::rename(stage_path(&destination, &transaction)?, &destination)
                            .map_err(fs_error)?;
                    }
                }
            }

            gate.before_lock_commit(&self.lockfile_path())?;
            let mut new_lock = old_lock;
            new_lock.transaction_id.clone_from(&transaction);
            for prepared in &prepared {
                new_lock.skills.insert(
                    prepared.update.name.to_string(),
                    LockedSkill {
                        source: prepared.update.locked_source.clone(),
                        source_status: prepared.update.source_status.clone(),
                        targets: prepared
                            .update
                            .targets
                            .iter()
                            .map(|target| LockedTarget {
                                agent: target.target.agent,
                                mode: target.mode,
                            })
                            .collect(),
                    },
                );
            }
            self.write_lock_atomic(&new_lock, &transaction)
        })();
        if let Err(error) = commit_result {
            return self.rollback_error(error, known_targets);
        }
        self.cleanup_committed(&journal, known_targets)
            .map_err(|error| StoreError::CommittedCleanupPending(error.to_string()))?;
        Ok(prepared
            .into_iter()
            .map(|prepared| prepared.update.name)
            .collect())
    }

    pub fn remove(
        &self,
        name: &SkillName,
        known_targets: &[ResolvedTarget],
    ) -> Result<(), StoreError> {
        ensure_write_capability()?;
        if !self.root.exists() {
            return Err(StoreError::NotFound(format!(
                "Skill {name} is not installed"
            )));
        }
        let _lock = self.lock_existing()?;
        self.recover_locked(known_targets)?;
        let old_lock = self.read_lock()?;
        let old_skill = old_lock
            .skills
            .get(name.as_str())
            .ok_or_else(|| StoreError::NotFound(format!("Skill {name} is not installed")))?;
        self.verify_managed_state(name, Some(old_skill), known_targets)?;

        let transaction = transaction_id();
        let canonical = self.root.join(name.as_str());
        let canonical_had_existing = path_exists(&canonical)?;
        let targets = unique_locked_target_paths(name, &old_skill.targets, known_targets)?;
        let mut journal_targets = Vec::new();
        for target in &targets {
            journal_targets.push(JournalTarget {
                agent: target.agent,
                had_existing: path_exists(&target.destination(name))?,
                promote: false,
            });
        }
        let journal = Journal {
            version: 1,
            transaction_id: transaction.clone(),
            skills: vec![JournalSkill {
                operation: JournalOperation::Remove,
                skill: name.to_string(),
                canonical_had_existing,
                targets: journal_targets,
            }],
        };
        self.write_journal(&journal)?;
        if canonical_had_existing {
            fs::rename(&canonical, backup_path(&canonical, &transaction)?).map_err(fs_error)?;
        }
        for target in &targets {
            let destination = target.destination(name);
            if path_exists(&destination)? {
                if let Err(error) =
                    fs::rename(&destination, backup_path(&destination, &transaction)?)
                        .map_err(fs_error)
                {
                    return self.rollback_error(error, known_targets);
                }
            }
        }

        let mut new_lock = old_lock;
        new_lock.transaction_id.clone_from(&transaction);
        new_lock.skills.remove(name.as_str());
        if let Err(error) = self.write_lock_atomic(&new_lock, &transaction) {
            return self.rollback_error(error, known_targets);
        }
        self.cleanup_committed(&journal, known_targets)
            .map_err(|error| StoreError::CommittedCleanupPending(error.to_string()))
    }

    fn prepare_root(&self) -> Result<(), StoreError> {
        reject_symlink_ancestors(&self.root).map_err(fs_error)?;
        fs::create_dir_all(&self.root).map_err(fs_error)?;
        reject_symlink_ancestors(&self.root).map_err(fs_error)?;
        reject_directory_symlink(&self.root, "Skill store").map_err(fs_error)
    }

    fn lock_existing(&self) -> Result<StoreLock, StoreError> {
        reject_symlink_ancestors(&self.root).map_err(fs_error)?;
        reject_directory_symlink(&self.root, "Skill store").map_err(fs_error)?;
        acquire_store_lock(&self.root)
    }

    fn reject_overlap(&self, source: &Path) -> Result<(), StoreError> {
        let store = resolve_path(&self.root).map_err(fs_error)?;
        if source.starts_with(&store) || store.starts_with(source) {
            return Err(StoreError::InvalidSource(
                "the local Skill source and store cannot overlap".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_target_changes(
        &self,
        name: &SkillName,
        old_skill: Option<&LockedSkill>,
        targets: &[TargetInstall],
        known_targets: &[ResolvedTarget],
    ) -> Result<(), StoreError> {
        let managed = old_skill
            .map(|skill| unique_locked_target_paths(name, &skill.targets, known_targets))
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|target| target.destination(name))
            .collect::<BTreeSet<_>>();
        for install in targets {
            let destination = install.target.destination(name);
            if path_exists(&destination)? && !managed.contains(&destination) {
                return Err(StoreError::Conflict(format!(
                    "Agent target already contains unmanaged Skill {name}: {}",
                    destination.display()
                )));
            }
            prepare_target_root(&install.target.root)?;
            if install.target.root.starts_with(&self.root)
                || self.root.starts_with(&install.target.root)
            {
                return Err(StoreError::InvalidTargetPath(install.target.root.clone()));
            }
        }
        Ok(())
    }

    fn verify_managed_state(
        &self,
        name: &SkillName,
        old_skill: Option<&LockedSkill>,
        known_targets: &[ResolvedTarget],
    ) -> Result<(), StoreError> {
        let Some(skill) = old_skill else {
            return Ok(());
        };
        let canonical = self.root.join(name.as_str());
        if path_exists(&canonical)? {
            let canonical_digest = hash_skill_tree(&canonical).map_err(|_| {
                StoreError::Conflict(format!("canonical Skill {name} changed after installation"))
            })?;
            if canonical_digest != source_digest(&skill.source_status) {
                return Err(StoreError::Conflict(format!(
                    "canonical Skill {name} changed after installation"
                )));
            }
        }
        for locked in &skill.targets {
            let target = find_target(known_targets, locked.agent)?;
            let destination = target.destination(name);
            if !path_exists(&destination)? {
                continue;
            }
            match locked.mode {
                InstallMode::Copy => {
                    let target_digest = hash_skill_tree(&destination).map_err(|_| {
                        StoreError::Conflict(format!(
                            "Agent target Skill {name} changed after installation: {}",
                            destination.display()
                        ))
                    })?;
                    if target_digest != source_digest(&skill.source_status) {
                        return Err(StoreError::Conflict(format!(
                            "Agent target Skill {name} changed after installation: {}",
                            destination.display()
                        )));
                    }
                }
                InstallMode::Symlink => verify_managed_link(&destination, &canonical)?,
            }
        }
        Ok(())
    }

    fn lockfile_path(&self) -> PathBuf {
        self.root.join(LOCKFILE_NAME)
    }

    fn read_lock(&self) -> Result<LockDocument, StoreError> {
        let path = self.lockfile_path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(LockDocument::default());
            }
            Err(error) => return Err(fs_error(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StoreError::InvalidLockfile(
                "the Skill lockfile must be a regular file".to_owned(),
            ));
        }
        let bytes = fs::read(&path).map_err(fs_error)?;
        let document: LockDocument = serde_json::from_slice(&bytes).map_err(|_| {
            StoreError::InvalidLockfile("the Skill lockfile is not valid JSON".to_owned())
        })?;
        if document.version != 1 {
            return Err(StoreError::InvalidLockfile(format!(
                "unsupported Skill lockfile version: {}",
                document.version
            )));
        }
        for name in document.skills.keys() {
            SkillName::parse(name.clone())
                .map_err(|error| StoreError::InvalidLockfile(error.to_string()))?;
        }
        Ok(document)
    }

    fn write_lock_atomic(
        &self,
        document: &LockDocument,
        transaction: &str,
    ) -> Result<(), StoreError> {
        let path = self.lockfile_path();
        let temporary = self.root.join(format!(".skilld-lock-stage-{transaction}"));
        let backup = self.root.join(format!(".skilld-lock-backup-{transaction}"));
        let bytes = serde_json::to_vec_pretty(document)
            .map_err(|error| StoreError::Filesystem(error.to_string()))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(fs_error)?;
        file.write_all(&bytes).map_err(fs_error)?;
        file.write_all(b"\n").map_err(fs_error)?;
        file.sync_all().map_err(fs_error)?;
        if path_exists(&path)? {
            fs::rename(&path, &backup).map_err(fs_error)?;
        }
        fs::rename(&temporary, &path).map_err(fs_error)
    }

    fn write_journal(&self, journal: &Journal) -> Result<(), StoreError> {
        let path = self.root.join(JOURNAL_NAME);
        if path_exists(&path)? {
            return Err(StoreError::InvalidLockfile(
                "a Skill transaction is already active".to_owned(),
            ));
        }
        let stage = self.root.join(format!(
            ".skilld-transaction-stage-{}",
            journal.transaction_id
        ));
        let write_result = (|| {
            fs::create_dir(&stage).map_err(fs_error)?;
            let state = stage.join("state.json");
            let bytes = serde_json::to_vec_pretty(journal)
                .map_err(|error| StoreError::Filesystem(error.to_string()))?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(state)
                .map_err(fs_error)?;
            file.write_all(&bytes).map_err(fs_error)?;
            file.sync_all().map_err(fs_error)?;
            fs::rename(&stage, &path).map_err(fs_error)
        })();
        if let Err(error) = write_result {
            return match remove_path(&stage) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(StoreError::Filesystem(format!(
                    "{error}; journal cleanup failed: {cleanup}"
                ))),
            };
        }
        Ok(())
    }

    fn read_journal(&self) -> Result<Option<Journal>, StoreError> {
        let path = self.root.join(JOURNAL_NAME);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(fs_error(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::InvalidLockfile(
                "the Skill transaction journal must be a directory".to_owned(),
            ));
        }
        let journal: Journal =
            serde_json::from_slice(&fs::read(path.join("state.json")).map_err(fs_error)?)
                .map_err(|error| StoreError::InvalidLockfile(error.to_string()))?;
        if journal.version != 1
            || !valid_transaction_id(&journal.transaction_id)
            || journal.skills.is_empty()
        {
            return Err(StoreError::InvalidLockfile(
                "invalid Skill transaction journal".to_owned(),
            ));
        }
        let mut names = BTreeSet::new();
        for skill in &journal.skills {
            let name = SkillName::parse(skill.skill.clone())
                .map_err(|error| StoreError::InvalidLockfile(error.to_string()))?;
            if !names.insert(name) {
                return Err(StoreError::InvalidLockfile(
                    "the Skill transaction journal contains duplicate Skills".to_owned(),
                ));
            }
        }
        Ok(Some(journal))
    }

    fn recover_locked(&self, known_targets: &[ResolvedTarget]) -> Result<(), StoreError> {
        self.cleanup_orphan_journal_stages()?;
        let Some(journal) = self.read_journal()? else {
            return Ok(());
        };
        let lock = self.read_lock()?;
        let committed = lock.transaction_id == journal.transaction_id;
        if committed {
            return self.cleanup_committed(&journal, known_targets);
        }
        self.rollback(&journal, known_targets)
    }

    fn cleanup_orphan_journal_stages(&self) -> Result<(), StoreError> {
        for entry in fs::read_dir(&self.root).map_err(fs_error)? {
            let entry = entry.map_err(fs_error)?;
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if name.starts_with(".skilld-transaction-stage-") {
                remove_path(&entry.path()).map_err(fs_error)?;
            }
        }
        Ok(())
    }

    fn rollback_error<T>(
        &self,
        error: StoreError,
        known_targets: &[ResolvedTarget],
    ) -> Result<T, StoreError> {
        match self.recover_locked(known_targets) {
            Ok(()) => Err(error),
            Err(rollback) => Err(StoreError::Filesystem(format!(
                "{error}; rollback failed: {rollback}"
            ))),
        }
    }

    fn rollback(
        &self,
        journal: &Journal,
        known_targets: &[ResolvedTarget],
    ) -> Result<(), StoreError> {
        for skill in &journal.skills {
            let name = SkillName::parse(skill.skill.clone())
                .map_err(|error| StoreError::InvalidLockfile(error.to_string()))?;
            let canonical = self.root.join(name.as_str());
            restore_path(
                &canonical,
                &stage_path(&canonical, &journal.transaction_id)?,
                &backup_path(&canonical, &journal.transaction_id)?,
                skill.canonical_had_existing,
            )?;
            for target in &skill.targets {
                let resolved = find_target(known_targets, target.agent)?;
                let destination = resolved.destination(&name);
                restore_path(
                    &destination,
                    &stage_path(&destination, &journal.transaction_id)?,
                    &backup_path(&destination, &journal.transaction_id)?,
                    target.had_existing,
                )?;
            }
        }
        restore_lockfile(&self.root, &journal.transaction_id)?;
        remove_path(&self.root.join(JOURNAL_NAME)).map_err(fs_error)
    }

    fn cleanup_committed(
        &self,
        journal: &Journal,
        known_targets: &[ResolvedTarget],
    ) -> Result<(), StoreError> {
        for skill in &journal.skills {
            let name = SkillName::parse(skill.skill.clone())
                .map_err(|error| StoreError::InvalidLockfile(error.to_string()))?;
            let canonical = self.root.join(name.as_str());
            remove_path(&stage_path(&canonical, &journal.transaction_id)?).map_err(fs_error)?;
            remove_path(&backup_path(&canonical, &journal.transaction_id)?).map_err(fs_error)?;
            for target in &skill.targets {
                let resolved = find_target(known_targets, target.agent)?;
                let destination = resolved.destination(&name);
                remove_path(&stage_path(&destination, &journal.transaction_id)?)
                    .map_err(fs_error)?;
                remove_path(&backup_path(&destination, &journal.transaction_id)?)
                    .map_err(fs_error)?;
            }
        }
        remove_path(
            &self
                .root
                .join(format!(".skilld-lock-stage-{}", journal.transaction_id)),
        )
        .map_err(fs_error)?;
        remove_path(
            &self
                .root
                .join(format!(".skilld-lock-backup-{}", journal.transaction_id)),
        )
        .map_err(fs_error)?;
        remove_path(&self.root.join(JOURNAL_NAME)).map_err(fs_error)
    }
}

#[derive(Clone, Debug)]
struct TargetChange {
    agent: AgentTargetId,
    target: ResolvedTarget,
    had_existing: bool,
    install: Option<TargetInstall>,
}

#[derive(Clone, Debug)]
struct ValidatedStoreUpdate {
    name: SkillName,
    source: PathBuf,
    digest: String,
    locked_source: LockedSource,
    source_status: SourceStatus,
    targets: Vec<TargetInstall>,
    expected_transaction_id: String,
    expected_skill: LockedSkill,
}

#[derive(Clone, Debug)]
struct BatchStoreUpdate {
    update: ValidatedStoreUpdate,
    canonical: PathBuf,
    canonical_had_existing: bool,
    changes: Vec<TargetChange>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Journal {
    version: u8,
    transaction_id: String,
    skills: Vec<JournalSkill>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct JournalSkill {
    operation: JournalOperation,
    skill: String,
    canonical_had_existing: bool,
    targets: Vec<JournalTarget>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum JournalOperation {
    Install,
    Remove,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct JournalTarget {
    agent: AgentTargetId,
    had_existing: bool,
    promote: bool,
}

fn target_changes(
    name: &SkillName,
    old_skill: Option<&LockedSkill>,
    installs: &[TargetInstall],
    known_targets: &[ResolvedTarget],
) -> Result<Vec<TargetChange>, StoreError> {
    let mut by_path = BTreeMap::<PathBuf, TargetChange>::new();
    if let Some(skill) = old_skill {
        for target in unique_locked_target_paths(name, &skill.targets, known_targets)? {
            let destination = target.destination(name);
            by_path.insert(
                destination.clone(),
                TargetChange {
                    agent: target.agent,
                    target,
                    had_existing: path_exists(&destination)?,
                    install: None,
                },
            );
        }
    }
    for install in installs {
        let destination = install.target.destination(name);
        if let Some(change) = by_path.get_mut(&destination) {
            change.install = Some(install.clone());
        } else {
            let had_existing = path_exists(&destination)?;
            by_path.insert(
                destination,
                TargetChange {
                    agent: install.target.agent,
                    target: install.target.clone(),
                    had_existing,
                    install: Some(install.clone()),
                },
            );
        }
    }
    Ok(by_path.into_values().collect())
}

fn unique_target_installs(targets: &[TargetInstall]) -> Vec<TargetInstall> {
    let mut by_path = BTreeMap::new();
    for target in targets {
        by_path
            .entry(target.target.root.clone())
            .or_insert_with(|| target.clone());
    }
    by_path.into_values().collect()
}

fn unique_locked_target_paths(
    _name: &SkillName,
    targets: &[LockedTarget],
    known_targets: &[ResolvedTarget],
) -> Result<Vec<ResolvedTarget>, StoreError> {
    let mut by_path = BTreeMap::new();
    for locked in targets {
        let target = find_target(known_targets, locked.agent)?.clone();
        by_path.entry(target.root.clone()).or_insert(target);
    }
    Ok(by_path.into_values().collect())
}

fn find_target(
    known_targets: &[ResolvedTarget],
    agent: AgentTargetId,
) -> Result<&ResolvedTarget, StoreError> {
    known_targets
        .iter()
        .find(|target| target.agent == agent)
        .ok_or_else(|| StoreError::InvalidTargetPath(PathBuf::from(agent.as_str())))
}

fn prepare_target_root(root: &Path) -> Result<(), StoreError> {
    reject_symlink_ancestors(root)
        .map_err(|_| StoreError::InvalidTargetPath(root.to_path_buf()))?;
    fs::create_dir_all(root).map_err(fs_error)?;
    reject_symlink_ancestors(root)
        .map_err(|_| StoreError::InvalidTargetPath(root.to_path_buf()))?;
    reject_directory_symlink(root, "Agent target")
        .map_err(|_| StoreError::InvalidTargetPath(root.to_path_buf()))
}

fn stage_target(
    install: &TargetInstall,
    canonical: &Path,
    canonical_stage: &Path,
    transaction: &str,
    expected_digest: &str,
) -> Result<(), StoreError> {
    prepare_target_root(&install.target.root)?;
    let name = SkillName::from_source(canonical)
        .map_err(|error| StoreError::InvalidSource(error.to_string()))?;
    let destination = install.target.destination(&name);
    let stage = stage_path(&destination, transaction)?;
    let result = match install.mode {
        InstallMode::Copy => copy_tree(canonical_stage, &stage),
        InstallMode::Symlink => {
            create_directory_symlink(&relative_path(&install.target.root, canonical)?, &stage)
                .map_err(fs_error)
        }
    };
    result?;
    if install.mode == InstallMode::Copy && hash_skill_tree(&stage)? != expected_digest {
        return Err(StoreError::InvalidSource(
            "the Agent target copy does not match the canonical Skill".to_owned(),
        ));
    }
    Ok(())
}

fn verify_managed_link(destination: &Path, canonical: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(destination).map_err(fs_error)?;
    if !metadata.file_type().is_symlink() {
        return Err(StoreError::Conflict(format!(
            "managed Agent target is no longer a link: {}",
            destination.display()
        )));
    }
    let link = fs::read_link(destination).map_err(fs_error)?;
    let parent = destination
        .parent()
        .ok_or_else(|| StoreError::InvalidTargetPath(destination.to_path_buf()))?;
    if normalize_path(&parent.join(link)) != normalize_path(canonical) {
        return Err(StoreError::Conflict(format!(
            "managed Agent target link changed: {}",
            destination.display()
        )));
    }
    Ok(())
}

fn source_digest(status: &SourceStatus) -> &str {
    match status {
        SourceStatus::Verified {
            installed_sha256, ..
        }
        | SourceStatus::Unverified {
            installed_sha256, ..
        } => installed_sha256,
        SourceStatus::Local { content_sha256 } => content_sha256,
    }
}

fn hash_skill_tree(root: &Path) -> Result<String, StoreError> {
    validate_skill_files(root)?;
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    for (relative, path) in files {
        let metadata = fs::symlink_metadata(&path).map_err(fs_error)?;
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(metadata.len().to_be_bytes());
        let mut file = File::open(path).map_err(fs_error)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(fs_error)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Ok(hex(&hasher.finalize()))
}

pub(crate) fn validate_skill_files(root: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(root).map_err(fs_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::InvalidSource(
            "the local Skill source must be a directory".to_owned(),
        ));
    }
    let skill = root.join("SKILL.md");
    let metadata = fs::symlink_metadata(&skill).map_err(|_| {
        StoreError::InvalidSource("the local Skill source must contain SKILL.md".to_owned())
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StoreError::InvalidSource(
            "SKILL.md must be a regular file".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_skill_source(root: &Path) -> Result<(), StoreError> {
    validate_skill_files(root)?;
    let directory_name = SkillName::from_source(root)
        .map_err(|error| StoreError::InvalidSource(error.to_string()))?;
    let frontmatter_name = read_frontmatter_name(&root.join("SKILL.md"))?;
    if frontmatter_name != directory_name.as_str() {
        return Err(StoreError::InvalidSource(format!(
            "SKILL.md name must match directory {}",
            directory_name.as_str()
        )));
    }
    Ok(())
}

fn read_frontmatter_name(path: &Path) -> Result<String, StoreError> {
    let file = File::open(path).map_err(fs_error)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|error| {
        StoreError::InvalidSource(format!("cannot read SKILL.md frontmatter: {error}"))
    })?;
    if line.trim_end_matches(['\r', '\n']) != "---" {
        return Err(StoreError::InvalidSource(
            "SKILL.md must start with frontmatter".to_owned(),
        ));
    }
    let mut bytes_read = line.len();
    let mut name = None;
    loop {
        line.clear();
        let read = reader.read_line(&mut line).map_err(|error| {
            StoreError::InvalidSource(format!("cannot read SKILL.md frontmatter: {error}"))
        })?;
        if read == 0 || bytes_read + read > 64 * 1024 {
            return Err(StoreError::InvalidSource(
                "SKILL.md frontmatter is not closed within 64 KiB".to_owned(),
            ));
        }
        bytes_read += read;
        let line = line.trim_end_matches(['\r', '\n']);
        if line == "---" {
            return name.ok_or_else(|| {
                StoreError::InvalidSource("SKILL.md frontmatter must contain name".to_owned())
            });
        }
        if let Some(value) = line.strip_prefix("name:") {
            if name.is_some() {
                return Err(StoreError::InvalidSource(
                    "SKILL.md frontmatter contains duplicate name fields".to_owned(),
                ));
            }
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| {
                    value
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                })
                .unwrap_or(value);
            SkillName::parse(value.to_owned())
                .map_err(|error| StoreError::InvalidSource(error.to_string()))?;
            name = Some(value.to_owned());
        }
    }
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), StoreError> {
    for entry in fs::read_dir(directory).map_err(fs_error)? {
        let entry = entry.map_err(fs_error)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(fs_error)?;
        if metadata.file_type().is_symlink() {
            return Err(StoreError::InvalidSource(
                "local Skill sources cannot contain links".to_owned(),
            ));
        }
        if metadata.is_dir() {
            collect_files(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| StoreError::InvalidSource(error.to_string()))?
                .to_str()
                .ok_or_else(|| StoreError::InvalidSource("Skill paths must use UTF-8".to_owned()))?
                .replace('\\', "/");
            files.push((relative, path));
        } else {
            return Err(StoreError::InvalidSource(
                "local Skill sources can contain only files and directories".to_owned(),
            ));
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), StoreError> {
    fs::create_dir(destination).map_err(fs_error)?;
    for entry in fs::read_dir(source).map_err(fs_error)? {
        let entry = entry.map_err(fs_error)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(fs_error)?;
        if metadata.file_type().is_symlink() {
            return Err(StoreError::InvalidSource(
                "local Skill sources cannot contain links".to_owned(),
            ));
        }
        if metadata.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            let mut destination_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination_path)
                .map_err(fs_error)?;
            let mut source_file = File::open(source_path).map_err(fs_error)?;
            io::copy(&mut source_file, &mut destination_file).map_err(fs_error)?;
            destination_file.sync_all().map_err(fs_error)?;
            fs::set_permissions(&destination_path, metadata.permissions()).map_err(fs_error)?;
        } else {
            return Err(StoreError::InvalidSource(
                "local Skill sources can contain only files and directories".to_owned(),
            ));
        }
    }
    let permissions = fs::symlink_metadata(source)
        .map_err(fs_error)?
        .permissions();
    fs::set_permissions(destination, permissions).map_err(fs_error)?;
    Ok(())
}

fn restore_path(
    destination: &Path,
    stage: &Path,
    backup: &Path,
    had_existing: bool,
) -> Result<(), StoreError> {
    remove_path(stage).map_err(fs_error)?;
    if path_exists(backup)? {
        remove_path(destination).map_err(fs_error)?;
        fs::rename(backup, destination).map_err(fs_error)?;
    } else if !had_existing {
        remove_path(destination).map_err(fs_error)?;
    }
    Ok(())
}

fn restore_lockfile(root: &Path, transaction: &str) -> Result<(), StoreError> {
    let lockfile = root.join(LOCKFILE_NAME);
    let stage = root.join(format!(".skilld-lock-stage-{transaction}"));
    let backup = root.join(format!(".skilld-lock-backup-{transaction}"));
    remove_path(&stage).map_err(fs_error)?;
    if path_exists(&backup)? {
        remove_path(&lockfile).map_err(fs_error)?;
        fs::rename(backup, lockfile).map_err(fs_error)?;
    }
    Ok(())
}

fn stage_path(destination: &Path, transaction: &str) -> Result<PathBuf, StoreError> {
    transaction_path(destination, ".skilld-stage", transaction)
}

fn backup_path(destination: &Path, transaction: &str) -> Result<PathBuf, StoreError> {
    transaction_path(destination, ".skilld-backup", transaction)
}

fn transaction_path(
    destination: &Path,
    prefix: &str,
    transaction: &str,
) -> Result<PathBuf, StoreError> {
    if !valid_transaction_id(transaction) {
        return Err(StoreError::InvalidLockfile(
            "invalid Skill transaction identifier".to_owned(),
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| StoreError::InvalidTargetPath(destination.to_path_buf()))?;
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StoreError::InvalidTargetPath(destination.to_path_buf()))?;
    Ok(parent.join(format!("{prefix}-{name}-{transaction}")))
}

fn valid_transaction_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn transaction_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{sequence}")
}

fn relative_path(from: &Path, to: &Path) -> Result<PathBuf, StoreError> {
    let from = normalize_path(from);
    let to = normalize_path(to);
    let from_parts = from.components().collect::<Vec<_>>();
    let to_parts = to.components().collect::<Vec<_>>();
    let shared = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();
    if shared == 0 {
        return Err(StoreError::InvalidTargetPath(to));
    }
    let mut relative = PathBuf::new();
    for _ in shared..from_parts.len() {
        relative.push("..");
    }
    for component in &to_parts[shared..] {
        relative.push(component.as_os_str());
    }
    Ok(relative)
}

#[cfg(unix)]
fn create_directory_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
fn create_directory_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(source, destination)
}

#[cfg(not(any(unix, windows)))]
fn create_directory_symlink(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "directory links are unavailable on this host",
    ))
}

#[cfg(not(target_os = "wasi"))]
fn ensure_write_capability() -> Result<(), StoreError> {
    Ok(())
}

#[cfg(target_os = "wasi")]
fn ensure_write_capability() -> Result<(), StoreError> {
    Err(StoreError::Unsupported(
        "WASIp2 Skill store locking is unavailable".to_owned(),
    ))
}

#[cfg(not(target_os = "wasi"))]
fn acquire_store_lock(root: &Path) -> Result<StoreLock, StoreError> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(LOCK_NAME))
        .map_err(fs_error)?;
    fs4::FileExt::lock(&lock).map_err(fs_error)?;
    Ok(StoreLock { _file: lock })
}

#[cfg(target_os = "wasi")]
fn acquire_store_lock(_root: &Path) -> Result<StoreLock, StoreError> {
    ensure_write_capability()?;
    Ok(StoreLock)
}

fn reject_directory_symlink(path: &Path, label: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} must be a directory"),
        ));
    }
    Ok(())
}

fn reject_symlink_ancestors(path: &Path) -> io::Result<()> {
    let mut ancestors = path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("managed path contains a link: {}", ancestor.display()),
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "managed path ancestor must be a directory: {}",
                        ancestor.display()
                    ),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)
        }
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn path_exists(path: &Path) -> Result<bool, StoreError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(fs_error(error)),
    }
}

fn fs_error(error: io::Error) -> StoreError {
    if error.kind() == io::ErrorKind::Unsupported {
        StoreError::Unsupported(error.to_string())
    } else if error.kind() == io::ErrorKind::InvalidInput {
        StoreError::InvalidSource(error.to_string())
    } else {
        StoreError::Filesystem(error.to_string())
    }
}

fn stale_update_plan() -> StoreError {
    StoreError::StalePlan("The Skill store changed while the update was preparing".to_owned())
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn absolute_normalized(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        return Ok(normalize_path(path));
    }
    std::env::current_dir().map(|current| normalize_path(&current.join(path)))
}

fn resolve_path(path: &Path) -> io::Result<PathBuf> {
    let mut existing = absolute_normalized(path)?;
    let mut missing = Vec::<OsString>::new();
    loop {
        match fs::canonicalize(&existing) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(normalize_path(&resolved));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(component) = existing.file_name().map(ToOwned::to_owned) else {
                    return Err(error);
                };
                existing.pop();
                missing.push(component);
            }
            Err(error) => return Err(error),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}
