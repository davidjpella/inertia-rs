//! A Rust server adapter for the [Inertia.js](https://inertiajs.com/) protocol.
//!
//! The crate targets the Inertia 3.11 protocol (see
//! `docs/compatibility-target.md`). It exposes a framework-neutral protocol
//! core plus framework integrations behind feature flags:
//!
//! | Feature | Provides |
//! | --- | --- |
//! | `rocket` (default) | Rocket responders, request guard, and fairing |
//! | `axum` | Axum extractor, Tower layer, and session integration hooks |
//! | `tower-sessions` | Validation-error and flash storage backed by `tower-sessions` |
//! | `ssr` | HTTP client for the official Inertia SSR server |
//! | `experimental-layers` | Unreleased Layers protocol from inertiajs/inertia#3236 |
//!
//! # Props
//!
//! Plain serializable values work as page props. [`Props`] and [`Prop`] add
//! lazy, asynchronous, optional, deferred, once, merge, scroll, and live
//! behaviour:
//!
//! ```rust
//! use inertia_rs::{Inertia, Prop, Props};
//!
//! let response = Inertia::response(
//!     "Users/Index",
//!     Props::new()
//!         .value("filters", serde_json::json!({ "role": "admin" }))
//!         .prop("users", Prop::lazy_async(|| async { vec!["Ada", "Grace"] }))
//!         .prop("stats", Prop::lazy_async(|| async { 42 }).defer().rescue()),
//! );
//! assert_eq!(response.component(), "Users/Index");
//! ```
//!
//! Only props the current request needs are resolved: partial reloads,
//! deferred props, optional props, and once props the client already has
//! never call their providers.

#[cfg(feature = "rocket")]
pub mod rocket;

#[cfg(feature = "axum")]
pub mod axum;

pub mod bigint;
mod html;
#[cfg(feature = "experimental-layers")]
pub mod layers;
pub mod live;
mod page;
mod props;
mod request;
mod resolve;
pub mod session;
pub mod ssr;

pub use html::{escape_json_for_html_script, HtmlResponseContext};
pub use live::{BroadcastPayload, LiveChannel, LiveChannelType, LiveListener, LiveUpdates};
#[cfg(feature = "experimental-layers")]
pub use page::Layer;
pub use page::{OnceProp, Page, PageMetadata, ScrollPage, ScrollProps};
pub use props::{BoxError, InertiaProps, Prop, Props, ScopedInertiaProps};
pub use request::RequestContext;
pub use resolve::{PropErrorReporter, PropFailure, PropTimeout, ResolveOptions, Sleeper};

use serde::Serialize;
use serde_json::{Map, Value};
use std::fmt;
use std::time::Duration;

/// Request header set by Inertia XHR visits.
pub const X_REQUESTED_WITH: &str = "X-Requested-With";

/// Request header containing accepted response content types.
pub const ACCEPT: &str = "Accept";

/// Request and response header used to mark Inertia protocol requests.
pub const X_INERTIA: &str = "X-Inertia";

/// Request header containing the client's current asset version; also sent
/// with the server's version on a version-mismatch `409`.
pub const X_INERTIA_VERSION: &str = "X-Inertia-Version";

/// Request header containing the component targeted by a partial reload.
pub const X_INERTIA_PARTIAL_COMPONENT: &str = "X-Inertia-Partial-Component";

/// Request header listing props to include in a partial reload.
pub const X_INERTIA_PARTIAL_DATA: &str = "X-Inertia-Partial-Data";

/// Request header listing props to exclude from a partial reload.
pub const X_INERTIA_PARTIAL_EXCEPT: &str = "X-Inertia-Partial-Except";

/// Request header listing props to reset on navigation.
pub const X_INERTIA_RESET: &str = "X-Inertia-Reset";

/// Request header identifying a validation error bag.
pub const X_INERTIA_ERROR_BAG: &str = "X-Inertia-Error-Bag";

/// Request header used by Inertia's infinite scroll protocol.
pub const X_INERTIA_INFINITE_SCROLL_MERGE_INTENT: &str = "X-Inertia-Infinite-Scroll-Merge-Intent";

