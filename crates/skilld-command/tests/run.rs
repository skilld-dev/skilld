use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use skilld_command::{
    BundledSkillProvider, CommandError, CommandPlatform, FileContent, FileKind, Host, LocalHost,
    OutputContext, PreparedRemoteSkill, RemoteLatestCommit, RemoteProvider, RemoteSourceState,
    RemoteUpdateComparison, RemoteUpdateResult, RunOutcome, SkillOrigin, run_with_output,
};
use skilld_core::{
    CommitSha, InstallSource, LockedSource, PreparedFile, RemoteError, RemoteSelector,
    SearchResponse, SourceStatus,
};

const INSTRUCTIONS: &[u8] =
    b"---\nname: vue\ndescription: Build Vue interfaces.\n---\n\n# Use Vue\n";

const MAX_TEST_DEPTH: usize = 9;

struct StubRemote {
    calls: AtomicUsize,
    exact_calls: AtomicUsize,
    files: Vec<PreparedFile>,
    skill_path: String,
}

impl StubRemote {
    fn new(files: Vec<PreparedFile>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            exact_calls: AtomicUsize::new(0),
            files,
            skill_path: "skills/vue".to_owned(),
        }
    }

    fn with_skill_path(mut self, skill_path: &str) -> Self {
        self.skill_path = skill_path.to_owned();
        self
    }

    fn prepared(&self, commit_sha: String) -> PreparedRemoteSkill {
        PreparedRemoteSkill {
            files: self.files.clone(),
            locked_source: LockedSource::Remote {
                source: "skilld:vuejs/core/vue".to_owned(),
                commit_sha,
                skill_path: self.skill_path.clone(),
            },
            source_status: SourceStatus::Unverified {
                content_sha256: "b".repeat(64),
                installed_sha256: "c".repeat(64),
            },
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
        Ok(self.prepared("a".repeat(40)))
    }

    fn prepare_exact(
        &self,
        selector: &RemoteSelector,
        expected_commit: &CommitSha,
        _direct: bool,
    ) -> Result<PreparedRemoteSkill, RemoteError> {
        let _ = selector;
        self.exact_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.prepared(expected_commit.as_str().to_owned()))
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
    remote: Arc<StubRemote>,
}

fn remote_fixture(files: Vec<PreparedFile>) -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let global = temporary.path().join("global");
    fs::create_dir_all(&project).unwrap();
    let remote = Arc::new(StubRemote::new(files));
    let host = LocalHost::new(project.clone(), global.clone()).with_remote_provider(remote.clone());
    Fixture {
        _temporary: temporary,
        project,
        global,
        host,
        remote,
    }
}

fn remote_fixture_with_skill_path(files: Vec<PreparedFile>, skill_path: &str) -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let global = temporary.path().join("global");
    fs::create_dir_all(&project).unwrap();
    let remote = Arc::new(StubRemote::new(files).with_skill_path(skill_path));
    let host = LocalHost::new(project.clone(), global.clone()).with_remote_provider(remote.clone());
    Fixture {
        _temporary: temporary,
        project,
        global,
        host,
        remote,
    }
}

fn run_cli<H: Host>(host: &H, args: Vec<String>) -> (u8, String, String) {
    run_cli_on(host, args, CommandPlatform::Unix)
}

