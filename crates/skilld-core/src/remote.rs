use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::SkillName;

const ATTESTATION_DOMAIN: &[u8] = b"skilld-attestation-v1\0";
const TRUSTED_KEY_DOMAIN: &[u8] = b"skilld-trusted-key-v1\0";
const MAX_STATEMENT_BYTES: usize = 6 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 2_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SourceRequest {
    pub provider: SourceProvider,
    pub owner: String,
    pub repository: String,
    pub selector: SourceSelector,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<SourceRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceProvider {
    Github,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "kebab-case")]
pub enum SourceSelector {
    Path { path: String },
    NamedSkill { name: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "lowercase")]
pub enum SourceRef {
    Branch { value: String },
    Tag { value: String },
    Commit { value: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteSelector {
    Skilld(SourceRequest),
    Github(SourceRequest),
}

impl RemoteSelector {
    pub fn parse(value: &str) -> Result<Self, RemoteError> {
        if value.chars().any(is_unsafe_terminal) {
            return Err(RemoteError::new(
                "INVALID_SOURCE",
                "the remote selector cannot contain terminal formatting characters",
            ));
        }
        let value = value.trim();
        if let Some(rest) = value.strip_prefix("skilld:") {
            let (source, reference) = rest
                .split_once('#')
                .map_or((rest, None), |(source, value)| {
                    (source, Some(parse_source_ref(value)))
                });
            let (owner, repository, name) = split_three(source)?;
            let request = SourceRequest {
                provider: SourceProvider::Github,
                owner: owner.to_owned(),
                repository: repository.to_owned(),
                selector: SourceSelector::NamedSkill {
                    name: name.to_owned(),
                },
                r#ref: reference,
            };
            validate_source_request(&request)?;
            return Ok(Self::Skilld(request));
        }
        if let Some(rest) = value.strip_prefix("github:") {
            return parse_github_shorthand(rest).map(Self::Github);
        }
        if let Some(rest) = value.strip_prefix("https://github.com/") {
            return parse_github_url(rest).map(Self::Github);
        }
        Err(RemoteError::new(
            "INVALID_SOURCE",
            "use a skilld search selector or an explicit GitHub Repository selector",
        ))
    }

    pub const fn source(&self) -> &SourceRequest {
        match self {
            Self::Skilld(source) | Self::Github(source) => source,
        }
    }

    pub const fn is_explicit_github(&self) -> bool {
        matches!(self, Self::Github(_))
    }

    pub fn canonical(&self) -> String {
        let source = self.source();
        let prefix = if self.is_explicit_github() {
            "github:"
        } else {
            "skilld:"
        };
        let selector = match &source.selector {
            SourceSelector::Path { path } | SourceSelector::NamedSkill { name: path } => path,
        };
        let reference = source.r#ref.as_ref().map_or_else(String::new, |reference| {
            let (kind, value) = match reference {
                SourceRef::Branch { value } => ("branch", value),
                SourceRef::Tag { value } => ("tag", value),
                SourceRef::Commit { value } => ("commit", value),
            };
            format!("#{kind}:{value}")
        });
        format!(
            "{prefix}{}/{}/{}{}",
            source.owner, source.repository, selector, reference
        )
    }
}

impl fmt::Display for RemoteSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

fn parse_github_shorthand(value: &str) -> Result<SourceRequest, RemoteError> {
    let (source, reference) = value
        .split_once('#')
        .map_or((value, None), |(source, value)| {
            (source, Some(parse_source_ref(value)))
        });
    let (owner, repository, path) = split_three(source)?;
    let request = SourceRequest {
        provider: SourceProvider::Github,
        owner: owner.to_owned(),
        repository: repository.trim_end_matches(".git").to_owned(),
        selector: SourceSelector::Path {
            path: path.trim_end_matches('/').to_owned(),
        },
        r#ref: reference,
    };
    validate_source_request(&request)?;
    Ok(request)
}

