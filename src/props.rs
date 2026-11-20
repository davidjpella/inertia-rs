//! Typed page props and the route-local prop container.
//!
//! A [`Prop`] couples a value source (an eager value, a lazily serialized
//! value, a nested prop tree, or a synchronous or asynchronous provider) with
//! independent behaviour flags such as *always*, *optional*, *deferred*,
//! *once*, *merge*, *scroll*, *live*, and *rescue*. Flags compose the way the
//! Inertia protocol describes in its prop evaluation model, and combinations
//! that the reference adapter rejects are reported as
//! [`Error::InvalidProp`](crate::Error::InvalidProp) when the page is
//! rendered.

use crate::live::LiveUpdates;
use crate::page::ScrollProps;
use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Boxed error returned by fallible prop providers.
pub type BoxError = Box<dyn StdError + Send + Sync + 'static>;

pub(crate) type SyncProvider<'a> = Box<dyn FnOnce() -> Result<Option<Value>, BoxError> + Send + 'a>;
pub(crate) type AsyncProvider<'a> =
    Box<dyn FnOnce() -> BoxFuture<'a, Result<Option<Value>, BoxError>> + Send + 'a>;
pub(crate) type ScrollMetadataFn = Arc<dyn Fn(&Value) -> ScrollProps + Send + Sync>;

/// Where a prop's value comes from.
pub(crate) enum Source<'a> {
    /// An eager JSON value. Plain objects participate in nested partial-reload
    /// filtering, like arrays in the reference adapter.
    Value(Value),
    /// A value that is serialized only when the prop is included.
    Plain(SyncProvider<'a>),
    /// A nested prop tree.
    Nested(Props<'a>),
    /// A synchronous provider.
    Sync(SyncProvider<'a>),
    /// An asynchronous provider.
    Async(AsyncProvider<'a>),
}

/// When a prop is loaded relative to the first visit of a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Load {
    /// Loaded on full visits and matching partial reloads.
    Eager,
    /// Never loaded on full visits and never announced.
    Optional,
    /// Never loaded on full visits; announced in `deferredProps`.
    Deferred { group: String },
}

/// Once-prop configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OnceConfig {
    pub(crate) key: Option<String>,
    pub(crate) expires_at: Option<u64>,
    pub(crate) ttl: Option<Duration>,
    pub(crate) fresh: bool,
}

impl OnceConfig {
    pub(crate) fn expires_at_ms(&self) -> Option<u64> {
        if let Some(expires_at) = self.expires_at {
            return Some(expires_at);
        }

        self.ttl.map(|ttl| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            // The reference adapter computes a whole-second availability time.
            (now + ttl).as_secs() * 1000
        })
    }
}

/// Merge configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MergeConfig {
    pub(crate) deep: bool,
    pub(crate) append: bool,
    pub(crate) append_at: Vec<String>,
    pub(crate) prepend_at: Vec<String>,
    pub(crate) deep_at: Vec<String>,
    pub(crate) match_on: Vec<String>,
}

impl Default for MergeConfig {
    fn default() -> Self {
        Self {
            deep: false,
            append: true,
            append_at: Vec::new(),
            prepend_at: Vec::new(),
            deep_at: Vec::new(),
            match_on: Vec::new(),
        }
    }
}

impl MergeConfig {
    fn merges_at_root(&self) -> bool {
        self.append_at.is_empty() && self.prepend_at.is_empty() && self.deep_at.is_empty()
    }
}

/// Infinite-scroll configuration.
#[derive(Clone)]
pub(crate) struct ScrollConfig {
    pub(crate) wrapper: String,
    pub(crate) metadata: ScrollMetadataSource,
}

#[derive(Clone)]
pub(crate) enum ScrollMetadataSource {
    Static(ScrollProps),
    FromValue(ScrollMetadataFn),
}

