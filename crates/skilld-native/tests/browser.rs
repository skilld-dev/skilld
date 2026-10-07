use skilld_auth::ServiceOrigin;
use skilld_native::{BrowserCommand, api_origin, auth_browser_command, token_override};

#[test]
fn windows_passes_the_complete_authorization_url_to_the_native_opener() {
    let url = "https://skilld.dev/auth/cli?code=a&state=b%23c&redirect_uri=http%3A%2F%2F127.0.0.1";

    let command = auth_browser_command("windows", url, &ServiceOrigin::production()).unwrap();

    assert_eq!(command, BrowserCommand::WindowsUrl(url.to_owned()));
}

#[test]
fn browser_launch_rejects_another_origin() {
    let error = auth_browser_command(
        "windows",
        "https://example.com/auth?state=a&code=b",
        &ServiceOrigin::production(),
    )
    .unwrap_err();

    assert_eq!(error.code, "INVALID_AUTH_URL");
}

#[test]
fn browser_launch_follows_the_configured_origin_and_refuses_production_there() {
    let origin = ServiceOrigin::parse("http://localhost:3000").unwrap();

    let command =
        auth_browser_command("linux", "http://localhost:3000/cli/authorize?a=b", &origin).unwrap();
    let error =
        auth_browser_command("linux", "https://skilld.dev/cli/authorize?a=b", &origin).unwrap_err();

    assert_eq!(
        command,
        BrowserCommand::Process {
            program: "xdg-open",
            arguments: vec!["http://localhost:3000/cli/authorize?a=b".to_owned()],
        }
    );
    assert_eq!(error.code, "INVALID_AUTH_URL");
}

#[test]
fn browser_launch_rejects_a_nul_before_the_native_opener_can_truncate_it() {
    let error = auth_browser_command(
        "windows",
        "https://skilld.dev/auth/cli?state=a\0&code=b",
        &ServiceOrigin::production(),
    )
    .unwrap_err();

    assert_eq!(error.code, "INVALID_AUTH_URL");
}

#[cfg(windows)]
#[test]
fn windows_url_opener_delivers_the_complete_url_to_the_registered_handler() {
    use std::{
        fs,
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    let temporary = tempfile::tempdir().unwrap();
    let captured = temporary.path().join("captured-url.txt");
    let scheme = format!("skilld-browser-test-{}", std::process::id());
    let key = format!(r"HKCU\Software\Classes\{scheme}");
    let url = format!(
        "{scheme}://authorize?code=a&state=b%23c&redirect_uri=http%3A%2F%2F127.0.0.1#fragment"
    );
    // A private protocol exercises ShellExecuteW without changing browser defaults.
    // The test executable acts as the handler, so no browser or account is needed.
    let command = format!(
        "\"{}\" --ignored --exact windows_browser_capture \"{}\" \"%1\"",
        std::env::current_exe().unwrap().display(),
        captured.display()
    );
    let result = std::panic::catch_unwind(|| {
        for args in [
            vec![
                "add".to_owned(),
                key.clone(),
                "/v".to_owned(),
                "URL Protocol".to_owned(),
                "/d".to_owned(),
                String::new(),
                "/f".to_owned(),
            ],
            vec![
                "add".to_owned(),
                format!(r"{key}\shell\open\command"),
                "/ve".to_owned(),
                "/d".to_owned(),
                command,
                "/f".to_owned(),
            ],
        ] {
            let output = Command::new("reg.exe").args(args).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        BrowserCommand::WindowsUrl(url.clone()).open().unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if fs::read_to_string(&captured).is_ok_and(|value| value == url) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "URL handler did not receive {url:?}: {:?}",
                fs::read_to_string(&captured)
            );
            thread::sleep(Duration::from_millis(20));
        }
    });
    let cleanup = Command::new("reg.exe")
        .args(["delete", &key, "/f"])
        .output()
        .unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    assert!(
        cleanup.status.success(),
        "{}",
        String::from_utf8_lossy(&cleanup.stderr)
    );
}

#[cfg(windows)]
#[test]
#[ignore = "Launched by the Windows URL handler test"]
fn windows_browser_capture() {
    let args = std::env::args().collect::<Vec<_>>();
    let url = args.last().unwrap();
    assert!(url.starts_with("skilld-browser-test-"));
    std::fs::write(&args[args.len() - 2], url).unwrap();
    // Opening a URL succeeds independently of the handler's eventual exit code.
    std::process::exit(1);
}

#[test]
fn the_api_origin_defaults_to_production_and_parses_an_override_once() {
    assert!(api_origin(None).unwrap().is_production());
    assert!(
        api_origin(Some(std::ffi::OsStr::new("")))
            .unwrap()
            .is_production()
    );
    assert_eq!(
        api_origin(Some(std::ffi::OsStr::new("http://127.0.0.1:8787/")))
            .unwrap()
            .as_str(),
        "http://127.0.0.1:8787"
    );
    let error = api_origin(Some(std::ffi::OsStr::new("http://example.com"))).unwrap_err();
    assert_eq!(error.code, "INVALID_ENDPOINT");
    assert!(
        error.message.contains("SKILLD_API_URL"),
        "{}",
        error.message
    );
}

#[test]
fn a_token_in_the_environment_is_used_as_given_and_an_empty_one_is_ignored() {
    assert!(token_override(None).unwrap().is_none());
    assert!(
        token_override(Some(std::ffi::OsStr::new("  ")))
            .unwrap()
            .is_none()
    );
    let token = token_override(Some(std::ffi::OsStr::new(" eyJ.a.b \n")))
        .unwrap()
        .unwrap();
    assert_eq!(token.expose(), "eyJ.a.b");
    assert_eq!(
        token_override(Some(std::ffi::OsStr::new("a\nb")))
            .unwrap_err()
            .code,
        "INVALID_TOKEN"
    );
}
