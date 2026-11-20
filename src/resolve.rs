//! Prop resolution.
//!
//! This module mirrors the reference adapter's `PropsResolver`:
//!
//! 1. **Plan.** Walk the prop tree, apply partial-reload filtering by path,
//!    skip props excluded from full visits (optional, deferred, and once props
//!    the client already has), and collect the metadata that does not depend on
//!    a resolved value. Providers are *not* called in this phase.
//! 2. **Run.** Call only the selected providers. Synchronous providers run in
//!    order; asynchronous providers are polled concurrently on the request
//!    task, optionally bounded by a concurrency limit and a per-prop timeout.
//! 3. **Assemble.** Insert resolved values, apply rescue handling, and collect
//!    the remaining metadata in tree order.

use crate::bigint::encode_big_integers;
use crate::live::LivePropMetadata;
use crate::page::{OnceProp, PageMetadata, ScrollProps};
use crate::props::{
    AsyncProvider, BoxError, Load, Prop, Props, ScrollMetadataSource, Source, SyncProvider,
};
use crate::request::RequestContext;
use crate::Error;
use futures_util::future::{self, BoxFuture, Either};
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, trace};

/// A sleep function used to enforce provider timeouts.
///
/// The core crate is runtime-agnostic; framework integrations supply a sleep
/// function from their runtime (Tokio for Axum and Rocket).
pub type Sleeper = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;

/// Reporter called for rescued prop failures.
pub type PropErrorReporter = Arc<dyn Fn(&PropFailure<'_>) + Send + Sync>;

/// A rescued prop failure passed to a [`PropErrorReporter`].
pub struct PropFailure<'e> {
    /// The prop path.
    pub path: &'e str,
    /// The provider error, or a timeout error.
    pub error: &'e (dyn std::error::Error + Send + Sync + 'static),
    /// Whether the prop was marked [`sensitive`](crate::Prop::sensitive).
    pub sensitive: bool,
}

/// Error returned when a provider exceeds its timeout.
#[derive(Debug)]
pub struct PropTimeout {
    /// The configured timeout.
    pub after: Duration,
}

impl fmt::Display for PropTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "prop provider timed out after {:?}", self.after)
    }
}

impl std::error::Error for PropTimeout {}

/// Controls how async prop providers are executed.
///
/// Providers are polled on the request task, so dropping the response future
/// (for example when the client disconnects) cancels every pending provider.
/// No provider is spawned onto another task.
#[derive(Clone, Default)]
pub struct ResolveOptions {
    concurrency: Option<usize>,
    timeout: Option<(Duration, Sleeper)>,
    reporter: Option<PropErrorReporter>,
}

impl fmt::Debug for ResolveOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolveOptions")
            .field("concurrency", &self.concurrency)
            .field("timeout", &self.timeout.as_ref().map(|(d, _)| d))
            .field("reporter", &self.reporter.is_some())
            .finish()
    }
}

impl ResolveOptions {
    /// Creates the default options: unbounded concurrency, no timeout, and
    /// rescued failures reported through `tracing`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Limits how many async providers are polled at the same time.
    ///
    /// `0` is treated as `1`.
    pub fn concurrency_limit(mut self, limit: usize) -> Self {
        self.concurrency = Some(limit.max(1));
        self
    }

    /// Fails any async provider that runs longer than `timeout`.
    ///
    /// A timed-out rescued prop is reported and listed in `rescuedProps`; any
    /// other timed-out prop fails the response with [`Error::Timeout`].
    pub fn timeout(mut self, timeout: Duration, sleeper: Sleeper) -> Self {
        self.timeout = Some((timeout, sleeper));
        self
    }

    /// Replaces the reporter for rescued prop failures.
    pub fn on_rescued_error<F>(mut self, reporter: F) -> Self
    where
        F: Fn(&PropFailure<'_>) + Send + Sync + 'static,
    {
        self.reporter = Some(Arc::new(reporter));
        self
    }

    /// Returns the configured concurrency limit.
    pub fn concurrency(&self) -> Option<usize> {
        self.concurrency
    }

    /// Returns the configured timeout.
    pub fn timeout_duration(&self) -> Option<Duration> {
        self.timeout.as_ref().map(|(duration, _)| *duration)
    }

    fn report(&self, failure: &PropFailure<'_>) {
        match &self.reporter {
            Some(reporter) => reporter(failure),
            None if failure.sensitive => {
                error!(prop = failure.path, "rescued Inertia prop failed")
            }
            None => error!(
                prop = failure.path,
                error = %failure.error,
                "rescued Inertia prop failed"
            ),
        }
    }
}

/// Metadata produced by resolution.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ResolvedMetadata {
    pub(crate) shared_props: Vec<String>,
    pub(crate) merge_props: Vec<String>,
    pub(crate) prepend_props: Vec<String>,
    pub(crate) deep_merge_props: Vec<String>,
    pub(crate) match_props_on: Vec<String>,
    pub(crate) deferred_props: BTreeMap<String, Vec<String>>,
    pub(crate) rescued_props: Vec<String>,
    pub(crate) scroll_props: BTreeMap<String, ScrollProps>,
    pub(crate) once_props: BTreeMap<String, OnceProp>,
    pub(crate) live_props: BTreeMap<String, LivePropMetadata>,
}

