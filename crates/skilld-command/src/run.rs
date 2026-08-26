//! Transient Skill loads.
//!
//! `skilld run` hands the calling Agent a Skill now. A remote run retains no
//! Skill files and creates no lockfile entry, Agent target, or project file.
//!
//! The initial load names supporting files without printing their content.
//! The Agent reads only the files that the instructions name.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use skilld_core::PreparedFile;
use skilld_ui::text::is_unsafe_terminal;

use crate::CommandError;

/// The instructions file every Skill carries.
pub const INSTRUCTIONS_FILE: &str = "SKILL.md";

const MAX_LOCAL_DEPTH: usize = 8;
const MAX_LOCAL_FILES: usize = 512;
const MAX_LOCAL_BYTES: u64 = 64 * 1024 * 1024;

/// Where a transient Skill came from, and what that means for its files.
///
/// A local Skill already sits on the user's disk, so its files carry a path.
/// Bundled and remote Skills have no path to give.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillOrigin {
    Bundled,
    Local {
        root: PathBuf,
    },
    Remote {
        source: String,
        exact_source: String,
        direct: bool,
    },
}

/// How skilld can hand one supporting file to an Agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    /// UTF-8 text. skilld prints it on request.
    Text,
    /// Marked executable by its author. skilld never prints it.
    Executable,
    /// Not valid UTF-8. skilld never prints it.
    Binary,
}

impl FileKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Executable => "executable",
            Self::Binary => "binary",
        }
    }

    /// Whether skilld will print this file's bytes.
    pub const fn is_readable(self) -> bool {
        matches!(self, Self::Text)
    }
}

/// One supporting file, named but not delivered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupportingFile {
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
}

/// One transient Skill: loaded for this session, recorded nowhere.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransientSkill {
    pub name: String,
    pub instructions: String,
    pub origin: SkillOrigin,
    /// `verified`, `local`, or `unverified`.
    pub source_status: &'static str,
    /// The exact remote Git commit. Local and bundled Skills have no revision.
    pub revision: Option<String>,
    pub files: Vec<SupportingFile>,
}

/// One supporting file the Agent asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PulledFile {
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
    pub content: FileContent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileContent {
    Text(String),
    /// skilld holds the bytes but will not print them.
    Withheld {
        reason: &'static str,
    },
}

/// What one `skilld run` invocation produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    Load(Box<TransientSkill>),
    Files {
        skill: String,
        origin: SkillOrigin,
        source_status: &'static str,
        revision: Option<String>,
        files: Vec<PulledFile>,
    },
}

pub(crate) fn reject_duplicate_files(wanted: &[String]) -> Result<(), CommandError> {
    if wanted
        .iter()
        .any(|path| path.chars().any(is_unsafe_terminal))
    {
        return Err(CommandError::input(
            "--file paths cannot contain terminal formatting characters",
        ));
    }
    let mut unique = BTreeSet::new();
    if wanted.iter().any(|path| !unique.insert(path)) {
        return Err(CommandError::input(
            "each --file path must appear only once",
        ));
    }
    Ok(())
}

/// Read the SKILL.md text out of a file set.
pub fn read_instructions(files: &[PreparedFile]) -> Result<String, CommandError> {
    let file = instructions_file(files)?;
    decode(&file.bytes).ok_or_else(|| {
        CommandError::operation("INVALID_ARTIFACT", "the SKILL.md file is not valid UTF-8")
    })
}

fn instructions_file(files: &[PreparedFile]) -> Result<&PreparedFile, CommandError> {
    files
        .iter()
        .find(|file| file.path == INSTRUCTIONS_FILE)
        .ok_or_else(|| {
            CommandError::operation("INVALID_ARTIFACT", "the Skill has no SKILL.md file")
        })
}

/// Describe every supporting file a Skill carries, without delivering one.
pub fn supporting_files(files: &[PreparedFile]) -> Vec<SupportingFile> {
    files
        .iter()
        .filter(|file| file.path != INSTRUCTIONS_FILE)
        .map(|file| {
            let kind = classify(file);
            SupportingFile {
                path: file.path.clone(),
                kind,
                size: file.bytes.len() as u64,
            }
        })
        .collect()
}

/// Hand over the supporting files the Agent named.
///
/// An unknown path fails the whole run. A path skilld will not print comes back
/// withheld, so the Agent learns the file exists and learns why it did not get it.
pub fn pull_files(
    skill: &str,
    files: &[PreparedFile],
    wanted: &[String],
) -> Result<Vec<PulledFile>, CommandError> {
    wanted
        .iter()
        .map(|path| {
            if path == INSTRUCTIONS_FILE {
                return Err(CommandError::input(
                    "SKILL.md arrives with every run. Drop --file SKILL.md.",
                ));
            }
            let file = files
                .iter()
                .find(|file| &file.path == path)
                .ok_or_else(|| {
                    CommandError::operation(
                        "SOURCE_NOT_FOUND",
                        format!("the Skill {skill} carries no file at {path}"),
                    )
                })?;
            let kind = classify(file);
            Ok(PulledFile {
                path: file.path.clone(),
                kind,
                size: file.bytes.len() as u64,
                content: match kind {
                    FileKind::Text => decode(&file.bytes).map_or(
                        FileContent::Withheld {
                            reason: "the file is not valid UTF-8",
                        },
                        FileContent::Text,
                    ),
                    FileKind::Executable => FileContent::Withheld {
                        reason: "the Skill marks this file executable",
                    },
                    FileKind::Binary => FileContent::Withheld {
                        reason: "the file is not valid UTF-8",
                    },
                },
            })
        })
        .collect()
}

