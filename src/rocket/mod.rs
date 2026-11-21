#![allow(clippy::type_complexity)]

//! Rocket integration for `inertia_rs`.
//!
//! Attach [`VersionFairing`] (the Inertia middleware) and return
//! [`Inertia`] values from routes. Responses with only synchronous props can
//! be returned directly; use the [`InertiaRequest`] guard and
//! [`InertiaRequest::render`] for asynchronous providers and SSR.
//!
//! Rocket 0.5 does not route the `QUERY` HTTP method, and this integration
//! does not include a session backend; see `docs/protocol-support.md`.

use crate::ssr::{Ssr, SsrError};
use crate::{
    is_see_other_method, HtmlResponseContext, Inertia, IntoPageProps, Location, Page, Prop,
    Redirect, RenderContext, RequestContext, ResolveOptions, PURPOSE, VARY, X_INERTIA,
    X_INERTIA_LOCATION, X_INERTIA_REDIRECT, X_INERTIA_VERSION,
};
use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::{self, uri::Reference, Method};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::{self, Redirect as RocketRedirect, Responder, Response};
use rocket::serde::json::Json;
use rocket::{error, get, routes, uri, Data};
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::io::Cursor;
use std::sync::Arc;
use tracing::trace;

const BASE_ROUTE: &str = "/inertia-rs";

