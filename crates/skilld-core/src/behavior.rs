//! Skill behaviors: what a Skill's files ask an Agent to do.
//!
//! skilld matches every Skill file against the fixed rules in
//! `packages/protocol/rules/skill-behaviors.json`. A match names the behavior and where it
//! appears. No match proves nothing: text patterns miss obfuscated code.
//!
//! The rules are data so that skilld.dev can apply the same rules. The cases in
//! `contracts/fixtures/skill-behaviors/cases.json` fix the matching semantics
//! for every implementation.

use std::sync::OnceLock;

use aho_corasick::{AhoCorasick, AhoCorasickKind};
use serde::Deserialize;

use crate::PreparedFile;

const RULES_JSON: &str = include_str!("../../../packages/protocol/rules/skill-behaviors.json");

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

/// One behavior rule as `packages/protocol/rules/skill-behaviors.json` states it.
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
    /// Every substring and command token of at least [`MIN_NEEDLE_LENGTH`]
    /// bytes, found in one pass over a file.
    needles: AhoCorasick,
    /// The needle of each substring, per behavior. `None` for a short one.
    substring_needles: Vec<Vec<Option<usize>>>,
    /// The needles of each command's tokens, per behavior. `None` for a short one.
    command_needles: Vec<Vec<Vec<Option<usize>>>>,
}

/// A shorter pattern counts as present in every file, because nearly every
/// file holds it: `-`, `/`, `i`, `sh`.
const MIN_NEEDLE_LENGTH: usize = 3;

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        parse_rules(RULES_JSON).expect("packages/protocol/rules/skill-behaviors.json must parse")
    })
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
    let mut patterns: Vec<String> = Vec::new();
    let mut needle = |pattern: &str| {
        (pattern.len() >= MIN_NEEDLE_LENGTH).then(|| {
            patterns
                .iter()
                .position(|known| *known == pattern)
                .unwrap_or_else(|| {
                    patterns.push(pattern.to_owned());
                    patterns.len() - 1
                })
        })
    };
    let substring_needles = document
        .behaviors
        .iter()
        .map(|rule| rule.substrings.iter().map(|value| needle(value)).collect())
        .collect::<Vec<_>>();
    let command_needles = document
        .behaviors
        .iter()
        .map(|rule| {
            rule.commands
                .iter()
                .map(|tokens| tokens.iter().map(|token| needle(token)).collect())
                .collect()
        })
        .collect::<Vec<_>>();
    let needles = AhoCorasick::builder()
        .ascii_case_insensitive(true)
        .kind(Some(AhoCorasickKind::DFA))
        .build(&patterns)
        .map_err(|error| error.to_string())?;
    Ok(Rules {
        behaviors: document.behaviors,
        codepoints,
        needles,
        substring_needles,
        command_needles,
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
        let candidates = Candidates::for_text(rules, text);
        scan_text(rules, &candidates, &file.path, text, &mut found);
    }
    found.into_behaviors(rules)
}

/// The substrings and commands one file can match.
///
/// A line can only hold a pattern that the whole file holds, so one pass over
/// the file rules out most patterns before the line scan. The line scan then
/// tests a few patterns per line instead of every one. A command stays a
/// candidate only when the file holds each of its tokens.
struct Candidates<'a> {
    substrings: Vec<Vec<&'a str>>,
    commands: Vec<Vec<&'a [String]>>,
    /// Whether any rule has a substring or command left to test.
    any: bool,
}

