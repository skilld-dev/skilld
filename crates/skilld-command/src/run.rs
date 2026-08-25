//! Transient Skill loads.
//!
//! `skilld run` hands the calling Agent a Skill now. It installs nothing: no
//! lockfile entry, no Agent target write, no project file. Remote content
//! lands in a content addressed run cache so the Skill can name its own
//! supporting files by an absolute path.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use skilld_core::{PreparedFile, SkillName};

use crate::CommandError;

/// The instructions file every Skill carries.
pub const INSTRUCTIONS_FILE: &str = "SKILL.md";

const MAX_LOCAL_DEPTH: usize = 8;
const MAX_LOCAL_FILES: usize = 512;

/// One transient Skill: loaded for this session, recorded nowhere.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransientSkill {
    /// The Skill name.
    pub name: String,
    /// The full SKILL.md text.
    pub instructions: String,
    /// The directory that holds the Skill files on this machine.
    pub root: PathBuf,
    /// Supporting file paths, relative to `root`, without SKILL.md.
    pub files: Vec<String>,
    /// The source the user gave, in canonical form.
    pub source: String,
    /// `verified`, `local`, or `unverified`.
    pub source_status: &'static str,
    /// Whether the user asked for a direct GitHub fetch.
    pub direct: bool,
}

/// The cache directory for one prepared Skill.
///
/// The digest addresses the content, so an existing directory already holds
/// these exact bytes and a second run reuses it.
pub fn cache_directory(
    root: &Path,
    digest: &str,
    name: &SkillName,
) -> Result<PathBuf, CommandError> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CommandError::operation(
            "INVALID_ARTIFACT",
            "the Skill content digest is invalid",
        ));
    }
    Ok(root.join(digest).join(name.as_str()))
}

/// Read the SKILL.md text out of a prepared file set.
pub fn read_instructions(files: &[PreparedFile]) -> Result<String, CommandError> {
    let file = files
        .iter()
        .find(|file| file.path == INSTRUCTIONS_FILE)
        .ok_or_else(|| {
            CommandError::operation("INVALID_ARTIFACT", "the Skill has no SKILL.md file")
        })?;
    String::from_utf8(file.bytes.clone()).map_err(|_| {
        CommandError::operation("INVALID_ARTIFACT", "the SKILL.md file is not valid UTF-8")
    })
}

/// List the supporting files a Skill carries beside its instructions.
pub fn supporting_files(files: &[PreparedFile]) -> Vec<String> {
    files
        .iter()
        .filter(|file| file.path != INSTRUCTIONS_FILE)
        .map(|file| file.path.clone())
        .collect()
}

/// Write a prepared Skill into the run cache and answer its directory.
///
/// The write stages beside the destination and renames, so a cancelled run
/// never leaves a partial directory for the next run to trust.
pub fn write_cache(
    root: &Path,
    digest: &str,
    name: &SkillName,
    files: &[PreparedFile],
) -> Result<PathBuf, CommandError> {
    let destination = cache_directory(root, digest, name)?;
    if destination.is_dir() {
        return Ok(destination);
    }
    let entry = destination
        .parent()
        .ok_or_else(|| CommandError::filesystem("cannot resolve the run cache directory"))?
        .to_path_buf();
    let staging = entry.with_extension(format!("staging-{}", std::process::id()));
    if staging.exists() {
        remove_directory(&staging)?;
    }
    let skill = staging.join(name.as_str());
    fs::create_dir_all(&skill).map_err(cache_error)?;
    for file in files {
        write_file(&skill, file)?;
    }
    match fs::rename(&staging, &entry) {
        Ok(()) => Ok(destination),
        Err(_) if destination.is_dir() => {
            remove_directory(&staging)?;
            Ok(destination)
        }
        Err(error) => {
            remove_directory(&staging)?;
            Err(cache_error(error))
        }
    }
}

/// Read a Skill that already sits on disk.
pub fn read_local(path: &Path) -> Result<(String, String, Vec<String>), CommandError> {
    let instructions = fs::read_to_string(path.join(INSTRUCTIONS_FILE)).map_err(|error| {
        CommandError::operation(
            "SOURCE_NOT_FOUND",
            format!("cannot read {INSTRUCTIONS_FILE} in this directory: {error}"),
        )
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            CommandError::operation("INVALID_SOURCE", "the Skill directory has no usable name")
        })?
        .to_owned();
    let mut files = Vec::new();
    collect_local(path, Path::new(""), 0, &mut files)?;
    files.sort();
    Ok((name, instructions, files))
}

fn collect_local(
    root: &Path,
    relative: &Path,
    depth: usize,
    files: &mut Vec<String>,
) -> Result<(), CommandError> {
    if depth > MAX_LOCAL_DEPTH || files.len() >= MAX_LOCAL_FILES {
        return Ok(());
    }
    let entries = fs::read_dir(root.join(relative)).map_err(|error| {
        CommandError::filesystem(format!("cannot read a Skill directory: {error}"))
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
        let Some(path) = child.to_str() else { continue };
        if path == INSTRUCTIONS_FILE {
            continue;
        }
        if files.len() >= MAX_LOCAL_FILES {
            return Ok(());
        }
        files.push(path.replace('\\', "/"));
    }
    Ok(())
}

fn write_file(root: &Path, file: &PreparedFile) -> Result<(), CommandError> {
    let path = root.join(&file.path);
    let parent = path
        .parent()
        .ok_or_else(|| CommandError::filesystem("cannot resolve a cached Skill file parent"))?;
    fs::create_dir_all(parent).map_err(cache_error)?;
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(cache_error)?;
    destination.write_all(&file.bytes).map_err(cache_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(file.mode)).map_err(cache_error)?;
    }
    Ok(())
}

fn remove_directory(path: &Path) -> Result<(), CommandError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(cache_error(error)),
    }
}

fn cache_error(error: std::io::Error) -> CommandError {
    CommandError::filesystem(format!("cannot write the Skill run cache: {error}"))
}
