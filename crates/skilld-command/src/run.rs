//! Transient Skill loads.
//!
//! `skilld run` hands the calling Agent a Skill now. A remote run writes
//! nothing: no lockfile entry, no Agent target, no project file, and no cache.
//! The Skill arrives in memory, the Agent reads what it asks for, and the
//! process exit takes the rest with it.
//!
//! Supporting files are named, never poured out. The Agent pulls the ones the
//! instructions call for. A file skilld cannot hand over as text is a file the
//! Agent needs on disk, and putting it there is what `skilld install` is for.

use std::fs;
use std::path::{Path, PathBuf};

use skilld_core::PreparedFile;

use crate::CommandError;

/// The instructions file every Skill carries.
pub const INSTRUCTIONS_FILE: &str = "SKILL.md";

const MAX_LOCAL_DEPTH: usize = 8;
const MAX_LOCAL_FILES: usize = 512;
const SUMMARY_WIDTH: usize = 80;

/// Where a transient Skill came from, and what that means for its files.
///
/// A local Skill already sits on the user's disk, so its files carry a path. A
/// remote Skill never lands, so it has no path to give.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillOrigin {
    Local { root: PathBuf },
    Remote { source: String, direct: bool },
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
    /// One line describing the file, read from its own content.
    pub summary: Option<String>,
}

/// One transient Skill: loaded for this session, recorded nowhere.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransientSkill {
    pub name: String,
    pub instructions: String,
    pub origin: SkillOrigin,
    /// `verified`, `local`, or `unverified`.
    pub source_status: &'static str,
    pub files: Vec<SupportingFile>,
}

/// One supporting file the Agent asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PulledFile {
    pub skill: String,
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
    Files(Vec<PulledFile>),
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
                summary: kind.is_readable().then(|| summarize(&file.bytes)).flatten(),
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
                skill: skill.to_owned(),
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
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            CommandError::operation("INVALID_SOURCE", "the Skill directory has no usable name")
        })?
        .to_owned();
    let mut files = Vec::new();
    collect_local(path, Path::new(""), 0, &mut files)?;
    if !files.iter().any(|file| file.path == INSTRUCTIONS_FILE) {
        return Err(CommandError::operation(
            "SOURCE_NOT_FOUND",
            format!("cannot read {INSTRUCTIONS_FILE} in this directory"),
        ));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((name, files))
}

fn collect_local(
    root: &Path,
    relative: &Path,
    depth: usize,
    files: &mut Vec<PreparedFile>,
) -> Result<(), CommandError> {
    if depth > MAX_LOCAL_DEPTH || files.len() >= MAX_LOCAL_FILES {
        return Ok(());
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
        let Some(name) = name.to_str() else { continue };
        let child = relative.join(name);
        let kind = entry.file_type().map_err(|error| {
            CommandError::filesystem(format!("cannot read a Skill file: {error}"))
        })?;
        if kind.is_dir() {
            collect_local(root, &child, depth + 1, files)?;
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let Some(path) = child.to_str() else { continue };
        if files.len() >= MAX_LOCAL_FILES {
            return Ok(());
        }
        let bytes = fs::read(entry.path()).map_err(|error| {
            CommandError::filesystem(format!("cannot read a Skill file: {error}"))
        })?;
        files.push(PreparedFile {
            path: path.replace('\\', "/"),
            mode: local_mode(&entry),
            bytes,
        });
    }
    Ok(())
}

#[cfg(unix)]
fn local_mode(entry: &fs::DirEntry) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    entry.metadata().map_or(0o644, |data| {
        if data.permissions().mode() & 0o111 == 0 {
            0o644
        } else {
            0o755
        }
    })
}

#[cfg(not(unix))]
fn local_mode(_entry: &fs::DirEntry) -> u32 {
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

/// Read one line describing a file, from the file itself.
///
/// The Skill author never writes this line, so it cannot drift from the content
/// the way a hand-written manifest entry does.
fn summarize(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    frontmatter_description(text)
        .or_else(|| first_heading(text))
        .or_else(|| first_prose_line(text))
        .map(|line| truncate(&sanitize(line), SUMMARY_WIDTH))
}

fn frontmatter_description(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---\n")?;
    let body = rest.split("\n---").next()?;
    body.lines()
        .find_map(|line| line.strip_prefix("description:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn first_heading(text: &str) -> Option<&str> {
    text.lines().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed
            .starts_with('#')
            .then(|| trimmed.trim_start_matches('#').trim())
            .filter(|value| !value.is_empty())
    })
}

fn first_prose_line(text: &str) -> Option<&str> {
    text.lines()
        .map(|line| line.trim_matches(|c: char| c.is_whitespace() || c == '#' || c == '/'))
        .find(|line| !line.is_empty())
}

/// Strip anything that could move the cursor or forge a line in our own output.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned()
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let kept = value
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    format!("{}…", kept.trim_end())
}
