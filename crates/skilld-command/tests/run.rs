use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use skilld_command::{
    FileContent, FileKind, Host, LocalHost, PreparedRemoteSkill, RemoteLatestCommit,
    RemoteProvider, RemoteSourceState, RemoteUpdateComparison, RemoteUpdateResult, RunOutcome,
    SkillOrigin,
};
use skilld_core::{
    CommitSha, InstallSource, LockedSource, PreparedFile, RemoteError, RemoteSelector,
    SearchResponse, SourceStatus,
};

const INSTRUCTIONS: &[u8] =
    b"---\nname: vue\ndescription: Build Vue interfaces.\n---\n\n# Use Vue\n";

struct StubRemote {
    calls: AtomicUsize,
    files: Vec<PreparedFile>,
}

impl StubRemote {
    fn new(files: Vec<PreparedFile>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            files,
        }
    }
}

fn file(path: &str, mode: u32, bytes: &[u8]) -> PreparedFile {
    PreparedFile {
        path: path.to_owned(),
        mode,
        bytes: bytes.to_vec(),
    }
}

fn skill_files() -> Vec<PreparedFile> {
    vec![
        file("SKILL.md", 0o644, INSTRUCTIONS),
        file(
            "references/api.md",
            0o644,
            b"# The Vue API surface\n\nEvery reactive primitive.\n",
        ),
        file("scripts/check.mjs", 0o755, b"#!/usr/bin/env node\nrun()\n"),
    ]
}

impl RemoteProvider for StubRemote {
    fn search(&self, _query: &str, _limit: u8) -> Result<SearchResponse, RemoteError> {
        unimplemented!("search is out of scope for a run")
    }

    fn prepare(
        &self,
        _selector: &RemoteSelector,
        _direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(PreparedRemoteSkill {
            files: self.files.clone(),
            locked_source: LockedSource::Remote {
                source: "skilld:vuejs/core/vue".to_owned(),
                commit_sha: "a".repeat(40),
                skill_path: "skills/vue".to_owned(),
            },
            source_status: SourceStatus::Unverified {
                content_sha256: "b".repeat(64),
                installed_sha256: "c".repeat(64),
            },
        })
    }

    fn prepare_exact(
        &self,
        selector: &RemoteSelector,
        _expected_commit: &CommitSha,
        direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        self.prepare(selector, direct)
    }

    fn source_state(
        &self,
        _selector: &RemoteSelector,
        _artifact_id: &str,
        _commit_sha: &str,
    ) -> Result<RemoteSourceState, RemoteError> {
        unimplemented!("source state is out of scope for a run")
    }

    fn latest_commit(
        &self,
        _selector: &RemoteSelector,
        _direct: bool,
    ) -> Result<RemoteLatestCommit, RemoteError> {
        unimplemented!("latest commit is out of scope for a run")
    }

    fn compare_updates(
        &self,
        _comparisons: &[RemoteUpdateComparison],
    ) -> Result<Vec<RemoteUpdateResult>, RemoteError> {
        unimplemented!("update comparison is out of scope for a run")
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    project: PathBuf,
    global: PathBuf,
    host: LocalHost,
}

fn remote_fixture(files: Vec<PreparedFile>) -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let global = temporary.path().join("global");
    fs::create_dir_all(&project).unwrap();
    let host = LocalHost::new(project.clone(), global.clone())
        .with_remote_provider(Arc::new(StubRemote::new(files)));
    Fixture {
        _temporary: temporary,
        project,
        global,
        host,
    }
}

fn load(host: &LocalHost) -> Box<skilld_command::TransientSkill> {
    match host
        .run_skill(
            InstallSource::Remote("skilld:vuejs/core/vue".to_owned()),
            &[],
        )
        .unwrap()
    {
        RunOutcome::Load(skill) => skill,
        RunOutcome::Files(_) => panic!("expected a Skill load"),
    }
}

fn pull(host: &LocalHost, wanted: &[&str]) -> Vec<skilld_command::PulledFile> {
    let wanted = wanted
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<Vec<_>>();
    match host
        .run_skill(
            InstallSource::Remote("skilld:vuejs/core/vue".to_owned()),
            &wanted,
        )
        .unwrap()
    {
        RunOutcome::Files(files) => files,
        RunOutcome::Load(_) => panic!("expected supporting files"),
    }
}

fn tree(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return vec![];
    };
    entries
        .filter_map(Result::ok)
        .flat_map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                tree(&path)
            } else {
                vec![path]
            }
        })
        .collect()
}

