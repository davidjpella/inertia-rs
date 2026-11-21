//! Inertia request header parsing.

use crate::headers::*;
use crate::live::X_SOCKET_ID;
use crate::page::PageMetadata;
use crate::props::{Prop, Props};
use crate::resolve::{resolve_sync, ResolveInput, ResolveOptions};
use serde_json::Value;

fn parse_header_list(value: Option<&str>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn contains_no_cache(value: &str) -> bool {
    value
        .split(',')
        .map(str::trim)
        .any(|part| part.eq_ignore_ascii_case("no-cache"))
}

/// Parsed Inertia request headers.
///
/// Framework integrations use this type to avoid duplicating header parsing
/// and partial-reload semantics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestContext {
    is_inertia: bool,
    version: Option<String>,
    partial_component: Option<String>,
    partial_data: Vec<String>,
    partial_except: Vec<String>,
    reset: Vec<String>,
    error_bag: Option<String>,
    infinite_scroll_merge_intent: Option<String>,
    except_once_props: Vec<String>,
    is_prefetch: bool,
    is_reload: bool,
    socket_id: Option<String>,
    is_precognitive: bool,
}

impl RequestContext {
    /// Parses a request context from a function that returns header values.
    ///
    /// The callback is invoked with canonical protocol header names such as
    /// `X-Inertia-Partial-Data`. HTTP frameworks usually handle
    /// case-insensitive lookup at this boundary.
    pub fn from_header_fn<'a, F>(mut header: F) -> Self
    where
        F: FnMut(&str) -> Option<&'a str>,
    {
        Self {
            is_inertia: header(X_INERTIA).is_some(),
            version: header(X_INERTIA_VERSION).map(ToOwned::to_owned),
            partial_component: header(X_INERTIA_PARTIAL_COMPONENT).map(ToOwned::to_owned),
            partial_data: parse_header_list(header(X_INERTIA_PARTIAL_DATA)),
            partial_except: parse_header_list(header(X_INERTIA_PARTIAL_EXCEPT)),
            reset: parse_header_list(header(X_INERTIA_RESET)),
            error_bag: header(X_INERTIA_ERROR_BAG).map(ToOwned::to_owned),
            infinite_scroll_merge_intent: header(X_INERTIA_INFINITE_SCROLL_MERGE_INTENT)
                .map(ToOwned::to_owned),
            except_once_props: parse_header_list(header(X_INERTIA_EXCEPT_ONCE_PROPS)),
            is_prefetch: header(PURPOSE)
                .map(|purpose| purpose.eq_ignore_ascii_case("prefetch"))
                .unwrap_or(false),
            is_reload: header(CACHE_CONTROL)
                .map(contains_no_cache)
                .unwrap_or(false),
            socket_id: header(X_SOCKET_ID).map(ToOwned::to_owned),
            is_precognitive: header(PRECOGNITION)
                .map(|value| value.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        }
    }

    /// Returns `true` when the request includes the `X-Inertia` header.
    pub fn is_inertia(&self) -> bool {
        self.is_inertia
    }

    /// Returns the request's `X-Inertia-Version` header value.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns the partial reload component, if present.
    pub fn partial_component(&self) -> Option<&str> {
        self.partial_component.as_deref()
    }

    /// Returns the props requested through `X-Inertia-Partial-Data`.
    pub fn partial_data(&self) -> &[String] {
        &self.partial_data
    }

    /// Returns the props excluded through `X-Inertia-Partial-Except`.
    pub fn partial_except(&self) -> &[String] {
        &self.partial_except
    }

    /// Returns the props requested for reset on navigation.
    pub fn reset(&self) -> &[String] {
        &self.reset
    }

    /// Returns the validation error bag name, if present.
    pub fn error_bag(&self) -> Option<&str> {
        self.error_bag.as_deref()
    }

    /// Returns the infinite-scroll merge intent, if present.
    pub fn infinite_scroll_merge_intent(&self) -> Option<&str> {
        self.infinite_scroll_merge_intent.as_deref()
    }

    /// Returns once-prop keys the client already has.
    pub fn except_once_props(&self) -> &[String] {
        &self.except_once_props
    }

    /// Returns `true` when the request purpose is `prefetch`.
    pub fn is_prefetch(&self) -> bool {
        self.is_prefetch
    }

    /// Returns `true` when the request cache control contains `no-cache`.
    pub fn is_reload(&self) -> bool {
        self.is_reload
    }

    /// Returns the broadcaster connection id from `X-Socket-Id`, if present.
    ///
    /// Live-prop broadcasters use it to skip the connection that caused a
    /// change.
    pub fn socket_id(&self) -> Option<&str> {
        self.socket_id.as_deref()
    }

    /// Returns `true` for Precognition validation requests.
    pub fn is_precognitive(&self) -> bool {
        self.is_precognitive
    }

    /// Returns `true` when partial reload headers target `component`.
    pub fn partial_reload_matches(&self, component: &str) -> bool {
        self.is_inertia && self.partial_component.as_deref() == Some(component)
    }

    /// Returns a copy with partial reload headers ignored.
    ///
    /// Framework integrations no longer call this: like the reference adapter,
    /// partial reload headers apply to every request method. Once-prop
    /// exclusions are preserved.
    pub fn without_partial_reload(mut self) -> Self {
        self.partial_component = None;
        self.partial_data.clear();
        self.partial_except.clear();
        self.reset.clear();
        self.infinite_scroll_merge_intent = None;
        self
    }

    /// Filters an already serialized props object according to this request.
    ///
    /// The object is resolved like route props (see [`Inertia::into_page`]),
    /// with `metadata` annotations applied, and always contains an `errors`
    /// object afterwards. Non-object values are left untouched.
    ///
    /// [`Inertia::into_page`]: crate::Inertia::into_page
    pub fn filter_props(&self, component: &str, props: &mut Value, metadata: &PageMetadata) {
        let Value::Object(map) = props else {
            return;
        };
        let entries = std::mem::take(map)
            .into_iter()
            .fold(Props::new(), |entries, (key, value)| {
                entries.prop(key, Prop::json(value))
            });
        let input = ResolveInput {
            component,
            request: self,
            shared: Vec::new(),
            props: entries,
            annotations: metadata,
            expose_shared_keys: false,
            preserve_big_integers: false,
        };

        if let Ok(resolved) = resolve_sync(input, &ResolveOptions::new()) {
            *map = resolved.props;
        }
    }
}
