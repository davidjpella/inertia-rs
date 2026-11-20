//! Axum integration for `inertia_rs`.
//!
//! Install [`InertiaLayer`] around your routes and extract [`InertiaRequest`]
//! in handlers:
//!
//! ```rust,no_run
//! use axum::{response::Html, routing::get, Router};
//! use inertia_rs::axum::{InertiaError, InertiaLayer, InertiaRequest};
//! use inertia_rs::{Inertia, Props};
//!
//! async fn dashboard(request: InertiaRequest) -> Result<axum::response::Response, InertiaError> {
//!     request
//!         .respond(Inertia::response(
//!             "Dashboard",
//!             Props::new()
//!                 .value("user", "Ada")
//!                 .defer_async("stats", || async { 42 }),
//!         ))
//!         .await
//! }
//!
//! let app: Router = Router::new().route("/", get(dashboard)).layer(
//!     InertiaLayer::new("asset-version-1").template(|page| {
//!         Html(format!(
//!             "<!doctype html><html><head>{}</head><body>{}</body></html>",
//!             page.head(),
//!             page.body()
//!         ))
//!     }),
//! );
//! ```

mod session;

#[cfg(feature = "tower-sessions")]
pub use session::TowerSessions;
pub use session::{RequestSession, SessionBackend};

use crate::session::{ResponseKind, SessionState, SessionWrite, ValidationErrors};
use crate::ssr::{Ssr, SsrError};
use crate::{
    is_see_other_method, Inertia, IntoPageProps, Location, Page, Prop, Redirect, RenderContext,
    RequestContext, ResolveOptions, Sleeper, PURPOSE, VARY, X_INERTIA, X_INERTIA_LOCATION,
    X_INERTIA_REDIRECT, X_INERTIA_VERSION,
};
use ::axum::body::HttpBody;
use ::axum::extract::{FromRequestParts, OriginalUri};
#[cfg(feature = "experimental-layers")]
use ::axum::http::header::HOST;
use ::axum::http::header::{InvalidHeaderValue, LOCATION, REFERER};
use ::axum::http::request::Parts;
use ::axum::http::uri::Uri;
use ::axum::http::{Extensions, HeaderMap, HeaderValue, Method, Request, StatusCode};
use ::axum::response::{IntoResponse, Response};
use ::axum::Json;
use fluent_uri::{ParseError, UriRef};
use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use std::convert::Infallible;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tower::{Layer, Service};
use tracing::{debug, error, warn};

pub use crate::HtmlResponseContext;

type VersionProvider = Arc<dyn Fn() -> String + Send + Sync>;
type Template = Arc<dyn Fn(HtmlResponseContext) -> Response + Send + Sync>;
type SharedPropProvider = Arc<dyn Fn(&InertiaRequest) -> Prop<'static> + Send + Sync>;

fn header<'headers>(headers: &'headers HeaderMap, name: &str) -> Option<&'headers str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn request_context(headers: &HeaderMap) -> RequestContext {
    RequestContext::from_header_fn(|name| header(headers, name))
}

fn add_vary_header(headers: &mut HeaderMap) {
    let already_varies = headers.get_all(VARY).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(X_INERTIA))
        })
    });

    if !already_varies {
        headers.append(VARY, HeaderValue::from_static(X_INERTIA));
    }
}

fn location_header(url: &str) -> Result<HeaderValue, InertiaError> {
    UriRef::parse(url).map_err(InertiaError::InvalidUri)?;
    HeaderValue::from_str(url).map_err(InertiaError::InvalidHeader)
}

fn local_uri(uri: &Uri) -> String {
    uri.path_and_query()
        .map(|path_and_query| path_and_query.as_str().to_owned())
        .unwrap_or_else(|| "/".to_owned())
}

fn original_uri(extensions: &Extensions, uri: &Uri) -> String {
    extensions
        .get::<OriginalUri>()
        .map(|original_uri| local_uri(&original_uri.0))
        .unwrap_or_else(|| local_uri(uri))
}

fn redirect_response(status: StatusCode, url: &str) -> Result<Response, InertiaError> {
    let mut response = status.into_response();
    response
        .headers_mut()
        .insert(LOCATION, location_header(url)?);
    add_vary_header(response.headers_mut());
    Ok(response)
}

