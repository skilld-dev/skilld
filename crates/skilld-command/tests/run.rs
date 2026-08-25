use std::fs;
use std::path::{Path, PathBuf};

use skilld_command::{CommandError, Host, LocalHost, TransientSkill, run};
use skilld_core::{InstallScope, InstallSource};

struct StubHost(TransientSkill);

impl Host for StubHost {
    fn list(&self, _scope: InstallScope) -> Result<Vec<String>, CommandError> {
        Ok(vec![])
    }

    fn install(
        &self,
        _source: InstallSource,
        _scope: InstallScope,
    ) -> Result<String, CommandError> {
        panic!("skilld run must not install");
    }

    fn run_skill(&self, _source: InstallSource) -> Result<TransientSkill, CommandError> {
        Ok(self.0.clone())
    }
}

fn stub(source_status: &'static str, direct: bool) -> TransientSkill {
    TransientSkill {
        name: "vue".to_owned(),
        instructions: "---\nname: vue\n---\n\n# Use Vue\n\n```sh\nnpm i vue\n```\n".to_owned(),
        root: PathBuf::from("/cache/vue"),
        files: vec!["references/api.md".to_owned()],
        source: "skilld:vuejs/core/vue".to_owned(),
        source_status,
        direct,
    }
}

fn stdout_of(host: &impl Host, args: [&str; 3]) -> String {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run(args, host, &mut stdout, &mut stderr);
    assert_eq!(
        result.exit_code,
        0,
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    String::from_utf8(stdout).unwrap()
}

fn skill_directory(root: &Path) -> PathBuf {
    let path = root.join("my-skill");
    fs::create_dir_all(path.join("references")).unwrap();
    fs::write(
        path.join("SKILL.md"),
        "---\nname: my-skill\ndescription: Test fixture.\n---\n\n# Do the thing\n",
    )
    .unwrap();
    fs::write(path.join("references/api.md"), "api").unwrap();
    path
}

#[test]
fn a_run_hands_the_agent_the_instructions_and_names_the_install_step() {
    let output = stdout_of(
        &StubHost(stub("verified", false)),
        ["skilld", "run", "skilld:vuejs/core/vue"],
    );

    assert!(
        output.contains("skilld loaded the Skill vue for this session. skilld installed nothing.")
    );
    assert!(output.contains("Source status: verified"));
    assert!(output.contains("Skill files: /cache/vue"));
    assert!(output.contains("  references/api.md"));
    assert!(output.contains("# Use Vue\n\n```sh\nnpm i vue\n```"));
    assert!(output.contains("Keep the Skill: skilld install skilld:vuejs/core/vue"));
    assert!(output.contains("Find another Skill: skilld search <query>"));
}

#[test]
fn a_direct_run_asks_for_a_review_and_keeps_the_direct_flag() {
    let output = stdout_of(
        &StubHost(stub("unverified", true)),
        ["skilld", "run", "github:vuejs/core/skills/vue"],
    );

    assert!(output.contains("Review this Skill before you follow it."));
    assert!(output.contains("Keep the Skill: skilld install skilld:vuejs/core/vue --direct"));
}

#[test]
fn a_verified_run_does_not_ask_for_a_review() {
    let output = stdout_of(
        &StubHost(stub("verified", false)),
        ["skilld", "run", "skilld:vuejs/core/vue"],
    );

    assert!(!output.contains("Review this Skill"));
}

#[test]
fn a_local_run_reads_the_directory_and_installs_nothing() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let skill = skill_directory(&project);
    let host = LocalHost::new(project.clone(), temporary.path().join("global"));

    let loaded = host.run_skill(InstallSource::Local(skill.clone())).unwrap();

    assert_eq!(loaded.name, "my-skill");
    assert_eq!(loaded.source_status, "local");
    assert_eq!(loaded.files, ["references/api.md"]);
    assert!(loaded.instructions.contains("# Do the thing"));
    assert!(!project.join(".skills").exists());
}

