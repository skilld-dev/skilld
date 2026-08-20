use std::env;
use std::path::PathBuf;

use skilld_command::{CommandError, Host, LocalHost, run, run_stdio_probe};
use skilld_core::{InstallScope, InstallSource};

wit_bindgen::generate!({
    path: "../../wit",
    world: "command",
});

struct SkilldComponent;

impl Guest for SkilldComponent {
    fn run() -> u32 {
        if env::var_os("SKILLD_PROBE_GIT").as_deref() == Some(std::ffi::OsStr::new("1")) {
            return run_git_probe();
        }
        if env::var_os("SKILLD_PROBE_STDIO").as_deref() == Some(std::ffi::OsStr::new("1")) {
            let mut stdin = std::io::stdin().lock();
            let mut stdout = std::io::stdout().lock();
            let mut stderr = std::io::stderr().lock();
            return run_stdio_probe(&mut stdin, &mut stdout, &mut stderr)
                .exit_code
                .into();
        }

        let project_root = PathBuf::from(".");
        let global_root = global_root();
        let mut local = LocalHost::new(project_root, global_root);
        if let Some(path) = env::var_os("SKILLD_BUNDLED_SKILL_DIR") {
            local = local.with_bundled_skill(PathBuf::from(path));
        }
        let host = WasiHost { local };
        let mut stdout = std::io::stdout().lock();
        let mut stderr = std::io::stderr().lock();
        let result = run(env::args_os(), &host, &mut stdout, &mut stderr);
        result.exit_code.into()
    }
}

fn run_git_probe() -> u32 {
    let command = skilld::host::process::GitCommand {
        args: vec!["--version".to_owned()],
        cwd: None,
        stdin: None,
        max_output_bytes: 4096,
        timeout_ms: 5000,
    };
    match skilld::host::process::run_git(&command) {
        Ok(output) => {
            print!("{}", String::from_utf8_lossy(&output.stdout));
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            if (0..=255).contains(&output.exit_code) {
                output.exit_code as u32
            } else {
                2
            }
        }
        Err(error) => {
            eprintln!("UNSUPPORTED_HOST: Git process capability failed: {error:?}");
            2
        }
    }
}

struct WasiHost {
    local: LocalHost,
}

impl Host for WasiHost {
    fn list(&self, scope: InstallScope) -> Result<Vec<String>, CommandError> {
        self.local.list(scope)
    }

    fn install(&self, source: InstallSource, scope: InstallScope) -> Result<String, CommandError> {
        self.local.install(source, scope)
    }

    fn auth_status(&self) -> Result<bool, CommandError> {
        skilld::host::credentials::get("skilld.dev", "default")
            .map(|credential| credential.is_some())
            .map_err(|error| credential_error("read", error))
    }

    fn auth_login(&self) -> Result<(), CommandError> {
        skilld::host::process::open_url("https://skilld.dev/auth/cli")
            .map_err(|error| process_error("open the authentication URL", error))
    }

    fn auth_logout(&self) -> Result<(), CommandError> {
        skilld::host::credentials::delete("skilld.dev", "default")
            .map(|_| ())
            .map_err(|error| credential_error("delete", error))
    }
}

fn credential_error(operation: &str, error: skilld::host::credentials::ErrorCode) -> CommandError {
    CommandError::unsupported_host(format!(
        "credential capability could not {operation} the account: {error:?}"
    ))
}

fn process_error(operation: &str, error: skilld::host::process::ErrorCode) -> CommandError {
    CommandError::unsupported_host(format!(
        "process capability could not {operation}: {error:?}"
    ))
}

fn global_root() -> PathBuf {
    if let Some(path) = env::var_os("SKILLD_DATA_DIR") {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(path).join("skilld");
    }
    if let Some(path) = env::var_os("HOME") {
        return PathBuf::from(path).join(".skilld");
    }
    PathBuf::from(".skilld")
}

export!(SkilldComponent);
