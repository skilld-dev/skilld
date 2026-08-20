use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use skilld_command::{LocalHost, run, run_stdio_probe};

fn main() -> ExitCode {
    if env::var_os("SKILLD_PROBE_STDIO").as_deref() == Some(std::ffi::OsStr::new("1")) {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        let mut stderr = std::io::stderr().lock();
        let result = run_stdio_probe(&mut stdin, &mut stdout, &mut stderr);
        return ExitCode::from(result.exit_code);
    }

    let project_root = match env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("SERVICE_UNAVAILABLE: cannot read the current directory: {error}");
            return ExitCode::from(2);
        }
    };
    let global_root = global_root();
    let mut host = LocalHost::new(project_root, global_root);
    if let Some(path) = env::var_os("SKILLD_BUNDLED_SKILL_DIR") {
        host = host.with_bundled_skill(PathBuf::from(path));
    }

    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let result = run(env::args_os(), &host, &mut stdout, &mut stderr);
    ExitCode::from(result.exit_code)
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