/// Request header listing once-prop keys the client already has.
pub const X_INERTIA_EXCEPT_ONCE_PROPS: &str = "X-Inertia-Except-Once-Props";

/// Response header used with `409 Conflict` to force a full-page visit.
pub const X_INERTIA_LOCATION: &str = "X-Inertia-Location";

/// Response header used with `409 Conflict` for fragment redirects.
pub const X_INERTIA_REDIRECT: &str = "X-Inertia-Redirect";

/// Response header used to separate HTML and JSON variants in caches.
pub const VARY: &str = "Vary";

/// Request header set to `prefetch` for Inertia prefetch requests.
pub const PURPOSE: &str = "Purpose";

/// Request header set to `no-cache` for Inertia reload requests.
pub const CACHE_CONTROL: &str = "Cache-Control";

/// Request header set on Precognition validation requests.
pub const PRECOGNITION: &str = "Precognition";

pub use live::X_SOCKET_ID;

/// Inertia protocol header constants.
///
/// Constants are also re-exported at the crate root.
pub mod headers {
    pub use super::{
        ACCEPT, CACHE_CONTROL, PRECOGNITION, PURPOSE, VARY, X_INERTIA, X_INERTIA_ERROR_BAG,
        X_INERTIA_EXCEPT_ONCE_PROPS, X_INERTIA_INFINITE_SCROLL_MERGE_INTENT, X_INERTIA_LOCATION,
        X_INERTIA_PARTIAL_COMPONENT, X_INERTIA_PARTIAL_DATA, X_INERTIA_PARTIAL_EXCEPT,
        X_INERTIA_REDIRECT, X_INERTIA_RESET, X_INERTIA_VERSION, X_REQUESTED_WITH, X_SOCKET_ID,
    };
}

/// Errors produced while building an Inertia page.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A value could not be serialized to JSON.
    Serialization(serde_json::Error),
    /// The page props did not serialize to a JSON object.
    InvalidProps,
    /// A prop combined behaviours that the protocol does not support.
    InvalidProp {
        /// The prop path.
        path: String,
        /// Why the combination is invalid.
        reason: &'static str,
    },
    /// A prop provider failed and the prop was not rescued.
    Prop {
        /// The prop path.
        path: String,
        /// The provider error.
        source: BoxError,
    },
    /// A prop provider exceeded the configured timeout and was not rescued.
    Timeout {
        /// The prop path.
        path: String,
        /// The configured timeout.
        after: Duration,
    },
    /// The response needs an asynchronous provider but was rendered through a
    /// synchronous API such as the Rocket `Responder` implementation.
    AsyncProp {
        /// The prop path.
        path: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialization(error) => write!(f, "failed to serialize Inertia page: {error}"),
            Self::InvalidProps => write!(f, "Inertia page props must serialize to a JSON object"),
            Self::InvalidProp { path, reason } => {
                write!(f, "invalid Inertia prop `{path}`: {reason}")
            }
            Self::Prop { path, source } => {
                write!(f, "Inertia prop `{path}` failed to resolve: {source}")
            }
            Self::Timeout { path, after } => {
                write!(f, "Inertia prop `{path}` timed out after {after:?}")
            }
            Self::AsyncProp { path } => write!(
                f,
                "Inertia prop `{path}` has an async provider; render the response with an async API"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialization(error) => Some(error),
            Self::Prop { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

/// Converts a value into page props.
///
/// Implemented for [`Props`] and for every serializable value. A serializable
/// value must serialize to a JSON object (or `null`, treated as no props);
/// each top-level key becomes a plain prop.
pub trait IntoPageProps<'a> {
    /// Converts `self` into a prop container.
    fn into_page_props(self) -> Result<Props<'a>, Error>;
}

impl<'a, T: Serialize> IntoPageProps<'a> for T {
    fn into_page_props(self) -> Result<Props<'a>, Error> {
        match serde_json::to_value(self)? {
            Value::Object(map) => Ok(map.into_iter().fold(Props::new(), |props, (key, value)| {
                props.prop(key, Prop::json(value))
            })),
            Value::Null => Ok(Props::new()),
            _ => Err(Error::InvalidProps),
        }
    }
}

impl<'a> IntoPageProps<'a> for Props<'a> {
    fn into_page_props(self) -> Result<Props<'a>, Error> {
        Ok(self)
    }
}

