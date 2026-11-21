# Protocol Support Matrix

Target: **Inertia 3.11.0** (`@inertiajs/core`, `react`, `vue3`, `svelte`,
`vite`), reference server adapter `inertia-laravel` v3.7.0. Exact versions,
commits, and verification dates are in
[`compatibility-target.md`](compatibility-target.md). The pre-work audit is
[`audit-2026-11.md`](audit-2026-11.md).

Update this document in the same change as any adapter behaviour change.

## Status vocabulary

- **Complete**: implemented and verified by the tests listed.
- **Partial**: implemented with a documented limitation.
- **Client-only**: implemented entirely by the official client; the server
  only needs ordinary protocol behaviour, which the listed tests exercise.
- **App**: application responsibility, with the integration path given.

## Evidence

- *Unit* tests are Rust tests (`cargo test --all-features`, 129 tests):
  `tests::*` (protocol core), `axum::tests::*`, `rocket::tests::*`, and
  module tests.
- *Browser* tests are Playwright tests in `e2e/tests` running the examples
  against the official packages from npm (3.11.0):
  `react.spec.js` (18, React, `examples/axum-react`),
  `smoke.spec.js` (4 × Vue and Svelte, `examples/smoke-server`),
  `rocket.spec.js` (2, Svelte, `examples/rocket-svelte`).
  Result on 2026-11-21: **28 passed, 0 failed** (Chromium).

## Stable protocol (Inertia 3.11.0)

