//! Read-only discovery. Paths remain observations until an action rechecks them.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;
use skilld_core::{AGENT_TARGETS, AgentTargetId, SkillName};

use crate::doctor_metadata::{Fingerprint, ForeignRecord, fingerprint, read_foreign_lock};
use crate::{CommandError, ResolvedTarget};

/// Pruned before reading children. Hidden Agent target directories stay visible.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "**/node_modules/**",
    "**/.git/**",
    "**/.hg/**",
    "**/.svn/**",
    "**/target/**",
    "**/dist/**",
    "**/.nuxt/**",
    "**/.output/**",
    "**/.next/**",
    "**/coverage/**",
    "**/__pycache__/**",
    "**/.venv/**",
    "**/.cache/**",
    "**/.npm/**",
    "**/.bun/**",
    "**/.pnpm-store/**",
    "**/.local/share/pnpm/**",
    "**/.cargo/registry/**",
    "**/.rustup/**",
    "**/.codex/.tmp/**",
    "**/.codex/sessions/**",
    "**/.codex/archived_sessions/**",
    "**/.claude/projects/**",
    "**/.claude/debug/**",
    "**/.claude/session-env/**",
    "**/.claude/plugins/cache/**",
    "**/.codex/plugins/cache/**",
    "**/.claude/plugins/marketplaces/**",
    "**/.codex/plugins/marketplaces/**",
    "**/.skilld/repos/**",
    "**/.skilld/references/**",
    "**/.skilld/llm-cache/**",
    "**/.local/share/harlan-github-agent/**",
    "**/.local/share/Trash/**",
    "**/scratch/**",
    "**/backups/**",
    "**/Backups/**",
    "**/*-backup*/**",
    "**/.skilld-doctor-backups/**",
    "**/.data/content/**",
    "**/data/trees/**",
    "**/.steam/**",
    "**/.local/share/Steam/**",
    "**/.mozilla/**",
    "**/.config/google-chrome/**",
    "**/.ssh/**",
    "**/.gnupg/**",
    "**/.dev-browser/**",
    "**/.artifacts/**",
    "**/test/fixtures/**",
    "**/tests/fixtures/**",
    "**/evals/**",
    "**/evals-opencode/**",
    "**/runtime/mysql/**",
    "**/.config/chromium/**",
    "**/.local/share/containers/**",
    "**/.local/share/uv/**",
    "**/go/pkg/mod/**",
    "**/.gradle/caches/**",
    "**/.docker/**",
    "**/.yarn/**",
    "**/.intellijPlatform/**",
    "**/.unlighthouse/**",
    "**/kv-dump/**",
    "**/*.trickplay/**",
    "**/google-cloud-sdk/**",
    "**/.gemini/tmp/**",
    "**/.gemini/history/**",
    "**/.gemini/antigravity-browser-profile/**",
    "**/.config/*/User/History/**",
    "**/.config/google-chrome-*/**",
    "**/.config/cef_user_data/**",
    "**/.config/JetBrains/**/chrome-user-data/**",
    "**/.config/JetBrains/**/jdbc-drivers/**",
    "**/.config/JetBrains/**/javascript/nodejs/**",
];

#[derive(Clone, Debug, Args)]
pub struct DoctorOptions {
    /// Scan these roots. The default is your home directory.
    #[arg(value_name = "ROOT")]
    pub roots: Vec<PathBuf>,
    /// Prune paths matching this glob. Repeat for several patterns.
    #[arg(long, value_name = "GLOB")]
    pub exclude: Vec<String>,
    /// Include dependency, cache, staging, and backup directories.
    #[arg(long)]
    pub include_excluded: bool,
    /// Include Git worktrees found beneath scan roots.
    #[arg(long)]
    pub include_worktrees: bool,
    /// Stop descending after this many directories.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=64))]
    pub max_depth: u16,
    /// Check source candidates through skilld.dev. Limited to 20 Skills per scan.
    #[arg(long)]
    pub check_sources: bool,
}