fn parse_github_url(value: &str) -> Result<SourceRequest, RemoteError> {
    if value.contains(['?', '#']) {
        return Err(RemoteError::new(
            "INVALID_SOURCE",
            "GitHub Repository URLs cannot contain a query or fragment",
        ));
    }
    let parts = value.trim_end_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() < 5 || parts[2] != "tree" {
        return Err(RemoteError::new(
            "INVALID_SOURCE",
            "use a GitHub tree URL that includes a Skill path",
        ));
    }
    let request = SourceRequest {
        provider: SourceProvider::Github,
        owner: parts[0].to_owned(),
        repository: parts[1].trim_end_matches(".git").to_owned(),
        selector: SourceSelector::Path {
            path: parts[4..].join("/"),
        },
        r#ref: Some(parse_source_ref(parts[3])),
    };
    validate_source_request(&request)?;
    Ok(request)
}

fn parse_source_ref(value: &str) -> SourceRef {
    if let Some(value) = value.strip_prefix("tag:") {
        SourceRef::Tag {
            value: value.to_owned(),
        }
    } else if let Some(value) = value.strip_prefix("branch:") {
        SourceRef::Branch {
            value: value.to_owned(),
        }
    } else if let Some(value) = value.strip_prefix("commit:") {
        SourceRef::Commit {
            value: value.to_owned(),
        }
    } else if is_commit_sha(value) {
        SourceRef::Commit {
            value: value.to_owned(),
        }
    } else {
        SourceRef::Branch {
            value: value.to_owned(),
        }
    }
}

fn split_three(value: &str) -> Result<(&str, &str, &str), RemoteError> {
    let mut parts = value.splitn(3, '/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(repository), Some(selector))
            if !owner.is_empty() && !repository.is_empty() && !selector.is_empty() =>
        {
            Ok((owner, repository, selector))
        }
        _ => Err(RemoteError::new(
            "INVALID_SOURCE",
            "the remote selector must include owner, repository, and Skill",
        )),
    }
}