| Feature | Axum | Rocket | Evidence |
| --- | --- | --- | --- |
| Initial HTML: `<script data-page type="application/json">` + root element; `<`, `>`, `&`, U+2028/9 escaped | Complete | Complete | Unit: `html::tests::escapes_script_breaking_sequences`, `axum::tests::html_response_escapes_json_for_script_context`, `rocket::tests::html_response_escapes_json_for_script_context`. Browser: every React/Vue/Svelte/Rocket test boots from it |
| JSON response: `X-Inertia: true`, `Content-Type: application/json` | Complete | Complete | Unit: `axum::tests::inertia_json_response_includes_headers_url_and_version`, `rocket::tests::json_response_has_protocol_headers`. Browser: `links swap components…` |
| `Vary: X-Inertia` on every response | Complete | Complete | Unit: `axum::tests::vary_is_added_to_every_response_once`, `rocket::tests::every_response_varies_on_x_inertia` |
| Page object fields; `version` always present (`""` when unversioned) | Complete | Complete | Unit: `tests::minimal_page_always_has_errors_and_version`, `tests::page_from_parts_serializes_every_metadata_field`, `axum::tests::unversioned_apps_send_an_empty_version_and_accept_a_missing_header` |
| Page URL: path + query, nested routers, absolute-form requests | Complete | Complete | Unit: `axum::tests::nested_routes_use_original_uri_for_page_urls_and_conflicts`, `axum::tests::absolute_form_requests_use_local_page_urls`, `rocket::tests::with_url_overrides_request_uri` |
| Static and dynamic asset versions | Complete | Complete | Unit: `axum::tests::dynamic_version_is_resolved_per_request`, `rocket::tests::dynamic_versions_are_used_for_pages_and_conflicts` |
| Version mismatch: `409` + `X-Inertia-Location` + `X-Inertia-Version`, `GET` only, missing header == `""`, before the handler runs | Complete | Complete | Unit: `axum::tests::version_conflicts_short_circuit_and_report_the_server_version`, `rocket::tests::version_conflicts_report_location_and_server_version`. Browser: `a version change forces a full reload…`, smoke `asset version changes force a full reload` |
| External redirects (`Inertia::location`): `409` + `X-Inertia-Location` | Complete | Complete | Unit: `axum::tests::external_locations_always_use_x_inertia_location`, `rocket::tests::locations_use_x_inertia_location_for_inertia_requests`. Browser: `external locations trigger a full window.location visit` |
| `302` → `303` after `PUT`/`PATCH`/`DELETE`/`QUERY` (middleware); `303` from `Inertia::redirect` after writes | Complete | Partial: no `QUERY` (Rocket 0.5 does not route it) | Unit: `axum::tests::middleware_converts_handler_built_302_after_write_methods`, `axum::tests::redirect_statuses_follow_the_method`, `rocket::tests::fairing_converts_found_after_put_for_inertia_requests`. Browser: `delete redirects with 303 and flashes` |
| Fragment redirects → `409` + `X-Inertia-Redirect` (not for prefetch) | Complete | Complete | Unit: `axum::tests::fragment_redirects_become_inertia_redirects_except_for_prefetch`, `rocket::tests::fairing_turns_fragment_redirects_into_inertia_redirects`. Browser: `fragment redirects become Inertia visits…` |
| Empty `200` → redirect back | Complete | Complete | Unit: `axum::tests::empty_inertia_responses_redirect_back`, `rocket::tests::fairing_redirects_back_on_empty_inertia_responses` |
| Partial reloads: `only`, `except`, both (narrow then remove), component match, every method | Complete | Complete | Unit: `tests::partial_data_and_except_compose_by_narrowing_then_removing`, `tests::partial_except_alone_returns_everything_else`, `axum::tests::partial_reload_headers_apply_to_every_method`, `rocket::tests::partial_reloads_filter_props_and_compose_only_and_except`. Browser: `optional props load only when requested` |
| Dot-notation paths (`auth.user`) in partial reloads and prop keys | Complete | Complete | Unit: `tests::dot_notation_paths_filter_inside_plain_values`, `tests::dotted_keys_build_nested_props_with_typed_children`, `tests::values_resolved_by_providers_are_returned_whole` |
| Lazy props never run when excluded | Complete | Complete | Unit: `tests::excluded_lazy_props_never_run`, `axum::tests::partial_reloads_filter_shared_props_and_skip_their_providers` |
| Always props | Complete | Complete | Unit: `tests::always_props_ignore_partial_filters_and_errors_are_always_included` |
| Optional props | Complete | Complete | Unit: `tests::optional_props_follow_reference_partial_rules`. Browser: `optional props load only when requested`, smoke and Rocket optional tests |
| Deferred props, groups, `deferredProps` only on full visits | Complete | Complete | Unit: `tests::deferred_props_are_announced_on_full_visits_only`, `rocket::tests::async_providers_render_through_the_guard`. Browser: `deferred props load in follow-up partial reloads per group`, Rocket `first visit … loads deferred props` |
| Rescued deferred props (`rescuedProps`, prop omitted, `200`, error reported) | Complete | Complete | Unit: `tests::rescued_deferred_props_are_omitted_and_reported`, `tests::unrescued_failures_fail_the_response`. Browser: `rescued deferred props render the rescue slot` |
| Once props: key, expiry, `fresh`, except-once on full visits only, deferred once | Complete | Complete | Unit: `tests::once_props_skip_resolution_when_the_client_has_them`, `tests::fresh_and_custom_key_once_props`, `tests::deferred_once_props_already_loaded_are_not_announced`. Browser: `shared once props are remembered across visits` |
| Shared props (resolved with route props, filtered by partial reloads, `sharedProps` keys, typed/once/async shared props) | Complete | Complete (request-aware `prop` providers run eagerly) | Unit: `tests::shared_props_are_filtered_like_route_props_and_route_props_win`, `tests::shared_once_props_respect_the_except_once_header`, `axum::tests::shared_props_are_resolved_with_route_props`, `rocket::tests::shared_props_are_filtered_by_partial_reloads`. Browser: `shared props are available on every page` |
| Merge, prepend, deep merge, nested merge paths, `matchPropsOn`, reset | Complete | Complete | Unit: `tests::merge_paths_and_matching_keys_are_relative_to_the_prop`, `tests::reset_props_drop_merge_metadata`, `tests::deferred_merge_props_emit_merge_metadata_on_the_full_visit`, `tests::partial_reloads_only_emit_metadata_for_requested_props` |
| Infinite scroll: `scrollProps` (number or cursor pages, `reset`), merge intent, deferred scroll | Complete | Complete | Unit: `tests::scroll_props_emit_pagination_reset_and_merge_intent`, `tests::scroll_metadata_can_be_derived_from_the_resolved_value`, `tests::deferred_scroll_props_omit_scroll_metadata_until_loaded`, `rocket::tests::scroll_reset_and_prepend_intent`. Browser: `loads further pages and merges them` |
| History encryption / `clearHistory` (per response and carried across a redirect) | Complete | Partial: per response only (no session backend) | Unit: `tests::page_from_parts_serializes_every_metadata_field`, `axum::tests::sessions::clear_history_is_carried_to_the_next_page` |
| `preserveFragment` (per response and carried across a redirect) | Complete | Partial: per response only | Unit: `tests::render_context_carries_session_state` |
| Validation errors: `errors` always prop, error bags, redirect back, once-only delivery | Complete (session backend) | Partial: `errors: {}` only; app supplies errors | Unit: `session::tests::error_bag_rules_match_reference_adapter`, `axum::tests::sessions::validation_errors_redirect_back_and_respect_error_bags`. Browser: `validation errors redirect back and are shown once`, smoke `validation errors and flash survive redirects` |
| Flash data: `flash` field, kept across redirects and version conflicts, delivered once, per-session | Complete (session backend) | Partial: per response only (`Inertia::flash`) | Unit: `axum::tests::sessions::flash_survives_one_redirect_and_is_delivered_once`, `…flash_is_kept_through_a_version_conflict`, `…sessions_are_isolated_per_user`. Browser: `successful submissions flash once and survive the redirect` |
| Big integers (`preserveBigIntegers`, `$bigint`) in props and flash | Complete | Complete | Unit: `bigint::tests::wraps_only_unsafe_integers`, `tests::big_integers_are_wrapped_in_props_and_flash`. Browser: `big integers survive as BigInt values` |
| Live props: `liveProps` listeners/throttle, survives exclusion, partial-reload scoping, invalid combinations rejected | Complete | Complete (metadata) | Unit: `tests::live_props_emit_listeners_and_survive_exclusion`, `tests::unsupported_prop_combinations_are_rejected`, `live::tests::*` |
| Live props transport: broadcast envelope (`__inertia.props`), `X-Socket-Id`/to-others, private-channel authorization | Complete (SSE broadcaster + custom `LiveTransport`) | App (use any broadcaster with `BroadcastPayload`) | Unit: `axum::live::tests::*`. Browser: `broadcast values update other clients without a request`, `reload events refresh other clients through partial reloads and skip the sender` |
| SSR: `POST /render`, Vite `/__inertia_ssr`, `head`/`body` injection, `null` = client render, timeouts, classified errors, fallback or fail | Complete | Complete (via `InertiaRequest` guard) | Unit: `ssr::http::tests::*`, `ssr::tests::*`, `axum::tests::ssr_renders_first_visits_only`, `axum::tests::ssr_failures_fall_back_or_fail_by_policy`, `rocket::tests::ssr_applies_to_first_visits_through_the_guard`. Browser: `server-renders the first visit and hydrates without errors` (production Node server). Dev endpoint verified manually on 2026-11-21 (SSR markup, hydration, no console errors) |
| Async providers: concurrent, bounded concurrency, timeouts, cancellation by drop | Complete | Complete (`InertiaRequest::render`); sync `Responder` rejects async props | Unit: `tests::async_props_resolve_concurrently`, `tests::concurrency_limit_bounds_in_flight_providers`, `tests::provider_timeouts_fail_or_rescue`, `axum::tests::prop_timeouts_are_configurable_on_the_layer`, `rocket::tests::async_providers_in_sync_responders_are_server_errors` |
| Error pages through Inertia with shared data | Complete (`error_page`) | App | Browser: `unknown routes render the Inertia error page with a 404` |
| `serverHead` (`page.props.head`) | Client-only | Client-only | Prop convention; no server behaviour |
| Prefetching, polling, `<Form>`, `useForm`, optimistic updates, `useHttp`, instant visits, view transitions, remember | Client-only | Client-only | Use ordinary visits; prefetch `Purpose` header respected for fragment redirects |
| Precognition | App | App | Headers parsed (`RequestContext::is_precognitive`); validation libraries own the `204`/`422` responses |
| DevTools protocol, testing helpers | Not implemented | Not implemented | Out of scope for this release |

