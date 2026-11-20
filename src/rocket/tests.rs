use super::*;
use crate::ssr::{SsrGateway, SsrResponse};
use crate::{
    InertiaProps, Props, ScrollProps, X_INERTIA_EXCEPT_ONCE_PROPS,
    X_INERTIA_INFINITE_SCROLL_MERGE_INTENT, X_INERTIA_PARTIAL_COMPONENT, X_INERTIA_PARTIAL_DATA,
    X_INERTIA_PARTIAL_EXCEPT, X_INERTIA_RESET,
};
use futures_util::future::BoxFuture;
use rocket::http::{Header, Status};
use rocket::local::blocking::{Client, LocalResponse};
use rocket::{delete, patch, post, put};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

const CURRENT_VERSION: &str = "1";

#[get("/foo")]
fn foo() -> Inertia<Value> {
    Inertia::response("foo", json!({ "n": 42 }))
}

#[get("/route-auth")]
fn route_auth() -> Inertia<Value> {
    Inertia::response(
        "route-auth",
        json!({ "auth": { "user": { "name": "Route" } } }),
    )
}

#[get("/url")]
fn url_override() -> Inertia<Value> {
    Inertia::response("foo", json!({ "n": 42 })).with_url("/custom-url")
}

#[get("/unsafe")]
fn unsafe_props() -> Inertia<Value> {
    Inertia::response(
        "unsafe",
        json!({ "text": "</script><script>alert(1)</script>&\u{2028}\u{2029}" }),
    )
}

#[get("/advanced")]
fn advanced() -> Inertia<Value> {
    Inertia::response(
        "advanced",
        json!({ "users": ["Ada"], "stats": 42, "plans": ["basic"], "notifications": ["hi"] }),
    )
    .encrypt_history()
    .clear_history()
    .preserve_fragment()
    .always("users")
    .merge("users")
    .prepend("notifications")
    .defer("stats")
    .once("plans")
}

#[get("/lazy")]
fn lazy() -> Inertia<InertiaProps> {
    Inertia::response(
        "lazy",
        InertiaProps::new()
            .value("users", json!(["Ada", "Grace"]))
            .defer("stats", || 42)
            .optional("audit", || json!(["created"])),
    )
}

#[get("/async")]
async fn async_page(inertia: InertiaRequest) -> Result<InertiaPage, InertiaError> {
    inertia
        .render(Inertia::response(
            "async",
            Props::new()
                .lazy_async("users", || async { vec!["Ada"] })
                .defer_async("stats", || async { 7 }),
        ))
        .await
}

#[get("/async-in-sync")]
fn async_in_sync() -> Inertia<InertiaProps> {
    Inertia::response("x", InertiaProps::new().lazy_async("users", || async { 1 }))
}

#[get("/scrolling")]
fn scrolling() -> Inertia<Value> {
    Inertia::page("scrolling")
        .scroll("posts", ScrollProps::new("page", 1).next_page(2))
        .props(json!({ "posts": { "data": [1, 2] } }))
}

#[rocket::post("/write")]
fn write() -> Inertia<Value> {
    Inertia::response("write", json!({ "users": ["Ada"], "plans": ["basic"] })).once("plans")
}

#[get("/external")]
fn external() -> Location {
    Inertia::location("https://example.com/outside")
}

#[post("/external")]
fn external_post() -> Location {
    Inertia::location("https://example.com/outside")
}

#[get("/external-fragment")]
fn external_fragment() -> Location {
    Inertia::location("/outside#fragment")
}

#[get("/bad-external")]
fn bad_external() -> Location {
    Inertia::location("/bad\nlocation")
}

#[get("/go")]
fn get_redirect() -> Redirect {
    Inertia::redirect("/target")
}

#[get("/bad-go")]
fn bad_redirect() -> Redirect {
    Inertia::redirect("/bad\nlocation")
}

#[post("/go")]
fn post_redirect() -> Redirect {
    Inertia::redirect("/target")
}

#[put("/go")]
fn put_redirect() -> Redirect {
    Inertia::redirect("/target")
}

#[patch("/go")]
fn patch_redirect() -> Redirect {
    Inertia::redirect("/target")
}

