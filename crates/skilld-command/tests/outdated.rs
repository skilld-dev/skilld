use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use skilld_command::{
    Host, LocalHost, PreparedRemoteSkill, RemoteProvider, RemoteSourceState, run,
};
use skilld_core::{
    AgentTargetId, InstallMode, InstallOperation, InstallRequest, InstallScope, InstallSource,
    LockedSource, PreparedFile, RemoteError, RemoteSelector, SearchResult, SourceProvider,
    SourceRequest, SourceSelector, SourceStatus,
};

struct Provider {
    content: Mutex<Vec<u8>>,
    stale: Mutex<bool>,
    fail_state: Mutex<bool>,
    search_results: Mutex<Vec<SearchResult>>,
    fail_search: Mutex<bool>,
}

impl Provider {
    fn new(content: &str) -> Self {
        Self {
            content: Mutex::new(content.as_bytes().to_vec()),
            stale: Mutex::new(false),
            fail_state: Mutex::new(false),
            search_results: Mutex::new(vec![]),
            fail_search: Mutex::new(false),
        }
    }

    fn search_result(name: &str) -> SearchResult {
        SearchResult {
            name: name.to_owned(),
            description: None,
            source: SourceRequest {
                provider: SourceProvider::Github,
                owner: "acme".to_owned(),
                repository: "skills".to_owned(),
                selector: SourceSelector::NamedSkill {
                    name: name.to_owned(),
                },
                r#ref: None,
            },
            stargazer_count: 0,
        }
    }
}

fn installed_digest(file: &PreparedFile) -> String {
    let mut hasher = Sha256::new();
    hasher.update((file.path.len() as u64).to_be_bytes());
    hasher.update(file.path.as_bytes());
    hasher.update((file.bytes.len() as u64).to_be_bytes());
    hasher.update(&file.bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl RemoteProvider for Provider {
    fn search(&self, _query: &str, _limit: u8) -> Result<Vec<SearchResult>, RemoteError> {
        if *self.fail_search.lock().unwrap() {
            return Err(RemoteError::new(
                "INVALID_RESPONSE",
                "Skill search returned invalid JSON",
            ));
        }
        Ok(self.search_results.lock().unwrap().clone())
    }

    fn prepare(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        let bytes = self.content.lock().unwrap().clone();
        let file = PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes,
        };
        let digest = installed_digest(&file);
        Ok(PreparedRemoteSkill {
            files: vec![file],
            locked_source: LockedSource::Remote {
                source: selector.canonical(),
                commit_sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                skill_path: "skills/example".to_owned(),
            },
            source_status: if direct {
                SourceStatus::Unverified {
                    content_sha256: digest.clone(),
                    installed_sha256: digest,
                }
            } else {
                SourceStatus::Verified {
                    artifact_id: format!("sha256:{digest}"),
                    content_sha256: digest.clone(),
                    installed_sha256: digest,
                    attestation_key_id: "test-key".to_owned(),
                }
            },
        })
    }

    fn source_state(
        &self,
        _selector: &RemoteSelector,
        _artifact_id: &str,
        _commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError> {
        if *self.fail_state.lock().unwrap() {
            return Err(RemoteError::new(
                "SERVICE_UNAVAILABLE",
                "the remote service returned HTTP 503",
            ));
        }
        Ok(if *self.stale.lock().unwrap() {
            RemoteSourceState::Stale {
                current_artifact_id: "sha256:new".to_owned(),
                current_commit_sha: "ffffffffffffffffffffffffffffffffffffffff".to_owned(),
            }
        } else {
            RemoteSourceState::Current
        })
    }
}

fn install_project(host: &LocalHost, selector: &str) {
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(selector.to_owned())),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
    })
    .unwrap();
}

fn install_global(host: &LocalHost, selector: &str) {
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::Remote(selector.to_owned())),
        scope: InstallScope::Global,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
    })
    .unwrap();
}

fn unmanaged_skill(home: &Path, agent_dir: &str, name: &str) {
    let directory = home.join(agent_dir).join("skills").join(name);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: unmanaged\n---\n"),
    )
    .unwrap();
}

