//! Authenticated HTTP calls to the T3 server.

use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use crate::error::{err, err_exit, exit};
use crate::protocol::{PROTOCOL_HEADER, PROTOCOL_VERSION};

/// The client of every HTTP request t3term sends a T3 server. It follows no redirect, so a
/// request reaches the origin t3term chose or fails there, and it uses no proxy, so a request
/// for a server on this machine stays on this machine. The WebSocket works the same way:
/// tokio-tungstenite opens it straight to the origin and fails on any answer but 101.
pub(crate) fn direct_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        // `reqwest::Client::new` panics on the same failure: a TLS backend or the system's
        // resolver configuration that can't load.
        .expect("an HTTP client")
}

#[derive(Clone)]
pub struct Api {
    pub origin: String,
    token: String,
    http: reqwest::Client,
}

impl Api {
    pub fn new(origin: &str, token: &str) -> Self {
        Self {
            origin: origin.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: direct_client(),
        }
    }

    /// An `Api` with no token for tests that talk to a fake T3 on 127.0.0.1. Like every `Api`,
    /// it ignores proxy settings, so a proxy in the test's environment can't take a request.
    #[cfg(test)]
    pub(crate) fn without_proxy(origin: &str) -> Self {
        Self {
            origin: origin.to_string(),
            token: String::new(),
            http: direct_client(),
        }
    }

    pub async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let mut request = self
            .http
            .request(method.clone(), format!("{}{}", self.origin, path))
            .bearer_auth(&self.token)
            // Orchestration routes reject requests that do not name the protocol; others ignore it.
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION.to_string())
            .timeout(Duration::from_secs(30));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|e| {
            err_exit(
                "T3_REQUEST_FAILED",
                exit::UNAVAILABLE,
                format!("{method} {path} failed: {e}"),
            )
        })?;
        let status = response.status();
        if status.is_redirection() {
            // No route t3term calls redirects. At T3 f570bd21 only the web app's pages and the
            // MCP sign-in page do. Following one would send this request, and its body, to a
            // server t3term didn't choose.
            return Err(err_exit(
                "T3_REDIRECTED",
                exit::REJECTED,
                format!(
                    "T3 at {} answered {method} {path} with a redirect. t3term sends a request only to the server it chose, so it stopped.",
                    self.origin
                ),
            ));
        }
        let text = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(err_exit(
                "T3_NOT_FOUND",
                exit::NOT_FOUND,
                format!("T3 has nothing at {path}."),
            ));
        }
        if !status.is_success() {
            let detail = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .or_else(|| v.get("error"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| text.chars().take(200).collect());
            return Err(err(
                "T3_API_ERROR",
                format!("T3 returned HTTP {status} for {method} {path}: {detail}"),
            ));
        }
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|_| {
            err(
                "T3_API_ERROR",
                format!("T3 sent unreadable JSON for {path}."),
            )
        })
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.request(reqwest::Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: Option<&Value>) -> Result<Value> {
        self.request(reqwest::Method::POST, path, body).await
    }

    /// Exchanges a one-time pairing credential for a bearer token, as T3's clients do at
    /// `/oauth/token`.
    pub async fn pair(&self, credential: &str, label: &str) -> Result<String> {
        let response = self
            .http
            .post(format!("{}/oauth/token", self.origin))
            .form(&[
                (
                    "grant_type",
                    "urn:ietf:params:oauth:grant-type:token-exchange",
                ),
                ("subject_token", credential),
                (
                    "subject_token_type",
                    "urn:t3:params:oauth:token-type:environment-bootstrap",
                ),
                (
                    "requested_token_type",
                    "urn:ietf:params:oauth:token-type:access_token",
                ),
                ("client_label", label),
                ("client_device_type", "desktop"),
            ])
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| {
                err_exit(
                    "T3_REQUEST_FAILED",
                    exit::UNAVAILABLE,
                    format!("POST /oauth/token failed: {e}"),
                )
            })?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(err(
                "T3_API_ERROR",
                format!(
                    "T3 refused the pairing code (HTTP {status}): {}",
                    text.chars().take(200).collect::<String>()
                ),
            ));
        }
        serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("access_token")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .ok_or_else(|| {
                err(
                    "T3_API_ERROR",
                    "T3 answered the pairing without a token.".to_string(),
                )
            })
    }

    /// Posts raw bytes, as an attachment upload sends a file to the URL T3 issued for it.
    pub async fn post_bytes(&self, path: &str, content_type: &str, bytes: Vec<u8>) -> Result<()> {
        let response = self
            .http
            .post(format!("{}{}", self.origin, path))
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| {
                err_exit(
                    "T3_REQUEST_FAILED",
                    exit::UNAVAILABLE,
                    format!("POST {path} failed: {e}"),
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(err(
                "T3_API_ERROR",
                format!(
                    "T3 returned HTTP {status} for the upload: {}",
                    text.chars().take(200).collect::<String>()
                ),
            ));
        }
        Ok(())
    }

    /// A single-use ticket that authenticates one WebSocket connection.
    pub async fn websocket_ticket(&self) -> Result<String> {
        let issued = self.post("/api/auth/websocket-ticket", None).await?;
        issued
            .get("ticket")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .ok_or_else(|| err("T3_AUTH_FAILED", "T3 returned an invalid WebSocket ticket."))
    }
}

