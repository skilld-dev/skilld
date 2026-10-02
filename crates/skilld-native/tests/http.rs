use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use skilld_command::{NativeRemoteConfig, NoTokenProvider, RemoteProvider, SkilldRemote};
use skilld_native::NativeHttpAdapter;

#[test]
fn native_http_adapter_reaches_the_v1_skill_search_route() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = include_bytes!("../../../contracts/fixtures/v1/skill-search.json").to_vec();
    let server = thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = connection.read(&mut request).unwrap();
        let request = std::str::from_utf8(&request[..read]).unwrap();
        assert!(request.starts_with("GET /api/v1/skills?q=testing&limit=20 HTTP/1.1\r\n"));
        write!(
            connection,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        connection.write_all(&body).unwrap();
    });
    let remote = SkilldRemote::new(
        Arc::new(NativeHttpAdapter::new()),
        Arc::new(NoTokenProvider),
        NativeRemoteConfig::Unconfigured,
    )
    .with_endpoint(&format!("http://{address}"))
    .unwrap();

    let response = remote.search("testing", 20).unwrap();

    assert_eq!(response.items[0].name, "vue-testing");
    assert_eq!(response.total, 1);
    server.join().unwrap();
}

/// Answers one request with the search fixture and returns the request line.
fn serve_one_search(listener: TcpListener) -> thread::JoinHandle<String> {
    let body = include_bytes!("../../../contracts/fixtures/v1/skill-search.json").to_vec();
    thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = connection.read(&mut request).unwrap();
        let request = std::str::from_utf8(&request[..read]).unwrap().to_owned();
        write!(
            connection,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        connection.write_all(&body).unwrap();
        request.lines().next().unwrap_or_default().to_owned()
    })
}

#[test]
fn skilld_api_url_points_the_cli_at_a_local_site() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = serve_one_search(listener);
    let temporary = tempfile::tempdir().unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_skilld"))
        .current_dir(temporary.path())
        .env("SKILLD_DATA_DIR", temporary.path().join("data"))
        .env("HOME", temporary.path())
        .env("SKILLD_NO_UPGRADE", "1")
        .env("SKILLD_API_URL", format!("http://{address}"))
        .args(["search", "testing", "--json"])
        .output()
        .unwrap();

    assert_eq!(
        server.join().unwrap(),
        "GET /api/v1/skills?q=testing&limit=20 HTTP/1.1"
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["data"]["items"][0]["name"], "vue-testing");
}

#[test]
fn an_invalid_skilld_api_url_stops_before_any_request() {
    let temporary = tempfile::tempdir().unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_skilld"))
        .current_dir(temporary.path())
        .env("SKILLD_DATA_DIR", temporary.path().join("data"))
        .env("HOME", temporary.path())
        .env("SKILLD_NO_UPGRADE", "1")
        .env("SKILLD_API_URL", "http://example.com")
        .args(["search", "testing"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("INVALID_ENDPOINT: SKILLD_API_URL"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
}
