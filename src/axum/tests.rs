use super::*;
use crate::ssr::{SsrFailurePolicy, SsrGateway, SsrResponse};
use crate::{
    Inertia, InertiaProps, LiveChannel, LiveUpdates, Prop, Props, X_INERTIA_EXCEPT_ONCE_PROPS,
    X_INERTIA_PARTIAL_COMPONENT, X_INERTIA_PARTIAL_DATA,
};
use ::axum::body::Body;
use ::axum::http::header::CONTENT_TYPE;
use ::axum::response::Html;
use ::axum::routing::{any, get};
use ::axum::{Extension, Router};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

#[derive(Clone)]
struct User {
    name: &'static str,
}

fn template(page: HtmlResponseContext) -> Html<String> {
    Html(format!(
        "<!doctype html><html><head>{}</head><body>{}</body></html>",
        page.head(),
        page.body()
    ))
}

async fn page(request: InertiaRequest) -> Result<Response, InertiaError> {
    request.render(
        Inertia::response(
            "foo",
            json!({ "n": 42, "plans": ["basic"], "stats": 7, "notifications": ["welcome"] }),
        )
        .once("plans")
        .defer("stats"),
        template,
    )
}

async fn lazy_page(request: InertiaRequest) -> Result<Response, InertiaError> {
    request
        .respond(Inertia::response(
            "lazy",
            InertiaProps::new()
                .value("users", json!(["Ada", "Grace"]))
                .defer_async("stats", || async { 7 })
                .optional("audit", || json!(["created"])),
        ))
        .await
}

fn get_request(uri: &str) -> ::axum::http::request::Builder {
    Request::builder().uri(uri)
}

fn inertia_get(uri: &str) -> ::axum::http::request::Builder {
    get_request(uri)
        .header(X_INERTIA, "true")
        .header(X_INERTIA_VERSION, "1")
}

async fn send(app: Router, request: Request<Body>) -> Response {
    app.oneshot(request).await.unwrap()
}

async fn body_text(response: Response) -> String {
    let bytes = ::axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn body_json(response: Response) -> Value {
    serde_json::from_str(&body_text(response).await).unwrap()
}

fn header_str<'r>(response: &'r Response, name: &str) -> Option<&'r str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn app() -> Router {
    Router::new()
        .route("/foo", get(page).post(page))
        .route("/lazy", get(lazy_page))
        .layer(InertiaLayer::new("1").template(template))
}