/// The outcome of resolving a page's props.
#[derive(Debug)]
pub(crate) struct Resolved {
    pub(crate) props: Map<String, Value>,
    pub(crate) metadata: ResolvedMetadata,
}

/// Inputs for one resolution.
pub(crate) struct ResolveInput<'a, 'r> {
    pub(crate) component: &'r str,
    pub(crate) request: &'r RequestContext,
    pub(crate) shared: Vec<(String, Prop<'a>)>,
    pub(crate) props: Props<'a>,
    pub(crate) annotations: &'r PageMetadata,
    pub(crate) expose_shared_keys: bool,
    pub(crate) preserve_big_integers: bool,
}

enum Node<'a> {
    Leaf(Prop<'a>),
    Branch(Vec<(String, Node<'a>)>),
}

fn root_of(path: &str) -> &str {
    path.split_once('.').map(|(root, _)| root).unwrap_or(path)
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn is_same_or_descendant(path: &str, parent: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// Converts a plain node into a branch so that a dotted key can be written
/// inside it. Non-object leaves are replaced, matching `Arr::set`.
fn into_branch(node: Node<'_>) -> Result<Vec<(String, Node<'_>)>, Error> {
    match node {
        Node::Branch(children) => Ok(children),
        Node::Leaf(prop) if prop.is_plain() => match prop.source {
            Source::Value(Value::Object(map)) => Ok(map
                .into_iter()
                .map(|(key, value)| (key, Node::Leaf(Prop::json(value))))
                .collect()),
            Source::Plain(provider) => match provider() {
                Ok(Some(Value::Object(map))) => Ok(map
                    .into_iter()
                    .map(|(key, value)| (key, Node::Leaf(Prop::json(value))))
                    .collect()),
                Ok(_) => Ok(Vec::new()),
                Err(error) => Err(Error::Prop {
                    path: String::new(),
                    source: error,
                }),
            },
            Source::Nested(props) => build_entries(props.into_entries()),
            _ => Ok(Vec::new()),
        },
        Node::Leaf(_) => Ok(Vec::new()),
    }
}

fn set_path<'a>(
    entries: &mut Vec<(String, Node<'a>)>,
    segments: &[&str],
    prop: Prop<'a>,
) -> Result<(), Error> {
    let (first, rest) = segments
        .split_first()
        .expect("set_path is called with at least one segment");

    if rest.is_empty() {
        let node = node_from_prop(prop)?;

        match entries.iter_mut().find(|(key, _)| key == first) {
            Some((_, existing)) => *existing = node,
            None => entries.push(((*first).to_owned(), node)),
        }

        return Ok(());
    }

    let index = match entries.iter().position(|(key, _)| key == first) {
        Some(index) => index,
        None => {
            entries.push(((*first).to_owned(), Node::Branch(Vec::new())));
            entries.len() - 1
        }
    };
    let node = std::mem::replace(&mut entries[index].1, Node::Branch(Vec::new()));
    let mut children = into_branch(node)?;
    set_path(&mut children, rest, prop)?;
    entries[index].1 = Node::Branch(children);

    Ok(())
}

fn node_from_prop(prop: Prop<'_>) -> Result<Node<'_>, Error> {
    match prop.source {
        Source::Nested(_) if prop.is_plain() => {
            let Source::Nested(props) = prop.source else {
                unreachable!("matched above")
            };
            Ok(Node::Branch(build_entries(props.into_entries())?))
        }
        _ => Ok(Node::Leaf(prop)),
    }
}

/// Builds a tree from ordered entries. Undotted keys are placed first and
/// dotted keys are then written into them, like the reference adapter's
/// `unpackDotProps`.
fn build_entries(entries: Vec<(String, Prop<'_>)>) -> Result<Vec<(String, Node<'_>)>, Error> {
    let mut tree = Vec::new();
    let mut dotted = Vec::new();

    for (key, prop) in entries {
        if key.contains('.') {
            dotted.push((key, prop));
        } else {
            let node = node_from_prop(prop)?;
            match tree.iter_mut().find(|(existing, _)| existing == &key) {
                Some((_, existing)) => *existing = node,
                None => tree.push((key, node)),
            }
        }
    }

    for (key, prop) in dotted {
        let segments = key
            .split('.')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();

        if segments.is_empty() {
            continue;
        }

        set_path(&mut tree, &segments, prop)?;
    }

    Ok(tree)
}

/// Merges shared and route props. Route props win: an undotted route key
/// replaces the shared subtree, and shared dotted keys never write into a
/// root owned by an undotted route key.
fn merge_shared<'a>(
    shared: Vec<(String, Prop<'a>)>,
    props: Vec<(String, Prop<'a>)>,
) -> Vec<(String, Prop<'a>)> {
    let route_roots = props
        .iter()
        .filter(|(key, _)| !key.contains('.'))
        .map(|(key, _)| key.clone())
        .collect::<BTreeSet<_>>();
    let mut merged = shared
        .into_iter()
        .filter(|(key, _)| !(key.contains('.') && route_roots.contains(root_of(key))))
        .collect::<Vec<_>>();

    for (key, prop) in props {
        match merged.iter_mut().find(|(existing, _)| existing == &key) {
            Some((_, existing)) => *existing = prop,
            None => merged.push((key, prop)),
        }
    }

    merged
}

#[derive(Clone, Copy)]
enum Mode {
    Full,
    Partial,
}

struct Filter<'r> {
    mode: Mode,
    is_inertia: bool,
    only: Option<BTreeSet<&'r str>>,
    except: Option<BTreeSet<&'r str>>,
    reset: BTreeSet<&'r str>,
    loaded_once: BTreeSet<&'r str>,
    prepend_intent: bool,
}

fn non_empty_set(values: &[String]) -> Option<BTreeSet<&str>> {
    (!values.is_empty()).then(|| values.iter().map(String::as_str).collect())
}

impl<'r> Filter<'r> {
    fn new(component: &str, request: &'r RequestContext) -> Self {
        let partial = request.partial_component() == Some(component);

        Self {
            mode: if partial { Mode::Partial } else { Mode::Full },
            is_inertia: request.is_inertia(),
            only: non_empty_set(request.partial_data()),
            except: non_empty_set(request.partial_except()),
            reset: request.reset().iter().map(String::as_str).collect(),
            loaded_once: request
                .except_once_props()
                .iter()
                .map(String::as_str)
                .collect(),
            prepend_intent: request.infinite_scroll_merge_intent() == Some("prepend"),
        }
    }

    fn is_partial(&self) -> bool {
        matches!(self.mode, Mode::Partial)
    }

    fn matches_only(&self, path: &str) -> bool {
        self.only.as_ref().is_some_and(|only| {
            only.iter()
                .any(|entry| is_same_or_descendant(path, entry))
        })
    }

    fn leads_to_only(&self, path: &str) -> bool {
        self.only.as_ref().is_some_and(|only| {
            only.iter().any(|entry| {
                entry
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('.'))
            })
        })
    }

    fn matches_except(&self, path: &str) -> bool {
        self.except.as_ref().is_some_and(|except| {
            except
                .iter()
                .any(|entry| is_same_or_descendant(path, entry))
        })
    }

    fn path_matches_partial_request(&self, path: &str) -> bool {
        if self.only.is_some() && !self.matches_only(path) && !self.leads_to_only(path) {
            return false;
        }

        !(self.except.is_some() && self.matches_except(path))
    }

    fn included_in_partial_metadata(&self, path: &str) -> bool {
        if self.only.is_some() && !self.matches_only(path) {
            return false;
        }

        !(self.except.is_some() && self.matches_except(path))
    }

    fn should_include(&self, bypass: bool, path: &str) -> bool {
        !self.is_partial() || bypass || self.path_matches_partial_request(path)
    }

    fn was_already_loaded(&self, prop: &Prop<'_>, path: &str) -> bool {
        prop.once.as_ref().is_some_and(|once| {
            !once.fresh
                && self
                    .loaded_once
                    .contains(once.key.as_deref().unwrap_or(path))
        })
    }
}

/// Metadata operations, ordered by the tree position of the prop that
/// produced them.
enum MetaOp {
    Deferred { group: String, path: String },
    Merge(crate::props::MergeMetadata),
    Scroll { path: String, scroll: ScrollProps },
    Once { key: String, once: OnceProp },
    Live { path: String, live: LivePropMetadata },
    Rescued(String),
}

enum Slot {
    Value(Value),
    Object(Vec<(String, Slot)>),
    Pending(usize),
}

enum TaskProvider<'a> {
    Sync(SyncProvider<'a>),
    Async(AsyncProvider<'a>),
}

/// Metadata that can only be collected once a provider has resolved.
struct DeferredMeta {
    prop: Prop<'static>,
    bypass: bool,
}

struct Task<'a> {
    path: String,
    seq: usize,
    rescue: bool,
    sensitive: bool,
    provider: Option<TaskProvider<'a>>,
    meta: DeferredMeta,
}

struct Planner<'r, 'a> {
    options: ResolveOptions,
    filter: Filter<'r>,
    annotations: Vec<Annotation>,
    seq: usize,
    ops: Vec<(usize, MetaOp)>,
    tasks: Vec<Task<'a>>,
}

/// A response-level annotation addressed by prop path.
#[derive(Clone)]
enum Annotation {
    Always(String),
    Defer { path: String, group: String },
    Rescue(String),
    Once { key: String, once: OnceProp },
    Merge(String),
    Prepend(String),
    DeepMerge(String),
    MatchOn(String),
    Scroll { path: String, scroll: ScrollProps },
}

impl Annotation {
    fn path(&self) -> &str {
        match self {
            Self::Always(path)
            | Self::Rescue(path)
            | Self::Merge(path)
            | Self::Prepend(path)
            | Self::DeepMerge(path) => path,
            Self::Defer { path, .. } | Self::Scroll { path, .. } => path,
            Self::Once { once, .. } => once.prop(),
            Self::MatchOn(path) => path
                .rsplit_once('.')
                .map(|(prop, _field)| prop)
                .unwrap_or(path),
        }
    }
}

fn annotations_from(metadata: &PageMetadata) -> Vec<Annotation> {
    let mut annotations = Vec::new();

    annotations.extend(metadata.always_props().iter().cloned().map(Annotation::Always));
    for (group, props) in metadata.deferred_props() {
        annotations.extend(props.iter().map(|path| Annotation::Defer {
            path: path.clone(),
            group: group.clone(),
        }));
    }
    annotations.extend(metadata.rescued_props().iter().cloned().map(Annotation::Rescue));
    for (key, once) in metadata.once_props() {
        annotations.push(Annotation::Once {
            key: key.clone(),
            once: once.clone(),
        });
    }
    for (path, scroll) in metadata.scroll_props() {
        annotations.push(Annotation::Scroll {
            path: path.clone(),
            scroll: scroll.clone(),
        });
    }

    // A scroll annotation already implies a merge at `<path>.data`, which the
    // 0.4 builder also listed in `merge_props`.
    let scroll_targets = metadata
        .scroll_props()
        .keys()
        .map(|path| format!("{path}.data"))
        .collect::<BTreeSet<_>>();
    annotations.extend(
        metadata
            .merge_props()
            .iter()
            .filter(|path| !scroll_targets.contains(*path))
            .cloned()
            .map(Annotation::Merge),
    );
    annotations.extend(
        metadata
            .prepend_props()
            .iter()
            .filter(|path| !scroll_targets.contains(*path))
            .cloned()
            .map(Annotation::Prepend),
    );
    annotations.extend(metadata.deep_merge_props().iter().cloned().map(Annotation::DeepMerge));
    annotations.extend(metadata.match_props_on().iter().cloned().map(Annotation::MatchOn));

    annotations
}

fn apply_exact<'a>(prop: Prop<'a>, annotation: &Annotation) -> Prop<'a> {
    match annotation {
        Annotation::Always(_) => prop.always(),
        Annotation::Defer { group, .. } => prop.defer_group(group.clone()),
        // 0.4 accepted `rescue` on any prop as metadata only. Rescue now
        // changes error handling, so it only applies to deferred props.
        Annotation::Rescue(_) if prop.is_deferred() => prop.rescue(),
        Annotation::Rescue(_) => prop,
        Annotation::Once { key, once } => {
            let mut prop = if key == once.prop() {
                prop.once()
            } else {
                prop.once_as(key.clone())
            };
            if let Some(expires_at) = once.expiration() {
                prop = prop.expires_at(expires_at);
            }
            prop
        }
        Annotation::Merge(_) => prop.merge(),
        Annotation::Prepend(_) => prop.prepend(),
        Annotation::DeepMerge(_) => prop.deep_merge(),
        Annotation::MatchOn(path) => {
            let field = path.rsplit_once('.').map(|(_, field)| field).unwrap_or(path);
            prop.match_on(field.to_owned())
        }
        Annotation::Scroll { scroll, .. } => prop.scroll(scroll.clone()),
    }
}

fn apply_relative<'a>(prop: Prop<'a>, annotation: &Annotation, base: &str) -> Prop<'a> {
    let relative = |path: &str| path[base.len() + 1..].to_owned();

    match annotation {
        Annotation::Merge(path) => prop.append_at(relative(path)),
        Annotation::Prepend(path) => prop.prepend_at(relative(path)),
        Annotation::DeepMerge(path) => prop.deep_merge_at(relative(path)),
        Annotation::MatchOn(path) => prop.match_on(relative(path)),
        // Other annotations cannot address a value inside a provider.
        _ => prop,
    }
}

impl<'r, 'a> Planner<'r, 'a> {
    fn next_seq(&mut self) -> usize {
        self.seq += 1;
        self.seq
    }

    fn apply_annotations(&self, mut prop: Prop<'a>, path: &str) -> Prop<'a> {
        let plain_object = prop.is_plain();

        for annotation in &self.annotations {
            let target = annotation.path();

            if target == path {
                prop = apply_exact(prop, annotation);
            } else if !plain_object && is_same_or_descendant(target, path) {
                prop = apply_relative(prop, annotation, path);
            }
        }

        prop
    }

    fn walk(
        &mut self,
        entries: Vec<(String, Node<'a>)>,
        prefix: &str,
        bypass: bool,
    ) -> Result<Vec<(String, Slot)>, Error> {
        let mut slots = Vec::new();

        for (key, node) in entries {
            let path = join(prefix, &key);

            match node {
                Node::Branch(children) => {
                    if !self.filter.should_include(bypass, &path) {
                        continue;
                    }

                    let children = self.walk(children, &path, bypass)?;
                    slots.push((key, Slot::Object(children)));
                }
                Node::Leaf(prop) => {
                    if let Some(slot) = self.visit(prop, &path, bypass)? {
                        slots.push((key, slot));
                    }
                }
            }
        }

        Ok(slots)
    }

    fn collect_deferred(&mut self, seq: usize, prop: &Prop<'_>, path: &str) {
        if let Load::Deferred { group } = &prop.load {
            if !self.filter.was_already_loaded(prop, path) {
                self.ops.push((
                    seq,
                    MetaOp::Deferred {
                        group: group.clone(),
                        path: path.to_owned(),
                    },
                ));
            }
        }
    }

    fn collect_merge(&mut self, seq: usize, prop: &Prop<'_>, path: &str) {
        if prop.merge.is_none() || self.filter.reset.contains(path) {
            return;
        }

        if self.filter.is_partial() && !self.filter.included_in_partial_metadata(path) {
            return;
        }

        self.ops.push((seq, MetaOp::Merge(prop.merge_metadata(path))));
    }

    fn collect_once(&mut self, seq: usize, prop: &Prop<'_>, path: &str) {
        let Some(once) = &prop.once else {
            return;
        };

        if self.filter.is_partial() && !self.filter.included_in_partial_metadata(path) {
            return;
        }

        let mut metadata = OnceProp::new(path);
        if let Some(expires_at) = once.expires_at_ms() {
            metadata = metadata.expires_at(expires_at);
        }

        self.ops.push((
            seq,
            MetaOp::Once {
                key: once.key.clone().unwrap_or_else(|| path.to_owned()),
                once: metadata,
            },
        ));
    }

    fn collect_live(&mut self, seq: usize, prop: &Prop<'_>, path: &str, bypass: bool) {
        let Some(live) = &prop.live else {
            return;
        };

        if self.filter.is_partial()
            && !(prop.always || bypass)
            && !self.filter.included_in_partial_metadata(path)
        {
            return;
        }

        self.ops.push((
            seq,
            MetaOp::Live {
                path: path.to_owned(),
                live: live.metadata(),
            },
        ));
    }

    fn collect_scroll(&mut self, seq: usize, prop: &Prop<'_>, path: &str, value: &Value) {
        let Some(scroll) = &prop.scroll else {
            return;
        };

        let mut metadata = match &scroll.metadata {
            ScrollMetadataSource::Static(metadata) => metadata.clone(),
            ScrollMetadataSource::FromValue(metadata) => metadata(value),
        };
        metadata.set_reset(self.filter.reset.contains(path));
        self.ops.push((
            seq,
            MetaOp::Scroll {
                path: path.to_owned(),
                scroll: metadata,
            },
        ));
    }

    fn collect_resolved(
        &mut self,
        seq: usize,
        prop: &Prop<'_>,
        path: &str,
        bypass: bool,
        value: &Value,
    ) {
        self.collect_merge(seq, prop, path);
        self.collect_scroll(seq, prop, path, value);
        self.collect_once(seq, prop, path);
        self.collect_live(seq, prop, path, bypass);
    }

    fn visit(&mut self, prop: Prop<'a>, path: &str, bypass: bool) -> Result<Option<Slot>, Error> {
        let mut prop = self.apply_annotations(prop, path);

        if !prop.is_plain() {
            prop.validate().map_err(|reason| Error::InvalidProp {
                path: path.to_owned(),
                reason,
            })?;
        }

        // `errors` is always included, like the reference middleware's
        // shared `Inertia::always()` errors prop.
        if path == "errors" {
            prop.always = true;
        }

        if !self.filter.should_include(prop.always || bypass, path) {
            return Ok(None);
        }

        let seq = self.next_seq();

        if !self.filter.is_partial() {
            // Excluding a prop never removes its live listeners.
            if prop.live.is_some() {
                self.collect_live(seq, &prop, path, bypass);
            }

            let ignored_first_load = prop.load == Load::Optional
                || (prop.is_deferred() && prop.scroll.is_none());

            if ignored_first_load {
                self.collect_deferred(seq, &prop, path);
                self.collect_merge(seq, &prop, path);
                self.collect_once(seq, &prop, path);
                return Ok(None);
            }

            if prop.is_deferred() {
                prop.configure_scroll_intent(self.filter.prepend_intent);
                self.collect_deferred(seq, &prop, path);
                self.collect_merge(seq, &prop, path);
                return Ok(None);
            }

            if self.filter.is_inertia && self.filter.was_already_loaded(&prop, path) {
                self.collect_once(seq, &prop, path);
                return Ok(None);
            }
        }

        prop.configure_scroll_intent(self.filter.prepend_intent);
        let plain = prop.is_plain();
        let rescue = prop.rescue;
        let sensitive = prop.sensitive;

        let source = std::mem::replace(&mut prop.source, Source::Value(Value::Null));
        let value = match source {
            Source::Value(value) => value,
            Source::Plain(provider) => match provider() {
                Ok(Some(value)) => value,
                Ok(None) => return Ok(None),
                Err(error) if rescue => {
                    self.rescue(seq, path, error.as_ref(), sensitive);
                    return Ok(None);
                }
                Err(source) => {
                    return Err(Error::Prop {
                        path: path.to_owned(),
                        source,
                    })
                }
            },
            Source::Nested(_) => unreachable!("nested props are expanded into branches"),
            Source::Sync(provider) => {
                return Ok(Some(self.task(prop, path, seq, bypass, TaskProvider::Sync(provider))))
            }
            Source::Async(provider) => {
                return Ok(Some(self.task(prop, path, seq, bypass, TaskProvider::Async(provider))))
            }
        };

        if plain {
            if let Value::Object(map) = value {
                let children = map
                    .into_iter()
                    .map(|(key, value)| (key, Node::Leaf(Prop::json(value))))
                    .collect();
                let children = self.walk(children, path, bypass)?;
                return Ok(Some(Slot::Object(children)));
            }

            return Ok(Some(Slot::Value(value)));
        }

        self.collect_resolved(seq, &prop, path, bypass, &value);
        Ok(Some(Slot::Value(value)))
    }

    fn task(
        &mut self,
        prop: Prop<'a>,
        path: &str,
        seq: usize,
        bypass: bool,
        provider: TaskProvider<'a>,
    ) -> Slot {
        let meta_prop = Prop {
            source: Source::Value(Value::Null),
            always: prop.always,
            load: prop.load.clone(),
            once: prop.once.clone(),
            merge: prop.merge.clone(),
            scroll: prop.scroll.clone(),
            live: prop.live.clone(),
            rescue: prop.rescue,
            sensitive: prop.sensitive,
        };
        let index = self.tasks.len();
        self.tasks.push(Task {
            path: path.to_owned(),
            seq,
            rescue: prop.rescue,
            sensitive: prop.sensitive,
            provider: Some(provider),
            meta: DeferredMeta {
                prop: meta_prop,
                bypass,
            },
        });

        Slot::Pending(index)
    }

    fn rescue(
        &mut self,
        seq: usize,
        path: &str,
        error: &(dyn std::error::Error + Send + Sync + 'static),
        sensitive: bool,
    ) {
        self.options.report(&PropFailure {
            path,
            error,
            sensitive,
        });
        self.ops.push((seq, MetaOp::Rescued(path.to_owned())));
    }
}


type TaskResult = Result<Option<Value>, BoxError>;

struct Plan<'r, 'a> {
    planner: Planner<'r, 'a>,
    slots: Vec<(String, Slot)>,
    shared_keys: Vec<String>,
}

fn plan<'r, 'a>(input: ResolveInput<'a, 'r>, options: &ResolveOptions) -> Result<Plan<'r, 'a>, Error> {
    let mut shared_keys = Vec::new();

    if input.expose_shared_keys {
        for (key, _) in &input.shared {
            let root = root_of(key).to_owned();
            if !shared_keys.contains(&root) {
                shared_keys.push(root);
            }
        }
    }

    for key in input.annotations.shared_props() {
        if !shared_keys.contains(key) {
            shared_keys.push(key.clone());
        }
    }

    let merged = merge_shared(input.shared, input.props.into_entries());
    let mut tree = build_entries(merged)?;

    if !tree.iter().any(|(key, _)| key == "errors") {
        tree.push((
            "errors".to_owned(),
            Node::Leaf(Prop::json(Value::Object(Map::new()))),
        ));
    }

    let mut planner = Planner {
        options: options.clone(),
        filter: Filter::new(input.component, input.request),
        annotations: annotations_from(input.annotations),
        seq: 0,
        ops: Vec::new(),
        tasks: Vec::new(),
    };
    let slots = planner.walk(tree, "", false)?;

    debug!(
        component = input.component,
        partial = planner.filter.is_partial(),
        providers = planner.tasks.len(),
        "planned Inertia prop resolution"
    );

    Ok(Plan {
        planner,
        slots,
        shared_keys,
    })
}

fn run_sync(provider: SyncProvider<'_>, path: &str) -> TaskResult {
    let started = Instant::now();
    let result = provider();
    trace!(prop = path, elapsed = ?started.elapsed(), "resolved sync Inertia prop");
    result
}

fn timed<'a>(
    future: BoxFuture<'a, TaskResult>,
    options: &ResolveOptions,
) -> BoxFuture<'a, TaskResult> {
    match &options.timeout {
        None => future,
        Some((after, sleeper)) => {
            let after = *after;
            let sleep = sleeper(after);
            Box::pin(async move {
                match future::select(future, sleep).await {
                    Either::Left((result, _)) => result,
                    Either::Right(((), _)) => Err(Box::new(PropTimeout { after }) as BoxError),
                }
            })
        }
    }
}

async fn run_async<'a>(
    providers: Vec<(usize, String, AsyncProvider<'a>)>,
    options: &ResolveOptions,
    results: &mut [Option<TaskResult>],
) {
    let limit = options.concurrency.unwrap_or(usize::MAX);
    let mut pending = providers.into_iter();
    let mut running = FuturesUnordered::new();

    let start = |(index, path, provider): (usize, String, AsyncProvider<'a>)| {
        let future = timed(provider(), options);
        async move {
            let started = Instant::now();
            let result = future.await;
            trace!(prop = %path, elapsed = ?started.elapsed(), "resolved async Inertia prop");
            (index, result)
        }
    };

    for provider in pending.by_ref().take(limit) {
        running.push(start(provider));
    }

    while let Some((index, result)) = running.next().await {
        results[index] = Some(result);

        if let Some(provider) = pending.next() {
            running.push(start(provider));
        }
    }
}

fn take_providers<'a>(
    tasks: &mut [Task<'a>],
) -> (Vec<(usize, String, SyncProvider<'a>)>, Vec<(usize, String, AsyncProvider<'a>)>) {
    let mut sync = Vec::new();
    let mut asynchronous = Vec::new();

    for (index, task) in tasks.iter_mut().enumerate() {
        match task.provider.take() {
            Some(TaskProvider::Sync(provider)) => sync.push((index, task.path.clone(), provider)),
            Some(TaskProvider::Async(provider)) => {
                asynchronous.push((index, task.path.clone(), provider))
            }
            None => {}
        }
    }

    (sync, asynchronous)
}

fn assemble(
    slots: Vec<(String, Slot)>,
    values: &mut [Option<Value>],
) -> Map<String, Value> {
    let mut map = Map::new();

    for (key, slot) in slots {
        match slot {
            Slot::Value(value) => {
                map.insert(key, value);
            }
            Slot::Object(children) => {
                map.insert(key, Value::Object(assemble(children, values)));
            }
            Slot::Pending(index) => {
                if let Some(value) = values[index].take() {
                    map.insert(key, value);
                }
            }
        }
    }

    map
}

fn finish_metadata(mut ops: Vec<(usize, MetaOp)>, metadata: &mut ResolvedMetadata) {
    ops.sort_by_key(|(seq, _)| *seq);

    for (_, op) in ops {
        match op {
            MetaOp::Deferred { group, path } => {
                let paths = metadata.deferred_props.entry(group).or_default();
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
            MetaOp::Merge(merge) => {
                metadata.merge_props.extend(merge.merge);
                metadata.prepend_props.extend(merge.prepend);
                metadata.deep_merge_props.extend(merge.deep_merge);
                metadata.match_props_on.extend(merge.match_on);
            }
            MetaOp::Scroll { path, scroll } => {
                metadata.scroll_props.insert(path, scroll);
            }
            MetaOp::Once { key, once } => {
                metadata.once_props.insert(key, once);
            }
            MetaOp::Live { path, live } => {
                metadata.live_props.insert(path, live);
            }
            MetaOp::Rescued(path) => {
                if !metadata.rescued_props.contains(&path) {
                    metadata.rescued_props.push(path);
                }
            }
        }
    }

    for list in [
        &mut metadata.merge_props,
        &mut metadata.prepend_props,
        &mut metadata.deep_merge_props,
        &mut metadata.match_props_on,
    ] {
        let mut seen = BTreeSet::new();
        list.retain(|value| seen.insert(value.clone()));
    }
}

fn finish(
    plan: Plan<'_, '_>,
    results: Vec<Option<TaskResult>>,
    preserve_big_integers: bool,
) -> Result<Resolved, Error> {
    let Plan {
        mut planner,
        slots,
        shared_keys,
    } = plan;
    let tasks = std::mem::take(&mut planner.tasks);
    let mut values = Vec::with_capacity(tasks.len());

    for (task, result) in tasks.into_iter().zip(results) {
        match result {
            Some(Ok(Some(value))) => {
                planner.collect_resolved(
                    task.seq,
                    &task.meta.prop,
                    &task.path,
                    task.meta.bypass,
                    &value,
                );
                values.push(Some(value));
            }
            Some(Ok(None)) | None => values.push(None),
            Some(Err(error)) if task.rescue => {
                planner.rescue(task.seq, &task.path, error.as_ref(), task.sensitive);
                values.push(None);
            }
            Some(Err(error)) => {
                return Err(match error.downcast::<PropTimeout>() {
                    Ok(timeout) => Error::Timeout {
                        path: task.path,
                        after: timeout.after,
                    },
                    Err(source) => Error::Prop {
                        path: task.path,
                        source,
                    },
                });
            }
        }
    }

    let mut props = assemble(slots, &mut values);
    let mut metadata = ResolvedMetadata::default();
    finish_metadata(planner.ops, &mut metadata);
    metadata.shared_props = shared_keys;

    if preserve_big_integers {
        for value in props.values_mut() {
            encode_big_integers(value);
        }
    }

    Ok(Resolved { props, metadata })
}

/// Resolves props without an async runtime.
///
/// Returns [`Error::AsyncProp`] when the response needs an async provider.
pub(crate) fn resolve_sync(
    input: ResolveInput<'_, '_>,
    options: &ResolveOptions,
) -> Result<Resolved, Error> {
    let preserve_big_integers = input.preserve_big_integers;
    let mut plan = plan(input, options)?;
    let (sync, asynchronous) = take_providers(&mut plan.planner.tasks);

    if let Some((_, path, _)) = asynchronous.first() {
        return Err(Error::AsyncProp { path: path.clone() });
    }

    let mut results = (0..plan.planner.tasks.len()).map(|_| None).collect::<Vec<_>>();
    for (index, path, provider) in sync {
        results[index] = Some(run_sync(provider, &path));
    }

    finish(plan, results, preserve_big_integers)
}

/// Resolves props, polling async providers concurrently.
pub(crate) async fn resolve_async(
    input: ResolveInput<'_, '_>,
    options: &ResolveOptions,
) -> Result<Resolved, Error> {
    let preserve_big_integers = input.preserve_big_integers;
    let mut plan = plan(input, options)?;
    let (sync, asynchronous) = take_providers(&mut plan.planner.tasks);
    let mut results = (0..plan.planner.tasks.len()).map(|_| None).collect::<Vec<_>>();

    for (index, path, provider) in sync {
        results[index] = Some(run_sync(provider, &path));
    }

    if !asynchronous.is_empty() {
        run_async(asynchronous, options, &mut results).await;
    }

    finish(plan, results, preserve_big_integers)
}
