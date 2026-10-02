//! The HTTP side of `zega cloud`: one client for the Zega Cloud public
//! management API (`https://cloud.zega.dev/openapi.json`) and the error type
//! every command returns.

use serde_json::Value;
use std::time::Duration;
use url::{Host, Url};

/// Production. Staging is `https://cloud.zega.world`.
pub const DEFAULT_API: &str = "https://cloud.zega.dev";
/// Where a person makes tokens and does what a token cannot.
pub const DASHBOARD: &str = "https://dashboard.zega.dev";

/// Longest answer the client reads: a page of 200 log entries with their
/// console lines is far below this.
const MAX_ANSWER_BYTES: u64 = 32 << 20;

/// Why a command failed.
#[derive(Debug)]
pub enum CloudError {
    /// The API answered with an error status.
    Api(ApiError),
    /// No answer arrived: DNS, connect, TLS or timeout.
    Network { host: String, detail: String },
    /// Something on this machine: no stored token, a missing file, bad flags.
    Local(String),
}

/// The API's error body: `{ "ok": false, "error": "...", "code": "..." }`,
/// plus `retryAfterSeconds` and `effect` on a 429 or 503.
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retry_after: Option<u64>,
    pub effect: Option<String>,
}

impl From<String> for CloudError {
    fn from(message: String) -> Self {
        CloudError::Local(message)
    }
}

impl From<&str> for CloudError {
    fn from(message: &str) -> Self {
        CloudError::Local(message.to_string())
    }
}

/// What a request carries.
pub enum Body<'a> {
    None,
    /// `application/json`.
    Json(Value),
    /// A function's module: `application/javascript`, the bytes as they are.
    Module(&'a [u8]),
}

/// A 2xx answer: the bytes as sent (for `--json`) and their parse.
pub struct Reply {
    pub raw: Vec<u8>,
    pub json: Value,
}

pub struct Client {
    agent: ureq::Agent,
    base: Url,
    token: String,
}

/// Validate `--api` or a stored host: an http(s) origin, plain http only for a
/// local name (a token must not cross the network in the clear), no
/// credentials, query or fragment in it. Returns it without a trailing `/`.
pub fn normalize_api(raw: &str) -> Result<String, String> {
    let url =
        Url::parse(raw.trim()).map_err(|error| format!("--api {raw:?} is not a URL: {error}"))?;
    let local = match url.host() {
        Some(Host::Domain(name)) => name == "localhost" || name.ends_with(".localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => return Err(format!("--api {raw:?} has no host")),
    };
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        "http" => return Err(format!(
            "--api {raw:?}: a token is only sent over https (plain http is allowed for localhost)"
        )),
        other => {
            return Err(format!(
                "--api {raw:?}: the scheme must be https, not {other}"
            ))
        }
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(format!(
            "--api {raw:?} must be an address only: no user, password, query or fragment"
        ));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

impl Client {
    pub fn new(api: &str, token: &str) -> Result<Self, CloudError> {
        let base = Url::parse(&normalize_api(api)?).map_err(|error| error.to_string())?;
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            // An API that redirects is not this API; never follow a redirect
            // with a bearer token attached.
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            // No proxy from the environment: product behavior has no
            // environment variables (the engine's own HTTP loader does the same).
            .proxy(None)
            .build();
        Ok(Self {
            agent: ureq::Agent::new_with_config(config),
            base,
            token: token.to_string(),
        })
    }

    /// `host` or `host:port`, as a person would write it.
    pub fn host(&self) -> String {
        let host = self.base.host_str().unwrap_or("?");
        match self.base.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        }
    }

    pub fn get(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Reply, CloudError> {
        self.call("GET", segments, query, Body::None)
    }

    pub fn call(
        &self,
        method: &str,
        segments: &[&str],
        query: &[(&str, String)],
        body: Body,
    ) -> Result<Reply, CloudError> {
        let mut url = self.base.clone();
        {
            // Every segment is percent-encoded: an id typed with a `/` stays one segment.
            let mut path = url
                .path_segments_mut()
                .map_err(|()| "the API address cannot carry a path")?;
            path.pop_if_empty().extend(segments);
        }
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }
        let mut request = ureq::http::Request::builder()
            .method(method)
            .uri(url.as_str())
            .header("authorization", format!("Bearer {}", self.token))
            .header("accept", "application/json")
            .header(
                "user-agent",
                concat!("zega-cli/", env!("CARGO_PKG_VERSION")),
            );
        let payload = match body {
            Body::None => Vec::new(),
            Body::Json(value) => {
                request = request.header("content-type", "application/json");
                serde_json::to_vec(&value).map_err(|error| error.to_string())?
            }
            Body::Module(bytes) => {
                request = request.header("content-type", "application/javascript");
                bytes.to_vec()
            }
        };
        let request = request
            .body(payload)
            .map_err(|_| "the token contains characters that cannot be sent in a header")?;
        let mut response = self
            .agent
            .run(request)
            .map_err(|error| CloudError::Network {
                host: self.host(),
                detail: error.to_string(),
            })?;
        let status = response.status().as_u16();
        let retry_header = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let raw = response
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER_BYTES)
            .read_to_vec()
            .map_err(|error| CloudError::Network {
                host: self.host(),
                detail: error.to_string(),
            })?;
        let json = serde_json::from_slice::<Value>(&raw);
        if (200..300).contains(&status) {
            return match json {
                Ok(json) => Ok(Reply { raw, json }),
                Err(_) => Err(CloudError::Local(format!(
                    "{} answered HTTP {status} with something that is not JSON",
                    self.host()
                ))),
            };
        }
        Err(CloudError::Api(api_error(
            status,
            json.ok(),
            retry_header,
            &self.host(),
        )))
    }
}

