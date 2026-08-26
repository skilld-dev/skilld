use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use skilld_core::{
    ArtifactAttestation, ArtifactFile, AttestationSignature, CheckOutcome, CheckResult,
    PreparedFile, RemoteSelector, RepositoryVisibility, ResolvedSource, SignatureAlgorithm,
    SourceProvider, TrustedKey, TrustedKeyStatus, TrustedRoot, TrustedRootPin,
    prepare_unverified_files, verify_artifact, verify_trusted_root,
};

const ROOT_DOMAIN: &[u8] = b"skilld-trusted-key-v1\0";
const ATTESTATION_DOMAIN: &[u8] = b"skilld-attestation-v1\0";

#[test]
fn public_remote_selectors_reject_control_characters_in_branch_and_tag_refs() {
    for selector in [
        "github:skilld-dev/skills/skills/example#branch:main\nforged",
        "github:skilld-dev/skills/skills/example#tag:v1\tforged",
        "github:skilld-dev/skills/skills/example#branch:main\u{0085}forged",
        "github:skilld-dev/skills/skills/example#tag:v1\n",
    ] {
        let error = RemoteSelector::parse(selector).unwrap_err();

        assert_eq!(error.code, "INVALID_SOURCE");
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyStatement<'a> {
    version: u8,
    root_key_id: &'a str,
    key_id: &'a str,
    algorithm: &'a str,
    public_key: &'a str,
    not_before: &'a str,
    not_after: &'a str,
    status: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactStatement<'a> {
    version: u8,
    artifact_id: &'a str,
    created_at: &'a str,
    source: &'a ResolvedSource,
    source_status: &'a str,
    format: &'a str,
    content_sha256: &'a str,
    content_bytes: u64,
    policy_version: &'a str,
    files: &'a [ArtifactFile],
    check_results: &'a [CheckResult],
}

fn signed_message(domain: &[u8], statement: &[u8]) -> Vec<u8> {
    let mut message = domain.to_vec();
    message.extend_from_slice(&Sha256::digest(statement));
    message
}

fn trusted_root() -> (TrustedRoot, TrustedRootPin, SigningKey) {
    let root_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
    let root_public_key = URL_SAFE_NO_PAD.encode(root_key.verifying_key().to_bytes());
    let public_key = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes());
    let statement = serde_json::to_vec(&KeyStatement {
        version: 1,
        root_key_id: "root-1",
        key_id: "signer-1",
        algorithm: "Ed25519",
        public_key: &public_key,
        not_before: "2026-01-01T00:00:00.000Z",
        not_after: "2027-01-01T00:00:00.000Z",
        status: "active",
    })
    .unwrap();
    let root_signature = root_key.sign(&signed_message(ROOT_DOMAIN, &statement));
    (
        TrustedRoot {
            version: 1,
            root_key_id: "root-1".to_owned(),
            root_public_key: root_public_key.clone(),
            keys: vec![TrustedKey {
                key_id: "signer-1".to_owned(),
                algorithm: SignatureAlgorithm::Ed25519,
                public_key,
                not_before: "2026-01-01T00:00:00.000Z".to_owned(),
                not_after: "2027-01-01T00:00:00.000Z".to_owned(),
                status: TrustedKeyStatus::Active,
                statement: URL_SAFE_NO_PAD.encode(statement),
                root_signature: URL_SAFE_NO_PAD.encode(root_signature.to_bytes()),
            }],
            fetched_at: "2026-08-20T00:00:00.000Z".to_owned(),
        },
        TrustedRootPin {
            key_id: "root-1".to_owned(),
            public_key: root_public_key,
        },
        signing_key,
    )
}

fn tar_entry(path: &str, mode: u32, content: &[u8], entry_type: u8) -> Vec<u8> {
    let mut header = [0_u8; 512];
    header[..path.len()].copy_from_slice(path.as_bytes());
    write_octal(&mut header[100..108], mode as u64);
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    write_octal(&mut header[124..136], content.len() as u64);
    write_octal(&mut header[136..148], 0);
    header[148..156].fill(b' ');
    header[156] = entry_type;
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum = header.iter().map(|byte| u64::from(*byte)).sum();
    write_checksum(&mut header[148..156], checksum);
    let mut output = header.to_vec();
    output.extend_from_slice(content);
    output.resize(output.len().div_ceil(512) * 512, 0);
    output
}