/// Read a Skill that already sits on disk.
///
/// A local Skill needs no delivery decision. The user owns these files already.
pub fn read_local(path: &Path) -> Result<(String, Vec<PreparedFile>), CommandError> {
    let path_text = path
        .to_str()
        .ok_or_else(|| invalid_local("Skill paths must use UTF-8"))?;
    reject_local_path_controls(path_text)?;
    crate::local_store::validate_skill_files(path).map_err(CommandError::store)?;
    let mut inventory = Vec::new();
    let mut total = 0;
    collect_local_metadata(path, Path::new(""), 0, &mut inventory, &mut total)?;
    crate::local_store::validate_skill_source(path).map_err(CommandError::store)?;
    let name = skilld_core::SkillName::from_source(path)
        .map_err(CommandError::domain)?
        .to_string();
    inventory.sort_by(|left, right| left.relative.cmp(&right.relative));
    let files = inventory
        .into_iter()
        .map(read_local_file)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((name, files))
}

struct LocalFile {
    path: PathBuf,
    relative: String,
    mode: u32,
    size: u64,
}

fn collect_local_metadata(
    root: &Path,
    relative: &Path,
    depth: usize,
    files: &mut Vec<LocalFile>,
    total: &mut u64,
) -> Result<(), CommandError> {
    if depth > MAX_LOCAL_DEPTH {
        return Err(too_large("the local Skill exceeds its depth limit"));
    }
    let entries = fs::read_dir(root.join(relative)).map_err(|error| {
        CommandError::operation(
            "SOURCE_NOT_FOUND",
            format!("cannot read the Skill directory: {error}"),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            CommandError::filesystem(format!("cannot read a Skill file: {error}"))
        })?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| invalid_local("Skill paths must use UTF-8"))?;
        reject_local_path_controls(name)?;
        let child = relative.join(name);
        let file_type = entry.file_type().map_err(|error| {
            CommandError::filesystem(format!("cannot read a Skill file: {error}"))
        })?;
        if file_type.is_symlink() {
            return Err(invalid_local("local Skill sources cannot contain links"));
        }
        let metadata = entry.metadata().map_err(|error| {
            CommandError::filesystem(format!("cannot read a Skill file: {error}"))
        })?;
        if metadata.is_dir() {
            collect_local_metadata(root, &child, depth + 1, files, total)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(invalid_local(
                "local Skill sources can contain only files and directories",
            ));
        }
        if files.len() >= MAX_LOCAL_FILES {
            return Err(too_large("the local Skill exceeds its file limit"));
        }
        *total = total
            .checked_add(metadata.len())
            .ok_or_else(|| too_large("the local Skill exceeds its content limit"))?;
        if *total > MAX_LOCAL_BYTES {
            return Err(too_large("the local Skill exceeds its content limit"));
        }
        files.push(LocalFile {
            path: entry.path(),
            relative: child
                .to_str()
                .ok_or_else(|| invalid_local("Skill paths must use UTF-8"))?
                .replace('\\', "/"),
            mode: local_mode(&metadata),
            size: metadata.len(),
        });
    }
    Ok(())
}

fn read_local_file(file: LocalFile) -> Result<PreparedFile, CommandError> {
    let input = File::open(&file.path)
        .map_err(|error| CommandError::filesystem(format!("cannot read a Skill file: {error}")))?;
    let mut bytes = Vec::new();
    input
        .take(file.size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| CommandError::filesystem(format!("cannot read a Skill file: {error}")))?;
    if bytes.len() as u64 != file.size {
        return Err(invalid_local(
            "a local Skill file changed while skilld read it",
        ));
    }
    Ok(PreparedFile {
        path: file.relative,
        mode: file.mode,
        bytes,
    })
}

fn invalid_local(message: &'static str) -> CommandError {
    CommandError::operation("INVALID_SOURCE", message)
}

fn reject_local_path_controls(value: &str) -> Result<(), CommandError> {
    if value.chars().any(is_unsafe_terminal) {
        return Err(invalid_local(
            "local Skill paths cannot contain terminal formatting characters",
        ));
    }
    Ok(())
}

fn too_large(message: &'static str) -> CommandError {
    CommandError::operation("SKILL_TOO_LARGE", message)
}

#[cfg(unix)]
fn local_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o111 == 0 {
        0o644
    } else {
        0o755
    }
}

#[cfg(not(unix))]
fn local_mode(_metadata: &fs::Metadata) -> u32 {
    0o644
}

fn classify(file: &PreparedFile) -> FileKind {
    if file.mode & 0o111 != 0 {
        return FileKind::Executable;
    }
    if std::str::from_utf8(&file.bytes).is_ok() {
        FileKind::Text
    } else {
        FileKind::Binary
    }
}

fn decode(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}
