use skilld_auth::ServiceOrigin;
use skilld_native::{api_origin, auth_browser_command};

#[test]
fn windows_opens_the_validated_authorization_url_as_one_direct_argument() {
    let url = "https://skilld.dev/auth/cli?code=a&state=b&redirect_uri=http%3A%2F%2F127.0.0.1";

    let command = auth_browser_command("windows", url, &ServiceOrigin::production()).unwrap();

    assert_eq!(command.program, "explorer.exe");
    assert_eq!(command.arguments, [url]);
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
        command.arguments,
        ["http://localhost:3000/cli/authorize?a=b"]
    );
    assert_eq!(error.code, "INVALID_AUTH_URL");
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
