//! Experimental Layers helpers (inertiajs/inertia#3236).
//!
//! The contract is pinned to client commit
//! `e57a7774430c095ab8616a5ac2b539bd324c26ca` and server PR
//! inertiajs/inertia-laravel#903 (`e819bc99db2f`). It can change before
//! upstream releases it.

/// Makes `url` host-relative when it is an absolute URL on `host`, like the
/// reference adapter does for a layer's `base`.
///
/// Returns `None` when `url` is already relative or points to another host.
pub fn to_relative_url(url: &str, host: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };

    if !authority.eq_ignore_ascii_case(host) {
        return None;
    }

    Some(if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_same_host_origins_only() {
        assert_eq!(
            to_relative_url("https://app.test/users?x=1", "app.test").as_deref(),
            Some("/users?x=1")
        );
        assert_eq!(
            to_relative_url("http://app.test", "app.test").as_deref(),
            Some("/")
        );
        assert_eq!(
            to_relative_url("http://app.test?x", "app.test").as_deref(),
            Some("/?x")
        );
        assert_eq!(
            to_relative_url("https://other.test/users", "app.test"),
            None
        );
        assert_eq!(to_relative_url("/users", "app.test"), None);
    }
}
