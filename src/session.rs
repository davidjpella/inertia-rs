//! Framework-neutral session state for validation errors, flash data, and
//! redirect-carried page flags.
//!
//! Inertia carries several values across a redirect: validation errors
//! (`errors`), flash data (`flash`), and the `clearHistory` and
//! `preserveFragment` flags. The reference adapter stores them in the session.
//! This module models that lifecycle without depending on a session library:
//!
//! 1. At the start of a request, the integration loads [`SessionData`] from
//!    the application's session store and creates a [`SessionState`].
//! 2. Handlers add *pending* data ([`SessionState::flash`],
//!    [`SessionState::errors`], ...) before redirecting.
//! 3. Rendering a page [consumes](SessionState::consume) incoming and pending
//!    data, so it is delivered exactly once.
//! 4. At the end of the request, [`SessionState::finish`] says what to store:
//!    redirects and version-mismatch responses keep unconsumed data for the
//!    next request (the reference adapter's *reflash*), while a rendered page
//!    clears it.
//!
//! Data is stored in the requesting user's own session, so it never leaks to
//! other users. Values are only delivered to the next page render of the same
//! session.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The default validation error bag.
pub const DEFAULT_ERROR_BAG: &str = "default";

/// Session key used by framework integrations.
pub const SESSION_KEY: &str = "inertia";

/// Validation errors grouped by error bag and field.
///
/// ```rust
/// use inertia_rs::session::ValidationErrors;
///
/// let errors = ValidationErrors::new()
///     .add("email", "The email field is required.")
///     .add("email", "The email must be valid.");
///
/// assert_eq!(
///     errors.to_prop(None, false),
///     serde_json::json!({ "email": "The email field is required." })
/// );
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ValidationErrors {
    bags: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

impl ValidationErrors {
    /// Creates an empty error set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a message for `field` in the default bag.
    pub fn add<F: Into<String>, M: Into<String>>(self, field: F, message: M) -> Self {
        self.add_to_bag(DEFAULT_ERROR_BAG, field, message)
    }

    /// Adds a message for `field` in `bag`.
    pub fn add_to_bag<B, F, M>(mut self, bag: B, field: F, message: M) -> Self
    where
        B: Into<String>,
        F: Into<String>,
        M: Into<String>,
    {
        self.push(bag, field, message);
        self
    }

    /// Adds a message through a mutable reference.
    pub fn push<B, F, M>(&mut self, bag: B, field: F, message: M)
    where
        B: Into<String>,
        F: Into<String>,
        M: Into<String>,
    {
        self.bags
            .entry(bag.into())
            .or_default()
            .entry(field.into())
            .or_default()
            .push(message.into());
    }

    /// Moves the default bag into `bag`.
    pub fn into_bag<B: Into<String>>(mut self, bag: B) -> Self {
        let bag = bag.into();

        if bag != DEFAULT_ERROR_BAG {
            if let Some(default) = self.bags.remove(DEFAULT_ERROR_BAG) {
                let target = self.bags.entry(bag).or_default();
                for (field, messages) in default {
                    target.entry(field).or_default().extend(messages);
                }
            }
        }

        self
    }

    /// Merges another error set into this one.
    pub fn extend(&mut self, other: ValidationErrors) {
        for (bag, fields) in other.bags {
            let target = self.bags.entry(bag).or_default();
            for (field, messages) in fields {
                target.entry(field).or_default().extend(messages);
            }
        }
    }

    /// Returns `true` when no messages have been added.
    pub fn is_empty(&self) -> bool {
        self.bags.values().all(BTreeMap::is_empty)
    }

    /// Returns the messages per field for `bag`.
    pub fn bag(&self, bag: &str) -> Option<&BTreeMap<String, Vec<String>>> {
        self.bags.get(bag)
    }

    /// Builds the `errors` prop.
    ///
    /// Follows the reference adapter's error-bag rules:
    ///
    /// - With an `X-Inertia-Error-Bag` header, the default bag is sent under
    ///   the requested name.
    /// - Without it, the default bag is sent unwrapped.
    /// - Named bags are sent under their own names only when no default bag
    ///   exists.
    ///
    /// Each field holds its first message, or every message when
    /// `all_messages` is true.
    pub fn to_prop(&self, error_bag: Option<&str>, all_messages: bool) -> Value {
        let render_bag = |fields: &BTreeMap<String, Vec<String>>| {
            Value::Object(
                fields
                    .iter()
                    .filter_map(|(field, messages)| {
                        let value = if all_messages {
                            Value::from(messages.clone())
                        } else {
                            Value::from(messages.first()?.clone())
                        };
                        Some((field.clone(), value))
                    })
                    .collect(),
            )
        };

        if let Some(default) = self.bags.get(DEFAULT_ERROR_BAG) {
            let rendered = render_bag(default);

            return match error_bag {
                Some(bag) if !bag.is_empty() => {
                    let mut wrapped = Map::new();
                    wrapped.insert(bag.to_owned(), rendered);
                    Value::Object(wrapped)
                }
                _ => rendered,
            };
        }

        Value::Object(
            self.bags
                .iter()
                .map(|(bag, fields)| (bag.clone(), render_bag(fields)))
                .collect(),
        )
    }
}

/// Values carried in the session between requests.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionData {
    /// Validation errors.
    #[serde(default, skip_serializing_if = "ValidationErrors::is_empty")]
    pub errors: ValidationErrors,
    /// Flash data.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub flash: Map<String, Value>,
    /// Sets `clearHistory` on the next page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_history: bool,
    /// Sets `preserveFragment` on the next page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preserve_fragment: bool,
    /// Sets the experimental `interstitial` flag on the next page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interstitial: bool,
}