/// A fake T3 for unit tests, on 127.0.0.1. It answers each request with the raw HTTP response
/// `respond` gives for the request's path, and records the paths and bodies. An empty response
/// closes the connection without an answer.
#[cfg(test)]
pub(crate) mod fake {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    pub(crate) struct Server {
        pub(crate) origin: String,
        requests: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl Server {
        /// Starts a server on a free port. Its thread answers until the test process exits.
        pub(crate) fn start(respond: impl Fn(&str) -> String + Send + 'static) -> Server {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = requests.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let Ok(reading) = stream.try_clone() else {
                        continue;
                    };
                    let mut reader = BufReader::new(reading);
                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).is_err() {
                        continue;
                    }
                    // Read up to the blank line that ends the headers, then the body its length
                    // gives.
                    let mut header = String::new();
                    let mut length = 0;
                    while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
                        let (name, value) = header.split_once(':').unwrap_or_default();
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                        header.clear();
                    }
                    let mut body = vec![0; length];
                    if reader.read_exact(&mut body).is_err() {
                        continue;
                    }
                    let path = request_line.split_whitespace().nth(1).unwrap_or_default();
                    let reply = respond(path);
                    let body = String::from_utf8_lossy(&body).into_owned();
                    seen.lock().unwrap().push((path.to_string(), body));
                    let _ = stream.write_all(reply.as_bytes());
                }
            });
            Server { origin, requests }
        }

        /// The path of each request so far, in order.
        pub(crate) fn paths(&self) -> Vec<String> {
            let requests = self.requests.lock().unwrap();
            requests.iter().map(|(path, _)| path.clone()).collect()
        }

        /// The body of each request so far, in order. A request without one has "".
        pub(crate) fn bodies(&self) -> Vec<String> {
            let requests = self.requests.lock().unwrap();
            requests.iter().map(|(_, body)| body.clone()).collect()
        }
    }

    /// An HTTP/1.1 response with `status`, such as `200 OK`, the `headers` given and `body`.
    /// It closes the connection.
    pub(crate) fn response(status: &str, headers: &[&str], body: &str) -> String {
        let mut text = format!(
            "HTTP/1.1 {status}\r\nconnection: close\r\ncontent-length: {}\r\n",
            body.len()
        );
        for header in headers {
            text.push_str(header);
            text.push_str("\r\n");
        }
        text.push_str("\r\n");
        text.push_str(body);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Server, response};
    use super::*;

    #[tokio::test]
    async fn websocket_ticket_reads_the_ticket() {
        let server = Server::start(|path| match path {
            "/api/auth/websocket-ticket" => response(
                "200 OK",
                &["content-type: application/json"],
                r#"{"ticket":"t-1","expiresAt":"2026-11-12T00:00:00.000Z"}"#,
            ),
            _ => response("404 Not Found", &[], ""),
        });
        let api = Api::without_proxy(&server.origin);
        assert_eq!(api.websocket_ticket().await.unwrap(), "t-1");
        assert_eq!(server.paths(), vec!["/api/auth/websocket-ticket"]);
        assert_eq!(server.bodies(), vec![String::new()]);
    }

    #[tokio::test]
    async fn an_auth_error_carries_the_server_message() {
        let server = Server::start(|_| {
            response(
                "401 Unauthorized",
                &["content-type: application/json"],
                r#"{"_tag":"EnvironmentAuthInvalidError","message":"invalid credential"}"#,
            )
        });
        let api = Api::without_proxy(&server.origin);
        let error = api.get("/api/auth/session").await.unwrap_err().to_string();
        assert!(error.contains("401"), "{error}");
        assert!(error.contains("invalid credential"), "{error}");
    }

    #[tokio::test]
    async fn an_empty_ticket_is_an_error() {
        let server = Server::start(|_| response("200 OK", &[], r#"{"ticket":""}"#));
        let api = Api::without_proxy(&server.origin);
        assert!(api.websocket_ticket().await.is_err());
    }
}
