//! Session backends for validation errors, flash data, and history flags.

use crate::session::{SessionData, SessionWrite};
use ::axum::http::Extensions;
use futures_util::future::BoxFuture;

/// Error type returned by session backends.
pub type SessionError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Access to one request's session.
pub trait RequestSession: Send + Sync {
    /// Loads Inertia data stored by an earlier request.
    fn load(&self) -> BoxFuture<'_, Result<Option<SessionData>, SessionError>>;

    /// Stores or clears Inertia data for the next request.
    fn store(&self, write: SessionWrite) -> BoxFuture<'_, Result<(), SessionError>>;
}

/// Connects [`InertiaLayer`](super::InertiaLayer) to a session store.
///
/// The backend is asked for a [`RequestSession`] per request, usually by
/// reading a session handle that an outer session middleware inserted into
/// the request extensions. Returning `None` disables carrying data for that
/// request.
///
/// The protocol core does not depend on a session library. Enable the
/// `tower-sessions` feature for [`TowerSessions`](super::TowerSessions), or
/// implement this trait for your own store.
pub trait SessionBackend: Send + Sync {
    /// Returns the session for a request.
    fn for_request(&self, extensions: &Extensions) -> Option<Box<dyn RequestSession>>;
}

impl<F> SessionBackend for F
where
    F: Fn(&Extensions) -> Option<Box<dyn RequestSession>> + Send + Sync,
{
    fn for_request(&self, extensions: &Extensions) -> Option<Box<dyn RequestSession>> {
        self(extensions)
    }
}

/// [`SessionBackend`] for [`tower_sessions`].
///
/// Install `tower_sessions::SessionManagerLayer` *outside* the
/// [`InertiaLayer`](super::InertiaLayer) so the session is available when the
/// Inertia middleware runs and is saved after it:
///
/// ```rust,no_run
/// use axum::Router;
/// use inertia_rs::axum::{InertiaLayer, TowerSessions};
/// use tower_sessions::{MemoryStore, SessionManagerLayer};
///
/// let app: Router = Router::new()
///     .layer(InertiaLayer::new("v1").session(TowerSessions::new()))
///     .layer(SessionManagerLayer::new(MemoryStore::default()));
/// ```
#[cfg(feature = "tower-sessions")]
#[derive(Clone, Debug)]
pub struct TowerSessions {
    key: String,
}

#[cfg(feature = "tower-sessions")]
impl TowerSessions {
    /// Stores Inertia data under the `inertia` session key.
    pub fn new() -> Self {
        Self::with_key(crate::session::SESSION_KEY)
    }

    /// Stores Inertia data under `key`.
    pub fn with_key<K: Into<String>>(key: K) -> Self {
        Self { key: key.into() }
    }
}

#[cfg(feature = "tower-sessions")]
impl Default for TowerSessions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "tower-sessions")]
struct TowerRequestSession {
    session: tower_sessions::Session,
    key: String,
}

#[cfg(feature = "tower-sessions")]
impl RequestSession for TowerRequestSession {
    fn load(&self) -> BoxFuture<'_, Result<Option<SessionData>, SessionError>> {
        Box::pin(async move {
            self.session
                .get::<SessionData>(&self.key)
                .await
                .map_err(|error| Box::new(error) as SessionError)
        })
    }

    fn store(&self, write: SessionWrite) -> BoxFuture<'_, Result<(), SessionError>> {
        Box::pin(async move {
            match write {
                SessionWrite::Unchanged => Ok(()),
                SessionWrite::Store(data) => self
                    .session
                    .insert(&self.key, data)
                    .await
                    .map_err(|error| Box::new(error) as SessionError),
                SessionWrite::Clear => self
                    .session
                    .remove::<SessionData>(&self.key)
                    .await
                    .map(|_| ())
                    .map_err(|error| Box::new(error) as SessionError),
            }
        })
    }
}

#[cfg(feature = "tower-sessions")]
impl SessionBackend for TowerSessions {
    fn for_request(&self, extensions: &Extensions) -> Option<Box<dyn RequestSession>> {
        let session = extensions.get::<tower_sessions::Session>()?.clone();
        Some(Box::new(TowerRequestSession {
            session,
            key: self.key.clone(),
        }))
    }
}
