#[cfg(not(target_family = "wasm"))]
use std::io::{ErrorKind, Read, Write};
#[cfg(not(target_family = "wasm"))]
use std::net::{Ipv4Addr, TcpListener, TcpStream};
#[cfg(not(target_family = "wasm"))]
use std::thread;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;
#[cfg(not(target_family = "wasm"))]
use std::time::Instant;

use subtle::ConstantTimeEq;
use url::Url;

use crate::model::{
    AuthError, AuthErrorKind, AuthorizationCode, BoundaryError, BoundaryErrorKind, CallbackBinding,
    CallbackListener, CallbackRequest,
};
#[cfg(not(target_family = "wasm"))]
use crate::model::{CallbackReply, CancellationToken};

const MAX_CALLBACK_REQUEST_BYTES: usize = 8 * 1024;
#[cfg(not(target_family = "wasm"))]
const POLL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(not(target_family = "wasm"))]
const READ_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(not(target_family = "wasm"))]
const ACCEPTED_BODY: &str = "<!doctype html><meta charset=utf-8><title>skilld signed in</title><h1>Signed in to skilld</h1><p>Return to the CLI.</p>";
#[cfg(not(target_family = "wasm"))]
const REJECTED_BODY: &str = "Authorization was rejected.";

fn callback_error(kind: AuthErrorKind, message: &'static str) -> AuthError {
    AuthError::new(kind, message)
}

#[must_use = "the callback result must be handled"]
pub fn parse_callback_request(
    request: &CallbackRequest,
    expected_port: u16,
    expected_state: &str,
) -> Result<AuthorizationCode, AuthError> {
    if request.bytes.len() > MAX_CALLBACK_REQUEST_BYTES {
        return Err(callback_error(
            AuthErrorKind::ResponseTooLarge,
            "The account callback exceeded its byte limit.",
        ));
    }
    if !request.peer.ip().is_loopback() {
        return Err(callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback did not come from the loopback interface.",
        ));
    }
    if request.local_port != expected_port {
        return Err(callback_error(
            AuthErrorKind::CallbackPortMismatch,
            "The account callback used a different port.",
        ));
    }

    let terminator = request
        .bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .ok_or_else(|| {
            callback_error(
                AuthErrorKind::CallbackInjection,
                "The account callback headers were incomplete.",
            )
        })?;
    if terminator != request.bytes.len() {
        return Err(callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback contained unexpected data.",
        ));
    }
    let text = std::str::from_utf8(&request.bytes[..terminator - 4]).map_err(|_| {
        callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback was not valid UTF-8.",
        )
    })?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let parts = request_line.split(' ').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "GET" || parts[2] != "HTTP/1.1" {
        return Err(callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback request line was invalid.",
        ));
    }
    let target = parts[1];
    if !target.starts_with("/?") || target.contains('#') {
        return Err(callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback target was invalid.",
        ));
    }

    let mut host = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(callback_error(
                AuthErrorKind::CallbackInjection,
                "The account callback contained an invalid header.",
            ));
        };
        if !name.bytes().all(is_header_name_byte)
            || value
                .bytes()
                .any(|byte| (byte < b' ' && byte != b'\t') || byte == 0x7f)
        {
            return Err(callback_error(
                AuthErrorKind::CallbackInjection,
                "The account callback contained an invalid header.",
            ));
        }
        if name.eq_ignore_ascii_case("host") {
            if host.is_some() {
                return Err(callback_error(
                    AuthErrorKind::CallbackInjection,
                    "The account callback contained duplicate host headers.",
                ));
            }
            host = Some(value.trim());
        }
        if name.eq_ignore_ascii_case("content-length") && value.trim() != "0" {
            return Err(callback_error(
                AuthErrorKind::CallbackInjection,
                "The account callback contained a request body.",
            ));
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(callback_error(
                AuthErrorKind::CallbackInjection,
                "The account callback contained a request body.",
            ));
        }
    }
    let expected_v4 = format!("127.0.0.1:{expected_port}");
    let expected_v6 = format!("[::1]:{expected_port}");
    if !matches!(host, Some(value) if value == expected_v4 || value == expected_v6) {
        return Err(callback_error(
            AuthErrorKind::CallbackPortMismatch,
            "The account callback host did not match its listener.",
        ));
    }

    let url = Url::parse(&format!("http://127.0.0.1:{expected_port}{target}")).map_err(|_| {
        callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback URL was invalid.",
        )
    })?;
    if url.path() != "/" {
        return Err(callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback path was invalid.",
        ));
    }
    let pairs = url.query_pairs().collect::<Vec<_>>();
    if pairs.len() != 2
        || pairs.iter().filter(|(name, _)| name == "code").count() != 1
        || pairs.iter().filter(|(name, _)| name == "state").count() != 1
    {
        return Err(callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback query was invalid.",
        ));
    }
    let code = pairs
        .iter()
        .find_map(|(name, value)| (name == "code").then_some(value.as_ref()))
        .unwrap_or_default();
    let state = pairs
        .iter()
        .find_map(|(name, value)| (name == "state").then_some(value.as_ref()))
        .unwrap_or_default();
    if !(16..=512).contains(&code.len()) || !code.bytes().all(is_base64_url_byte) {
        return Err(callback_error(
            AuthErrorKind::CallbackInjection,
            "The account callback code was invalid.",
        ));
    }
    if state
        .as_bytes()
        .ct_eq(expected_state.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Err(callback_error(
            AuthErrorKind::StateMismatch,
            "The account callback state did not match.",
        ));
    }
    Ok(AuthorizationCode::new(code.to_owned()))
}