## Deliberate deviations from the reference adapter

| Topic | Reference (`inertia-laravel` 3.7.0) | `inertia_rs` | Reason |
| --- | --- | --- | --- |
| Shared dotted key vs route root (`auth.user` shared, `auth` route prop) | Shared value written into the route's array | Route root wins; shared dotted key dropped | Route props win consistently; kept from 0.4 after review |
| Dotted key through a closure prop | Closure is evaluated to traverse it | Closure is replaced by the nested object | Async providers cannot be evaluated synchronously during tree building |
| `always` + merge on one prop | Not expressible (class hierarchy) | Allowed | Not contradictory in the protocol; 0.4 code relied on it |
| Version-mismatch `X-Inertia-Location` | Absolute URL | Path and query | Avoids trusting `Host` behind proxies; the client's `window.location` accepts both |
| Version-mismatch timing | Handler runs, response replaced | Short-circuits before the handler | Avoids side effects for a response that is discarded |
| Unconsumed flash on non-redirect, non-page responses | Session flash expires after one request | Kept until a page renders | Asset or API requests do not swallow flash data |
| Channel inference for live props | From the event class's `broadcastOn()` | Channels are declared explicitly | No event classes in Rust |

## Experimental: Layers (inertiajs/inertia#3236)

**Not part of stable Inertia.** Behind the `experimental-layers` Cargo
feature. Target: client commit `e57a7774430c`, server PR
inertiajs/inertia-laravel#903 (`e819bc99db2f`).