type SharedPropFactory = Arc<dyn Fn(&Request<'_>) -> Prop<'static> + Send + Sync>;
type VersionProvider = Arc<dyn Fn() -> String + Send + Sync>;
type HtmlResponder<'resp> =
    Arc<dyn Fn(&Request<'_>, &HtmlResponseContext) -> response::Result<'resp> + Send + Sync>;

fn request_context(request: &Request<'_>) -> RequestContext {
    RequestContext::from_header_fn(|name| request.headers().get_one(name))
}

/// Parsed Inertia headers for use as a Rocket request guard.
///
/// This guard always succeeds.
pub struct InertiaHeaders {
    context: RequestContext,
}

impl InertiaHeaders {
    /// Returns `true` when the request includes the `X-Inertia` header.
    pub fn is_inertia(&self) -> bool {
        self.context.is_inertia()
    }

    /// Returns the request's `X-Inertia-Version` header value.
    pub fn version(&self) -> Option<&str> {
        self.context.version()
    }

    /// Returns the parsed request context.
    pub fn context(&self) -> &RequestContext {
        &self.context
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for InertiaHeaders {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(Self {
            context: request_context(request),
        })
    }
}

/// Shared Inertia props resolved for every Rocket page response.
///
/// Register this as managed state with [`rocket::Rocket::manage`]. Shared
/// props are resolved together with route props, so partial reloads filter
/// them; route props win on key collisions and dotted keys build nested
/// props.
///
/// Request-aware providers registered with [`prop`](Self::prop) run when the
/// page is built, because they borrow the request. Use [`typed`](Self::typed)
/// to return a lazy [`Prop`] that captures owned request data.
///
/// ```rust
/// use inertia_rs::rocket::SharedProps;
/// use inertia_rs::Prop;
///
/// let shared_props = SharedProps::new()
///     .value("appName", "My App")
///     .prop("auth.csrfToken", |request| {
///         request.headers().get_one("X-CSRF").map(ToOwned::to_owned)
///     })
///     .typed("countries", |_request| Prop::lazy(|| ["NL", "US"]).once());
/// ```
#[derive(Clone, Default)]
pub struct SharedProps {
    providers: Vec<(String, SharedPropFactory)>,
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
    pub fn prop<K, F, T>(self, key: K, provider: F) -> Self
    where
        K: Into<String>,
        F: for<'request, 'rocket> Fn(&'request Request<'rocket>) -> T + Send + Sync + 'static,
        T: Serialize,
    {
        self.typed(key, move |request| {
            Prop::computed(
                serde_json::to_value(provider(request))
                    .map(Some)
                    .map_err(Into::into),
            )
        })
    }

    /// Registers a request-aware provider that can skip its key by returning
    /// `None`.
    pub fn prop_optional<K, F, T>(self, key: K, provider: F) -> Self
    where
        K: Into<String>,
        F: for<'request, 'rocket> Fn(&'request Request<'rocket>) -> Option<T>
            + Send
            + Sync
            + 'static,
        T: Serialize,
    {
        self.typed(key, move |request| {
            Prop::computed(
                provider(request)
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(Into::into),
            )
        })
    }

    /// Registers a shared prop built per request, which may carry prop
    /// behaviour such as `once`, `always`, `defer`, or `live`.
    pub fn typed<K, F>(mut self, key: K, factory: F) -> Self
    where
        K: Into<String>,
        F: Fn(&Request<'_>) -> Prop<'static> + Send + Sync + 'static,
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

    fn build(&self, request: &Request<'_>) -> Vec<(String, Prop<'static>)> {
        self.providers
            .iter()
            .map(|(key, factory)| (key.clone(), factory(request)))
            .collect()
    }
}

#[derive(Clone)]
struct InertiaVersion(String);

struct VersionProviderFn(VersionProvider);

struct ResponderFn<'resp>(HtmlResponder<'resp>);

fn cached_version(request: &Request<'_>) -> Option<String> {
    let provider = request
        .rocket()
        .state::<VersionProviderFn>()
        .map(|provider| provider.0.clone());

    request
        .local_cache(|| provider.map(|provider| InertiaVersion(provider())))
        .clone()
        .map(|version| version.0)
}

fn shared_entries(request: &Request<'_>) -> Vec<(String, Prop<'static>)> {
    let mut entries = vec![(
        "errors".to_owned(),
        Prop::json(Value::Object(Default::default())).always(),
    )];

    if let Some(shared) = request.rocket().state::<SharedProps>() {
        entries.extend(shared.build(request));
    }

    entries
}

fn resolve_options(request: &Request<'_>) -> ResolveOptions {
    request
        .rocket()
        .state::<ResolveOptions>()
        .cloned()
        .unwrap_or_default()
}

fn render_context<'r, 'a>(
    request: &Request<'_>,
    context: &'r RequestContext,
) -> RenderContext<'r, 'a> {
    RenderContext::new(context, request.uri().to_string())
        .version(cached_version(request))
        .share_all(
            shared_entries(request)
                .into_iter()
                .map(|(key, prop)| (key, prop.scoped())),
        )
        .options(resolve_options(request))
}

fn json_response<'r, 'o: 'r>(request: &'r Request<'_>, page: &Page<Value>) -> response::Result<'o> {
    Response::build()
        .merge(Json(page).respond_to(request)?)
        .raw_header(X_INERTIA, "true")
        .raw_header_adjoin(VARY, X_INERTIA)
        .ok()
}

fn html_response<'r, 'o: 'r>(
    request: &'r Request<'_>,
    context: HtmlResponseContext,
) -> response::Result<'o> {
    match request.rocket().state::<ResponderFn>() {
        Some(responder) => responder.0(request, &context),
        None => {
            error!("Inertia HTML responder not found; attach VersionFairing");
            Err(http::Status::InternalServerError)
        }
    }
}

impl<'r, 'o: 'r, R: IntoPageProps<'o>> Responder<'r, 'o> for Inertia<R> {
    fn respond_to(self, request: &'r Request<'_>) -> response::Result<'o> {
        let context = request_context(request);
        let page = self
            .resolve_sync(render_context(request, &context))
            .map_err(|error| {
                error!("failed to build Inertia page: {}", error);
                http::Status::InternalServerError
            })?;

        if context.is_inertia() {
            json_response(request, &page)
        } else {
            let html = HtmlResponseContext::from_page(&page).map_err(|error| {
                error!("failed to serialize Inertia page: {}", error);
                http::Status::InternalServerError
            })?;
            html_response(request, html)
        }
    }
}

