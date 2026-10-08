use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::CommandError;

const MAX_DEPTH: usize = 32;
const MAX_FILES: usize = 10_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_LOCK_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Fingerprint {
    pub sha256: String,
    pub git_tree: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ForeignRecord {
    pub lockfile: PathBuf,
    pub name: String,
    pub source: String,
    pub source_type: String,
    pub skill_path: Option<String>,
    pub revision: Option<String>,
    pub expected_tree: Option<String>,
    pub expected_content: Option<String>,
    pub lock_sha256: String,
    pub global: bool,
}

/// Hash the complete Skill tree, excluding only `.git` entries.
///
/// The SHA256 includes framed names, file kinds, permissions and link targets.
/// The Git SHA1 uses Git's tree encoding and omits empty directories.
/// Internal links remain links. Absolute and escaping links are rejected.
pub fn fingerprint(root: &Path) -> Result<Fingerprint, CommandError> {
    let metadata = fs::symlink_metadata(root).map_err(tree_io)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(unsupported_tree());
    }
    let mut budget = Budget {
        files: 0,
        bytes: 0,
        entries: 0,
    };
    let resolved_root = fs::canonicalize(root).map_err(tree_io)?;
    let tree = hash_directory(root, &resolved_root, 0, &mut budget)?;
    Ok(Fingerprint {
        sha256: hex(&tree.sha256),
        git_tree: hex(&tree.git_tree),
        files: budget.files,
        bytes: budget.bytes,
    })
}

struct Budget {
    files: usize,
    bytes: u64,
    entries: usize,
}

struct TreeHash {
    sha256: Vec<u8>,
    git_tree: Vec<u8>,
    has_git_entries: bool,
}