fn conflict_response(header_name: &'static str, url: &str) -> Result<Response, InertiaError> {
    let mut response = StatusCode::CONFLICT.into_response();
    response
        .headers_mut()
        .insert(header_name, location_header(url)?);
    add_vary_header(response.headers_mut());
    Ok(response)
}

fn tokio_sleeper() -> Sleeper {
    Arc::new(|duration| Box::pin(tokio::time::sleep(duration)))
}

/// Error returned while building Axum Inertia responses.
#[derive(Debug)]
#[non_exhaustive]
pub enum InertiaError {
    /// The page object could not be serialized.
    Serialization(serde_json::Error),
    /// A response header value could not be constructed.
    InvalidHeader(InvalidHeaderValue),
    /// A redirect or location URL was not a valid URI reference.
    InvalidUri(ParseError),
    /// Props could not be resolved.
    Props(crate::Error),
    /// Server-side rendering failed and the SSR policy is
    /// [`SsrFailurePolicy::Error`](crate::ssr::SsrFailurePolicy::Error).
    Ssr(Box<SsrError>),
    /// No root template was configured for a first-page HTML response.
    MissingTemplate,
}

impl fmt::Display for InertiaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialization(error) => write!(f, "failed to serialize Inertia page: {error}"),
            Self::InvalidHeader(error) => write!(f, "invalid Inertia response header: {error}"),
            Self::InvalidUri(error) => write!(f, "invalid Inertia URI reference: {error}"),
            Self::Props(error) => write!(f, "{error}"),
            Self::Ssr(error) => write!(f, "{error}"),
            Self::MissingTemplate => write!(
                f,
                "no Inertia root template configured; use InertiaLayer::template or respond_with"
            ),
        }
    }
}

impl Error for InertiaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialization(error) => Some(error),
            Self::InvalidHeader(error) => Some(error),
            Self::InvalidUri(error) => Some(error),
            Self::Props(error) => Some(error),
            Self::Ssr(error) => Some(error.as_ref()),
            Self::MissingTemplate => None,
        }
    }
}

impl From<serde_json::Error> for InertiaError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<crate::Error> for InertiaError {
    fn from(error: crate::Error) -> Self {
        match error {
            crate::Error::Serialization(error) => Self::Serialization(error),
            other => Self::Props(other),
        }
    }
}

impl IntoResponse for InertiaError {
    fn into_response(self) -> Response {
        error!(error = %self, "failed to build Axum Inertia response");

        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to build Inertia response",
        )
            .into_response()
    }
}

/// Current asset version inserted by [`InertiaLayer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InertiaVersion(String);

impl InertiaVersion {
    /// Creates an asset version value for request extensions.
    pub fn new<V: Into<String>>(version: V) -> Self {
        Self(version.into())
    }

