use std::fmt;

use serde::{Deserialize, Serialize};

use crate::DomainError;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentTargetId {
    ClaudeCode,
    Cursor,
    Windsurf,
    Cline,
    Codex,
    GithubCopilot,
    GeminiCli,
    Goose,
    Amp,
    Opencode,
    Roo,
    Antigravity,
}

impl AgentTargetId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
            Self::Windsurf => "windsurf",
            Self::Cline => "cline",
            Self::Codex => "codex",
            Self::GithubCopilot => "github-copilot",
            Self::GeminiCli => "gemini-cli",
            Self::Goose => "goose",
            Self::Amp => "amp",
            Self::Opencode => "opencode",
            Self::Roo => "roo",
            Self::Antigravity => "antigravity",
        }
    }

    pub fn parse(value: &str) -> Result<Self, DomainError> {
        AGENT_TARGETS
            .iter()
            .find(|target| target.id.as_str() == value)
            .map(|target| target.id)
            .ok_or_else(|| DomainError::InvalidTarget(value.to_owned()))
    }
}

impl fmt::Display for AgentTargetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalTargetPath {
    Home(&'static str),
    ConfigHome(&'static str),
    ClaudeHome(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTarget {
    pub id: AgentTargetId,
    pub display_name: &'static str,
    pub project_skills_dir: &'static str,
    pub global_skills_dir: GlobalTargetPath,
}

pub const AGENT_TARGETS: [AgentTarget; 12] = [
    AgentTarget {
        id: AgentTargetId::ClaudeCode,
        display_name: "Claude Code",
        project_skills_dir: ".claude/skills",
        global_skills_dir: GlobalTargetPath::ClaudeHome("skills"),
    },
    AgentTarget {
        id: AgentTargetId::Cursor,
        display_name: "Cursor",
        project_skills_dir: ".cursor/skills",
        global_skills_dir: GlobalTargetPath::Home(".cursor/skills"),
    },
    AgentTarget {
        id: AgentTargetId::Windsurf,
        display_name: "Windsurf",
        project_skills_dir: ".windsurf/skills",
        global_skills_dir: GlobalTargetPath::Home(".codeium/windsurf/skills"),
    },
    AgentTarget {
        id: AgentTargetId::Cline,
        display_name: "Cline",
        project_skills_dir: ".cline/skills",
        global_skills_dir: GlobalTargetPath::Home(".cline/skills"),
    },
    AgentTarget {
        id: AgentTargetId::Codex,
        display_name: "Codex",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
    },
    AgentTarget {
        id: AgentTargetId::GithubCopilot,
        display_name: "GitHub Copilot",
        project_skills_dir: ".github/skills",
        global_skills_dir: GlobalTargetPath::Home(".copilot/skills"),
    },
    AgentTarget {
        id: AgentTargetId::GeminiCli,
        display_name: "Gemini CLI",
        project_skills_dir: ".gemini/skills",
        global_skills_dir: GlobalTargetPath::Home(".gemini/skills"),
    },
    AgentTarget {
        id: AgentTargetId::Goose,
        display_name: "Goose",
        project_skills_dir: ".goose/skills",
        global_skills_dir: GlobalTargetPath::ConfigHome("goose/skills"),
    },
    AgentTarget {
        id: AgentTargetId::Amp,
        display_name: "Amp",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::ConfigHome("agents/skills"),
    },
    AgentTarget {
        id: AgentTargetId::Opencode,
        display_name: "OpenCode",
        project_skills_dir: ".opencode/skills",
        global_skills_dir: GlobalTargetPath::ConfigHome("opencode/skills"),
    },
    AgentTarget {
        id: AgentTargetId::Roo,
        display_name: "Roo Code",
        project_skills_dir: ".roo/skills",
        global_skills_dir: GlobalTargetPath::Home(".roo/skills"),
    },
    AgentTarget {
        id: AgentTargetId::Antigravity,
        display_name: "Antigravity",
        project_skills_dir: ".agent/skills",
        global_skills_dir: GlobalTargetPath::Home(".gemini/antigravity/skills"),
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetSelection {
    Explicit(Vec<AgentTargetId>),
    Detected(Vec<AgentTargetId>),
    Configured(Vec<AgentTargetId>),
}

impl TargetSelection {
    pub fn into_targets(self) -> Vec<AgentTargetId> {
        match self {
            Self::Explicit(targets) | Self::Detected(targets) | Self::Configured(targets) => {
                targets
            }
        }
    }
}

pub fn select_target_ids(
    explicit: &[AgentTargetId],
    detected: &[AgentTargetId],
    configured: &[AgentTargetId],
) -> Result<TargetSelection, DomainError> {
    if !explicit.is_empty() {
        return Ok(TargetSelection::Explicit(deduplicate(explicit)));
    }
    if !detected.is_empty() {
        return Ok(TargetSelection::Detected(deduplicate(detected)));
    }
    if !configured.is_empty() {
        return Ok(TargetSelection::Configured(deduplicate(configured)));
    }
    Err(DomainError::TargetRequired)
}

fn deduplicate(targets: &[AgentTargetId]) -> Vec<AgentTargetId> {
    let mut result = Vec::new();
    for target in targets {
        if !result.contains(target) {
            result.push(*target);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TargetFixture {
        id: AgentTargetId,
        display_name: String,
        project_skills_dir: String,
        global_skills_dir: String,
    }

    #[test]
    fn registry_matches_the_language_neutral_fixture() {
        let fixture: Vec<TargetFixture> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/v3-rust/agent-targets.json"
        ))
        .unwrap();

        let actual = AGENT_TARGETS
            .iter()
            .map(|target| TargetFixture {
                id: target.id,
                display_name: target.display_name.to_owned(),
                project_skills_dir: target.project_skills_dir.to_owned(),
                global_skills_dir: match target.global_skills_dir {
                    GlobalTargetPath::Home(path) => format!("home:{path}"),
                    GlobalTargetPath::ConfigHome(path) => format!("config:{path}"),
                    GlobalTargetPath::ClaudeHome(path) => format!("claude:{path}"),
                },
            })
            .collect::<Vec<_>>();

        assert_eq!(actual.len(), fixture.len());
        for (actual, expected) in actual.iter().zip(fixture.iter()) {
            assert_eq!(actual.id, expected.id);
            assert_eq!(actual.display_name, expected.display_name);
            assert_eq!(actual.project_skills_dir, expected.project_skills_dir);
            assert_eq!(actual.global_skills_dir, expected.global_skills_dir);
        }
    }
}