/// A single Inertia page prop.
///
/// Construct a prop from a value or provider, then add behaviour:
///
/// ```rust
/// use inertia_rs::Prop;
///
/// let stats = Prop::lazy(|| 42).defer_group("sidebar");
/// let feed = Prop::value(vec!["post-1"]).merge().match_on("id");
/// let plans = Prop::lazy_async(|| async { vec!["basic", "pro"] }).once();
/// # let _ = (stats, feed, plans);
/// ```
///
/// Providers run only when the current request needs the prop. Asynchronous
/// providers selected for the same response are polled concurrently on the
/// request task.
pub struct Prop<'a> {
    pub(crate) source: Source<'a>,
    pub(crate) always: bool,
    pub(crate) load: Load,
    pub(crate) once: Option<OnceConfig>,
    pub(crate) merge: Option<MergeConfig>,
    pub(crate) scroll: Option<ScrollConfig>,
    pub(crate) live: Option<LiveUpdates>,
    pub(crate) rescue: bool,
    pub(crate) sensitive: bool,
}

impl fmt::Debug for Prop<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match &self.source {
            Source::Value(_) => "value",
            Source::Plain(_) => "plain",
            Source::Nested(_) => "nested",
            Source::Sync(_) => "sync",
            Source::Async(_) => "async",
        };

        f.debug_struct("Prop")
            .field("source", &source)
            .field("always", &self.always)
            .field("load", &self.load)
            .field("once", &self.once)
            .field("merge", &self.merge)
            .field("scroll", &self.scroll.is_some())
            .field("live", &self.live)
            .field("rescue", &self.rescue)
            .finish()
    }
}

fn serialize<T: Serialize>(value: T) -> Result<Option<Value>, BoxError> {
    serde_json::to_value(value)
        .map(Some)
        .map_err(|error| Box::new(error) as BoxError)
}

impl<'a> Prop<'a> {
    fn from_source(source: Source<'a>) -> Self {
        Self {
            source,
            always: false,
            load: Load::Eager,
            once: None,
            merge: None,
            scroll: None,
            live: None,
            rescue: false,
            sensitive: false,
        }
    }

    /// Creates a prop from a serializable value.
    ///
    /// The value is serialized only when the response includes the prop. A
    /// value that serializes to an object takes part in nested partial-reload
    /// filtering (`only: ['auth.user']`), like a plain array in the reference
    /// adapter.
    pub fn value<T>(value: T) -> Self
    where
        T: Serialize + Send + 'a,
    {
        Self::from_source(Source::Plain(Box::new(move || serialize(value))))
    }

    /// Creates a prop from an already serialized JSON value.
    pub fn json(value: Value) -> Self {
        Self::from_source(Source::Value(value))
    }

    /// Creates a prop from a nested prop tree.
    ///
    /// Nested typed props are addressed with dot-notation paths, such as
    /// `auth.permissions`, in metadata and partial reloads.
    pub fn nested(props: Props<'a>) -> Self {
        Self::from_source(Source::Nested(props))
    }

