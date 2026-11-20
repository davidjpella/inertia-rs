//! Live props: page metadata and broadcast payload helpers.
//!
//! A live prop lists the broadcast channels and events that refresh it. The
//! page object carries those listeners in `liveProps`, and the official
//! client subscribes through the transport configured in
//! `createInertiaApp({ live })` (Echo, `pusher-js`, Action Cable, or a custom
//! `LiveTransport`). When an event arrives the client issues an ordinary
//! partial reload for the listening props, or writes values carried in the
//! event's `__inertia.props` envelope directly into the page.
//!
//! This module is transport-agnostic. It produces the page metadata and the
//! broadcast envelope; delivering events is the job of whichever broadcaster
//! the application uses.

use serde::Serialize;
use serde_json::{Map, Value};
use std::fmt;

/// Header carrying the broadcaster connection id of the client that made the
/// request, so the server can skip broadcasting back to it.
pub const X_SOCKET_ID: &str = "X-Socket-Id";

/// The kind of broadcast channel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LiveChannelType {
    /// A public channel. Bare channel names are public.
    Public,
    /// A private channel (`private-` prefix on Pusher-compatible brokers).
    Private,
    /// A presence channel (`presence-` prefix on Pusher-compatible brokers).
    Presence,
    /// An encrypted private channel (`private-encrypted-` prefix).
    EncryptedPrivate,
    /// A broadcaster-specific channel type, passed to the client unchanged.
    Other(String),
}

impl LiveChannelType {
    /// Returns the type name used on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Presence => "presence",
            Self::EncryptedPrivate => "encrypted-private",
            Self::Other(other) => other,
        }
    }

    fn broker_prefix(&self) -> &'static str {
        match self {
            Self::Private => "private-",
            Self::Presence => "presence-",
            Self::EncryptedPrivate => "private-encrypted-",
            Self::Public | Self::Other(_) => "",
        }
    }
}

impl Serialize for LiveChannelType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A broadcast channel a live prop listens on.
///
/// Names are sent unprefixed; the client transport applies any prefix its
/// broadcaster expects. A broker prefix matching the type (`private-orders.1`
/// for a private channel) is stripped, like the reference adapter does.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LiveChannel {
    name: String,
    #[serde(rename = "type")]
    channel_type: LiveChannelType,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Map<String, Value>>,
}

impl LiveChannel {
    /// Creates a channel of `channel_type`.
    pub fn new<N: Into<String>>(name: N, channel_type: LiveChannelType) -> Self {
        let name = name.into();
        let prefix = channel_type.broker_prefix();
        let name = match name.strip_prefix(prefix) {
            Some(stripped) if !prefix.is_empty() => stripped.to_owned(),
            _ => name,
        };

        Self {
            name,
            channel_type,
            params: None,
        }
    }

    /// Creates a public channel.
    pub fn public<N: Into<String>>(name: N) -> Self {
        Self::new(name, LiveChannelType::Public)
    }

    /// Creates a private channel. Authorize subscriptions in your broadcaster.
    pub fn private<N: Into<String>>(name: N) -> Self {
        Self::new(name, LiveChannelType::Private)
    }

    /// Creates a presence channel.
    pub fn presence<N: Into<String>>(name: N) -> Self {
        Self::new(name, LiveChannelType::Presence)
    }

    /// Creates an encrypted private channel.
    pub fn encrypted_private<N: Into<String>>(name: N) -> Self {
        Self::new(name, LiveChannelType::EncryptedPrivate)
    }

    /// Adds transport parameters passed to the client untouched.
    ///
    /// A channel is identified by its type and name, so parameters must be
    /// stable for a given channel.
    pub fn with_params(mut self, params: Map<String, Value>) -> Self {
        self.params = Some(params);
        self
    }

    /// Returns the unprefixed channel name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the channel type.
    pub fn channel_type(&self) -> &LiveChannelType {
        &self.channel_type
    }

    /// Returns the channel name with the conventional broker prefix, such as
    /// `private-orders.1`.
    pub fn broker_name(&self) -> String {
        format!("{}{}", self.channel_type.broker_prefix(), self.name)
    }

    fn identity(&self) -> (String, String) {
        (self.channel_type.as_str().to_owned(), self.name.clone())
    }
}

impl From<&str> for LiveChannel {
    fn from(name: &str) -> Self {
        Self::public(name)
    }
}

impl From<String> for LiveChannel {
    fn from(name: String) -> Self {
        Self::public(name)
    }
}