fn run_cli_on<H: Host>(
    host: &H,
    args: Vec<String>,
    platform: CommandPlatform,
) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_with_output(
        args,
        host,
        OutputContext::Plain { platform },
        &mut stdout,
        &mut stderr,
    );
    (
        result.exit_code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn load(host: &LocalHost) -> Box<skilld_command::TransientSkill> {
    match host
        .run_skill(
            InstallSource::Remote("skilld:vuejs/core/vue".to_owned()),
            &[],
            None,
        )
        .unwrap()
    {
        RunOutcome::Load(skill) => skill,
        RunOutcome::Files { .. } => panic!("expected a Skill load"),
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
            Some(&CommitSha::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap()),
        )
        .unwrap()
    {
        RunOutcome::Files { files, .. } => files,
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
    let fixture = remote_fixture(vec![
        file("SKILL.md", 0o644, INSTRUCTIONS),
        file("references/api.md", 0o644, b"secret-supporting-prompt\n"),
        file("scripts/check.mjs", 0o755, b"#!/usr/bin/env node\nrun()\n"),
    ]);

    let (_, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--json".to_owned(),
        ],
    );
    let output: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let files = output["data"]["files"].as_array().unwrap();

    let paths = files
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["references/api.md", "scripts/check.mjs"]);
    assert!(stderr.is_empty());
    assert!(!stdout.contains("secret-supporting-prompt"));
    assert!(!stdout.contains("env node"));
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
            Some(&CommitSha::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap()),
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
        .run_skill(InstallSource::Local(skill.clone()), &[], None)
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
        .run_skill(InstallSource::Local(empty), &[], None)
        .unwrap_err();

    assert_eq!(error.code, "INVALID_SOURCE");
}

struct TrackingBundled {
    source: PathBuf,
    source_calls: AtomicUsize,
}

impl BundledSkillProvider for TrackingBundled {
    fn skilld_run_files(&self) -> Result<Vec<PreparedFile>, CommandError> {
        Ok(vec![
            file(
                "SKILL.md",
                0o644,
                b"---\nname: skilld\ndescription: Test bundled Skill.\n---\n\n# Use skilld\n",
            ),
            file("references/api.md", 0o644, b"# API\n"),
        ])
    }

    fn skilld_source(&self) -> Result<PathBuf, CommandError> {
        self.source_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.source.clone())
    }
}

#[test]
fn bundled_run_preserves_identity_without_install_staging() {
    let temporary = tempfile::tempdir().unwrap();
    let provider = Arc::new(TrackingBundled {
        source: temporary.path().join("must-not-materialize"),
        source_calls: AtomicUsize::new(0),
    });
    let host = LocalHost::new(
        temporary.path().join("project"),
        temporary.path().join("global"),
    )
    .with_bundled_provider(provider.clone());

    let (exit, stdout, stderr) = run_cli(
        &host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld".to_owned(),
            "--json".to_owned(),
        ],
    );
    let output: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(exit, 0);
    assert!(stderr.is_empty());
    assert_eq!(provider.source_calls.load(Ordering::SeqCst), 0);
    assert_eq!(output["data"]["origin"]["_tag"], "bundled");
    assert_eq!(output["data"]["origin"]["source"], "skilld");
    assert_eq!(
        output["data"]["files"][0]["readArgv"],
        serde_json::json!([
            "skilld",
            "run",
            "skilld",
            "--file=references/api.md",
            "--json"
        ])
    );
    assert!(output["data"]["installArgv"]["project"].is_null());
    assert_eq!(
        output["data"]["installArgv"]["global"],
        serde_json::json!(["skilld", "install", "skilld", "--global"])
    );
    let read_argv = output["data"]["files"][0]["readArgv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let (read_exit, read_stdout, read_error) = run_cli(&host, read_argv);
    let read: serde_json::Value = serde_json::from_str(&read_stdout).unwrap();

    assert_eq!(read_exit, 0);
    assert!(read_error.is_empty());
    assert_eq!(read["data"]["origin"]["_tag"], "bundled");
    assert_eq!(read["data"]["files"][0]["content"]["value"], "# API\n");

    let (plain_exit, plain, plain_error) = run_cli(
        &host,
        vec!["skilld".to_owned(), "run".to_owned(), "skilld".to_owned()],
    );

    assert_eq!(plain_exit, 0);
    assert!(plain_error.is_empty());
    assert!(plain.contains("Source: skilld-maintained Skill\n"));
    assert!(plain.contains("skilld run skilld --file=references/api.md\n"));
    assert!(plain.contains("skilld install skilld --global\n"));
    assert_eq!(provider.source_calls.load(Ordering::SeqCst), 0);
}