impl<'a> Candidates<'a> {
    fn for_text(rules: &'a Rules, text: &str) -> Self {
        let mut present = vec![false; rules.needles.patterns_len()];
        for found in rules.needles.find_overlapping_iter(text) {
            present[found.pattern().as_usize()] = true;
        }
        let holds = |needle: &Option<usize>| needle.is_none_or(|index| present[index]);
        let substrings = rules
            .behaviors
            .iter()
            .zip(&rules.substring_needles)
            .map(|(rule, needles)| {
                rule.substrings
                    .iter()
                    .zip(needles)
                    .filter(|(_, needle)| holds(needle))
                    .map(|(substring, _)| substring.as_str())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let commands = rules
            .behaviors
            .iter()
            .zip(&rules.command_needles)
            .map(|(rule, needles)| {
                rule.commands
                    .iter()
                    .zip(needles)
                    .filter(|(_, needles)| needles.iter().all(holds))
                    .map(|(tokens, _)| tokens.as_slice())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let any = substrings.iter().any(|list| !list.is_empty())
            || commands.iter().any(|list| !list.is_empty());
        Self {
            substrings,
            commands,
            any,
        }
    }
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

fn scan_text(
    rules: &Rules,
    candidates: &Candidates<'_>,
    path: &str,
    text: &str,
    found: &mut Found,
) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let markdown = is_markdown(path);
    let frontmatter_end = if path == "SKILL.md" {
        frontmatter_end(text)
    } else {
        0
    };
    let mut fence: Option<Fence> = None;
    let mut in_tools_list = false;
    let mut prose = Prose::default();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let mut facts = LineFacts::default();
        if number <= frontmatter_end {
            facts.tools = frontmatter_tools(line, &mut in_tools_list);
        } else if markdown {
            markdown_line(line, &mut fence, &mut prose, &mut facts);
        } else {
            facts.code.push(line);
        }
        let lowered = if candidates.any {
            facts
                .code
                .iter()
                .map(|code| code.to_ascii_lowercase())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for (rule_index, rule) in rules.behaviors.iter().enumerate() {
            let patterns = LinePatterns {
                substrings: &candidates.substrings[rule_index],
                commands: &candidates.commands[rule_index],
            };
            if line_matches(
                rule,
                &rules.codepoints[rule_index],
                &patterns,
                line,
                &lowered,
                &facts,
            ) {
                found.record(rule_index, path, Some(number));
            }
        }
    }
}

/// The substrings and commands of one rule that a file can match.
struct LinePatterns<'a, 'b> {
    substrings: &'b [&'a str],
    commands: &'b [&'a [String]],
}

fn line_matches(
    rule: &BehaviorRule,
    codepoints: &[(u32, u32)],
    patterns: &LinePatterns<'_, '_>,
    line: &str,
    code: &[String],
    facts: &LineFacts<'_>,
) -> bool {
    if holds_codepoint(line, codepoints) {
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
        patterns
            .substrings
            .iter()
            .any(|substring| segment.contains(substring))
            || patterns
                .commands
                .iter()
                .any(|tokens| command_matches(segment, tokens))
    })
}

/// Whether a line holds a code point in one of these ranges.
///
/// An ASCII line holds no code point above 0x7F, so it skips the walk over
/// its characters when every range starts above it.
fn holds_codepoint(line: &str, codepoints: &[(u32, u32)]) -> bool {
    if codepoints.is_empty()
        || (line.is_ascii() && codepoints.iter().all(|(start, _)| *start > 0x7F))
    {
        return false;
    }
    line.chars().any(|character| {
        let value = u32::from(character);
        codepoints
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&value))
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

fn markdown_line<'a>(
    line: &'a str,
    fence: &mut Option<Fence>,
    prose: &mut Prose,
    facts: &mut LineFacts<'a>,
) {
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
            prose.reset();
            return;
        }
    }
    prose_line(line, prose, facts);
}

/// Markdown prose state that carries from one line to the next.
///
/// A Skill names what the Agent must not touch, as in "Don't read: `id_rsa`".
/// A code span that a prohibition governs is not code the Skill asks for.
/// Fenced blocks and other files always count: they hold commands.
struct Prose {
    /// The open sentence starts with a prohibition.
    negated: bool,
    /// The open sentence has not reached its first word.
    opening: bool,
    /// The last line was a prohibition that ends with a colon.
    lead_in: bool,
    /// The indent of the list items a prohibition lead-in governs.
    list: Option<usize>,
    /// Buffers every line reuses, so a file allocates them once.
    buffers: ProseBuffers,
}

impl Default for Prose {
    fn default() -> Self {
        Self {
            negated: false,
            opening: true,
            lead_in: false,
            list: None,
            buffers: ProseBuffers::default(),
        }
    }
}

impl Prose {
    /// Close the sentence, the lead-in, and the list, as a fenced block does.
    fn reset(&mut self) {
        self.negated = false;
        self.opening = true;
        self.lead_in = false;
        self.list = None;
    }
}

