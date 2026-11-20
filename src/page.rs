//! The Inertia page object and response-level metadata.

use crate::live::LivePropMetadata;
use crate::resolve::ResolvedMetadata;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

fn is_false(value: &bool) -> bool {
    !value
}

fn empty_map<K, V>(map: &BTreeMap<K, V>) -> bool {
    map.is_empty()
}

fn serialize_version<S: Serializer>(version: &Option<String>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(version.as_deref().unwrap_or(""))
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

/// A page number or cursor used by infinite-scroll metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ScrollPage {
    /// A numeric page.
    Number(u64),
    /// An opaque cursor.
    Cursor(String),
}

impl From<u64> for ScrollPage {
    fn from(page: u64) -> Self {
        Self::Number(page)
    }
}

impl From<u32> for ScrollPage {
    fn from(page: u32) -> Self {
        Self::Number(page.into())
    }
}

impl From<i32> for ScrollPage {
    fn from(page: i32) -> Self {
        Self::Number(page.max(0) as u64)
    }
}

impl From<&str> for ScrollPage {
    fn from(cursor: &str) -> Self {
        Self::Cursor(cursor.to_owned())
    }
}

impl From<String> for ScrollPage {
    fn from(cursor: String) -> Self {
        Self::Cursor(cursor)
    }
}

/// Infinite-scroll metadata for one page prop (`scrollProps[path]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollProps {
    page_name: String,
    previous_page: Option<ScrollPage>,
    next_page: Option<ScrollPage>,
    current_page: Option<ScrollPage>,
    reset: bool,
}

impl ScrollProps {
    /// Creates scroll metadata for `page_name` at `current_page`.
    pub fn new<N: Into<String>, P: Into<ScrollPage>>(page_name: N, current_page: P) -> Self {
        Self {
            page_name: page_name.into(),
            previous_page: None,
            next_page: None,
            current_page: Some(current_page.into()),
            reset: false,
        }
    }

    /// Creates metadata for a numeric paginator.
    ///
    /// `previousPage` is set when `current_page > 1` and `nextPage` when
    /// `has_more` is true, like the reference adapter's paginator support.
    pub fn paginated<N: Into<String>>(page_name: N, current_page: u64, has_more: bool) -> Self {
        let mut scroll = Self::new(page_name, current_page);

        if current_page > 1 {
            scroll.previous_page = Some(ScrollPage::Number(current_page - 1));
        }

        if has_more {
            scroll.next_page = Some(ScrollPage::Number(current_page + 1));
        }

        scroll
    }

    /// Sets the previous page number or cursor.
    pub fn previous_page<P: Into<ScrollPage>>(mut self, previous_page: P) -> Self {
        self.previous_page = Some(previous_page.into());
        self
    }

    /// Sets the next page number or cursor.
    pub fn next_page<P: Into<ScrollPage>>(mut self, next_page: P) -> Self {
        self.next_page = Some(next_page.into());
        self
    }

    /// Returns the query parameter name used for pagination.
    pub fn page_name(&self) -> &str {
        &self.page_name
    }

    /// Returns the previous page, if any.
    pub fn previous(&self) -> Option<&ScrollPage> {
        self.previous_page.as_ref()
    }

    /// Returns the next page, if any.
    pub fn next(&self) -> Option<&ScrollPage> {
        self.next_page.as_ref()
    }

    /// Returns the current page, if any.
    pub fn current(&self) -> Option<&ScrollPage> {
        self.current_page.as_ref()
    }

    /// Returns whether this response resets the client's pagination state.
    pub fn is_reset(&self) -> bool {
        self.reset
    }

    pub(crate) fn set_reset(&mut self, reset: bool) {
        self.reset = reset;
    }
}

/// Metadata for a prop that should only be resolved once (`onceProps[key]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnceProp {
    prop: String,
    expires_at: Option<u64>,
}

impl OnceProp {
    /// Creates once-prop metadata for `prop`.
    pub fn new<P: Into<String>>(prop: P) -> Self {
        Self {
            prop: prop.into(),
            expires_at: None,
        }
    }

