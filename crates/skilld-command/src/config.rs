use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use skilld_core::{AgentTargetId, InstallMode};
use skilld_ui::Line;

use crate::CommandError;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct LocalConfig {
    pub agent_targets: Vec<AgentTargetId>,
    pub install_mode: InstallMode,
}

impl LocalConfig {
    pub fn get(&self, key: &str) -> Result<String, CommandError> {
        match key {
            "agent.targets" => Ok(self
                .agent_targets
                .iter()
                .map(|target| target.as_str())
                .collect::<Vec<_>>()
                .join(",")),
            "install.mode" => Ok(self.install_mode.as_str().to_owned()),
            _ => Err(CommandError::config(format!(
                "unknown configuration key: {key}"
            ))),
        }
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), CommandError> {
        match key {
            "agent.targets" => {
                let mut targets = Vec::new();
                for value in value.split(',').filter(|value| !value.is_empty()) {
                    let target = AgentTargetId::parse(value).map_err(CommandError::domain)?;
                    if !targets.contains(&target) {
                        targets.push(target);
                    }
                }
                self.agent_targets = targets;
                Ok(())
            }
            "install.mode" => {
                self.install_mode = InstallMode::parse(value).map_err(CommandError::domain)?;
                Ok(())
            }
            _ => Err(CommandError::config(format!(
                "unknown configuration key: {key}"
            ))),
        }
    }

    pub fn entries(&self) -> Vec<Line> {
        let targets = self
            .agent_targets
            .iter()
            .map(|target| target.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let mode = self.install_mode.as_str();
        vec![
            Line::field_plain(format!("agent.targets={targets}"), "agent.targets", targets),
            Line::field_plain(format!("install.mode={mode}"), "install.mode", mode),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(data_root: &Path) -> Self {
        Self {
            path: data_root.join("config.json"),
        }
    }

    pub fn read(&self) -> Result<LocalConfig, CommandError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LocalConfig::default());
            }
            Err(error) => return Err(CommandError::filesystem(error.to_string())),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(CommandError::config(
                "the configuration path must be a regular file",
            ));
        }
        let bytes =
            fs::read(&self.path).map_err(|error| CommandError::filesystem(error.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|error| CommandError::config(error.to_string()))
    }

    pub fn write(&self, config: &LocalConfig) -> Result<(), CommandError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| CommandError::config("configuration path has no parent"))?;
        fs::create_dir_all(parent).map_err(|error| CommandError::filesystem(error.to_string()))?;
        let temporary = parent.join(".config-stage");
        let bytes = serde_json::to_vec_pretty(config)
            .map_err(|error| CommandError::config(error.to_string()))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| CommandError::filesystem(error.to_string()))?;
        file.write_all(&bytes)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| CommandError::filesystem(error.to_string()))?;
        if self.path.exists() {
            fs::remove_file(&self.path)
                .map_err(|error| CommandError::filesystem(error.to_string()))?;
        }
        fs::rename(&temporary, &self.path)
            .map_err(|error| CommandError::filesystem(error.to_string()))
    }
}
