//! Framework-neutral protocol tests.
//!
//! Expectations follow the protocol specification and the reference adapter
//! (`inertia-laravel` v3.7.0 `PropsResolver`); see
//! `docs/compatibility-target.md`.

use super::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn request(headers: &[(&str, &str)]) -> RequestContext {
    let headers = headers.iter().copied().collect::<HashMap<_, _>>();
    RequestContext::from_header_fn(|name| headers.get(name).copied())
}

fn inertia() -> Vec<(&'static str, &'static str)> {
    vec![(X_INERTIA, "true")]
}

fn partial(component: &'static str, only: &'static str) -> Vec<(&'static str, &'static str)> {
    let mut headers = inertia();
    headers.push((X_INERTIA_PARTIAL_COMPONENT, component));
    if !only.is_empty() {
        headers.push((X_INERTIA_PARTIAL_DATA, only));
    }
    headers
}

fn render<'a, T: IntoPageProps<'a>>(response: Inertia<T>, headers: &[(&str, &str)]) -> Value {
    let request = request(headers);
    serde_json::to_value(response.into_page("/page", Some("v1".into()), &request).unwrap()).unwrap()
}

async fn render_async<'a, T: IntoPageProps<'a>>(
    response: Inertia<T>,
    headers: &[(&str, &str)],
    shared: Vec<(&str, Prop<'a>)>,
) -> Result<Value, Error> {
    let request = request(headers);
    let context = RenderContext::new(&request, "/page")
        .version(Some("v1".into()))
        .share_all(shared);
    response
        .resolve(context)
        .await
        .map(|page| serde_json::to_value(page).unwrap())
}

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

fn counted(calls: &Arc<AtomicUsize>, value: Value) -> impl FnOnce() -> Value + Send + 'static {
    let calls = calls.clone();
    move || {
        calls.fetch_add(1, Ordering::SeqCst);
        value
    }
}

#[test]
fn request_context_parses_inertia_headers() {
    let context = request(&[
        (X_INERTIA, "true"),
        (X_INERTIA_VERSION, "abc"),
        (X_INERTIA_PARTIAL_COMPONENT, "Users/Index"),
        (X_INERTIA_PARTIAL_DATA, "users, stats"),
        (X_INERTIA_PARTIAL_EXCEPT, "audit"),
        (X_INERTIA_RESET, "users"),
        (X_INERTIA_ERROR_BAG, "createUser"),
        (X_INERTIA_INFINITE_SCROLL_MERGE_INTENT, "append"),
        (X_INERTIA_EXCEPT_ONCE_PROPS, "plans,features"),
        (PURPOSE, "prefetch"),
        (CACHE_CONTROL, "max-age=0, no-cache"),
        (X_SOCKET_ID, "123.456"),
        (PRECOGNITION, "true"),
    ]);

    assert!(context.is_inertia());
    assert_eq!(context.version(), Some("abc"));
    assert_eq!(context.partial_component(), Some("Users/Index"));
    assert_eq!(context.partial_data(), ["users", "stats"]);
    assert_eq!(context.partial_except(), ["audit"]);
    assert_eq!(context.reset(), ["users"]);
    assert_eq!(context.error_bag(), Some("createUser"));
    assert_eq!(context.infinite_scroll_merge_intent(), Some("append"));
    assert_eq!(context.except_once_props(), ["plans", "features"]);
    assert!(context.is_prefetch());
    assert!(context.is_reload());
    assert_eq!(context.socket_id(), Some("123.456"));
    assert!(context.is_precognitive());
    assert!(context.partial_reload_matches("Users/Index"));
}

#[test]
fn minimal_page_always_has_errors_and_version() {
    let page = render(Inertia::response("Home", json!({ "name": "Ada" })), &[]);

    assert_eq!(
        page,
        json!({
            "component": "Home",
            "props": { "name": "Ada", "errors": {} },
            "url": "/page",
            "version": "v1"
        })
    );

    let request = request(&[]);
    let unversioned = Inertia::response("Home", ())
        .into_page("/", None, &request)
        .unwrap();
    assert_eq!(serde_json::to_value(unversioned).unwrap()["version"], "");
}

