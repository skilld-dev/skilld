//! Skill behaviors: what a Skill's files ask an Agent to do.
//!
//! skilld matches every Skill file against the fixed rules in
//! `contracts/skill-behaviors.json`. A match names the behavior and where it
//! appears. No match proves nothing: text patterns miss obfuscated code.
//!
//! The rules are data so that skilld.dev can apply the same rules. The cases in
//! `contracts/fixtures/skill-behaviors/cases.json` fix the matching semantics
//! for every implementation.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::PreparedFile;

const RULES_JSON: &str = include_str!("../../../contracts/skill-behaviors.json");

/// The locations one behavior keeps. `total` still counts every match.
pub const MAX_BEHAVIOR_LOCATIONS: usize = 5;

/// Characters that delimit a shell token on their own.
const SELF_DELIMITING: &[char] = &['|', '&', ';', '(', ')', '<', '>', '\'', '"', '`'];

/// Whether a behavior needs the user's approval before a remote run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BehaviorTier {
    /// skilld loads nothing until the user approves the behavior.
    Ask,
    /// skilld names the behavior and loads the Skill.
    Show,
}

impl BehaviorTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Show => "show",
        }
    }
}

/// One place a behavior appears. A rule that reads no lines has no line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BehaviorLocation {
    pub path: String,
    pub line: Option<usize>,
}

impl std::fmt::Display for BehaviorLocation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "{}:{line}", self.path),
            None => formatter.write_str(&self.path),
        }
    }
}

/// One behavior a Skill's files matched.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Behavior {
    pub id: &'static str,
    pub tier: BehaviorTier,
    pub label: &'static str,
    /// The first matches, at most [`MAX_BEHAVIOR_LOCATIONS`].
    pub locations: Vec<BehaviorLocation>,
    /// Every matching line, or every matching file for a rule without lines.
    pub total: usize,
}

/// One behavior rule as `contracts/skill-behaviors.json` states it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BehaviorRule {
    pub id: String,
    pub tier: BehaviorTier,
    pub label: String,
    #[serde(default)]
    commands: Vec<Vec<String>>,
    #[serde(default)]
    substrings: Vec<String>,
    #[serde(default)]
    fences: Vec<String>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    codepoints: Vec<[String; 2]>,
    #[serde(default)]
    executable: bool,
    #[serde(default)]
    extensions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleDocument {
    version: u32,
    /// Plain-language matching notes for people. skilld reads none of them.
    #[serde(rename = "matching")]
    _matching: Vec<String>,
    behaviors: Vec<BehaviorRule>,
}

struct Rules {
    behaviors: Vec<BehaviorRule>,
    /// Inclusive ranges per behavior, parsed once from hexadecimal.
    codepoints: Vec<Vec<(u32, u32)>>,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES
        .get_or_init(|| parse_rules(RULES_JSON).expect("contracts/skill-behaviors.json must parse"))
}