fn archive(entries: &[(&str, u32, &[u8], u8)]) -> Vec<u8> {
    let mut archive = Vec::new();
    for (path, mode, content, entry_type) in entries {
        archive.extend(tar_entry(path, *mode, content, *entry_type));
    }
    archive.extend([0_u8; 1024]);
    archive
}

fn write_octal(field: &mut [u8], value: u64) {
    field.fill(b'0');
    let value = format!("{value:o}");
    let start = field.len() - value.len() - 1;
    field[start..start + value.len()].copy_from_slice(value.as_bytes());
    field[field.len() - 1] = 0;
}

fn write_checksum(field: &mut [u8], value: u64) {
    let value = format!("{value:06o}");
    field[..6].copy_from_slice(value.as_bytes());
    field[6] = 0;
    field[7] = b' ';
}

fn attestation(
    archive: &[u8],
    files: Vec<ArtifactFile>,
    signing_key: &SigningKey,
) -> ArtifactAttestation {
    attestation_at_path(archive, files, signing_key, "skills/example")
}

fn attestation_at_path(
    archive: &[u8],
    files: Vec<ArtifactFile>,
    signing_key: &SigningKey,
    skill_path: &str,
) -> ArtifactAttestation {
    let content_sha256 = hex(&Sha256::digest(archive));
    let artifact_id = format!("sha256:{content_sha256}");
    let source = ResolvedSource {
        provider: SourceProvider::Github,
        repository_id: 1,
        owner: "skilld-dev".to_owned(),
        repository: "skills".to_owned(),
        visibility: RepositoryVisibility::Public,
        commit_sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        tree_sha: "89abcdef0123456789abcdef0123456789abcdef".to_owned(),
        skill_path: skill_path.to_owned(),
    };
    let checks = vec![CheckResult {
        name: "path-policy".to_owned(),
        version: "1".to_owned(),
        outcome: CheckOutcome::Pass,
        required: true,
        summary: None,
        findings: vec![],
    }];
    let statement = serde_json::to_vec(&ArtifactStatement {
        version: 1,
        artifact_id: &artifact_id,
        created_at: "2026-08-20T00:00:00.000Z",
        source: &source,
        source_status: "verified",
        format: "skilld-tar-v1",
        content_sha256: &content_sha256,
        content_bytes: archive.len() as u64,
        policy_version: "2026-08-20",
        files: &files,
        check_results: &checks,
    })
    .unwrap();
    let signature = signing_key.sign(&signed_message(ATTESTATION_DOMAIN, &statement));
    ArtifactAttestation {
        version: 1,
        artifact_id,
        created_at: "2026-08-20T00:00:00.000Z".to_owned(),
        source,
        source_status: "verified".to_owned(),
        format: "skilld-tar-v1".to_owned(),
        content_sha256,
        content_bytes: archive.len() as u64,
        policy_version: "2026-08-20".to_owned(),
        files,
        check_results: checks,
        statement: URL_SAFE_NO_PAD.encode(statement),
        signature: AttestationSignature {
            algorithm: SignatureAlgorithm::Ed25519,
            key_id: "signer-1".to_owned(),
            value: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        },
    }
}

fn file(path: &str, mode: u32, bytes: &[u8]) -> ArtifactFile {
    ArtifactFile {
        path: path.to_owned(),
        mode,
        size: bytes.len() as u64,
        sha256: hex(&Sha256::digest(bytes)),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn verifies_exact_statements_root_signatures_and_ustar_files() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    let verified = verify_artifact(
        attestation(&archive, vec![file("SKILL.md", 0o644, skill)], &signing_key),
        &root,
        &archive,
    )
    .unwrap();

    assert_eq!(verified.name.as_str(), "example");
    assert_eq!(verified.files[0].bytes, skill);
}

#[test]
fn rejects_a_hash_in_the_attested_skill_path_but_allows_it_in_supporting_files() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let supporting = b"# Fragment\n";
    let archive = archive(&[
        ("SKILL.md", 0o644, skill, b'0'),
        ("references/topic#part.md", 0o644, supporting, b'0'),
    ]);
    let files = vec![
        file("SKILL.md", 0o644, skill),
        file("references/topic#part.md", 0o644, supporting),
    ];
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    let valid = verify_artifact(
        attestation(&archive, files.clone(), &signing_key),
        &root,
        &archive,
    )
    .unwrap();
    let error = verify_artifact(
        attestation_at_path(&archive, files, &signing_key, "skills/example#archive"),
        &root,
        &archive,
    )
    .unwrap_err();

    assert_eq!(valid.files[1].path, "references/topic#part.md");
    assert_eq!(error.code, "INVALID_SOURCE");
}