    /// Sets the client-side expiration timestamp in milliseconds.
    pub fn expires_at(mut self, expires_at: u64) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Returns the prop path associated with this once key.
    pub fn prop(&self) -> &str {
        &self.prop
    }

    /// Returns the optional expiration timestamp in milliseconds.
    pub fn expiration(&self) -> Option<u64> {
        self.expires_at
    }
}

/// Experimental layer metadata (`layer`) from
/// [inertiajs/inertia#3236](https://github.com/inertiajs/inertia/pull/3236).
#[cfg(feature = "experimental-layers")]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Layer {
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<String>,
}

#[cfg(feature = "experimental-layers")]
impl Layer {
    /// Creates layer metadata with optional base URL and key.
    pub fn new(base: Option<String>, key: Option<String>) -> Self {
        Self { key, base }
    }

    /// Returns the layer key. The client defaults it to the component name.
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// Returns the base page URL rendered beneath the layer.
    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    pub(crate) fn set_base(&mut self, base: String) {
        self.base = Some(base);
    }
}

/// Response-level page metadata and prop annotations.
///
/// Prop annotations such as [`merge`](Self::merge) or [`defer`](Self::defer)
/// address props by path and are applied to the matching prop before
/// resolution, so `Inertia::response("Feed", props).merge("posts")` behaves
/// like `Prop::value(posts).merge()`. Prefer configuring [`Prop`](crate::Prop)
/// values directly in new code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageMetadata {
    pub(crate) encrypt_history: bool,
    pub(crate) clear_history: bool,
    pub(crate) preserve_fragment: bool,
    pub(crate) preserve_big_integers: bool,
    always_props: Vec<String>,
    merge_props: Vec<String>,
    prepend_props: Vec<String>,
    deep_merge_props: Vec<String>,
    match_props_on: Vec<String>,
    scroll_props: BTreeMap<String, ScrollProps>,
    deferred_props: BTreeMap<String, Vec<String>>,
    rescued_props: Vec<String>,
    shared_props: Vec<String>,
    once_props: BTreeMap<String, OnceProp>,
}

impl PageMetadata {
    /// Creates empty page metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the page's history state for encryption.
    pub fn encrypt_history(mut self) -> Self {
        self.encrypt_history = true;
        self
    }

    /// Marks the response as clearing encrypted history state.
    pub fn clear_history(mut self) -> Self {
        self.clear_history = true;
        self
    }

    /// Asks the client to keep the original request's URL fragment.
    pub fn preserve_fragment(mut self) -> Self {
        self.preserve_fragment = true;
        self
    }

    /// Wraps integers outside JavaScript's safe range in `$bigint` markers.
    pub fn preserve_big_integers(mut self) -> Self {
        self.preserve_big_integers = true;
        self
    }

    /// Marks a prop key to always be included during partial reloads.
    pub fn always<P: Into<String>>(mut self, prop: P) -> Self {
        push_unique(&mut self.always_props, prop.into());
        self
    }

    /// Marks a prop path for append-style merging.
    pub fn merge<P: Into<String>>(mut self, prop: P) -> Self {
        push_unique(&mut self.merge_props, prop.into());
        self
    }

    /// Marks a prop path for prepend-style merging.
    pub fn prepend<P: Into<String>>(mut self, prop: P) -> Self {
        push_unique(&mut self.prepend_props, prop.into());
        self
    }

    /// Marks a prop path for deep merging.
    pub fn deep_merge<P: Into<String>>(mut self, prop: P) -> Self {
        push_unique(&mut self.deep_merge_props, prop.into());
        self
    }

    /// Adds a `<propPath>.<field>` matching key used when merging.
    pub fn match_on<P: Into<String>>(mut self, prop: P) -> Self {
        push_unique(&mut self.match_props_on, prop.into());
        self
    }

    /// Adds infinite-scroll metadata for a prop. Items merge at
    /// `<prop>.data`.
    pub fn scroll<P: Into<String>>(mut self, prop: P, scroll: ScrollProps) -> Self {
        let prop = prop.into();
        let merge_target = format!("{prop}.data");

        if !self.merge_props.contains(&merge_target) && !self.prepend_props.contains(&merge_target)
        {
            self.merge_props.push(merge_target);
        }

        self.scroll_props.insert(prop, scroll);
        self
    }