    /// Returns the asset version string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Shared Inertia props resolved for every page response.
///
/// Register shared props with [`InertiaLayer::shared`], or as an Axum
/// [`Extension`](::axum::Extension) for compatibility with 0.4. Shared props
/// are resolved together with route props, so partial reloads filter them the
/// same way and excluded providers never run. Route props win on key
/// collisions. Dotted keys, such as `auth.user`, build nested props.
///
/// ```rust
/// use inertia_rs::axum::SharedProps;
/// use inertia_rs::Prop;
///
/// #[derive(Clone)]
/// struct CurrentUser(String);
///
/// let shared = SharedProps::new()
///     .value("appName", "My App")
///     .prop_optional("auth.user", |request| {
///         request.extension::<CurrentUser>().map(|user| user.0.clone())
///     })
///     .prop_async("notifications", |_request| async { vec!["Welcome"] })
///     .typed("countries", |_request| Prop::lazy(|| vec!["NL", "US"]).once());
/// # let _ = shared;
/// ```
#[derive(Clone, Default)]
pub struct SharedProps {
    providers: Vec<(String, SharedPropProvider)>,
}

impl fmt::Debug for SharedProps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedProps")
            .field(
                "keys",
                &self
                    .providers
                    .iter()
                    .map(|(key, _)| key)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl SharedProps {
    /// Creates an empty shared prop registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a fixed serializable shared prop value.
    pub fn value<K, T>(self, key: K, value: T) -> Self
    where
        K: Into<String>,
        T: Clone + Send + Sync + Serialize + 'static,
    {
        self.typed(key, move |_request| Prop::value(value.clone()))
    }

    /// Registers a request-aware shared prop provider.
    ///
    /// The provider runs only when the response includes the prop.
    pub fn prop<K, F, T>(self, key: K, provider: F) -> Self
    where
        K: Into<String>,
        F: Fn(&InertiaRequest) -> T + Send + Sync + 'static,
        T: Serialize,
    {
        let provider = Arc::new(provider);
        self.typed(key, move |request| {
            let provider = provider.clone();
            let request = request.clone();
            Prop::lazy(move || provider(&request))
        })
    }

    /// Registers a request-aware shared prop provider that can skip its key.
    ///
    /// Returning `None` omits the shared prop instead of serializing `null`.
    pub fn prop_optional<K, F, T>(self, key: K, provider: F) -> Self
    where
        K: Into<String>,
        F: Fn(&InertiaRequest) -> Option<T> + Send + Sync + 'static,
        T: Serialize,
    {
        let provider = Arc::new(provider);
        self.typed(key, move |request| {
            let provider = provider.clone();
            let request = request.clone();
            Prop::lazy_optional_value(move || provider(&request))
        })
    }

    /// Registers an asynchronous request-aware shared prop provider.
    pub fn prop_async<K, F, Fut, T>(self, key: K, provider: F) -> Self
    where
        K: Into<String>,
        F: Fn(InertiaRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = T> + Send + 'static,
        T: Serialize,
    {
        let provider = Arc::new(provider);
        self.typed(key, move |request| {
            let provider = provider.clone();
            let request = request.clone();
            Prop::lazy_async(move || provider(request))
        })
    }

    /// Registers a shared prop built per request, which may carry prop
    /// behaviour such as `once`, `always`, `defer`, or `live`.
    pub fn typed<K, F>(mut self, key: K, factory: F) -> Self
    where
        K: Into<String>,
        F: Fn(&InertiaRequest) -> Prop<'static> + Send + Sync + 'static,
    {
        let key = key.into();
        self.providers.retain(|(existing, _)| existing != &key);
        self.providers.push((key, Arc::new(factory)));
        self
    }

    /// Returns `true` when no shared props have been registered.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    fn extend(&mut self, other: &SharedProps) {
        for (key, provider) in &other.providers {
            self.providers.retain(|(existing, _)| existing != key);
            self.providers.push((key.clone(), provider.clone()));
        }
    }

    fn build(&self, request: &InertiaRequest) -> Vec<(String, Prop<'static>)> {
        self.providers
            .iter()
            .map(|(key, factory)| (key.clone(), factory(request)))
            .collect()
    }
}

/// Configuration installed by [`InertiaLayer`] and read by
/// [`InertiaRequest`].
#[derive(Clone)]
struct InertiaConfig {
    version: Option<VersionProvider>,
    template: Option<Template>,
    root_id: String,
    shared: SharedProps,
    ssr: Option<Ssr>,
    options: ResolveOptions,
    session: Option<Arc<dyn SessionBackend>>,
    redirect_empty_responses: bool,
    all_error_messages: bool,
    preserve_big_integers: bool,
    expose_shared_keys: bool,
}

impl Default for InertiaConfig {
    fn default() -> Self {
        Self {
            version: None,
            template: None,
            root_id: "app".to_owned(),
            shared: SharedProps::new(),
            ssr: None,
            options: ResolveOptions::new(),
            session: None,
            redirect_empty_responses: true,
            all_error_messages: false,
            preserve_big_integers: false,
            expose_shared_keys: true,
        }
    }
}

/// Per-request session handle inserted by [`InertiaLayer`].
#[derive(Clone, Default)]
struct SessionHandle(Arc<Mutex<SessionState>>);

impl SessionHandle {
    fn with<R>(&self, f: impl FnOnce(&mut SessionState) -> R) -> R {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        f(&mut state)
    }
}

/// Axum request extractor for Inertia protocol state.
///
/// Handlers use it to render pages, redirect, flash data, and report
/// validation errors. Pair it with [`InertiaLayer`].
#[derive(Clone)]
pub struct InertiaRequest {
    context: RequestContext,
    extensions: Arc<Extensions>,
    headers: Arc<HeaderMap>,
    method: Method,
    uri: String,
    version: Option<String>,
    config: Arc<InertiaConfig>,
    session: SessionHandle,
}

impl fmt::Debug for InertiaRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InertiaRequest")
            .field("context", &self.context)
            .field("method", &self.method)
            .field("uri", &self.uri)
            .field("version", &self.version)
            .finish()
    }
}