fn api_error(status: u16, body: Option<Value>, retry_header: Option<u64>, host: &str) -> ApiError {
    let field = |name: &str| body.as_ref().and_then(|body| body.get(name)).cloned();
    let code = field("code").and_then(|code| code.as_str().map(str::to_string));
    let message = field("error").and_then(|message| message.as_str().map(str::to_string));
    ApiError {
        status,
        // Not the API's shape (a proxy's page, a redirect): say what we know.
        code: code.unwrap_or_else(|| format!("http_{status}")),
        message: message
            .unwrap_or_else(|| format!("{host} answered HTTP {status} without an error message")),
        retry_after: field("retryAfterSeconds")
            .and_then(|seconds| seconds.as_u64())
            .or(retry_header),
        effect: field("effect").and_then(|effect| effect.as_str().map(str::to_string)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_addresses() {
        assert_eq!(
            normalize_api("https://cloud.zega.world/").unwrap(),
            "https://cloud.zega.world"
        );
        assert_eq!(
            normalize_api("http://127.0.0.1:8787").unwrap(),
            "http://127.0.0.1:8787"
        );
        assert_eq!(
            normalize_api("http://cloud.zega.localhost:8787").unwrap(),
            "http://cloud.zega.localhost:8787"
        );
        assert!(
            normalize_api("http://cloud.zega.dev").is_err(),
            "plain http to a remote host"
        );
        assert!(normalize_api("ftp://cloud.zega.dev").is_err());
        assert!(normalize_api("https://user:pw@cloud.zega.dev").is_err());
        assert!(normalize_api("https://cloud.zega.dev/?x=1").is_err());
        assert!(normalize_api("cloud.zega.dev").is_err());
    }

    #[test]
    fn error_body_without_the_apis_shape_keeps_its_status() {
        let error = api_error(502, None, Some(9), "cloud.zega.dev");
        assert_eq!(error.code, "http_502");
        assert!(
            error.message.contains("cloud.zega.dev"),
            "{}",
            error.message
        );
        assert_eq!(error.retry_after, Some(9));
    }
}