/// Request-scoped inputs for rendering a page.
///
/// Framework integrations build this from the incoming request, shared
/// props, and session state. Applications only need it when integrating
/// another framework.
pub struct RenderContext<'r, 'a> {
    request: &'r RequestContext,
    url: String,
    version: Option<String>,
    shared: Vec<(String, Prop<'a>)>,
    expose_shared_keys: bool,
    flash: Map<String, Value>,
    clear_history: bool,
    preserve_fragment: bool,
    preserve_big_integers: bool,
    #[cfg(feature = "experimental-layers")]
    interstitial: bool,
    options: ResolveOptions,
}

impl<'r, 'a> RenderContext<'r, 'a> {
    /// Creates a context for `request` with the page `url`.
    pub fn new<U: Into<String>>(request: &'r RequestContext, url: U) -> Self {
        Self {
            request,
            url: url.into(),
            version: None,
            shared: Vec::new(),
            expose_shared_keys: true,
            flash: Map::new(),
            clear_history: false,
            preserve_fragment: false,
            preserve_big_integers: false,
            #[cfg(feature = "experimental-layers")]
            interstitial: false,
            options: ResolveOptions::new(),
        }
    }

    /// Sets the asset version.
    pub fn version(mut self, version: Option<String>) -> Self {
        self.version = version;
        self
    }

    /// Adds a shared prop. Route props win on key collisions.
    pub fn share<K: Into<String>>(mut self, key: K, prop: Prop<'a>) -> Self {
        self.shared.push((key.into(), prop));
        self
    }

    /// Adds several shared props.
    pub fn share_all<I, K>(mut self, props: I) -> Self
    where
        I: IntoIterator<Item = (K, Prop<'a>)>,
        K: Into<String>,
    {
        self.shared
            .extend(props.into_iter().map(|(key, prop)| (key.into(), prop)));
        self
    }

    /// Controls whether shared root keys are listed in `sharedProps`
    /// (default `true`).
    pub fn expose_shared_keys(mut self, expose: bool) -> Self {
        self.expose_shared_keys = expose;
        self
    }

    /// Adds flash data loaded from the session.
    pub fn flash(mut self, flash: Map<String, Value>) -> Self {
        self.flash.extend(flash);
        self
    }

    /// Sets `clearHistory`, typically carried across a redirect.
    pub fn clear_history(mut self, clear: bool) -> Self {
        self.clear_history = clear;
        self
    }

    /// Sets `preserveFragment`, typically carried across a redirect.
    pub fn preserve_fragment(mut self, preserve: bool) -> Self {
        self.preserve_fragment = preserve;
        self
    }

    /// Enables big-integer preservation for every response rendered with
    /// this context.
    pub fn preserve_big_integers(mut self, preserve: bool) -> Self {
        self.preserve_big_integers = preserve;
        self
    }

    /// Sets the experimental `interstitial` flag, carried across a redirect.
    #[cfg(feature = "experimental-layers")]
    pub fn interstitial(mut self, interstitial: bool) -> Self {
        self.interstitial = interstitial;
        self
    }

    /// Sets async resolution options.
    pub fn options(mut self, options: ResolveOptions) -> Self {
        self.options = options;
        self
    }

    /// Returns the parsed request.
    pub fn request(&self) -> &RequestContext {
        self.request
    }
}

/// Builder for the advanced `Inertia::page(...).props(...)` API.
pub struct InertiaPageBuilder {
    component: String,
    url: Option<String>,
    metadata: PageMetadata,
}

macro_rules! metadata_builder_methods {
    () => {
        /// Marks the page's history state for encryption.
        pub fn encrypt_history(mut self) -> Self {
            self.metadata = self.metadata.encrypt_history();
            self
        }

        /// Marks the response as clearing encrypted history state.
        pub fn clear_history(mut self) -> Self {
            self.metadata = self.metadata.clear_history();
            self
        }

        /// Asks the client to keep the original request's URL fragment.
        pub fn preserve_fragment(mut self) -> Self {
            self.metadata = self.metadata.preserve_fragment();
            self
        }

        /// Wraps integers outside JavaScript's safe range in `$bigint`
        /// markers and sets `preserveBigIntegers`.
        pub fn preserve_big_integers(mut self) -> Self {
            self.metadata = self.metadata.preserve_big_integers();
            self
        }

        /// Marks a prop key to always be included during partial reloads.
        pub fn always<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.always(prop);
            self
        }

        /// Marks a prop path for append-style merging.
        pub fn merge<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.merge(prop);
            self
        }