impl InertiaRequest {
    /// Returns `true` when the request includes the `X-Inertia` header.
    pub fn is_inertia(&self) -> bool {
        self.context.is_inertia()
    }

    /// Returns the parsed request context.
    pub fn context(&self) -> &RequestContext {
        &self.context
    }

    /// Returns the request URI used as the default page-object URL.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// Returns the request method.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Returns the request headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns the current asset version installed by [`InertiaLayer`].
    pub fn asset_version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns a request extension inserted by an outer layer.
    ///
    /// Extensions are captured when the extractor runs, so values inserted
    /// by later extractors are not visible.
    pub fn extension<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.extensions.get::<T>()
    }

    /// Flashes a value to the next rendered page.
    ///
    /// Before a redirect the value is stored in the session (requires a
    /// session backend on [`InertiaLayer`]); on a rendered page it is sent in
    /// the same response.
    pub fn flash<K: Into<String>, V: Serialize>(
        &self,
        key: K,
        value: V,
    ) -> Result<(), InertiaError> {
        self.session
            .with(|state| state.flash(key, value))
            .map_err(InertiaError::Serialization)
    }

    /// Adds validation errors for the next rendered page, usually followed by
    /// [`back`](Self::back).
    ///
    /// When the request names an error bag (`X-Inertia-Error-Bag`), the
    /// errors are delivered under that bag.
    pub fn with_errors(&self, errors: ValidationErrors) {
        self.session.with(|state| state.errors(errors));
    }

    /// Sets `clearHistory` on the next rendered page.
    pub fn clear_history(&self) {
        self.session.with(SessionState::clear_history);
    }

    /// Sets `preserveFragment` on the next rendered page.
    pub fn preserve_fragment(&self) {
        self.session.with(SessionState::preserve_fragment);
    }

    /// Marks the next page as an experimental interstitial.
    #[cfg(feature = "experimental-layers")]
    pub fn interstitial(&self) {
        self.session.with(SessionState::interstitial);
    }

    fn render_context<'r, 'a>(&'r self) -> RenderContext<'r, 'a> {
        let session = self.session.with(SessionState::consume);
        let mut shared = self.config.shared.clone();

        if let Some(extension_shared) = self.extensions.get::<SharedProps>() {
            shared.extend(extension_shared);
        }

        let errors = session
            .errors
            .to_prop(self.context.error_bag(), self.config.all_error_messages);
        let mut entries = vec![("errors".to_owned(), Prop::json(errors).always())];
        entries.extend(shared.build(self));
        let entries = entries.into_iter().map(|(key, prop)| (key, prop.scoped()));

        let context = RenderContext::new(&self.context, self.uri.clone())
            .version(self.version.clone())
            .share_all(entries)
            .expose_shared_keys(self.config.expose_shared_keys)
            .flash(session.flash)
            .clear_history(session.clear_history)
            .preserve_fragment(session.preserve_fragment)
            .preserve_big_integers(self.config.preserve_big_integers)
            .options(self.config.options.clone());

        #[cfg(feature = "experimental-layers")]
        let context = context.interstitial(session.interstitial);

        context
    }

    #[cfg(feature = "experimental-layers")]
    fn relative_layer_base(&self, page: &mut Page<Value>) {
        let Some(base) = page.layer().and_then(|layer| layer.base()) else {
            return;
        };
        let Some(host) = header(&self.headers, HOST.as_str()) else {
            return;
        };

        if let Some(relative) = crate::layers::to_relative_url(base, host) {
            let mut layer = page.layer().cloned().unwrap_or_default();
            layer.set_base(relative);
            page.set_layer(Some(layer));
        }
    }

    fn json_response(&self, page: &Page<Value>) -> Response {
        let mut response = Json(page).into_response();
        response
            .headers_mut()
            .insert(X_INERTIA, HeaderValue::from_static("true"));
        add_vary_header(response.headers_mut());
        response
    }

    /// Renders a page synchronously with an explicit HTML renderer.
    ///
    /// Kept for compatibility with 0.4. If the request needs an asynchronous
    /// provider, including an async shared prop registered with
    /// [`SharedProps::prop_async`], this returns [`InertiaError::Props`] with
    /// [`Error::AsyncProp`](crate::Error::AsyncProp). SSR is not applied. Use
    /// [`respond`](Self::respond) or [`respond_with`](Self::respond_with) in new
    /// code.
    pub fn render<'a, T, F, R>(
        &self,
        inertia: Inertia<T>,
        html_response: F,
    ) -> Result<Response, InertiaError>
    where
        T: IntoPageProps<'a>,
        F: FnOnce(HtmlResponseContext) -> R,
        R: IntoResponse,
    {
        #[allow(unused_mut)]
        let mut page = inertia.resolve_sync(self.render_context())?;

        #[cfg(feature = "experimental-layers")]
        self.relative_layer_base(&mut page);

        if self.context.is_inertia() {
            return Ok(self.json_response(&page));
        }

        let context = HtmlResponseContext::from_page(&page)?.with_root_id(&self.config.root_id);
        let mut response = html_response(context).into_response();
        add_vary_header(response.headers_mut());
        Ok(response)
    }

    /// Renders a page with the root template configured on
    /// [`InertiaLayer::template`].
    ///
    /// Asynchronous providers needed by the request are polled concurrently.
    /// First-page loads are server-rendered when SSR is configured.
    pub async fn respond<'a, T>(&self, inertia: Inertia<T>) -> Result<Response, InertiaError>
    where
        T: IntoPageProps<'a>,
    {
        let template = self.config.template.clone();

        match template {
            Some(template) => self.respond_with(inertia, move |page| template(page)).await,
            None if self.context.is_inertia() => {
                self.respond_with(inertia, |_page| StatusCode::INTERNAL_SERVER_ERROR)
                    .await
            }
            None => Err(InertiaError::MissingTemplate),
        }
    }

    /// Renders a page with an explicit HTML renderer.
    pub async fn respond_with<'a, T, F, R>(
        &self,
        inertia: Inertia<T>,
        html_response: F,
    ) -> Result<Response, InertiaError>
    where
        T: IntoPageProps<'a>,
        F: FnOnce(HtmlResponseContext) -> R,
        R: IntoResponse,
    {
        #[allow(unused_mut)]
        let mut page = inertia.resolve(self.render_context()).await?;

        #[cfg(feature = "experimental-layers")]
        self.relative_layer_base(&mut page);

        if self.context.is_inertia() {
            return Ok(self.json_response(&page));
        }

        let mut context = HtmlResponseContext::from_page(&page)?.with_root_id(&self.config.root_id);

        if let Some(ssr) = &self.config.ssr {
            let path = self.uri.split('?').next().unwrap_or("/");

            if ssr.applies_to(path) {
                let page = serde_json::to_value(&page)?;
                if let Some(rendered) = ssr
                    .render(&page)
                    .await
                    .map_err(|error| InertiaError::Ssr(Box::new(error)))?
                {
                    context = context.with_ssr(rendered);
                }
            }
        }

        let mut response = html_response(context).into_response();
        add_vary_header(response.headers_mut());
        Ok(response)
    }

    /// Converts an external location visit into a response.
    ///
    /// Inertia requests receive `409 Conflict` with `X-Inertia-Location`, so
    /// the client performs a full `window.location` visit. Other requests
    /// receive a method-aware redirect.
    pub fn location(&self, location: Location) -> Result<Response, InertiaError> {
        if self.context.is_inertia() {
            conflict_response(X_INERTIA_LOCATION, location.url())
        } else {
            self.redirect_status(location.url())
        }
    }

    /// Converts a redirect into a response.
    ///
    /// Uses `303 See Other` after `POST`, `PUT`, `PATCH`, `DELETE`, and
    /// `QUERY` requests and `302 Found` otherwise. [`InertiaLayer`] turns
    /// redirects to URLs with a fragment into `X-Inertia-Redirect` responses
    /// for Inertia requests.
    pub fn redirect(&self, redirect: Redirect) -> Result<Response, InertiaError> {
        self.redirect_status(redirect.url())
    }

    fn redirect_status(&self, url: &str) -> Result<Response, InertiaError> {
        if is_see_other_method(self.method.as_str()) {
            redirect_response(StatusCode::SEE_OTHER, url)
        } else {
            redirect_response(StatusCode::FOUND, url)
        }
    }

    /// Redirects back to the `Referer`, or to `/` when there is none.
    pub fn back(&self) -> Result<Response, InertiaError> {
        self.back_or("/")
    }

    /// Redirects back to the `Referer`, or to `fallback`.
    pub fn back_or(&self, fallback: &str) -> Result<Response, InertiaError> {
        let target = header(&self.headers, REFERER.as_str())
            .filter(|referer| UriRef::parse(*referer).is_ok())
            .unwrap_or(fallback)
            .to_owned();
        self.redirect_status(&target)
    }

    /// Closes the experimental layer that made the request.
    ///
    /// Inertia requests receive the close page object
    /// (`{component: "", props: {}, url, version, close: true}`) and the
    /// client refreshes the page beneath, which consumes any flash data.
    /// Other requests are redirected back.
    #[cfg(feature = "experimental-layers")]
    pub fn close(&self) -> Result<Response, InertiaError> {
        self.session.with(|state| {
            state.clear_interstitial();
            state.carry_over();
        });

        if self.context.is_inertia() {
            let page = Page::close(self.uri.clone(), self.version.clone());
            Ok(self.json_response(&page))
        } else {
            self.back()
        }
    }
}

