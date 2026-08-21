use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use skilld_command::{Cancellation, HttpAdapter, HttpMethod, HttpRequest, HttpResponse};
use skilld_core::RemoteError;
use url::Url;

#[cfg(not(target_os = "wasi"))]
pub mod update_ui;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserCommand {
    pub program: &'static str,
    pub arguments: Vec<String>,
}

pub fn auth_browser_command(
    platform: &str,
    authorization_url: &str,
) -> Result<BrowserCommand, RemoteError> {
    let url = Url::parse(authorization_url)
        .map_err(|_| RemoteError::new("INVALID_AUTH_URL", "the authorization URL is invalid"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("skilld.dev")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(RemoteError::new(
            "INVALID_AUTH_URL",
            "the authorization URL must stay on skilld.dev",
        ));
    }
    let program = match platform {
        "macos" => "open",
        "linux" => "xdg-open",
        "windows" => "explorer.exe",
        _ => {
            return Err(RemoteError::new(
                "UNSUPPORTED_HOST",
                "this host cannot open the authorization URL",
            ));
        }
    };
    Ok(BrowserCommand {
        program,
        arguments: vec![authorization_url.to_owned()],
    })
}

#[derive(Clone, Debug)]
pub struct NativeHttpAdapter {
    agent: ureq::Agent,
}

impl NativeHttpAdapter {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent("skilld/3")
            .build()
            .into();
        Self { agent }
    }
}

impl Default for NativeHttpAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpAdapter for NativeHttpAdapter {
    fn send(
        &self,
        request: &HttpRequest,
        cancellation: &dyn Cancellation,
        timeout: Option<Duration>,
    ) -> Result<HttpResponse, RemoteError> {
        if cancellation.is_cancelled() {
            return Err(RemoteError::new(
                "CANCELLED",
                "the remote operation was cancelled",
            ));
        }
        let response = match request.method {
            HttpMethod::Get => {
                let mut builder = self.agent.get(&request.url);
                for header in &request.headers {
                    builder = builder.header(&header.name, header.value.expose());
                }
                if let Some(timeout) = timeout {
                    builder = builder
                        .config()
                        .timeout_global(Some(timeout.min(Duration::from_secs(30))))
                        .build();
                }
                builder.call()
            }
            HttpMethod::Post => {
                let mut builder = self.agent.post(&request.url);
                for header in &request.headers {
                    builder = builder.header(&header.name, header.value.expose());
                }
                if let Some(timeout) = timeout {
                    builder = builder
                        .config()
                        .timeout_global(Some(timeout.min(Duration::from_secs(30))))
                        .build();
                }
                builder.send(request.body.as_slice())
            }
        }
        .map_err(|error| transport_error(&error, &request.url))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_ascii_lowercase(), value.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        if headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .is_some_and(|length| length > request.response_limit)
        {
            return Err(RemoteError::new(
                "RESPONSE_TOO_LARGE",
                "a remote response exceeded its limit",
            ));
        }
        let mut body = response.into_body();
        let mut reader = body.as_reader();
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if cancellation.is_cancelled() {
                return Err(RemoteError::new(
                    "CANCELLED",
                    "the remote operation was cancelled",
                ));
            }
            let read = reader.read(&mut buffer).map_err(|error| {
                let reason = match error.kind() {
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "timed out",
                    _ => "could not be read",
                };
                RemoteError::new(
                    "HTTP_TRANSPORT",
                    format!("the remote response {reason}. Retry the command."),
                )
            })?;
            if read == 0 {
                break;
            }
            if bytes.len().saturating_add(read) > request.response_limit {
                return Err(RemoteError::new(
                    "RESPONSE_TOO_LARGE",
                    "a remote response exceeded its limit",
                ));
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        Ok(HttpResponse {
            status,
            headers,
            body: bytes,
        })
    }
}

fn transport_error(error: &ureq::Error, request_url: &str) -> RemoteError {
    let host = Url::parse(request_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "the remote service".to_owned());
    let reason = match error {
        ureq::Error::HostNotFound => format!("the {host} address could not be resolved"),
        ureq::Error::ConnectionFailed => format!("the connection to {host} failed"),
        ureq::Error::Timeout(_) => format!("the request to {host} timed out"),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_) => {
            format!("the secure connection to {host} failed")
        }
        ureq::Error::Io(io) => match io.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                format!("the request to {host} timed out")
            }
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionReset => {
                format!("the connection to {host} failed")
            }
            std::io::ErrorKind::NotFound => format!("the {host} address could not be resolved"),
            _ => "the remote request could not be completed".to_owned(),
        },
        _ => "the remote request could not be completed".to_owned(),
    };
    let timed_out = matches!(error, ureq::Error::Timeout(_))
        || matches!(error, ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut);
    let secure = matches!(
        error,
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_)
    );
    let recovery = if secure {
        "Check the system clock and certificates, then retry."
    } else if timed_out {
        "Retry the command. A slow network can cause this."
    } else {
        "Check the network connection, then retry the command."
    };
    RemoteError::new("HTTP_TRANSPORT", format!("{reason}. {recovery}"))
}

#[cfg(test)]
mod tests {
    use super::transport_error;

    fn message(error: ureq::Error) -> String {
        transport_error(&error, "https://skilld.dev/api/v1/skills").message
    }

    #[test]
    fn dns_failures_name_the_host_and_a_recovery_step() {
        assert_eq!(
            message(ureq::Error::HostNotFound),
            "the skilld.dev address could not be resolved. Check the network connection, then retry the command."
        );
    }

    #[test]
    fn connection_failures_name_the_host() {
        assert_eq!(
            message(ureq::Error::ConnectionFailed),
            "the connection to skilld.dev failed. Check the network connection, then retry the command."
        );
    }

    #[test]
    fn timeouts_say_so() {
        assert_eq!(
            message(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timed out"
            ))),
            "the request to skilld.dev timed out. Retry the command. A slow network can cause this."
        );
    }
}
