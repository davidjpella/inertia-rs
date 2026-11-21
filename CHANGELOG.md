# Changelog

All notable changes to this project will be documented in this file.

## Unreleased (0.5.0)

Targets Inertia 3.11.0 (`docs/compatibility-target.md`). Upgrade notes:
`docs/migration-0.5.md`. Evidence: `docs/protocol-support.md`.

### Added

- Reference-compatible prop resolver: path-based partial reloads (`only`,
  `except`, both), dot-notation props, full-visit exclusions, and metadata
  rules matching `inertia-laravel` 3.7.0.
- `Prop`/`Props` with sync and async providers (concurrent, optional
  concurrency limit and timeouts), fallible providers, rescue, once
  (custom keys, expiry, `fresh`), merge/prepend/deep merge with nested paths
  and `matchPropsOn`, infinite scroll with cursor pages and `reset`, live
  props, and validation of unsupported combinations.
- Page object: `flash`, `preserveBigIntegers` (`$bigint` markers),
  `liveProps`; `version` always present.
- Framework-neutral session state (`session` module): validation errors with
  error bags, flash, and redirect-carried `clearHistory`/`preserveFragment`.
- Axum: `InertiaLayer` middleware (version conflicts with `X-Inertia-Version`,
  `Vary` on every response, `303` conversion incl. `QUERY`, fragment redirects,
  empty-response redirects, session carry-over); async `respond`/
  `respond_with`; root templates; `flash`, `with_errors`, `back`,
  `clear_history`, `preserve_fragment`; `error_page`; `SessionBackend` and
  `TowerSessions`; SSE live-prop `Broadcaster` with channel authorization.
- Rocket: `InertiaRequest` guard with async rendering and SSR; fairing
  post-processing (`Vary`, `303`, fragment redirects, empty-response
  redirects, `X-Inertia-Version`); typed/optional shared props.
- SSR: `SsrGateway`, `Ssr` (fallback or error policy, failure reporting,
  path filter), `HttpSsrGateway` for `/render` and Vite `/__inertia_ssr`.
- Live props: `LiveUpdates`, `LiveChannel`, `BroadcastPayload`.
- Experimental Layers (`experimental-layers`): `layer`, close responses,
  `interstitial`.
- Examples: `axum-react` (full), `axum-vue`, shared `smoke-server`; Playwright
  suites for React, Vue, Svelte, Rocket, and experimental Layers; CI jobs for
  them and for MSRV with MSRV-aware resolution.

### Changed

- See the behaviour table in `docs/migration-0.5.md`, notably shared props
  following partial-reload filtering, `only`+`except` composition, partial
  reloads on every method, and `Inertia::location` always using
  `X-Inertia-Location`.
- Prop providers must be `Send`; `InertiaProps`/`ScopedInertiaProps` are
  aliases of `Props`.
- Error enums are `#[non_exhaustive]`.

### Fixed

- Missing `X-Inertia-Version` no longer conflicts with an empty server version.
- `rescue` now catches provider failures instead of only emitting metadata.

## 0.4.0 - 2026-05-15

### Added

- Rocket 0.5.1, Svelte 5, and Vite example updates.
- Public Inertia protocol header constants and `RequestContext` parsing.
- Inertia v3 page-object metadata for history flags, merge props, deferred props, once props, shared props, and scroll props.
- Partial reload filtering for matching Inertia components.
- Asset version handling for successful page responses and stale Inertia visits.
- `VersionFairing::dynamic` for request-time asset version values.
- `InertiaHeaders` Rocket request guard.
- `Inertia::location` for external Inertia redirects.
- `Inertia::redirect` for method-aware application redirects.
- Rocket `SharedProps` managed state for common page props.
- Initial Axum integration with `InertiaRequest`, `VersionLayer`, page response rendering, external locations, and method-aware redirects.
- Minimal Axum example.
- `InertiaProps` and `ScopedInertiaProps` for synchronous lazy, optional, deferred, and once prop resolvers.
- Axum `SharedProps` extension support for common page props.
- README protocol support matrix.

### Changed

- Preserved query strings in generated page object URLs.
- Added `Vary: X-Inertia` to responses that vary between HTML and JSON.
- Modernized GitHub Actions CI and declared an MSRV of Rust 1.88.
- Expanded crate and public API rustdoc.

### Fixed

- Escaped serialized page JSON for safe embedding in HTML script contexts.
- Kept shared dotted props from merging into route-owned prop roots.
- Preserved route-owned prop roots across partial filtering before shared-prop merging.
- Kept internal route-prop tracking out of `Page` equality.