    /// Creates a lazily evaluated prop from a synchronous closure.
    ///
    /// Keep synchronous providers cheap: they run on the async request task.
    /// Use [`Prop::lazy_async`] for I/O, or move blocking work to a blocking
    /// thread pool inside an async provider.
    pub fn lazy<F, T>(provider: F) -> Self
    where
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        Self::from_source(Source::Sync(Box::new(move || serialize(provider()))))
    }

    /// Creates a lazily evaluated prop from a fallible synchronous closure.
    ///
    /// An `Err` fails the whole response unless the prop is
    /// [rescued](Prop::rescue).
    pub fn try_lazy<F, T, E>(provider: F) -> Self
    where
        F: FnOnce() -> Result<T, E> + Send + 'a,
        T: Serialize,
        E: Into<BoxError>,
    {
        Self::from_source(Source::Sync(Box::new(move || {
            provider().map_err(Into::into).and_then(serialize)
        })))
    }

    /// Creates a lazily evaluated prop from an asynchronous provider.
    ///
    /// The closure is called, and its future polled, only when the response
    /// includes the prop.
    pub fn lazy_async<F, Fut, T>(provider: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = T> + Send + 'a,
        T: Serialize,
    {
        Self::from_source(Source::Async(Box::new(move || {
            Box::pin(async move { serialize(provider().await) })
        })))
    }

    /// Creates a lazily evaluated prop from a fallible asynchronous provider.
    ///
    /// An `Err` fails the whole response unless the prop is
    /// [rescued](Prop::rescue).
    pub fn try_lazy_async<F, Fut, T, E>(provider: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = Result<T, E>> + Send + 'a,
        T: Serialize,
        E: Into<BoxError>,
    {
        Self::from_source(Source::Async(Box::new(move || {
            Box::pin(async move { provider().await.map_err(Into::into).and_then(serialize) })
        })))
    }

    /// Creates a lazily evaluated prop whose provider may omit the prop.
    ///
    /// Returning `None` leaves the key out of `props` instead of sending
    /// `null`.
    pub fn lazy_optional_value<F, T>(provider: F) -> Self
    where
        F: FnOnce() -> Option<T> + Send + 'a,
        T: Serialize,
    {
        Self::from_source(Source::Sync(Box::new(move || match provider() {
            Some(value) => serialize(value),
            None => Ok(None),
        })))
    }

    /// Creates a prop from an already computed outcome. `Ok(None)` omits the
    /// key; `Err` fails the response.
    #[cfg_attr(not(feature = "rocket"), allow(dead_code))]
    pub(crate) fn computed(outcome: Result<Option<Value>, BoxError>) -> Self {
        Self::from_source(Source::Sync(Box::new(move || outcome)))
    }

    /// Always includes the prop, even when a partial reload's `only` or
    /// `except` lists would leave it out.
    pub fn always(mut self) -> Self {
        self.always = true;
        self
    }

    /// Skips the prop on full visits without announcing it. Partial reloads
    /// that select the prop with `only` or `except` load it.
    pub fn optional(mut self) -> Self {
        self.load = Load::Optional;
        self
    }

    /// Defers the prop to a follow-up request in the `default` group.
    pub fn defer(self) -> Self {
        self.defer_group("default")
    }

    /// Defers the prop to a follow-up request in `group`.
    ///
    /// The client sends one partial reload per group.
    pub fn defer_group<G: Into<String>>(mut self, group: G) -> Self {
        self.load = Load::Deferred {
            group: group.into(),
        };
        self
    }

    /// Rescues provider failures for a deferred prop.
    ///
    /// A rescued prop that fails is left out of `props`, its path is added to
    /// `rescuedProps`, and the response still succeeds. The failure is passed
    /// to the configured prop error reporter. Only deferred props may be
    /// rescued.
    pub fn rescue(mut self) -> Self {
        self.rescue = true;
        self
    }

    /// Marks the prop as a once prop keyed by its path.
    ///
    /// The client remembers once props and sends their keys in
    /// `X-Inertia-Except-Once-Props`; full visits then skip the provider.
    pub fn once(mut self) -> Self {
        self.once.get_or_insert_with(OnceConfig::default);
        self
    }

    /// Marks the prop as a once prop with a custom key shared across pages.
    pub fn once_as<K: Into<String>>(mut self, key: K) -> Self {
        self.once.get_or_insert_with(OnceConfig::default).key = Some(key.into());
        self
    }

    /// Sets the once prop's client-side expiry as a Unix timestamp in
    /// milliseconds.
    pub fn expires_at(mut self, expires_at_ms: u64) -> Self {
        let once = self.once.get_or_insert_with(OnceConfig::default);
        once.expires_at = Some(expires_at_ms);
        once.ttl = None;
        self
    }

    /// Expires the once prop `ttl` after the response is rendered.
    pub fn once_for(mut self, ttl: Duration) -> Self {
        let once = self.once.get_or_insert_with(OnceConfig::default);
        once.ttl = Some(ttl);
        once.expires_at = None;
        self
    }

    /// Sends a once prop even when the client reports that it already has it.
    pub fn fresh(mut self) -> Self {
        self.once.get_or_insert_with(OnceConfig::default).fresh = true;
        self
    }

    fn merge_config(&mut self) -> &mut MergeConfig {
        self.merge.get_or_insert_with(MergeConfig::default)
    }

    /// Appends new values to the client's existing value on partial reloads.
    pub fn merge(mut self) -> Self {
        self.merge_config().append = true;
        self
    }

    /// Prepends new values to the client's existing value on partial reloads.
    pub fn prepend(mut self) -> Self {
        self.merge_config().append = false;
        self
    }

    /// Deep merges new values into the client's existing value.
    pub fn deep_merge(mut self) -> Self {
        self.merge_config().deep = true;
        self
    }

    /// Appends at a nested `path` of the prop, such as `data`.
    pub fn append_at<P: Into<String>>(mut self, path: P) -> Self {
        self.merge_config().append_at.push(path.into());
        self
    }

    /// Prepends at a nested `path` of the prop.
    pub fn prepend_at<P: Into<String>>(mut self, path: P) -> Self {
        self.merge_config().prepend_at.push(path.into());
        self
    }

    pub(crate) fn deep_merge_at<P: Into<String>>(mut self, path: P) -> Self {
        self.merge_config().deep_at.push(path.into());
        self
    }

    /// Matches merged items on `field`, relative to the prop path.
    ///
    /// For example, `Prop::value(posts).merge().match_on("id")` emits
    /// `matchPropsOn: ["posts.id"]`.
    pub fn match_on<F: Into<String>>(mut self, field: F) -> Self {
        self.merge_config().match_on.push(field.into());
        self
    }

    /// Configures the prop as an infinite-scroll prop with fixed pagination
    /// metadata.
    ///
    /// Items are merged at `<path>.data`; use [`Prop::scroll_wrapper`] for a
    /// different wrapper key. The merge direction follows the
    /// `X-Inertia-Infinite-Scroll-Merge-Intent` request header.
    pub fn scroll(mut self, metadata: ScrollProps) -> Self {
        self.scroll = Some(ScrollConfig {
            wrapper: self.scroll_wrapper_name(),
            metadata: ScrollMetadataSource::Static(metadata),
        });
        self
    }

    /// Configures the prop as an infinite-scroll prop whose pagination
    /// metadata is derived from the resolved value.
    pub fn scroll_with<F>(mut self, metadata: F) -> Self
    where
        F: Fn(&Value) -> ScrollProps + Send + Sync + 'static,
    {
        self.scroll = Some(ScrollConfig {
            wrapper: self.scroll_wrapper_name(),
            metadata: ScrollMetadataSource::FromValue(Arc::new(metadata)),
        });
        self
    }

    /// Changes the key that wraps an infinite-scroll prop's items.
    pub fn scroll_wrapper<W: Into<String>>(mut self, wrapper: W) -> Self {
        let wrapper = wrapper.into();

        match &mut self.scroll {
            Some(scroll) => scroll.wrapper = wrapper,
            None => {
                self.scroll = Some(ScrollConfig {
                    wrapper,
                    metadata: ScrollMetadataSource::Static(ScrollProps::new("page", 1)),
                })
            }
        }

        self
    }

    fn scroll_wrapper_name(&self) -> String {
        self.scroll
            .as_ref()
            .map(|scroll| scroll.wrapper.clone())
            .unwrap_or_else(|| "data".to_owned())
    }

    /// Refreshes the prop whenever one of the listed broadcast events
    /// arrives. Call it more than once to listen on several declarations.
    pub fn live(mut self, updates: LiveUpdates) -> Self {
        match &mut self.live {
            Some(existing) => existing.extend(updates),
            None => self.live = Some(updates),
        }
        self
    }

    /// Overrides the client throttle, in milliseconds, for a live prop.
    pub fn live_throttle(mut self, throttle_ms: u64) -> Self {
        self.live
            .get_or_insert_with(LiveUpdates::empty)
            .set_throttle(throttle_ms);
        self
    }

    /// Hides the prop's provider errors from tracing output.
    ///
    /// Prop values are never logged. Error messages are logged at `error`
    /// level unless the prop is marked sensitive.
    pub fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }

    pub(crate) fn is_plain(&self) -> bool {
        matches!(
            self.source,
            Source::Value(_) | Source::Plain(_) | Source::Nested(_)
        ) && !self.always
            && self.load == Load::Eager
            && self.once.is_none()
            && self.merge.is_none()
            && self.scroll.is_none()
            && self.live.is_none()
            && !self.rescue
    }

    pub(crate) fn is_deferred(&self) -> bool {
        matches!(self.load, Load::Deferred { .. })
    }

    /// Returns why this prop's flag combination is unsupported, if it is.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let merges = self.merge.is_some() || self.scroll.is_some();

        if self.live.is_some() && merges {
            return Err("live updates are not supported on merge or scroll props");
        }

        if self.always && self.load != Load::Eager {
            return Err("an always prop cannot be optional or deferred");
        }

        // `always` and `once` contradict each other; `always` and merge
        // metadata compose (the reference adapter cannot express it only
        // because of its class hierarchy).
        if self.always && (self.once.is_some() || self.scroll.is_some()) {
            return Err("an always prop cannot be a once or scroll prop");
        }

        if self.rescue && !self.is_deferred() {
            return Err("only deferred props can be rescued");
        }

        if self.scroll.is_some() && self.load == Load::Optional {
            return Err("a scroll prop cannot be optional");
        }

        if self.scroll.is_some() && self.once.is_some() {
            return Err("a scroll prop cannot be a once prop");
        }

        if let Some(live) = &self.live {
            live.validate()?;
        }

        if matches!(self.source, Source::Nested(_)) && !self.is_plain() {
            return Err(
                "nested prop trees cannot carry prop behaviour; configure the nested props",
            );
        }

        Ok(())
    }

    pub(crate) fn merge_metadata(&self, path: &str) -> MergeMetadata {
        let mut metadata = MergeMetadata::default();
        let Some(merge) = &self.merge else {
            return metadata;
        };

        if merge.deep {
            metadata.deep_merge.push(path.to_owned());
        } else if merge.merges_at_root() {
            if merge.append {
                metadata.merge.push(path.to_owned());
            } else {
                metadata.prepend.push(path.to_owned());
            }
        } else {
            for append in &merge.append_at {
                metadata.merge.push(format!("{path}.{append}"));
            }

            for prepend in &merge.prepend_at {
                metadata.prepend.push(format!("{path}.{prepend}"));
            }

            for deep in &merge.deep_at {
                metadata.deep_merge.push(format!("{path}.{deep}"));
            }
        }

        for field in &merge.match_on {
            metadata.match_on.push(format!("{path}.{field}"));
        }

        metadata
    }

    /// Applies the infinite-scroll merge intent before collecting metadata.
    pub(crate) fn configure_scroll_intent(&mut self, prepend: bool) {
        let Some(scroll) = &self.scroll else {
            return;
        };
        let wrapper = scroll.wrapper.clone();
        let merge = self.merge_config();
        merge.append_at.retain(|path| path != &wrapper);
        merge.prepend_at.retain(|path| path != &wrapper);

        if prepend {
            merge.prepend_at.push(wrapper);
        } else {
            merge.append_at.push(wrapper);
        }
    }
}