    /// Defers a prop to a follow-up request in the default group.
    pub fn defer<P: Into<String>>(self, prop: P) -> Self {
        self.defer_group("default", prop)
    }

    /// Defers a prop to a follow-up request in `group`.
    pub fn defer_group<G: Into<String>, P: Into<String>>(mut self, group: G, prop: P) -> Self {
        let props = self.deferred_props.entry(group.into()).or_default();
        push_unique(props, prop.into());
        self
    }

    /// Rescues failures of a deferred prop. Ignored for props that are not
    /// deferred.
    pub fn rescue<P: Into<String>>(mut self, prop: P) -> Self {
        push_unique(&mut self.rescued_props, prop.into());
        self
    }

    /// Lists a top-level key in `sharedProps`.
    ///
    /// Shared props registered with a framework integration are listed
    /// automatically.
    pub fn share<P: Into<String>>(mut self, prop: P) -> Self {
        push_unique(&mut self.shared_props, prop.into());
        self
    }

    /// Marks a prop as a once prop keyed by its path.
    pub fn once<P: Into<String>>(mut self, prop: P) -> Self {
        let prop = prop.into();
        self.once_props.insert(prop.clone(), OnceProp::new(prop));
        self
    }

    /// Marks a prop as a once prop with an explicit once key.
    pub fn once_with_key<K: Into<String>>(mut self, key: K, once: OnceProp) -> Self {
        self.once_props.insert(key.into(), once);
        self
    }

    /// Returns whether encrypted history is enabled.
    pub fn encrypt_history_enabled(&self) -> bool {
        self.encrypt_history
    }

    /// Returns whether clear history is enabled.
    pub fn clear_history_enabled(&self) -> bool {
        self.clear_history
    }

    /// Returns whether fragment preservation is enabled.
    pub fn preserve_fragment_enabled(&self) -> bool {
        self.preserve_fragment
    }

    /// Returns whether big integers are preserved.
    pub fn preserve_big_integers_enabled(&self) -> bool {
        self.preserve_big_integers
    }

    /// Returns props that always survive partial-reload filtering.
    pub fn always_props(&self) -> &[String] {
        &self.always_props
    }

    /// Returns append-merge prop paths.
    pub fn merge_props(&self) -> &[String] {
        &self.merge_props
    }

    /// Returns prepend-merge prop paths.
    pub fn prepend_props(&self) -> &[String] {
        &self.prepend_props
    }

    /// Returns deep-merge prop paths.
    pub fn deep_merge_props(&self) -> &[String] {
        &self.deep_merge_props
    }

    /// Returns merge matching keys.
    pub fn match_props_on(&self) -> &[String] {
        &self.match_props_on
    }

    /// Returns infinite-scroll prop metadata.
    pub fn scroll_props(&self) -> &BTreeMap<String, ScrollProps> {
        &self.scroll_props
    }

    /// Returns deferred prop groups.
    pub fn deferred_props(&self) -> &BTreeMap<String, Vec<String>> {
        &self.deferred_props
    }

    /// Returns rescued deferred prop paths.
    pub fn rescued_props(&self) -> &[String] {
        &self.rescued_props
    }

    /// Returns shared prop keys.
    pub fn shared_props(&self) -> &[String] {
        &self.shared_props
    }

    /// Returns once-prop metadata.
    pub fn once_props(&self) -> &BTreeMap<String, OnceProp> {
        &self.once_props
    }
}