/// Whether a value is a valid GitHub account login.
pub(crate) fn valid_owner(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 39
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Whether a value is a valid GitHub Repository name.
pub(crate) fn valid_repository(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_source_request(source: &SourceRequest) -> Result<(), RemoteError> {
    if !valid_owner(&source.owner) || !valid_repository(&source.repository) {
        return Err(RemoteError::new(
            "INVALID_SOURCE",
            "the GitHub Repository owner or name is invalid",
        ));
    }
    match &source.selector {
        SourceSelector::Path { path } if path.contains('#') => Err(RemoteError::new(
            "INVALID_SOURCE",
            "the Skill source path cannot contain #",
        )),
        SourceSelector::Path { path } => validate_relative_path(path, 1024),
        SourceSelector::NamedSkill { name } => SkillName::parse(name.clone())
            .map(|_| ())
            .map_err(|_| RemoteError::new("INVALID_SOURCE", "the named Skill selector is invalid")),
    }?;
    if let Some(reference) = &source.r#ref {
        let value = match reference {
            SourceRef::Branch { value } | SourceRef::Tag { value } => {
                if value.is_empty()
                    || value.len() > 255
                    || value.contains(['\0', '\\'])
                    || value.chars().any(is_unsafe_terminal)
                    || value.starts_with('-')
                {
                    return Err(RemoteError::new(
                        "INVALID_SOURCE",
                        "the Git source reference is invalid",
                    ));
                }
                value
            }
            SourceRef::Commit { value } => {
                if !is_commit_sha(value) {
                    return Err(RemoteError::new(
                        "INVALID_SOURCE",
                        "the Git commit must use 40 lowercase hexadecimal characters",
                    ));
                }
                value
            }
        };
        if value.contains("..") || value.ends_with('.') || value.ends_with('/') {
            return Err(RemoteError::new(
                "INVALID_SOURCE",
                "the Git source reference is invalid",
            ));
        }
    }
    Ok(())
}

fn is_commit_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SearchResponse {
    pub items: Vec<SearchResult>,
    pub total: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SearchResult {
    pub name: String,
    pub description: Option<String>,
    pub source: SourceRequest,
    pub stargazer_count: u64,
}

impl SearchResult {
    pub fn selector(&self) -> Result<RemoteSelector, RemoteError> {
        validate_source_request(&self.source)?;
        if self.source.r#ref.is_some()
            || !matches!(self.source.selector, SourceSelector::NamedSkill { .. })
        {
            return Err(RemoteError::new(
                "INVALID_RESPONSE",
                "Skill search returned an invalid source selector",
            ));
        }
        Ok(RemoteSelector::Skilld(self.source.clone()))
    }
}

pub fn parse_search_response(bytes: &[u8]) -> Result<SearchResponse, RemoteError> {
    if bytes.len() > 1024 * 1024 {
        return Err(RemoteError::new(
            "RESPONSE_TOO_LARGE",
            "Skill search exceeded the response limit",
        ));
    }
    let response: SearchResponse = serde_json::from_slice(bytes)
        .map_err(|_| RemoteError::new("INVALID_RESPONSE", "Skill search returned invalid JSON"))?;
    if response.items.len() > 50 || response.total < response.items.len() as u64 {
        return Err(RemoteError::new(
            "INVALID_RESPONSE",
            "Skill search returned invalid result counts",
        ));
    }
    for item in &response.items {
        if SkillName::parse(item.name.clone()).is_err()
            || item
                .description
                .as_ref()
                .is_some_and(|value| value.len() > 500)
        {
            return Err(RemoteError::new(
                "INVALID_RESPONSE",
                "Skill search returned an oversized field",
            ));
        }
        item.selector()?;
        if !matches!(&item.source.selector, SourceSelector::NamedSkill { name } if name == &item.name)
        {
            return Err(RemoteError::new(
                "INVALID_RESPONSE",
                "Skill search returned mismatched Skill names",
            ));
        }
    }
    Ok(response)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RepositoryVisibility {
    Public,
    Private,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResolvedSource {
    pub provider: SourceProvider,
    pub repository_id: u64,
    pub owner: String,
    pub repository: String,
    pub visibility: RepositoryVisibility,
    pub commit_sha: String,
    pub tree_sha: String,
    pub skill_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckOutcome {
    Pass,
    Warn,
    Fail,
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CheckResult {
    pub name: String,
    pub version: String,
    pub outcome: CheckOutcome,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactFile {
    pub path: String,
    pub mode: u32,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AttestationSignature {
    pub algorithm: SignatureAlgorithm,
    pub key_id: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SignatureAlgorithm {
    Ed25519,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactAttestation {
    pub version: u8,
    pub artifact_id: String,
    pub created_at: String,
    pub source: ResolvedSource,
    pub source_status: String,
    pub format: String,
    pub content_sha256: String,
    pub content_bytes: u64,
    pub policy_version: String,
    pub files: Vec<ArtifactFile>,
    pub check_results: Vec<CheckResult>,
    pub statement: String,
    pub signature: AttestationSignature,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SignedStatement {
    version: u8,
    artifact_id: String,
    created_at: String,
    source: ResolvedSource,
    source_status: String,
    format: String,
    content_sha256: String,
    content_bytes: u64,
    policy_version: String,
    files: Vec<ArtifactFile>,
    check_results: Vec<CheckResult>,
}

impl ArtifactAttestation {
    fn signed_fields(&self) -> SignedStatement {
        SignedStatement {
            version: self.version,
            artifact_id: self.artifact_id.clone(),
            created_at: self.created_at.clone(),
            source: self.source.clone(),
            source_status: self.source_status.clone(),
            format: self.format.clone(),
            content_sha256: self.content_sha256.clone(),
            content_bytes: self.content_bytes,
            policy_version: self.policy_version.clone(),
            files: self.files.clone(),
            check_results: self.check_results.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TrustedRoot {
    pub version: u8,
    pub root_key_id: String,
    pub root_public_key: String,
    pub keys: Vec<TrustedKey>,
    pub fetched_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TrustedKey {
    pub key_id: String,
    pub algorithm: SignatureAlgorithm,
    pub public_key: String,
    pub not_before: String,
    pub not_after: String,
    pub status: TrustedKeyStatus,
    pub statement: String,
    pub root_signature: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustedKeyStatus {
    Active,
    Overlapping,
    Retired,
    Revoked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TrustedKeyStatement {
    version: u8,
    root_key_id: String,
    key_id: String,
    algorithm: SignatureAlgorithm,
    public_key: String,
    not_before: String,
    not_after: String,
    status: TrustedKeyStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedRootPin {
    pub key_id: String,
    pub public_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTrustedRoot(TrustedRoot);

impl VerifiedTrustedRoot {
    pub const fn root(&self) -> &TrustedRoot {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedFile {
    pub path: String,
    pub mode: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedArtifact {
    pub name: SkillName,
    pub files: Vec<PreparedFile>,
    pub installed_sha256: String,
    pub attestation: ArtifactAttestation,
}

pub fn prepare_unverified_files(
    mut files: Vec<PreparedFile>,
) -> Result<(SkillName, String, Vec<PreparedFile>), RemoteError> {
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(archive_error("the Skill file count is invalid"));
    }
    let mut paths = BTreeSet::new();
    let mut total = 0_usize;
    for file in &files {
        validate_relative_path(&file.path, 1024)?;
        if !paths.insert(file.path.clone()) || !matches!(file.mode, 0o644 | 0o755) {
            return Err(archive_error(
                "the Skill contains an invalid file declaration",
            ));
        }
        total = total
            .checked_add(file.bytes.len())
            .ok_or_else(|| archive_error("the Skill content size overflowed"))?;
        if total > MAX_ARCHIVE_BYTES {
            return Err(archive_error("the Skill content exceeds the size limit"));
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let name = skill_name_from_files(&files)?;
    let digest = installed_digest(&files);
    Ok((name, digest, files))
}

pub fn verify_artifact(
    attestation: ArtifactAttestation,
    root: &VerifiedTrustedRoot,
    archive: &[u8],
) -> Result<VerifiedArtifact, RemoteError> {
    verify_attestation(&attestation, root)?;
    if archive.len() > MAX_ARCHIVE_BYTES || archive.len() as u64 != attestation.content_bytes {
        return Err(RemoteError::new(
            "ARTIFACT_SIZE_MISMATCH",
            "the Artifact size does not match its attestation",
        ));
    }
    let content_sha256 = sha256(archive);
    if content_sha256 != attestation.content_sha256
        || attestation.artifact_id != format!("sha256:{content_sha256}")
    {
        return Err(RemoteError::new(
            "ARTIFACT_DIGEST_MISMATCH",
            "the Artifact digest does not match its attestation",
        ));
    }
    let files = verify_ustar(archive, &attestation.files)?;
    let name = skill_name_from_files(&files)?;
    let installed_sha256 = installed_digest(&files);
    Ok(VerifiedArtifact {
        name,
        files,
        installed_sha256,
        attestation,
    })
}

pub fn verify_attestation(
    attestation: &ArtifactAttestation,
    root: &VerifiedTrustedRoot,
) -> Result<(), RemoteError> {
    validate_attestation_shape(attestation)?;
    let statement = decode_base64url(&attestation.statement, MAX_STATEMENT_BYTES)?;
    let parsed: SignedStatement = serde_json::from_slice(&statement).map_err(|_| {
        RemoteError::new(
            "ATTESTATION_INVALID",
            "the attestation statement is invalid JSON",
        )
    })?;
    if parsed != attestation.signed_fields() {
        return Err(RemoteError::new(
            "ATTESTATION_MISMATCH",
            "the attestation statement does not match its outer fields",
        ));
    }
    verify_signature(attestation, root, &statement)?;
    if attestation
        .check_results
        .iter()
        .any(|result| result.required && result.outcome != CheckOutcome::Pass)
    {
        return Err(RemoteError::new(
            "CHECK_BLOCKED",
            "a required check result did not pass",
        ));
    }
    Ok(())
}

pub fn verify_trusted_root(
    root: TrustedRoot,
    pin: &TrustedRootPin,
) -> Result<VerifiedTrustedRoot, RemoteError> {
    if root.version != 1
        || root.root_key_id != pin.key_id
        || root.root_public_key != pin.public_key
        || root.keys.is_empty()
    {
        return Err(RemoteError::new(
            "TRUSTED_ROOT_MISMATCH",
            "the trusted root does not match the CLI pin",
        ));
    }
    parse_utc_timestamp(&root.fetched_at)?;
    let root_key = verifying_key(&root.root_public_key, "the root public key is invalid")?;
    let mut key_ids = BTreeSet::new();
    for key in &root.keys {
        if !key_ids.insert(key.key_id.clone()) {
            return Err(RemoteError::new(
                "TRUSTED_ROOT_INVALID",
                "the trusted root contains duplicate signing keys",
            ));
        }
        let statement_bytes = decode_base64url(&key.statement, 4096)?;
        let statement: TrustedKeyStatement =
            serde_json::from_slice(&statement_bytes).map_err(|_| {
                RemoteError::new(
                    "TRUSTED_ROOT_INVALID",
                    "a trusted key statement is invalid JSON",
                )
            })?;
        let expected = TrustedKeyStatement {
            version: 1,
            root_key_id: root.root_key_id.clone(),
            key_id: key.key_id.clone(),
            algorithm: key.algorithm.clone(),
            public_key: key.public_key.clone(),
            not_before: key.not_before.clone(),
            not_after: key.not_after.clone(),
            status: key.status.clone(),
        };
        if statement != expected {
            return Err(RemoteError::new(
                "TRUSTED_ROOT_MISMATCH",
                "a trusted key statement does not match its outer fields",
            ));
        }
        parse_utc_timestamp(&key.not_before)?;
        parse_utc_timestamp(&key.not_after)?;
        if compare_timestamp(&key.not_before, &key.not_after)? >= 0 {
            return Err(RemoteError::new(
                "TRUSTED_ROOT_INVALID",
                "a trusted signing key has an invalid validity window",
            ));
        }
        verifying_key(&key.public_key, "a trusted signing key is invalid")?;
        verify_ed25519(
            &root_key,
            TRUSTED_KEY_DOMAIN,
            &statement_bytes,
            &key.root_signature,
            "TRUSTED_ROOT_SIGNATURE_INVALID",
            "a trusted key root signature is invalid",
        )?;
    }
    Ok(VerifiedTrustedRoot(root))
}

fn validate_attestation_shape(attestation: &ArtifactAttestation) -> Result<(), RemoteError> {
    if attestation.version != 1
        || attestation.source_status != "verified"
        || attestation.format != "skilld-tar-v1"
        || !is_sha256(&attestation.content_sha256)
        || attestation.artifact_id != format!("sha256:{}", attestation.content_sha256)
        || attestation.files.is_empty()
        || attestation.files.len() > MAX_FILES
        || attestation.check_results.is_empty()
        || attestation.check_results.len() > 100
        || attestation.content_bytes == 0
        || attestation.policy_version.is_empty()
        || attestation.policy_version.len() > 100
        || attestation.signature.key_id.is_empty()
        || attestation.signature.key_id.len() > 100
        || attestation.source.repository_id == 0
        || !is_commit_sha(&attestation.source.commit_sha)
        || !is_commit_sha(&attestation.source.tree_sha)
    {
        return Err(RemoteError::new(
            "ATTESTATION_INVALID",
            "the Artifact attestation has invalid fields",
        ));
    }
    parse_utc_timestamp(&attestation.created_at)?;
    validate_source_request(&SourceRequest {
        provider: SourceProvider::Github,
        owner: attestation.source.owner.clone(),
        repository: attestation.source.repository.clone(),
        selector: SourceSelector::Path {
            path: attestation.source.skill_path.clone(),
        },
        r#ref: Some(SourceRef::Commit {
            value: attestation.source.commit_sha.clone(),
        }),
    })?;
    let mut paths = BTreeSet::new();
    let mut folded_paths = BTreeSet::new();
    let mut content_size = 0_u64;
    for file in &attestation.files {
        validate_relative_path(&file.path, 1024)?;
        if !paths.insert(file.path.clone())
            || !folded_paths.insert(file.path.to_ascii_lowercase())
            || !matches!(file.mode, 0o644 | 0o755)
            || !is_sha256(&file.sha256)
        {
            return Err(RemoteError::new(
                "ATTESTATION_INVALID",
                "the Artifact file declaration is invalid",
            ));
        }
        content_size = content_size.checked_add(file.size).ok_or_else(|| {
            RemoteError::new("ATTESTATION_INVALID", "the Artifact file sizes overflowed")
        })?;
    }
    if content_size > attestation.content_bytes
        || paths.iter().any(|path| {
            paths
                .iter()
                .any(|other| path != other && other.starts_with(&format!("{path}/")))
        })
    {
        return Err(RemoteError::new(
            "ATTESTATION_INVALID",
            "the Artifact file declarations conflict",
        ));
    }
    for check in &attestation.check_results {
        if check.name.is_empty()
            || check.name.len() > 100
            || check.version.is_empty()
            || check.version.len() > 50
            || check
                .summary
                .as_ref()
                .is_some_and(|value| value.len() > 500)
            || check.findings.len() > 100
            || check.findings.iter().any(|value| value.len() > 500)
        {
            return Err(RemoteError::new(
                "ATTESTATION_INVALID",
                "an Artifact check result is invalid",
            ));
        }
    }
    Ok(())
}

fn verify_signature(
    attestation: &ArtifactAttestation,
    root: &VerifiedTrustedRoot,
    statement: &[u8],
) -> Result<(), RemoteError> {
    let key = root
        .root()
        .keys
        .iter()
        .find(|key| key.key_id == attestation.signature.key_id)
        .ok_or_else(|| {
            RemoteError::new(
                "SIGNING_KEY_UNKNOWN",
                "the attestation signing key is not trusted",
            )
        })?;
    if !matches!(
        key.status,
        TrustedKeyStatus::Active | TrustedKeyStatus::Overlapping
    ) || compare_timestamp(&attestation.created_at, &key.not_before)? < 0
        || compare_timestamp(&attestation.created_at, &key.not_after)? > 0
    {
        return Err(RemoteError::new(
            "SIGNING_KEY_INVALID",
            "the signing key was not valid when the attestation was created",
        ));
    }
    let key = verifying_key(&key.public_key, "the signing public key is invalid")?;
    verify_ed25519(
        &key,
        ATTESTATION_DOMAIN,
        statement,
        &attestation.signature.value,
        "ATTESTATION_SIGNATURE_INVALID",
        "the attestation signature is invalid",
    )
}

fn verifying_key(value: &str, message: &'static str) -> Result<VerifyingKey, RemoteError> {
    let bytes = decode_base64url(value, 32)?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| RemoteError::new("TRUSTED_ROOT_INVALID", message))?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| RemoteError::new("TRUSTED_ROOT_INVALID", message))
}

fn verify_ed25519(
    key: &VerifyingKey,
    domain: &[u8],
    statement: &[u8],
    signature: &str,
    code: &'static str,
    message: &'static str,
) -> Result<(), RemoteError> {
    let signature = decode_base64url(signature, 64)?;
    let signature =
        Signature::try_from(signature.as_slice()).map_err(|_| RemoteError::new(code, message))?;
    let digest = Sha256::digest(statement);
    let mut signed = Vec::with_capacity(domain.len() + digest.len());
    signed.extend_from_slice(domain);
    signed.extend_from_slice(&digest);
    key.verify_strict(&signed, &signature)
        .map_err(|_| RemoteError::new(code, message))
}

fn verify_ustar(
    archive: &[u8],
    declarations: &[ArtifactFile],
) -> Result<Vec<PreparedFile>, RemoteError> {
    if archive.len() % 512 != 0 {
        return Err(archive_error("the USTAR archive is truncated"));
    }
    let declarations = declarations
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    let mut folded = BTreeSet::new();
    let mut offset = 0_usize;
    let mut zero_blocks = 0_u8;
    while offset < archive.len() {
        let header = &archive[offset..offset + 512];
        offset += 512;
        if header.iter().all(|byte| *byte == 0) {
            zero_blocks += 1;
            if zero_blocks == 2 {
                if archive[offset..].iter().any(|byte| *byte != 0) {
                    return Err(archive_error("the USTAR archive has trailing data"));
                }
                break;
            }
            continue;
        }
        if zero_blocks != 0 {
            return Err(archive_error("the USTAR archive has an invalid end marker"));
        }
        if &header[257..263] != b"ustar\0" {
            return Err(archive_error("the archive must use USTAR format"));
        }
        verify_tar_checksum(header)?;
        let path = tar_path(header)?;
        if !seen.insert(path.clone()) || !folded.insert(path.to_ascii_lowercase()) {
            return Err(archive_error("the USTAR archive contains a duplicate path"));
        }
        let entry_type = header[156];
        let size = parse_tar_octal(&header[124..136])?;
        if entry_type == b'5' {
            if size != 0 {
                return Err(archive_error("a USTAR directory has content"));
            }
            continue;
        }
        if !matches!(entry_type, 0 | b'0') {
            return Err(archive_error(
                "USTAR links, devices, extensions, and sparse files are rejected",
            ));
        }
        let declaration = declarations.get(path.as_str()).ok_or_else(|| {
            RemoteError::new(
                "UNDECLARED_ARTIFACT_FILE",
                "the USTAR archive contains an undeclared file",
            )
        })?;
        if size != declaration.size || size > MAX_ARCHIVE_BYTES as u64 {
            return Err(archive_error(
                "a USTAR file size does not match its declaration",
            ));
        }
        let mode = parse_tar_octal(&header[100..108])? as u32 & 0o777;
        if mode != declaration.mode {
            return Err(archive_error(
                "a USTAR file mode does not match its declaration",
            ));
        }
        let end = offset
            .checked_add(size as usize)
            .ok_or_else(|| archive_error("a USTAR file size overflowed"))?;
        if end > archive.len() {
            return Err(archive_error("the USTAR archive is truncated"));
        }
        let bytes = archive[offset..end].to_vec();
        if sha256(&bytes) != declaration.sha256 {
            return Err(RemoteError::new(
                "ARTIFACT_FILE_DIGEST_MISMATCH",
                "an Artifact file digest does not match its declaration",
            ));
        }
        files.push(PreparedFile { path, mode, bytes });
        let padded = (size as usize)
            .checked_add(511)
            .map(|value| value / 512 * 512)
            .ok_or_else(|| archive_error("a USTAR file size overflowed"))?;
        offset = offset
            .checked_add(padded)
            .ok_or_else(|| archive_error("a USTAR offset overflowed"))?;
        if offset > archive.len() {
            return Err(archive_error("the USTAR archive is truncated"));
        }
    }
    if zero_blocks != 2 || files.len() != declarations.len() {
        return Err(RemoteError::new(
            "ARTIFACT_FILES_MISMATCH",
            "the USTAR files do not match the attestation",
        ));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn tar_path(header: &[u8]) -> Result<String, RemoteError> {
    let name = tar_string(&header[0..100])?;
    let prefix = tar_string(&header[345..500])?;
    let path = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    validate_relative_path(path.trim_end_matches('/'), 1024)?;
    Ok(path.trim_end_matches('/').to_owned())
}

fn tar_string(field: &[u8]) -> Result<String, RemoteError> {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    if field[end..].iter().any(|byte| *byte != 0) {
        return Err(archive_error("a USTAR text field has trailing bytes"));
    }
    std::str::from_utf8(&field[..end])
        .map(str::to_owned)
        .map_err(|_| archive_error("USTAR paths must use UTF-8"))
}

fn parse_tar_octal(field: &[u8]) -> Result<u64, RemoteError> {
    if field.first().is_some_and(|byte| byte & 0x80 != 0) {
        return Err(archive_error("USTAR base-256 numbers are rejected"));
    }
    let value = field
        .iter()
        .copied()
        .take_while(|byte| *byte != 0 && *byte != b' ')
        .filter(|byte| *byte != b' ')
        .collect::<Vec<_>>();
    if value.is_empty() {
        return Ok(0);
    }
    if value.iter().any(|byte| !(b'0'..=b'7').contains(byte)) {
        return Err(archive_error("a USTAR number is invalid"));
    }
    let text =
        std::str::from_utf8(&value).map_err(|_| archive_error("a USTAR number is invalid"))?;
    u64::from_str_radix(text, 8).map_err(|_| archive_error("a USTAR number overflowed"))
}

fn verify_tar_checksum(header: &[u8]) -> Result<(), RemoteError> {
    let expected = parse_tar_octal(&header[148..156])?;
    let actual = header
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                b' '
            } else {
                *byte
            }
        })
        .map(u64::from)
        .sum::<u64>();
    if actual == expected {
        Ok(())
    } else {
        Err(archive_error("the USTAR header checksum is invalid"))
    }
}

fn skill_name_from_files(files: &[PreparedFile]) -> Result<SkillName, RemoteError> {
    let skill = files
        .iter()
        .find(|file| file.path == "SKILL.md")
        .ok_or_else(|| archive_error("the Artifact must contain SKILL.md at its root"))?;
    let text =
        std::str::from_utf8(&skill.bytes).map_err(|_| archive_error("SKILL.md must use UTF-8"))?;
    let name = text
        .strip_prefix("---\n")
        .and_then(|text| text.split_once("\n---"))
        .and_then(|(frontmatter, _)| {
            frontmatter
                .lines()
                .filter_map(|line| line.strip_prefix("name:"))
                .map(str::trim)
                .next()
        })
        .ok_or_else(|| archive_error("SKILL.md must declare its name"))?;
    SkillName::parse(name.trim_matches(['\'', '"']).to_owned())
        .map_err(|error| RemoteError::new(error.code(), error.to_string()))
}

fn installed_digest(files: &[PreparedFile]) -> String {
    let mut files = files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update((file.path.len() as u64).to_be_bytes());
        hasher.update(file.path.as_bytes());
        hasher.update((file.bytes.len() as u64).to_be_bytes());
        hasher.update(&file.bytes);
    }
    hex(&hasher.finalize())
}

fn validate_relative_path(value: &str, maximum: usize) -> Result<(), RemoteError> {
    let path = Path::new(value);
    let valid = !value.is_empty()
        && value.len() <= maximum
        && !value.contains(['\\', ':'])
        && !value.chars().any(is_unsafe_terminal)
        && !path.is_absolute()
        && path.components().all(|component| {
            matches!(component, Component::Normal(_))
                && component.as_os_str().to_str().is_some_and(valid_path_part)
        });
    if valid {
        Ok(())
    } else {
        Err(RemoteError::new(
            "INVALID_PATH",
            "the Skill path must stay inside the Artifact root",
        ))
    }
}

pub(crate) fn is_unsafe_terminal(character: char) -> bool {
    let code = u32::from(character);
    character.is_control()
        || matches!(
            code,
            0x061C | 0x200E..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069
        )
}

fn valid_path_part(part: &str) -> bool {
    if part.is_empty() || part.ends_with(['.', ' ']) {
        return false;
    }
    let stem = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn compare_timestamp(left: &str, right: &str) -> Result<i8, RemoteError> {
    let left = parse_utc_timestamp(left)?;
    let right = parse_utc_timestamp(right)?;
    Ok(left.cmp(&right) as i8)
}

fn parse_utc_timestamp(value: &str) -> Result<OffsetDateTime, RemoteError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| RemoteError::new("INVALID_TIMESTAMP", "a protocol timestamp is invalid"))
}

fn decode_base64url(value: &str, maximum: usize) -> Result<Vec<u8>, RemoteError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| RemoteError::new("INVALID_BASE64URL", "a protocol value is not base64url"))?;
    if decoded.len() > maximum || URL_SAFE_NO_PAD.encode(&decoded) != value {
        return Err(RemoteError::new(
            "INVALID_BASE64URL",
            "a protocol value is not canonical base64url",
        ));
    }
    Ok(decoded)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn archive_error(message: &'static str) -> RemoteError {
    RemoteError::new("INVALID_ARTIFACT_ARCHIVE", message)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteError {
    pub code: &'static str,
    pub message: String,
}

impl RemoteError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for RemoteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RemoteError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_skilld_and_github_selectors() {
        let skilld = RemoteSelector::parse("skilld:skilld-dev/skills/vue-testing").unwrap();
        assert_eq!(skilld.canonical(), "skilld:skilld-dev/skills/vue-testing");
        let exact_skilld = RemoteSelector::parse(
            "skilld:skilld-dev/skills/vue-testing#commit:0123456789abcdef0123456789abcdef01234567",
        )
        .unwrap();
        assert_eq!(
            exact_skilld.canonical(),
            "skilld:skilld-dev/skills/vue-testing#commit:0123456789abcdef0123456789abcdef01234567"
        );
        let github = RemoteSelector::parse(
            "github:skilld-dev/skills/skills/vue-testing#commit:0123456789abcdef0123456789abcdef01234567",
        )
        .unwrap();
        assert!(github.is_explicit_github());
        assert!(matches!(
            github.source().r#ref,
            Some(SourceRef::Commit { .. })
        ));
    }

    #[test]
    fn selector_paths_reject_traversal() {
        let error = RemoteSelector::parse("github:skilld-dev/skills/../secret").unwrap_err();
        assert_eq!(error.code, "INVALID_PATH");
    }

    #[test]
    fn parses_the_exported_search_contract_fixture() {
        let response = parse_search_response(include_bytes!(
            "../../../contracts/fixtures/v1/skill-search.json"
        ))
        .unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(
            response.items[0].selector().unwrap().canonical(),
            "skilld:skilld-dev/skills/vue-testing"
        );
    }

    #[test]
    fn named_skill_selectors_enforce_the_64_character_contract_boundary() {
        let valid = format!("skilld:skilld-dev/skills/{}", "a".repeat(64));
        let invalid = format!("skilld:skilld-dev/skills/{}", "a".repeat(65));

        assert!(RemoteSelector::parse(&valid).is_ok());
        assert_eq!(
            RemoteSelector::parse(&invalid).unwrap_err().code,
            "INVALID_SOURCE"
        );
    }
}
