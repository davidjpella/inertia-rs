//! Axum + Svelte 5 example. Serves the `Hello` page plus the shared smoke
//! routes used by the end-to-end suite.

use axum::response::Response;
use axum::routing::get;
use axum::Router;
use inertia_rs::axum::{InertiaError, InertiaRequest};
use inertia_rs::{Inertia, Props};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

async fn hello(request: InertiaRequest) -> Result<Response, InertiaError> {
    request
        .respond(Inertia::response(
            "Hello",
            Props::new()
                .value("name", "world")
                .value(
                    "message",
                    "Rendered by Axum and hydrated by Svelte through Inertia.",
                )
                .value(
                    "auth.user",
                    serde_json::json!({ "name": "Grace Hopper", "role": "Example user" }),
                )
                .defer("stats", || {
                    serde_json::json!({
                        "adapter": "Axum",
                        "deferred": true,
                        "generatedAt": SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|elapsed| elapsed.as_secs())
                            .unwrap_or_default(),
                    })
                })
                .optional("debug", || {
                    serde_json::json!({
                        "partialReload": true,
                        "loadedBy": "X-Inertia-Partial-Data: debug",
                    })
                }),
        ))
        .await
}

#[tokio::main]
async fn main() {
    let public_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public");
    let app = smoke_server::app(
        public_dir,
        "src/main.js",
        Router::new().route("/hello", get(hello)),
    );

    smoke_server::serve(app, "127.0.0.1:3002").await;
}
