//! HTTP client for the official Inertia SSR server.

use super::{SsrError, SsrErrorKind, SsrGateway, SsrResponse};
use bytes::Bytes;
use futures_util::future::BoxFuture;
use http_body_util::{BodyExt, Full};
use hyper::header::CONTENT_TYPE;
use hyper::{Method, Request, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde_json::Value;
use std::fmt;
use std::time::Duration;

/// Default address of the production SSR server started with
/// `node bootstrap/ssr/ssr.js` (or your build's SSR bundle).
pub const DEFAULT_SSR_URL: &str = "http://127.0.0.1:13714";

/// Renders pages through the Inertia SSR HTTP endpoint.
///
/// - Production: [`HttpSsrGateway::new`] posts to `<url>/render` on the Node
///   SSR server built from your SSR entry (`vite build --ssr`).
/// - Development: [`HttpSsrGateway::vite_dev`] posts to `/__inertia_ssr` on
///   the Vite dev server, which `@inertiajs/vite` exposes when it finds an SSR
///   entry, so no separate Node process is needed.
///
/// Only plain `http://` URLs are supported; the SSR server is expected to
/// run next to the application. Implement [`SsrGateway`] for other
/// transports.
#[derive(Clone)]
pub struct HttpSsrGateway {
    render_uri: Uri,
    health_uri: Uri,
    timeout: Duration,
    client: Client<HttpConnector, Full<Bytes>>,
}

impl fmt::Debug for HttpSsrGateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpSsrGateway")
            .field("render_uri", &self.render_uri)
            .field("timeout", &self.timeout)
            .finish()
    }
}

fn join(base: &str, path: &str) -> Result<Uri, SsrError> {
    let base = base.trim_end_matches('/');

    if !base.starts_with("http://") {
        return Err(SsrError::new(
            SsrErrorKind::Connection,
            format!("unsupported SSR URL `{base}`: only http:// URLs are supported"),
        ));
    }

    format!("{base}{path}").parse().map_err(|error| {
        SsrError::new(
            SsrErrorKind::Connection,
            format!("invalid SSR URL `{base}{path}`: {error}"),
        )
    })
}

impl HttpSsrGateway {
    /// Targets the production SSR server at `url` (for example
    /// [`DEFAULT_SSR_URL`]).
    pub fn new(url: &str) -> Result<Self, SsrError> {
        Self::with_render_path(url, "/render")
    }

    /// Targets the Vite dev server at `url` (for example
    /// `http://localhost:5173`).
    pub fn vite_dev(url: &str) -> Result<Self, SsrError> {
        Self::with_render_path(url, "/__inertia_ssr")
    }

    /// Targets `url` with a custom render path.
    pub fn with_render_path(url: &str, render_path: &str) -> Result<Self, SsrError> {
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);

        Ok(Self {
            render_uri: join(url, render_path)?,
            health_uri: join(url, "/health")?,
            timeout: Duration::from_secs(5),
            client: Client::builder(TokioExecutor::new()).build(connector),
        })
    }

    /// Sets the request timeout (default 5 seconds). A timed-out render is a
    /// [`SsrErrorKind::Connection`] failure.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Returns the render endpoint.
    pub fn render_uri(&self) -> &Uri {
        &self.render_uri
    }

    /// Checks `GET /health` on the production SSR server.
    pub async fn is_healthy(&self) -> bool {
        let request = Request::get(self.health_uri.clone()).body(Full::new(Bytes::new()));
        let Ok(request) = request else {
            return false;
        };

        matches!(
            tokio::time::timeout(self.timeout, self.client.request(request)).await,
            Ok(Ok(response)) if response.status().is_success()
        )
    }

    async fn post(&self, page: &Value) -> Result<SsrResponse, SsrError> {
        let body = serde_json::to_vec(page).map_err(|error| {
            SsrError::new(
                SsrErrorKind::InvalidResponse,
                format!("invalid page object: {error}"),
            )
        })?;
        let request = Request::builder()
            .method(Method::POST)
            .uri(self.render_uri.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(|error| SsrError::new(SsrErrorKind::Connection, error.to_string()))?;

        let response = tokio::time::timeout(self.timeout, self.client.request(request))
            .await
            .map_err(|_| {
                SsrError::new(
                    SsrErrorKind::Connection,
                    format!("SSR request timed out after {:?}", self.timeout),
                )
            })?
            .map_err(|error| SsrError::new(SsrErrorKind::Connection, error.to_string()))?;
        let status = response.status();
        let bytes = tokio::time::timeout(self.timeout, response.into_body().collect())
            .await
            .map_err(|_| SsrError::new(SsrErrorKind::Connection, "SSR response timed out"))?
            .map_err(|error| SsrError::new(SsrErrorKind::Connection, error.to_string()))?
            .to_bytes();

        if status != StatusCode::OK {
            return Err(match serde_json::from_slice::<Value>(&bytes) {
                Ok(body) => SsrError::from_body(&body),
                Err(_) => SsrError::new(
                    SsrErrorKind::InvalidResponse,
                    format!("SSR server answered {status}"),
                ),
            });
        }

        serde_json::from_slice(&bytes).map_err(|error| {
            SsrError::new(
                SsrErrorKind::InvalidResponse,
                format!("invalid SSR response body: {error}"),
            )
        })
    }
}