fn is_base64_url_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn is_header_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(byte, b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^'..=b'`' | b'|' | b'~')
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeLoopbackListener;

impl CallbackListener for NativeLoopbackListener {
    #[cfg(not(target_family = "wasm"))]
    fn bind(&self) -> Result<Box<dyn CallbackBinding>, BoundaryError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| BoundaryError::new(BoundaryErrorKind::Failed))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| BoundaryError::new(BoundaryErrorKind::Failed))?;
        let port = listener
            .local_addr()
            .map_err(|_| BoundaryError::new(BoundaryErrorKind::Failed))?
            .port();
        Ok(Box::new(NativeBinding {
            listener,
            port,
            pending: None,
        }))
    }

    #[cfg(target_family = "wasm")]
    fn bind(&self) -> Result<Box<dyn CallbackBinding>, BoundaryError> {
        Err(BoundaryError::new(BoundaryErrorKind::Unsupported))
    }
}

#[cfg(not(target_family = "wasm"))]
struct NativeBinding {
    listener: TcpListener,
    port: u16,
    pending: Option<TcpStream>,
}

#[cfg(not(target_family = "wasm"))]
impl CallbackBinding for NativeBinding {
    fn port(&self) -> u16 {
        self.port
    }

    fn receive(
        &mut self,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<CallbackRequest, BoundaryError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| BoundaryError::new(BoundaryErrorKind::Timeout))?;
        loop {
            if cancellation.is_cancelled() {
                return Err(BoundaryError::new(BoundaryErrorKind::Cancelled));
            }
            if Instant::now() >= deadline {
                return Err(BoundaryError::new(BoundaryErrorKind::Timeout));
            }
            match self.listener.accept() {
                Ok((mut stream, peer)) => {
                    let bytes = read_request(&mut stream, deadline, cancellation)?;
                    self.pending = Some(stream);
                    return Ok(CallbackRequest {
                        bytes,
                        peer,
                        local_port: self.port,
                    });
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(POLL_INTERVAL);
                }
                Err(_) => return Err(BoundaryError::new(BoundaryErrorKind::Failed)),
            }
        }
    }

    fn respond(&mut self, reply: CallbackReply) -> Result<(), BoundaryError> {
        let mut stream = self
            .pending
            .take()
            .ok_or_else(|| BoundaryError::new(BoundaryErrorKind::Failed))?;
        let (status, content_type, body) = match reply {
            CallbackReply::Accepted => ("200 OK", "text/html", ACCEPTED_BODY),
            CallbackReply::Rejected => ("400 Bad Request", "text/plain", REJECTED_BODY),
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}; charset=utf-8\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .and_then(|()| stream.flush())
            .map_err(|_| BoundaryError::new(BoundaryErrorKind::Failed))
    }
}

#[cfg(not(target_family = "wasm"))]
fn read_request(
    stream: &mut TcpStream,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, BoundaryError> {
    stream
        .set_read_timeout(Some(READ_INTERVAL))
        .map_err(|_| BoundaryError::new(BoundaryErrorKind::Failed))?;
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        if cancellation.is_cancelled() {
            return Err(BoundaryError::new(BoundaryErrorKind::Cancelled));
        }
        if Instant::now() >= deadline {
            return Err(BoundaryError::new(BoundaryErrorKind::Timeout));
        }
        match stream.read(&mut buffer) {
            Ok(0) => return Err(BoundaryError::new(BoundaryErrorKind::Failed)),
            Ok(read) => {
                request.extend_from_slice(&buffer[..read]);
                if request.len() > MAX_CALLBACK_REQUEST_BYTES {
                    return Err(BoundaryError::new(BoundaryErrorKind::ResponseTooLarge));
                }
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    return Ok(request);
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return Err(BoundaryError::new(BoundaryErrorKind::Failed)),
        }
    }
}
