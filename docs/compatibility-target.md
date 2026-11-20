# Compatibility Target

Verification date: **2026-11-20**

This file records the exact upstream versions that `inertia_rs` is developed
and tested against. A compatibility claim in this repository always refers to
the versions listed here. Update this file (and re-run the end-to-end suite)
before raising the target.

## Stable Inertia

| Package | Version | Source |
| --- | --- | --- |
| `@inertiajs/core` | 3.11.0 | npm `latest` dist-tag, published 2026-11-02 |
| `@inertiajs/react` | 3.11.0 | npm `latest` dist-tag |
| `@inertiajs/vue3` | 3.11.0 | npm `latest` dist-tag |
| `@inertiajs/svelte` | 3.11.0 | npm `latest` dist-tag |
| `@inertiajs/vite` | 3.11.0 | npm `latest` dist-tag |

- Upstream git: `inertiajs/inertia` tag `v3.11.0`, commit
  `f3125577a9e5a5a8bcdecffefc2e6577282f7868`.
- Protocol specification: `inertiajs/docs` commit
  `8cfece18fd9daa35acf028c3cb0519344c0252fd`, files
  `v3/core-concepts/the-protocol.mdx` and `v3/advanced/building-an-adapter.mdx`.
- Reference server adapter: `inertiajs/inertia-laravel` tag `v3.7.0`, commit
  `9d3f9060c1614565eb4d3a3208015ed01de347bb` (released 2026-11-02, the same day
  as client 3.11.0). Where the protocol leaves a choice to the server, this
  crate follows the reference adapter unless a deviation is documented in
  [`protocol-support.md`](protocol-support.md).

Releases between the previous target (Inertia 3.x as of 2026-05) and 3.11.0
were reviewed. Server-relevant changes:

| Release | Change | Server impact |
| --- | --- | --- |
| 3.0.0 | Page object in `<script type="application/json" data-page>`; `preserveFragment`; fragment redirects; optional `clearHistory`/`encryptHistory`; nested dot-notation props | Already supported in 0.4.0; dot-notation partial paths were not |
| 3.1.0 | `rescue` slot for failed deferred props | Server must emit `rescuedProps` and omit the failed prop |
| 3.5.0 | `serverHead` client option | Prop convention only (`page.props.head`); no protocol change |
| 3.6.0 | Built-in client sends `X-Requested-With` | None |
| 3.7.1 | Escape `<` in initial page JSON | Already escaped |
| 3.8.0 | Big integers (`preserveBigIntegers`, `{"$bigint": "..."}`) | New page-object field and prop encoding |
| 3.10.0 | `QUERY` HTTP method | `302` after `QUERY` must become `303` |
| 3.11.0 | Live props (`liveProps`, `X-Socket-Id`, `__inertia` broadcast envelope) | New page-object field and broadcast payload helper |

## Experimental: Layers

| Item | Value |
| --- | --- |
| Client PR | [`inertiajs/inertia#3236`](https://github.com/inertiajs/inertia/pull/3236) |
| PR state on 2026-11-20 | **Open, not merged, not released** |
| Pinned client commit | `e57a7774430c095ab8616a5ac2b539bd324c26ca` (branch `layers`) |
| Companion server PR | [`inertiajs/inertia-laravel#903`](https://github.com/inertiajs/inertia-laravel/pull/903), open, head `e819bc99db2fbe872fa10ef8b1994da92adcffd9` |
| Preview packages | None published (`pkg.pr.new` returns 404); the e2e suite builds the client from the pinned commit |

Layers are implemented behind the `experimental-layers` Cargo feature. The
contract can change before upstream merges it. Nothing in the stable feature
set depends on it, and stable compatibility claims do not include it.

## Rust toolchain

- MSRV: Rust 1.88 (verified in CI).
- Development toolchain at verification time: Rust 1.90.0.
