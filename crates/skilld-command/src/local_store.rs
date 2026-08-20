use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use skilld_core::{InstallPlan, SkillName};

#[cfg(not(target_os = "wasi"))]
use std::fs::OpenOptions;

const JOURNAL_NAME: &str = ".skilld-transaction";
#[cfg(not(target_os = "wasi"))]
const LOCK_NAME: &str = ".skilld-store-lock";
static TRANSACTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(not(target_os = "wasi"))]
struct StoreLock {
    _file: File,
}

#[cfg(target_os = "wasi")]
struct StoreLock;

pub trait PromotionGate {
    fn before_promote(&self, destination: &Path) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AllowPromotion;

impl PromotionGate for AllowPromotion {
    fn before_promote(&self, _destination: &Path) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct LocalStore {
    root: PathBuf,
}

impl LocalStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn list(&self) -> io::Result<Vec<String>> {
        if !self.root.exists() {
            return Ok(vec![]);
        }
        match fs::read_dir(&self.root) {
            Ok(entries) => {
                let mut skills = Vec::new();
                for entry in entries {
                    if let Some(name) = installed_skill_name(&entry?.path())? {
                        skills.push(name);
                    }
                }
                skills.sort();
                Ok(skills)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(vec![]),
            Err(error) => Err(error),
        }
    }

    pub fn install_from(&self, source: &Path) -> io::Result<SkillName> {
        self.install_from_with_gate(source, &AllowPromotion)
    }

    pub fn install_from_with_gate<G: PromotionGate>(
        &self,
        source: &Path,
        gate: &G,
    ) -> io::Result<SkillName> {
        let source = absolute_normalized(source)?;
        validate_source(&source)?;
        let source = resolve_path(&source)?;
        reject_symlink_ancestors(&self.root)?;
        let store = resolve_path(&self.root)?;
        if source.starts_with(&store) || store.starts_with(&source) {
            return Err(invalid_input(
                "the local Skill source and store cannot overlap",
            ));
        }
        ensure_write_capability()?;
        fs::create_dir_all(&self.root)
            .map_err(|error| with_context("create the Skill store", error))?;
        reject_symlink_ancestors(&self.root)?;
        reject_symlink(&self.root, "Skill store")?;
        let _lock = acquire_store_lock(&self.root)?;
        self.recover()?;

        let plan = InstallPlan::local(source, self.root.clone()).map_err(invalid_input)?;
        let destination = plan.destination();
        if normalize_path(&destination) == plan.source {
            return Err(invalid_input("the local Skill is already the destination"));
        }
        if let Ok(metadata) = fs::symlink_metadata(&destination) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(invalid_input(
                    "the managed Skill destination must be a directory",
                ));
            }
        }

        let transaction = transaction_id();
        let stage = self.root.join(format!(
            ".skilld-stage-{}-{transaction}",
            plan.name.as_str()
        ));
        let backup = self.root.join(format!(
            ".skilld-backup-{}-{transaction}",
            plan.name.as_str()
        ));
        let journal = self.root.join(JOURNAL_NAME);
        if let Err(error) = write_journal(&journal, &plan.name, &transaction) {
            cleanup_path(&journal);
            return Err(error);
        }

        let copied = copy_tree(&plan.source, &stage);
        if let Err(error) = copied {
            cleanup_path(&stage);
            cleanup_path(&journal);
            return Err(error);
        }

        let had_destination = destination.exists();
        if had_destination {
            if let Err(error) = fs::rename(&destination, &backup) {
                cleanup_path(&stage);
                cleanup_path(&journal);
                return Err(error);
            }
        }

        let promoted = gate
            .before_promote(&destination)
            .and_then(|()| fs::rename(&stage, &destination));
        if let Err(error) = promoted {
            cleanup_path(&stage);
            if had_destination {
                if let Err(rollback_error) = fs::rename(&backup, &destination) {
                    return Err(io::Error::other(format!(
                        "{error}; rollback failed: {rollback_error}"
                    )));
                }
            }
            cleanup_path(&journal);
            return Err(error);
        }

        remove_path(&backup)?;
        remove_path(&journal)?;
        Ok(plan.name)
    }

