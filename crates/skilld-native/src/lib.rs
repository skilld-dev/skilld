use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use skilld_command::{Cancellation, HttpAdapter, HttpMethod, HttpRequest, HttpResponse};
use skilld_core::RemoteError;
use url::Url;

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
        .map_err(|_| {
            RemoteError::new(
                "HTTP_TRANSPORT",
                "the remote request could not be completed",
            )
        })?;
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
            let read = reader.read(&mut buffer).map_err(|_| {
                RemoteError::new("HTTP_TRANSPORT", "the remote response could not be read")
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
