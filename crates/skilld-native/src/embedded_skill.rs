use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use skilld_command::{BundledSkillProvider, CommandError};
use skilld_core::PreparedFile;
use tempfile::TempDir;

const SKILLD_SKILL: &[u8] = include_bytes!("../../../skills/skilld/SKILL.md");

pub struct EmbeddedSkilld {
    directory: Mutex<Option<TempDir>>,
}

impl EmbeddedSkilld {
    pub const fn new() -> Self {
        Self {
            directory: Mutex::new(None),
        }
    }
}

impl BundledSkillProvider for EmbeddedSkilld {
    fn skilld_run_files(&self) -> Result<Vec<PreparedFile>, CommandError> {
        Ok(vec![PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes: SKILLD_SKILL.to_vec(),
        }])
    }

    fn skilld_source(&self) -> Result<PathBuf, CommandError> {
        let mut directory = self.directory.lock().map_err(|_| {
            CommandError::service("the bundled Skill workspace lock is unavailable")
        })?;
        if directory.is_none() {
            let temporary = tempfile::Builder::new()
                .prefix("skilld-bundled-")
                .tempdir()
                .map_err(|error| {
                    CommandError::filesystem(format!(
                        "cannot prepare the bundled Skill workspace: {error}"
                    ))
                })?;
            let skill = temporary.path().join("skilld");
            fs::create_dir(&skill).map_err(|error| {
                CommandError::filesystem(format!(
                    "cannot prepare the bundled Skill directory: {error}"
                ))
            })?;
            fs::write(skill.join("SKILL.md"), SKILLD_SKILL).map_err(|error| {
                CommandError::filesystem(format!("cannot prepare the bundled Skill: {error}"))
            })?;
            *directory = Some(temporary);
        }

        directory
            .as_ref()
            .map(|temporary| temporary.path().join("skilld"))
            .ok_or_else(|| CommandError::service("the bundled Skill workspace is unavailable"))
    }
}
