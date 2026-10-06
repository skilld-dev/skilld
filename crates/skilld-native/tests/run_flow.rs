//! `skilld run` delivery over real HTTP, against a scripted local site.
//!
//! The adapter, the retries, and the archive checks all run as shipped. Only
//! the trusted root key is a test key.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use skilld_command::{NativeRemoteConfig, NoTokenProvider, RemoteProvider, SkilldRemote};
use skilld_core::{RemoteSelector, SourceStatus, TrustedRootPin};
use skilld_native::NativeHttpAdapter;

const RESOLUTION_ID: &str = "018f47a4-2d38-7c5f-8d3e-1c5a6b7d8e9f";

/// One scripted answer: the request line it expects, then the raw response.
struct Answer {
    request: String,
    status: u16,
    body: Vec<u8>,
    /// Send only this many body bytes, then close. The header still declares
    /// the full length, like a connection that drops mid-download.
    cut_at: Option<usize>,
}

fn answer(request: impl Into<String>, status: u16, body: Vec<u8>) -> Answer {
    Answer {
        request: request.into(),
        status,
        body,
        cut_at: None,
    }
}

fn read_request(connection: &mut TcpStream) -> String {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = connection.read(&mut buffer).unwrap();
        received.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&received);
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if received.len() >= end + 4 + length || read == 0 {
                return text.lines().next().unwrap_or_default().to_owned();
            }
        }
        if read == 0 {
            return String::new();
        }
    }
}

/// Serve each answer on its own connection, and return the request lines.
fn serve(listener: TcpListener, answers: Vec<Answer>) -> thread::JoinHandle<Vec<String>> {
    thread::spawn(move || {
        let mut lines = Vec::new();
        for answer in answers {
            let (mut connection, _) = listener.accept().unwrap();
            let line = read_request(&mut connection);
            assert!(
                line.starts_with(&answer.request),
                "expected {}, got {line}",
                answer.request
            );
            lines.push(line);
            write!(
                connection,
                "HTTP/1.1 {} Scripted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                answer.status,
                answer.body.len()
            )
            .unwrap();
            let sent = answer.cut_at.unwrap_or(answer.body.len());
            connection.write_all(&answer.body[..sent]).unwrap();
            connection.flush().unwrap();
        }
        lines
    })
}

