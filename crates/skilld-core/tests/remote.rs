use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use skilld_core::{
    ArtifactAttestation, ArtifactFile, AttestationSignature, CheckOutcome, CheckResult, LinkedFile,
    PreparedFile, RemoteSelector, RepositoryVisibility, ResolvedSource, SignatureAlgorithm,
    SourceProvider, TrustedKey, TrustedKeyStatus, TrustedRoot, TrustedRootPin, declared_skill_name,
    prepare_unverified_files, skill_identity, verify_artifact, verify_linked_file,
    verify_trusted_root, with_linked_files,
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

#[test]
fn a_v2_bare_package_name_points_to_skill_search() {
    let error = RemoteSelector::parse("vue").unwrap_err();

    assert_eq!(error.code, "INVALID_SOURCE");
    assert!(
        error.message.contains("skilld search vue"),
        "{}",
        error.message
    );
}

#[test]
fn public_remote_selectors_reject_bidi_formatting_characters() {
    for selector in [
        "github:skilld-dev/skills/skills/\u{202e}example",
        "github:skilld-dev/skills/skills/example#branch:main\u{2067}forged",
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
    #[serde(skip_serializing_if = "<[LinkedFile]>::is_empty")]
    linked_files: &'a [LinkedFile],
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
    attestation_with_linked(archive, files, signing_key, skill_path, vec![])
}

fn attestation_with_linked(
    archive: &[u8],
    files: Vec<ArtifactFile>,
    signing_key: &SigningKey,
    skill_path: &str,
    linked_files: Vec<LinkedFile>,
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
        linked_files: &linked_files,
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
        linked_files,
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
fn verifies_a_skill_at_the_repository_root() {
    let skill = b"---\nname: skills\ndescription: fixture\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    let verified = verify_artifact(
        attestation_at_path(
            &archive,
            vec![file("SKILL.md", 0o644, skill)],
            &signing_key,
            ".",
        ),
        &root,
        &archive,
    )
    .unwrap();

    assert_eq!(verified.name.as_str(), "skills");
    assert_eq!(verified.files[0].bytes, skill);
    assert_eq!(verified.attestation.source.skill_path, ".");
}

#[test]
fn a_verified_skill_takes_its_name_from_the_attested_folder_not_the_frontmatter() {
    let skill = b"---\nname: design-taste-frontend\ndescription: fixture\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    let verified = verify_artifact(
        attestation_at_path(
            &archive,
            vec![file("SKILL.md", 0o644, skill)],
            &signing_key,
            "skills/taste-skill",
        ),
        &root,
        &archive,
    )
    .unwrap();

    assert_eq!(verified.name.as_str(), "taste-skill");
}

#[test]
fn a_root_skill_takes_its_name_from_the_repository_without_case() {
    assert_eq!(
        skill_identity("Frontend-Slides", ".", None)
            .unwrap()
            .as_str(),
        "frontend-slides"
    );
}

#[test]
fn a_mixed_case_folder_takes_its_lowercase_name_like_the_registry() {
    // better-auth/skills admits better-auth/emailAndPassword as emailandpassword.
    assert_eq!(
        skill_identity(
            "skills",
            "better-auth/emailAndPassword",
            Some("email-and-password")
        )
        .unwrap()
        .as_str(),
        "emailandpassword"
    );
}

#[test]
fn a_folder_that_cannot_be_a_skill_name_falls_back_to_the_declared_name() {
    assert_eq!(
        skill_identity("skills", "skills/My_Skill", Some("my-skill"))
            .unwrap()
            .as_str(),
        "my-skill"
    );
    let error = skill_identity("skills", "skills/My_Skill", None).unwrap_err();
    assert_eq!(error.code, "INVALID_SOURCE");
    assert!(error.message.contains("My_Skill"), "{}", error.message);
}

#[test]
fn a_verified_skill_with_crlf_frontmatter_and_a_bom_verifies() {
    let skill = "\u{feff}---\r\nname: awwwards-sections\r\ndescription: fixture\r\n---\r\n\r\n# Sections\r\n";
    let archive = archive(&[("SKILL.md", 0o644, skill.as_bytes(), b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    let verified = verify_artifact(
        attestation_at_path(
            &archive,
            vec![file("SKILL.md", 0o644, skill.as_bytes())],
            &signing_key,
            "skills/awwwards-sections",
        ),
        &root,
        &archive,
    )
    .unwrap();

    assert_eq!(verified.name.as_str(), "awwwards-sections");
}

#[test]
fn the_declared_name_reads_crlf_lines_a_bom_and_quotes() {
    assert_eq!(
        declared_skill_name(
            "\u{feff}---\r\nname: design-style\r\ndescription: |\r\n  x\r\n---\r\n"
        ),
        Some("design-style".to_owned())
    );
    assert_eq!(
        declared_skill_name("---\nname: \"quoted-name\"\n---\n"),
        Some("quoted-name".to_owned())
    );
    assert_eq!(declared_skill_name("---\ndescription: x\n---\n"), None);
    assert_eq!(declared_skill_name("# No frontmatter\nname: body\n"), None);
    assert_eq!(declared_skill_name("---\nname: unclosed\n"), None);
}

#[test]
fn rejects_root_aliases_and_traversal_in_attested_source_paths() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    for path in ["", "./", "..", "../skill", "/", "./skill"] {
        let error = verify_artifact(
            attestation_at_path(
                &archive,
                vec![file("SKILL.md", 0o644, skill)],
                &signing_key,
                path,
            ),
            &root,
            &archive,
        )
        .unwrap_err();
        assert_eq!(error.code, "INVALID_PATH", "{path}");
    }
}

#[test]
fn rejects_a_repository_root_marker_as_an_artifact_file() {
    let error = prepare_unverified_files(vec![PreparedFile {
        path: ".".to_owned(),
        mode: 0o644,
        bytes: b"---\nname: example\n---\n".to_vec(),
    }])
    .unwrap_err();

    assert_eq!(error.code, "INVALID_PATH");
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
fn unverified_files_reject_paths_that_differ_only_by_case() {
    let result = prepare_unverified_files(vec![
        PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes: b"---\nname: example\n---\n".to_vec(),
        },
        PreparedFile {
            path: "skill.md".to_owned(),
            mode: 0o644,
            bytes: b"different instructions".to_vec(),
        },
    ]);
    assert_eq!(result.unwrap_err().code, "INVALID_ARTIFACT_ARCHIVE");
}

#[test]
fn unverified_files_reject_windows_reserved_names_and_characters() {
    for path in [
        "references/api<draft>.md",
        "references/api|draft.md",
        "references/api\"draft.md",
        "references/api?.md",
        "references/api*.md",
        "references/COM¹.md",
        "references/com².md",
        "references/LPT³.md",
    ] {
        let result = prepare_unverified_files(vec![
            PreparedFile {
                path: "SKILL.md".to_owned(),
                mode: 0o644,
                bytes: b"---\nname: example\n---\n".to_vec(),
            },
            PreparedFile {
                path: path.to_owned(),
                mode: 0o644,
                bytes: b"reference".to_vec(),
            },
        ]);
        assert_eq!(result.unwrap_err().code, "INVALID_PATH", "{path}");
    }
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
fn unverified_files_reject_bidi_formatting_characters_in_paths() {
    let files = vec![
        PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes: b"---\nname: example\n---\n".to_vec(),
        },
        PreparedFile {
            path: "references/api\u{202e}forged.md".to_owned(),
            mode: 0o644,
            bytes: b"# API\n".to_vec(),
        },
    ];

    let error = prepare_unverified_files(files).unwrap_err();

    assert_eq!(error.code, "INVALID_PATH");
}

#[test]
fn verified_artifacts_reject_bidi_formatting_in_source_and_file_paths() {
    let skill = b"---\nname: example\n---\n";
    let supporting = b"# API\n";
    let valid_archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let bidi_archive = archive(&[
        ("SKILL.md", 0o644, skill, b'0'),
        ("references/api\u{202e}forged.md", 0o644, supporting, b'0'),
    ]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    let source_error = verify_artifact(
        attestation_at_path(
            &valid_archive,
            vec![file("SKILL.md", 0o644, skill)],
            &signing_key,
            "skills/\u{202e}example",
        ),
        &root,
        &valid_archive,
    )
    .unwrap_err();
    let file_error = verify_artifact(
        attestation(
            &bidi_archive,
            vec![
                file("SKILL.md", 0o644, skill),
                file("references/api\u{202e}forged.md", 0o644, supporting),
            ],
            &signing_key,
        ),
        &root,
        &bidi_archive,
    )
    .unwrap_err();

    assert_eq!(source_error.code, "INVALID_PATH");
    assert_eq!(file_error.code, "INVALID_PATH");
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

fn linked(path: &str, bytes: &[u8]) -> LinkedFile {
    LinkedFile {
        path: path.to_owned(),
        mode: 0o644,
        size: bytes.len() as u64,
        git_blob_sha: git_blob_sha(bytes),
    }
}

fn git_blob_sha(bytes: &[u8]) -> String {
    use sha1::{Digest as _, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex(&hasher.finalize())
}

#[test]
fn installs_a_linked_file_beside_the_archive_files_once_its_git_blob_matches() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let binary = b"\x7fELF a binary too large to pack";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();
    let attestation = attestation_with_linked(
        &archive,
        vec![file("SKILL.md", 0o644, skill)],
        &signing_key,
        "skills/example",
        vec![linked("scripts/tool", binary)],
    );

    let verified = verify_artifact(attestation, &root, &archive).unwrap();
    let declaration = verified.attestation.linked_files[0].clone();
    let packed_only = verified.installed_sha256.clone();
    let tool = verify_linked_file(&declaration, binary.to_vec()).unwrap();
    let installed = with_linked_files(verified, vec![tool]).unwrap();

    assert_eq!(
        installed
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["SKILL.md", "scripts/tool"]
    );
    assert_ne!(installed.installed_sha256, packed_only);
}

#[test]
fn refuses_a_linked_file_whose_bytes_fail_its_git_blob_sha() {
    let declaration = linked("scripts/tool", b"the attested bytes");

    let other = verify_linked_file(&declaration, b"the attested bytez".to_vec()).unwrap_err();
    let short = verify_linked_file(&declaration, b"short".to_vec()).unwrap_err();

    assert_eq!(other.code, "LINKED_FILE_DIGEST_MISMATCH");
    assert_eq!(short.code, "LINKED_FILE_SIZE_MISMATCH");
}

#[test]
fn refuses_to_install_until_every_linked_file_is_read() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();
    let attestation = attestation_with_linked(
        &archive,
        vec![file("SKILL.md", 0o644, skill)],
        &signing_key,
        "skills/example",
        vec![linked("assets/a.mp3", b"a"), linked("assets/b.mp3", b"b")],
    );
    let verified = verify_artifact(attestation, &root, &archive).unwrap();
    let first = verify_linked_file(&verified.attestation.linked_files[0], b"a".to_vec()).unwrap();

    let error = with_linked_files(verified, vec![first]).unwrap_err();

    assert_eq!(error.code, "LINKED_FILES_MISMATCH");
}

#[test]
fn rejects_a_linked_file_that_shares_a_path_with_a_packed_file() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();

    for path in ["SKILL.md", "skill.md", "SKILL.md/inner"] {
        let attestation = attestation_with_linked(
            &archive,
            vec![file("SKILL.md", 0o644, skill)],
            &signing_key,
            "skills/example",
            vec![linked(path, b"x")],
        );
        let error = verify_artifact(attestation, &root, &archive).unwrap_err();
        assert_eq!(error.code, "ATTESTATION_INVALID", "{path}");
    }
}

#[test]
fn rejects_linked_files_past_the_byte_limit() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (root, pin, signing_key) = trusted_root();
    let root = verify_trusted_root(root, &pin).unwrap();
    let mut huge = linked("assets/film.mp4", b"x");
    huge.size = 256 * 1024 * 1024 + 1;
    let attestation = attestation_with_linked(
        &archive,
        vec![file("SKILL.md", 0o644, skill)],
        &signing_key,
        "skills/example",
        vec![huge],
    );

    let error = verify_artifact(attestation, &root, &archive).unwrap_err();

    assert_eq!(error.code, "ATTESTATION_INVALID");
}

#[test]
fn an_attestation_without_linked_files_serializes_without_the_field() {
    let skill = b"---\nname: example\ndescription: fixture\n---\n";
    let archive = archive(&[("SKILL.md", 0o644, skill, b'0')]);
    let (_, _, signing_key) = trusted_root();
    let attestation = attestation(&archive, vec![file("SKILL.md", 0o644, skill)], &signing_key);

    let value = serde_json::to_value(&attestation).unwrap();

    assert!(value.get("linkedFiles").is_none());
}