/// One channel and the events that refresh a prop on it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LiveListener {
    /// The channel to subscribe to.
    pub channel: LiveChannel,
    /// Event names broadcast on the channel.
    pub events: Vec<String>,
}

/// The `liveProps` entry for one prop.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LivePropMetadata {
    /// Channel and event pairs.
    pub listeners: Vec<LiveListener>,
    /// Per-prop throttle in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throttle: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Default)]
struct Declaration {
    events: Vec<String>,
    channels: Vec<LiveChannel>,
}

/// Live-update configuration for a prop.
///
/// ```rust
/// use inertia_rs::{LiveChannel, LiveUpdates, Prop};
///
/// let order = Prop::lazy(|| "shipped").live(
///     LiveUpdates::on("OrderUpdated").channel(LiveChannel::private("orders.1")),
/// );
/// # let _ = order;
/// ```
///
/// Every declaration needs at least one event and one channel. Unlike the
/// Laravel adapter, channels cannot be inferred from an event class.
#[derive(Clone, PartialEq, Default)]
pub struct LiveUpdates {
    declarations: Vec<Declaration>,
    throttle: Option<u64>,
}

impl fmt::Debug for LiveUpdates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveUpdates")
            .field("listeners", &self.listeners())
            .field("throttle", &self.throttle)
            .finish()
    }
}

impl LiveUpdates {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    /// Starts a declaration listening for `event`.
    pub fn on<E: Into<String>>(event: E) -> Self {
        Self {
            declarations: vec![Declaration {
                events: vec![event.into()],
                channels: Vec::new(),
            }],
            throttle: None,
        }
    }

    /// Starts a declaration listening for several events.
    pub fn on_events<I, E>(events: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: Into<String>,
    {
        Self {
            declarations: vec![Declaration {
                events: events.into_iter().map(Into::into).collect(),
                channels: Vec::new(),
            }],
            throttle: None,
        }
    }

    fn current(&mut self) -> &mut Declaration {
        if self.declarations.is_empty() {
            self.declarations.push(Declaration::default());
        }

        self.declarations
            .last_mut()
            .expect("a declaration was just ensured")
    }

    /// Adds another event to the current declaration.
    pub fn event<E: Into<String>>(mut self, event: E) -> Self {
        self.current().events.push(event.into());
        self
    }

    /// Adds a channel to the current declaration. Strings are public
    /// channels.
    pub fn channel<C: Into<LiveChannel>>(mut self, channel: C) -> Self {
        self.current().channels.push(channel.into());
        self
    }

    /// Sets the per-prop throttle in milliseconds.
    pub fn throttle(mut self, throttle_ms: u64) -> Self {
        self.throttle = Some(throttle_ms);
        self
    }

    pub(crate) fn set_throttle(&mut self, throttle_ms: u64) {
        self.throttle = Some(throttle_ms);
    }

    pub(crate) fn extend(&mut self, other: LiveUpdates) {
        self.declarations.extend(other.declarations);

        if other.throttle.is_some() {
            self.throttle = other.throttle;
        }
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if !self.declarations.iter().any(|d| !d.events.is_empty()) {
            return Err("a live prop must listen for at least one event");
        }

        if self.declarations.iter().any(|d| d.channels.is_empty()) {
            return Err("every live declaration needs at least one channel");
        }

        Ok(())
    }

    /// Returns the listeners, grouped by channel, in declaration order.
    pub fn listeners(&self) -> Vec<LiveListener> {
        let mut listeners: Vec<LiveListener> = Vec::new();

        for declaration in &self.declarations {
            for event in &declaration.events {
                for channel in &declaration.channels {
                    let identity = channel.identity();
                    let index = listeners
                        .iter()
                        .position(|listener| listener.channel.identity() == identity);
                    let listener = match index {
                        Some(index) => &mut listeners[index],
                        None => {
                            listeners.push(LiveListener {
                                channel: channel.clone(),
                                events: Vec::new(),
                            });
                            listeners.last_mut().expect("listener was pushed")
                        }
                    };

                    if !listener.events.contains(event) {
                        listener.events.push(event.clone());
                    }
                }
            }
        }

        listeners
    }

    pub(crate) fn metadata(&self) -> LivePropMetadata {
        LivePropMetadata {
            listeners: self.listeners(),
            throttle: self.throttle,
        }
    }
}

/// Builder for the `__inertia` broadcast envelope.
///
/// Events may carry new prop values so listening clients update without a
/// request. Every subscriber of a channel receives the same payload, so only
/// include values that every subscriber may see.
///
/// ```rust
/// use inertia_rs::BroadcastPayload;
/// use serde_json::json;
///
/// let payload = BroadcastPayload::new()
///     .prop("order", json!({ "id": 1, "status": "shipped" }))
///     .unwrap()
///     .into_payload(json!({ "order_id": 1 }));
///
/// assert_eq!(payload["__inertia"]["props"]["order"]["status"], "shipped");
/// assert_eq!(payload["order_id"], 1);
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BroadcastPayload {
    props: Map<String, Value>,
    event: Option<String>,
    socket_id: Option<String>,
}

impl BroadcastPayload {
    /// Creates an empty envelope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a prop value, keyed by prop path (dot notation for nested
    /// props).
    pub fn prop<K, T>(mut self, key: K, value: T) -> Result<Self, serde_json::Error>
    where
        K: Into<String>,
        T: Serialize,
    {
        self.props.insert(key.into(), serde_json::to_value(value)?);
        Ok(self)
    }

