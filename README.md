# Inertia.rs

[![Current Crates.io Version](https://img.shields.io/crates/v/inertia_rs)](https://crates.io/crates/inertia_rs)
[![Build Status](https://github.com/stuarth/inertia-rs/workflows/CI/badge.svg)](https://github.com/stuarth/inertia-rs/actions)
[![docs.rs](https://img.shields.io/badge/docs-latest-blue.svg?style=flat)](https://docs.rs/inertia_rs/)

A Rust server adapter for [Inertia.js](https://inertiajs.com/), with
integrations for **Axum** and **Rocket** on top of a framework-neutral
protocol core.

Inertia lets you build server-driven applications that render client-side
pages without a separate API or client-side router. Your Rust routes return
page components and props; the official React, Vue, or Svelte client handles
navigation, forms, and page swaps.

## Compatibility

| | Version |
| --- | --- |
| Inertia client (`@inertiajs/react`, `vue3`, `svelte`, `vite`) | **3.11.0** |
| Reference server adapter | `inertia-laravel` 3.7.0 |
| Axum | 0.8.9+ |
| Rocket | 0.5.1+ |
| Rust (MSRV) | 1.88 |

Every server-side requirement of the 3.11 protocol is implemented for Axum and
verified by Rust tests and Playwright browser tests against the official React
(full suite), Vue, and Svelte packages. Rocket implements the protocol except
session-backed flash/validation storage and the `QUERY` method. The
feature-by-feature evidence is in
[`docs/protocol-support.md`](docs/protocol-support.md); pinned versions are in
[`docs/compatibility-target.md`](docs/compatibility-target.md).

**Layers** ([inertiajs/inertia#3236](https://github.com/inertiajs/inertia/pull/3236))
are unreleased upstream. They are available behind the `experimental-layers`
feature, tested against a pinned client commit, and are **not** part of the
stable compatibility claim.

## Installation

```toml
[dependencies]
# Axum, with tower-sessions storage for validation errors and flash data,
# and the SSR client:
inertia_rs = { version = "0.5", default-features = false, features = ["axum", "tower-sessions", "ssr"] }

# Rocket (default feature):
# inertia_rs = "0.5"
```

| Feature | Provides |
| --- | --- |
| `rocket` (default) | Rocket responders, `InertiaRequest` guard, `VersionFairing` |
| `axum` | `InertiaLayer`, `InertiaRequest` extractor, SSE live-prop broadcaster |
| `tower-sessions` | `TowerSessions` session backend for Axum |
| `ssr` | `HttpSsrGateway` for the official SSR server and Vite dev endpoint |
| `experimental-layers` | Unreleased Layers protocol |

## Axum quick start

```rust
use axum::{response::{Html, Response}, routing::get, Router};
use inertia_rs::axum::{InertiaError, InertiaLayer, InertiaRequest, SharedProps, TowerSessions};
use inertia_rs::{Inertia, Prop, Props};
use tower_sessions::{MemoryStore, SessionManagerLayer};

async fn dashboard(request: InertiaRequest) -> Result<Response, InertiaError> {
    request
        .respond(Inertia::response(
            "Dashboard",
            Props::new()
                .value("user", "Ada")
                .lazy_async("projects", || async { load_projects().await })
                .prop("stats", Prop::lazy_async(|| async { 42 }).defer().rescue()),
        ))
        .await
}

# async fn load_projects() -> Vec<String> { vec![] }
let app: Router = Router::new()
    .route("/", get(dashboard))
    .layer(
        InertiaLayer::new("asset-version")
            .template(|page| Html(format!(
                "<!doctype html><html><head>{}<script type=module src=/build/app.js></script></head><body>{}</body></html>",
                page.head(),
                page.body(),
            )))
            .shared(SharedProps::new().value("appName", "My App"))
            .session(TowerSessions::new()),
    )
    .layer(SessionManagerLayer::new(MemoryStore::default()));
```

`InertiaLayer` is the Inertia middleware: asset-version conflicts, `Vary`,
`303` after writes, fragment redirects, empty-response redirects, and
session carry-over. `page.body()` renders the page-object script and root
element (or the SSR markup); `page.head()` renders SSR head tags.

### Props

Plain serializable values work as props. `Props` and `Prop` add behaviour;
providers only run when the request needs the prop, and async providers needed
by one response are polled concurrently.

```rust
use inertia_rs::{LiveChannel, LiveUpdates, Prop, Props, ScrollProps};
# let (posts, has_more) = (serde_json::json!({ "data": [] }), false);

let props = Props::new()
    .value("auth.user", "Ada")                                 // dot notation
    .prop("plans", Prop::lazy(|| vec!["basic"]).once())        // once
    .prop("audit", Prop::lazy(|| vec!["login"]).optional())    // optional
    .prop("comments", Prop::lazy_async(|| async { 3 }).defer_group("sidebar"))
    .prop("feed", Prop::value(vec![1, 2]).merge().match_on("id"))
    .prop("posts", Prop::value(posts).scroll(ScrollProps::paginated("page", 1, has_more)))
    .prop("order", Prop::lazy(|| "shipped")
        .live(LiveUpdates::on("OrderUpdated").channel(LiveChannel::private("orders.1"))));
```

| Need | API |
| --- | --- |
| Fallible provider | `Prop::try_lazy`, `Prop::try_lazy_async` (`Err` fails the response unless `.rescue()`) |
| Borrow handler data | `Props<'a>` providers may borrow; render before returning |
| Limit / time out providers | `InertiaLayer::prop_concurrency(n)`, `prop_timeout(duration)` |
| Report rescued failures | `ResolveOptions::on_rescued_error` |
| Big integers | `.preserve_big_integers()` on the response or layer |

Prop values are never logged; provider errors are logged at `error` level
unless the prop is `.sensitive()`. Dropping the response future (for example
when the client disconnects) cancels every pending provider; no provider is
spawned onto another task.

### Validation errors and flash

```rust
# use axum::{response::Response, Json};
# use inertia_rs::axum::{InertiaError, InertiaRequest};
# use inertia_rs::session::ValidationErrors;
# use inertia_rs::Inertia;
# #[derive(serde::Deserialize)] struct NewUser { name: String }
async fn store(request: InertiaRequest, Json(user): Json<NewUser>) -> Result<Response, InertiaError> {
    if user.name.is_empty() {
        request.with_errors(ValidationErrors::new().add("name", "The name field is required."));
        return request.back();
    }

    request.flash("message", "User created.")?;
    request.redirect(Inertia::redirect("/users"))
}
```

Errors honour `X-Inertia-Error-Bag`. Data is stored in the requesting user's
session, kept across redirects and version conflicts, and delivered once.
Implement `SessionBackend` to use a store other than `tower-sessions`.

### Server-side rendering

```rust
use inertia_rs::ssr::{HttpSsrGateway, Ssr};
# use inertia_rs::axum::InertiaLayer;

let gateway = HttpSsrGateway::new("http://127.0.0.1:13714")?;  // production
// let gateway = HttpSsrGateway::vite_dev("http://localhost:5173")?; // development
let layer = InertiaLayer::new("v1").ssr(Ssr::new(gateway));
# Ok::<(), inertia_rs::ssr::SsrError>(())
```

Build the SSR bundle with `@inertiajs/vite` (`vite build --ssr`) and run it
with Node. Failed renders fall back to client rendering by default;
`SsrFailurePolicy::Error` fails loudly.

### Live props

`liveProps` metadata works with any official transport (`echo()`, `pusher()`,
`actionCable()`). For apps without a broadcast server,
`inertia_rs::axum::live::Broadcaster` streams events over Server-Sent Events
with channel authorization; the matching ~70-line `LiveTransport` is in
[`examples/axum-react/frontend/src/liveTransport.js`](examples/axum-react/frontend/src/liveTransport.js).
Use `BroadcastPayload` to send prop values in the `__inertia` envelope through
any broadcaster.

## Rocket

```rust,ignore
#[get("/hello")]
fn hello() -> Inertia<serde_json::Value> {
    Inertia::response("Hello", serde_json::json!({ "name": "world" }))
}

#[get("/dashboard")]
async fn dashboard(inertia: InertiaRequest) -> Result<InertiaPage, InertiaError> {
    inertia.render(Inertia::response("Dashboard",
        Props::new().defer_async("stats", || async { 42 }))).await
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes![hello, dashboard])
        .manage(SharedProps::new().value("appName", "My App"))
        .attach(VersionFairing::new("asset-version", |request, page| {
            Template::render("app", page).respond_to(request)
        }))
}
```

`Inertia<T>` responders support synchronous props; use the `InertiaRequest`
guard for async providers and SSR (`rocket.manage(Ssr::new(...))`). Templates
receive `data_page` (script-safe JSON) and `root_id`.

## Examples

| Example | Shows |
| --- | --- |
| [`examples/axum-react`](examples/axum-react) | Full Axum + React + Vite app: every protocol feature, SSR (dev and production), live props, experimental layers |
| [`examples/axum-vue`](examples/axum-vue), [`examples/axum-svelte`](examples/axum-svelte) | Vue and Svelte smoke apps |
| [`examples/rocket-svelte`](examples/rocket-svelte), [`examples/rocket-minimal`](examples/rocket-minimal), [`examples/axum-minimal`](examples/axum-minimal) | Smaller integrations |

Browser tests live in [`e2e`](e2e) (`npx playwright test`).

## Upgrading from 0.4

See [`docs/migration-0.5.md`](docs/migration-0.5.md).

## License

MIT