/// Error returned by [`InertiaRequest::render`].
#[derive(Debug)]
#[non_exhaustive]
pub enum InertiaError {
    /// Props could not be resolved or serialized.
    Props(crate::Error),
    /// SSR failed and the SSR policy is
    /// [`SsrFailurePolicy::Error`](crate::ssr::SsrFailurePolicy::Error).
    Ssr(Box<SsrError>),
}

impl fmt::Display for InertiaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Props(error) => write!(f, "{error}"),
            Self::Ssr(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for InertiaError {}

impl<'r, 'o: 'r> Responder<'r, 'o> for InertiaError {
    fn respond_to(self, _request: &'r Request<'_>) -> response::Result<'o> {
        error!("failed to build Inertia response: {}", self);
        Err(http::Status::InternalServerError)
    }
}

/// Request guard for asynchronous rendering.
///
/// ```rust,no_run
/// use inertia_rs::rocket::{InertiaError, InertiaPage, InertiaRequest};
/// use inertia_rs::{Inertia, Props};
///
/// #[rocket::get("/dashboard")]
/// async fn dashboard(inertia: InertiaRequest) -> Result<InertiaPage, InertiaError> {
///     inertia
///         .render(Inertia::response(
///             "Dashboard",
///             Props::new().defer_async("stats", || async { 42 }),
///         ))
///         .await
/// }
/// ```
///
/// The guard captures what rendering needs from the request, including
/// request-aware shared props, when it runs.
pub struct InertiaRequest {
    context: RequestContext,
    url: String,
    path: String,
    version: Option<String>,
    shared: Vec<(String, Prop<'static>)>,
    options: ResolveOptions,
    ssr: Option<Ssr>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for InertiaRequest {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(Self {
            context: request_context(request),
            url: request.uri().to_string(),
            path: request.uri().path().as_str().to_owned(),
            version: cached_version(request),
            shared: shared_entries(request),
            options: resolve_options(request),
            ssr: request.rocket().state::<Ssr>().cloned(),
        })
    }
}

impl InertiaRequest {
    /// Returns the parsed request context.
    pub fn context(&self) -> &RequestContext {
        &self.context
    }

    /// Resolves props (polling async providers concurrently) and, for
    /// first-page loads, applies SSR when an [`Ssr`] is managed.
    pub async fn render<'a, T>(self, inertia: Inertia<T>) -> Result<InertiaPage, InertiaError>
    where
        T: IntoPageProps<'a>,
    {
        let Self {
            context,
            url,
            path,
            version,
            shared,
            options,
            ssr,
        } = self;
        let render_context = RenderContext::new(&context, url)
            .version(version)
            .share_all(shared.into_iter().map(|(key, prop)| (key, prop.scoped())))
            .options(options);
        let page = inertia
            .resolve(render_context)
            .await
            .map_err(InertiaError::Props)?;
        let mut rendered = None;

        if let Some(gateway) = ssr.filter(|ssr| !context.is_inertia() && ssr.applies_to(&path)) {
            let value =
                serde_json::to_value(&page).map_err(|error| InertiaError::Props(error.into()))?;
            rendered = gateway
                .render(&value)
                .await
                .map_err(|error| InertiaError::Ssr(Box::new(error)))?;
        }

        Ok(InertiaPage {
            page,
            ssr: rendered,
        })
    }
}

/// A resolved page returned by [`InertiaRequest::render`].
pub struct InertiaPage {
    page: Page<Value>,
    ssr: Option<crate::ssr::SsrResponse>,
}

impl InertiaPage {
    /// Returns the page object.
    pub fn page(&self) -> &Page<Value> {
        &self.page
    }
}

impl<'r, 'o: 'r> Responder<'r, 'o> for InertiaPage {
    fn respond_to(self, request: &'r Request<'_>) -> response::Result<'o> {
        if request_context(request).is_inertia() {
            return json_response(request, &self.page);
        }

