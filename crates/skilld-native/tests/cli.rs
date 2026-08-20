use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_skilld"))
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/v3-rust/local-skill")
}

#[test]
fn version_reports_the_rust_package_version() {
    let output = Command::new(binary()).arg("--version").output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("skilld {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn local_fixture_install_and_list_use_project_state() {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let install = Command::new(binary())
        .current_dir(project.path())
        .env("SKILLD_DATA_DIR", data.path())
        .args(["install", fixture().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(install.status.success());
    assert_eq!(
        String::from_utf8(install.stdout).unwrap(),
        "Installed Skill local-skill.\n"
    );
    assert!(install.stderr.is_empty());
    assert_eq!(
        fs::read_to_string(project.path().join(".skills/local-skill/SKILL.md")).unwrap(),
        fs::read_to_string(fixture().join("SKILL.md")).unwrap()
    );

    let list = Command::new(binary())
        .current_dir(project.path())
        .env("SKILLD_DATA_DIR", data.path())
        .arg("list")
        .output()
        .unwrap();

    assert!(list.status.success());
    assert_eq!(String::from_utf8(list.stdout).unwrap(), "local-skill\n");
    assert!(list.stderr.is_empty());
}

#[test]
fn missing_native_credential_capability_is_explicit() {
    let project = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .current_dir(project.path())
        .args(["auth", "status"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "UNSUPPORTED_HOST: credential access is unavailable on this host\n"
    );
}

#[test]
fn global_skilld_install_uses_the_normal_global_store() {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let bundled = assets.path().join("skilld");
    fs::create_dir(&bundled).unwrap();
    fs::write(bundled.join("SKILL.md"), "fixture").unwrap();

    let output = Command::new(binary())
        .current_dir(project.path())
        .env("SKILLD_DATA_DIR", data.path())
        .env("SKILLD_BUNDLED_SKILL_DIR", &bundled)
        .args(["install", "skilld", "--global"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(data.path().join("skills/skilld/SKILL.md")).unwrap(),
        "fixture"
    );
    assert!(!project.path().join(".skills/skilld").exists());
}