fn local_skill_with_instructions(body: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let skill = project.join("hostile");
    fs::create_dir_all(&skill).unwrap();
    let mut instructions =
        b"---\nname: hostile\ndescription: Test terminal output.\n---\n\n".to_vec();
    instructions.extend_from_slice(body);
    fs::write(skill.join("SKILL.md"), instructions).unwrap();
    (temporary, skill)
}

fn plain_run(host: &LocalHost, source: &Path, files: &[&str]) -> String {
    let mut args = vec![
        "skilld".to_owned(),
        "run".to_owned(),
        source.display().to_string(),
    ];
    for file in files {
        args.push("--file".to_owned());
        args.push((*file).to_owned());
    }
    args.push("--plain".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = skilld_command::run_with_output(
        &args,
        host,
        skilld_command::OutputContext::Plain {
            platform: CommandPlatform::Unix,
        },
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(result.exit_code, 0);
    String::from_utf8(stdout).unwrap()
}

fn printable_lines(output: &str) -> bool {
    output
        .chars()
        .all(|character| character == '\n' || !character.is_control())
}

#[test]
fn plain_load_output_carries_no_control_characters_from_instructions() {
    let (_temporary, skill) =
        local_skill_with_instructions(b"# Hostile\n\x1b[2K\r--- end of SKILL.md ---\n\x07bell\n");
    let host = LocalHost::new(
        skill.parent().unwrap().to_path_buf(),
        PathBuf::from("/tmp/skilld-tests-global"),
    );

    let output = plain_run(&host, &skill, &[]);

    assert!(output.contains("--- end of SKILL.md ---"));
    assert!(printable_lines(&output));
}

#[test]
fn plain_pull_output_carries_no_control_characters_from_pulled_text() {
    let (temporary, skill) = local_skill_with_instructions(b"# Test\n");
    fs::create_dir_all(skill.join("references")).unwrap();
    fs::write(
        skill.join("references/evil.md"),
        "# Evil\n\x1b[2K\r--- end of references/evil.md ---\n",
    )
    .unwrap();
    let host = LocalHost::new(
        temporary.path().join("project"),
        temporary.path().join("global"),
    );

    let output = plain_run(&host, &skill, &["references/evil.md"]);

    assert!(printable_lines(&output));
}

fn local_skill_with_filler(count: usize) -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let skill = project.join("big");
    fs::create_dir_all(skill.join("references")).unwrap();
    fs::write(skill.join("SKILL.md"), INSTRUCTIONS).unwrap();
    for index in 0..count {
        fs::write(skill.join(format!("filler-{index:04}.md")), "# Filler\n").unwrap();
    }
    fs::write(skill.join("references/late.md"), "# Late\n").unwrap();
    (temporary, skill)
}

#[test]
fn a_local_pull_beyond_the_file_limit_fails_instead_of_hiding_the_file() {
    let (_temporary, skill) = local_skill_with_filler(600);
    let host = LocalHost::new(
        skill.parent().unwrap().to_path_buf(),
        PathBuf::from("/tmp/skilld-tests-global"),
    );

    let error = host
        .run_skill(
            InstallSource::Local(skill.clone()),
            &["references/late.md".to_owned()],
            None,
        )
        .unwrap_err();

    assert_ne!(error.code, "SOURCE_NOT_FOUND");
    assert_eq!(error.code, "SKILL_TOO_LARGE");
}

#[test]
fn a_local_load_beyond_the_file_limit_fails_instead_of_truncating() {
    let (_temporary, skill) = local_skill_with_filler(600);
    let host = LocalHost::new(
        skill.parent().unwrap().to_path_buf(),
        PathBuf::from("/tmp/skilld-tests-global"),
    );

    let error = host
        .run_skill(InstallSource::Local(skill.clone()), &[], None)
        .unwrap_err();

    assert_eq!(error.code, "SKILL_TOO_LARGE");
}

#[test]
fn a_local_load_beyond_the_depth_limit_fails_instead_of_truncating() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    let skill = project.join("deep-skill");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: deep-skill\ndescription: Test depth.\n---\n\n# Test\n",
    )
    .unwrap();
    let mut deepest = skill;
    for level in 0..=MAX_TEST_DEPTH {
        deepest = deepest.join(format!("level-{level}"));
    }
    fs::create_dir_all(&deepest).unwrap();
    fs::write(deepest.join("note.md"), "# Deep\n").unwrap();
    let host = LocalHost::new(project.clone(), temporary.path().join("global"));

    let error = host
        .run_skill(InstallSource::Local(project.join("deep-skill")), &[], None)
        .unwrap_err();

    assert_eq!(error.code, "SKILL_TOO_LARGE");
}

