//! A Server-Sent Events broadcaster for live props.
//!
//! Live props work with any broadcaster the official client can reach through
//! a `LiveTransport` (Echo, `pusher-js`, Action Cable, or a custom transport).
//! This module provides a small, replaceable in-process option for Axum apps
//! that do not run a broadcast server:
//!
//! - [`Broadcaster`] publishes events to subscribers in this process.
//! - [`Broadcaster::sse`] streams a channel's events to an `EventSource`,
//!   after a [`ChannelAuthorizer`] approves private, presence, and encrypted
//!   channels.
//! - The browser side is a custom `LiveTransport` (see
//!   `examples/axum-react/frontend/src/liveTransport.js`) passed to
//!   `createInertiaApp({ live })`.
//!
//! The wire format between the SSE endpoint and that transport is specific
//! to this module. Everything Inertia sees (`liveProps`, event names, the
//! `__inertia` envelope, and `X-Socket-Id`) follows the official protocol.
//!
//! Events reach only the subscribers of the process that broadcast them. Run
//! a shared broker (for example a Pusher-compatible server) when the
//! application runs on several instances.

use super::InertiaRequest;
use crate::live::{BroadcastPayload, LiveChannel, LiveChannelType};
use ::axum::extract::Query;
use ::axum::http::StatusCode;
use ::axum::response::sse::{Event, KeepAlive, Sse};
use ::axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::convert::Infallible;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// Decides whether a request may subscribe to a non-public channel.
pub trait ChannelAuthorizer: Send + Sync {
    /// Returns `true` when `request` may subscribe to `channel`.
    fn authorize<'a>(
        &'a self,
        request: &'a InertiaRequest,
        channel: &'a LiveChannel,
    ) -> BoxFuture<'a, bool>;
}

impl<F> ChannelAuthorizer for F
where
    F: Fn(&InertiaRequest, &LiveChannel) -> bool + Send + Sync,
{
    fn authorize<'a>(
        &'a self,
        request: &'a InertiaRequest,
        channel: &'a LiveChannel,
    ) -> BoxFuture<'a, bool> {
        let allowed = self(request, channel);
        Box::pin(async move { allowed })
    }
}

/// Denies every non-public channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct PublicOnly;

impl ChannelAuthorizer for PublicOnly {
    fn authorize<'a>(
        &'a self,
        _request: &'a InertiaRequest,
        _channel: &'a LiveChannel,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
}

#[derive(Clone, Debug)]
struct Message {
    channel: (String, String),
    event: String,
    payload: Arc<Value>,
    except_socket: Option<String>,
}

/// Publishes broadcast events to SSE subscribers in this process.
///
/// Clone it freely; clones share subscribers.
#[derive(Clone)]
pub struct Broadcaster {
    sender: broadcast::Sender<Message>,
    authorizer: Arc<dyn ChannelAuthorizer>,
    keep_alive: Duration,
}

impl fmt::Debug for Broadcaster {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Broadcaster")
            .field("subscribers", &self.sender.receiver_count())
            .finish()
    }
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new(1024)
    }
}

fn identity(channel: &LiveChannel) -> (String, String) {
    (
        channel.channel_type().as_str().to_owned(),
        channel.name().to_owned(),
    )
}

/// Query parameters accepted by [`Broadcaster::sse`].
#[derive(Debug, Deserialize)]
pub struct SubscribeQuery {
    /// Channel name, unprefixed.
    pub channel: String,
    /// Channel type: `public`, `private`, `presence`, or
    /// `encrypted-private`.
    #[serde(rename = "type", default = "default_type")]
    pub channel_type: String,
    /// The client's socket id, used to skip events it caused.
    #[serde(default)]
    pub socket_id: Option<String>,
}

fn default_type() -> String {
    "public".to_owned()
}

