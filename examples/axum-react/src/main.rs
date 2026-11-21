//! Axum + React + Vite example for `inertia_rs`.
//!
//! See README.md for development, production, and SSR commands.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use inertia_rs::axum::live::{Broadcaster, SubscribeQuery};
use inertia_rs::axum::{
    error_page, InertiaError, InertiaLayer, InertiaRequest, SharedProps, TowerSessions,
};
use inertia_rs::session::ValidationErrors;
use inertia_rs::ssr::{HttpSsrGateway, Ssr};
use inertia_rs::{
    BroadcastPayload, HtmlResponseContext, Inertia, LiveChannel, LiveUpdates, Prop, Props,
    ScrollProps,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer};

const PAGE_SIZE: usize = 15;

#[derive(Clone, Debug, Serialize)]
struct User {
    id: u64,
    name: String,
    email: String,
}

#[derive(Clone)]
struct AppState {
    users: Arc<RwLock<Vec<User>>>,
    orders: Arc<AtomicU64>,
    broadcaster: Broadcaster,
    test_version: Arc<AtomicU64>,
}

impl AppState {
    fn new() -> Self {
        let users = (1..=45)
            .map(|id| User {
                id,
                name: format!("User {id}"),
                email: format!("user{id}@example.com"),
            })
            .collect();

        Self {
            users: Arc::new(RwLock::new(users)),
            orders: Arc::new(AtomicU64::new(0)),
            broadcaster: Broadcaster::default(),
            test_version: Arc::new(AtomicU64::new(0)),
        }
    }
}

fn orders_channel() -> LiveChannel {
    LiveChannel::public("orders")
}

/// Frontend assets: either the Vite dev server or a production build.
#[derive(Clone, Debug)]
enum Assets {
    Dev {
        origin: String,
    },
    Build {
        script: String,
        styles: Vec<String>,
        version: String,
    },
}

#[derive(Deserialize)]
struct ManifestEntry {
    file: String,
    #[serde(default)]
    css: Vec<String>,
}

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_assets() -> Assets {
    if let Ok(origin) = std::env::var("VITE_DEV_SERVER") {
        return Assets::Dev {
            origin: origin.trim_end_matches('/').to_owned(),
        };
    }

    let manifest_path = example_dir().join("public/build/.vite/manifest.json");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap_or_else(|_| {
        panic!(
            "missing {}; run `npm install && npm run build` in examples/axum-react/frontend, \
             or set VITE_DEV_SERVER=http://localhost:5173",
            manifest_path.display()
        )
    });
    let mut hasher = DefaultHasher::new();
    manifest.hash(&mut hasher);
    let entries: BTreeMap<String, ManifestEntry> =
        serde_json::from_str(&manifest).expect("Vite manifest is valid JSON");
    let entry = entries
        .get("src/app.jsx")
        .expect("Vite manifest contains src/app.jsx");

    Assets::Build {
        script: format!("/build/{}", entry.file),
        styles: entry
            .css
            .iter()
            .map(|css| format!("/build/{css}"))
            .collect(),
        version: format!("{:016x}", hasher.finish()),
    }
}

