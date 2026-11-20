//! Server-side rendering (SSR) bridge.
//!
//! The official SSR contract: the adapter `POST`s the page object as JSON to
//! the SSR server (`/render` on the production Node server, or
//! `/__inertia_ssr` on the Vite dev server when `@inertiajs/vite` finds an SSR
//! entry). The server answers `{"head": [...], "body": "..."}`. `head` holds
//! elements for the document `<head>`; `body` replaces the page-object script
//! and root element, and its root carries `data-server-rendered="true"` so the
//! client hydrates instead of mounting.
//!
//! A failed render is not fatal: the adapter reports the failure and returns
//! the document without SSR markup, which the client renders normally.
//! Configure [`SsrFailurePolicy::Error`] to fail loudly instead, for example in
//! end-to-end tests.

// SSR failures are rare and carry diagnostic detail; their size does not
// matter on the hot path.
#![allow(clippy::result_large_err)]

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

#[cfg(feature = "ssr")]
mod http;
#[cfg(feature = "ssr")]
pub use http::{HttpSsrGateway, DEFAULT_SSR_URL};

/// A successful SSR result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SsrResponse {
    #[serde(default)]
    head: Vec<String>,
    #[serde(default)]
    body: String,
}

impl SsrResponse {
    /// Creates an SSR result.
    pub fn new<B: Into<String>>(head: Vec<String>, body: B) -> Self {
        Self {
            head,
            body: body.into(),
        }
    }

    /// Returns the head elements.
    pub fn head(&self) -> &[String] {
        &self.head
    }

    /// Returns the body markup.
    pub fn body(&self) -> &str {
        &self.body
    }
}

/// The classified cause of an SSR failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SsrErrorKind {
    /// The render used a browser-only global (`browser-api`).
    BrowserApi,
    /// The SSR server could not resolve the page component.
    ComponentResolution,
    /// A component threw during render.
    Render,
    /// The SSR server could not be reached or did not answer in time.
    Connection,
    /// The SSR server returned an unexpected response.
    InvalidResponse,
    /// Another error type reported by the SSR server.
    Other(String),
}

impl SsrErrorKind {
    fn from_type(value: Option<&str>) -> Self {
        match value {
            Some("browser-api") => Self::BrowserApi,
            Some("component-resolution") => Self::ComponentResolution,
            Some("render") => Self::Render,
            Some("connection") => Self::Connection,
            Some(other) => Self::Other(other.to_owned()),
            None => Self::Render,
        }
    }
}

/// An SSR failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SsrError {
    /// The classified cause.
    pub kind: SsrErrorKind,
    /// The error message.
    pub message: String,
    /// A hint from the SSR server, when provided.
    pub hint: Option<String>,
    /// The browser global involved, for [`SsrErrorKind::BrowserApi`].
    pub browser_api: Option<String>,
    /// The component being rendered, when known.
    pub component: Option<String>,
    /// The source location, when known.
    pub source_location: Option<String>,
}

impl SsrError {
    /// Creates an error of `kind`.
    pub fn new<M: Into<String>>(kind: SsrErrorKind, message: M) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
            browser_api: None,
            component: None,
            source_location: None,
        }
    }

    /// Parses the SSR server's JSON error body.
    pub fn from_body(body: &Value) -> Self {
        let text = |key: &str| body.get(key).and_then(Value::as_str).map(ToOwned::to_owned);

        Self {
            kind: SsrErrorKind::from_type(body.get("type").and_then(Value::as_str)),
            message: text("error").unwrap_or_else(|| "Unknown SSR error".to_owned()),
            hint: text("hint"),
            browser_api: text("browserApi"),
            component: text("component"),
            source_location: text("sourceLocation"),
        }
    }
}

impl fmt::Display for SsrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Inertia SSR failed ({:?}): {}", self.kind, self.message)?;

        if let Some(hint) = &self.hint {
            write!(f, " — {hint}")?;
        }

        Ok(())
    }
}

impl std::error::Error for SsrError {}

/// Renders page objects on a server-side rendering backend.
///
/// `HttpSsrGateway` (feature `ssr`) talks to the official Node SSR server
/// or the Vite dev server. Implement this trait to use another transport.
pub trait SsrGateway: Send + Sync {
    /// Renders `page`, the full page object.
    fn render<'a>(&'a self, page: &'a Value) -> BoxFuture<'a, Result<SsrResponse, SsrError>>;
}