#[test]
fn unverified_files_reject_c1_control_characters_in_paths() {
    let files = vec![
        PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes: b"---\nname: example\n---\n".to_vec(),
        },
        PreparedFile {
            path: "references/api\u{0085}forged.md".to_owned(),
            mode: 0o644,
            bytes: b"# API\n".to_vec(),
        },
    ];

    let error = prepare_unverified_files(files).unwrap_err();

    assert_eq!(error.code, "INVALID_PATH");
}

#[test]
fn rejects_an_outer_field_that_differs_from_the_signed_statement() {
    let skill = b"---\nname: example\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();
    let mut attestation = attestation(&archive, vec![file("SKILL.md", 0o644, skill)], &signing_key);
    attestation.policy_version = "changed".to_owned();

    let error = verify_artifact(attestation, &root, &archive).unwrap_err();

    assert_eq!(error.code, "ATTESTATION_MISMATCH");
}

#[test]
fn rejects_ustar_links_and_undeclared_files() {
    let skill = b"---\nname: example\n---\n";
    let linked = archive(&[("SKILL.md", 0o644, skill, b'2')]);
    let extra = archive(&[
        ("SKILL.md", 0o644, skill, b'0'),
        ("secret", 0o644, b"secret", b'0'),
    ]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    let link_error = verify_artifact(
        attestation(&linked, vec![file("SKILL.md", 0o644, skill)], &signing_key),
        &root,
        &linked,
    )
    .unwrap_err();
    let extra_error = verify_artifact(
        attestation(&extra, vec![file("SKILL.md", 0o644, skill)], &signing_key),
        &root,
        &extra,
    )
    .unwrap_err();

    assert_eq!(link_error.code, "INVALID_ARTIFACT_ARCHIVE");
    assert_eq!(extra_error.code, "UNDECLARED_ARTIFACT_FILE");
}

#[test]
fn rejects_duplicate_traversal_device_and_sparse_ustar_entries() {
    let skill = b"---\nname: example\n---\n";
    let duplicate = archive(&[
        ("SKILL.md", 0o644, skill, b'0'),
        ("SKILL.md", 0o644, skill, b'0'),
    ]);
    let traversal = archive(&[("../SKILL.md", 0o644, skill, b'0')]);
    let device = archive(&[("SKILL.md", 0o644, skill, b'3')]);
    let sparse = archive(&[("SKILL.md", 0o644, skill, b'S')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();
    let declaration = vec![file("SKILL.md", 0o644, skill)];

    for archive in [&duplicate, &device, &sparse] {
        let error = verify_artifact(
            attestation(archive, declaration.clone(), &signing_key),
            &root,
            archive,
        )
        .unwrap_err();
        assert_eq!(error.code, "INVALID_ARTIFACT_ARCHIVE");
    }
    let error = verify_artifact(
        attestation(
            &traversal,
            vec![file("../SKILL.md", 0o644, skill)],
            &signing_key,
        ),
        &root,
        &traversal,
    )
    .unwrap_err();
    assert_eq!(error.code, "INVALID_PATH");
}

#[test]
fn rejects_a_root_that_differs_from_the_compile_time_pin() {
    let (root, mut pin, _) = trusted_root();
    pin.key_id = "other-root".to_owned();

    let error = verify_trusted_root(root, &pin).unwrap_err();

    assert_eq!(error.code, "TRUSTED_ROOT_MISMATCH");
}
