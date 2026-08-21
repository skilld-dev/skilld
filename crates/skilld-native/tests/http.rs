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