        /// Marks a prop path for prepend-style merging.
        pub fn prepend<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.prepend(prop);
            self
        }

        /// Marks a prop path for deep merging.
        pub fn deep_merge<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.deep_merge(prop);
            self
        }

        /// Adds a `<propPath>.<field>` matching key used when merging.
        pub fn match_on<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.match_on(prop);
            self
        }

        /// Adds infinite-scroll metadata for a prop.
        pub fn scroll<P: Into<String>>(mut self, prop: P, scroll: ScrollProps) -> Self {
            self.metadata = self.metadata.scroll(prop, scroll);
            self
        }

        /// Defers a prop to a follow-up request in the default group.
        ///
        /// The value is omitted until a partial reload requests it. Use a
        /// lazy [`Prop`] so its provider does not run on the first visit.
        pub fn defer<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.defer(prop);
            self
        }

        /// Defers a prop to a follow-up request in `group`.
        pub fn defer_group<G: Into<String>, P: Into<String>>(mut self, group: G, prop: P) -> Self {
            self.metadata = self.metadata.defer_group(group, prop);
            self
        }

        /// Rescues failures of a deferred prop.
        pub fn rescue<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.rescue(prop);
            self
        }

        /// Lists a top-level key in `sharedProps`.
        pub fn share<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.share(prop);
            self
        }

        /// Marks a prop as a once prop keyed by its path.
        pub fn once<P: Into<String>>(mut self, prop: P) -> Self {
            self.metadata = self.metadata.once(prop);
            self
        }

        /// Marks a prop as a once prop with an explicit once key.
        pub fn once_with_key<K: Into<String>>(mut self, key: K, once: OnceProp) -> Self {
            self.metadata = self.metadata.once_with_key(key, once);
            self
        }
    };
}

impl InertiaPageBuilder {
    metadata_builder_methods!();

    /// Sets the page props and returns an [`Inertia`] response.
    pub fn props<T>(self, props: T) -> Inertia<T> {
        Inertia {
            component: self.component,
            props,
            url: self.url,
            metadata: self.metadata,
            flash: Map::new(),
            #[cfg(feature = "experimental-layers")]
            layer: None,
        }
    }

    /// Overrides the page object's `url` field.
    pub fn with_url<U: Into<String>>(mut self, url: U) -> Self {
        self.url = Some(url.into());
        self
    }
}

impl Inertia<()> {
    /// Starts the advanced page builder API.
    pub fn page<C: Into<String>>(component: C) -> InertiaPageBuilder {
        InertiaPageBuilder {
            component: component.into(),
            url: None,
            metadata: PageMetadata::new(),
        }
    }

    /// Creates an external redirect response.
    ///
    /// Framework integrations answer Inertia requests with `409 Conflict` and
    /// `X-Inertia-Location`, so the client performs a full `window.location`
    /// visit. Other requests receive an ordinary redirect.
    pub fn location<U: Into<String>>(url: U) -> Location {
        Location::new(url)
    }

    /// Creates a redirect response.
    ///
    /// Framework integrations use `303 See Other` after `POST`, `PUT`,
    /// `PATCH`, `DELETE`, and `QUERY` requests so the follow-up request is a
    /// `GET`, and turn redirects to URLs with a fragment into
    /// `X-Inertia-Redirect` responses for Inertia requests.
    pub fn redirect<U: Into<String>>(url: U) -> Redirect {
        Redirect::new(url)
    }
}

/// A server-initiated external location visit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    url: String,
}

impl Location {
    /// Creates an external location visit to `url`.
    pub fn new<U: Into<String>>(url: U) -> Self {
        Self { url: url.into() }
    }

