//! `skilld add` installs every Skill a multi-skill ref names.

use std::fs;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use skilld_command::{
    CommandPlatform, DetectionEnvironment, Host, LocalHost, OutputContext, PreparedRemoteSkill,
    RemoteLatestCommit, RemoteProvider, RemoteSourceState, RemoteUpdateComparison,
    RemoteUpdateResult, run_with_output,
};
use skilld_core::{
    CommitSha, InstallScope, ListedOrigin, ListedSkill, LockedSource, MultiSkillRef, PreparedFile,
    RemoteError, RemoteSelector, SearchResponse, SkillListing, SourceSelector, SourceStatus,
};

/// Lists two Skills for one Repository and prepares whichever one is asked for.
struct ListingRemote;

impl RemoteProvider for ListingRemote {
    fn list_direct_skills(&self, reference: &MultiSkillRef) -> Result<SkillListing, RemoteError> {
        let mut listing = self.list_skills(reference)?;
        for skill in &mut listing.items {
            skill.origin = ListedOrigin::Direct {
                path: format!("skills/{}", skill.name),
            };
        }
        Ok(listing)
    }
    fn list_skills(&self, reference: &MultiSkillRef) -> Result<SkillListing, RemoteError> {
        assert_eq!(
            *reference,
            MultiSkillRef::Repository {
                owner: "vuejs".to_owned(),
                repository: "core".to_owned(),
            }
        );
        Ok(SkillListing {
            reference: reference.clone(),
            items: ["vue", "nuxt"]
                .into_iter()
                .map(|name| ListedSkill {
                    name: name.to_owned(),
                    owner: "vuejs".to_owned(),
                    repository: "core".to_owned(),
                    description: None,
                    origin: ListedOrigin::Registry { path: None },
                })
                .collect(),
        })
    }

    fn search(&self, _query: &str, _limit: u8) -> Result<SearchResponse, RemoteError> {
        unimplemented!("search is outside an add")
    }

    fn prepare(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        let name = match &selector.source().selector {
            SourceSelector::NamedSkill { name } => name.as_str(),
            SourceSelector::Path { path } => path.rsplit('/').next().unwrap(),
        };
        if direct {
            assert!(selector.canonical().starts_with("github:"));
        }
        let file = PreparedFile {
            path: "SKILL.md".to_owned(),
            mode: 0o644,
            bytes: format!("---\nname: {name}\ndescription: Use {name}.\n---\n\n# {name}\n")
                .into_bytes(),
        };
        let digest = installed_digest(&file);
        Ok(PreparedRemoteSkill {
            files: vec![file],
            locked_source: LockedSource::Remote {
                source: selector.canonical(),
                commit_sha: "a".repeat(40),
                skill_path: format!("skills/{name}"),
            },
            source_status: SourceStatus::Unverified {
                content_sha256: digest.clone(),
                installed_sha256: digest,
            },
            page_url: None,
            omitted_files: Vec::new(),
            behavior_readings: Vec::new(),
        })
    }

    fn prepare_exact(
        &self,
        _selector: &RemoteSelector,
        _expected_commit: &CommitSha,
        _direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        unimplemented!("exact preparation is outside an add")
    }