impl<S> FromRequestParts<S> for InertiaRequest
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let config = parts
            .extensions
            .get::<Arc<InertiaConfig>>()
            .cloned()
            .unwrap_or_default();
        let session = parts
            .extensions
            .get::<SessionHandle>()
            .cloned()
            .unwrap_or_default();
        let version = parts
            .extensions
            .get::<InertiaVersion>()
            .map(|version| version.0.clone());

        Ok(Self {
            context: request_context(&parts.headers),
            extensions: Arc::new(parts.extensions.clone()),
            headers: Arc::new(parts.headers.clone()),
            method: parts.method.clone(),
            uri: original_uri(&parts.extensions, &parts.uri),
            version,
            config,
            session,
        })
    }
}

/// Tower layer implementing the Inertia middleware for Axum.
///
/// The layer:
///
/// - inserts the asset version and configuration used by [`InertiaRequest`];
/// - answers Inertia `GET` requests with a stale or missing
///   `X-Inertia-Version` with `409 Conflict`, `X-Inertia-Location`, and the
///   server's `X-Inertia-Version`, before the handler runs (a missing header
///   matches an empty server version);
/// - adds `Vary: X-Inertia` to every response;
/// - converts `302` redirects after `PUT`, `PATCH`, `DELETE`, and `QUERY` to
///   `303`;
/// - converts redirects to URLs with a fragment into `409` responses with
///   `X-Inertia-Redirect` for Inertia requests (except prefetches);
/// - redirects back when an Inertia request receives an empty `200`;
/// - loads and stores validation errors, flash data, and history flags
///   through the configured [`SessionBackend`], keeping undelivered data
///   across redirects and version conflicts.
#[derive(Clone, Default)]
pub struct InertiaLayer {
    config: InertiaConfig,
}