impl Prop<'static> {
    /// Narrows a `'static` prop to a shorter lifetime so it can be combined
    /// with borrowed route props.
    pub fn scoped<'a>(self) -> Prop<'a> {
        let source = match self.source {
            Source::Value(value) => Source::Value(value),
            Source::Plain(provider) => Source::Plain(provider),
            Source::Nested(props) => Source::Nested(Props {
                entries: props
                    .entries
                    .into_iter()
                    .map(|(key, prop)| (key, prop.scoped()))
                    .collect(),
            }),
            Source::Sync(provider) => Source::Sync(provider),
            Source::Async(provider) => {
                Source::Async(Box::new(move || -> BoxFuture<'a, _> { provider() }))
            }
        };

        Prop {
            source,
            always: self.always,
            load: self.load,
            once: self.once,
            merge: self.merge,
            scroll: self.scroll,
            live: self.live,
            rescue: self.rescue,
            sensitive: self.sensitive,
        }
    }
}

#[derive(Default)]
pub(crate) struct MergeMetadata {
    pub(crate) merge: Vec<String>,
    pub(crate) prepend: Vec<String>,
    pub(crate) deep_merge: Vec<String>,
    pub(crate) match_on: Vec<String>,
}

/// An ordered set of page props.
///
/// `Props` is the main route-local prop container. Keys may use dot notation
/// (`auth.user`) to address nested props. Later entries with the same key
/// replace earlier ones.
///
/// ```rust
/// use inertia_rs::{Inertia, Prop, Props};
///
/// let props = Props::new()
///     .value("user", "Ada")
///     .lazy_async("stats", || async { 42 })
///     .prop("feed", Prop::lazy(|| vec!["post"]).defer().merge());
/// let response = Inertia::response("Dashboard", props);
/// # let _ = response;
/// ```
///
/// Providers may borrow data from the handler: the lifetime `'a` lets a
/// handler keep references alive while the response is rendered.
#[derive(Default)]
pub struct Props<'a> {
    pub(crate) entries: Vec<(String, Prop<'a>)>,
}

impl fmt::Debug for Props<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.entries.iter().map(|(key, prop)| (key, prop)))
            .finish()
    }
}