    /// Returns the destination URL.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// A method-aware redirect response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Redirect {
    url: String,
}

impl Redirect {
    /// Creates a redirect response to `url`.
    pub fn new<U: Into<String>>(url: U) -> Self {
        Self { url: url.into() }
    }

    /// Returns the redirect destination URL.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// Returns `true` for methods whose `302` redirects become `303 See Other`.
///
/// The reference adapter converts redirects after `PUT`, `PATCH`, `DELETE`,
/// and `QUERY`. `POST` is included because browsers already follow a `302`
/// after `POST` with `GET`, so `303` states the same intent explicitly.
pub fn is_see_other_method(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE" | "QUERY"
    )
}

/// A framework-neutral Inertia page response.
///
/// Framework integrations convert this value into either an HTML first-load
/// response or a JSON Inertia response, depending on the request headers.
pub struct Inertia<T> {
    component: String,
    props: T,
    url: Option<String>,
    metadata: PageMetadata,
    flash: Map<String, Value>,
    #[cfg(feature = "experimental-layers")]
    layer: Option<Layer>,
}

impl<T> Inertia<T> {
    /// Constructs a response for `component` with `props`.
    ///
    /// Framework integrations default the page object's `url` field to the
    /// current request URI unless [`with_url`](Self::with_url) is used.
    pub fn response<C: Into<String>>(component: C, props: T) -> Self {
        Self {
            component: component.into(),
            props,
            url: None,
            metadata: PageMetadata::new(),
            flash: Map::new(),
            #[cfg(feature = "experimental-layers")]
            layer: None,
        }
    }

    /// Alias of [`Inertia::response`], matching the reference adapter's
    /// `render` naming.
    pub fn render<C: Into<String>>(component: C, props: T) -> Self {
        Self::response(component, props)
    }

    /// Overrides the page object's `url` field.
    pub fn with_url<U: Into<String>>(mut self, url: U) -> Self {
        self.url = Some(url.into());
        self
    }

    metadata_builder_methods!();

    /// Adds flash data to this response.
    ///
    /// Flash data is delivered once in the page's `flash` field and is not
    /// stored in history state. To flash before a redirect, use your
    /// framework integration's session flash API instead.
    pub fn flash<K, V>(mut self, key: K, value: V) -> Result<Self, Error>
    where
        K: Into<String>,
        V: Serialize,
    {
        self.flash.insert(key.into(), serde_json::to_value(value)?);
        Ok(self)
    }

    /// Renders the page as an experimental layer over `base`.
    ///
    /// The base URL is rendered beneath the layer when the URL is opened
    /// directly. Absolute URLs on the current origin are sent host-relative by
    /// framework integrations.
    #[cfg(feature = "experimental-layers")]
    pub fn layer(mut self, base: Option<String>, key: Option<String>) -> Self {
        self.layer = Some(Layer::new(base, key));
        self
    }

    /// Returns the Inertia component name.
    pub fn component(&self) -> &str {
        &self.component
    }

    /// Returns a reference to the component props.
    pub fn props(&self) -> &T {
        &self.props
    }

    /// Returns the explicit page URL override, if one was set.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// Returns the configured page metadata.
    pub fn metadata(&self) -> &PageMetadata {
        &self.metadata
    }

    /// Returns experimental layer metadata.
    #[cfg(feature = "experimental-layers")]
    pub fn layer_metadata(&self) -> Option<&Layer> {
        self.layer.as_ref()
    }
}

struct PageParts {
    component: String,
    url: String,
    version: Option<String>,
    flash: Map<String, Value>,
    clear_history: bool,
    preserve_fragment: bool,
    #[cfg(feature = "experimental-layers")]
    interstitial: bool,
    #[cfg(feature = "experimental-layers")]
    layer: Option<Layer>,
}