#[test]
fn outdated_reports_current_and_stale_project_skills() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(Provider::new(
        "---\nname: example\ndescription: first\n---\n",
    ));
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    install_project(&host, "skilld:skilld-dev/skills/example");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let current = run(["skilld", "outdated"], &host, &mut stdout, &mut stderr);

    assert_eq!(current.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout.clone()).unwrap(),
        "Current Skill example.\n"
    );
    *provider.stale.lock().unwrap() = true;
    stdout.clear();
    stderr.clear();

    let outdated = run(["skilld", "outdated"], &host, &mut stdout, &mut stderr);

    assert_eq!(outdated.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Outdated Skill example. Run skilld upgrade example.\n"
    );
}

#[test]
fn outdated_system_reports_a_stale_global_skill_with_the_global_upgrade() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(Provider::new(
        "---\nname: example\ndescription: first\n---\n",
    ));
    let host = LocalHost::new(project, temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    install_global(&host, "skilld:skilld-dev/skills/example");
    *provider.stale.lock().unwrap() = true;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "outdated", "--system"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Outdated Skill example. Run skilld upgrade example --global.\n"
    );
}

#[test]
fn outdated_system_links_unmanaged_skills_to_a_repository() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(Provider::new("---\nname: example\n---\n"));
    *provider.search_results.lock().unwrap() = vec![Provider::search_result("vue-testing")];
    let home = temporary.path();
    unmanaged_skill(home, ".claude", "vue-testing");
    let host = LocalHost::new(project, home.join("data")).with_remote_provider(provider);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "outdated", "--system"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    let output = String::from_utf8(stdout).unwrap();
    let expected = format!(
        "Unmanaged Skill vue-testing (claude-code). Candidate source skilld:acme/skills/vue-testing, 0 stars.\nDelete {}, then run skilld install skilld:acme/skills/vue-testing --global --agent claude-code.\n",
        home.join(".claude/skills/vue-testing").display()
    );
    assert_eq!(output, expected);
}

#[test]
fn outdated_system_reports_unmanaged_skills_without_a_match() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(Provider::new("---\nname: example\n---\n"));
    *provider.search_results.lock().unwrap() = vec![Provider::search_result("vue-testing")];
    let home = temporary.path();
    unmanaged_skill(home, ".agents", "private-skill");
    let host = LocalHost::new(project, home.join("data")).with_remote_provider(provider);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "outdated", "--system"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Unmanaged Skill private-skill (codex). No Repository match found.\n"
    );
}

#[test]
fn outdated_system_surfaces_a_search_failure_and_keeps_scanning() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(Provider::new("---\nname: example\n---\n"));
    *provider.search_results.lock().unwrap() = vec![Provider::search_result("vue-testing")];
    *provider.fail_search.lock().unwrap() = true;
    let home = temporary.path();
    unmanaged_skill(home, ".claude", "vue-testing");
    unmanaged_skill(home, ".agents", "other-skill");
    let host = LocalHost::new(project, home.join("data")).with_remote_provider(provider);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "outdated", "--system"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Unmanaged Skill other-skill (codex). Skill search unavailable: Skill search returned invalid JSON.\nUnmanaged Skill vue-testing (claude-code). Skill search unavailable: Skill search returned invalid JSON.\n"
    );
}

#[test]
fn outdated_system_reports_a_managed_skill_once() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(Provider::new(
        "---\nname: example\ndescription: first\n---\n",
    ));
    let host = LocalHost::new(project.clone(), temporary.path().join("data"))
        .with_remote_provider(provider);
    install_project(&host, "skilld:skilld-dev/skills/example");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "outdated", "--system"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Current Skill example.\n"
    );
    assert!(project.join(".skills/example/SKILL.md").exists());
    assert!(project.join(".agents/skills/example/SKILL.md").exists());
}

#[test]
fn outdated_without_installed_skills_reports_none() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project, temporary.path().join("data"));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(["skilld", "outdated"], &host, &mut stdout, &mut stderr);

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "No installed Skills found.\n"
    );
}

