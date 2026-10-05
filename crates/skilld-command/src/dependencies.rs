//! Notices for references outside the selected Skill. This module performs no I/O.

use std::collections::BTreeSet;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde::Serialize;

use crate::{CommandError, RemoteProvenance, SkillOrigin};
use skilld_core::LockedSource;

enum ReferenceSource<'a> {
    Remote(&'a RemoteProvenance),
    Local,
    Bundled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    tag = "_tag",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ExternalReference {
    /// A possible sibling Skill, not a verified dependency.
    Remote {
        reference: String,
        read_argv: Vec<String>,
    },
    /// Local paths require the calling Agent's file access permissions.
    Local { reference: String },
    /// No safe Skill read can be inferred from this reference.
    Unresolved { reference: String },
}

impl ExternalReference {
    pub fn reference(&self) -> &str {
        match self {
            Self::Remote { reference, .. }
            | Self::Local { reference }
            | Self::Unresolved { reference } => reference,
        }
    }
}

fn body(markdown: &str) -> &str {
    let Some(rest) = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))
    else {
        return markdown;
    };
    rest.find("\n---\n")
        .map(|end| &rest[end + 5..])
        .or_else(|| rest.find("\r\n---\r\n").map(|end| &rest[end + 7..]))
        .unwrap_or(markdown)
}

fn references(markdown: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    let mut skip = 0;
    for event in Parser::new(body(markdown)) {
        match event {
            Event::Start(Tag::HtmlBlock | Tag::Image { .. }) => skip += 1,
            Event::End(TagEnd::HtmlBlock | TagEnd::Image) => skip -= 1,
            Event::Start(Tag::Link { dest_url, .. })
                if skip == 0 && leaves_skill(&dest_url) && seen.insert(dest_url.to_string()) =>
            {
                found.push(dest_url.to_string());
            }
            Event::Text(text) | Event::Code(text) if skip == 0 => {
                for word in text.split(|c: char| {
                    c.is_whitespace()
                        || matches!(
                            c,
                            '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '='
                        )
                }) {
                    let reference = word.trim_end_matches(['.', ':', '!', '?']);
                    if leaves_skill(reference) && seen.insert(reference.to_owned()) {
                        found.push(reference.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    found
}

fn leaves_skill(reference: &str) -> bool {
    if reference.starts_with('/') || reference.contains(':') {
        return false;
    }
    let reference = reference.replace('\\', "/");
    if reference.contains('$') && reference.contains("/..") {
        return true;
    }
    let mut depth = 0;
    for part in reference.split('/') {
        match part {
            ".." if depth == 0 => return true,
            ".." => depth -= 1,
            "." | "" => {}
            _ => depth += 1,
        }
    }
    false
}

fn sibling_path(root: &str, reference: &str) -> Option<String> {
    let reference = reference.replace('\\', "/");
    let reference = reference
        .strip_suffix("/SKILL.md")
        .unwrap_or(&reference)
        .trim_end_matches('/');
    let mut parts: Vec<&str> = root.split('/').filter(|part| !part.is_empty()).collect();
    for part in reference.split('/') {
        match part {
            ".." => {
                parts.pop()?;
            }
            "." => {}
            "" => return None,
            name if name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) =>
            {
                parts.push(name)
            }
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Analyse instructions without reading siblings, fetching sources, or installing Skills.
pub fn external_references(markdown: &str, origin: &SkillOrigin) -> Vec<ExternalReference> {
    let source = match origin {
        SkillOrigin::Remote { provenance, .. } => ReferenceSource::Remote(provenance),
        SkillOrigin::Local { .. } => ReferenceSource::Local,
        SkillOrigin::Bundled => ReferenceSource::Bundled,
    };
    analyse(markdown, source)
}

pub(crate) fn external_references_for_source(
    markdown: &str,
    source: &LockedSource,
) -> Result<Vec<ExternalReference>, CommandError> {
    let provenance = RemoteProvenance::from_locked(source)?;
    let source = match source {
        LockedSource::Remote { .. } => {
            ReferenceSource::Remote(provenance.as_ref().expect("remote provenance"))
        }
        LockedSource::Local { .. } => ReferenceSource::Local,
        LockedSource::BundledSkilld => ReferenceSource::Bundled,
    };
    Ok(analyse(markdown, source))
}

fn analyse(markdown: &str, origin: ReferenceSource<'_>) -> Vec<ExternalReference> {
    references(markdown)
        .into_iter()
        .map(|reference| match origin {
            ReferenceSource::Remote(provenance) => {
                match sibling_path(&provenance.skill_path, &reference) {
                    Some(path) if path != provenance.skill_path => ExternalReference::Remote {
                        read_argv: vec![
                            "skilld".to_owned(),
                            "run".to_owned(),
                            format!(
                                "github:{}/{}/{}#commit:{}",
                                provenance.owner,
                                provenance.repository,
                                path,
                                provenance.commit_sha
                            ),
                            "--json".to_owned(),
                        ],
                        reference,
                    },
                    _ => ExternalReference::Unresolved { reference },
                }
            }
            ReferenceSource::Local => ExternalReference::Local { reference },
            ReferenceSource::Bundled => ExternalReference::Unresolved { reference },
        })
        .collect()
}