impl SessionData {
    /// Returns `true` when nothing needs to be carried.
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
            && self.flash.is_empty()
            && !self.clear_history
            && !self.preserve_fragment
            && !self.interstitial
    }

    /// Merges `newer` into `self`. Newer flash keys win.
    pub fn merge(&mut self, newer: SessionData) {
        self.errors.extend(newer.errors);
        self.flash.extend(newer.flash);
        self.clear_history |= newer.clear_history;
        self.preserve_fragment |= newer.preserve_fragment;
        self.interstitial |= newer.interstitial;
    }
}

/// What to do with the session store at the end of a request.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionWrite {
    /// Nothing changed; skip the write.
    Unchanged,
    /// Store this data.
    Store(SessionData),
    /// Remove the stored data.
    Clear,
}

/// How a response ended, as far as session carry-over is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseKind {
    /// A `3xx` redirect, or a `409` asset-version mismatch, external
    /// location, or fragment redirect. Unconsumed data is kept for the next
    /// request.
    Redirect,
    /// Any other response.
    Other,
}

/// Per-request session state.
#[derive(Clone, Debug, Default)]
pub struct SessionState {
    incoming: SessionData,
    pending: SessionData,
    consumed: bool,
    carry_over: bool,
}

impl SessionState {
    /// Starts a request with data loaded from the session.
    pub fn new(incoming: Option<SessionData>) -> Self {
        Self {
            incoming: incoming.unwrap_or_default(),
            pending: SessionData::default(),
            consumed: false,
            carry_over: false,
        }
    }

    /// Keeps incoming and pending data for the next request even though this
    /// response is not a redirect.
    ///
    /// An experimental layer close response uses this: the client refreshes
    /// the page beneath, and that refresh consumes the flash data.
    pub fn carry_over(&mut self) {
        self.carry_over = true;
    }

    /// Flashes a value for the next page render.
    pub fn flash<K: Into<String>, V: Serialize>(
        &mut self,
        key: K,
        value: V,
    ) -> Result<(), serde_json::Error> {
        self.pending
            .flash
            .insert(key.into(), serde_json::to_value(value)?);
        Ok(())
    }

    /// Adds validation errors for the next page render.
    pub fn errors(&mut self, errors: ValidationErrors) {
        self.pending.errors.extend(errors);
    }

    /// Sets `clearHistory` on the next page render.
    pub fn clear_history(&mut self) {
        self.pending.clear_history = true;
    }

    /// Sets `preserveFragment` on the next page render.
    pub fn preserve_fragment(&mut self) {
        self.pending.preserve_fragment = true;
    }

    /// Sets the experimental `interstitial` flag on the next page render.
    pub fn interstitial(&mut self) {
        self.pending.interstitial = true;
    }

    /// Clears a pending interstitial mark, as an experimental layer close
    /// response does.
    pub fn clear_interstitial(&mut self) {
        self.incoming.interstitial = false;
        self.pending.interstitial = false;
    }

    /// Returns the data a page render would consume, without consuming it.
    pub fn peek(&self) -> SessionData {
        let mut data = self.incoming.clone();
        data.merge(self.pending.clone());
        data
    }