#[test]
fn upgrade_global_upgrades_a_global_skill() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let data = temporary.path().join("data");
    let provider = Arc::new(Provider::new(
        "---\nname: example\ndescription: first\n---\n",
    ));
    let host = LocalHost::new(project, data.clone()).with_remote_provider(provider.clone());
    install_global(&host, "skilld:skilld-dev/skills/example");
    *provider.content.lock().unwrap() = b"---\nname: example\ndescription: second\n---\n".to_vec();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "upgrade", "example", "--global"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Upgraded Skill example.\n"
    );
    assert_eq!(
        fs::read_to_string(data.join("skills/example/SKILL.md")).unwrap(),
        "---\nname: example\ndescription: second\n---\n"
    );
}

#[test]
fn outdated_survives_a_source_state_failure() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(Provider::new(
        "---\nname: example\ndescription: first\n---\n",
    ));
    *provider.fail_state.lock().unwrap() = true;
    let host = LocalHost::new(project, temporary.path().join("data"))
        .with_remote_provider(provider.clone());
    install_project(&host, "skilld:skilld-dev/skills/example");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(["skilld", "outdated"], &host, &mut stdout, &mut stderr);

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Source state unavailable for Skill example: the remote service returned HTTP 503.\n"
    );
}

#[test]
fn outdated_system_survives_a_corrupt_global_store() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let home = temporary.path();
    let data = home.join("data");
    fs::create_dir_all(data.join("skills")).unwrap();
    fs::write(data.join("skills/skilld-lock.yaml"), "not json").unwrap();
    unmanaged_skill(&project, ".agents", "unmanaged-project");
    unmanaged_skill(home, ".claude", "hidden-global");
    let provider = Arc::new(Provider::new("---\nname: example\n---\n"));
    let host = LocalHost::new(project, data.clone()).with_remote_provider(provider);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "outdated", "--system"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    let output = String::from_utf8(stdout).unwrap();
    assert!(
        output.starts_with("Skill store unavailable in global scope: "),
        "expected a store failure line, got: {output}"
    );
    assert!(
        output.contains(
            "Unmanaged Skill unmanaged-project (amp, codex). No Repository match found.\n"
        )
    );
    assert!(
        !output.contains("hidden-global"),
        "a scope with an unreadable lockfile must not report its Skills: {output}"
    );
}

#[test]
fn outdated_system_groups_agents_sharing_one_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let home = temporary.path();
    unmanaged_skill(&project, ".agents", "shared");
    let provider = Arc::new(Provider::new("---\nname: example\n---\n"));
    let host = LocalHost::new(project, home.join("data")).with_remote_provider(provider);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "outdated", "--system"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    let output = String::from_utf8(stdout).unwrap();
    assert!(
        output.contains("Unmanaged Skill shared (amp, codex). No Repository match found.\n"),
        "expected both agents sharing .agents/skills, got: {output}"
    );
}

#[test]
fn outdated_gives_the_direct_recovery_for_unverified_skills() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let provider = Arc::new(Provider::new(
        "---\nname: example\ndescription: direct\n---\n",
    ));
    let host =
        LocalHost::new(project, temporary.path().join("data")).with_remote_provider(provider);
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::DirectRemote(
            "github:skilld-dev/skills/skills/example".to_owned(),
        )),
        scope: InstallScope::Project,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
    })
    .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(["skilld", "outdated"], &host, &mut stdout, &mut stderr);

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "Unverified Skill example. Run skilld install github:skilld-dev/skills/skills/example --direct --agent codex to upgrade it.\n"
    );
}

#[test]
fn outdated_reports_the_bundled_skill_by_its_source() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let bundled = temporary.path().join("bundled").join("skilld");
    fs::create_dir_all(&bundled).unwrap();
    fs::write(
        bundled.join("SKILL.md"),
        "---\nname: skilld\ndescription: bundled\n---\n",
    )
    .unwrap();
    let provider = Arc::new(Provider::new("---\nname: example\n---\n"));
    let host = LocalHost::new(project, temporary.path().join("data"))
        .with_remote_provider(provider)
        .with_bundled_skill(bundled);
    host.install_request(InstallRequest {
        operation: InstallOperation::Install(InstallSource::BundledSkilld),
        scope: InstallScope::Global,
        targets: vec![AgentTargetId::Codex],
        mode: Some(InstallMode::Copy),
    })
    .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run(
        ["skilld", "outdated", "--system"],
        &host,
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "skilld-maintained Skill skilld.\n"
    );
}