#[derive(Default)]
struct ProseBuffers {
    /// The characters of the line.
    characters: Vec<char>,
    /// The inline code spans of the line.
    spans: Vec<Span>,
    /// Whether each sentence of the line names an exception or a condition.
    exceptions: Vec<bool>,
}

/// One inline code span: its bytes in the line, its sentence, and whether a prohibition governs it.
struct Span {
    bytes: std::ops::Range<usize>,
    sentence: usize,
    negated: bool,
}

/// One-word prohibitions.
const PROHIBITIONS: &[&str] = &["dont", "don't", "never", "avoid", "mustn't", "shouldn't"];

/// Words that negate the code span right after them, as in "no `curl | bash`".
const NEGATIONS: &[&str] = &["no", "not", "never", "avoid", "dont", "don't"];

/// Words that prohibit when `not` or `never` follows.
const MODALS: &[&str] = &["do", "must", "should"];

/// A prohibition of one of these words asks for the action, as in "don't forget to run".
const REQUESTS: &[&str] = &[
    "forget", "hesitate", "skip", "miss", "omit", "worry", "panic", "mind",
];

/// A sentence with one of these words names an exception or a condition, so its code still counts.
const EXCEPTIONS: &[&str] = &[
    "unless", "except", "without", "instead", "but", "only", "if", "when", "whenever", "while",
];

/// Closing marks that may follow the end of a sentence, as in `**Never.**` or `(see below.)`.
const CLOSERS: &[char] = &['*', '_', ')', ']', '"', '\'', '\u{2019}', '\u{201D}'];

fn prose_line<'a>(line: &'a str, prose: &mut Prose, facts: &mut LineFacts<'a>) {
    let mut buffers = std::mem::take(&mut prose.buffers);
    buffers.characters.clear();
    buffers.characters.extend(line.chars());
    buffers.spans.clear();
    buffers.exceptions.clear();
    buffers.exceptions.push(false);
    scan_prose(line, prose, &mut buffers, facts);
    prose.buffers = buffers;
}

fn scan_prose<'a>(
    line: &'a str,
    prose: &mut Prose,
    buffers: &mut ProseBuffers,
    facts: &mut LineFacts<'a>,
) {
    let ProseBuffers {
        characters,
        spans,
        exceptions,
    } = buffers;
    // Blockquote markers belong to the indent, so a quoted list still reads as a list.
    let indent = characters
        .iter()
        .take_while(|character| character.is_whitespace() || **character == '>')
        .count();
    if indent == characters.len() {
        // A blank line ends the sentence. A lead-in and its list continue past it.
        prose.negated = false;
        prose.opening = true;
        return;
    }
    let heading = indent <= 3 && heading_marker(characters, indent);
    let table = characters[indent] == '|';
    let item = if heading || table {
        0
    } else {
        list_marker(characters, indent)
    };
    if heading || table {
        prose.list = None;
        prose.lead_in = false;
        prose.negated = false;
        prose.opening = true;
    } else if item > 0 {
        if prose.list.is_some_and(|list| indent < list) {
            prose.list = None;
        }
        if prose.lead_in {
            prose.list = Some(prose.list.map_or(indent, |list| list.min(indent)));
        }
        prose.lead_in = false;
        prose.negated = false;
        prose.opening = true;
    } else {
        // A line that does not indent past the list ends it.
        if prose.list.is_some_and(|list| indent <= list) {
            prose.list = None;
        }
        prose.lead_in = false;
    }
    let governed = prose.list.is_some();
    let backticks = line.bytes().filter(|byte| *byte == b'`').count();
    // Exceptions and negations decide only code spans and a lead-in. A line
    // with neither skips them.
    let classify = backticks > 0
        || line
            .trim_end_matches(|character: char| {
                character.is_whitespace() || matches!(character, '*' | '_')
            })
            .ends_with(':');
    let mut last_colon = false;
    // The word right before a code span negates it, with nothing but whitespace or emphasis between.
    let mut before_negates = false;
    let (mut byte, mut character_at) = (0, 0);
    for (index, segment) in line.split('`').enumerate() {
        let bytes = byte..byte + segment.len();
        let end = character_at + segment.chars().count();
        let start = character_at;
        byte = bytes.end + 1;
        character_at = end + 1;
        if index % 2 == 1 {
            prose.opening = false;
            spans.push(Span {
                bytes,
                sentence: exceptions.len() - 1,
                negated: prose.negated || governed || before_negates,
            });
            before_negates = false;
            continue;
        }
        let skip = if index == 0 { indent + item } else { 0 };
        let text = &characters[(start + skip).min(end)..end];
        let last = index == backticks;
        let mut at = 0;
        while at < text.len() {
            let character = text[at];
            if character.is_ascii_alphanumeric() {
                let (word, end) = read_word(text, at);
                if prose.opening {
                    prose.opening = false;
                    prose.negated = opens_prohibition(word, text, end);
                }
                if classify {
                    if word.is(EXCEPTIONS)
                        && let Some(exception) = exceptions.last_mut()
                    {
                        *exception = true;
                    }
                    before_negates = word.is(NEGATIONS);
                }
                at = end;
                continue;
            }
            if !character.is_whitespace() && !matches!(character, '*' | '_' | '~') {
                before_negates = false;
            }
            match boundary(text, at, last) {
                Some(Boundary::Sentence) => {
                    prose.negated = false;
                    prose.opening = true;
                    exceptions.push(false);
                }
                // A dash ends a prohibition, as in "Never X — use `Y`". It opens none.
                Some(Boundary::Clause) => prose.negated = false,
                // A label such as "Tip:" ends, and the words after it open the sentence again.
                None if character == ':' && !prose.negated => prose.opening = true,
                None => {}
            }
            at += 1;
        }
        if last {
            last_colon = ends_with_colon(text);
        }
    }
    for span in spans.iter() {
        if !span.negated || exceptions[span.sentence] {
            facts.code.push(&line[span.bytes.clone()]);
        }
    }
    if heading {
        prose.negated = false;
        prose.opening = true;
    } else if !table {
        prose.lead_in =
            prose.negated && !exceptions.last().copied().unwrap_or_default() && last_colon;
    }
}