#[tokio::test]
async fn html_response_includes_escaped_page_and_version() {
    let response = send(
        app(),
        get_request("/foo?bar=baz").body(Body::empty()).unwrap(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header_str(&response, "vary"), Some(X_INERTIA));
    let body = body_text(response).await;
    assert!(body.contains(r#"<script data-page="app" type="application/json">"#));
    assert!(body.contains(r#"<div id="app"></div>"#));
    assert!(body.contains(r#""url":"/foo?bar=baz""#));
    assert!(body.contains(r#""version":"1""#));
}

#[tokio::test]
async fn html_response_escapes_json_for_script_context() {
    async fn unsafe_page(request: InertiaRequest) -> Result<Response, InertiaError> {
        request
            .respond(Inertia::response(
                "unsafe",
                json!({ "text": "</script><script>alert(1)</script>&\u{2028}\u{2029}" }),
            ))
            .await
    }

    let app = Router::new()
        .route("/unsafe", get(unsafe_page))
        .layer(InertiaLayer::new("1").template(template));
    let body =
        body_text(send(app, get_request("/unsafe").body(Body::empty()).unwrap()).await).await;

    assert!(!body.contains("</script><script>"));
    assert!(body.contains("\\u003C/script\\u003E"));
    assert!(body.contains("\\u0026"));
    assert!(body.contains("\\u2028\\u2029"));
}

#[tokio::test]
async fn inertia_json_response_includes_headers_url_and_version() {
    let response = send(
        app(),
        inertia_get("/foo?bar=baz").body(Body::empty()).unwrap(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_str(&response, CONTENT_TYPE.as_str()),
        Some("application/json")
    );
    assert_eq!(header_str(&response, X_INERTIA), Some("true"));
    assert_eq!(header_str(&response, "vary"), Some(X_INERTIA));

    let page = body_json(response).await;
    assert_eq!(page["component"], "foo");
    assert_eq!(page["url"], "/foo?bar=baz");
    assert_eq!(page["version"], "1");
    assert_eq!(page["props"]["n"], 42);
    assert!(page["props"].get("stats").is_none());
    assert_eq!(page["deferredProps"], json!({ "default": ["stats"] }));
    assert_eq!(page["sharedProps"], json!(["errors"]));
}

#[tokio::test]
async fn nested_routes_use_original_uri_for_page_urls_and_conflicts() {
    let app = Router::new().nest(
        "/nested",
        Router::new()
            .route("/foo", get(page))
            .layer(InertiaLayer::new("1")),
    );

    let response = send(
        app.clone(),
        inertia_get("/nested/foo?bar=baz")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(body_json(response).await["url"], "/nested/foo?bar=baz");

    let response = send(
        app,
        get_request("/nested/foo?bar=baz")
            .header(X_INERTIA, "true")
            .header(X_INERTIA_VERSION, "stale")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        header_str(&response, X_INERTIA_LOCATION),
        Some("/nested/foo?bar=baz")
    );
}

#[tokio::test]
async fn absolute_form_requests_use_local_page_urls() {
    let response = send(
        app(),
        inertia_get("http://example.com/foo?bar=baz")
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(body_json(response).await["url"], "/foo?bar=baz");
}

#[tokio::test]
async fn render_respects_explicit_url_and_builder_metadata() {
    async fn builder(request: InertiaRequest) -> Result<Response, InertiaError> {
        request
            .respond(
                Inertia::page("builder")
                    .merge("notifications")
                    .with_url("/custom-url")
                    .props(json!({ "notifications": ["welcome"] })),
            )
            .await
    }

    let app = Router::new()
        .route("/builder", get(builder))
        .layer(InertiaLayer::new("1"));
    let page =
        body_json(send(app, inertia_get("/builder").body(Body::empty()).unwrap()).await).await;

    assert_eq!(page["url"], "/custom-url");
    assert_eq!(page["mergeProps"], json!(["notifications"]));
}

#[tokio::test]
async fn async_lazy_deferred_and_optional_props_through_axum() {
    let full =
        body_json(send(app(), inertia_get("/lazy").body(Body::empty()).unwrap()).await).await;
    assert_eq!(
        full["props"],
        json!({ "users": ["Ada", "Grace"], "errors": {} })
    );
    assert_eq!(full["deferredProps"], json!({ "default": ["stats"] }));

    let deferred = body_json(
        send(
            app(),
            inertia_get("/lazy")
                .header(X_INERTIA_PARTIAL_COMPONENT, "lazy")
                .header(X_INERTIA_PARTIAL_DATA, "stats,audit")
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;
    assert_eq!(
        deferred["props"],
        json!({ "stats": 7, "audit": ["created"], "errors": {} })
    );
}

#[tokio::test]
async fn respond_without_template_fails_for_html_but_not_json() {
    let app = Router::new()
        .route("/lazy", get(lazy_page))
        .layer(InertiaLayer::new("1"));

    let html = send(
        app.clone(),
        get_request("/lazy").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(html.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = send(app, inertia_get("/lazy").body(Body::empty()).unwrap()).await;
    assert_eq!(json.status(), StatusCode::OK);
}

fn shared_app(shared: SharedProps) -> Router {
    async fn route_auth(request: InertiaRequest) -> Result<Response, InertiaError> {
        request
            .respond(Inertia::response(
                "route-auth",
                json!({ "auth": { "user": { "name": "Route" } }, "n": 1 }),
            ))
            .await
    }

    Router::new()
        .route("/foo", get(page))
        .route("/route-auth", get(route_auth))
        .layer(InertiaLayer::new("1").template(template).shared(shared))
        .layer(Extension(User { name: "Ada" }))
}

#[tokio::test]
async fn shared_props_are_resolved_with_route_props() {
    let shared = SharedProps::new()
        .value("appName", "Demo")
        .value("n", 99)
        .prop("auth.user", |request| {
            request
                .extension::<User>()
                .map(|user| json!({ "name": user.name }))
        });
    let page = body_json(
        send(
            shared_app(shared),
            inertia_get("/foo").body(Body::empty()).unwrap(),
        )
        .await,
    )
    .await;

    assert_eq!(page["props"]["appName"], "Demo");
    assert_eq!(page["props"]["n"], 42, "route props win");
    assert_eq!(page["props"]["auth"], json!({ "user": { "name": "Ada" } }));
    assert_eq!(
        page["sharedProps"],
        json!(["errors", "appName", "n", "auth"])
    );
}

#[tokio::test]
async fn shared_props_registered_as_extensions_still_work() {
    let app = Router::new()
        .route("/foo", get(page))
        .layer(InertiaLayer::new("1"))
        .layer(Extension(SharedProps::new().value("appName", "Legacy")));
    let page = body_json(send(app, inertia_get("/foo").body(Body::empty()).unwrap()).await).await;

    assert_eq!(page["props"]["appName"], "Legacy");
}

#[tokio::test]
async fn partial_reloads_filter_shared_props_and_skip_their_providers() {
    let calls = Arc::new(AtomicUsize::new(0));
    let shared = SharedProps::new()
        .value("appName", "Demo")
        .prop("expensive", {
            let calls = calls.clone();
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                "value"
            }
        });
    let page = body_json(
        send(
            shared_app(shared),
            inertia_get("/foo")
                .header(X_INERTIA_PARTIAL_COMPONENT, "foo")
                .header(X_INERTIA_PARTIAL_DATA, "n")
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;

    assert_eq!(page["props"], json!({ "n": 42, "errors": {} }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn shared_dotted_props_do_not_replace_route_owned_roots() {
    let shared = SharedProps::new()
        .value("auth.user", json!({ "name": "Shared" }))
        .value("auth.csrf", "token");
    let page = body_json(
        send(
            shared_app(shared),
            inertia_get("/route-auth").body(Body::empty()).unwrap(),
        )
        .await,
    )
    .await;

    assert_eq!(
        page["props"]["auth"],
        json!({ "user": { "name": "Route" } })
    );
}

#[tokio::test]
async fn optional_and_async_shared_props() {
    let shared = SharedProps::new()
        .prop_optional("missing", |_| Option::<String>::None)
        .prop_async("count", |request| async move { request.uri().len() })
        .typed("countries", |_| Prop::lazy(|| ["NL"]).once());
    let page = body_json(
        send(
            shared_app(shared),
            inertia_get("/route-auth")
                .header(X_INERTIA_EXCEPT_ONCE_PROPS, "countries")
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;

    assert!(page["props"].get("missing").is_none());
    assert_eq!(page["props"]["count"], "/route-auth".len());
    assert!(page["props"].get("countries").is_none());
    assert_eq!(page["onceProps"]["countries"]["prop"], "countries");
}

#[tokio::test]
async fn request_extensions_are_available() {
    async fn extension_value(request: InertiaRequest) -> String {
        request
            .extension::<User>()
            .map(|user| user.name)
            .unwrap_or("missing")
            .to_owned()
    }

    let app = Router::new()
        .route("/extension-value", get(extension_value))
        .layer(Extension(User { name: "Ada" }));
    let response = send(
        app,
        get_request("/extension-value").body(Body::empty()).unwrap(),
    )
    .await;

    assert_eq!(body_text(response).await, "Ada");
}

#[tokio::test]
async fn unversioned_apps_send_an_empty_version_and_accept_a_missing_header() {
    let app = Router::new()
        .route("/foo", get(page))
        .layer(InertiaLayer::unversioned());
    let response = send(
        app,
        get_request("/foo")
            .header(X_INERTIA, "true")
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["version"], "");
}

#[tokio::test]
async fn routes_without_the_layer_still_render() {
    let app = Router::new().route("/foo", get(page));
    let response = send(app, inertia_get("/foo").body(Body::empty()).unwrap()).await;

    assert_eq!(body_json(response).await["version"], "");
}

#[tokio::test]
async fn version_conflicts_short_circuit_and_report_the_server_version() {
    let calls = Arc::new(AtomicUsize::new(0));
    let handler = {
        let calls = calls.clone();
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { "handler" }
        }
    };
    let app = Router::new()
        .route("/foo", get(handler))
        .layer(InertiaLayer::new("2"));

    for version in [Some("1"), None] {
        let mut request = get_request("/foo?x=1").header(X_INERTIA, "true");
        if let Some(version) = version {
            request = request.header(X_INERTIA_VERSION, version);
        }
        let response = send(app.clone(), request.body(Body::empty()).unwrap()).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(header_str(&response, X_INERTIA_LOCATION), Some("/foo?x=1"));
        assert_eq!(header_str(&response, X_INERTIA_VERSION), Some("2"));
        assert_eq!(header_str(&response, "vary"), Some(X_INERTIA));
    }

    // Non-GET requests never receive a version conflict.
    let response = send(
        app,
        Request::builder()
            .method(Method::POST)
            .uri("/foo")
            .header(X_INERTIA, "true")
            .header(X_INERTIA_VERSION, "1")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_ne!(response.status(), StatusCode::CONFLICT);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dynamic_version_is_resolved_per_request() {
    let version = Arc::new(AtomicUsize::new(1));
    let app = Router::new()
        .route("/foo", get(page))
        .layer(InertiaLayer::dynamic({
            let version = version.clone();
            move || format!("dynamic-{}", version.load(Ordering::SeqCst))
        }));
    let request = || {
        get_request("/foo")
            .header(X_INERTIA, "true")
            .header(X_INERTIA_VERSION, "dynamic-1")
            .body(Body::empty())
            .unwrap()
    };

    assert_eq!(send(app.clone(), request()).await.status(), StatusCode::OK);
    version.store(2, Ordering::SeqCst);
    assert_eq!(send(app, request()).await.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn partial_reload_headers_apply_to_every_method() {
    // Reference adapter: PropsResolver applies partial headers regardless of
    // the request method (0.4 ignored them for non-GET requests).
    let response = send(
        app(),
        Request::builder()
            .method(Method::POST)
            .uri("/foo")
            .header(X_INERTIA, "true")
            .header(X_INERTIA_PARTIAL_COMPONENT, "foo")
            .header(X_INERTIA_PARTIAL_DATA, "n")
            .header(X_INERTIA_EXCEPT_ONCE_PROPS, "plans")
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(
        body_json(response).await["props"],
        json!({ "n": 42, "errors": {} })
    );
}

fn redirect_app() -> Router {
    async fn external(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.location(Inertia::location("https://example.com/outside#top"))
    }
    async fn go(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.redirect(Inertia::redirect("/target"))
    }
    async fn fragment(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.redirect(Inertia::redirect("/target#section"))
    }
    async fn raw_found() -> Response {
        (StatusCode::FOUND, [(LOCATION, "/raw")]).into_response()
    }
    async fn bad(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.redirect(Inertia::redirect("bad location"))
    }
    async fn empty() -> StatusCode {
        StatusCode::OK
    }
    async fn back(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.back()
    }
    async fn plain() -> &'static str {
        "plain"
    }

    Router::new()
        .route("/external", any(external))
        .route("/go", any(go))
        .route("/fragment", any(fragment))
        .route("/raw", any(raw_found))
        .route("/bad", get(bad))
        .route("/empty", any(empty))
        .route("/back", any(back))
        .route("/plain", get(plain))
        .layer(InertiaLayer::new("1"))
}

fn method_request(method: &str, uri: &str, inertia: bool) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(uri);
    if inertia {
        builder = builder
            .header(X_INERTIA, "true")
            .header(X_INERTIA_VERSION, "1");
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn external_locations_always_use_x_inertia_location() {
    let response = send(redirect_app(), method_request("GET", "/external", true)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        header_str(&response, X_INERTIA_LOCATION),
        Some("https://example.com/outside#top")
    );
    assert!(response.headers().get(X_INERTIA_REDIRECT).is_none());

    let response = send(redirect_app(), method_request("GET", "/external", false)).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let response = send(redirect_app(), method_request("POST", "/external", false)).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn redirect_statuses_follow_the_method() {
    for (method, status) in [
        ("GET", StatusCode::FOUND),
        ("POST", StatusCode::SEE_OTHER),
        ("PUT", StatusCode::SEE_OTHER),
        ("PATCH", StatusCode::SEE_OTHER),
        ("DELETE", StatusCode::SEE_OTHER),
        ("QUERY", StatusCode::SEE_OTHER),
    ] {
        let response = send(redirect_app(), method_request(method, "/go", true)).await;
        assert_eq!(response.status(), status, "{method}");
        assert_eq!(header_str(&response, LOCATION.as_str()), Some("/target"));
    }
}

#[tokio::test]
async fn middleware_converts_handler_built_302_after_write_methods() {
    for (method, status) in [
        ("PUT", StatusCode::SEE_OTHER),
        ("PATCH", StatusCode::SEE_OTHER),
        ("DELETE", StatusCode::SEE_OTHER),
        ("QUERY", StatusCode::SEE_OTHER),
        ("POST", StatusCode::FOUND),
        ("GET", StatusCode::FOUND),
    ] {
        let response = send(redirect_app(), method_request(method, "/raw", true)).await;
        assert_eq!(response.status(), status, "{method}");
    }

    // Non-Inertia requests are left alone, like the reference middleware.
    let response = send(redirect_app(), method_request("PUT", "/raw", false)).await;
    assert_eq!(response.status(), StatusCode::FOUND);
}

#[tokio::test]
async fn fragment_redirects_become_inertia_redirects_except_for_prefetch() {
    let response = send(redirect_app(), method_request("POST", "/fragment", true)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        header_str(&response, X_INERTIA_REDIRECT),
        Some("/target#section")
    );
    assert!(response.headers().get(LOCATION).is_none());

    let mut prefetch = method_request("GET", "/fragment", true);
    prefetch
        .headers_mut()
        .insert(PURPOSE, HeaderValue::from_static("prefetch"));
    let response = send(redirect_app(), prefetch).await;
    assert_eq!(response.status(), StatusCode::FOUND);

    let response = send(redirect_app(), method_request("GET", "/fragment", false)).await;
    assert_eq!(response.status(), StatusCode::FOUND);
}

#[tokio::test]
async fn empty_inertia_responses_redirect_back() {
    let mut request = method_request("PUT", "/empty", true);
    request
        .headers_mut()
        .insert(REFERER, HeaderValue::from_static("/form"));
    let response = send(redirect_app(), request).await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(header_str(&response, LOCATION.as_str()), Some("/form"));

    let response = send(redirect_app(), method_request("GET", "/empty", false)).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn back_uses_the_referer() {
    let mut request = method_request("POST", "/back", true);
    request.headers_mut().insert(
        REFERER,
        HeaderValue::from_static("http://localhost/users?page=2"),
    );
    let response = send(redirect_app(), request).await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_str(&response, LOCATION.as_str()),
        Some("http://localhost/users?page=2")
    );
}

#[tokio::test]
async fn vary_is_added_to_every_response_once() {
    let response = send(redirect_app(), method_request("GET", "/plain", false)).await;
    assert_eq!(
        response.headers().get_all(VARY).iter().count(),
        1,
        "Vary is set on non-Inertia responses too"
    );
}

#[tokio::test]
async fn invalid_redirect_targets_are_server_errors() {
    let response = send(redirect_app(), method_request("GET", "/bad", true)).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

struct FakeSsr {
    fail: bool,
    calls: Arc<AtomicUsize>,
}

impl SsrGateway for FakeSsr {
    fn render<'a>(&'a self, page: &'a Value) -> BoxFuture<'a, Result<SsrResponse, SsrError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let fail = self.fail;
        let component = page["component"].as_str().unwrap_or_default().to_owned();
        Box::pin(async move {
            if fail {
                Err(SsrError::new(
                    crate::ssr::SsrErrorKind::Connection,
                    "refused",
                ))
            } else {
                Ok(SsrResponse::new(
                    vec![format!("<title>{component}</title>")],
                    format!(r#"<div data-server-rendered="true" id="app">{component}</div>"#),
                ))
            }
        })
    }
}

fn ssr_app(fail: bool, policy: SsrFailurePolicy, calls: Arc<AtomicUsize>) -> Router {
    Router::new().route("/lazy", get(lazy_page)).layer(
        InertiaLayer::new("1").template(template).ssr(
            crate::ssr::Ssr::new(FakeSsr { fail, calls })
                .failure_policy(policy)
                .on_failure(|_, _| {}),
        ),
    )
}

#[tokio::test]
async fn ssr_renders_first_visits_only() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = ssr_app(false, SsrFailurePolicy::Fallback, calls.clone());

    let body = body_text(
        send(
            app.clone(),
            get_request("/lazy").body(Body::empty()).unwrap(),
        )
        .await,
    )
    .await;
    assert!(body.contains("<head><title>lazy</title></head>"));
    assert!(body.contains(r#"<div data-server-rendered="true" id="app">lazy</div>"#));
    assert!(!body.contains("data-page"));

    let json = send(app, inertia_get("/lazy").body(Body::empty()).unwrap()).await;
    assert_eq!(json.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ssr_failures_fall_back_or_fail_by_policy() {
    let calls = Arc::new(AtomicUsize::new(0));
    let body = body_text(
        send(
            ssr_app(true, SsrFailurePolicy::Fallback, calls.clone()),
            get_request("/lazy").body(Body::empty()).unwrap(),
        )
        .await,
    )
    .await;
    assert!(body.contains(r#"<script data-page="app""#));

    let response = send(
        ssr_app(true, SsrFailurePolicy::Error, calls),
        get_request("/lazy").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn prop_timeouts_are_configurable_on_the_layer() {
    async fn slow(request: InertiaRequest) -> Result<Response, InertiaError> {
        request
            .respond(Inertia::response(
                "slow",
                Props::new()
                    .lazy_async("slow", || async {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        1
                    })
                    .prop(
                        "rescued",
                        Prop::lazy_async(|| async {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            1
                        })
                        .defer()
                        .rescue(),
                    ),
            ))
            .await
    }

    let app = Router::new().route("/slow", get(slow)).layer(
        InertiaLayer::new("1")
            .prop_timeout(Duration::from_millis(20))
            .resolve_options(ResolveOptions::new().on_rescued_error(|_| {}))
            .prop_timeout(Duration::from_millis(20)),
    );

    let response = send(
        app.clone(),
        inertia_get("/slow").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let response = send(
        app,
        inertia_get("/slow")
            .header(X_INERTIA_PARTIAL_COMPONENT, "slow")
            .header(X_INERTIA_PARTIAL_DATA, "rescued")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let page = body_json(response).await;
    assert_eq!(page["rescuedProps"], json!(["rescued"]));
}

#[tokio::test]
async fn live_props_are_serialized_through_axum() {
    async fn order(request: InertiaRequest) -> Result<Response, InertiaError> {
        request
            .respond(Inertia::response(
                "Orders/Show",
                Props::new().prop(
                    "order",
                    Prop::lazy(|| json!({ "status": "paid" })).live(
                        LiveUpdates::on("OrderUpdated").channel(LiveChannel::private("orders.1")),
                    ),
                ),
            ))
            .await
    }

    let app = Router::new()
        .route("/orders/1", get(order))
        .layer(InertiaLayer::new("1"));
    let page =
        body_json(send(app, inertia_get("/orders/1").body(Body::empty()).unwrap()).await).await;

    assert_eq!(
        page["liveProps"]["order"]["listeners"][0]["channel"],
        json!({ "name": "orders.1", "type": "private" })
    );
}

#[cfg(feature = "tower-sessions")]
mod sessions {
    use super::*;
    use crate::session::ValidationErrors;
    use crate::X_INERTIA_ERROR_BAG;
    use ::axum::http::header::{COOKIE, SET_COOKIE};
    use ::axum::routing::post;
    use tower_sessions::{MemoryStore, SessionManagerLayer};

    async fn store(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.flash("message", "User created")?;
        request.redirect(Inertia::redirect("/users"))
    }

    async fn invalid(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.with_errors(ValidationErrors::new().add("name", "The name field is required."));
        request.back_or("/users/create")
    }

    async fn logout(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.clear_history();
        request.redirect(Inertia::redirect("/users"))
    }

    async fn users(request: InertiaRequest) -> Result<Response, InertiaError> {
        request
            .respond(Inertia::response("Users/Index", json!({ "users": [] })))
            .await
    }

    async fn render_with_flash(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.flash("inline", true)?;
        request
            .respond(Inertia::response("Inline", ()).flash("highlight", 3)?)
            .await
    }

    pub(super) fn session_app() -> Router {
        Router::new()
            .route("/users", get(users).post(store))
            .route("/users/create", get(users).post(invalid))
            .route("/logout", post(logout))
            .route("/inline", get(render_with_flash))
            .route("/favicon.ico", get(|| async { "icon" }))
            .layer(
                InertiaLayer::new("1")
                    .template(template)
                    .session(TowerSessions::new()),
            )
            .layer(SessionManagerLayer::new(MemoryStore::default()).with_secure(false))
    }

    pub(super) fn cookie(response: &Response) -> Option<String> {
        response
            .headers()
            .get(SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap().to_owned())
    }

    pub(super) fn with_cookie(mut request: Request<Body>, cookie: &str) -> Request<Body> {
        request
            .headers_mut()
            .insert(COOKIE, HeaderValue::from_str(cookie).unwrap());
        request
    }

    #[tokio::test]
    async fn flash_survives_one_redirect_and_is_delivered_once() {
        let app = session_app();
        let response = send(app.clone(), method_request("POST", "/users", true)).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let cookie = cookie(&response).expect("session cookie");

        // An unrelated request does not consume the flash.
        send(
            app.clone(),
            with_cookie(method_request("GET", "/favicon.ico", false), &cookie),
        )
        .await;

        let page = body_json(
            send(
                app.clone(),
                with_cookie(method_request("GET", "/users", true), &cookie),
            )
            .await,
        )
        .await;
        assert_eq!(page["flash"], json!({ "message": "User created" }));
        assert!(page["props"].get("flash").is_none());

        let page = body_json(
            send(
                app,
                with_cookie(method_request("GET", "/users", true), &cookie),
            )
            .await,
        )
        .await;
        assert!(page.get("flash").is_none());
    }

    #[tokio::test]
    async fn flash_is_kept_through_a_version_conflict() {
        let app = session_app();
        let response = send(app.clone(), method_request("POST", "/users", true)).await;
        let cookie = cookie(&response).unwrap();

        let mut stale = method_request("GET", "/users", true);
        stale
            .headers_mut()
            .insert(X_INERTIA_VERSION, HeaderValue::from_static("0"));
        let conflict = send(app.clone(), with_cookie(stale, &cookie)).await;
        assert_eq!(conflict.status(), StatusCode::CONFLICT);

        // The client follows with a full page load.
        let html = body_text(
            send(
                app,
                with_cookie(method_request("GET", "/users", false), &cookie),
            )
            .await,
        )
        .await;
        assert!(html.contains(r#""flash":{"message":"User created"}"#));
    }

    #[tokio::test]
    async fn validation_errors_redirect_back_and_respect_error_bags() {
        let app = session_app();
        let mut request = method_request("POST", "/users/create", true);
        request
            .headers_mut()
            .insert(REFERER, HeaderValue::from_static("/users/create"));
        let response = send(app.clone(), request).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            header_str(&response, LOCATION.as_str()),
            Some("/users/create")
        );
        let cookie = cookie(&response).unwrap();

        let page = body_json(
            send(
                app.clone(),
                with_cookie(method_request("GET", "/users/create", true), &cookie),
            )
            .await,
        )
        .await;
        assert_eq!(
            page["props"]["errors"],
            json!({ "name": "The name field is required." })
        );

        let page = body_json(
            send(
                app.clone(),
                with_cookie(method_request("GET", "/users/create", true), &cookie),
            )
            .await,
        )
        .await;
        assert_eq!(
            page["props"]["errors"],
            json!({}),
            "errors are delivered once"
        );

        let mut request = method_request("POST", "/users/create", true);
        request
            .headers_mut()
            .insert(X_INERTIA_ERROR_BAG, HeaderValue::from_static("createUser"));
        let response = send(app.clone(), with_cookie(request, &cookie)).await;
        let mut follow = method_request("GET", "/users/create", true);
        follow
            .headers_mut()
            .insert(X_INERTIA_ERROR_BAG, HeaderValue::from_static("createUser"));
        let page =
            body_json(send(app, with_cookie(follow, &cookie_or(&response, &cookie))).await).await;
        assert_eq!(
            page["props"]["errors"],
            json!({ "createUser": { "name": "The name field is required." } })
        );
    }

    fn cookie_or(response: &Response, fallback: &str) -> String {
        cookie(response).unwrap_or_else(|| fallback.to_owned())
    }

    #[tokio::test]
    async fn clear_history_is_carried_to_the_next_page() {
        let app = session_app();
        let response = send(app.clone(), method_request("POST", "/logout", true)).await;
        let cookie = cookie(&response).unwrap();

        let page = body_json(
            send(
                app.clone(),
                with_cookie(method_request("GET", "/users", true), &cookie),
            )
            .await,
        )
        .await;
        assert_eq!(page["clearHistory"], true);
        let page = body_json(
            send(
                app,
                with_cookie(method_request("GET", "/users", true), &cookie),
            )
            .await,
        )
        .await;
        assert!(page.get("clearHistory").is_none());
    }

    #[tokio::test]
    async fn rendered_pages_merge_request_and_response_flash() {
        let page =
            body_json(send(session_app(), method_request("GET", "/inline", true)).await).await;
        assert_eq!(page["flash"], json!({ "inline": true, "highlight": 3 }));
    }

    #[tokio::test]
    async fn sessions_are_isolated_per_user() {
        let app = session_app();
        let response = send(app.clone(), method_request("POST", "/users", true)).await;
        assert!(cookie(&response).is_some());

        let page = body_json(send(app, method_request("GET", "/users", true)).await).await;
        assert!(
            page.get("flash").is_none(),
            "another session never sees the flash"
        );
    }
}

#[cfg(all(feature = "experimental-layers", feature = "tower-sessions"))]
mod layers {
    use super::sessions::{cookie, with_cookie};
    use super::*;
    use ::axum::routing::post;
    use tower_sessions::{MemoryStore, SessionManagerLayer};

    async fn create(request: InertiaRequest) -> Result<Response, InertiaError> {
        request
            .respond(
                Inertia::response("Users/Create", json!({ "roles": [] }))
                    .layer(Some("http://app.test/users".into()), None),
            )
            .await
    }

    async fn close(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.flash("message", "Saved")?;
        request.close()
    }

    async fn confirm(request: InertiaRequest) -> Result<Response, InertiaError> {
        request.interstitial();
        request.redirect(Inertia::redirect("/users/create"))
    }

    fn layer_app() -> Router {
        Router::new()
            .route("/users/create", get(create))
            .route("/users/close", post(close))
            .route("/confirm", get(confirm))
            .layer(
                InertiaLayer::new("1")
                    .template(template)
                    .session(TowerSessions::new()),
            )
            .layer(SessionManagerLayer::new(MemoryStore::default()).with_secure(false))
    }

    #[tokio::test]
    async fn layer_base_is_host_relative() {
        let mut request = method_request("GET", "/users/create", true);
        request
            .headers_mut()
            .insert(HOST, HeaderValue::from_static("app.test"));
        let page = body_json(send(layer_app(), request).await).await;

        assert_eq!(page["layer"], json!({ "base": "/users" }));
    }

    #[tokio::test]
    async fn close_responses_keep_flash_for_the_refresh_beneath() {
        let app = layer_app();
        let response = send(app.clone(), method_request("POST", "/users/close", true)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = cookie(&response);
        assert_eq!(
            body_json(response).await,
            json!({ "component": "", "props": {}, "url": "/users/close", "version": "1", "close": true })
        );

        // The close page did not consume the flash...
        let page = body_json(
            send(
                app,
                with_cookie(
                    method_request("GET", "/users/create", true),
                    &cookie.unwrap(),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(page["flash"], json!({ "message": "Saved" }));

        // ...and non-Inertia closes redirect back.
        let response = send(layer_app(), method_request("POST", "/users/close", false)).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn interstitial_flag_is_carried_across_a_redirect_once() {
        let app = layer_app();
        let response = send(app.clone(), method_request("GET", "/confirm", true)).await;
        assert_eq!(response.status(), StatusCode::FOUND);
        let cookie = cookie(&response).unwrap();

        let page = body_json(
            send(
                app.clone(),
                with_cookie(method_request("GET", "/users/create", true), &cookie),
            )
            .await,
        )
        .await;
        assert_eq!(page["interstitial"], true);
        let page = body_json(
            send(
                app,
                with_cookie(method_request("GET", "/users/create", true), &cookie),
            )
            .await,
        )
        .await;
        assert!(page.get("interstitial").is_none());
    }
}
