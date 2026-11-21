# Migrating from 0.4 to 0.5

0.5 targets the Inertia 3.11 protocol and replaces prop resolution with a
reference-compatible resolver. Most 0.4 code compiles unchanged; the changes
below are either compile-time breaks or protocol behaviour changes.

## Behaviour changes (protocol alignment)

These follow the specification and the reference adapter. Review them if your
frontend relied on the 0.4 behaviour.

| Area | 0.4 | 0.5 |
| --- | --- | --- |
| `only` + `except` together | `except` replaced `only` | Narrow to `only`, then remove `except` |
| Dot paths in `only`/`except` | Top-level keys only | Nested paths inside plain values (`auth.user`) |
| Shared props on partial reloads | Always sent | Filtered like route props (only `errors` is always sent); excluded providers do not run |
| `sharedProps` metadata | Keys actually inserted | Root keys of every registered shared prop, including `errors` |
| Partial reloads on non-`GET` requests | Ignored | Applied (the reference applies them to every method) |
| Optional/deferred props on partial reloads | Optional props needed `only` | Any prop the partial filter keeps is resolved (`except`-only reloads included) |
| Once props on partial reloads | `X-Inertia-Except-Once-Props` still applied | Ignored on partial reloads |
| `deferredProps` | Could appear on partial reloads | Full visits only |
| Merge/once metadata for deferred or optional props | Dropped when the value was absent | Emitted on the full visit |
| `version` | Omitted without a version layer | Always present; `""` when unversioned |
| Missing `X-Inertia-Version` header | Always a conflict | Equal to `""`; conflicts only when the server version is non-empty |
| Version-conflict `409` | `X-Inertia-Location` | Adds `X-Inertia-Version` with the server version |
| `Inertia::location` with a fragment | `X-Inertia-Redirect` | Always `X-Inertia-Location` (external visit) |
| Fragment redirects | Only via `location` | Any redirect to a URL with `#` becomes `409 X-Inertia-Redirect` for Inertia requests (middleware) |
| `Vary: X-Inertia` | Inertia-built responses | Every response through the middleware |
| `302` after `PUT`/`PATCH`/`DELETE`/`QUERY` | Untouched unless built with `Inertia::redirect` | Converted to `303` by the middleware |
| Empty `200` to an Inertia request | Returned as-is | Redirect back (disable with `InertiaLayer::redirect_empty_responses(false)`) |
| `rescue("prop")` | Metadata only | Catches provider failures; ignored for non-deferred props |
| `errors` with error bags | `{}` only | Rendered from session errors with bag rules (Axum session backend) |

## Compile-time changes

- **Prop closures must be `Send`.** `Props`/`InertiaProps`/
  `ScopedInertiaProps` (now one type, `Props<'a>`) hold `Send` providers so
  async rendering works in Axum and Rocket handlers. Replace `Rc`/`Cell`
  captures with `Arc`/atomics.
- **`IntoPageProps`** now has a lifetime and one method,
  `into_page_props(self) -> Result<Props<'a>, Error>`. Implementations by
  applications are unusual; plain `Serialize` types and `Props` implement it.
- **Props must serialize to an object.** Non-object, non-`null` props return
  `Error::InvalidProps`.
- **`Inertia::into_page`** returns `inertia_rs::Error` instead of
  `serde_json::Error`.
- **Axum `InertiaError` and Rocket errors are `#[non_exhaustive]`** with new
  variants (`Props`, `Ssr`, `MissingTemplate`).
- **`ScrollProps` accessors** return `Option<&ScrollPage>` (pages may be
  numbers or cursors). Constructors are source compatible.
- **Axum `SharedProps::prop`** providers run lazily when the prop is included.
  Rocket `SharedProps::prop` providers still run per page.
- **`RequestContext::filter_props`** is a compatibility helper that runs the
  new resolver; prefer `Inertia::resolve`.
- `Page::with_shared_props` remains for hand-built pages; framework
  integrations no longer use it.

## Recommended updates

### Axum

```rust
// 0.4
.layer(Extension(shared_props))
.layer(VersionLayer::new("v1"))

// 0.5
.layer(
    InertiaLayer::new("v1")            // VersionLayer is an alias
        .template(|page| Html(render_root(page)))
        .shared(shared_props)          // Extension(SharedProps) still works
        .session(TowerSessions::new()),
)
.layer(SessionManagerLayer::new(store))
```

```rust
// 0.4: synchronous render with a per-call template
request.render(Inertia::response("Home", props), |page| Html(render_root(page)))

// 0.5: async, supports async props and SSR
request.respond(Inertia::response("Home", props)).await
```

`render` still works for synchronous props; it returns
`Error::AsyncProp` when an async provider (including an async shared prop) is
needed and does not apply SSR.

### Rocket

Responders keep working. Use the `InertiaRequest` guard and
`InertiaRequest::render(...).await` for async providers and SSR.

### Templates

`HtmlResponseContext::body()` renders the page script and root element (or
SSR markup) and `head()` renders SSR head tags. `data_page()` is unchanged.