fn tar_skill(content: &[u8]) -> Vec<u8> {
    fn octal(field: &mut [u8], value: u64) {
        field.fill(b'0');
        let value = format!("{value:o}");
        let start = field.len() - value.len() - 1;
        field[start..start + value.len()].copy_from_slice(value.as_bytes());
        field[field.len() - 1] = 0;
    }
    let mut header = [0_u8; 512];
    header[..8].copy_from_slice(b"SKILL.md");
    octal(&mut header[100..108], 0o644);
    octal(&mut header[108..116], 0);
    octal(&mut header[116..124], 0);
    octal(&mut header[124..136], content.len() as u64);
    octal(&mut header[136..148], 0);
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum = format!(
        "{:06o}",
        header.iter().map(|byte| u64::from(*byte)).sum::<u64>()
    );
    header[148..154].copy_from_slice(checksum.as_bytes());
    header[154] = 0;
    header[155] = b' ';
    let mut archive = header.to_vec();
    archive.extend_from_slice(content);
    archive.resize(archive.len().div_ceil(512) * 512, 0);
    archive.extend([0_u8; 1024]);
    archive
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn signed(domain: &[u8], statement: &[u8]) -> Vec<u8> {
    let mut message = domain.to_vec();
    message.extend_from_slice(&Sha256::digest(statement));
    message
}

/// The trusted root, the ready Resolution, the grant, and the archive for one
/// CRLF Skill in a mixed-case folder, all signed with test keys. The ready
/// answer carries a field this release does not know.
struct Site {
    pin: TrustedRootPin,
    root: Vec<u8>,
    ready: Vec<u8>,
    grant: Vec<u8>,
    archive: Vec<u8>,
}

fn site(content_url: &str) -> Site {
    let skill = b"---\r\nname: email-and-password\r\ndescription: CRLF frontmatter\r\n---\r\n\r\n# Sign in\r\n";
    let archive = tar_skill(skill);
    let root_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
    let root_public_key = URL_SAFE_NO_PAD.encode(root_key.verifying_key().to_bytes());
    let signing_public_key = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes());
    let key_statement = serde_json::to_vec(&json!({
        "version": 1,
        "rootKeyId": "root-1",
        "keyId": "signer-1",
        "algorithm": "Ed25519",
        "publicKey": signing_public_key,
        "notBefore": "2026-01-01T00:00:00.000Z",
        "notAfter": "2027-01-01T00:00:00.000Z",
        "status": "active"
    }))
    .unwrap();
    let root_signature = root_key.sign(&signed(b"skilld-trusted-key-v1\0", &key_statement));
    let content_sha256 = hex(&Sha256::digest(&archive));
    let artifact_id = format!("sha256:{content_sha256}");
    let source = json!({
        "provider": "github",
        "repositoryId": 1,
        "owner": "better-auth",
        "repository": "skills",
        "visibility": "public",
        "commitSha": "0123456789abcdef0123456789abcdef01234567",
        "treeSha": "89abcdef0123456789abcdef0123456789abcdef",
        "skillPath": "better-auth/emailAndPassword"
    });
    let files = json!([{
        "path": "SKILL.md",
        "mode": 0o644,
        "size": skill.len(),
        "sha256": hex(&Sha256::digest(skill))
    }]);
    let checks = json!([
        {
            "name": "path-policy",
            "version": "1",
            "outcome": "pass",
            "required": true,
            "summary": null,
            "findings": []
        },
        {
            "name": "omitted-files",
            "version": "1",
            "outcome": "warn",
            "required": false,
            "summary": "skilld.dev left out files over the size limits.",
            "findings": ["assets/demo.mp4: 9,311,232 bytes, https://github.com/better-auth/skills/blob/0123456789abcdef0123456789abcdef01234567/better-auth/emailAndPassword/assets/demo.mp4"]
        }
    ]);
    let statement = serde_json::to_vec(&json!({
        "version": 1,
        "artifactId": artifact_id,
        "createdAt": "2026-08-20T00:00:00.000Z",
        "source": source,
        "sourceStatus": "verified",
        "format": "skilld-tar-v1",
        "contentSha256": content_sha256,
        "contentBytes": archive.len(),
        "policyVersion": "2026-08-20",
        "files": files,
        "checkResults": checks
    }))
    .unwrap();
    let signature = signing_key.sign(&signed(b"skilld-attestation-v1\0", &statement));
    let attestation = json!({
        "version": 1,
        "artifactId": artifact_id,
        "createdAt": "2026-08-20T00:00:00.000Z",
        "source": source,
        "sourceStatus": "verified",
        "format": "skilld-tar-v1",
        "contentSha256": content_sha256,
        "contentBytes": archive.len(),
        "policyVersion": "2026-08-20",
        "files": files,
        "checkResults": checks,
        "statement": URL_SAFE_NO_PAD.encode(&statement),
        "signature": {
            "algorithm": "Ed25519",
            "keyId": "signer-1",
            "value": URL_SAFE_NO_PAD.encode(signature.to_bytes())
        }
    });
    Site {
        pin: TrustedRootPin {
            key_id: "root-1".to_owned(),
            public_key: root_public_key.clone(),
        },
        root: serde_json::to_vec(&json!({
            "version": 1,
            "rootKeyId": "root-1",
            "rootPublicKey": root_public_key,
            "keys": [{
                "keyId": "signer-1",
                "algorithm": "Ed25519",
                "publicKey": signing_public_key,
                "notBefore": "2026-01-01T00:00:00.000Z",
                "notAfter": "2027-01-01T00:00:00.000Z",
                "status": "active",
                "statement": URL_SAFE_NO_PAD.encode(&key_statement),
                "rootSignature": URL_SAFE_NO_PAD.encode(root_signature.to_bytes())
            }],
            "fetchedAt": "2026-08-20T00:00:00.000Z"
        }))
        .unwrap(),
        ready: serde_json::to_vec(&json!({
            "state": "ready",
            "resolutionId": RESOLUTION_ID,
            "artifact": {
                "artifactId": artifact_id,
                "visibility": "public",
                "attestation": attestation
            },
            "laterField": "a field this release does not know"
        }))
        .unwrap(),
        grant: serde_json::to_vec(&json!({
            "kind": "public",
            "artifactId": artifact_id,
            "contentUrl": content_url,
            "expiresAt": "2026-08-20T00:05:00.000Z",
            "attestation": attestation
        }))
        .unwrap(),
        archive,
    }
}

#[test]
fn a_run_survives_a_bad_gateway_a_pending_answer_and_a_dropped_download() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let site = site(&format!("http://{address}/content"));
    let pending = serde_json::to_vec(&json!({
        "state": "pending",
        "resolutionId": RESOLUTION_ID,
        "stage": "fetching",
        "pollAfterMs": 250
    }))
    .unwrap();
    let mut dropped = answer("GET /content", 200, site.archive.clone());
    dropped.cut_at = Some(700);
    let server = serve(
        listener,
        vec![
            answer(
                "POST /api/v1/resolutions",
                502,
                b"<html>Bad gateway</html>".to_vec(),
            ),
            answer("POST /api/v1/resolutions", 202, pending),
            answer(
                format!("GET /api/v1/resolutions/{RESOLUTION_ID}"),
                200,
                site.ready.clone(),
            ),
            answer("GET /api/v1/trusted-root", 200, site.root.clone()),
            answer("POST /api/v1/artifacts/sha256%3A", 200, site.grant.clone()),
            dropped,
            answer("GET /content", 200, site.archive.clone()),
        ],
    );
    let remote = SkilldRemote::new(
        Arc::new(NativeHttpAdapter::new()),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Pinned(site.pin),
    )
    .with_endpoint(&format!("http://{address}"))
    .unwrap();
    let selector = RemoteSelector::parse("better-auth/skills/emailandpassword").unwrap();

    let prepared = remote.prepare(&selector, false);

    let lines = server.join().unwrap();
    let prepared = prepared.unwrap();
    assert_eq!(lines.len(), 7);
    assert!(matches!(
        prepared.source_status,
        SourceStatus::Verified { .. }
    ));
    assert_eq!(prepared.skill_name().unwrap().as_str(), "emailandpassword");
    assert_eq!(prepared.omitted_files.len(), 1);
    assert_eq!(prepared.omitted_files[0].path, "assets/demo.mp4");
    assert_eq!(prepared.omitted_files[0].bytes, Some(9_311_232));
}
