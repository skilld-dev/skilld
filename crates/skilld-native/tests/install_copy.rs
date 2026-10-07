use std::fs;
use std::path::Path;
use std::process::Command;

fn run(project: &Path, home: &Path, data: &Path, args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_skilld"))
        .current_dir(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("SKILLD_DATA_DIR", data)
        .env("SKILLD_NO_UPGRADE", "1")
        .env("SKILLD_NO_WEEKLY", "1")
        .env_remove("CLAUDE_CONFIG_DIR")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn local_copy_install_replace_and_remove_without_symlink_privileges() {
    for global in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let home = temporary.path().join("home");
        let data = temporary.path().join("data");
        let source = project.join("src/demo");
        fs::create_dir_all(source.join("references")).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(source.join("references/check.md"), "supporting file").unwrap();
        let target = if global { &home } else { &project }.join(".claude/skills/demo");
        let mut install = vec![
            "install",
            "./src/demo",
            "--agent",
            "claude-code",
            "--mode",
            "copy",
        ];
        let mut remove = vec!["remove", "demo"];
        if global {
            install.push("--global");
            remove.push("--global");
        }

        for content in ["first", "replacement"] {
            let text = format!("---\nname: demo\ndescription: demo skill\n---\n{content}\n");
            fs::write(source.join("SKILL.md"), &text).unwrap();
            run(&project, &home, &data, &install);
            assert_eq!(fs::read_to_string(target.join("SKILL.md")).unwrap(), text);
            assert_eq!(
                fs::read_to_string(target.join("references/check.md")).unwrap(),
                "supporting file"
            );
            assert!(
                !fs::symlink_metadata(&target)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
        run(&project, &home, &data, &remove);
        assert!(!target.exists());
        assert!(source.join("SKILL.md").exists());
    }
}
