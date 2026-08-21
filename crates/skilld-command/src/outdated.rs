use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use skilld_core::{AgentTargetId, InstallScope, SkillName};

use crate::ResolvedTarget;
use crate::local_store::normalize_path;

pub(crate) struct UnmanagedSkill {
    pub name: String,
    pub path: PathBuf,
    pub scope: InstallScope,
    pub agents: Vec<AgentTargetId>,
}

pub(crate) struct SkillCandidate {
    pub selector: String,
    pub stargazer_count: u64,
}

pub(crate) fn scan_unmanaged(
    scan: &[(InstallScope, Vec<ResolvedTarget>)],
    store_roots: &[PathBuf],
    managed: &BTreeMap<String, Vec<PathBuf>>,
) -> Vec<UnmanagedSkill> {
    let stores = store_roots
        .iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .collect::<Vec<_>>();
    let mut roots = BTreeMap::<PathBuf, (InstallScope, Vec<AgentTargetId>)>::new();
    for (scope, targets) in scan {
        for target in targets {
            let entry = roots
                .entry(normalize_path(&target.root))
                .or_insert_with(|| (*scope, Vec::new()));
            if !entry.1.contains(&target.agent) {
                entry.1.push(target.agent);
            }
        }
    }
    let mut by_path = BTreeMap::new();
    for (root, (scope, agents)) in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if SkillName::parse(name.clone()).is_err() || !entry.path().join("SKILL.md").is_file() {
                continue;
            }
            let Some(canonical) = fs::canonicalize(entry.path()).ok() else {
                continue;
            };
            if stores.iter().any(|root| canonical.starts_with(root)) {
                continue;
            }
            let managed = managed.get(&name).is_some_and(|paths| {
                paths.iter().any(|path| {
                    normalize_path(path) == normalize_path(&entry.path())
                        || fs::canonicalize(path).is_ok_and(|resolved| resolved == canonical)
                })
            });
            if managed {
                continue;
            }
            let skill = by_path
                .entry(canonical.clone())
                .or_insert_with(|| UnmanagedSkill {
                    name: name.clone(),
                    path: canonical,
                    scope,
                    agents: Vec::new(),
                });
            for agent in &agents {
                if !skill.agents.contains(agent) {
                    skill.agents.push(*agent);
                }
            }
        }
    }
    let mut skills = by_path.into_values().collect::<Vec<_>>();
    for skill in &mut skills {
        skill.agents.sort_by_key(|agent| agent.as_str());
    }
    skills.sort_by(|left, right| left.name.cmp(&right.name).then(left.path.cmp(&right.path)));
    skills
}

pub(crate) fn render_search_failure(skill: &UnmanagedSkill, message: &str) -> String {
    format!(
        "Unmanaged Skill {} ({}). Skill search unavailable: {message}.",
        skill.name,
        agent_list(skill)
    )
}

pub(crate) fn render_unmanaged(
    skill: &UnmanagedSkill,
    candidate: Option<&SkillCandidate>,
) -> Vec<String> {
    let agents = agent_list(skill);
    let Some(candidate) = candidate else {
        return vec![format!(
            "Unmanaged Skill {} ({agents}). No Repository match found.",
            skill.name
        )];
    };
    let global = if skill.scope == InstallScope::Global {
        " --global"
    } else {
        ""
    };
    let agent_flags = agent_flags(&skill.agents);
    vec![
        format!(
            "Unmanaged Skill {} ({agents}). Candidate source {}, {} stars.",
            skill.name, candidate.selector, candidate.stargazer_count
        ),
        format!(
            "Delete {}, then run skilld install {}{global}{agent_flags}.",
            skill.path.display(),
            candidate.selector
        ),
    ]
}

pub(crate) fn agent_flags(agents: &[AgentTargetId]) -> String {
    if agents.is_empty() {
        return String::new();
    }
    let flags = agents
        .iter()
        .map(|agent| format!("--agent {}", agent.as_str()))
        .collect::<Vec<_>>()
        .join(" ");
    format!(" {flags}")
}

fn agent_list(skill: &UnmanagedSkill) -> String {
    skill
        .agents
        .iter()
        .map(|agent| agent.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