        let mut html = HtmlResponseContext::from_page(&self.page).map_err(|error| {
            error!("failed to serialize Inertia page: {}", error);
            http::Status::InternalServerError
        })?;

        if let Some(ssr) = self.ssr {
            html = html.with_ssr(ssr);
        }

        html_response(request, html)
    }
}

fn validated_uri_reference(url: String) -> Result<String, http::Status> {
    let uri: Reference<'static> = url.try_into().map_err(|_e| {
        error!("Invalid URI used for redirect.");
        http::Status::InternalServerError
    })?;

    Ok(uri.to_string())
}

impl<'r, 'o: 'r> Responder<'r, 'o> for Location {
    fn respond_to(self, request: &'r Request<'_>) -> response::Result<'o> {
        let url = validated_uri_reference(self.url().to_owned())?;

        if request_context(request).is_inertia() {
            Response::build()
                .status(http::Status::Conflict)
                .raw_header(X_INERTIA_LOCATION, url)
                .raw_header_adjoin(VARY, X_INERTIA)
                .ok()
        } else if is_see_other_method(request.method().as_str()) {
            RocketRedirect::to(url).respond_to(request)
        } else {
            RocketRedirect::found(url).respond_to(request)
        }
    }
}

impl<'r, 'o: 'r> Responder<'r, 'o> for Redirect {
    fn respond_to(self, request: &'r Request<'_>) -> response::Result<'o> {
        let url = validated_uri_reference(self.url().to_owned())?;

        if is_see_other_method(request.method().as_str()) {
            RocketRedirect::to(url).respond_to(request)
        } else {
            RocketRedirect::found(url).respond_to(request)
        }
    }
}

/// Rocket fairing implementing the Inertia middleware.
///
/// - Stores the asset version for page responses and answers Inertia `GET`
///   requests with a stale or missing `X-Inertia-Version` with
///   `409 Conflict`, `X-Inertia-Location`, and `X-Inertia-Version`.
/// - Registers the HTML responder used for first-page loads.
/// - Adds `Vary: X-Inertia` to every response.
/// - For Inertia requests, converts `302` after `PUT`, `PATCH`, and `DELETE`
///   to `303`, turns redirects to URLs with a fragment into
///   `X-Inertia-Redirect` responses (except prefetches), and redirects back
///   on an empty `200`.
pub struct VersionFairing<'resp> {
    version_provider: VersionProvider,
    html_response: HtmlResponder<'resp>,
}

impl<'resp> VersionFairing<'resp> {
    /// Creates a fairing with a static asset `version` and HTML renderer.
    pub fn new<F, V: Into<String>>(version: V, html_response: F) -> Self
    where
        F: Fn(&Request<'_>, &HtmlResponseContext) -> response::Result<'resp>
            + Send
            + Sync
            + 'static,
    {
        let version = version.into();

        Self::dynamic(move || version.clone(), html_response)
    }

    /// Creates a fairing with a dynamic asset-version provider and HTML
    /// renderer.
    ///
    /// Keep the provider fast and non-blocking; cache versions read from
    /// disk.
    pub fn dynamic<F, H, V>(version_provider: F, html_response: H) -> Self
    where
        F: Fn() -> V + Send + Sync + 'static,
        H: Fn(&Request<'_>, &HtmlResponseContext) -> response::Result<'resp>
            + Send
            + Sync
            + 'static,
        V: Into<String>,
    {
        Self {
            version_provider: Arc::new(move || version_provider().into()),
            html_response: Arc::new(html_response),
        }
    }
}

struct VersionConflictResponse {
    location: String,
    version: String,
}

impl<'r, 'o: 'r> Responder<'r, 'o> for VersionConflictResponse {
    fn respond_to(self, _request: &'r Request<'_>) -> response::Result<'o> {
        Response::build()
            .status(http::Status::Conflict)
            .raw_header(X_INERTIA_LOCATION, self.location)
            .raw_header(X_INERTIA_VERSION, self.version)
            .raw_header_adjoin(VARY, X_INERTIA)
            .ok()
    }
}