#[test]
fn skill_md_is_not_a_pullable_file() {
    let fixture = remote_fixture(skill_files());

    let error = fixture
        .host
        .run_skill(
            InstallSource::Remote("skilld:vuejs/core/vue".to_owned()),
            &["SKILL.md".to_owned()],
            Some(&CommitSha::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap()),
        )
        .unwrap_err();

    assert_eq!(error.code, "INVALID_SOURCE");
}

#[test]
fn generated_file_read_uses_the_loaded_remote_revision() {
    let fixture = remote_fixture_with_skill_path(skill_files(), "packages/vue");
    let (_, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--json".to_owned(),
        ],
    );
    let loaded: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let read_argv = loaded["data"]["files"][0]["readArgv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(stderr.is_empty());
    assert_eq!(
        read_argv[2],
        "github:vuejs/core/packages/vue#commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );

    let (exit, stdout, stderr) = run_cli(&fixture.host, read_argv);
    let files: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(exit, 0);
    assert!(stderr.is_empty());
    assert_eq!(files["data"]["revision"], "a".repeat(40));
    assert_eq!(files["data"]["sourceStatus"], "unverified");
    assert_eq!(
        files["data"]["sourceCaution"],
        "skilld did not check this source. Read this Skill before you follow it."
    );
    assert_eq!(
        files["data"]["origin"]["source"],
        "github:vuejs/core/packages/vue#commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(files["data"]["wroteSkillFiles"], false);
    assert_eq!(fixture.remote.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.remote.exact_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn conflicting_source_commit_and_revision_fail_before_fetch() {
    let fixture = remote_fixture(skill_files());
    let source = format!("github:vuejs/core/skills/vue#commit:{}", "a".repeat(40));

    let (exit, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            source,
            "--revision".to_owned(),
            "b".repeat(40),
            "--file=references/api.md".to_owned(),
            "--json".to_owned(),
        ],
    );
    let error: serde_json::Value = serde_json::from_str(&stderr).unwrap();

    assert_eq!(exit, 1);
    assert!(stdout.is_empty());
    assert_eq!(error["_tag"], "OperationError");
    assert_eq!(error["error"]["code"], "SOURCE_MISMATCH");
    assert_eq!(fixture.remote.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.remote.exact_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn matching_source_commit_and_revision_keep_exact_provenance() {
    let fixture = remote_fixture(skill_files());
    let source = format!("github:vuejs/core/skills/vue#commit:{}", "a".repeat(40));

    let (exit, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            source.clone(),
            "--revision".to_owned(),
            "a".repeat(40),
            "--file=references/api.md".to_owned(),
            "--json".to_owned(),
        ],
    );
    let output: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(exit, 0);
    assert!(stderr.is_empty());
    assert_eq!(output["data"]["origin"]["source"], source);
    assert_eq!(output["data"]["revision"], "a".repeat(40));
    assert_eq!(fixture.remote.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.remote.exact_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn remote_file_read_without_revision_fails_before_fetch() {
    let fixture = remote_fixture(skill_files());

    let (exit, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--file".to_owned(),
            "references/api.md".to_owned(),
            "--json".to_owned(),
        ],
    );
    let error: serde_json::Value = serde_json::from_str(&stderr).unwrap();

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(error["error"]["code"], "INVALID_SOURCE");
    assert_eq!(
        error["error"]["message"],
        "Remote --file reads require --revision. Run the Skill without --file first. Then repeat this run with the returned revision."
    );
    assert_eq!(fixture.remote.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.remote.exact_calls.load(Ordering::SeqCst), 0);

    let error = fixture
        .host
        .run_skill(
            InstallSource::Remote("skilld:vuejs/core/vue".to_owned()),
            &["references/api.md".to_owned()],
            None,
        )
        .unwrap_err();

    assert_eq!(error.code, "INVALID_SOURCE");
    assert_eq!(fixture.remote.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn remote_run_rejects_a_hash_in_the_attested_skill_path() {
    let fixture = remote_fixture_with_skill_path(skill_files(), "skills/vue#archive");

    let (exit, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--json".to_owned(),
        ],
    );
    let error: serde_json::Value = serde_json::from_str(&stderr).unwrap();

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(error["error"]["code"], "INVALID_SOURCE");
}

#[test]
fn json_run_is_compact_typed_and_uses_argument_arrays() {
    let fixture = remote_fixture(skill_files());

    let (_, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--json".to_owned(),
        ],
    );
    let output: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert!(stderr.is_empty());
    assert!(!stdout.contains("\n  \""));
    assert_eq!(output["_tag"], "Success");
    assert_eq!(output["notices"], serde_json::json!([]));
    assert_eq!(output["data"]["_tag"], "load");
    assert_eq!(output["data"]["revision"], "a".repeat(40));
    assert_eq!(output["data"]["wroteSkillFiles"], false);
    assert_eq!(
        output["data"]["files"][0]["readArgv"],
        serde_json::json!([
            "skilld",
            "run",
            "github:vuejs/core/skills/vue#commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--revision",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--file=references/api.md",
            "--json"
        ])
    );
    assert_eq!(
        output["data"]["installArgv"]["project"],
        serde_json::json!([
            "skilld",
            "install",
            "github:vuejs/core/skills/vue#commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ])
    );
    assert_eq!(
        output["data"]["installArgv"]["global"],
        serde_json::json!([
            "skilld",
            "install",
            "github:vuejs/core/skills/vue#commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--global"
        ])
    );
}

#[test]
fn remote_install_guidance_pins_the_reviewed_path_and_commit() {
    let fixture = remote_fixture(skill_files());

    let (_, plain, plain_error) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
        ],
    );
    let (_, json, json_error) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "github:vuejs/core/catalog/vue".to_owned(),
            "--direct".to_owned(),
            "--json".to_owned(),
        ],
    );
    let output: serde_json::Value = serde_json::from_str(&json).unwrap();
    let exact = format!("github:vuejs/core/skills/vue#commit:{}", "a".repeat(40));

    assert!(plain_error.is_empty());
    assert!(json_error.is_empty());
    assert!(plain.contains(&format!("skilld install '{exact}'\n")));
    assert!(plain.contains(&format!("skilld install '{exact}' --global\n")));
    assert_eq!(
        output["data"]["installArgv"]["project"],
        serde_json::json!(["skilld", "install", exact.clone(), "--direct"])
    );
    assert_eq!(
        output["data"]["installArgv"]["global"],
        serde_json::json!(["skilld", "install", exact, "--direct", "--global"])
    );
}

#[test]
fn plain_and_json_run_outputs_handle_metacharacter_paths_as_data() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project with $(printf injected)");
    let skill = project.join("my-skill");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: my-skill\ndescription: Test fixture.\n---\n\n# Do the thing\n",
    )
    .unwrap();
    fs::write(skill.join("-$(printf injected).md"), "# Notes\n").unwrap();
    let host = LocalHost::new(project.clone(), temporary.path().join("global"));

    let (_, plain, plain_error) = run_cli(
        &host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            skill.display().to_string(),
            "--plain".to_owned(),
        ],
    );
    let (_, json, json_error) = run_cli(
        &host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            skill.display().to_string(),
            "--json".to_owned(),
        ],
    );
    let output: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert!(plain_error.is_empty());
    assert!(json_error.is_empty());
    assert!(plain.contains("'--file=-$(printf injected).md'"));
    assert!(plain.contains(&format!("skilld install '{}'", skill.display())));
    assert_eq!(
        output["data"]["files"][0]["readArgv"],
        serde_json::json!([
            "skilld",
            "run",
            skill.display().to_string(),
            "--file=-$(printf injected).md",
            "--json"
        ])
    );
    let read_argv = output["data"]["files"][0]["readArgv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let (exit, read_stdout, read_stderr) = run_cli(&host, read_argv);
    let read: serde_json::Value = serde_json::from_str(&read_stdout).unwrap();
    assert_eq!(exit, 0);
    assert!(read_stderr.is_empty());
    assert_eq!(read["data"]["files"][0]["content"]["value"], "# Notes\n");
}

