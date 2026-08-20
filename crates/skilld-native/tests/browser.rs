use skilld_native::auth_browser_command;

#[test]
fn windows_opens_the_validated_authorization_url_as_one_direct_argument() {
    let url = "https://skilld.dev/auth/cli?code=a&state=b&redirect_uri=http%3A%2F%2F127.0.0.1";

    let command = auth_browser_command("windows", url).unwrap();

    assert_eq!(command.program, "explorer.exe");
    assert_eq!(command.arguments, [url]);
}

#[test]
fn browser_launch_rejects_another_origin() {
    let error =
        auth_browser_command("windows", "https://example.com/auth?state=a&code=b").unwrap_err();

    assert_eq!(error.code, "INVALID_AUTH_URL");
}
