use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use skilld_auth::ServiceOrigin;
use skilld_command::{
    Cancellation, HttpAdapter, HttpMethod, HttpRequest, HttpResponse, SecretValue,
};
use skilld_core::RemoteError;
use url::Url;

pub mod behavior_prompt;
#[cfg(not(target_os = "wasi"))]
pub mod select_ui;
pub mod update_ui;
#[cfg(not(target_os = "wasi"))]
pub mod upgrade;
pub mod weekly;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserCommand {
    pub program: &'static str,
    pub arguments: Vec<String>,
}

/// The command that opens one authorization URL in the browser.
///
/// The URL must stay on `origin`, the skilld.dev origin the account signs in
/// to. Production is `https://skilld.dev`; `SKILLD_API_URL` can name another.
pub fn auth_browser_command(
    platform: &str,
    authorization_url: &str,
    origin: &ServiceOrigin,
) -> Result<BrowserCommand, RemoteError> {
    let url = Url::parse(authorization_url)
        .map_err(|_| RemoteError::new("INVALID_AUTH_URL", "the authorization URL is invalid"))?;
    if !origin.contains(&url) {
        return Err(RemoteError::new(
            "INVALID_AUTH_URL",
            "the authorization URL must stay on the skilld.dev origin",
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

/// The environment variable that points the CLI at another skilld.dev origin.
pub const API_URL_VARIABLE: &str = "SKILLD_API_URL";
/// A skilld token for scripts and CI. It wins over the stored sign-in.
pub const TOKEN_VARIABLE: &str = "SKILLD_TOKEN";

/// The skilld.dev origin this run talks to.
///
/// `SKILLD_API_URL` names a local or preview site. Unset or empty, the CLI
/// uses `https://skilld.dev`. Remote Skills, search, account sign-in, and
/// every account command use the same origin, so a credential stays with the
/// origin that issued it.
pub fn api_origin(value: Option<&std::ffi::OsStr>) -> Result<ServiceOrigin, RemoteError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(ServiceOrigin::production());
    };
    let invalid = || {
        RemoteError::new(
            "INVALID_ENDPOINT",
            "SKILLD_API_URL must be an HTTPS origin, or an HTTP origin on localhost or 127.0.0.1. Unset it to use https://skilld.dev.",
        )
    };
    let value = value.to_str().ok_or_else(invalid)?;
    ServiceOrigin::parse(value).map_err(|_| invalid())
}

/// The skilld token `SKILLD_TOKEN` names, if any.
///
/// A token created at skilld.dev/me/cli-tokens/new, or with `skilld tokens
/// create`, lets a script or a CI job act for an account without a browser
/// sign-in. It wins over the stored sign-in and is never refreshed or stored.
/// Unset or blank, the CLI uses the sign-in from `skilld auth login`.
pub fn token_override(value: Option<&std::ffi::OsStr>) -> Result<Option<SecretValue>, RemoteError> {
    let invalid = || {
        RemoteError::new(
            "INVALID_TOKEN",
            "SKILLD_TOKEN must be one skilld token on one line. Unset it to use the stored sign-in.",
        )
    };
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.to_str().ok_or_else(invalid)?.trim();
    if value.is_empty() {
        return Ok(None);
    }
    SecretValue::new(value).map(Some).map_err(|_| invalid())
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

impl NativeHttpAdapter {
    /// Apply the request headers and the bounded timeout to one builder.
    fn prepared<B>(
        &self,
        mut builder: ureq::RequestBuilder<B>,
        request: &HttpRequest,
        timeout: Option<Duration>,
    ) -> ureq::RequestBuilder<B> {
        for header in &request.headers {
            builder = builder.header(&header.name, header.value.expose());
        }
        if let Some(timeout) = timeout {
            builder = builder.config().timeout_global(Some(timeout)).build();
        }
        builder
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
            HttpMethod::Get => self
                .prepared(self.agent.get(&request.url), request, timeout)
                .call(),
            HttpMethod::Delete if request.body.is_empty() => self
                .prepared(self.agent.delete(&request.url), request, timeout)
                .call(),
            HttpMethod::Delete => self
                .prepared(self.agent.delete(&request.url), request, timeout)
                .force_send_body()
                .send(request.body.as_slice()),
            HttpMethod::Post => self
                .prepared(self.agent.post(&request.url), request, timeout)
                .send(request.body.as_slice()),
            HttpMethod::Put => self
                .prepared(self.agent.put(&request.url), request, timeout)
                .send(request.body.as_slice()),
            HttpMethod::Patch => self
                .prepared(self.agent.patch(&request.url), request, timeout)
                .send(request.body.as_slice()),
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
            format!("the TLS connection to {host} failed")
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