#[test]
fn generated_commands_quote_apostrophes_for_the_declared_platform() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project's & $(printf injected)");
    let skill = project.join("my-skill");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: my-skill\ndescription: Test fixture.\n---\n\n# Test\n",
    )
    .unwrap();
    fs::write(
        skill.join("reference's & $(printf injected).md"),
        "# Notes\n",
    )
    .unwrap();
    let host = LocalHost::new(project, temporary.path().join("global"));
    let args = vec![
        "skilld".to_owned(),
        "run".to_owned(),
        skill.display().to_string(),
        "--plain".to_owned(),
    ];

    let (_, unix, unix_error) = run_cli_on(&host, args.clone(), CommandPlatform::Unix);
    let (_, powershell, powershell_error) =
        run_cli_on(&host, args, CommandPlatform::WindowsPowerShell);
    let mut human_stdout = Vec::new();
    let mut human_stderr = Vec::new();
    let human_result = run_with_output(
        ["skilld", "run", skill.to_str().unwrap()],
        &host,
        OutputContext::HumanTerminal {
            width: 120,
            color: false,
            platform: CommandPlatform::WindowsPowerShell,
        },
        &mut human_stdout,
        &mut human_stderr,
    );
    let human = String::from_utf8(human_stdout).unwrap();

    assert!(unix_error.is_empty());
    assert!(powershell_error.is_empty());
    assert_eq!(human_result.exit_code, 0);
    assert!(human_stderr.is_empty());
    assert!(unix.contains("project'\\''s & $(printf injected)"));
    assert!(unix.contains("'--file=reference'\\''s & $(printf injected).md'"));
    assert!(powershell.contains("project''s & $(printf injected)"));
    assert!(powershell.contains("'--file=reference''s & $(printf injected).md'"));
    assert!(!powershell.contains("'\\''"));
    assert!(human.contains("'--file=reference''s & $(printf injected).md'"));
}