/// Compatibility name for [`InertiaLayer`].
pub type VersionLayer = InertiaLayer;

/// Compatibility name for [`InertiaService`].
pub type VersionService<S> = InertiaService<S>;

impl InertiaLayer {
    /// Creates a layer with a static asset `version`.
    pub fn new<V: Into<String>>(version: V) -> Self {
        let version = version.into();
        Self::dynamic(move || version.clone())
    }

    /// Creates a layer with a dynamic asset-version provider.
    ///
    /// Keep the provider fast and non-blocking. If the version comes from a
    /// manifest file, cache it in application state and read the cached value
    /// here.
    pub fn dynamic<F, V>(version_provider: F) -> Self
    where
        F: Fn() -> V + Send + Sync + 'static,
        V: Into<String>,
    {
        let mut layer = Self::default();
        layer.config.version = Some(Arc::new(move || version_provider().into()));
        layer
    }

    /// Creates a layer without asset versioning. Page objects carry
    /// `"version": ""`.
    pub fn unversioned() -> Self {
        Self::default()
    }

    /// Sets the root template used by [`InertiaRequest::respond`].
    pub fn template<F, R>(mut self, template: F) -> Self
    where
        F: Fn(HtmlResponseContext) -> R + Send + Sync + 'static,
        R: IntoResponse,
    {
        self.config.template = Some(Arc::new(move |page| template(page).into_response()));
        self
    }