fn hash_directory(
    path: &Path,
    root: &Path,
    depth: usize,
    budget: &mut Budget,
) -> Result<TreeHash, CommandError> {
    if depth > MAX_DEPTH {
        return Err(scan_limit());
    }
    let before = fs::symlink_metadata(path).map_err(tree_io)?;
    if !before.is_dir() || before.file_type().is_symlink() {
        return Err(unsupported_tree());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(tree_io)? {
        let entry = entry.map_err(tree_io)?;
        if entry.file_name() == ".git" {
            continue;
        }
        budget.entries += 1;
        if budget.entries > MAX_FILES * 2 {
            return Err(scan_limit());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(tree_io)?;
        let mut sort_key = entry.file_name().as_encoded_bytes().to_vec();
        if metadata.is_dir() {
            sort_key.push(b'/');
        }
        entries.push((sort_key, entry, metadata));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    frame(&mut digest, b"skilld-doctor-tree-v1");
    frame(&mut digest, &permission_mode(&before).to_be_bytes());
    let mut git_body = Vec::new();
    for (_, entry, metadata) in entries {
        let name = entry.file_name();
        frame(&mut digest, name.as_encoded_bytes());
        frame(&mut digest, &permission_mode(&metadata).to_be_bytes());
        let (git_mode, object) = if metadata.is_dir() {
            frame(&mut digest, b"directory");
            let child = hash_directory(&entry.path(), root, depth + 1, budget)?;
            frame(&mut digest, &child.sha256);
            if !child.has_git_entries {
                continue;
            }
            ("40000", child.git_tree)
        } else {
            budget.files += 1;
            if budget.files > MAX_FILES {
                return Err(scan_limit());
            }
            let (kind, mode, content) = if metadata.file_type().is_symlink() {
                let target = fs::read_link(entry.path()).map_err(tree_io)?;
                check_link(&target, depth)?;
                // Resolve location only. Never read or recurse through link contents.
                let resolved = fs::canonicalize(entry.path()).map_err(|_| unsupported_tree())?;
                if !resolved.starts_with(root) {
                    return Err(unsupported_tree());
                }
                let bytes = target.as_os_str().as_encoded_bytes().to_vec();
                add_bytes(budget, bytes.len() as u64)?;
                (b"symlink".as_slice(), "120000", bytes)
            } else if metadata.is_file() {
                if metadata.len() > MAX_BYTES - budget.bytes {
                    return Err(scan_limit());
                }
                let content =
                    read_regular(&entry.path(), &metadata, MAX_BYTES - budget.bytes, tree_io)?;
                add_bytes(budget, content.len() as u64)?;
                let mode = if permission_mode(&metadata) & 0o100 != 0 {
                    "100755"
                } else {
                    "100644"
                };
                (b"file".as_slice(), mode, content)
            } else {
                return Err(unsupported_tree());
            };
            ensure_unchanged(&entry.path(), &metadata, tree_io)?;
            frame(&mut digest, kind);
            frame(&mut digest, &content);
            (mode, git_object("blob", &content))
        };
        git_body.extend_from_slice(git_mode.as_bytes());
        git_body.push(b' ');
        git_body.extend_from_slice(name.as_encoded_bytes());
        git_body.push(0);
        git_body.extend_from_slice(&object);
    }
    ensure_unchanged(path, &before, tree_io)?;
    Ok(TreeHash {
        sha256: digest.finalize().to_vec(),
        git_tree: git_object("tree", &git_body),
        has_git_entries: !git_body.is_empty(),
    })
}

fn add_bytes(budget: &mut Budget, bytes: u64) -> Result<(), CommandError> {
    if bytes > MAX_BYTES - budget.bytes {
        return Err(scan_limit());
    }
    budget.bytes += bytes;
    Ok(())
}

fn check_link(target: &Path, parent_depth: usize) -> Result<(), CommandError> {
    let mut depth = parent_depth;
    for component in target.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return Err(unsupported_tree()),
        }
    }
    Ok(())
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn git_object(kind: &str, body: &[u8]) -> Vec<u8> {
    let mut hash = Sha1::new();
    hash.update(format!("{kind} {}\0", body.len()));
    hash.update(body);
    hash.finalize().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(unix)]
fn permission_mode(metadata: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn permission_mode(metadata: &Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

fn same_metadata(left: &Metadata, right: &Metadata) -> bool {
    let same = left.file_type() == right.file_type()
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && permission_mode(left) == permission_mode(right);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same && left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.ctime() == right.ctime()
            && left.ctime_nsec() == right.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        same
    }
}

fn ensure_unchanged(
    path: &Path,
    before: &Metadata,
    error: fn(std::io::Error) -> CommandError,
) -> Result<(), CommandError> {
    let after = fs::symlink_metadata(path).map_err(error)?;
    if !same_metadata(before, &after) {
        return Err(CommandError::operation(
            "DOCTOR_CONTENT_CHANGED",
            "Skill files changed during the scan. Run doctor again.",
        ));
    }
    Ok(())
}

fn read_regular(
    path: &Path,
    before: &Metadata,
    limit: u64,
    error: fn(std::io::Error) -> CommandError,
) -> Result<Vec<u8>, CommandError> {
    let file = File::open(path).map_err(error)?;
    let opened = file.metadata().map_err(error)?;
    if !opened.is_file() || !same_metadata(before, &opened) {
        return Err(CommandError::operation(
            "DOCTOR_CONTENT_CHANGED",
            "Skill files changed during the scan. Run doctor again.",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() as u64 > limit {
        return Err(scan_limit());
    }
    ensure_unchanged(path, before, error)?;
    Ok(bytes)
}

/// Read known skills.sh formats without exposing URLs or arbitrary metadata.
pub fn read_foreign_lock(path: &Path, global: bool) -> Result<Vec<ForeignRecord>, CommandError> {
    let bytes = read_lock_bytes(path)?;
    let document = parse_lock(&bytes, global)?;
    records_from_document(path, global, &bytes, &document)
}

/// Prepare a foreign lockfile with exactly one selected Skill removed.
/// The caller owns atomic replacement and rollback across both installers.
pub fn foreign_lock_without(path: &Path, record: &ForeignRecord) -> Result<Vec<u8>, CommandError> {
    if path != record.lockfile {
        return Err(stale_lock());
    }
    let bytes = read_lock_bytes(path)?;
    if hex(&Sha256::digest(&bytes)) != record.lock_sha256 {
        return Err(stale_lock());
    }
    let mut document = parse_lock(&bytes, record.global)?;
    let skills = document
        .get_mut("skills")
        .and_then(Value::as_object_mut)
        .ok_or_else(invalid_lock)?;
    if skills.remove(&record.name).is_none() {
        return Err(stale_lock());
    }
    let mut output = serde_json::to_vec_pretty(&document).map_err(|_| invalid_lock())?;
    output.push(b'\n');
    Ok(output)
}

fn read_lock_bytes(path: &Path) -> Result<Vec<u8>, CommandError> {
    let metadata = fs::symlink_metadata(path).map_err(lock_io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_LOCK_BYTES {
        return Err(invalid_lock());
    }
    read_regular(path, &metadata, MAX_LOCK_BYTES, lock_io)
}

fn parse_lock(bytes: &[u8], global: bool) -> Result<Value, CommandError> {
    let UniqueValue(document) = serde_json::from_slice(bytes).map_err(|_| invalid_lock())?;
    if document.get("version").and_then(Value::as_u64) != Some(if global { 3 } else { 1 })
        || document.get("skills").and_then(Value::as_object).is_none()
    {
        return Err(invalid_lock());
    }
    Ok(document)
}

fn records_from_document(
    path: &Path,
    global: bool,
    bytes: &[u8],
    document: &Value,
) -> Result<Vec<ForeignRecord>, CommandError> {
    let skills = document
        .get("skills")
        .and_then(Value::as_object)
        .ok_or_else(invalid_lock)?;
    if skills.len() > MAX_FILES {
        return Err(invalid_lock());
    }
    let lock_sha256 = hex(&Sha256::digest(bytes));
    skills
        .iter()
        .map(|(name, entry)| {
            if !safe_component(name) || !entry.is_object() {
                return Err(invalid_lock());
            }
            let source = required_text(entry, "source", 4096)?;
            let source_type = required_text(entry, "sourceType", 64)?;
            if !safe_component(&source_type)
                || !source_without_credentials(&source)
                || (source_type == "github" && !github_source(&source))
            {
                return Err(invalid_lock());
            }
            let skill_path = optional_text(entry, "skillPath", 4096)?;
            if skill_path
                .as_ref()
                .is_some_and(|path| !relative_source_path(path))
            {
                return Err(invalid_lock());
            }
            let revision = optional_text(entry, "ref", 1024)?;
            let raw_hash = optional_text(
                entry,
                if global {
                    "skillFolderHash"
                } else {
                    "computedHash"
                },
                64,
            )?;
            if raw_hash.as_ref().is_some_and(|hash| {
                !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || (!matches!(hash.len(), 40 | 64))
                    || (!global && hash.len() != 64)
            }) {
                return Err(invalid_lock());
            }
            let expected_tree = if global && source_type == "github" {
                raw_hash.clone().filter(|hash| hash.len() == 40)
            } else {
                None
            };
            let expected_content = raw_hash.filter(|hash| hash.len() == 64);
            Ok(ForeignRecord {
                lockfile: path.to_owned(),
                name: name.clone(),
                source,
                source_type,
                skill_path,
                revision,
                expected_tree,
                expected_content,
                lock_sha256: lock_sha256.clone(),
                global,
            })
        })
        .collect()
}

fn required_text(entry: &Value, field: &str, limit: usize) -> Result<String, CommandError> {
    optional_text(entry, field, limit)?.ok_or_else(invalid_lock)
}

fn optional_text(entry: &Value, field: &str, limit: usize) -> Result<Option<String>, CommandError> {
    match entry.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.is_empty() => Ok(None),
        Some(Value::String(value))
            if value.len() <= limit
                && value.trim() == value
                && !value.chars().any(char::is_control) =>
        {
            Ok(Some(value.clone()))
        }
        _ => Err(invalid_lock()),
    }
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn github_source(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2 && parts.iter().all(|part| safe_component(part))
}

fn relative_source_path(value: &str) -> bool {
    !value.contains(['\\', ':'])
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn source_without_credentials(source: &str) -> bool {
    if source.contains("://") {
        return url::Url::parse(source).is_ok_and(|url| {
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
    }
    true
}

/// Duplicate keys make surgical edits ambiguous. Reject them at every depth.
struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, UniqueValue(value))) =
                    map.next_entry::<String, UniqueValue>()?
                {
                    if values.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate object key"));
                    }
                }
                Ok(UniqueValue(Value::Object(values)))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.to_owned())))
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|number| UniqueValue(Value::Number(number)))
                    .ok_or_else(|| serde::de::Error::custom("invalid number"))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

fn invalid_lock() -> CommandError {
    CommandError::operation(
        "DOCTOR_INVALID_LOCKFILE",
        "The skills.sh lockfile has an unsupported format or invalid Skill record.",
    )
}

fn stale_lock() -> CommandError {
    CommandError::operation(
        "DOCTOR_STALE_LOCKFILE",
        "The skills.sh lockfile changed. Run doctor again before changing Skills.",
    )
}

fn unsupported_tree() -> CommandError {
    CommandError::operation(
        "DOCTOR_UNSUPPORTED_TREE",
        "The Skill contains an unsupported file or a link outside its directory.",
    )
}

fn scan_limit() -> CommandError {
    CommandError::operation(
        "DOCTOR_SCAN_LIMIT",
        "The Skill exceeds the scan limit for depth, files, or bytes.",
    )
}

fn tree_io(error: std::io::Error) -> CommandError {
    CommandError::operation(
        "DOCTOR_READ_FAILED",
        format!("Cannot read the Skill directory: {error}"),
    )
}

fn lock_io(error: std::io::Error) -> CommandError {
    CommandError::operation(
        "DOCTOR_READ_FAILED",
        format!("Cannot read the skills.sh lockfile: {error}"),
    )
}