#[cfg(unix)]
#[test]
fn local_run_rejects_terminal_formatting_in_the_source_root_without_stdout() {
    for control in ['\n', '\t', '\u{0085}', '\u{202e}'] {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join(format!("project{control}forged"));
        let skill = project.join("my-skill");
        write_local_skill(&skill, "my-skill");
        let host = LocalHost::new(
            temporary.path().join("project"),
            temporary.path().join("global"),
        );

        let (exit, stdout, stderr) = run_cli(
            &host,
            vec![
                "skilld".to_owned(),
                "run".to_owned(),
                skill.display().to_string(),
                "--plain".to_owned(),
            ],
        );

        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert!(printable_lines(&stderr));
    }
}

#[cfg(unix)]
#[test]
fn local_run_rejects_terminal_formatting_in_file_names_without_stdout() {
    for control in ['\n', '\t', '\u{0085}', '\u{202e}'] {
        let temporary = tempfile::tempdir().unwrap();
        let skill = temporary.path().join("my-skill");
        write_local_skill(&skill, "my-skill");
        fs::write(
            skill.join(format!("reference{control}forged.md")),
            "# Notes\n",
        )
        .unwrap();
        let host = LocalHost::new(
            temporary.path().join("project"),
            temporary.path().join("global"),
        );

        let (exit, stdout, stderr) = run_cli(
            &host,
            vec![
                "skilld".to_owned(),
                "run".to_owned(),
                skill.display().to_string(),
                "--plain".to_owned(),
            ],
        );

        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert!(printable_lines(&stderr));
    }
}