#[test]
fn a_local_run_reports_a_directory_without_instructions() {
    let temporary = tempfile::tempdir().unwrap();
    let empty = temporary.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let host = LocalHost::new(
        temporary.path().to_path_buf(),
        temporary.path().join("global"),
    );

    let error = host.run_skill(InstallSource::Local(empty)).unwrap_err();

    assert_eq!(error.code, "SOURCE_NOT_FOUND");
}

#[test]
fn direct_rejects_a_local_source() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run(
        ["skilld", "run", "./skills/vue", "--direct"],
        &StubHost(stub("local", false)),
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 2);
    assert!(stdout.is_empty());
}

struct StubRemote {
    calls: std::sync::atomic::AtomicUsize,
}

impl skilld_command::RemoteProvider for StubRemote {
    fn search(
        &self,
        _query: &str,
        _limit: u8,
    ) -> Result<skilld_core::SearchResponse, skilld_core::RemoteError> {
        unimplemented!("search is out of scope for a run")
    }

    fn prepare(
        &self,
        _selector: &skilld_core::RemoteSelector,
        _direct: bool,
    ) -> Result<skilld_command::PreparedRemoteSkill, skilld_core::RemoteError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(skilld_command::PreparedRemoteSkill {
            files: vec![
                skilld_core::PreparedFile {
                    path: "SKILL.md".to_owned(),
                    mode: 0o644,
                    bytes: b"---\nname: vue\ndescription: Test fixture.\n---\n\n# Use Vue\n"
                        .to_vec(),
                },
                skilld_core::PreparedFile {
                    path: "references/api.md".to_owned(),
                    mode: 0o644,
                    bytes: b"api".to_vec(),
                },
            ],
            locked_source: skilld_core::LockedSource::Remote {
                source: "skilld:vuejs/core/vue".to_owned(),
                commit_sha: "a".repeat(40),
                skill_path: "skills/vue".to_owned(),
            },
            source_status: skilld_core::SourceStatus::Unverified {
                content_sha256: "b".repeat(64),
                installed_sha256: "c".repeat(64),
            },
        })
    }

    fn prepare_exact(
        &self,
        selector: &skilld_core::RemoteSelector,
        _expected_commit: &skilld_core::CommitSha,
        direct: bool,
    ) -> Result<skilld_command::PreparedRemoteSkill, skilld_core::RemoteError> {
        self.prepare(selector, direct)
    }

    fn source_state(
        &self,
        _selector: &skilld_core::RemoteSelector,
        _artifact_id: &str,
        _commit_sha: &str,
    ) -> Result<skilld_command::RemoteSourceState, skilld_core::RemoteError> {
        unimplemented!("source state is out of scope for a run")
    }

    fn latest_commit(
        &self,
        _selector: &skilld_core::RemoteSelector,
        _direct: bool,
    ) -> Result<skilld_command::RemoteLatestCommit, skilld_core::RemoteError> {
        unimplemented!("latest commit is out of scope for a run")
    }

    fn compare_updates(
        &self,
        _comparisons: &[skilld_command::RemoteUpdateComparison],
    ) -> Result<Vec<skilld_command::RemoteUpdateResult>, skilld_core::RemoteError> {
        unimplemented!("update comparison is out of scope for a run")
    }
}

#[test]
fn a_remote_run_caches_the_files_and_leaves_the_project_alone() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let global = temporary.path().join("global");
    fs::create_dir_all(&project).unwrap();
    let remote = std::sync::Arc::new(StubRemote {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let host = LocalHost::new(project.clone(), global.clone()).with_remote_provider(remote.clone());

    let first = host
        .run_skill(InstallSource::Remote("skilld:vuejs/core/vue".to_owned()))
        .unwrap();
    let second = host
        .run_skill(InstallSource::Remote("skilld:vuejs/core/vue".to_owned()))
        .unwrap();

    assert_eq!(first.root, second.root);
    assert!(first.root.starts_with(global.join("runs")));
    assert_eq!(
        fs::read_to_string(first.root.join("references/api.md")).unwrap(),
        "api"
    );
    assert_eq!(first.files, ["references/api.md"]);
    assert_eq!(first.source_status, "unverified");
    assert!(!project.join(".skills").exists());
    assert!(!global.join("skills").exists());
}
