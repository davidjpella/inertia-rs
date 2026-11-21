# Release Readiness: 0.5.0

Status on 2026-11-21: **ready for review; not published.** Publishing to
crates.io requires explicit maintainer approval.

## Claims

- **Stable:** compatible with Inertia **3.11.0** (React, Vue, Svelte, Vite),
  following `inertia-laravel` 3.7.0 where the protocol leaves a choice.
  Axum implements every applicable server-side requirement; Rocket implements
  all except session-backed flash/validation storage and `QUERY` (see
  `protocol-support.md`).
- **Experimental:** Layers against client commit `e57a7774430c`
  (inertiajs/inertia#3236, unmerged). Not part of the stable claim; the
  `experimental-layers` feature may change in any release.

## Verification performed

| Check | Result |
| --- | --- |
| `cargo fmt --check` (crate and examples) | Pass |
| `cargo clippy --all-features --all-targets -D warnings` | Pass |
| `cargo test --all-features` | 129 unit + 13 doc tests pass |
| Feature subsets (`axum`, `rocket`, `ssr`, `experimental-layers`, none) | Pass |
| `cargo doc -D warnings` (all features / no default features) | Pass |
| MSRV 1.88 with MSRV-aware resolution (`check --all-features`, unit tests) | Pass (Rocket 0.5.4 needs 1.89; resolver selects 0.5.1) |
| Example workspace build | Pass |
| Playwright, stable (React 18, Vue 4, Svelte 4, Rocket 2) | 28 / 28 pass (Chromium) |
| Playwright, experimental Layers | 4 / 4 pass (Chromium) |
| SSR via Vite dev endpoint | Manually verified: SSR markup, hydration, no console errors |

## Before publishing

1. Push the branch and confirm the GitHub Actions jobs pass on Linux (they
   were run locally on macOS only).
2. Re-check `npm view @inertiajs/core dist-tags`; if a newer stable release
   exists, review its notes and update `compatibility-target.md`.
3. Re-check inertiajs/inertia#3236; if it merged with a different contract,
   update or keep the feature experimental.
4. Review `docs/migration-0.5.md` with users of 0.4: shared props now follow
   partial-reload filtering, which is the main behaviour change.
5. `cargo publish --dry-run`.

## Known limitations

See "Remaining limitations" in `protocol-support.md`. Highlights:

- Rocket has no session backend.
- The SSE live broadcaster is single-process.
- `HttpSsrGateway` is `http://` only.
- Browser coverage is Chromium only; Vue and Svelte have smoke coverage
  (navigation, deferred/optional props, validation, flash, version reloads),
  React has the full suite.
- Layers: when the base page has deferred props, the PR client discards the
  flash set before `close()` (client behaviour; the server delivers it).