impl<'a> Props<'a> {
    /// Creates an empty prop container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a typed prop.
    pub fn prop<K: Into<String>>(mut self, key: K, prop: Prop<'a>) -> Self {
        self.insert(key, prop);
        self
    }

    /// Adds a typed prop through a mutable reference.
    pub fn insert<K: Into<String>>(&mut self, key: K, prop: Prop<'a>) {
        let key = key.into();
        self.entries.retain(|(existing, _)| existing != &key);
        self.entries.push((key, prop));
    }

    /// Returns `true` when no props have been added.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of top-level entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the configured keys in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> + use<'_, 'a> {
        self.entries.iter().map(|(key, _)| key.as_str())
    }

    /// Adds a standard prop value.
    pub fn value<P, T>(self, prop: P, value: T) -> Self
    where
        P: Into<String>,
        T: Serialize + Send + 'a,
    {
        self.prop(prop, Prop::value(value))
    }

    /// Adds a standard lazy prop.
    pub fn lazy<P, F, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver))
    }

    /// Adds a standard asynchronous lazy prop.
    pub fn lazy_async<P, F, Fut, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = T> + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy_async(resolver))
    }

    /// Adds a prop that is only resolved when a partial reload selects it.
    pub fn optional<P, F, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver).optional())
    }

    /// Adds an asynchronous prop that is only resolved when a partial reload
    /// selects it.
    pub fn optional_async<P, F, Fut, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = T> + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy_async(resolver).optional())
    }

    /// Adds a prop that is always included, even during partial reloads.
    pub fn always<P, F, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver).always())
    }

    /// Adds a deferred prop in the default group.
    pub fn defer<P, F, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver).defer())
    }

    /// Adds an asynchronous deferred prop in the default group.
    pub fn defer_async<P, F, Fut, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = T> + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy_async(resolver).defer())
    }

    /// Adds a deferred prop in `group`.
    pub fn defer_group<G, P, F, T>(self, group: G, prop: P, resolver: F) -> Self
    where
        G: Into<String>,
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver).defer_group(group))
    }

    /// Adds an asynchronous deferred prop in `group`.
    pub fn defer_group_async<G, P, F, Fut, T>(self, group: G, prop: P, resolver: F) -> Self
    where
        G: Into<String>,
        P: Into<String>,
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = T> + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy_async(resolver).defer_group(group))
    }

    /// Adds a standard lazy prop with once-prop metadata.
    pub fn once<P, F, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver).once())
    }

    /// Adds a standard lazy prop with a custom once key.
    ///
    /// The prop is stored under [`OnceProp::prop`](crate::OnceProp::prop).
    pub fn once_with_key<K, F, T>(self, key: K, once: crate::OnceProp, resolver: F) -> Self
    where
        K: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        let prop = once.prop().to_owned();
        let mut typed = Prop::lazy(resolver).once_as(key);

        if let Some(expires_at) = once.expiration() {
            typed = typed.expires_at(expires_at);
        }

        self.prop(prop, typed)
    }

    /// Adds an optional prop with once-prop metadata.
    pub fn optional_once<P, F, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver).optional().once())
    }

    /// Adds a deferred prop with once-prop metadata in the default group.
    pub fn defer_once<P, F, T>(self, prop: P, resolver: F) -> Self
    where
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver).defer().once())
    }

    /// Adds a deferred prop with once-prop metadata in `group`.
    pub fn defer_group_once<G, P, F, T>(self, group: G, prop: P, resolver: F) -> Self
    where
        G: Into<String>,
        P: Into<String>,
        F: FnOnce() -> T + Send + 'a,
        T: Serialize,
    {
        self.prop(prop, Prop::lazy(resolver).defer_group(group).once())
    }

    pub(crate) fn into_entries(self) -> Vec<(String, Prop<'a>)> {
        self.entries
    }
}

/// Route-local props with `'static` providers.
///
/// Kept as an alias of [`Props`] for compatibility with 0.4.
pub type InertiaProps = Props<'static>;

/// Lifetime-aware route-local props.
///
/// Kept as an alias of [`Props`] for compatibility with 0.4.
pub type ScopedInertiaProps<'a> = Props<'a>;