fn parse_rules(source: &str) -> Result<Rules, String> {
    let document: RuleDocument = serde_json::from_str(source).map_err(|error| error.to_string())?;
    if document.version != 1 {
        return Err(format!("unknown rules version {}", document.version));
    }
    let mut ids = std::collections::BTreeSet::new();
    for rule in &document.behaviors {
        if rule.id.is_empty()
            || !rule
                .id
                .chars()
                .all(|character| character.is_ascii_lowercase() || character == '-')
        {
            return Err(format!(
                "behavior id {:?} must use lowercase letters and hyphens",
                rule.id
            ));
        }
        if !ids.insert(rule.id.as_str()) {
            return Err(format!("behavior id {} repeats", rule.id));
        }
        let patterns = rule
            .commands
            .iter()
            .flatten()
            .chain(&rule.substrings)
            .chain(&rule.fences)
            .chain(&rule.tools)
            .chain(&rule.extensions);
        for pattern in patterns {
            if pattern.is_empty()
                || pattern
                    .chars()
                    .any(|character| character.is_ascii_uppercase())
            {
                return Err(format!(
                    "behavior {} pattern {pattern:?} must be lowercase and not empty",
                    rule.id
                ));
            }
        }
    }
    let codepoints = document
        .behaviors
        .iter()
        .map(|rule| {
            rule.codepoints
                .iter()
                .map(|[start, end]| {
                    let start =
                        u32::from_str_radix(start, 16).map_err(|error| error.to_string())?;
                    let end = u32::from_str_radix(end, 16).map_err(|error| error.to_string())?;
                    Ok((start, end))
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Rules {
        behaviors: document.behaviors,
        codepoints,
    })
}

/// Every behavior rule, in the order skilld reports behaviors.
pub fn behavior_rules() -> &'static [BehaviorRule] {
    &rules().behaviors
}

/// The rule with this id, if skilld knows one.
pub fn behavior_rule(id: &str) -> Option<&'static BehaviorRule> {
    behavior_rules().iter().find(|rule| rule.id == id)
}

/// Match a Skill's files against every behavior rule.
///
/// Behaviors come back in rule order. A behavior with no match is absent.
pub fn detect_behaviors(files: &[PreparedFile]) -> Vec<Behavior> {
    let rules = rules();
    let mut found = Found::new(rules.behaviors.len());
    for file in files {
        for (index, rule) in rules.behaviors.iter().enumerate() {
            if file_matches(rule, file) {
                found.record(index, &file.path, None);
            }
        }
        let Ok(text) = std::str::from_utf8(&file.bytes) else {
            continue;
        };
        scan_text(rules, &file.path, text, &mut found);
    }
    found.into_behaviors(rules)
}

struct Found {
    locations: Vec<Vec<BehaviorLocation>>,
    totals: Vec<usize>,
}

impl Found {
    fn new(size: usize) -> Self {
        Self {
            locations: vec![Vec::new(); size],
            totals: vec![0; size],
        }
    }

    fn record(&mut self, index: usize, path: &str, line: Option<usize>) {
        self.totals[index] += 1;
        if self.locations[index].len() < MAX_BEHAVIOR_LOCATIONS {
            self.locations[index].push(BehaviorLocation {
                path: path.to_owned(),
                line,
            });
        }
    }

    fn into_behaviors(self, rules: &'static Rules) -> Vec<Behavior> {
        rules
            .behaviors
            .iter()
            .zip(self.locations)
            .zip(self.totals)
            .filter(|(_, total)| *total > 0)
            .map(|((rule, locations), total)| Behavior {
                id: &rule.id,
                tier: rule.tier,
                label: &rule.label,
                locations,
                total,
            })
            .collect()
    }
}

fn file_matches(rule: &BehaviorRule, file: &PreparedFile) -> bool {
    if rule.executable && file.mode & 0o111 != 0 {
        return true;
    }
    let path = file.path.to_ascii_lowercase();
    rule.extensions
        .iter()
        .any(|extension| path.ends_with(extension.as_str()))
}

/// What one line of a file offers the rules.
#[derive(Default)]
struct LineFacts<'a> {
    /// Code text: a fenced line, inline code spans, or a whole non-Markdown line.
    code: Vec<&'a str>,
    /// The first info string word of a fence this line opens.
    fence: Option<String>,
    /// Tool names an `allowed-tools` entry on this line declares.
    tools: Vec<String>,
}

fn scan_text(rules: &Rules, path: &str, text: &str, found: &mut Found) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let markdown = is_markdown(path);
    let frontmatter_end = if path == "SKILL.md" {
        frontmatter_end(text)
    } else {
        0
    };
    let mut fence: Option<Fence> = None;
    let mut in_tools_list = false;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let mut facts = LineFacts::default();
        if number <= frontmatter_end {
            facts.tools = frontmatter_tools(line, &mut in_tools_list);
        } else if markdown {
            markdown_line(line, &mut fence, &mut facts);
        } else {
            facts.code.push(line);
        }
        let lowered = facts
            .code
            .iter()
            .map(|code| code.to_ascii_lowercase())
            .collect::<Vec<_>>();
        for (rule_index, rule) in rules.behaviors.iter().enumerate() {
            if line_matches(rule, &rules.codepoints[rule_index], line, &lowered, &facts) {
                found.record(rule_index, path, Some(number));
            }
        }
    }
}