impl DoctorOptions {
    pub fn for_root(root: &Path) -> Self {
        Self {
            roots: vec![root.to_path_buf()],
            exclude: vec![],
            include_excluded: false,
            include_worktrees: false,
            max_depth: 20,
            check_sources: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScanContext {
    pub home: PathBuf,
    pub global_store: PathBuf,
    pub global_targets: Vec<ResolvedTarget>,
    pub skills_sh_global_lock: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanPhase {
    Discover,
    Fingerprint { completed: usize, total: usize },
    Sources { completed: usize, total: usize },
}

#[derive(Clone, Debug)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub current_path: PathBuf,
    pub visited_directories: usize,
    pub skipped_directories: usize,
    pub found_skills: usize,
    pub problems: usize,
}

impl ScanProgress {
    pub fn from_report(
        report: &DoctorReport,
        phase: ScanPhase,
        current_path: PathBuf,
        found_skills: usize,
    ) -> Self {
        Self {
            phase,
            current_path,
            visited_directories: report.visited_directories,
            skipped_directories: report.skipped_directories,
            found_skills,
            problems: report.problems.len(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLocation {
    pub path: PathBuf,
    pub agent: Option<AgentTargetId>,
    /// None means global scope. Source directories have no Agent target.
    pub project_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "_tag",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DoctorOwner {
    SkillsSh { record: ForeignRecord },
    Skilld { store: PathBuf },
    Plugin,
    Source,
    Unknown,
    Unavailable { message: String },
}

impl DoctorOwner {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SkillsSh { .. } => "skills.sh",
            Self::Skilld { .. } => "skilld",
            Self::Plugin => "Plugin",
            Self::Source => "Source",
            Self::Unknown => "Unknown",
            Self::Unavailable { .. } => "Unavailable",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMatch {
    pub source: String,
    pub commit: Option<String>,
    pub result: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSkill {
    pub name: String,
    pub canonical_path: PathBuf,
    pub paths: Vec<SkillLocation>,
    pub owner: DoctorOwner,
    pub fingerprint: Option<Fingerprint>,
    pub duplicates: Vec<PathBuf>,
    pub source_match: Option<SourceMatch>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanProblem {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub roots: Vec<PathBuf>,
    pub excludes: Vec<String>,
    pub skills: Vec<DoctorSkill>,
    pub claude_files: Vec<PathBuf>,
    pub problems: Vec<ScanProblem>,
    pub visited_directories: usize,
    pub skipped_directories: usize,
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> CommandError {
    CommandError::filesystem(format!("Cannot read {}: {error}", path.display()))
}

fn excludes(options: &DoctorOptions) -> Result<(Vec<String>, GlobSet), CommandError> {
    let mut patterns = if options.include_excluded {
        vec!["**/.git/**".into()]
    } else {
        DEFAULT_EXCLUDES.iter().map(|s| (*s).to_owned()).collect()
    };
    patterns.extend(options.exclude.clone());
    let mut builder = GlobSetBuilder::new();
    for pattern in &patterns {
        builder.add(
            Glob::new(pattern).map_err(|e| {
                CommandError::config(format!("Invalid exclude glob {pattern}: {e}"))
            })?,
        );
        // A trailing /** must prune the directory itself, before readdir.
        if let Some(parent) = pattern.strip_suffix("/**") {
            builder.add(Glob::new(parent).map_err(|e| CommandError::config(e.to_string()))?);
        }
    }
    Ok((
        patterns,
        builder
            .build()
            .map_err(|e| CommandError::config(e.to_string()))?,
    ))
}

fn location(path: &Path, context: &ScanContext) -> SkillLocation {
    let parent = path.parent().unwrap_or(path);
    if let Some(target) = context.global_targets.iter().find(|t| t.root == parent) {
        return SkillLocation {
            path: path.to_owned(),
            agent: Some(target.agent),
            project_root: None,
        };
    }
    if parent == context.home.join(".codex/skills") {
        return SkillLocation {
            path: path.to_owned(),
            agent: Some(AgentTargetId::Codex),
            project_root: None,
        };
    }
    for target in AGENT_TARGETS {
        // Unprefixed source directories are not evidence of an Agent installation.
        if !target.project_skills_dir.starts_with('.') {
            continue;
        }
        if parent.ends_with(target.project_skills_dir) {
            let mut root = parent;
            for _ in Path::new(target.project_skills_dir).components() {
                root = root.parent().unwrap_or(root);
            }
            return SkillLocation {
                path: path.to_owned(),
                agent: Some(target.id),
                project_root: Some(root.to_owned()),
            };
        }
    }
    SkillLocation {
        path: path.to_owned(),
        agent: None,
        project_root: None,
    }
}

fn target_root(path: &Path, context: &ScanContext) -> bool {
    context.global_targets.iter().any(|t| t.root == path)
        || path == context.home.join(".codex/skills")
        || AGENT_TARGETS
            .iter()
            .any(|t| t.project_skills_dir.starts_with('.') && path.ends_with(t.project_skills_dir))
}

pub fn scan(options: &DoctorOptions, context: &ScanContext) -> Result<DoctorReport, CommandError> {
    scan_with_progress(options, context, &mut |_| {})
}

pub fn scan_with_progress(
    options: &DoctorOptions,
    context: &ScanContext,
    progress: &mut impl FnMut(ScanProgress),
) -> Result<DoctorReport, CommandError> {
    let (patterns, excluded) = excludes(options)?;
    let home_boundary = if options.roots.is_empty() {
        Some(fs::canonicalize(&context.home).map_err(|e| io_error(&context.home, e))?)
    } else {
        None
    };
    let roots = if options.roots.is_empty() {
        vec![context.home.clone()]
    } else {
        options.roots.clone()
    };
    let roots = roots
        .into_iter()
        .map(|p| {
            fs::metadata(&p).map_err(|e| io_error(&p, e))?;
            std::path::absolute(&p).map_err(|e| io_error(&p, e))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut report = DoctorReport {
        roots: roots.clone(),
        excludes: patterns,
        skills: vec![],
        claude_files: vec![],
        problems: vec![],
        visited_directories: 0,
        skipped_directories: 0,
    };
    let mut pending = roots.iter().map(|p| (p.clone(), 0)).collect::<Vec<_>>();
    progress(ScanProgress::from_report(
        &report,
        ScanPhase::Discover,
        roots[0].clone(),
        0,
    ));
    let mut visited = BTreeSet::new();
    let mut found = BTreeSet::new();
    while let Some((path, depth)) = pending.pop() {
        if excluded.is_match(&path) {
            report.skipped_directories += 1;
            continue;
        }
        if !options.include_worktrees && depth > 0 && path.join(".git").is_file() {
            report.skipped_directories += 1;
            continue;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        if report.visited_directories >= 250_000 || found.len() >= 5_000 {
            report.problems.push(ScanProblem {
                path,
                message: "Scan limit reached. Choose narrower roots.".into(),
            });
            break;
        }
        report.visited_directories += 1;
        if report.visited_directories.is_multiple_of(128) {
            progress(ScanProgress::from_report(
                &report,
                ScanPhase::Discover,
                path.clone(),
                found.len(),
            ));
        }
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) => {
                report.problems.push(ScanProblem {
                    path,
                    message: e.to_string(),
                });
                continue;
            }
        };
        let mut is_skill = false;
        let mut children = vec![];
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    report.problems.push(ScanProblem {
                        path: path.clone(),
                        message: e.to_string(),
                    });
                    continue;
                }
            };
            let child = entry.path();
            if excluded.is_match(&child) {
                report.skipped_directories += 1;
                continue;
            }
            let kind = match entry.file_type() {
                Ok(t) => t,
                Err(e) => {
                    report.problems.push(ScanProblem {
                        path: child,
                        message: e.to_string(),
                    });
                    continue;
                }
            };
            if kind.is_symlink()
                && let Some(home) = &home_boundary
                && let Ok(real) = fs::canonicalize(&child)
                && !real.starts_with(home)
            {
                report.skipped_directories += 1;
                if target_root(&path, context) || target_root(&child, context) {
                    report.problems.push(ScanProblem { path: child, message: "Link points outside the home directory. Pass its root explicitly to scan it.".into() });
                }
                continue;
            }
            if entry.file_name() == "SKILL.md" && (kind.is_file() || kind.is_symlink()) {
                is_skill = true;
            }
            if kind.is_file()
                && (entry.file_name() == "CLAUDE.md"
                    || (path.ends_with(".claude/commands")
                        && child.extension().is_some_and(|e| e == "md")))
            {
                report.claude_files.push(child.clone());
            }
            if kind.is_dir() {
                children.push(child);
            } else if kind.is_symlink() && target_root(&path, context) {
                match fs::canonicalize(&child) {
                    Ok(real) if real.join("SKILL.md").is_file() => {
                        found.insert(child);
                    }
                    Ok(_) => {}
                    Err(e) => report.problems.push(ScanProblem {
                        path: child,
                        message: format!("Broken Agent target: {e}"),
                    }),
                }
            } else if kind.is_symlink() && target_root(&child, context) {
                // A target root can itself be a symlink. Only inspect this known boundary.
                match fs::read_dir(&child) {
                    Ok(entries) => {
                        for e in entries {
                            match e {
                                Ok(e) if !excluded.is_match(e.path()) => {
                                    if let Some(home) = &home_boundary
                                        && let Ok(real) = fs::canonicalize(e.path())
                                        && !real.starts_with(home)
                                    {
                                        report.skipped_directories += 1;
                                        report.problems.push(ScanProblem { path: e.path(), message: "Link points outside the home directory. Pass its root explicitly to scan it.".into() });
                                        continue;
                                    }
                                    if e.path().join("SKILL.md").is_file() {
                                        found.insert(e.path());
                                    }
                                }
                                Ok(_) => {}
                                Err(e) => report.problems.push(ScanProblem {
                                    path: child.clone(),
                                    message: e.to_string(),
                                }),
                            }
                        }
                    }
                    Err(e) => report.problems.push(ScanProblem {
                        path: child,
                        message: e.to_string(),
                    }),
                }
            }
        }
        if is_skill {
            found.insert(path.clone());
            // A repository can ship SKILL.md at its root and still contain Agent targets.
            if !path.join(".git").exists() && !path.join("package.json").is_file() {
                continue;
            }
        }
        if depth == options.max_depth {
            if !children.is_empty() {
                report.problems.push(ScanProblem {
                    path,
                    message: "Maximum scan depth reached.".into(),
                });
            }
        } else {
            pending.extend(children.into_iter().map(|p| (p, depth + 1)));
        }
    }
    let mut by_canonical: BTreeMap<PathBuf, Vec<SkillLocation>> = BTreeMap::new();
    for path in found {
        match fs::canonicalize(&path) {
            Ok(canonical)
                if home_boundary
                    .as_ref()
                    .is_some_and(|home| !canonical.starts_with(home)) =>
            {
                report.skipped_directories += 1;
                report.problems.push(ScanProblem { path, message: "Link points outside the home directory. Pass its root explicitly to scan it.".into() });
            }
            Ok(canonical) => by_canonical
                .entry(canonical)
                .or_default()
                .push(location(&path, context)),
            Err(e) => report.problems.push(ScanProblem {
                path,
                message: e.to_string(),
            }),
        }
    }
    let mut locks: BTreeMap<PathBuf, Vec<ForeignRecord>> = BTreeMap::new();
    let total = by_canonical.len();
    for (canonical, mut paths) in by_canonical {
        progress(ScanProgress::from_report(
            &report,
            ScanPhase::Fingerprint {
                completed: report.skills.len(),
                total,
            },
            canonical.clone(),
            total,
        ));
        paths.sort_by(|a, b| a.path.cmp(&b.path));
        let name = paths
            .iter()
            .find(|p| p.agent.is_some())
            .unwrap_or(&paths[0])
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if SkillName::parse(name.clone()).is_err() {
            report.problems.push(ScanProblem {
                path: canonical.clone(),
                message: "Invalid Skill directory name.".into(),
            });
        }
        let owner = owner(
            &canonical,
            &name,
            &paths,
            context,
            home_boundary.as_deref(),
            &mut locks,
            &mut report.problems,
        );
        let fingerprint = if canonical.join(".git").exists() && matches!(owner, DoctorOwner::Source)
        {
            None
        } else {
            match fingerprint(&canonical) {
                Ok(f) => Some(f),
                Err(e) => {
                    report.problems.push(ScanProblem {
                        path: canonical.clone(),
                        message: e.to_string(),
                    });
                    None
                }
            }
        };
        report.skills.push(DoctorSkill {
            name,
            canonical_path: canonical,
            paths,
            owner,
            fingerprint,
            duplicates: vec![],
            source_match: None,
        });
    }
    let mut hashes: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for skill in &report.skills {
        if let Some(f) = &skill.fingerprint {
            hashes
                .entry(f.sha256.clone())
                .or_default()
                .push(skill.canonical_path.clone());
        }
    }
    for skill in &mut report.skills {
        if let Some(f) = &skill.fingerprint {
            skill.duplicates = hashes[&f.sha256]
                .iter()
                .filter(|p| **p != skill.canonical_path)
                .cloned()
                .collect();
        }
    }
    report.skills.sort_by(|a, b| {
        a.owner
            .label()
            .cmp(b.owner.label())
            .then(a.name.cmp(&b.name))
            .then(a.canonical_path.cmp(&b.canonical_path))
    });
    report.claude_files.sort();
    report.claude_files.dedup();
    progress(ScanProgress::from_report(
        &report,
        ScanPhase::Fingerprint {
            completed: total,
            total,
        },
        roots[0].clone(),
        total,
    ));
    Ok(report)
}

fn owner(
    canonical: &Path,
    name: &str,
    paths: &[SkillLocation],
    context: &ScanContext,
    home_boundary: Option<&Path>,
    locks: &mut BTreeMap<PathBuf, Vec<ForeignRecord>>,
    problems: &mut Vec<ScanProblem>,
) -> DoctorOwner {
    if canonical.components().any(|p| p.as_os_str() == "plugins")
        || canonical.starts_with(context.home.join(".codex/skills/.system"))
    {
        return DoctorOwner::Plugin;
    }
    let stores = paths
        .iter()
        .filter(|p| p.agent.is_some() && p.project_root.is_none())
        .map(|_| context.global_store.clone())
        .chain(
            paths
                .iter()
                .filter_map(|p| p.project_root.as_ref().map(|r| r.join(".skills"))),
        )
        .chain(canonical.ancestors().take(2).map(Path::to_path_buf))
        .collect::<BTreeSet<_>>();
    for store in stores {
        let lock = store.join("skilld-lock.yaml");
        if let Some(owner) = outside_metadata(&lock, context, home_boundary, problems) {
            return owner;
        }
        if !lock.is_file() {
            continue;
        }
        if store.join(name) == canonical {
            return DoctorOwner::Skilld { store };
        }
        match fs::read(&lock).map_err(|e| e.to_string()).and_then(|b| {
            serde_json::from_slice::<serde_json::Value>(&b).map_err(|e| e.to_string())
        }) {
            Ok(v) if v["skills"].get(name).is_some() => return DoctorOwner::Skilld { store },
            Ok(_) => {}
            Err(message) => {
                problems.push(ScanProblem {
                    path: lock,
                    message: message.clone(),
                });
                return DoctorOwner::Unavailable { message };
            }
        }
    }
    for location in paths.iter().filter(|p| p.agent.is_some()) {
        let global = location.project_root.is_none();
        let lock = location.project_root.as_ref().map_or_else(
            || context.skills_sh_global_lock.clone(),
            |r| r.join("skills-lock.json"),
        );
        if let Some(owner) = outside_metadata(&lock, context, home_boundary, problems) {
            return owner;
        }
        if !locks.contains_key(&lock) {
            let records = if fs::symlink_metadata(&lock).is_ok() {
                match read_foreign_lock(&lock, global) {
                    Ok(r) => r,
                    Err(e) => {
                        let message = e.to_string();
                        problems.push(ScanProblem {
                            path: lock.clone(),
                            message: message.clone(),
                        });
                        return DoctorOwner::Unavailable { message };
                    }
                }
            } else {
                vec![]
            };
            locks.insert(lock.clone(), records);
        }
        if let Some(record) = locks[&lock].iter().find(|r| r.name == name) {
            return DoctorOwner::SkillsSh {
                record: record.clone(),
            };
        }
    }
    if paths.iter().any(|p| p.agent.is_some()) {
        DoctorOwner::Unknown
    } else {
        DoctorOwner::Source
    }
}

fn outside_metadata(
    path: &Path,
    context: &ScanContext,
    home_boundary: Option<&Path>,
    problems: &mut Vec<ScanProblem>,
) -> Option<DoctorOwner> {
    let home = home_boundary?;
    if path.starts_with(&context.home)
        && fs::canonicalize(path).map_or(true, |real| real.starts_with(home))
    {
        return None;
    }
    let message = "Ownership metadata is outside the home directory. Pass scan roots explicitly to inspect it.".to_owned();
    problems.push(ScanProblem {
        path: path.to_owned(),
        message: message.clone(),
    });
    Some(DoctorOwner::Unavailable { message })
}

pub fn render_plain(report: &DoctorReport) -> String {
    use skilld_ui::text::sanitize;
    let mut text = format!(
        "{} Skills. {} directories scanned. {} directories excluded.\n",
        report.skills.len(),
        report.visited_directories,
        report.skipped_directories
    );
    let mut last = "";
    for skill in &report.skills {
        let label = skill.owner.label();
        if label != last {
            text.push_str(&format!("\n{label}\n"));
            last = label;
        }
        text.push_str(&format!(
            "  {}  {}  {} Agent paths{}\n",
            sanitize(&skill.name),
            sanitize(&skill.canonical_path.display().to_string()),
            skill.paths.iter().filter(|p| p.agent.is_some()).count(),
            if skill.duplicates.is_empty() {
                ""
            } else {
                "  identical copies"
            }
        ));
        if let Some(source) = &skill.source_match {
            text.push_str(&format!(
                "    {}: {}\n",
                sanitize(&source.source),
                sanitize(&source.result)
            ));
        }
    }
    text.push_str(&format!(
        "\n{} Claude instruction files. {} scan problems.\n",
        report.claude_files.len(),
        report.problems.len()
    ));
    for p in &report.problems {
        text.push_str(&format!(
            "  {}: {}\n",
            sanitize(&p.path.display().to_string()),
            sanitize(&p.message)
        ));
    }
    text
}