impl Broadcaster {
    /// Creates a broadcaster buffering up to `capacity` events per slow
    /// subscriber. Subscribers that fall further behind skip the missed
    /// events; the client then refreshes on reconnect.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self {
            sender,
            authorizer: Arc::new(PublicOnly),
            keep_alive: Duration::from_secs(15),
        }
    }

    /// Sets the authorizer for private, presence, and encrypted channels.
    pub fn authorize_with<A: ChannelAuthorizer + 'static>(mut self, authorizer: A) -> Self {
        self.authorizer = Arc::new(authorizer);
        self
    }

    /// Sets the SSE keep-alive interval (default 15 seconds).
    pub fn keep_alive(mut self, interval: Duration) -> Self {
        self.keep_alive = interval;
        self
    }

    /// Broadcasts `event` on `channel` to every subscriber. Returns the number
    /// of active subscriptions in this process.
    pub fn broadcast(&self, channel: &LiveChannel, event: &str, payload: Value) -> usize {
        self.publish(channel, event, payload, None)
    }

    /// Broadcasts to every subscriber except the connection identified by
    /// `socket_id`, usually [`RequestContext::socket_id`](crate::RequestContext::socket_id)
    /// of the request that caused the change.
    pub fn broadcast_to_others(
        &self,
        channel: &LiveChannel,
        event: &str,
        payload: Value,
        socket_id: Option<&str>,
    ) -> usize {
        self.publish(channel, event, payload, socket_id.map(ToOwned::to_owned))
    }

    /// Broadcasts prop values with the `__inertia` envelope, so listening
    /// clients update without a request.
    pub fn broadcast_props(
        &self,
        channel: &LiveChannel,
        event: &str,
        props: BroadcastPayload,
        payload: Value,
    ) -> usize {
        self.broadcast(channel, event, props.into_payload(payload))
    }

    fn publish(
        &self,
        channel: &LiveChannel,
        event: &str,
        payload: Value,
        except_socket: Option<String>,
    ) -> usize {
        self.sender
            .send(Message {
                channel: identity(channel),
                event: event.to_owned(),
                payload: Arc::new(payload),
                except_socket,
            })
            .unwrap_or(0)
    }

    /// Serves an SSE stream for one channel.
    ///
    /// Mount it on a `GET` route and pass the request and query:
    ///
    /// ```rust,no_run
    /// use axum::{extract::{Query, State}, routing::get, Router};
    /// use inertia_rs::axum::{live::{Broadcaster, SubscribeQuery}, InertiaRequest};
    ///
    /// async fn live(
    ///     State(broadcaster): State<Broadcaster>,
    ///     request: InertiaRequest,
    ///     query: Query<SubscribeQuery>,
    /// ) -> axum::response::Response {
    ///     broadcaster.sse(request, query).await
    /// }
    ///
    /// let app: Router = Router::new()
    ///     .route("/inertia/live", get(live))
    ///     .with_state(Broadcaster::default());
    /// ```
    ///
    /// Non-public channels return `403 Forbidden` unless the authorizer
    /// approves them. Each event is sent with the broadcast event name as the
    /// SSE event type and the payload as JSON data.
    pub async fn sse(
        &self,
        request: InertiaRequest,
        Query(query): Query<SubscribeQuery>,
    ) -> Response {
        let channel_type = match query.channel_type.as_str() {
            "public" => LiveChannelType::Public,
            "private" => LiveChannelType::Private,
            "presence" => LiveChannelType::Presence,
            "encrypted-private" => LiveChannelType::EncryptedPrivate,
            other => LiveChannelType::Other(other.to_owned()),
        };
        let channel = LiveChannel::new(query.channel, channel_type);

        if channel.channel_type() != &LiveChannelType::Public
            && !self.authorizer.authorize(&request, &channel).await
        {
            tracing::debug!(channel = channel.name(), "live channel subscription denied");
            return StatusCode::FORBIDDEN.into_response();
        }

        Sse::new(self.stream(identity(&channel), query.socket_id))
            .keep_alive(KeepAlive::new().interval(self.keep_alive))
            .into_response()
    }

    fn stream(
        &self,
        channel: (String, String),
        socket_id: Option<String>,
    ) -> impl Stream<Item = Result<Event, Infallible>> + Send + 'static {
        let receiver = self.sender.subscribe();
        let ready =
            stream::once(async { Ok(Event::default().event("inertia:subscribed").data("{}")) });
        let events = stream::unfold(receiver, |mut receiver| async move {
            match receiver.recv().await {
                Ok(message) => Some((Some(message), receiver)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "live subscriber lagged; events were skipped");
                    Some((None, receiver))
                }
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .filter_map(move |message| {
            let channel = channel.clone();
            let socket_id = socket_id.clone();
            async move {
                let message = message?;
                if message.channel != channel {
                    return None;
                }
                if message.except_socket.is_some() && message.except_socket == socket_id {
                    return None;
                }
                let data = serde_json::to_string(message.payload.as_ref()).ok()?;
                Some(Ok(Event::default().event(message.event).data(data)))
            }
        });

        ready.chain(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::axum::InertiaLayer;
    use ::axum::body::Body;
    use ::axum::extract::State;
    use ::axum::http::Request;
    use ::axum::routing::get;
    use ::axum::Router;
    use serde_json::json;
    use tower::ServiceExt;

    async fn next_text(body: &mut ::axum::body::BodyDataStream) -> String {
        let chunk = tokio::time::timeout(Duration::from_secs(2), body.next())
            .await
            .expect("an SSE chunk arrives")
            .expect("stream is open")
            .expect("chunk is readable");
        String::from_utf8(chunk.to_vec()).unwrap()
    }

    #[derive(Clone)]
    struct Admin;

    async fn live(
        State(broadcaster): State<Broadcaster>,
        request: InertiaRequest,
        query: Query<SubscribeQuery>,
    ) -> Response {
        broadcaster.sse(request, query).await
    }

    fn app(broadcaster: Broadcaster) -> Router {
        Router::new()
            .route("/live", get(live))
            .with_state(broadcaster)
            .layer(InertiaLayer::unversioned())
    }

    #[tokio::test]
    async fn streams_events_for_the_subscribed_channel_only() {
        let broadcaster = Broadcaster::default();
        let response = app(broadcaster.clone())
            .oneshot(
                Request::get("/live?channel=stats&socket_id=me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        assert!(next_text(&mut body)
            .await
            .contains("event: inertia:subscribed"));

        broadcaster.broadcast(&LiveChannel::public("other"), "Ignored", json!({}));
        broadcaster.broadcast_to_others(
            &LiveChannel::public("stats"),
            "Skipped",
            json!({}),
            Some("me"),
        );
        broadcaster.broadcast_props(
            &LiveChannel::public("stats"),
            "StatsUpdated",
            BroadcastPayload::new().prop("count", 3).unwrap(),
            json!({ "id": 1 }),
        );

        let text = next_text(&mut body).await;
        assert!(text.contains("event: StatsUpdated"), "{text}");
        assert!(
            text.contains(r#""__inertia":{"props":{"count":3}}"#),
            "{text}"
        );
    }

    #[tokio::test]
    async fn private_channels_require_authorization() {
        let denied = app(Broadcaster::default())
            .oneshot(
                Request::get("/live?channel=orders.1&type=private")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let broadcaster = Broadcaster::default().authorize_with(
            |request: &InertiaRequest, channel: &LiveChannel| {
                request.extension::<Admin>().is_some() && channel.name().starts_with("orders.")
            },
        );
        let allowed = app(broadcaster)
            .layer(::axum::Extension(Admin))
            .oneshot(
                Request::get("/live?channel=orders.1&type=private")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);
    }
}