/// A serializable Inertia page object.
///
/// Empty metadata fields are omitted, which the client reads the same as an
/// empty array, object, or `false`. `version` is always present and
/// serializes as `""` when no asset version is configured.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    component: String,
    props: T,
    url: String,
    #[serde(serialize_with = "serialize_version")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shared_props: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    merge_props: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    prepend_props: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deep_merge_props: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    match_props_on: Vec<String>,
    #[serde(skip_serializing_if = "empty_map")]
    deferred_props: BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    rescued_props: Vec<String>,
    #[serde(skip_serializing_if = "empty_map")]
    scroll_props: BTreeMap<String, ScrollProps>,
    #[serde(skip_serializing_if = "empty_map")]
    once_props: BTreeMap<String, OnceProp>,
    #[serde(skip_serializing_if = "empty_map")]
    live_props: BTreeMap<String, LivePropMetadata>,
    #[serde(skip_serializing_if = "is_false")]
    preserve_big_integers: bool,
    #[serde(skip_serializing_if = "is_false")]
    clear_history: bool,
    #[serde(skip_serializing_if = "is_false")]
    encrypt_history: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    flash: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "is_false")]
    preserve_fragment: bool,
    #[cfg(feature = "experimental-layers")]
    #[serde(skip_serializing_if = "is_false")]
    interstitial: bool,
    #[cfg(feature = "experimental-layers")]
    #[serde(skip_serializing_if = "Option::is_none")]
    layer: Option<Layer>,
    #[cfg(feature = "experimental-layers")]
    #[serde(skip_serializing_if = "is_false")]
    close: bool,
}

impl<T> Page<T> {
    /// Creates a minimal page object.
    pub fn new<C: Into<String>, U: Into<String>>(component: C, props: T, url: U) -> Self {
        Self::from_parts(component, props, url, None, PageMetadata::new())
    }

    /// Creates a page object from explicit parts.
    ///
    /// Metadata lists are copied as-is. Use
    /// [`Inertia::into_page`](crate::Inertia::into_page) to resolve props and
    /// derive request-specific metadata.
    pub fn from_parts<C: Into<String>, U: Into<String>>(
        component: C,
        props: T,
        url: U,
        version: Option<String>,
        metadata: PageMetadata,
    ) -> Self {
        Self {
            component: component.into(),
            props,
            url: url.into(),
            version,
            shared_props: metadata.shared_props,
            merge_props: metadata.merge_props,
            prepend_props: metadata.prepend_props,
            deep_merge_props: metadata.deep_merge_props,
            match_props_on: metadata.match_props_on,
            deferred_props: metadata.deferred_props,
            rescued_props: metadata.rescued_props,
            scroll_props: metadata.scroll_props,
            once_props: metadata.once_props,
            live_props: BTreeMap::new(),
            preserve_big_integers: metadata.preserve_big_integers,
            clear_history: metadata.clear_history,
            encrypt_history: metadata.encrypt_history,
            flash: None,
            preserve_fragment: metadata.preserve_fragment,
            #[cfg(feature = "experimental-layers")]
            interstitial: false,
            #[cfg(feature = "experimental-layers")]
            layer: None,
            #[cfg(feature = "experimental-layers")]
            close: false,
        }
    }

    pub(crate) fn from_resolved<C: Into<String>, U: Into<String>>(
        component: C,
        props: T,
        url: U,
        version: Option<String>,
        metadata: &PageMetadata,
        resolved: ResolvedMetadata,
    ) -> Self {
        let mut page = Self::from_parts(component, props, url, version, PageMetadata::new());
        page.shared_props = resolved.shared_props;
        page.merge_props = resolved.merge_props;
        page.prepend_props = resolved.prepend_props;
        page.deep_merge_props = resolved.deep_merge_props;
        page.match_props_on = resolved.match_props_on;
        page.deferred_props = resolved.deferred_props;
        page.rescued_props = resolved.rescued_props;
        page.scroll_props = resolved.scroll_props;
        page.once_props = resolved.once_props;
        page.live_props = resolved.live_props;
        page.preserve_big_integers = metadata.preserve_big_integers;
        page.clear_history = metadata.clear_history;
        page.encrypt_history = metadata.encrypt_history;
        page.preserve_fragment = metadata.preserve_fragment;
        page
    }

    /// Sets the page object's asset version.
    pub fn version<V: Into<String>>(mut self, version: V) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Sets the page's one-time flash data.
    pub fn with_flash(mut self, flash: Map<String, Value>) -> Self {
        self.flash = (!flash.is_empty()).then_some(flash);
        self
    }

    pub(crate) fn set_clear_history(&mut self, clear: bool) {
        self.clear_history |= clear;
    }

    pub(crate) fn set_preserve_fragment(&mut self, preserve: bool) {
        self.preserve_fragment |= preserve;
    }