| Feature | Axum | Rocket | Evidence |
| --- | --- | --- | --- |
| `layer: {key?, base?}`, base made host-relative | Complete | Not implemented | Unit: `tests::layer_metadata_and_close_pages`, `layers::tests::strips_same_host_origins_only`, `axum::tests::layers::layer_base_is_host_relative`. Browser: `a layer opens over its base…` |
| Close response `{component: "", props: {}, url, version, close: true}`; redirect back for non-Inertia requests; flash left for the refresh beneath | Complete | Not implemented | Unit: `axum::tests::layers::close_responses_keep_flash_for_the_refresh_beneath`. Browser: `validation errors stay in the layer and close refreshes…` |
| `interstitial: true` carried across a redirect once; cleared by a close | Complete | Not implemented | Unit: `axum::tests::layers::interstitial_flag_is_carried_across_a_redirect_once`. Browser: `an interstitial detour opens the pending layer…` |
| Cold visits (initial HTML) and SSR of layers | Complete | Not implemented | Browser: `a cold visit renders the layer and fetches its base beneath` (SSR renders the `<dialog>` shell) |
| Partial reloads, validation, and deferred props inside layers | Complete (no server-specific behaviour) | — | Browser: validation inside the layer |

Browser result on 2026-11-21: **4 passed** with the client built by
`scripts/build-layers-client.sh`.

Observed client behaviour (not a server defect): after a close, the PR client
first re-requests the base page's deferred props and ignores `flash` on
deferred-prop requests, so a flash message set before `close()` is delivered
by the server but not displayed when the base page has deferred props. The
reference adapter delivers flash the same way.

## Remaining limitations

- Rocket has no session backend: validation errors, flash, and redirect-carried
  flags must be supplied by the application (`RenderContext`/`SessionState`
  are framework-neutral building blocks). Rocket does not route `QUERY`.
- The SSE live-prop broadcaster delivers events within one process. Use a
  shared broker (for example a Pusher-compatible server via
  `BroadcastPayload`) for multi-instance deployments.
- `HttpSsrGateway` supports `http://` only; implement `SsrGateway` for TLS.
- Browser tests run on Chromium only.
- DevTools protocol and testing helpers are not implemented.