    /// Sets the root element id (default `app`).
    pub fn root_id<I: Into<String>>(mut self, root_id: I) -> Self {
        self.config.root_id = root_id.into();
        self
    }

    /// Registers shared props.
    pub fn shared(mut self, shared: SharedProps) -> Self {
        self.config.shared.extend(&shared);
        self
    }

    /// Enables server-side rendering for first-page loads.
    pub fn ssr(mut self, ssr: Ssr) -> Self {
        self.config.ssr = Some(ssr);
        self
    }

    /// Stores validation errors, flash data, and history flags in a session.
    pub fn session<B: SessionBackend + 'static>(mut self, backend: B) -> Self {
        self.config.session = Some(Arc::new(backend));
        self
    }

    /// Sets async prop resolution options.
    pub fn resolve_options(mut self, options: ResolveOptions) -> Self {
        self.config.options = options;
        self
    }

    /// Fails async providers that run longer than `timeout`, using Tokio's
    /// timer. Rescued deferred props are listed in `rescuedProps` instead.
    pub fn prop_timeout(mut self, timeout: Duration) -> Self {
        self.config.options = self.config.options.timeout(timeout, tokio_sleeper());
        self
    }

    /// Limits how many async providers one response polls at a time.
    pub fn prop_concurrency(mut self, limit: usize) -> Self {
        self.config.options = self.config.options.concurrency_limit(limit);
        self
    }

    /// Sends every validation message per field instead of the first.
    pub fn all_error_messages(mut self, all: bool) -> Self {
        self.config.all_error_messages = all;
        self
    }

    /// Enables `preserveBigIntegers` for every page.
    pub fn preserve_big_integers(mut self, preserve: bool) -> Self {
        self.config.preserve_big_integers = preserve;
        self
    }

    /// Controls whether shared prop keys are listed in `sharedProps`
    /// (default `true`).
    pub fn expose_shared_keys(mut self, expose: bool) -> Self {
        self.config.expose_shared_keys = expose;
        self
    }

    /// Controls whether an empty `200` answer to an Inertia request becomes a
    /// redirect back (default `true`).
    pub fn redirect_empty_responses(mut self, redirect: bool) -> Self {
        self.config.redirect_empty_responses = redirect;
        self
    }
}

impl<S> Layer<S> for InertiaLayer {
    type Service = InertiaService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        InertiaService {
            inner,
            config: Arc::new(self.config.clone()),
        }
    }
}

/// Service produced by [`InertiaLayer`].
#[derive(Clone)]
pub struct InertiaService<S> {
    inner: S,
    config: Arc<InertiaConfig>,
}

struct RequestFacts {
    is_inertia: bool,
    is_prefetch: bool,
    method: Method,
    referer: Option<String>,
}

fn is_redirect(status: StatusCode) -> bool {
    status.is_redirection()
}