#[test]
fn plain_run_removes_terminal_formatting_but_json_preserves_text() {
    let instructions = "---\nname: vue\ndescription: Test.\n---\n\n# Start\n\u{1b}[2JCSI\n\u{1b}]0;forged\u{7}OSC\tkept\n\u{202e}reordered\n";
    let supporting =
        "before\u{1b}[31mred\u{1b}[0m\n\u{1b}]8;;https://example.com\u{7}link\n\u{2067}isolated\n";
    let fixture = remote_fixture(vec![
        file("SKILL.md", 0o644, instructions.as_bytes()),
        file("references/api.md", 0o644, supporting.as_bytes()),
    ]);

    let (_, loaded_plain, _) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
        ],
    );
    let (_, pulled_plain, _) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--revision".to_owned(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            "--file".to_owned(),
            "references/api.md".to_owned(),
        ],
    );
    let (_, loaded_json, _) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--json".to_owned(),
        ],
    );
    let json: serde_json::Value = serde_json::from_str(&loaded_json).unwrap();
    let (_, pulled_json, _) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--revision".to_owned(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            "--file".to_owned(),
            "references/api.md".to_owned(),
            "--json".to_owned(),
        ],
    );
    let pulled_json: serde_json::Value = serde_json::from_str(&pulled_json).unwrap();

    assert!(
        loaded_plain
            .chars()
            .chain(pulled_plain.chars())
            .all(|character| !character.is_control() || matches!(character, '\n' | '\t'))
    );
    assert!(!loaded_plain.contains('\u{202e}'));
    assert!(!pulled_plain.contains('\u{2067}'));
    assert_eq!(json["data"]["instructions"], instructions);
    assert_eq!(
        pulled_json["data"]["files"][0]["content"]["value"],
        supporting
    );
}

#[test]
fn a_plain_file_read_reports_provenance_and_unverified_status() {
    let fixture = remote_fixture(skill_files());

    let (_, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--revision".to_owned(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            "--file".to_owned(),
            "references/api.md".to_owned(),
        ],
    );

    assert!(stderr.is_empty());
    assert!(stdout.contains("Source: skilld:vuejs/core/vue\n"));
    assert!(stdout.contains(&format!("Revision: {}\n", "a".repeat(40))));
    assert!(stdout.contains("Source status: unverified\n"));
    assert!(stdout.contains("skilld did not check this source."));
}

#[test]
fn duplicate_file_requests_fail_before_remote_content_is_loaded() {
    let fixture = remote_fixture(skill_files());

    let (exit, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--file".to_owned(),
            "references/api.md".to_owned(),
            "--file".to_owned(),
            "references/api.md".to_owned(),
            "--json".to_owned(),
        ],
    );
    let error: serde_json::Value = serde_json::from_str(&stderr).unwrap();

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(error["error"]["code"], "INVALID_SOURCE");
    assert_eq!(fixture.remote.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn remote_file_requests_reject_c1_control_characters_before_fetch() {
    let fixture = remote_fixture(skill_files());

    let (exit, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--revision".to_owned(),
            "a".repeat(40),
            "--file=references/api\u{0085}forged.md".to_owned(),
            "--json".to_owned(),
        ],
    );
    let error: serde_json::Value = serde_json::from_str(&stderr).unwrap();

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(error["error"]["code"], "INVALID_SOURCE");
    assert_eq!(fixture.remote.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.remote.exact_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn remote_file_requests_reject_bidi_formatting_before_fetch() {
    let fixture = remote_fixture(skill_files());

    let (exit, stdout, stderr) = run_cli(
        &fixture.host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld:vuejs/core/vue".to_owned(),
            "--revision".to_owned(),
            "a".repeat(40),
            "--file=references/api\u{202e}forged.md".to_owned(),
            "--json".to_owned(),
        ],
    );
    let error: serde_json::Value = serde_json::from_str(&stderr).unwrap();

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(error["error"]["code"], "INVALID_SOURCE");
    assert_eq!(fixture.remote.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.remote.exact_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn direct_run_errors_use_run_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let host = LocalHost::new(
        temporary.path().join("project"),
        temporary.path().join("global"),
    );

    let local = run_cli(
        &host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "./my-skill".to_owned(),
            "--direct".to_owned(),
            "--json".to_owned(),
        ],
    );
    let bundled = run_cli(
        &host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            "skilld".to_owned(),
            "--direct".to_owned(),
            "--json".to_owned(),
        ],
    );
    let local_error: serde_json::Value = serde_json::from_str(&local.2).unwrap();
    let bundled_error: serde_json::Value = serde_json::from_str(&bundled.2).unwrap();

    assert_eq!(local.0, 2);
    assert_eq!(
        local_error["error"]["message"],
        "--direct cannot run a local Skill. Remove --direct, then run the same command again."
    );
    assert_eq!(bundled.0, 2);
    assert_eq!(
        bundled_error["error"]["message"],
        "--direct cannot run the skilld-maintained Skill. Run skilld run skilld without --direct."
    );
}