#[delete("/go")]
fn delete_redirect() -> Redirect {
    Inertia::redirect("/target")
}

#[get("/fragment")]
fn fragment_redirect() -> Redirect {
    Inertia::redirect("/target#section")
}

#[put("/raw")]
fn raw_found() -> RocketRedirect {
    RocketRedirect::found("/raw-target")
}

#[put("/empty")]
fn empty_put() -> &'static str {
    ""
}

#[get("/plain")]
fn plain() -> &'static str {
    "plain"
}

#[get("/headers")]
fn headers(headers: InertiaHeaders) -> String {
    format!(
        "{}:{}:{}",
        headers.is_inertia(),
        headers.version().unwrap_or("none"),
        headers.context().partial_data().join("|")
    )
}

fn html_responder(
    request: &Request<'_>,
    context: &HtmlResponseContext,
) -> response::Result<'static> {
    rocket::response::content::RawHtml(format!(
        "<html><head>{}</head><body>{}</body></html>",
        context.head(),
        context.body()
    ))
    .respond_to(request)
}

fn rocket() -> rocket::Rocket<rocket::Build> {
    rocket::build()
        .mount(
            "/",
            routes![
                foo,
                route_auth,
                url_override,
                unsafe_props,
                advanced,
                lazy,
                async_page,
                async_in_sync,
                scrolling,
                write,
                external,
                external_post,
                external_fragment,
                bad_external,
                get_redirect,
                bad_redirect,
                post_redirect,
                put_redirect,
                patch_redirect,
                delete_redirect,
                fragment_redirect,
                raw_found,
                empty_put,
                plain,
                headers
            ],
        )
        .attach(VersionFairing::new(CURRENT_VERSION, html_responder))
}

fn client() -> Client {
    Client::tracked(rocket()).unwrap()
}

fn inertia_get<'c>(
    client: &'c Client,
    uri: &'static str,
) -> rocket::local::blocking::LocalRequest<'c> {
    client
        .get(uri)
        .header(Header::new(X_INERTIA, "true"))
        .header(Header::new(X_INERTIA_VERSION, CURRENT_VERSION))
}

fn page(response: LocalResponse<'_>) -> Value {
    serde_json::from_str(&response.into_string().unwrap()).unwrap()
}