#[test]
fn non_object_props_are_rejected() {
    let request = request(&[]);
    let error = Inertia::response("Home", vec![1, 2])
        .into_page("/", None, &request)
        .unwrap_err();

    assert!(matches!(error, Error::InvalidProps));
}

#[test]
fn page_from_parts_serializes_every_metadata_field() {
    let metadata = PageMetadata::new()
        .encrypt_history()
        .clear_history()
        .preserve_fragment()
        .preserve_big_integers()
        .merge("posts")
        .prepend("notifications")
        .deep_merge("conversations")
        .match_on("posts.id")
        .defer_group("sidebar", "related")
        .rescue("permissions")
        .share("auth")
        .once_with_key("plans", OnceProp::new("plans").expires_at(1_700_000_000_000));
    let page = Page::from_parts("Feed", json!({}), "/feed", Some("v".into()), metadata);

    assert_eq!(
        serde_json::to_value(page).unwrap(),
        json!({
            "component": "Feed",
            "props": {},
            "url": "/feed",
            "version": "v",
            "sharedProps": ["auth"],
            "mergeProps": ["posts"],
            "prependProps": ["notifications"],
            "deepMergeProps": ["conversations"],
            "matchPropsOn": ["posts.id"],
            "deferredProps": { "sidebar": ["related"] },
            "rescuedProps": ["permissions"],
            "onceProps": { "plans": { "prop": "plans", "expiresAt": 1_700_000_000_000u64 } },
            "preserveBigIntegers": true,
            "clearHistory": true,
            "encryptHistory": true,
            "preserveFragment": true
        })
    );
}

#[test]
fn partial_data_filters_matching_component_only() {
    let props = || json!({ "users": [1], "stats": 2, "audit": 3 });

    let page = render(Inertia::response("Users", props()), &partial("Users", "users"));
    assert_eq!(page["props"], json!({ "users": [1], "errors": {} }));

    let other = render(Inertia::response("Login", props()), &partial("Users", "users"));
    assert_eq!(other["props"], json!({ "users": [1], "stats": 2, "audit": 3, "errors": {} }));
}

#[test]
fn partial_data_and_except_compose_by_narrowing_then_removing() {
    // Spec: "With both headers, the server first narrows the response to the
    // X-Inertia-Partial-Data list, then removes the X-Inertia-Partial-Except
    // list. A prop named in both headers is left out."
    let mut headers = partial("Users", "users,stats");
    headers.push((X_INERTIA_PARTIAL_EXCEPT, "stats,audit"));
    let page = render(
        Inertia::response("Users", json!({ "users": 1, "stats": 2, "audit": 3, "other": 4 })),
        &headers,
    );

    assert_eq!(page["props"], json!({ "users": 1, "errors": {} }));
}

#[test]
fn partial_except_alone_returns_everything_else() {
    let mut headers = partial("Users", "");
    headers.push((X_INERTIA_PARTIAL_EXCEPT, "audit"));
    let page = render(
        Inertia::response("Users", json!({ "users": 1, "audit": 3 })),
        &headers,
    );

    assert_eq!(page["props"], json!({ "users": 1, "errors": {} }));
}

#[test]
fn dot_notation_paths_filter_inside_plain_values() {
    let props = || {
        json!({
            "auth": { "user": { "name": "Ada" }, "permissions": ["edit"] },
            "other": true
        })
    };

    let only = render(Inertia::response("P", props()), &partial("P", "auth.user"));
    assert_eq!(
        only["props"],
        json!({ "auth": { "user": { "name": "Ada" } }, "errors": {} })
    );

    let mut headers = partial("P", "");
    headers.push((X_INERTIA_PARTIAL_EXCEPT, "auth.permissions"));
    let except = render(Inertia::response("P", props()), &headers);
    assert_eq!(
        except["props"],
        json!({ "auth": { "user": { "name": "Ada" } }, "other": true, "errors": {} })
    );
}

#[test]
fn values_resolved_by_providers_are_returned_whole() {
    // The reference adapter "returns a prop that a closure resolves in full".
    let page = render(
        Inertia::response(
            "P",
            Props::new().lazy("auth", || json!({ "user": "Ada", "permissions": ["edit"] })),
        ),
        &partial("P", "auth.user"),
    );

    assert_eq!(
        page["props"]["auth"],
        json!({ "user": "Ada", "permissions": ["edit"] })
    );
}