fn write_local_skill(path: &Path, frontmatter_name: &str) {
    fs::create_dir_all(path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {frontmatter_name}\ndescription: Test.\n---\n\n# Test\n"),
    )
    .unwrap();
}

fn local_run_error(path: &Path) -> serde_json::Value {
    let host = LocalHost::new(
        path.parent().unwrap().to_path_buf(),
        path.parent().unwrap().join("global"),
    );
    let (exit, stdout, stderr) = run_cli(
        &host,
        vec![
            "skilld".to_owned(),
            "run".to_owned(),
            path.display().to_string(),
            "--json".to_owned(),
        ],
    );
    assert_eq!(exit, 1);
    assert!(stdout.is_empty());
    serde_json::from_str(&stderr).unwrap()
}

#[test]
fn a_local_run_rejects_frontmatter_name_drift() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = temporary.path().join("my-skill");
    write_local_skill(&skill, "another-skill");

    let error = local_run_error(&skill);

    assert_eq!(error["error"]["code"], "INVALID_SOURCE");
}

#[test]
fn a_local_run_rejects_excess_depth_without_a_partial_inventory() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = temporary.path().join("my-skill");
    write_local_skill(&skill, "my-skill");
    let mut deep = skill.clone();
    for index in 0..9 {
        deep.push(format!("level-{index}"));
    }
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("secret.md"), "never partially returned").unwrap();

    let error = local_run_error(&skill);

    assert_eq!(error["error"]["code"], "SKILL_TOO_LARGE");
}

#[test]
fn a_local_run_rejects_excess_files_without_a_partial_inventory() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = temporary.path().join("my-skill");
    write_local_skill(&skill, "my-skill");
    for index in 0..512 {
        fs::write(skill.join(format!("file-{index}.md")), "x").unwrap();
    }

    let error = local_run_error(&skill);

    assert_eq!(error["error"]["code"], "SKILL_TOO_LARGE");
}

#[test]
fn a_local_run_rejects_a_sparse_file_before_reading_its_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let skill = temporary.path().join("my-skill");
    write_local_skill(&skill, "my-skill");
    File::create(skill.join("huge.bin"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();

    let error = local_run_error(&skill);

    assert_eq!(error["error"]["code"], "SKILL_TOO_LARGE");
}

#[cfg(unix)]
#[test]
fn a_local_run_rejects_non_utf8_paths_and_links() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let non_utf8 = temporary.path().join("non-utf8");
    write_local_skill(&non_utf8, "non-utf8");
    fs::write(non_utf8.join(OsString::from_vec(vec![0xff])), "x").unwrap();
    let linked = temporary.path().join("linked");
    write_local_skill(&linked, "linked");
    symlink(linked.join("SKILL.md"), linked.join("copy.md")).unwrap();

    let non_utf8_error = local_run_error(&non_utf8);
    let link_error = local_run_error(&linked);

    assert_eq!(non_utf8_error["error"]["code"], "INVALID_SOURCE");
    assert_eq!(link_error["error"]["code"], "INVALID_SOURCE");
}