#[test]
fn html_response_uses_the_registered_responder() {
    let client = client();
    let response = client.get("/foo?bar=baz").dispatch();

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.headers().get_one(VARY), Some(X_INERTIA));
    assert_eq!(response.headers().get_one(X_INERTIA), None);
    let body = response.into_string().unwrap();
    assert!(body.contains(r#"<script data-page="app" type="application/json">"#));
    assert!(body.contains(r#""url":"/foo?bar=baz""#));
    assert!(body.contains(r#""version":"1""#));
}

#[test]
fn html_response_escapes_json_for_script_context() {
    let body = client().get("/unsafe").dispatch().into_string().unwrap();

    assert!(!body.contains("</script><script>"));
    assert!(body.contains("\\u003C/script\\u003E"));
    assert!(body.contains("\\u2028\\u2029"));
}

#[test]
fn json_response_has_protocol_headers() {
    let client = client();
    let response = inertia_get(&client, "/foo?bar=baz").dispatch();

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.headers().get_one("Content-Type"),
        Some("application/json")
    );
    assert_eq!(response.headers().get_one(X_INERTIA), Some("true"));
    assert_eq!(response.headers().get(VARY).count(), 1);
    let page = page(response);
    assert_eq!(page["url"], "/foo?bar=baz");
    assert_eq!(page["version"], CURRENT_VERSION);
    assert_eq!(page["props"], json!({ "n": 42, "errors": {} }));
}

#[test]
fn with_url_overrides_request_uri() {
    let client = client();
    assert_eq!(
        page(inertia_get(&client, "/url").dispatch())["url"],
        "/custom-url"
    );
}

#[test]
fn responses_without_the_fairing_have_an_empty_version() {
    let client = Client::tracked(rocket::build().mount("/", routes![foo])).unwrap();
    let response = client
        .get("/foo")
        .header(Header::new(X_INERTIA, "true"))
        .dispatch();

    assert_eq!(page(response)["version"], "");
}

#[test]
fn version_conflicts_report_location_and_server_version() {
    let client = client();

    for version in [Some("0"), None] {
        let mut request = client
            .get("/foo?bar=baz")
            .header(Header::new(X_INERTIA, "true"));
        if let Some(version) = version {
            request = request.header(Header::new(X_INERTIA_VERSION, version));
        }
        let response = request.dispatch();

        assert_eq!(response.status(), Status::Conflict);
        assert_eq!(
            response.headers().get_one(X_INERTIA_LOCATION),
            Some("/foo?bar=baz")
        );
        assert_eq!(
            response.headers().get_one(X_INERTIA_VERSION),
            Some(CURRENT_VERSION)
        );
        assert_eq!(response.headers().get_one(VARY), Some(X_INERTIA));
    }
}

#[test]
fn dynamic_versions_are_used_for_pages_and_conflicts() {
    let version = Arc::new(AtomicUsize::new(1));
    let rocket = rocket::build()
        .mount("/", routes![foo])
        .attach(VersionFairing::dynamic(
            {
                let version = version.clone();
                move || format!("dynamic-{}", version.load(Ordering::SeqCst))
            },
            html_responder,
        ));
    let client = Client::tracked(rocket).unwrap();
    let request = || {
        client
            .get("/foo")
            .header(Header::new(X_INERTIA, "true"))
            .header(Header::new(X_INERTIA_VERSION, "dynamic-1"))
    };

    assert_eq!(page(request().dispatch())["version"], "dynamic-1");
    version.store(2, Ordering::SeqCst);
    assert_eq!(request().dispatch().status(), Status::Conflict);
}

#[test]
fn version_conflict_route_rejects_non_local_locations() {
    let client = client();

    for location in [
        "https://example.com",
        "//example.com",
        "/\\example.com",
        "/a%0Ab",
    ] {
        let response = client
            .get(format!("/inertia-rs/version-conflict?location={location}"))
            .dispatch();
        assert_eq!(response.status(), Status::BadRequest, "{location}");
    }
}

#[test]
fn advanced_builder_metadata_is_resolved() {
    let client = client();
    let page = page(inertia_get(&client, "/advanced").dispatch());

    assert_eq!(page["encryptHistory"], true);
    assert_eq!(page["clearHistory"], true);
    assert_eq!(page["preserveFragment"], true);
    assert_eq!(page["mergeProps"], json!(["users"]));
    assert_eq!(page["prependProps"], json!(["notifications"]));
    assert_eq!(page["deferredProps"], json!({ "default": ["stats"] }));
    assert_eq!(page["onceProps"]["plans"]["prop"], "plans");
    assert!(page["props"].get("stats").is_none());
}

#[test]
fn partial_reloads_filter_props_and_compose_only_and_except() {
    let client = client();
    let page = page(
        inertia_get(&client, "/lazy")
            .header(Header::new(X_INERTIA_PARTIAL_COMPONENT, "lazy"))
            .header(Header::new(X_INERTIA_PARTIAL_DATA, "stats,audit,users"))
            .header(Header::new(X_INERTIA_PARTIAL_EXCEPT, "users"))
            .dispatch(),
    );

    assert_eq!(
        page["props"],
        json!({ "stats": 42, "audit": ["created"], "errors": {} })
    );

    let mismatch = super::tests::page(
        inertia_get(&client, "/lazy")
            .header(Header::new(X_INERTIA_PARTIAL_COMPONENT, "other"))
            .header(Header::new(X_INERTIA_PARTIAL_DATA, "audit"))
            .dispatch(),
    );
    assert_eq!(
        mismatch["props"],
        json!({ "users": ["Ada", "Grace"], "errors": {} })
    );
}

#[test]
fn partial_reloads_apply_to_post_requests_and_once_exclusions_hold() {
    let client = client();
    let page = page(
        client
            .post("/write")
            .header(Header::new(X_INERTIA, "true"))
            .header(Header::new(X_INERTIA_EXCEPT_ONCE_PROPS, "plans"))
            .dispatch(),
    );

    assert_eq!(page["props"], json!({ "users": ["Ada"], "errors": {} }));
    assert_eq!(page["onceProps"]["plans"]["prop"], "plans");
}

#[test]
fn scroll_reset_and_prepend_intent() {
    let client = client();
    let reset = page(
        inertia_get(&client, "/scrolling")
            .header(Header::new(X_INERTIA_PARTIAL_COMPONENT, "scrolling"))
            .header(Header::new(X_INERTIA_PARTIAL_DATA, "posts"))
            .header(Header::new(X_INERTIA_RESET, "posts"))
            .dispatch(),
    );
    assert_eq!(reset["scrollProps"]["posts"]["reset"], true);
    assert!(reset.get("mergeProps").is_none());

    let prepend = page(
        inertia_get(&client, "/scrolling")
            .header(Header::new(X_INERTIA_PARTIAL_COMPONENT, "scrolling"))
            .header(Header::new(X_INERTIA_PARTIAL_DATA, "posts"))
            .header(Header::new(
                X_INERTIA_INFINITE_SCROLL_MERGE_INTENT,
                "prepend",
            ))
            .dispatch(),
    );
    assert_eq!(prepend["prependProps"], json!(["posts.data"]));
}

#[test]
fn async_providers_render_through_the_guard() {
    let client = client();
    let page = page(inertia_get(&client, "/async").dispatch());

    assert_eq!(page["props"], json!({ "users": ["Ada"], "errors": {} }));
    assert_eq!(page["deferredProps"], json!({ "default": ["stats"] }));

    let html = client.get("/async").dispatch().into_string().unwrap();
    assert!(html.contains(r#""users":["Ada"]"#));
}

#[test]
fn async_providers_in_sync_responders_are_server_errors() {
    let client = client();
    assert_eq!(
        inertia_get(&client, "/async-in-sync").dispatch().status(),
        Status::InternalServerError
    );
}

fn rocket_with_shared_props() -> rocket::Rocket<rocket::Build> {
    rocket().manage(
        SharedProps::new()
            .value("appName", "Demo")
            .value("n", 99)
            .value("auth.user", json!({ "name": "Shared" }))
            .prop("csrfToken", |request| {
                request.headers().get_one("X-CSRF").map(ToOwned::to_owned)
            })
            .prop_optional("missing", |_| Option::<String>::None)
            .typed("countries", |_| Prop::lazy(|| ["NL"]).once()),
    )
}

#[test]
fn shared_props_resolve_with_route_props() {
    let client = Client::tracked(rocket_with_shared_props()).unwrap();
    let page = page(
        inertia_get(&client, "/foo")
            .header(Header::new("X-CSRF", "token"))
            .dispatch(),
    );

    assert_eq!(
        page["props"],
        json!({
            "errors": {},
            "appName": "Demo",
            "n": 42,
            "auth": { "user": { "name": "Shared" } },
            "csrfToken": "token",
            "countries": ["NL"]
        })
    );
    assert_eq!(
        page["sharedProps"],
        json!([
            "errors",
            "appName",
            "n",
            "auth",
            "csrfToken",
            "missing",
            "countries"
        ])
    );
}

#[test]
fn shared_props_are_filtered_by_partial_reloads() {
    let client = Client::tracked(rocket_with_shared_props()).unwrap();
    let page = page(
        inertia_get(&client, "/foo")
            .header(Header::new(X_INERTIA_PARTIAL_COMPONENT, "foo"))
            .header(Header::new(X_INERTIA_PARTIAL_DATA, "n"))
            .dispatch(),
    );

    assert_eq!(page["props"], json!({ "n": 42, "errors": {} }));
}

#[test]
fn shared_dotted_props_do_not_replace_route_owned_roots() {
    let client = Client::tracked(rocket_with_shared_props()).unwrap();
    let page = page(inertia_get(&client, "/route-auth").dispatch());

    assert_eq!(
        page["props"]["auth"],
        json!({ "user": { "name": "Route" } })
    );
}

#[test]
fn locations_use_x_inertia_location_for_inertia_requests() {
    let client = client();

    for uri in ["/external", "/external-fragment"] {
        let response = inertia_get(&client, uri).dispatch();
        assert_eq!(response.status(), Status::Conflict);
        assert!(response.headers().get_one(X_INERTIA_LOCATION).is_some());
        assert!(response.headers().get_one(X_INERTIA_REDIRECT).is_none());
    }

    assert_eq!(client.get("/external").dispatch().status(), Status::Found);
    assert_eq!(
        client.post("/external").dispatch().status(),
        Status::SeeOther
    );
    assert_eq!(
        inertia_get(&client, "/bad-external").dispatch().status(),
        Status::InternalServerError
    );
}

#[test]
fn redirects_are_method_aware() {
    let client = client();

    assert_eq!(client.get("/go").dispatch().status(), Status::Found);
    assert_eq!(client.post("/go").dispatch().status(), Status::SeeOther);
    assert_eq!(client.put("/go").dispatch().status(), Status::SeeOther);
    assert_eq!(client.patch("/go").dispatch().status(), Status::SeeOther);
    assert_eq!(client.delete("/go").dispatch().status(), Status::SeeOther);
    assert_eq!(
        client.get("/bad-go").dispatch().status(),
        Status::InternalServerError
    );
}

#[test]
fn fairing_converts_found_after_put_for_inertia_requests() {
    let client = client();
    let response = client
        .put("/raw")
        .header(Header::new(X_INERTIA, "true"))
        .dispatch();
    assert_eq!(response.status(), Status::SeeOther);

    assert_eq!(client.put("/raw").dispatch().status(), Status::Found);
}

#[test]
fn fairing_turns_fragment_redirects_into_inertia_redirects() {
    let client = client();
    let response = inertia_get(&client, "/fragment").dispatch();
    assert_eq!(response.status(), Status::Conflict);
    assert_eq!(
        response.headers().get_one(X_INERTIA_REDIRECT),
        Some("/target#section")
    );
    assert!(response.headers().get_one("Location").is_none());

    let prefetch = inertia_get(&client, "/fragment")
        .header(Header::new(PURPOSE, "prefetch"))
        .dispatch();
    assert_eq!(prefetch.status(), Status::Found);
    assert_eq!(client.get("/fragment").dispatch().status(), Status::Found);
}

#[test]
fn fairing_redirects_back_on_empty_inertia_responses() {
    let client = client();
    let response = client
        .put("/empty")
        .header(Header::new(X_INERTIA, "true"))
        .header(Header::new("Referer", "/form"))
        .dispatch();

    assert_eq!(response.status(), Status::SeeOther);
    assert_eq!(response.headers().get_one("Location"), Some("/form"));
}

#[test]
fn every_response_varies_on_x_inertia() {
    let client = client();
    assert_eq!(
        client.get("/plain").dispatch().headers().get_one(VARY),
        Some(X_INERTIA)
    );
    assert_eq!(
        client.get("/missing").dispatch().headers().get_one(VARY),
        Some(X_INERTIA)
    );
}

#[test]
fn inertia_headers_guard_reads_headers() {
    let client = client();
    let body = inertia_get(&client, "/headers")
        .header(Header::new(X_INERTIA_PARTIAL_DATA, "a, b"))
        .dispatch()
        .into_string()
        .unwrap();
    assert_eq!(body, "true:1:a|b");

    let body = client.get("/headers").dispatch().into_string().unwrap();
    assert_eq!(body, "false:none:");
}

struct EchoSsr;

impl SsrGateway for EchoSsr {
    fn render<'a>(&'a self, page: &'a Value) -> BoxFuture<'a, Result<SsrResponse, SsrError>> {
        let component = page["component"].as_str().unwrap_or_default().to_owned();
        Box::pin(async move {
            Ok(SsrResponse::new(
                vec![format!("<title>{component}</title>")],
                format!(r#"<div data-server-rendered="true" id="app">{component}</div>"#),
            ))
        })
    }
}

#[test]
fn ssr_applies_to_first_visits_through_the_guard() {
    let client = Client::tracked(rocket().manage(Ssr::new(EchoSsr))).unwrap();
    let html = client.get("/async").dispatch().into_string().unwrap();

    assert!(html.contains("<head><title>async</title></head>"));
    assert!(html.contains(r#"<div data-server-rendered="true" id="app">async</div>"#));

    let response = inertia_get(&client, "/async").dispatch();
    assert_eq!(page(response)["component"], "async");
}