fn is_local_location(location: &str) -> bool {
    location.starts_with('/')
        && !location.starts_with("//")
        && !location.starts_with("/\\")
        && !location.chars().any(|c| c.is_ascii_control())
}

#[get("/version-conflict?<location>&<version>")]
fn version_conflict(
    location: String,
    version: Option<String>,
) -> Result<VersionConflictResponse, http::Status> {
    if is_local_location(&location) {
        Ok(VersionConflictResponse {
            location,
            version: version.unwrap_or_default(),
        })
    } else {
        Err(http::Status::BadRequest)
    }
}

fn is_inertia_request(request: &Request<'_>) -> bool {
    request.headers().get_one(X_INERTIA).is_some()
}

#[rocket::async_trait]
impl Fairing for VersionFairing<'static> {
    fn info(&self) -> Info {
        Info {
            name: "Inertia",
            kind: Kind::Ignite | Kind::Request | Kind::Response,
        }
    }

    async fn on_ignite(&self, rocket: rocket::Rocket<rocket::Build>) -> rocket::fairing::Result {
        Ok(rocket
            .mount(BASE_ROUTE, routes![version_conflict])
            .manage(VersionProviderFn(self.version_provider.clone()))
            .manage(ResponderFn(self.html_response.clone())))
    }

    async fn on_request(&self, request: &mut Request<'_>, _: &mut Data<'_>) {
        if request.method() != Method::Get || !is_inertia_request(request) {
            return;
        }

        let version = request
            .local_cache(|| Some(InertiaVersion((self.version_provider)())))
            .clone()
            .map(|version| version.0)
            .unwrap_or_default();
        let request_version = request.headers().get_one(X_INERTIA_VERSION).unwrap_or("");
        trace!(request_version, version = %version, "checking Inertia asset version");

        if request_version != version {
            let uri = uri!(
                "/inertia-rs",
                version_conflict(
                    location = request.uri().to_string(),
                    version = Some(version)
                )
            );
            request.set_uri(uri);
        }
    }

    async fn on_response<'r>(&self, request: &'r Request<'_>, response: &mut Response<'r>) {
        let varies = response.headers().get(VARY).any(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(X_INERTIA))
        });
        if !varies {
            response.adjoin_raw_header(VARY, X_INERTIA);
        }

        if !is_inertia_request(request) {
            return;
        }

        if response.status() == http::Status::Ok && response.body().preset_size() == Some(0) {
            let target = request
                .headers()
                .get_one("Referer")
                .unwrap_or("/")
                .to_owned();
            response.set_status(http::Status::Found);
            response.set_raw_header("Location", target);
        }

        if response.status() == http::Status::Found
            && matches!(
                request.method(),
                Method::Put | Method::Patch | Method::Delete
            )
        {
            response.set_status(http::Status::SeeOther);
        }

        let is_prefetch = request
            .headers()
            .get_one(PURPOSE)
            .is_some_and(|purpose| purpose.eq_ignore_ascii_case("prefetch"));
        let fragment_location = response
            .headers()
            .get_one("Location")
            .filter(|location| location.contains('#'))
            .map(ToOwned::to_owned);

        if response.status().class() == http::StatusClass::Redirection && !is_prefetch {
            if let Some(location) = fragment_location {
                response.set_status(http::Status::Conflict);
                response.remove_header("Location");
                response.set_raw_header(X_INERTIA_REDIRECT, location);
                response.set_sized_body(0, Cursor::new(Vec::<u8>::new()));
            }
        }
    }
}

#[cfg(test)]
mod tests;
