# Axum + Svelte 5

Svelte 5 example on `@inertiajs/svelte` 3.11.0, served by the shared
[`smoke-server`](../smoke-server) routes plus a `/hello` page.

```sh
cd examples/axum-svelte/svelte-app && npm install && npm run build
cd ../.. && cargo run -p axum-svelte     # http://127.0.0.1:3002/ and /hello
```

Routes: `/` (deferred and optional props), `/smoke/form` (validation errors and
flash), `/hello` (shared, deferred, and optional props with partial reloads).