impl SsrGateway for HttpSsrGateway {
    fn render<'a>(&'a self, page: &'a Value) -> BoxFuture<'a, Result<SsrResponse, SsrError>> {
        Box::pin(self.post(page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serves one canned HTTP response and returns the request it received.
    async fn serve_once(
        status: &'static str,
        body: &'static str,
        delay: Duration,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                received.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&received);
                if let Some(index) = text.find("\r\n\r\n") {
                    let length = text[..index]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if received.len() >= index + 4 + length {
                        break;
                    }
                }
            }
            tokio::time::sleep(delay).await;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            String::from_utf8(received).unwrap()
        });
        (url, handle)
    }

    #[tokio::test]
    async fn posts_the_page_object_and_parses_head_and_body() {
        let (url, server) = serve_once(
            "200 OK",
            r#"{"head":["<title>Event</title>"],"body":"<div data-server-rendered=\"true\" id=\"app\">x</div>"}"#,
            Duration::ZERO,
        )
        .await;
        let gateway = HttpSsrGateway::new(&url).unwrap();
        let response = gateway
            .render(&json!({ "component": "Event", "props": {}, "url": "/", "version": "" }))
            .await
            .unwrap();

        assert_eq!(response.head(), ["<title>Event</title>"]);
        assert!(response.body().contains("data-server-rendered"));
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /render HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("content-type: application/json"));
        assert!(request.ends_with(r#"{"component":"Event","props":{},"url":"/","version":""}"#));
    }

    #[tokio::test]
    async fn vite_dev_uses_the_dev_endpoint() {
        let (url, server) = serve_once("200 OK", r#"{"head":[],"body":""}"#, Duration::ZERO).await;
        HttpSsrGateway::vite_dev(&url)
            .unwrap()
            .render(&json!({}))
            .await
            .unwrap();

        assert!(server
            .await
            .unwrap()
            .starts_with("POST /__inertia_ssr HTTP/1.1"));
    }

    #[tokio::test]
    async fn classified_errors_and_timeouts() {
        let (url, _server) = serve_once(
            "500 Internal Server Error",
            r#"{"error":"window is not defined","type":"browser-api","browserApi":"window","hint":"h","timestamp":"t"}"#,
            Duration::ZERO,
        )
        .await;
        let error = HttpSsrGateway::new(&url)
            .unwrap()
            .render(&json!({}))
            .await
            .unwrap_err();
        assert_eq!(error.kind, SsrErrorKind::BrowserApi);
        assert_eq!(error.browser_api.as_deref(), Some("window"));

        let (url, _server) = serve_once("200 OK", "{}", Duration::from_secs(2)).await;
        let error = HttpSsrGateway::new(&url)
            .unwrap()
            .timeout(Duration::from_millis(50))
            .render(&json!({}))
            .await
            .unwrap_err();
        assert_eq!(error.kind, SsrErrorKind::Connection);
    }

    #[tokio::test]
    async fn unreachable_servers_are_connection_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);

        let gateway = HttpSsrGateway::new(&url).unwrap();
        assert_eq!(
            gateway.render(&json!({})).await.unwrap_err().kind,
            SsrErrorKind::Connection
        );
        assert!(!gateway.is_healthy().await);
        assert!(HttpSsrGateway::new("https://example.com").is_err());
    }
}