/// What to do when SSR fails.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SsrFailurePolicy {
    /// Report the failure and return the client-rendered document.
    #[default]
    Fallback,
    /// Fail the response.
    Error,
}

type SsrPredicate = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type SsrReporter = Arc<dyn Fn(&SsrError, &Value) + Send + Sync>;

/// SSR configuration shared by framework integrations.
#[derive(Clone)]
pub struct Ssr {
    gateway: Arc<dyn SsrGateway>,
    policy: SsrFailurePolicy,
    enabled_for: Option<SsrPredicate>,
    reporter: Option<SsrReporter>,
}

impl fmt::Debug for Ssr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ssr")
            .field("policy", &self.policy)
            .field("path_filter", &self.enabled_for.is_some())
            .finish()
    }
}

impl Ssr {
    /// Uses `gateway` for first-page renders.
    pub fn new<G: SsrGateway + 'static>(gateway: G) -> Self {
        Self {
            gateway: Arc::new(gateway),
            policy: SsrFailurePolicy::Fallback,
            enabled_for: None,
            reporter: None,
        }
    }

    /// Sets the failure policy.
    pub fn failure_policy(mut self, policy: SsrFailurePolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Only server-renders requests whose path matches `predicate`.
    pub fn only_when<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&str) -> bool + Send + Sync + 'static,
    {
        self.enabled_for = Some(Arc::new(predicate));
        self
    }

    /// Calls `reporter` for every failure, before the policy applies.
    ///
    /// By default failures are logged with `tracing` at `error` level. The
    /// page object is passed for context; do not log its props if they may be
    /// sensitive.
    pub fn on_failure<F>(mut self, reporter: F) -> Self
    where
        F: Fn(&SsrError, &Value) + Send + Sync + 'static,
    {
        self.reporter = Some(Arc::new(reporter));
        self
    }

    /// Returns whether SSR applies to `path`.
    pub fn applies_to(&self, path: &str) -> bool {
        self.enabled_for
            .as_ref()
            .map(|predicate| predicate(path))
            .unwrap_or(true)
    }

    /// Renders `page`, applying the failure policy.
    ///
    /// Returns `Ok(None)` when the render failed and the policy falls back to
    /// client rendering.
    pub async fn render(&self, page: &Value) -> Result<Option<SsrResponse>, SsrError> {
        match self.gateway.render(page).await {
            Ok(response) => Ok(Some(response)),
            Err(error) => {
                match &self.reporter {
                    Some(reporter) => reporter(&error, page),
                    None => tracing::error!(
                        kind = ?error.kind,
                        component = ?page.get("component"),
                        error = %error.message,
                        hint = error.hint.as_deref(),
                        "Inertia SSR render failed"
                    ),
                }

                match self.policy {
                    SsrFailurePolicy::Fallback => Ok(None),
                    SsrFailurePolicy::Error => Err(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Failing;

    impl SsrGateway for Failing {
        fn render<'a>(&'a self, _page: &'a Value) -> BoxFuture<'a, Result<SsrResponse, SsrError>> {
            Box::pin(async {
                Err(SsrError::from_body(&json!({
                    "error": "window is not defined",
                    "type": "browser-api",
                    "browserApi": "window",
                    "hint": "Wrap browser code",
                })))
            })
        }
    }

    #[test]
    fn parses_classified_error_bodies() {
        let error = SsrError::from_body(&json!({
            "error": "window is not defined",
            "type": "browser-api",
            "browserApi": "window",
            "component": "Events/Show",
        }));

        assert_eq!(error.kind, SsrErrorKind::BrowserApi);
        assert_eq!(error.browser_api.as_deref(), Some("window"));
        assert_eq!(error.component.as_deref(), Some("Events/Show"));
    }

    #[test]
    fn failure_policy_controls_fallback() {
        let reported = Arc::new(AtomicUsize::new(0));
        let ssr = Ssr::new(Failing).on_failure({
            let reported = reported.clone();
            move |_, _| {
                reported.fetch_add(1, Ordering::SeqCst);
            }
        });
        let page = json!({ "component": "Home" });

        let fallback = futures_executor::block_on(ssr.render(&page));
        assert_eq!(fallback, Ok(None));

        let strict = ssr.clone().failure_policy(SsrFailurePolicy::Error);
        assert!(futures_executor::block_on(strict.render(&page)).is_err());
        assert_eq!(reported.load(Ordering::SeqCst), 2);
    }
}