struct Prepared<'r, 'a> {
    request: &'r RequestContext,
    shared: Vec<(String, Prop<'a>)>,
    props: Props<'a>,
    expose_shared_keys: bool,
    options: ResolveOptions,
    metadata: PageMetadata,
    parts: PageParts,
}

impl Prepared<'_, '_> {
    fn finish(
        parts: PageParts,
        metadata: &PageMetadata,
        resolved: resolve::Resolved,
    ) -> Page<Value> {
        let resolve::Resolved {
            props,
            metadata: resolved_metadata,
        } = resolved;
        let mut flash = Value::Object(parts.flash);

        if metadata.preserve_big_integers {
            bigint::encode_big_integers(&mut flash);
        }

        let Value::Object(flash) = flash else {
            unreachable!("flash is an object")
        };

        let mut page = Page::from_resolved(
            parts.component,
            Value::Object(props),
            parts.url,
            parts.version,
            metadata,
            resolved_metadata,
        )
        .with_flash(flash);
        page.set_clear_history(parts.clear_history);
        page.set_preserve_fragment(parts.preserve_fragment);

        #[cfg(feature = "experimental-layers")]
        {
            page.set_interstitial(parts.interstitial);
            page.set_layer(parts.layer);
        }

        page
    }
}

impl<'a, T: IntoPageProps<'a>> Inertia<T> {
    fn prepare<'r>(self, context: RenderContext<'r, 'a>) -> Result<Prepared<'r, 'a>, Error> {
        let props = self.props.into_page_props()?;
        let RenderContext {
            request,
            url,
            version,
            shared,
            expose_shared_keys,
            mut flash,
            clear_history,
            preserve_fragment,
            preserve_big_integers,
            #[cfg(feature = "experimental-layers")]
            interstitial,
            options,
        } = context;
        flash.extend(self.flash);

        let mut metadata = self.metadata;
        metadata.preserve_big_integers |= preserve_big_integers;

        Ok(Prepared {
            request,
            shared,
            props,
            expose_shared_keys,
            options,
            parts: PageParts {
                component: self.component,
                url: self.url.unwrap_or(url),
                version,
                flash,
                clear_history,
                preserve_fragment,
                #[cfg(feature = "experimental-layers")]
                interstitial,
                #[cfg(feature = "experimental-layers")]
                layer: self.layer,
            },
            metadata,
        })
    }

    /// Resolves props and builds the page object, polling asynchronous
    /// providers concurrently.
    ///
    /// Framework integrations call this; use it directly when integrating
    /// another framework.
    pub async fn resolve<'r>(self, context: RenderContext<'r, 'a>) -> Result<Page<Value>, Error> {
        let Prepared {
            request,
            shared,
            props,
            expose_shared_keys,
            options,
            metadata,
            parts,
        } = self.prepare(context)?;
        let input = resolve::ResolveInput {
            component: &parts.component,
            request,
            shared,
            props,
            annotations: &metadata,
            expose_shared_keys,
            preserve_big_integers: metadata.preserve_big_integers,
        };
        let resolved = resolve::resolve_async(input, &options).await?;

        Ok(Prepared::finish(parts, &metadata, resolved))
    }

    /// Resolves props and builds the page object without an async runtime.
    ///
    /// Returns [`Error::AsyncProp`] when the request needs an asynchronous
    /// provider.
    pub fn resolve_sync(self, context: RenderContext<'_, 'a>) -> Result<Page<Value>, Error> {
        let Prepared {
            request,
            shared,
            props,
            expose_shared_keys,
            options,
            metadata,
            parts,
        } = self.prepare(context)?;
        let input = resolve::ResolveInput {
            component: &parts.component,
            request,
            shared,
            props,
            annotations: &metadata,
            expose_shared_keys,
            preserve_big_integers: metadata.preserve_big_integers,
        };
        let resolved = resolve::resolve_sync(input, &options)?;

        Ok(Prepared::finish(parts, &metadata, resolved))
    }

    /// Builds a concrete Inertia page object synchronously.
    ///
    /// Kept for compatibility with 0.4. Equivalent to
    /// [`resolve_sync`](Self::resolve_sync) with a context holding only the
    /// URL and version.
    pub fn into_page(
        self,
        url: impl Into<String>,
        version: Option<String>,
        request: &RequestContext,
    ) -> Result<Page<Value>, Error> {
        self.resolve_sync(RenderContext::new(request, url).version(version))
    }
}

#[cfg(test)]
mod tests;