    fn recover(&self) -> io::Result<()> {
        let journal = self.root.join(JOURNAL_NAME);
        if !journal.exists() {
            return Ok(());
        }
        let contents = fs::read_to_string(journal.join("state"))?;
        let mut lines = contents.lines();
        let name = SkillName::parse(lines.next().unwrap_or_default()).map_err(invalid_input)?;
        let transaction = lines.next().unwrap_or_default();
        if transaction.is_empty()
            || transaction.len() > 96
            || !transaction
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(invalid_input("invalid Skill transaction journal"));
        }

        let destination = self.root.join(name.as_str());
        let stage = self
            .root
            .join(format!(".skilld-stage-{}-{transaction}", name.as_str()));
        let backup = self
            .root
            .join(format!(".skilld-backup-{}-{transaction}", name.as_str()));

        if !destination.exists() && backup.exists() {
            fs::rename(&backup, &destination)?;
        }
        remove_path(&stage)?;
        if destination.exists() {
            remove_path(&backup)?;
        }
        remove_path(&journal)?;
        Ok(())
    }
}

fn installed_skill_name(path: &Path) -> io::Result<Option<String>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(None);
    }
    let skill_file = path.join("SKILL.md");
    let skill_metadata = match fs::symlink_metadata(skill_file) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if skill_metadata.file_type().is_symlink() || !skill_metadata.is_file() {
        return Ok(None);
    }
    SkillName::from_source(path)
        .map(|name| Some(name.to_string()))
        .map_err(invalid_input)
}

fn validate_source(source: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| with_context("read the local Skill source", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_input("the local Skill source must be a directory"));
    }
    let skill_file = source.join("SKILL.md");
    let metadata = fs::symlink_metadata(&skill_file)
        .map_err(|_| invalid_input("the local Skill source must contain SKILL.md"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_input("SKILL.md must be a regular file"));
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_input("local Skill sources cannot contain links"));
        }
        if metadata.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            let mut destination_file = File::create(destination_path)?;
            let mut source_file = File::open(source_path)?;
            io::copy(&mut source_file, &mut destination_file)?;
        } else {
            return Err(invalid_input(
                "local Skill sources can contain only files and directories",
            ));
        }
    }
    Ok(())
}

fn write_journal(path: &Path, name: &SkillName, transaction: &str) -> io::Result<()> {
    fs::create_dir(path).map_err(|error| with_context("create the transaction journal", error))?;
    let mut file = File::create(path.join("state"))
        .map_err(|error| with_context("open the transaction journal", error))?;
    writeln!(file, "{name}")
        .map_err(|error| with_context("write the transaction journal", error))?;
    writeln!(file, "{transaction}")
        .map_err(|error| with_context("write the transaction journal", error))
}

#[cfg(not(target_os = "wasi"))]
fn ensure_write_capability() -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "wasi")]
fn ensure_write_capability() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "WASIp2 Skill store locking is unavailable",
    ))
}

#[cfg(not(target_os = "wasi"))]
fn acquire_store_lock(root: &Path) -> io::Result<StoreLock> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(LOCK_NAME))
        .map_err(|error| with_context("open the Skill store lock", error))?;
    fs4::FileExt::lock(&lock).map_err(|error| with_context("lock the Skill store", error))?;
    Ok(StoreLock { _file: lock })
}

#[cfg(target_os = "wasi")]
fn acquire_store_lock(_root: &Path) -> io::Result<StoreLock> {
    Ok(StoreLock)
}

fn transaction_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{sequence}")
}