    fn source_state(
        &self,
        _selector: &RemoteSelector,
        _artifact_id: &str,
        _commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError> {
        unimplemented!("source state is outside an add")
    }

    fn latest_commit(
        &self,
        _selector: &RemoteSelector,
        _direct: bool,
    ) -> Result<RemoteLatestCommit, RemoteError> {
        unimplemented!("latest commit is outside an add")
    }

    fn compare_updates(
        &self,
        _comparisons: &[RemoteUpdateComparison],
    ) -> Result<Vec<RemoteUpdateResult>, RemoteError> {
        unimplemented!("update comparison is outside an add")
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

#[test]
fn noninteractive_add_requires_explicit_selection_before_writing_files() {
    for flags in [vec![], vec!["--plain"]] {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let host = LocalHost::new(project.clone(), temporary.path().join("global"))
            .with_detection_environment(DetectionEnvironment::new(["CLAUDE_CODE".to_owned()]))
            .with_remote_provider(Arc::new(ListingRemote));
        let mut args = vec!["skilld", "add", "vuejs/core"];
        args.extend(flags);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = run_with_output(
            args,
            &host,
            OutputContext::Plain {
                platform: CommandPlatform::Unix,
            },
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(result.exit_code, 1);
        let error = String::from_utf8(stderr).unwrap();
        assert!(error.contains("SKILL_SELECTION_REQUIRED"), "{error}");
        assert!(error.contains("vue"), "{error}");
        assert!(error.contains("nuxt"), "{error}");
        assert!(error.contains("--all"), "{error}");
        assert!(!project.join(".skills").exists());
        assert!(!project.join(".claude/skills").exists());
    }
}

#[test]
fn add_installs_every_skill_the_repository_ref_names() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project.clone(), temporary.path().join("global"))
        .with_detection_environment(DetectionEnvironment::new(["CLAUDE_CODE".to_owned()]))
        .with_remote_provider(Arc::new(ListingRemote));

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(
        ["skilld", "add", "vuejs/core", "--all"],
        &host,
        OutputContext::Plain {
            platform: CommandPlatform::Unix,
        },
        &mut stdout,
        &mut stderr,
    );

    assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let output = String::from_utf8(stdout).unwrap();
    for name in ["vue", "nuxt"] {
        assert!(
            output.contains(&format!("Installed Skill {name}.")),
            "{output}"
        );
        assert!(
            output.contains(&format!(
                "Files: {}",
                project.join(".claude/skills").join(name).display()
            )),
            "{output}"
        );
        assert!(
            output.contains(&format!(
                "https://github.com/vuejs/core/blob/{}/skills/{name}/SKILL.md",
                "a".repeat(40)
            )),
            "{output}"
        );
    }
    assert_eq!(
        output
            .matches("Read the instructions before using each Skill.")
            .count(),
        1
    );
    assert_eq!(host.list(InstallScope::Project).unwrap(), ["nuxt", "vue"]);
    for name in ["vue", "nuxt"] {
        assert!(
            project
                .join(".claude/skills")
                .join(name)
                .join("SKILL.md")
                .exists(),
            "{name} reached the Agent target"
        );
    }
}

/// Lists the same two Skills, but `nuxt` tells the Agent to run `sudo`.
struct PrivilegedNuxtRemote;

impl RemoteProvider for PrivilegedNuxtRemote {
    fn list_skills(&self, reference: &MultiSkillRef) -> Result<SkillListing, RemoteError> {
        ListingRemote.list_skills(reference)
    }

    fn search(&self, query: &str, limit: u8) -> Result<SearchResponse, RemoteError> {
        ListingRemote.search(query, limit)
    }

    fn prepare(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        let mut prepared = ListingRemote.prepare(selector, direct)?;
        let file = &mut prepared.files[0];
        if String::from_utf8_lossy(&file.bytes).contains("name: nuxt") {
            file.bytes
                .extend_from_slice(b"\n```sh\nsudo nuxi upgrade\n```\n");
            let digest = installed_digest(file);
            prepared.source_status = SourceStatus::Unverified {
                content_sha256: digest.clone(),
                installed_sha256: digest,
            };
        }
        Ok(prepared)
    }

    fn prepare_exact(
        &self,
        selector: &RemoteSelector,
        expected_commit: &CommitSha,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        ListingRemote.prepare_exact(selector, expected_commit, direct)
    }

    fn source_state(
        &self,
        selector: &RemoteSelector,
        artifact_id: &str,
        commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError> {
        ListingRemote.source_state(selector, artifact_id, commit_sha)
    }

    fn latest_commit(
        &self,
        selector: &RemoteSelector,
        direct: bool,
    ) -> Result<RemoteLatestCommit, RemoteError> {
        ListingRemote.latest_commit(selector, direct)
    }

    fn compare_updates(
        &self,
        comparisons: &[RemoteUpdateComparison],
    ) -> Result<Vec<RemoteUpdateResult>, RemoteError> {
        ListingRemote.compare_updates(comparisons)
    }
}

fn add_with(host: &LocalHost, args: &[&str]) -> (u8, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(
        args,
        host,
        OutputContext::Plain {
            platform: CommandPlatform::Unix,
        },
        &mut stdout,
        &mut stderr,
    );
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap() + &String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn explicit_direct_add_installs_all_listed_skills_for_the_selected_agent() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project.clone(), temporary.path().join("global"))
        .with_remote_provider(Arc::new(ListingRemote));
    let (exit, output) = add_with(
        &host,
        &[
            "skilld",
            "add",
            "vuejs/core",
            "--direct",
            "--all",
            "--agent",
            "codex",
        ],
    );
    assert_eq!(exit, 0, "{output}");
    assert_eq!(host.list(InstallScope::Project).unwrap(), ["nuxt", "vue"]);
    for name in ["vue", "nuxt"] {
        let path = project.join(".agents/skills").join(name);
        assert!(path.join("SKILL.md").is_file());
        assert!(
            output.contains(&format!("Files: {}", path.display())),
            "{output}"
        );
    }
    assert_eq!(output.matches("Source status: unverified").count(), 2);
}

#[test]
fn add_holds_back_only_the_skill_with_an_unapproved_behavior() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project.clone(), temporary.path().join("global"))
        .with_detection_environment(DetectionEnvironment::new(["CLAUDE_CODE".to_owned()]))
        .with_remote_provider(Arc::new(PrivilegedNuxtRemote));

    let (exit, output) = add_with(&host, &["skilld", "add", "vuejs/core", "--all"]);

    assert_ne!(exit, 0, "{output}");
    assert!(
        output.contains("skilld installed 1 of 2 Skills"),
        "{output}"
    );
    assert!(
        output.contains("nuxt: BEHAVIOR_CONFIRMATION_REQUIRED:"),
        "{output}"
    );
    assert!(output.contains("--allow privilege"), "{output}");
    assert_eq!(host.list(InstallScope::Project).unwrap(), ["vue"]);

    let (exit, output) = add_with(
        &host,
        &[
            "skilld",
            "add",
            "vuejs/core",
            "--all",
            "--allow",
            "privilege",
        ],
    );

    assert_eq!(exit, 0, "{output}");
    assert_eq!(host.list(InstallScope::Project).unwrap(), ["nuxt", "vue"]);
}