/// Whether prose ends with a colon, after closing emphasis such as `**Don't read:**`.
fn ends_with_colon(text: &[char]) -> bool {
    text.iter()
        .rev()
        .find(|character| !(character.is_whitespace() || matches!(character, '*' | '_')))
        == Some(&':')
}

/// Whether ATX heading hashes open the line at `at`.
fn heading_marker(characters: &[char], at: usize) -> bool {
    let run = characters[at..]
        .iter()
        .take_while(|character| **character == '#')
        .count();
    (1..=6).contains(&run)
        && characters
            .get(at + run)
            .is_none_or(|character| matches!(character, ' ' | '\t'))
}

/// The length of a list marker at `at`, with the whitespace and checkbox after it.
/// 0 when the line holds no list item.
fn list_marker(characters: &[char], at: usize) -> usize {
    let mut end = at;
    if matches!(characters.get(end), Some('-' | '*' | '+')) {
        end += 1;
    } else {
        while end - at < 9 && characters.get(end).is_some_and(char::is_ascii_digit) {
            end += 1;
        }
        if end == at || !matches!(characters.get(end), Some('.' | ')')) {
            return 0;
        }
        end += 1;
    }
    if characters
        .get(end)
        .is_some_and(|character| !character.is_whitespace())
    {
        return 0;
    }
    while characters
        .get(end)
        .is_some_and(|character| character.is_whitespace())
    {
        end += 1;
    }
    let checkbox = characters.get(end) == Some(&'[')
        && matches!(characters.get(end + 1), Some(' ' | 'x' | 'X'))
        && characters.get(end + 2) == Some(&']')
        && characters
            .get(end + 3)
            .is_none_or(|character| character.is_whitespace());
    if checkbox {
        end += 3;
    }
    end - at
}

/// A word of ASCII letters, digits, and apostrophes in prose.
#[derive(Clone, Copy)]
struct Word<'t>(&'t [char]);

impl Word<'_> {
    /// Whether the word is one of `words`, ignoring ASCII case and trailing apostrophes.
    /// A typographic apostrophe counts as `'`.
    fn is(self, words: &[&str]) -> bool {
        let mut word = self.0;
        while let [rest @ .., '\'' | '\u{2019}'] = word {
            word = rest;
        }
        words.iter().any(|candidate| {
            candidate.len() == word.len()
                && candidate.chars().zip(word).all(|(expected, actual)| {
                    let actual = if *actual == '\u{2019}' {
                        '\''
                    } else {
                        actual.to_ascii_lowercase()
                    };
                    expected == actual
                })
        })
    }
}