fn reject_symlink(path: &Path, label: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_input(format!("{label} must be a directory")));
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
                return Err(invalid_input(format!(
                    "Skill store path contains a link: {}",
                    ancestor.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(invalid_input(format!(
                    "Skill store ancestor must be a directory: {}",
                    ancestor.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn cleanup_path(path: &Path) {
    // Cleanup preserves the primary transaction error when recovery is still possible.
    let _ = remove_path(path);
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

fn invalid_input(error: impl fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}

fn with_context(context: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("cannot {context}: {error}"))
}

fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;

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
    std::env::current_dir()
        .map(|current| normalize_path(&current.join(path)))
        .map_err(|error| with_context("resolve the path", error))
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
                    return Err(with_context("resolve the path", error));
                };
                existing.pop();
                missing.push(component);
            }
            Err(error) => return Err(with_context("resolve the path", error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    struct RejectPromotion;

    impl PromotionGate for RejectPromotion {
        fn before_promote(&self, _destination: &Path) -> io::Result<()> {
            Err(io::Error::other("injected promotion failure"))
        }
    }

    struct BlockingPromotion {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl PromotionGate for BlockingPromotion {
        fn before_promote(&self, _destination: &Path) -> io::Result<()> {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
            Ok(())
        }
    }

    #[test]
    fn failed_promotion_restores_the_installed_skill() {
        let temporary = tempfile::tempdir().unwrap();
        let store = LocalStore::new(temporary.path().join(".skills"));
        let source = temporary.path().join("example");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("SKILL.md"), "old").unwrap();
        store.install_from(&source).unwrap();
        fs::write(source.join("SKILL.md"), "new").unwrap();

        let result = store.install_from_with_gate(&source, &RejectPromotion);

        assert_eq!(
            result.unwrap_err().to_string(),
            "injected promotion failure"
        );
        assert_eq!(
            fs::read_to_string(temporary.path().join(".skills/example/SKILL.md")).unwrap(),
            "old"
        );
        assert!(
            !temporary
                .path()
                .join(".skills/.skilld-transaction")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_links_are_rejected() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("example");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("SKILL.md"), "fixture").unwrap();
        symlink("SKILL.md", source.join("linked.md")).unwrap();

        let error = LocalStore::new(temporary.path().join(".skills"))
            .install_from(&source)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "local Skill sources cannot contain links"
        );
        assert!(!temporary.path().join(".skills/example").exists());
    }

    #[cfg(unix)]
    #[test]
    fn skill_store_ancestor_links_are_rejected_before_writes() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("example");
        let actual_store = temporary.path().join("actual-store");
        let linked_store = temporary.path().join("linked-store");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("SKILL.md"), "fixture").unwrap();
        fs::create_dir(&actual_store).unwrap();
        symlink(&actual_store, &linked_store).unwrap();

        let error = LocalStore::new(linked_store.join("skills"))
            .install_from(&source)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("Skill store path contains a link:")
        );
        assert!(!actual_store.join("skills").exists());
    }

    #[test]
    fn source_and_store_cannot_overlap() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("example");
        let store_root = source.join(".skills");
        let journal = store_root.join(JOURNAL_NAME);
        fs::create_dir_all(&journal).unwrap();
        fs::write(source.join("SKILL.md"), "fixture").unwrap();
        fs::write(journal.join("state"), "invalid").unwrap();

        let error = LocalStore::new(store_root)
            .install_from(&source)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "the local Skill source and store cannot overlap"
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_and_store_aliases_cannot_overlap() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let actual_parent = temporary.path().join("actual");
        let alias_parent = temporary.path().join("alias");
        let source = actual_parent.join("example");
        let alias = alias_parent.join("example");
        let store_root = source.join(".skills");
        let journal = store_root.join(JOURNAL_NAME);
        fs::create_dir_all(&journal).unwrap();
        fs::write(source.join("SKILL.md"), "fixture").unwrap();
        fs::write(journal.join("state"), "invalid").unwrap();
        symlink(&actual_parent, &alias_parent).unwrap();

        let error = LocalStore::new(store_root)
            .install_from(&alias)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "the local Skill source and store cannot overlap"
        );
    }

    #[test]
    fn concurrent_installs_are_serialized() {
        let temporary = tempfile::tempdir().unwrap();
        let store = LocalStore::new(temporary.path().join(".skills"));
        let first = temporary.path().join("first/example");
        let second = temporary.path().join("second/example");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("SKILL.md"), "first").unwrap();
        fs::write(second.join("SKILL.md"), "second").unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_store = store.clone();
        let first_install = thread::spawn(move || {
            first_store.install_from_with_gate(
                &first,
                &BlockingPromotion {
                    entered: entered_tx,
                    release: release_rx,
                },
            )
        });
        entered_rx.recv().unwrap();
        let second_store = store.clone();
        let (second_tx, second_rx) = mpsc::channel();
        let second_install = thread::spawn(move || {
            second_tx.send(second_store.install_from(&second)).unwrap();
        });

        assert!(matches!(
            second_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release_tx.send(()).unwrap();

        assert!(first_install.join().unwrap().is_ok());
        assert!(
            second_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_ok()
        );
        second_install.join().unwrap();
        assert_eq!(
            fs::read_to_string(store.root.join("example/SKILL.md")).unwrap(),
            "second"
        );
    }
}
