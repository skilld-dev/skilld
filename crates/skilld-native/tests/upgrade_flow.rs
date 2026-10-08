#![cfg(unix)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nix::pty::{Winsize, openpty};

struct Fixture {
    root: tempfile::TempDir,
    binary: PathBuf,
}

impl Fixture {
    fn new(package_exit: u8) -> Self {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("pnpm/global/v11/native/skilld");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::hard_link(env!("CARGO_BIN_EXE_skilld"), &binary).unwrap();
        let bin = root.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let manager = bin.join("pnpm");
        fs::write(
            &manager,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/manager-args\"\nexit {package_exit}\n"
            ),
        )
        .unwrap();
        fs::set_permissions(manager, fs::Permissions::from_mode(0o755)).unwrap();
        let fixture = Self { root, binary };
        fixture.cache("9.0.0");
        fixture
    }

    fn cache(&self, version: &str) {
        fs::write(
            self.root.path().join("upgrade.json"),
            serde_json::to_vec(&serde_json::json!({
                "checkedAt": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                "latest": version
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .env_clear()
            .env("HOME", self.root.path())
            .env("PATH", self.root.path().join("bin"))
            .env("SKILLD_DATA_DIR", self.root.path())
            .env("SKILLD_LAUNCHER", "pnpm")
            .env("SKILLD_NO_WEEKLY", "1")
            .env("TERM", "xterm-256color")
            .current_dir(self.root.path());
        command
    }

    fn terminal(
        &self,
        args: &[&str],
        answer: Option<&[u8]>,
        env: Option<(&str, &str)>,
    ) -> (bool, String) {
        let pair = openpty(
            Some(&Winsize {
                ws_row: 20,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )
        .unwrap();
        let mut writer = File::from(pair.master);
        let mut reader = writer.try_clone().unwrap();
        let slave = File::from(pair.slave);
        let mut command = self.command();
        command
            .args(args)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        if let Some((key, value)) = env {
            command.env(key, value);
        }
        // The child needs its own controlling terminal for crossterm input.
        unsafe {
            command.pre_exec(|| {
                if nix::libc::setsid() < 0 || nix::libc::ioctl(0, nix::libc::TIOCSCTTY, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        drop(command);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut output = String::new();
        let mut answer = answer;
        loop {
            match receiver.recv_timeout(Duration::from_secs(10)) {
                Ok(bytes) => {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                    if output.contains("continue.")
                        && let Some(keys) = answer.take()
                    {
                        writer.write_all(keys).unwrap();
                        writer.flush().unwrap();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("upgrade terminal timed out: {output}");
                }
            }
        }
        (child.wait().unwrap().success(), output)
    }
}

#[test]
fn restarting_with_a_cached_release_prompts_and_runs_the_package_manager() {
    let fixture = Fixture::new(0);
    let (success, output) = fixture.terminal(&["list", "--global"], Some(b"\r"), None);
    assert!(success, "{output}");
    assert!(output.contains("\x1b[38;2;"), "{output}");
    assert!(
        output.contains("Upgrade complete. Run your command again."),
        "{output}"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("manager-args")).unwrap(),
        "add\n--global\nskilld@9.0.0\n"
    );
    let restore = output.find("\x1b[?1049l").expect("terminal restored");
    assert!(restore < output.find("Running pnpm").unwrap());
}

#[test]
fn not_now_runs_the_original_command_without_installing() {
    let fixture = Fixture::new(0);
    let (success, output) = fixture.terminal(
        &["list", "--global"],
        Some(b"\x1b[B\r"),
        Some(("NO_COLOR", "1")),
    );
    assert!(success, "{output}");
    assert!(output.contains("Restart skilld to upgrade."), "{output}");
    assert!(!output.contains("\x1b[38;"), "{output}");
    assert!(!output.contains("\x1b[48;"), "{output}");
    assert!(!fixture.root.path().join("manager-args").exists());
    assert!(!fixture.root.path().join("upgrade-dismissed").exists());
}

#[test]
fn dismissal_skips_the_next_prompt_but_a_new_release_prompts_again() {
    let fixture = Fixture::new(0);
    let (success, output) = fixture.terminal(&["list", "--global"], Some(b"\x1b[A\r"), None);
    assert!(success, "{output}");
    let (success, output) = fixture.terminal(&["list", "--global"], None, None);
    assert!(success, "{output}");
    assert!(!output.contains("Don't"), "{output}");
    fixture.cache("9.1.0");
    let (success, output) = fixture.terminal(&["list", "--global"], Some(b"q"), None);
    assert!(success && output.contains("Don't"), "{output}");
}

#[test]
fn package_manager_failure_is_visible_and_never_reports_success() {
    let fixture = Fixture::new(7);
    let (success, output) = fixture.terminal(&["list", "--global"], Some(b"\r"), None);
    assert!(!success, "{output}");
    assert!(output.contains("UPGRADE_FAILED"), "{output}");
    assert!(!output.contains("Upgrade complete"), "{output}");
}

#[test]
fn agents_ci_opt_out_and_machine_output_never_prompt() {
    let fixture = Fixture::new(0);
    for signal in [
        ("AGENT_SESSION_ID", "test"),
        ("CI", "1"),
        ("SKILLD_NO_UPGRADE", "1"),
    ] {
        let (success, output) = fixture.terminal(&["list", "--global"], None, Some(signal));
        assert!(success, "{output}");
        assert!(!output.contains("Upgrade available"), "{output}");
    }
    for args in [
        &["update", "--check", "--global", "--json"][..],
        &["list", "--global", "--plain"][..],
        &["--version"][..],
        &["--help"][..],
    ] {
        let (success, output) = fixture.terminal(args, None, None);
        assert!(success, "{output}");
        assert!(!output.contains("\x1b[?1049h"), "{output}");
    }
    let output = fixture
        .command()
        .args(["list", "--global"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Upgrade available"));
    assert!(!fixture.root.path().join("manager-args").exists());
}