fn line_matches(
    rule: &BehaviorRule,
    codepoints: &[(u32, u32)],
    line: &str,
    code: &[String],
    facts: &LineFacts<'_>,
) -> bool {
    if !codepoints.is_empty()
        && line.chars().any(|character| {
            let value = u32::from(character);
            codepoints
                .iter()
                .any(|(start, end)| (*start..=*end).contains(&value))
        })
    {
        return true;
    }
    if facts
        .fence
        .as_ref()
        .is_some_and(|language| rule.fences.iter().any(|fence| fence == language))
    {
        return true;
    }
    if facts
        .tools
        .iter()
        .any(|tool| rule.tools.iter().any(|pattern| tool_matches(pattern, tool)))
    {
        return true;
    }
    code.iter().any(|segment| {
        rule.substrings
            .iter()
            .any(|substring| segment.contains(substring.as_str()))
            || rule
                .commands
                .iter()
                .any(|tokens| command_matches(segment, tokens))
    })
}

fn tool_matches(pattern: &str, tool: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => tool.starts_with(prefix),
        None => tool == pattern,
    }
}

fn is_markdown(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    [".md", ".mdx", ".markdown"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

/// The line number that closes the SKILL.md frontmatter, or 0 without one.
fn frontmatter_end(text: &str) -> usize {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return 0;
    }
    lines
        .position(|line| line.trim_end() == "---")
        .map_or(0, |position| position + 2)
}

/// The tool names one frontmatter line declares under `allowed-tools`.
///
/// The key takes a string of names split by commas or spaces, a flow list, or
/// a block list on the lines that follow.
fn frontmatter_tools(line: &str, in_list: &mut bool) -> Vec<String> {
    if let Some(value) = line.strip_prefix("allowed-tools:") {
        let value = value.trim();
        *in_list = value.is_empty();
        let value = value
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(value);
        return split_tools(value);
    }
    if *in_list {
        let trimmed = line.trim_start();
        if let Some(item) = trimmed.strip_prefix("- ") {
            return split_tools(item);
        }
        if !line.starts_with(char::is_whitespace) || trimmed.is_empty() {
            *in_list = false;
        }
    }
    Vec::new()
}

fn split_tools(value: &str) -> Vec<String> {
    let mut tools = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for character in value.chars() {
        match character {
            '(' => {
                depth += 1;
                current.push(character);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            ',' | ' ' | '\t' if depth == 0 => push_tool(&mut tools, &mut current),
            _ => current.push(character),
        }
    }
    push_tool(&mut tools, &mut current);
    tools
}

fn push_tool(tools: &mut Vec<String>, current: &mut String) {
    let entry = current
        .trim()
        .trim_matches(|character| character == '"' || character == '\'');
    let name = entry.split('(').next().unwrap_or_default().trim();
    if !name.is_empty() {
        tools.push(name.to_ascii_lowercase());
    }
    current.clear();
}

struct Fence {
    marker: char,
    length: usize,
}

fn markdown_line<'a>(line: &'a str, fence: &mut Option<Fence>, facts: &mut LineFacts<'a>) {
    let trimmed = line.trim_start();
    if let Some(open) = fence {
        let run = trimmed
            .chars()
            .take_while(|character| *character == open.marker)
            .count();
        if run >= open.length && trimmed[run..].trim().is_empty() {
            *fence = None;
        } else {
            facts.code.push(line);
        }
        return;
    }
    for marker in ['`', '~'] {
        let run = trimmed
            .chars()
            .take_while(|character| *character == marker)
            .count();
        if run >= 3 {
            let info = trimmed[run..].trim();
            let language = info
                .trim_start_matches('{')
                .split(|character: char| {
                    character.is_whitespace()
                        || character == '{'
                        || character == '}'
                        || character == ','
                })
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !language.is_empty() {
                facts.fence = Some(language);
            }
            *fence = Some(Fence {
                marker,
                length: run,
            });
            return;
        }
    }
    facts.code.extend(line.split('`').skip(1).step_by(2));
}

fn command_matches(line: &str, tokens: &[String]) -> bool {
    let mut from = 0;
    for token in tokens {
        match find_token(line, token, from) {
            Some(end) => from = end,
            None => return false,
        }
    }
    true
}

/// The end of the first bounded `token` at or after `from`.
fn find_token(line: &str, token: &str, from: usize) -> Option<usize> {
    let first = token.chars().next()?;
    let last = token.chars().next_back()?;
    let mut start = from;
    while let Some(offset) = line[start..].find(token) {
        let at = start + offset;
        let end = at + token.len();
        if left_bounded(line, at, first) && right_bounded(line, end, last) {
            return Some(end);
        }
        start = at + first.len_utf8();
    }
    None
}

fn left_bounded(line: &str, at: usize, first: char) -> bool {
    if SELF_DELIMITING.contains(&first) {
        return true;
    }
    line[..at].chars().next_back().is_none_or(|before| {
        before.is_whitespace()
            || SELF_DELIMITING.contains(&before)
            || before == '/'
            || before == '='
    })
}

fn right_bounded(line: &str, end: usize, last: char) -> bool {
    if SELF_DELIMITING.contains(&last) {
        return true;
    }
    line[end..]
        .chars()
        .next()
        .is_none_or(|after| after.is_whitespace() || SELF_DELIMITING.contains(&after))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CaseDocument {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Case {
        name: String,
        files: Vec<CaseFile>,
        behaviors: BTreeMap<String, Vec<String>>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CaseFile {
        path: String,
        text: String,
        #[serde(default)]
        executable: bool,
    }

    #[test]
    fn behaviors_match_the_shared_cases() {
        let document: CaseDocument = serde_json::from_str(include_str!(
            "../../../contracts/fixtures/skill-behaviors/cases.json"
        ))
        .unwrap();
        let mut failures = Vec::new();
        for case in document.cases {
            let files = case
                .files
                .iter()
                .map(|file| PreparedFile {
                    path: file.path.clone(),
                    mode: if file.executable { 0o755 } else { 0o644 },
                    bytes: file.text.as_bytes().to_vec(),
                })
                .collect::<Vec<_>>();
            let actual = detect_behaviors(&files)
                .into_iter()
                .map(|behavior| {
                    (
                        behavior.id.to_owned(),
                        behavior
                            .locations
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            if actual != case.behaviors {
                failures.push(format!(
                    "{}\n  expected {:?}\n  actual   {:?}",
                    case.name, case.behaviors, actual
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn locations_stop_at_the_cap_while_the_total_counts_every_line() {
        let text = "```sh\n".to_owned() + &"sudo true\n".repeat(8) + "```\n";
        let behaviors = detect_behaviors(&[PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes: text.into_bytes(),
        }]);
        let privilege = behaviors
            .iter()
            .find(|behavior| behavior.id == "privilege")
            .unwrap();
        assert_eq!(privilege.total, 8);
        assert_eq!(privilege.locations.len(), MAX_BEHAVIOR_LOCATIONS);
    }

    #[test]
    fn rules_reject_unknown_fields_and_bad_ranges() {
        assert!(
            parse_rules(r#"{"version":1,"matching":[],"behaviors":[{"id":"a","tier":"ask","label":"A","regex":"x"}]}"#)
                .is_err()
        );
        assert!(
            parse_rules(r#"{"version":1,"matching":[],"behaviors":[{"id":"a","tier":"ask","label":"A","codepoints":[["zz","1"]]}]}"#)
                .is_err()
        );
        assert!(parse_rules(r#"{"version":2,"matching":[],"behaviors":[]}"#).is_err());
        assert!(
            parse_rules(r#"{"version":1,"matching":[],"behaviors":[{"id":"a","tier":"ask","label":"A"},{"id":"a","tier":"show","label":"B"}]}"#)
                .is_err()
        );
        assert!(
            parse_rules(r#"{"version":1,"matching":[],"behaviors":[{"id":"a","tier":"ask","label":"A","commands":[["Sudo"]]}]}"#)
                .is_err()
        );
    }
}