fn template(assets: Assets) -> impl Fn(HtmlResponseContext) -> Html<String> + Clone {
    move |page| {
        let tags = match &assets {
            Assets::Dev { origin } => format!(
                r#"<script type="module">
import RefreshRuntime from "{origin}/@react-refresh";
RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$ = () => {{}};
window.$RefreshSig$ = () => (type) => type;
window.__vite_plugin_react_preamble_installed__ = true;
</script>
<script type="module" src="{origin}/@vite/client"></script>
<script type="module" src="{origin}/src/app.jsx"></script>"#
            ),
            Assets::Build { script, styles, .. } => {
                let styles = styles
                    .iter()
                    .map(|href| format!(r#"<link rel="stylesheet" href="{href}" />"#))
                    .collect::<String>();
                format!(r#"{styles}<script type="module" src="{script}"></script>"#)
            }
        };
        let fallback_title = if page.is_server_rendered() {
            String::new()
        } else {
            "<title>Inertia Rust</title>".to_owned()
        };

        Html(format!(
            r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    {fallback_title}
    {head}
    {tags}
  </head>
  <body>
    {body}
  </body>
</html>"#,
            head = page.head(),
            body = page.body(),
        ))
    }
}

async fn dashboard(
    State(state): State<AppState>,
    request: InertiaRequest,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<Response, InertiaError> {
    let fail_report = query.contains_key("fail");
    let orders = state.orders.load(Ordering::SeqCst);

    request
        .respond(
            Inertia::response(
                "Dashboard",
                Props::new()
                    .value("greeting", "Hello from Axum")
                    .prop(
                        "stats",
                        Prop::lazy_async(|| async {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            json!({ "visits": 1234, "conversion": 3.2 })
                        })
                        .defer(),
                    )
                    .prop(
                        "report",
                        Prop::try_lazy_async(move || async move {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            if fail_report {
                                Err("report service unavailable")
                            } else {
                                Ok(json!({ "rows": 3 }))
                            }
                        })
                        .defer_group("reports")
                        .rescue(),
                    )
                    .optional("audit", || vec!["Signed in", "Viewed dashboard"])
                    .prop(
                        "ordersCount",
                        Prop::lazy(move || orders)
                            .live(LiveUpdates::on("OrderPlaced").channel(orders_channel())),
                    )
                    .prop(
                        "lastOrderAt",
                        Prop::lazy(|| {
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|elapsed| elapsed.as_millis() as u64)
                                .unwrap_or_default()
                        })
                        .live(LiveUpdates::on("OrderRefresh").channel(orders_channel())),
                    ),
            )
            .preserve_big_integers()
            .flash("bigId", 900_719_925_474_099_988u64)?,
        )
        .await
}

#[derive(Deserialize)]
struct OrderForm {
    #[serde(default)]
    mode: String,
}

/// Places an order and broadcasts it. `mode=reload` broadcasts an event the
/// clients answer with a partial reload; otherwise the new value travels in
/// the `__inertia.props` envelope.
async fn place_order(
    State(state): State<AppState>,
    request: InertiaRequest,
    Json(form): Json<OrderForm>,
) -> Result<Response, InertiaError> {
    let count = state.orders.fetch_add(1, Ordering::SeqCst) + 1;

    if form.mode == "reload" {
        state.broadcaster.broadcast_to_others(
            &orders_channel(),
            "OrderRefresh",
            json!({}),
            request.context().socket_id(),
        );
    } else {
        state.broadcaster.broadcast_props(
            &orders_channel(),
            "OrderPlaced",
            BroadcastPayload::new()
                .prop("ordersCount", count)
                .map_err(InertiaError::Serialization)?,
            json!({ "count": count }),
        );
    }

    request.flash("message", format!("Order #{count} placed"))?;
    request.back_or("/")
}

async fn live(
    State(state): State<AppState>,
    request: InertiaRequest,
    query: Query<SubscribeQuery>,
) -> Response {
    state.broadcaster.sse(request, query).await
}

#[derive(Deserialize)]
struct PageQuery {
    page: Option<usize>,
}

async fn users_index(
    State(state): State<AppState>,
    request: InertiaRequest,
    Query(query): Query<PageQuery>,
) -> Result<Response, InertiaError> {
    let page = query.page.unwrap_or(1).max(1);
    let users = state.users.read().expect("users lock").clone();
    let total = users.len();
    let data = users
        .into_iter()
        .rev()
        .skip((page - 1) * PAGE_SIZE)
        .take(PAGE_SIZE)
        .collect::<Vec<_>>();
    let has_more = page * PAGE_SIZE < total;

    request
        .respond(Inertia::response(
            "Users/Index",
            Props::new().value("total", total).prop(
                "users",
                Prop::value(json!({ "data": data }))
                    .scroll(ScrollProps::paginated("page", page as u64, has_more))
                    .match_on("data.id"),
            ),
        ))
        .await
}

async fn users_create(request: InertiaRequest) -> Result<Response, InertiaError> {
    request
        .respond(Inertia::response(
            "Users/Create",
            json!({ "roles": ["admin", "member"] }),
        ))
        .await
}

#[derive(Deserialize)]
struct UserForm {
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
}

async fn users_store(
    State(state): State<AppState>,
    request: InertiaRequest,
    Json(form): Json<UserForm>,
) -> Result<Response, InertiaError> {
    let mut errors = ValidationErrors::new();

    if form.name.trim().is_empty() {
        errors.push("default", "name", "The name field is required.");
    }

    if !form.email.contains('@') {
        errors.push(
            "default",
            "email",
            "The email must be a valid email address.",
        );
    }

    if !errors.is_empty() {
        request.with_errors(errors);
        return request.back_or("/users/create");
    }

    let id = {
        let mut users = state.users.write().expect("users lock");
        let id = users.iter().map(|user| user.id).max().unwrap_or(0) + 1;
        users.push(User {
            id,
            name: form.name.trim().to_owned(),
            email: form.email,
        });
        id
    };

    request.flash("message", "User created.")?;
    request.flash("newUserId", id)?;
    request.redirect(Inertia::redirect("/users"))
}

async fn users_show(
    State(state): State<AppState>,
    request: InertiaRequest,
    Path(id): Path<u64>,
) -> Result<Response, InertiaError> {
    let user = state
        .users
        .read()
        .expect("users lock")
        .iter()
        .find(|user| user.id == id)
        .cloned();
    let Some(user) = user else {
        return not_found(request).await;
    };

    request
        .respond(Inertia::response(
            "Users/Show",
            Props::new()
                .value("user", user)
                .defer_async("activity", move || async move {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    vec![
                        format!("User {id} signed in"),
                        format!("User {id} updated profile"),
                    ]
                }),
        ))
        .await
}

async fn users_destroy(
    State(state): State<AppState>,
    request: InertiaRequest,
    Path(id): Path<u64>,
) -> Result<Response, InertiaError> {
    state
        .users
        .write()
        .expect("users lock")
        .retain(|user| user.id != id);
    request.flash("message", format!("User {id} deleted."))?;
    request.redirect(Inertia::redirect("/users"))
}

async fn external(request: InertiaRequest) -> Result<Response, InertiaError> {
    request.location(Inertia::location("https://inertiajs.com/"))
}

async fn fragment(request: InertiaRequest) -> Result<Response, InertiaError> {
    request.redirect(Inertia::redirect("/users#bottom"))
}

async fn not_found(request: InertiaRequest) -> Result<Response, InertiaError> {
    error_page(
        &request,
        StatusCode::NOT_FOUND,
        Inertia::response("Error", json!({ "status": 404 })),
    )
    .await
}

/// Test-only hook: changes the asset version so the next Inertia visit gets a
/// version conflict. Enabled with `INERTIA_E2E=1`.
async fn bump_version(State(state): State<AppState>) -> impl IntoResponse {
    state.test_version.fetch_add(1, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

fn ssr() -> Option<Ssr> {
    let gateway = if let Ok(url) = std::env::var("INERTIA_SSR_URL") {
        HttpSsrGateway::new(&url)
    } else if let (Ok(origin), Ok(_)) = (
        std::env::var("VITE_DEV_SERVER"),
        std::env::var("INERTIA_SSR_DEV"),
    ) {
        HttpSsrGateway::vite_dev(&origin)
    } else {
        return None;
    };

    Some(Ssr::new(
        gateway
            .expect("valid SSR URL")
            .timeout(Duration::from_secs(2)),
    ))
}

fn app(state: AppState) -> Router {
    let assets = load_assets();
    let base_version = match &assets {
        Assets::Dev { .. } => "dev".to_owned(),
        Assets::Build { version, .. } => version.clone(),
    };
    let test_version = state.test_version.clone();
    let shared = SharedProps::new()
        .value("appName", "Inertia Rust")
        .value("auth.user", json!({ "name": "Ada Lovelace" }))
        // Shared once prop: resolved on the first visit, then remembered by
        // the client and skipped by the server on later visits.
        .typed("countries", |_| {
            Prop::lazy(|| vec!["Netherlands", "United States", "Japan"]).once()
        });

    let mut layer = InertiaLayer::dynamic(move || match test_version.load(Ordering::SeqCst) {
        0 => base_version.clone(),
        bump => format!("{base_version}-{bump}"),
    })
    .template(template(assets))
    .shared(shared)
    .session(TowerSessions::new())
    .prop_timeout(Duration::from_secs(5));

    if let Some(ssr) = ssr() {
        println!("server-side rendering enabled");
        layer = layer.ssr(ssr);
    }

    let mut router = Router::new()
        .route("/", get(dashboard))
        .route("/orders", post(place_order))
        .route("/inertia/live", get(live))
        .route("/users", get(users_index).post(users_store))
        .route("/users/create", get(users_create))
        .route("/users/{id}", get(users_show).delete(users_destroy))
        .route("/external", get(external))
        .route("/fragment", get(fragment));

    if std::env::var("INERTIA_E2E").is_ok() {
        router = router.route("/__e2e/bump-version", post(bump_version));
    }

    router
        .fallback(not_found)
        .with_state(state)
        .nest_service("/build", ServeDir::new(example_dir().join("public/build")))
        .layer(layer)
        .layer(SessionManagerLayer::new(MemoryStore::default()).with_secure(false))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,inertia_rs=debug".into()),
        )
        .init();

    let addr = std::env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind listener");

    println!("listening on http://{addr}");
    axum::serve(listener, app(AppState::new()))
        .await
        .expect("server failed");
}
