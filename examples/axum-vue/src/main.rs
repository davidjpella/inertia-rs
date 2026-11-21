//! Axum + Vue 3 smoke example for the official `@inertiajs/vue3` adapter.

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let public_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public");
    let app = smoke_server::app(public_dir, "src/main.js", axum::Router::new());

    smoke_server::serve(app, "127.0.0.1:3003").await;
}