    /// Sets `__inertia.event`, used by channel transports that cannot route
    /// by event name (such as Action Cable).
    pub fn event<E: Into<String>>(mut self, event: E) -> Self {
        self.event = Some(event.into());
        self
    }

    /// Sets `__inertia.socketId`, the connection that caused the broadcast.
    /// The client that owns that connection drops the event. Use it when the
    /// broadcaster cannot exclude a connection itself.
    pub fn socket_id<S: Into<String>>(mut self, socket_id: S) -> Self {
        self.socket_id = Some(socket_id.into());
        self
    }

    /// Returns the `__inertia` envelope object.
    pub fn envelope(&self) -> Value {
        let mut envelope = Map::new();

        if !self.props.is_empty() {
            envelope.insert("props".into(), Value::Object(self.props.clone()));
        }

        if let Some(event) = &self.event {
            envelope.insert("event".into(), Value::String(event.clone()));
        }

        if let Some(socket_id) = &self.socket_id {
            envelope.insert("socketId".into(), Value::String(socket_id.clone()));
        }

        Value::Object(envelope)
    }

    /// Merges the envelope into an application payload object.
    ///
    /// Non-object payloads are replaced by an object holding only the
    /// envelope.
    pub fn into_payload(self, payload: Value) -> Value {
        let mut payload = match payload {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        payload.insert("__inertia".into(), self.envelope());
        Value::Object(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listeners_group_events_by_channel_and_dedupe() {
        let updates = LiveUpdates::on("OrderUpdated")
            .event("OrderShipped")
            .channel(LiveChannel::private("private-orders.1"))
            .channel("orders");
        let mut updates = updates;
        updates.extend(LiveUpdates::on("OrderUpdated").channel(LiveChannel::private("orders.1")));

        assert_eq!(
            serde_json::to_value(updates.listeners()).unwrap(),
            json!([
                {
                    "channel": { "name": "orders.1", "type": "private" },
                    "events": ["OrderUpdated", "OrderShipped"]
                },
                {
                    "channel": { "name": "orders", "type": "public" },
                    "events": ["OrderUpdated", "OrderShipped"]
                }
            ])
        );
    }

    #[test]
    fn live_updates_require_events_and_channels() {
        assert!(LiveUpdates::on("A").validate().is_err());
        assert!(LiveUpdates::empty().channel("a").validate().is_err());
        assert!(LiveUpdates::on("A").channel("a").validate().is_ok());
    }

    #[test]
    fn channel_prefixes_are_stripped_for_matching_types_only() {
        assert_eq!(LiveChannel::presence("presence-room.1").name(), "room.1");
        assert_eq!(
            LiveChannel::encrypted_private("private-encrypted-x").name(),
            "x"
        );
        assert_eq!(LiveChannel::public("private-x").name(), "private-x");
        assert_eq!(
            LiveChannel::private("orders.1").broker_name(),
            "private-orders.1"
        );
    }

    #[test]
    fn broadcast_payload_builds_envelope() {
        let payload = BroadcastPayload::new()
            .prop("stats.count", 3)
            .unwrap()
            .event("StatsUpdated")
            .socket_id("123.456")
            .into_payload(json!(null));

        assert_eq!(
            payload,
            json!({
                "__inertia": {
                    "props": { "stats.count": 3 },
                    "event": "StatsUpdated",
                    "socketId": "123.456"
                }
            })
        );
    }
}