fn finish_response(
    mut response: Response,
    facts: &RequestFacts,
    config: &InertiaConfig,
) -> Response {
    add_vary_header(response.headers_mut());

    if !facts.is_inertia {
        return response;
    }

    if config.redirect_empty_responses
        && response.status() == StatusCode::OK
        && response.body().size_hint().exact() == Some(0)
    {
        let target = facts.referer.as_deref().unwrap_or("/");
        debug!(target, "redirecting back after an empty Inertia response");
        match redirect_response(StatusCode::FOUND, target) {
            Ok(redirect) => response = redirect,
            Err(error) => warn!(%error, "invalid Referer for redirect back"),
        }
    }

    if response.status() == StatusCode::FOUND
        && matches!(facts.method.as_str(), "PUT" | "PATCH" | "DELETE" | "QUERY")
    {
        *response.status_mut() = StatusCode::SEE_OTHER;
    }

    if is_redirect(response.status()) && !facts.is_prefetch {
        let location = response
            .headers()
            .get(LOCATION)
            .filter(|location| location.as_bytes().contains(&b'#'))
            .cloned();

        if let Some(location) = location {
            let mut conflict = StatusCode::CONFLICT.into_response();
            conflict.headers_mut().insert(X_INERTIA_REDIRECT, location);
            add_vary_header(conflict.headers_mut());
            response = conflict;
        }
    }

    response
}

impl<S, B> Service<Request<B>> for InertiaService<S>
where
    S: Service<Request<B>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        let config = self.config.clone();
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let version = config
                .version
                .as_ref()
                .map(|provider| provider())
                .unwrap_or_default();
            let headers = request.headers();
            let facts = RequestFacts {
                is_inertia: header(headers, X_INERTIA).is_some(),
                is_prefetch: header(headers, PURPOSE)
                    .is_some_and(|purpose| purpose.eq_ignore_ascii_case("prefetch")),
                method: request.method().clone(),
                referer: header(headers, REFERER.as_str()).map(ToOwned::to_owned),
            };

            let request_session = config
                .session
                .as_ref()
                .and_then(|backend| backend.for_request(request.extensions()));
            let stored = match &request_session {
                Some(session) => match session.load().await {
                    Ok(stored) => stored,
                    Err(error) => {
                        error!(%error, "failed to load Inertia session data");
                        None
                    }
                },
                None => None,
            };
            let had_stored_data = stored.as_ref().is_some_and(|data| !data.is_empty());
            let handle = SessionHandle(Arc::new(Mutex::new(SessionState::new(stored))));

            let stale_version = request.method() == Method::GET
                && facts.is_inertia
                && header(request.headers(), X_INERTIA_VERSION).unwrap_or("") != version;

            let response = if stale_version {
                let url = original_uri(request.extensions(), request.uri());
                debug!(%url, "Inertia asset version mismatch");
                let mut response = conflict_response(X_INERTIA_LOCATION, &url)
                    .unwrap_or_else(IntoResponse::into_response);
                if let Ok(value) = HeaderValue::from_str(&version) {
                    response.headers_mut().insert(X_INERTIA_VERSION, value);
                }
                response
            } else {
                request.extensions_mut().insert(config.clone());
                request.extensions_mut().insert(handle.clone());
                if config.version.is_some() {
                    request
                        .extensions_mut()
                        .insert(InertiaVersion::new(version));
                }
                let response = inner.call(request).await?;
                finish_response(response, &facts, &config)
            };

            let status = response.status();
            let kind = if is_redirect(status) || status == StatusCode::CONFLICT {
                ResponseKind::Redirect
            } else {
                ResponseKind::Other
            };
            let state = handle.with(std::mem::take);
            let write = state.finish(kind, had_stored_data);

            match (request_session, write) {
                (_, SessionWrite::Unchanged) => {}
                (Some(session), write) => {
                    if let Err(error) = session.store(write).await {
                        error!(%error, "failed to store Inertia session data");
                    }
                }
                (None, SessionWrite::Store(_)) => warn!(
                    "Inertia flash data or validation errors were dropped before a redirect; \
                     configure InertiaLayer::session to carry them to the next page"
                ),
                (None, SessionWrite::Clear) => {}
            }

            Ok(response)
        })
    }
}

/// Builds an Inertia-aware error page response with shared props.
///
/// Use it from a fallback handler or error mapper so that `404` and `500`
/// pages render through Inertia with the middleware's shared data. The
/// response keeps `status`.
pub async fn error_page<'a, T>(
    request: &InertiaRequest,
    status: StatusCode,
    inertia: Inertia<T>,
) -> Result<Response, InertiaError>
where
    T: IntoPageProps<'a>,
{
    let mut response = request.respond(inertia).await?;
    *response.status_mut() = status;
    Ok(response)
}

#[cfg(test)]
mod tests;
