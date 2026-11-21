//! Shared Axum backend for the Vue and Svelte smoke examples.
//!
//! The routes cover the protocol paths that every official client adapter
//! must handle: first-page HTML, client-side navigation, shared props,
//! deferred and optional props, partial reloads, validation errors, flash
//! data, and asset-version conflicts.

use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use inertia_rs::axum::{InertiaError, InertiaLayer, InertiaRequest, SharedProps, TowerSessions};
use inertia_rs::session::ValidationErrors;
use inertia_rs::{HtmlResponseContext, Inertia, Props};
use serde::Deserialize;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer};

#[derive(Deserialize)]
struct ManifestEntry {
    file: String,
    #[serde(default)]
    css: Vec<String>,
}

struct Assets {
    tags: String,
    version: String,
}

fn load_assets(public_dir: &Path, entry: &str) -> Assets {
    let path = public_dir.join("build/.vite/manifest.json");
    let manifest = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}; build the frontend first", path.display()));
    let mut hasher = DefaultHasher::new();
    manifest.hash(&mut hasher);
    let entries: BTreeMap<String, ManifestEntry> =
        serde_json::from_str(&manifest).expect("valid Vite manifest");
    let entry = entries
        .get(entry)
        .expect("Vite manifest contains the entry");
    let styles = entry
        .css
        .iter()
        .map(|css| format!(r#"<link rel="stylesheet" href="/build/{css}" />"#))
        .collect::<String>();

    Assets {
        tags: format!(
            r#"{styles}<script type="module" src="/build/{}"></script>"#,
            entry.file
        ),
        version: format!("{:016x}", hasher.finish()),
    }
}

async fn home(request: InertiaRequest) -> Result<Response, InertiaError> {
    request
        .respond(Inertia::response(
            "Smoke/Home",
            Props::new()
                .value("message", "Hello from the smoke server")
                .defer_async("stats", || async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    json!({ "visits": 42 })
                })
                .optional("debug", || "loaded on demand"),
        ))
        .await
}

async fn form(request: InertiaRequest) -> Result<Response, InertiaError> {
    request
        .respond(Inertia::response("Smoke/Form", json!({})))
        .await
}

#[derive(Deserialize)]
struct FormData {
    #[serde(default)]
    name: String,
}

async fn submit(
    request: InertiaRequest,
    Json(data): Json<FormData>,
) -> Result<Response, InertiaError> {
    if data.name.trim().is_empty() {
        request.with_errors(ValidationErrors::new().add("name", "The name field is required."));
        return request.back_or("/smoke/form");
    }

    request.flash("message", format!("Saved {}", data.name.trim()))?;
    request.redirect(Inertia::redirect("/"))
}

/// Builds the smoke application. `extra` routes (such as an example's own
/// pages) are merged in before the Inertia layer is applied.
pub fn app(public_dir: PathBuf, entry: &str, extra: Router) -> Router {
    let assets = Arc::new(load_assets(&public_dir, entry));
    let template_assets = assets.clone();
    let bump = Arc::new(AtomicU64::new(0));
    let version_bump = bump.clone();
    let base_version = assets.version.clone();

    let layer = InertiaLayer::dynamic(move || match version_bump.load(Ordering::SeqCst) {
        0 => base_version.clone(),
        bump => format!("{base_version}-{bump}"),
    })
    .template(move |page: HtmlResponseContext| {
        Html(format!(
            r#"<!doctype html><html lang="en"><head><meta charset="utf-8" /><title>Inertia smoke</title>{}</head><body>{}</body></html>"#,
            template_assets.tags,
            page.body()
        ))
    })
    .shared(SharedProps::new().value("appName", "Inertia smoke"))
    .session(TowerSessions::new());

    let mut router = Router::new()
        .route("/", get(home))
        .route("/smoke/form", get(form).post(submit))
        .merge(extra);

    if std::env::var("INERTIA_E2E").is_ok() {
        router = router.route(
            "/__e2e/bump-version",
            post(move || {
                bump.fetch_add(1, Ordering::SeqCst);
                async { axum::http::StatusCode::NO_CONTENT.into_response() }
            }),
        );
    }

    router
        .nest_service("/build", ServeDir::new(public_dir.join("build")))
        .layer(layer)
        .layer(SessionManagerLayer::new(MemoryStore::default()).with_secure(false))
}

/// Serves `app` on `ADDR`, or `default_addr`.
pub async fn serve(app: Router, default_addr: &str) {
    let addr = std::env::var("ADDR").unwrap_or_else(|_| default_addr.to_owned());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind listener");
    println!("listening on http://{addr}");
    axum::serve(listener, app).await.expect("server failed");
}
