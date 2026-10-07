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
    Openclaw,
    Hermes,
    Kiro,
    Kilo,
    Droid,
    Trae,
    Zed,
    AiderDesk,
    AntigravityCli,
    Astrbot,
    Augment,
    Bob,
    CodeartsAgent,
    Codebuddy,
    Codemaker,
    Codestudio,
    CommandCode,
    Continue,
    Cortex,
    Crush,
    Deepagents,
    Devin,
    Dexto,
    Firebender,
    Forgecode,
    Fx,
    InferenceSh,
    Jazz,
    Junie,
    IflowCli,
    Kimchi,
    KimiCodeCli,
    Kode,
    Lingma,
    Loaf,
    Mcpjam,
    MinimaxCode,
    Moxby,
    Mux,
    Openhands,
    Ona,
    Pi,
    PositAssistant,
    Qoder,
    QoderCn,
    QwenCode,
    Replit,
    Reasonix,
    Rovodev,
    SarvamCode,
    TabnineCli,
    Terramind,
    Tinycloud,
    TraeCn,
    Warp,
    Zcode,
    Zencoder,
    Zenflow,
    Neovate,
    Pochi,
    Adal,
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
            Self::Openclaw => "openclaw",
            Self::Hermes => "hermes",
            Self::Kiro => "kiro",
            Self::Kilo => "kilo",
            Self::Droid => "droid",
            Self::Trae => "trae",
            Self::Zed => "zed",
            Self::AiderDesk => "aider-desk",
            Self::AntigravityCli => "antigravity-cli",
            Self::Astrbot => "astrbot",
            Self::Augment => "augment",
            Self::Bob => "bob",
            Self::CodeartsAgent => "codearts-agent",
            Self::Codebuddy => "codebuddy",
            Self::Codemaker => "codemaker",
            Self::Codestudio => "codestudio",
            Self::CommandCode => "command-code",
            Self::Continue => "continue",
            Self::Cortex => "cortex",
            Self::Crush => "crush",
            Self::Deepagents => "deepagents",
            Self::Devin => "devin",
            Self::Dexto => "dexto",
            Self::Firebender => "firebender",
            Self::Forgecode => "forgecode",
            Self::Fx => "fx",
            Self::InferenceSh => "inference-sh",
            Self::Jazz => "jazz",
            Self::Junie => "junie",
            Self::IflowCli => "iflow-cli",
            Self::Kimchi => "kimchi",
            Self::KimiCodeCli => "kimi-code-cli",
            Self::Kode => "kode",
            Self::Lingma => "lingma",
            Self::Loaf => "loaf",
            Self::Mcpjam => "mcpjam",
            Self::MinimaxCode => "minimax-code",
            Self::Moxby => "moxby",
            Self::Mux => "mux",
            Self::Openhands => "openhands",
            Self::Ona => "ona",
            Self::Pi => "pi",
            Self::PositAssistant => "posit-assistant",
            Self::Qoder => "qoder",
            Self::QoderCn => "qoder-cn",
            Self::QwenCode => "qwen-code",
            Self::Replit => "replit",
            Self::Reasonix => "reasonix",
            Self::Rovodev => "rovodev",
            Self::SarvamCode => "sarvam-code",
            Self::TabnineCli => "tabnine-cli",
            Self::Terramind => "terramind",
            Self::Tinycloud => "tinycloud",
            Self::TraeCn => "trae-cn",
            Self::Warp => "warp",
            Self::Zcode => "zcode",
            Self::Zencoder => "zencoder",
            Self::Zenflow => "zenflow",
            Self::Neovate => "neovate",
            Self::Pochi => "pochi",
            Self::Adal => "adal",
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
    OpenclawHome(&'static str),
    HermesHome(&'static str),
    KiroHome(&'static str),
}

/// How skilld may select an Agent target when the user names none.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetDetection {
    /// skilld selects the target when it finds the Agent's directories,
    /// environment, or installation.
    Detected,
    /// Only `--agent` or `agent.targets` selects the target. It shares a
    /// skills directory with an earlier target, so detecting it would add a
    /// second target for a directory that skilld already writes.
    ExplicitOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTarget {
    pub id: AgentTargetId,
    pub display_name: &'static str,
    pub project_skills_dir: &'static str,
    pub global_skills_dir: GlobalTargetPath,
    pub detection: TargetDetection,
}

impl AgentTarget {
    /// A bare project directory such as `skills` is a common first-party
    /// path, so its existence alone must not select this target.
    pub fn auto_detects_project_dir(&self) -> bool {
        self.project_skills_dir.starts_with('.')
    }

    pub fn is_detected(&self) -> bool {
        self.detection == TargetDetection::Detected
    }
}

pub const AGENT_TARGETS: [AgentTarget; 73] = [
    AgentTarget {
        id: AgentTargetId::ClaudeCode,
        display_name: "Claude Code",
        project_skills_dir: ".claude/skills",
        global_skills_dir: GlobalTargetPath::ClaudeHome("skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Cursor,
        display_name: "Cursor",
        project_skills_dir: ".cursor/skills",
        global_skills_dir: GlobalTargetPath::Home(".cursor/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Windsurf,
        display_name: "Windsurf",
        project_skills_dir: ".windsurf/skills",
        global_skills_dir: GlobalTargetPath::Home(".codeium/windsurf/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Cline,
        display_name: "Cline",
        project_skills_dir: ".cline/skills",
        global_skills_dir: GlobalTargetPath::Home(".cline/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Codex,
        display_name: "Codex",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::GithubCopilot,
        display_name: "GitHub Copilot",
        project_skills_dir: ".github/skills",
        global_skills_dir: GlobalTargetPath::Home(".copilot/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::GeminiCli,
        display_name: "Gemini CLI",
        project_skills_dir: ".gemini/skills",
        global_skills_dir: GlobalTargetPath::Home(".gemini/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Goose,
        display_name: "Goose",
        project_skills_dir: ".goose/skills",
        global_skills_dir: GlobalTargetPath::ConfigHome("goose/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Amp,
        display_name: "Amp",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::ConfigHome("agents/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Opencode,
        display_name: "OpenCode",
        project_skills_dir: ".opencode/skills",
        global_skills_dir: GlobalTargetPath::ConfigHome("opencode/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Roo,
        display_name: "Roo Code",
        project_skills_dir: ".roo/skills",
        global_skills_dir: GlobalTargetPath::Home(".roo/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Antigravity,
        display_name: "Antigravity",
        project_skills_dir: ".agent/skills",
        global_skills_dir: GlobalTargetPath::Home(".gemini/antigravity/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Openclaw,
        display_name: "OpenClaw",
        project_skills_dir: "skills",
        global_skills_dir: GlobalTargetPath::OpenclawHome("skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Hermes,
        display_name: "Hermes Agent",
        project_skills_dir: ".hermes/skills",
        global_skills_dir: GlobalTargetPath::HermesHome("skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Kiro,
        display_name: "Kiro CLI",
        project_skills_dir: ".kiro/skills",
        global_skills_dir: GlobalTargetPath::KiroHome("skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Kilo,
        display_name: "Kilo Code",
        project_skills_dir: ".kilo/skills",
        global_skills_dir: GlobalTargetPath::Home(".kilo/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Droid,
        display_name: "Droid",
        project_skills_dir: ".factory/skills",
        global_skills_dir: GlobalTargetPath::Home(".factory/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Trae,
        display_name: "Trae",
        project_skills_dir: ".trae/skills",
        global_skills_dir: GlobalTargetPath::Home(".trae/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Zed,
        display_name: "Zed",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
        detection: TargetDetection::Detected,
    },
    // The targets below take their ids, names, and paths from
    // vercel-labs/skills src/agents.ts (MIT licence) at commit
    // 958f4b7389ba698b0a6a26a1e505ae2af82364d2.
    AgentTarget {
        id: AgentTargetId::AiderDesk,
        display_name: "AiderDesk",
        project_skills_dir: ".aider-desk/skills",
        global_skills_dir: GlobalTargetPath::Home(".aider-desk/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::AntigravityCli,
        display_name: "Antigravity CLI",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".gemini/antigravity-cli/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Astrbot,
        display_name: "AstrBot",
        project_skills_dir: "data/skills",
        global_skills_dir: GlobalTargetPath::Home(".astrbot/data/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Augment,
        display_name: "Augment",
        project_skills_dir: ".augment/skills",
        global_skills_dir: GlobalTargetPath::Home(".augment/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Bob,
        display_name: "IBM Bob",
        project_skills_dir: ".bob/skills",
        global_skills_dir: GlobalTargetPath::Home(".bob/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::CodeartsAgent,
        display_name: "CodeArts Agent",
        project_skills_dir: ".codeartsdoer/skills",
        global_skills_dir: GlobalTargetPath::Home(".codeartsdoer/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Codebuddy,
        display_name: "CodeBuddy",
        project_skills_dir: ".codebuddy/skills",
        global_skills_dir: GlobalTargetPath::Home(".codebuddy/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Codemaker,
        display_name: "Codemaker",
        project_skills_dir: ".codemaker/skills",
        global_skills_dir: GlobalTargetPath::Home(".codemaker/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Codestudio,
        display_name: "Code Studio",
        project_skills_dir: ".codestudio/skills",
        global_skills_dir: GlobalTargetPath::Home(".codestudio/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::CommandCode,
        display_name: "Command Code",
        project_skills_dir: ".commandcode/skills",
        global_skills_dir: GlobalTargetPath::Home(".commandcode/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Continue,
        display_name: "Continue",
        project_skills_dir: ".continue/skills",
        global_skills_dir: GlobalTargetPath::Home(".continue/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Cortex,
        display_name: "Cortex Code",
        project_skills_dir: ".cortex/skills",
        global_skills_dir: GlobalTargetPath::Home(".snowflake/cortex/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Crush,
        display_name: "Crush",
        project_skills_dir: ".crush/skills",
        global_skills_dir: GlobalTargetPath::Home(".config/crush/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Deepagents,
        display_name: "Deep Agents",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".deepagents/agent/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Devin,
        display_name: "Devin for Terminal",
        project_skills_dir: ".devin/skills",
        global_skills_dir: GlobalTargetPath::ConfigHome("devin/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Dexto,
        display_name: "Dexto",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Firebender,
        display_name: "Firebender",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".firebender/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Forgecode,
        display_name: "ForgeCode",
        project_skills_dir: ".forge/skills",
        global_skills_dir: GlobalTargetPath::Home(".forge/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Fx,
        display_name: "fx",
        project_skills_dir: ".fx/skills",
        global_skills_dir: GlobalTargetPath::Home(".fx/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::InferenceSh,
        display_name: "inference.sh",
        project_skills_dir: ".inferencesh/skills",
        global_skills_dir: GlobalTargetPath::Home(".inferencesh/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Jazz,
        display_name: "Jazz",
        project_skills_dir: ".jazz/skills",
        global_skills_dir: GlobalTargetPath::Home(".jazz/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Junie,
        display_name: "Junie",
        project_skills_dir: ".junie/skills",
        global_skills_dir: GlobalTargetPath::Home(".junie/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::IflowCli,
        display_name: "iFlow CLI",
        project_skills_dir: ".iflow/skills",
        global_skills_dir: GlobalTargetPath::Home(".iflow/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Kimchi,
        display_name: "Kimchi",
        project_skills_dir: ".kimchi/skills",
        global_skills_dir: GlobalTargetPath::Home(".config/kimchi/harness/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::KimiCodeCli,
        display_name: "Kimi Code CLI",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Kode,
        display_name: "Kode",
        project_skills_dir: ".kode/skills",
        global_skills_dir: GlobalTargetPath::Home(".kode/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Lingma,
        display_name: "Lingma",
        project_skills_dir: ".lingma/skills",
        global_skills_dir: GlobalTargetPath::Home(".lingma/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Loaf,
        display_name: "Loaf",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Mcpjam,
        display_name: "MCPJam",
        project_skills_dir: ".mcpjam/skills",
        global_skills_dir: GlobalTargetPath::Home(".mcpjam/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::MinimaxCode,
        display_name: "MiniMax Code",
        project_skills_dir: ".minimax/skills",
        global_skills_dir: GlobalTargetPath::Home(".minimax/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Moxby,
        display_name: "Moxby",
        project_skills_dir: ".moxby/skills",
        global_skills_dir: GlobalTargetPath::Home(".moxby/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Mux,
        display_name: "Mux",
        project_skills_dir: ".mux/skills",
        global_skills_dir: GlobalTargetPath::Home(".mux/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Openhands,
        display_name: "OpenHands",
        project_skills_dir: ".openhands/skills",
        global_skills_dir: GlobalTargetPath::Home(".openhands/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Ona,
        display_name: "Ona",
        project_skills_dir: ".ona/skills",
        global_skills_dir: GlobalTargetPath::Home(".ona/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Pi,
        display_name: "Pi",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::PositAssistant,
        display_name: "Posit Assistant",
        project_skills_dir: ".posit/assistant/skills",
        global_skills_dir: GlobalTargetPath::Home(".posit/assistant/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Qoder,
        display_name: "Qoder",
        project_skills_dir: ".qoder/skills",
        global_skills_dir: GlobalTargetPath::Home(".qoder/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::QoderCn,
        display_name: "Qoder CN",
        project_skills_dir: ".qoder/skills",
        global_skills_dir: GlobalTargetPath::Home(".qoder-cn/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::QwenCode,
        display_name: "Qwen Code",
        project_skills_dir: ".qwen/skills",
        global_skills_dir: GlobalTargetPath::Home(".qwen/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Replit,
        display_name: "Replit",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::ConfigHome("agents/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Reasonix,
        display_name: "Reasonix",
        project_skills_dir: ".reasonix/skills",
        global_skills_dir: GlobalTargetPath::Home(".reasonix/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Rovodev,
        display_name: "Rovo Dev",
        project_skills_dir: ".rovodev/skills",
        global_skills_dir: GlobalTargetPath::Home(".rovodev/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::SarvamCode,
        display_name: "Sarvam Code",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::TabnineCli,
        display_name: "Tabnine CLI",
        project_skills_dir: ".tabnine/agent/skills",
        global_skills_dir: GlobalTargetPath::Home(".tabnine/agent/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Terramind,
        display_name: "Terramind",
        project_skills_dir: ".terramind/skills",
        global_skills_dir: GlobalTargetPath::Home(".terramind/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Tinycloud,
        display_name: "Tinycloud",
        project_skills_dir: ".tinycloud/skills",
        global_skills_dir: GlobalTargetPath::Home(".tinycloud/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::TraeCn,
        display_name: "Trae CN",
        project_skills_dir: ".trae/skills",
        global_skills_dir: GlobalTargetPath::Home(".trae-cn/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Warp,
        display_name: "Warp",
        project_skills_dir: ".agents/skills",
        global_skills_dir: GlobalTargetPath::Home(".agents/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Zcode,
        display_name: "ZCode",
        project_skills_dir: ".zcode/skills",
        global_skills_dir: GlobalTargetPath::Home(".zcode/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Zencoder,
        display_name: "Zencoder",
        project_skills_dir: ".zencoder/skills",
        global_skills_dir: GlobalTargetPath::Home(".zencoder/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Zenflow,
        display_name: "Zenflow",
        project_skills_dir: ".zencoder/skills",
        global_skills_dir: GlobalTargetPath::Home(".zencoder/skills"),
        detection: TargetDetection::ExplicitOnly,
    },
    AgentTarget {
        id: AgentTargetId::Neovate,
        display_name: "Neovate",
        project_skills_dir: ".neovate/skills",
        global_skills_dir: GlobalTargetPath::Home(".neovate/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Pochi,
        display_name: "Pochi",
        project_skills_dir: ".pochi/skills",
        global_skills_dir: GlobalTargetPath::Home(".pochi/skills"),
        detection: TargetDetection::Detected,
    },
    AgentTarget {
        id: AgentTargetId::Adal,
        display_name: "AdaL",
        project_skills_dir: ".adal/skills",
        global_skills_dir: GlobalTargetPath::Home(".adal/skills"),
        detection: TargetDetection::Detected,
    },
];

/// The `--agent` value that selects every known Agent target.
pub const ALL_AGENT_TARGETS: &str = "all";

/// Parse `--agent` values. `all` expands to every known Agent target in registry order.
pub fn parse_agent_targets(values: &[String]) -> Result<Vec<AgentTargetId>, DomainError> {
    let mut parsed = Vec::with_capacity(values.len());
    let mut expand_all = false;
    for value in values {
        if value == ALL_AGENT_TARGETS {
            expand_all = true;
            continue;
        }
        parsed.push(AgentTargetId::parse(value)?);
    }
    if expand_all {
        return Ok(AGENT_TARGETS.iter().map(|target| target.id).collect());
    }
    Ok(parsed)
}

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
        detection: String,
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
                    GlobalTargetPath::OpenclawHome(path) => format!("openclaw:{path}"),
                    GlobalTargetPath::HermesHome(path) => format!("hermes:{path}"),
                    GlobalTargetPath::KiroHome(path) => format!("kiro:{path}"),
                },
                detection: match target.detection {
                    TargetDetection::Detected => "detected".to_owned(),
                    TargetDetection::ExplicitOnly => "explicit".to_owned(),
                },
            })
            .collect::<Vec<_>>();

        assert_eq!(actual.len(), fixture.len());
        for (actual, expected) in actual.iter().zip(fixture.iter()) {
            assert_eq!(actual.id, expected.id);
            assert_eq!(actual.display_name, expected.display_name);
            assert_eq!(actual.project_skills_dir, expected.project_skills_dir);
            assert_eq!(actual.global_skills_dir, expected.global_skills_dir);
            assert_eq!(actual.detection, expected.detection, "{}", actual.id);
        }
    }
}