    /// Consumes incoming and pending data for a page render.
    pub fn consume(&mut self) -> SessionData {
        let mut data = std::mem::take(&mut self.incoming);
        data.merge(std::mem::take(&mut self.pending));
        self.consumed = true;
        data
    }

    /// Returns what to write to the session store.
    ///
    /// `had_stored_data` tells whether the session held data at the start of
    /// the request, so consumed data can be cleared.
    pub fn finish(self, response: ResponseKind, had_stored_data: bool) -> SessionWrite {
        let response = if self.carry_over && !self.consumed {
            ResponseKind::Redirect
        } else {
            response
        };
        let mut carry = match response {
            // Reflash: keep everything that has not been delivered yet.
            ResponseKind::Redirect => {
                let mut carry = self.incoming;
                carry.merge(self.pending);
                carry
            }
            // Data that no page has consumed survives unrelated responses,
            // such as asset or API requests, but pending data from a handler
            // that did not redirect is dropped.
            ResponseKind::Other if !self.consumed => self.incoming,
            ResponseKind::Other => SessionData::default(),
        };

        if !matches!(response, ResponseKind::Redirect) && self.consumed {
            carry = SessionData::default();
        }

        match (carry.is_empty(), had_stored_data) {
            (true, true) => SessionWrite::Clear,
            (true, false) => SessionWrite::Unchanged,
            (false, _) => SessionWrite::Store(carry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data_with_flash() -> SessionData {
        let mut data = SessionData::default();
        data.flash.insert("message".into(), json!("Saved"));
        data
    }

    #[test]
    fn error_bag_rules_match_reference_adapter() {
        let errors = ValidationErrors::new()
            .add("name", "Required")
            .add("name", "Too short");
        assert_eq!(errors.to_prop(None, false), json!({ "name": "Required" }));
        assert_eq!(
            errors.to_prop(Some("createUser"), false),
            json!({ "createUser": { "name": "Required" } })
        );
        assert_eq!(
            errors.to_prop(None, true),
            json!({ "name": ["Required", "Too short"] })
        );

        let named = ValidationErrors::new().add_to_bag("login", "email", "Invalid");
        assert_eq!(
            named.to_prop(None, false),
            json!({ "login": { "email": "Invalid" } })
        );
        assert_eq!(
            named.to_prop(Some("other"), false),
            json!({ "login": { "email": "Invalid" } })
        );
        assert_eq!(ValidationErrors::new().to_prop(None, false), json!({}));
    }

    #[test]
    fn redirect_reflashes_and_render_consumes() {
        let mut first = SessionState::new(None);
        first.flash("message", "Saved").unwrap();
        let SessionWrite::Store(stored) = first.finish(ResponseKind::Redirect, false) else {
            panic!("redirect stores pending flash");
        };

        // A version-mismatch 409 keeps the data.
        let conflict = SessionState::new(Some(stored.clone()));
        assert_eq!(
            conflict.finish(ResponseKind::Redirect, true),
            SessionWrite::Store(stored.clone())
        );

        // An unrelated response keeps unconsumed data.
        let asset = SessionState::new(Some(stored.clone()));
        assert_eq!(
            asset.finish(ResponseKind::Other, true),
            SessionWrite::Store(stored.clone())
        );

        // A page render consumes it exactly once.
        let mut render = SessionState::new(Some(stored));
        assert_eq!(render.consume(), data_with_flash());
        assert_eq!(
            render.finish(ResponseKind::Other, true),
            SessionWrite::Clear
        );
    }

    #[test]
    fn pending_data_without_redirect_is_not_stored() {
        let mut state = SessionState::new(None);
        state.flash("message", "Lost").unwrap();
        assert_eq!(
            state.finish(ResponseKind::Other, false),
            SessionWrite::Unchanged
        );
    }

    #[test]
    fn session_data_round_trips_through_json() {
        let mut data = data_with_flash();
        data.errors = ValidationErrors::new().add("email", "Invalid");
        data.clear_history = true;
        let json = serde_json::to_value(&data).unwrap();

        assert_eq!(
            json,
            json!({
                "errors": { "default": { "email": ["Invalid"] } },
                "flash": { "message": "Saved" },
                "clearHistory": true
            })
        );
        assert_eq!(serde_json::from_value::<SessionData>(json).unwrap(), data);
    }
}