/// The word that starts at `at`, and the index after it.
fn read_word(text: &[char], at: usize) -> (Word<'_>, usize) {
    let end = at
        + text[at..]
            .iter()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '\'' | '\u{2019}')
            })
            .count();
    (Word(&text[at..end]), end)
}

/// The next word after emphasis and whitespace, unless something else comes first.
fn next_word(text: &[char], at: usize) -> Option<(Word<'_>, usize)> {
    let start = at
        + text[at..]
            .iter()
            .take_while(|character| {
                character.is_whitespace() || matches!(character, '*' | '_' | '~')
            })
            .count();
    text.get(start)
        .is_some_and(char::is_ascii_alphanumeric)
        .then(|| read_word(text, start))
}

fn opens_prohibition(word: Word<'_>, text: &[char], end: usize) -> bool {
    let second = next_word(text, end);
    let asks = |next: Option<(Word<'_>, usize)>| next.is_some_and(|(word, _)| word.is(REQUESTS));
    if word.is(PROHIBITIONS) {
        return !asks(second);
    }
    match second {
        Some((negation, after)) if word.is(MODALS) && negation.is(&["not", "never"]) => {
            !asks(next_word(text, after))
        }
        _ => false,
    }
}

/// What one character of prose ends.
enum Boundary {
    Sentence,
    Clause,
}

/// What the character at `at` ends, if anything.
///
/// `;` and `|` end a sentence. `.`, `!`, and `?` end one before whitespace or the
/// end of the line, after any closing marks. A dash ends a clause.
fn boundary(text: &[char], at: usize, last: bool) -> Option<Boundary> {
    match text[at] {
        ';' | '|' => Some(Boundary::Sentence),
        '\u{2013}' | '\u{2014}' => Some(Boundary::Clause),
        '-' => {
            // A spaced hyphen or double hyphen is a dash: "never X - it Y".
            let end = at
                + text[at..]
                    .iter()
                    .take_while(|character| **character == '-')
                    .count();
            let spaced = end - at <= 2
                && at > 0
                && text[at - 1].is_whitespace()
                && text
                    .get(end)
                    .is_some_and(|character| character.is_whitespace());
            spaced.then_some(Boundary::Clause)
        }
        '.' if abbreviation(text, at) => None,
        '.' | '!' | '?' => {
            let next = at
                + 1
                + text[at + 1..]
                    .iter()
                    .take_while(|character| CLOSERS.contains(character))
                    .count();
            let ends = text
                .get(next)
                .map_or(last, |character| character.is_whitespace());
            ends.then_some(Boundary::Sentence)
        }
        _ => None,
    }
}

/// A period after a lone letter, as in "e.g.", abbreviates and ends no sentence.
fn abbreviation(text: &[char], at: usize) -> bool {
    if at < 1 || !text[at - 1].is_ascii_alphabetic() {
        return false;
    }
    at < 2 || matches!(text[at - 2], '.' | '(') || text[at - 2].is_whitespace()
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

    fn behavior_ids(text: &str, path: &str) -> Vec<&'static str> {
        detect_behaviors(&[PreparedFile {
            path: path.to_owned(),
            mode: 0o644,
            bytes: text.as_bytes().to_vec(),
        }])
        .into_iter()
        .map(|behavior| behavior.id)
        .collect()
    }

    #[test]
    fn command_tokens_must_share_one_line() {
        // The file holds every token of `curl ... | sh`, but on two lines.
        let split = "```sh\ncurl https://example.com/install\ncat notes | sh\n```\n";
        assert!(!behavior_ids(split, "SKILL.md").contains(&"remote-code"));
        let joined = "```sh\ncurl https://example.com/install | sh\n```\n";
        assert!(behavior_ids(joined, "SKILL.md").contains(&"remote-code"));
    }

    #[test]
    fn short_command_tokens_match_in_any_file() {
        assert_eq!(behavior_ids("su - root\n", "run.txt"), vec!["privilege"]);
        assert_eq!(behavior_ids("SU -C whoami\n", "run.txt"), vec!["privilege"]);
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