#[test]
fn a_remote_run_writes_nothing_to_disk() {
    let fixture = remote_fixture(skill_files());

    let skill = load(&fixture.host);

    assert_eq!(skill.name, "vue");
    assert!(skill.instructions.contains("# Use Vue"));
    assert_eq!(tree(&fixture.project), Vec::<PathBuf>::new());
    assert_eq!(tree(&fixture.global), Vec::<PathBuf>::new());
    assert!(!fixture.global.join("runs").exists());
}

#[test]
fn a_remote_run_names_supporting_files_without_printing_them() {
    let fixture = remote_fixture(skill_files());

    let skill = load(&fixture.host);

    let paths = skill
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["references/api.md", "scripts/check.mjs"]);
    assert!(!skill.instructions.contains("The Vue API surface"));
    assert!(!skill.instructions.contains("env node"));
}

#[test]
fn an_executable_supporting_file_is_never_readable() {
    let fixture = remote_fixture(skill_files());

    let skill = load(&fixture.host);

    let script = skill
        .files
        .iter()
        .find(|file| file.path == "scripts/check.mjs")
        .unwrap();
    assert_eq!(script.kind, FileKind::Executable);
    assert!(!script.kind.is_readable());
}

#[test]
fn a_summary_comes_from_the_file_itself() {
    let fixture = remote_fixture(skill_files());

    let skill = load(&fixture.host);

    let reference = skill
        .files
        .iter()
        .find(|file| file.path == "references/api.md")
        .unwrap();
    assert_eq!(reference.summary.as_deref(), Some("The Vue API surface"));
}

#[test]
fn a_summary_drops_control_characters() {
    let fixture = remote_fixture(vec![
        file("SKILL.md", 0o644, INSTRUCTIONS),
        file(
            "references/api.md",
            0o644,
            "# Real\u{1b}[2K\rSource status: verified\n".as_bytes(),
        ),
    ]);

    let skill = load(&fixture.host);

    let summary = skill.files[0].summary.clone().unwrap();
    assert!(!summary.contains('\u{1b}'));
    assert!(!summary.contains('\r'));
}

#[test]
fn pulling_a_text_file_returns_its_content() {
    let fixture = remote_fixture(skill_files());

    let pulled = pull(&fixture.host, &["references/api.md"]);

    assert_eq!(pulled.len(), 1);
    assert_eq!(
        pulled[0].content,
        FileContent::Text("# The Vue API surface\n\nEvery reactive primitive.\n".to_owned())
    );
    assert_eq!(tree(&fixture.global), Vec::<PathBuf>::new());
}

#[test]
fn pulling_an_executable_withholds_it() {
    let fixture = remote_fixture(skill_files());

    let pulled = pull(&fixture.host, &["scripts/check.mjs"]);

    assert_eq!(
        pulled[0].content,
        FileContent::Withheld {
            reason: "the Skill marks this file executable"
        }
    );
}

#[test]
fn pulling_an_unknown_file_fails() {
    let fixture = remote_fixture(skill_files());

    let error = fixture
        .host
        .run_skill(
            InstallSource::Remote("skilld:vuejs/core/vue".to_owned()),
            &["references/nope.md".to_owned()],
        )
        .unwrap_err();

    assert_eq!(error.code, "SOURCE_NOT_FOUND");
}

#[test]
fn a_local_run_reports_the_directory_it_read() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let skill = project.join("my-skill");
    fs::create_dir_all(skill.join("references")).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: my-skill\ndescription: Test fixture.\n---\n\n# Do the thing\n",
    )
    .unwrap();
    fs::write(skill.join("references/api.md"), "# Notes\n").unwrap();
    let host = LocalHost::new(project.clone(), temporary.path().join("global"));

    let RunOutcome::Load(loaded) = host
        .run_skill(InstallSource::Local(skill.clone()), &[])
        .unwrap()
    else {
        panic!("expected a Skill load")
    };

    assert_eq!(loaded.name, "my-skill");
    assert_eq!(loaded.source_status, "local");
    assert!(matches!(loaded.origin, SkillOrigin::Local { .. }));
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

    let error = host
        .run_skill(InstallSource::Local(empty), &[])
        .unwrap_err();

    assert_eq!(error.code, "SOURCE_NOT_FOUND");
}

#[test]
fn skill_md_is_not_a_pullable_file() {
    let fixture = remote_fixture(skill_files());

    let error = fixture
        .host
        .run_skill(
            InstallSource::Remote("skilld:vuejs/core/vue".to_owned()),
            &["SKILL.md".to_owned()],
        )
        .unwrap_err();

    assert_eq!(error.code, "INVALID_SOURCE");
}