    #[cfg(feature = "experimental-layers")]
    pub(crate) fn set_interstitial(&mut self, interstitial: bool) {
        self.interstitial |= interstitial;
    }

    #[cfg(feature = "experimental-layers")]
    pub(crate) fn set_layer(&mut self, layer: Option<Layer>) {
        self.layer = layer;
    }

    /// Returns the component name.
    pub fn component(&self) -> &str {
        &self.component
    }

    /// Returns the page props.
    pub fn props(&self) -> &T {
        &self.props
    }

    /// Returns mutable page props.
    pub fn props_mut(&mut self) -> &mut T {
        &mut self.props
    }

    /// Returns the page URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the asset version, if present.
    pub fn asset_version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns the flash data, if any.
    pub fn flash(&self) -> Option<&Map<String, Value>> {
        self.flash.as_ref()
    }

    /// Returns the `sharedProps` keys.
    pub fn shared_prop_keys(&self) -> &[String] {
        &self.shared_props
    }

    /// Returns the `deferredProps` groups.
    pub fn deferred_props(&self) -> &BTreeMap<String, Vec<String>> {
        &self.deferred_props
    }

    /// Returns the `rescuedProps` paths.
    pub fn rescued_props(&self) -> &[String] {
        &self.rescued_props
    }

    /// Returns the `liveProps` metadata.
    pub fn live_props(&self) -> &BTreeMap<String, LivePropMetadata> {
        &self.live_props
    }

    /// Returns experimental layer metadata.
    #[cfg(feature = "experimental-layers")]
    pub fn layer(&self) -> Option<&Layer> {
        self.layer.as_ref()
    }

    /// Returns whether this is an experimental layer close response.
    #[cfg(feature = "experimental-layers")]
    pub fn is_close(&self) -> bool {
        self.close
    }
}

#[cfg(feature = "experimental-layers")]
impl Page<Value> {
    /// Builds the experimental layer close page object:
    /// `{component: "", props: {}, url, version, close: true}`.
    pub fn close<U: Into<String>>(url: U, version: Option<String>) -> Self {
        let mut page = Self::from_parts("", Value::Object(Map::new()), url, version, PageMetadata::new());
        page.close = true;
        page
    }
}

impl Page<Value> {
    /// Merges already resolved shared props into the page object.
    ///
    /// Framework integrations resolve shared props together with route props.
    /// This helper remains for code that builds pages by hand. Existing page
    /// props win on root-key collisions, dotted keys expand into nested
    /// objects, and inserted roots are listed in `sharedProps`.
    pub fn with_shared_props<I, K>(mut self, shared_props: I) -> Self
    where
        I: IntoIterator<Item = (K, Value)>,
        K: Into<String>,
    {
        let mut shared_props = shared_props.into_iter().peekable();

        if shared_props.peek().is_none() {
            return self;
        }

        if !self.props.is_object() {
            self.props = Value::Object(Map::new());
        }

        let props = self
            .props
            .as_object_mut()
            .expect("props was normalized to an object");
        props
            .entry("errors")
            .or_insert_with(|| Value::Object(Map::new()));
        let route_roots = props.keys().cloned().collect::<BTreeSet<_>>();

        for (key, value) in shared_props {
            let key = key.into();
            let path = key
                .split('.')
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>();
            let Some(root) = path.first().map(|root| (*root).to_owned()) else {
                continue;
            };

            if route_roots.contains(&root) {
                continue;
            }

            if insert_path(props, &path, value) {
                push_unique(&mut self.shared_props, root);
            }
        }

        self
    }
}

fn insert_path(props: &mut Map<String, Value>, path: &[&str], value: Value) -> bool {
    match path {
        [] => false,
        [leaf] => {
            if props.contains_key(*leaf) {
                return false;
            }
            props.insert((*leaf).to_owned(), value);
            true
        }
        [head, rest @ ..] => {
            let entry = props
                .entry((*head).to_owned())
                .or_insert_with(|| Value::Object(Map::new()));
            match entry {
                Value::Object(nested) => insert_path(nested, rest, value),
                _ => false,
            }
        }
    }
}