#[test]
fn dotted_keys_build_nested_props_with_typed_children() {
    let calls = counter();
    let response = || {
        Inertia::response(
            "P",
            Props::new()
                .value("auth.user", "Ada")
                .prop(
                    "auth.permissions",
                    Prop::lazy(counted(&calls, json!(["edit"]))).defer(),
                ),
        )
    };

    let full = render(response(), &inertia());
    assert_eq!(full["props"], json!({ "auth": { "user": "Ada" }, "errors": {} }));
    assert_eq!(full["deferredProps"], json!({ "default": ["auth.permissions"] }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let deferred = render(response(), &partial("P", "auth.permissions"));
    assert_eq!(
        deferred["props"],
        json!({ "auth": { "permissions": ["edit"] }, "errors": {} })
    );
    assert!(deferred.get("deferredProps").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn excluded_lazy_props_never_run() {
    let users = counter();
    let stats = counter();
    let response = || {
        Inertia::response(
            "Dashboard",
            Props::new()
                .lazy("users", counted(&users, json!(["Ada"])))
                .lazy("stats", counted(&stats, json!(7))),
        )
    };

    let page = render(response(), &partial("Dashboard", "users"));
    assert_eq!(page["props"], json!({ "users": ["Ada"], "errors": {} }));
    assert_eq!(users.load(Ordering::SeqCst), 1);
    assert_eq!(stats.load(Ordering::SeqCst), 0);

    render(response(), &[]);
    assert_eq!(users.load(Ordering::SeqCst), 2);
    assert_eq!(stats.load(Ordering::SeqCst), 1);
}

#[test]
fn optional_props_follow_reference_partial_rules() {
    let calls = counter();
    let response = || {
        Inertia::response(
            "Dashboard",
            Props::new()
                .value("users", 1)
                .optional("audit", counted(&calls, json!(["created"]))),
        )
    };

    let full = render(response(), &inertia());
    assert!(full["props"].get("audit").is_none());
    assert!(full.get("deferredProps").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let other = render(response(), &partial("Dashboard", "users"));
    assert!(other["props"].get("audit").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let only = render(response(), &partial("Dashboard", "audit"));
    assert_eq!(only["props"]["audit"], json!(["created"]));

    // Selected by `except`: the reference resolves any prop the partial
    // filter keeps.
    let mut headers = partial("Dashboard", "");
    headers.push((X_INERTIA_PARTIAL_EXCEPT, "users"));
    let except = render(response(), &headers);
    assert_eq!(except["props"], json!({ "audit": ["created"], "errors": {} }));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn deferred_props_are_announced_on_full_visits_only() {
    let calls = counter();
    let response = || {
        Inertia::response(
            "Posts",
            Props::new()
                .value("user", "Ada")
                .prop("comments", Prop::lazy(counted(&calls, json!([1]))).defer())
                .prop("related", Prop::lazy(|| json!([2])).defer_group("sidebar")),
        )
    };

    let full = render(response(), &inertia());
    assert_eq!(full["props"], json!({ "user": "Ada", "errors": {} }));
    assert_eq!(
        full["deferredProps"],
        json!({ "default": ["comments"], "sidebar": ["related"] })
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let group = render(response(), &partial("Posts", "comments"));
    assert_eq!(group["props"], json!({ "comments": [1], "errors": {} }));
    assert!(group.get("deferredProps").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn deferred_merge_props_emit_merge_metadata_on_the_full_visit() {
    let page = render(
        Inertia::response(
            "Feed",
            Props::new().prop("posts", Prop::lazy(|| json!([1])).defer().merge().match_on("id")),
        ),
        &inertia(),
    );

    assert!(page["props"].get("posts").is_none());
    assert_eq!(page["deferredProps"], json!({ "default": ["posts"] }));
    assert_eq!(page["mergeProps"], json!(["posts"]));
    assert_eq!(page["matchPropsOn"], json!(["posts.id"]));
}

#[test]
fn always_props_ignore_partial_filters_and_errors_are_always_included() {
    let mut headers = partial("P", "users");
    headers.push((X_INERTIA_PARTIAL_EXCEPT, "flags"));
    let page = render(
        Inertia::response(
            "P",
            Props::new()
                .value("users", 1)
                .value("stats", 2)
                .always("flags", || json!({ "beta": true }))
                .value("errors", json!({ "name": "Required" })),
        ),
        &headers,
    );

    assert_eq!(
        page["props"],
        json!({ "users": 1, "flags": { "beta": true }, "errors": { "name": "Required" } })
    );
}

#[test]
fn once_props_skip_resolution_when_the_client_has_them() {
    let calls = counter();
    let response = || {
        Inertia::response(
            "Billing",
            Props::new().once("plans", counted(&calls, json!(["basic"]))),
        )
    };
    let mut loaded = inertia();
    loaded.push((X_INERTIA_EXCEPT_ONCE_PROPS, "plans"));

    let first = render(response(), &inertia());
    assert_eq!(first["props"]["plans"], json!(["basic"]));
    assert_eq!(
        first["onceProps"],
        json!({ "plans": { "prop": "plans", "expiresAt": null } })
    );

    let cached = render(response(), &loaded);
    assert!(cached["props"].get("plans").is_none());
    assert_eq!(cached["onceProps"]["plans"]["prop"], "plans");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Full-page (non-Inertia) loads always resolve once props.
    let html = render(response(), &[(X_INERTIA_EXCEPT_ONCE_PROPS, "plans")]);
    assert_eq!(html["props"]["plans"], json!(["basic"]));

    // Partial reloads ignore the except-once header.
    let mut reload = partial("Billing", "plans");
    reload.push((X_INERTIA_EXCEPT_ONCE_PROPS, "plans"));
    let partial_page = render(response(), &reload);
    assert_eq!(partial_page["props"]["plans"], json!(["basic"]));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn fresh_and_custom_key_once_props() {
    let mut loaded = inertia();
    loaded.push((X_INERTIA_EXCEPT_ONCE_PROPS, "billing-plans,countries"));

    let page = render(
        Inertia::response(
            "Billing",
            Props::new()
                .prop("plans", Prop::lazy(|| 1).once_as("billing-plans").expires_at(42))
                .prop("countries", Prop::lazy(|| 2).once().fresh()),
        ),
        &loaded,
    );

    assert!(page["props"].get("plans").is_none());
    assert_eq!(page["props"]["countries"], 2);
    assert_eq!(
        page["onceProps"],
        json!({
            "billing-plans": { "prop": "plans", "expiresAt": 42 },
            "countries": { "prop": "countries", "expiresAt": null }
        })
    );
}

#[test]
fn deferred_once_props_already_loaded_are_not_announced() {
    let mut loaded = inertia();
    loaded.push((X_INERTIA_EXCEPT_ONCE_PROPS, "permissions"));
    let page = render(
        Inertia::response("P", Props::new().defer_once("permissions", || 1)),
        &loaded,
    );

    assert!(page.get("deferredProps").is_none());
    assert_eq!(page["onceProps"]["permissions"]["prop"], "permissions");
}

#[test]
fn merge_paths_and_matching_keys_are_relative_to_the_prop() {
    let page = render(
        Inertia::response(
            "Feed",
            Props::new()
                .prop("posts", Prop::value(json!({ "data": [] })).append_at("data").match_on("data.id"))
                .prop("notifications", Prop::value(json!([])).prepend())
                .prop("conversations", Prop::value(json!({})).deep_merge()),
        ),
        &inertia(),
    );

    assert_eq!(page["mergeProps"], json!(["posts.data"]));
    assert_eq!(page["prependProps"], json!(["notifications"]));
    assert_eq!(page["deepMergeProps"], json!(["conversations"]));
    assert_eq!(page["matchPropsOn"], json!(["posts.data.id"]));
}

#[test]
fn reset_props_drop_merge_metadata() {
    let mut headers = partial("Feed", "posts");
    headers.push((X_INERTIA_RESET, "posts"));
    let page = render(
        Inertia::response("Feed", Props::new().prop("posts", Prop::value(json!([1])).merge())),
        &headers,
    );

    assert_eq!(page["props"]["posts"], json!([1]));
    assert!(page.get("mergeProps").is_none());
}

#[test]
fn partial_reloads_only_emit_metadata_for_requested_props() {
    let page = render(
        Inertia::response(
            "Feed",
            Props::new()
                .prop("posts", Prop::value(json!([1])).merge())
                .prop("events", Prop::value(json!([2])).merge()),
        ),
        &partial("Feed", "posts"),
    );

    assert_eq!(page["mergeProps"], json!(["posts"]));
}

#[test]
fn scroll_props_emit_pagination_reset_and_merge_intent() {
    let response = || {
        Inertia::response(
            "Posts",
            Props::new().prop(
                "posts",
                Prop::lazy(|| json!({ "data": [{ "id": 1 }] }))
                    .scroll(ScrollProps::paginated("page", 2, true)),
            ),
        )
    };

    let page = render(response(), &inertia());
    assert_eq!(page["mergeProps"], json!(["posts.data"]));
    assert_eq!(
        page["scrollProps"],
        json!({
            "posts": {
                "pageName": "page",
                "previousPage": 1,
                "nextPage": 3,
                "currentPage": 2,
                "reset": false
            }
        })
    );

    let mut prepend = partial("Posts", "posts");
    prepend.push((X_INERTIA_INFINITE_SCROLL_MERGE_INTENT, "prepend"));
    let page = render(response(), &prepend);
    assert_eq!(page["prependProps"], json!(["posts.data"]));
    assert!(page.get("mergeProps").is_none());

    let mut reset = partial("Posts", "posts");
    reset.push((X_INERTIA_RESET, "posts"));
    let page = render(response(), &reset);
    assert_eq!(page["scrollProps"]["posts"]["reset"], true);
    assert!(page.get("mergeProps").is_none());
}

#[test]
fn scroll_metadata_can_be_derived_from_the_resolved_value() {
    let page = render(
        Inertia::response(
            "Posts",
            Props::new().prop(
                "posts",
                Prop::lazy(|| json!({ "data": [], "next": "cursor-2" })).scroll_with(|value| {
                    ScrollProps::new("cursor", "cursor-1")
                        .next_page(value["next"].as_str().unwrap_or_default().to_owned())
                }),
            ),
        ),
        &inertia(),
    );

    assert_eq!(page["scrollProps"]["posts"]["nextPage"], "cursor-2");
    assert_eq!(page["scrollProps"]["posts"]["currentPage"], "cursor-1");
    assert_eq!(page["scrollProps"]["posts"]["previousPage"], Value::Null);
}

#[test]
fn deferred_scroll_props_omit_scroll_metadata_until_loaded() {
    let page = render(
        Inertia::response(
            "Posts",
            Props::new().prop(
                "posts",
                Prop::lazy(|| json!({ "data": [] }))
                    .scroll(ScrollProps::new("page", 1))
                    .defer(),
            ),
        ),
        &inertia(),
    );

    assert!(page.get("scrollProps").is_none());
    assert_eq!(page["deferredProps"], json!({ "default": ["posts"] }));
    assert_eq!(page["mergeProps"], json!(["posts.data"]));
}

#[test]
fn legacy_response_annotations_still_apply() {
    let page = render(
        Inertia::page("Builder")
            .merge("notifications")
            .defer("stats")
            .scroll("posts", ScrollProps::new("page", 1).next_page(2))
            .props(json!({
                "notifications": ["welcome"],
                "stats": 7,
                "posts": { "data": [] }
            })),
        &inertia(),
    );

    assert_eq!(page["props"]["notifications"], json!(["welcome"]));
    assert!(page["props"].get("stats").is_none());
    assert_eq!(page["deferredProps"], json!({ "default": ["stats"] }));
    assert_eq!(page["mergeProps"], json!(["notifications", "posts.data"]));
    assert_eq!(page["scrollProps"]["posts"]["nextPage"], 2);
}

#[test]
fn live_props_emit_listeners_and_survive_exclusion() {
    let live = || {
        LiveUpdates::on("OrderUpdated")
            .channel(LiveChannel::private("orders.1"))
            .throttle(5000)
    };
    let response = || {
        Inertia::response(
            "Orders/Show",
            Props::new()
                .prop("order", Prop::lazy(|| json!({ "status": "shipped" })).live(live()))
                .prop("metrics", Prop::lazy(|| 1).defer().live(LiveUpdates::on("M").channel("metrics")))
                .prop("audit", Prop::lazy(|| 2).optional().live(LiveUpdates::on("A").channel("audit")))
                .value("other", 3),
        )
    };

    let full = render(response(), &inertia());
    assert_eq!(
        full["liveProps"]["order"],
        json!({
            "listeners": [{ "channel": { "name": "orders.1", "type": "private" }, "events": ["OrderUpdated"] }],
            "throttle": 5000
        })
    );
    assert!(full["liveProps"].get("metrics").is_some());
    assert!(full["liveProps"].get("audit").is_some());

    let reload = render(response(), &partial("Orders/Show", "other"));
    assert!(reload.get("liveProps").is_none());

    let refresh = render(response(), &partial("Orders/Show", "order"));
    assert_eq!(refresh["props"]["order"]["status"], "shipped");
    assert_eq!(
        refresh["liveProps"].as_object().unwrap().keys().collect::<Vec<_>>(),
        ["order"]
    );
}

#[test]
fn unsupported_prop_combinations_are_rejected() {
    let request = request(&[]);
    let cases = vec![
        Prop::lazy(|| 1).merge().live(LiveUpdates::on("E").channel("c")),
        Prop::lazy(|| 1).scroll(ScrollProps::new("page", 1)).live(LiveUpdates::on("E").channel("c")),
        Prop::lazy(|| 1).always().defer(),
        Prop::lazy(|| 1).always().once(),
        Prop::lazy(|| 1).rescue(),
        Prop::lazy(|| 1).live(LiveUpdates::on("E")),
    ];

    for prop in cases {
        let error = Inertia::response("P", Props::new().prop("x", prop))
            .into_page("/", None, &request)
            .unwrap_err();
        assert!(matches!(error, Error::InvalidProp { ref path, .. } if path == "x"), "{error}");
    }
}

#[tokio::test]
async fn async_props_resolve_only_when_needed() {
    let calls = counter();
    let response = || {
        let calls = calls.clone();
        Inertia::response(
            "P",
            Props::new()
                .value("cheap", 1)
                .lazy_async("expensive", move || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    vec![1, 2, 3]
                }),
        )
    };

    let page = render_async(response(), &partial("P", "cheap"), vec![]).await.unwrap();
    assert_eq!(page["props"], json!({ "cheap": 1, "errors": {} }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let page = render_async(response(), &[], vec![]).await.unwrap();
    assert_eq!(page["props"]["expensive"], json!([1, 2, 3]));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn async_props_resolve_concurrently() {
    // Each provider waits for the other: sequential execution would deadlock.
    let (tx_a, rx_a) = tokio::sync::oneshot::channel::<u8>();
    let (tx_b, rx_b) = tokio::sync::oneshot::channel::<u8>();
    let response = Inertia::response(
        "P",
        Props::new()
            .lazy_async("a", move || async move {
                tx_b.send(1).unwrap();
                rx_a.await.unwrap()
            })
            .lazy_async("b", move || async move {
                tx_a.send(2).unwrap();
                rx_b.await.unwrap()
            }),
    );

    let page = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        render_async(response, &[], vec![]),
    )
    .await
    .expect("providers ran concurrently")
    .unwrap();

    assert_eq!(page["props"]["a"], 2);
    assert_eq!(page["props"]["b"], 1);
}

#[tokio::test]
async fn concurrency_limit_bounds_in_flight_providers() {
    let in_flight = counter();
    let peak = counter();
    let mut props = Props::new();

    for index in 0..6 {
        let in_flight = in_flight.clone();
        let peak = peak.clone();
        props = props.lazy_async(format!("p{index}"), move || async move {
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            in_flight.fetch_sub(1, Ordering::SeqCst);
            index
        });
    }

    let request = request(&[]);
    let page = Inertia::response("P", props)
        .resolve(
            RenderContext::new(&request, "/")
                .options(ResolveOptions::new().concurrency_limit(2)),
        )
        .await
        .unwrap();

    assert_eq!(page.props()["p5"], 5);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

fn tokio_sleeper() -> Sleeper {
    Arc::new(|duration| Box::pin(tokio::time::sleep(duration)))
}

#[tokio::test]
async fn rescued_deferred_props_are_omitted_and_reported() {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let request = request(&partial("Users", "permissions,roles"));
    let options = ResolveOptions::new().on_rescued_error({
        let reported = reported.clone();
        move |failure| {
            reported
                .lock()
                .unwrap()
                .push((failure.path.to_owned(), failure.error.to_string()))
        }
    });
    let page = Inertia::response(
        "Users",
        Props::new()
            .prop(
                "permissions",
                Prop::try_lazy_async(|| async { Err::<Value, _>("database unavailable") })
                    .defer()
                    .rescue(),
            )
            .prop("roles", Prop::lazy_async(|| async { vec!["admin"] }).defer()),
    )
    .resolve(RenderContext::new(&request, "/users").options(options))
    .await
    .unwrap();
    let page = serde_json::to_value(page).unwrap();

    assert_eq!(page["props"], json!({ "roles": ["admin"], "errors": {} }));
    assert_eq!(page["rescuedProps"], json!(["permissions"]));
    assert_eq!(
        *reported.lock().unwrap(),
        vec![("permissions".to_owned(), "database unavailable".to_owned())]
    );
}

#[tokio::test]
async fn unrescued_failures_fail_the_response() {
    let error = render_async(
        Inertia::response(
            "Users",
            Props::new().prop(
                "permissions",
                Prop::try_lazy(|| Err::<Value, _>("boom")).defer(),
            ),
        ),
        &partial("Users", "permissions"),
        vec![],
    )
    .await
    .unwrap_err();

    assert!(matches!(error, Error::Prop { ref path, .. } if path == "permissions"));
    assert!(error.to_string().contains("boom"));
}

#[tokio::test]
async fn provider_timeouts_fail_or_rescue() {
    let request = request(&partial("P", "slow,rescued"));
    let options = ResolveOptions::new()
        .timeout(std::time::Duration::from_millis(10), tokio_sleeper())
        .on_rescued_error(|_| {});
    let slow = || {
        Prop::lazy_async(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            1
        })
    };

    let page = Inertia::response("P", Props::new().prop("rescued", slow().defer().rescue()))
        .resolve(RenderContext::new(&request, "/").options(options.clone()))
        .await
        .unwrap();
    assert_eq!(page.rescued_props(), ["rescued"]);

    let error = Inertia::response("P", Props::new().prop("slow", slow()))
        .resolve(RenderContext::new(&request, "/").options(options))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Timeout { ref path, .. } if path == "slow"));
}

#[test]
fn sync_rendering_rejects_needed_async_providers_only() {
    let response = || {
        Inertia::response(
            "P",
            Props::new()
                .value("a", 1)
                .lazy_async("b", || async { 2 }),
        )
    };

    let page = render(response(), &partial("P", "a"));
    assert_eq!(page["props"]["a"], 1);

    let request = request(&[]);
    let error = response().into_page("/", None, &request).unwrap_err();
    assert!(matches!(error, Error::AsyncProp { ref path } if path == "b"));
}

#[test]
fn props_can_borrow_handler_data() {
    let request = request(&[]);
    let name = String::from("Ada");
    let page = Inertia::response(
        "Profile",
        Props::new()
            .value("name", &name)
            .lazy("upperName", || name.to_uppercase()),
    )
    .into_page("/profile", None, &request)
    .unwrap();

    assert_eq!(page.props()["name"], "Ada");
    assert_eq!(page.props()["upperName"], "ADA");
}

#[tokio::test]
async fn shared_props_are_filtered_like_route_props_and_route_props_win() {
    let shared_calls = counter();
    let shared = || {
        vec![
            ("appName", Prop::json(json!("Demo"))),
            ("auth.user", Prop::json(json!({ "name": "Shared" }))),
            ("notifications", Prop::lazy(counted(&shared_calls, json!([1])))),
            ("title", Prop::json(json!("Shared title"))),
        ]
    };
    let response = || Inertia::response("P", json!({ "title": "Route title", "users": [] }));

    let full = render_async(response(), &inertia(), shared()).await.unwrap();
    assert_eq!(
        full["props"],
        json!({
            "appName": "Demo",
            "auth": { "user": { "name": "Shared" } },
            "notifications": [1],
            "title": "Route title",
            "users": [],
            "errors": {}
        })
    );
    assert_eq!(
        full["sharedProps"],
        json!(["appName", "auth", "notifications", "title"])
    );

    let reload = render_async(response(), &partial("P", "users"), shared()).await.unwrap();
    assert_eq!(reload["props"], json!({ "users": [], "errors": {} }));
    assert_eq!(shared_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shared_dotted_props_and_route_roots() {
    // An undotted route root replaces the shared subtree...
    let page = render_async(
        Inertia::response("P", json!({ "auth": { "user": "Route" } })),
        &[],
        vec![("auth.user", Prop::json(json!("Shared"))), ("auth.csrf", Prop::json(json!("t")))],
    )
    .await
    .unwrap();
    assert_eq!(page["props"]["auth"], json!({ "user": "Route" }));

    // ...while a dotted route prop writes into a shared object.
    let page = render_async(
        Inertia::response("P", Props::new().value("auth.permissions", ["edit"])),
        &[],
        vec![("auth", Prop::json(json!({ "user": "Shared" })))],
    )
    .await
    .unwrap();
    assert_eq!(
        page["props"]["auth"],
        json!({ "user": "Shared", "permissions": ["edit"] })
    );
}

#[tokio::test]
async fn shared_once_props_respect_the_except_once_header() {
    let calls = counter();
    let mut headers = inertia();
    headers.push((X_INERTIA_EXCEPT_ONCE_PROPS, "countries"));
    let page = render_async(
        Inertia::response("P", ()),
        &headers,
        vec![("countries", Prop::lazy(counted(&calls, json!(["NL"]))).once())],
    )
    .await
    .unwrap();

    assert!(page["props"].get("countries").is_none());
    assert_eq!(page["onceProps"]["countries"]["prop"], "countries");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn big_integers_are_wrapped_in_props_and_flash() {
    let request = request(&[]);
    let page = Inertia::response("P", json!({ "id": 900_719_925_474_099_988u64, "small": 1 }))
        .preserve_big_integers()
        .flash("created", 900_719_925_474_099_989u64)
        .unwrap()
        .into_page("/", None, &request)
        .unwrap();
    let page = serde_json::to_value(page).unwrap();

    assert_eq!(page["props"]["id"], json!({ "$bigint": "900719925474099988" }));
    assert_eq!(page["props"]["small"], 1);
    assert_eq!(page["flash"]["created"], json!({ "$bigint": "900719925474099989" }));
    assert_eq!(page["preserveBigIntegers"], true);
}

#[test]
fn render_context_carries_session_state() {
    let request = request(&[]);
    let mut flash = Map::new();
    flash.insert("message".into(), json!("Saved"));
    let page = Inertia::response("P", ())
        .flash("highlight", 3)
        .unwrap()
        .resolve_sync(
            RenderContext::new(&request, "/p")
                .flash(flash)
                .clear_history(true)
                .preserve_fragment(true),
        )
        .unwrap();
    let page = serde_json::to_value(page).unwrap();

    assert_eq!(page["flash"], json!({ "message": "Saved", "highlight": 3 }));
    assert_eq!(page["clearHistory"], true);
    assert_eq!(page["preserveFragment"], true);
}

#[test]
fn filter_props_compatibility_helper_uses_the_resolver() {
    let request = request(&partial("Users", "users.name"));
    let mut props = json!({ "users": { "name": "Ada", "email": "a@example.com" }, "stats": 1 });
    request.filter_props("Users", &mut props, &PageMetadata::new());

    assert_eq!(props, json!({ "users": { "name": "Ada" }, "errors": {} }));
}

#[cfg(feature = "experimental-layers")]
#[test]
fn layer_metadata_and_close_pages() {
    let request = request(&inertia());
    let page = Inertia::response("Users/Create", ())
        .layer(Some("/users".into()), Some("users-create".into()))
        .into_page("/users/create", Some("v".into()), &request)
        .unwrap();
    let page = serde_json::to_value(page).unwrap();
    assert_eq!(page["layer"], json!({ "key": "users-create", "base": "/users" }));

    let bare = Inertia::response("Users/Create", ())
        .layer(None, None)
        .into_page("/users/create", None, &request)
        .unwrap();
    assert_eq!(serde_json::to_value(bare).unwrap()["layer"], json!({}));

    assert_eq!(
        serde_json::to_value(Page::close("/close", None)).unwrap(),
        json!({ "component": "", "props": {}, "url": "/close", "version": "", "close": true })
    );
}
