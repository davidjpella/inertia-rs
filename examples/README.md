# Examples

| Example | Client | Notes |
| --- | --- | --- |
| [`axum-react`](axum-react) | `@inertiajs/react` 3.11.0 | Full feature tour, SSR (dev and production), live props, experimental layers |
| [`axum-vue`](axum-vue) | `@inertiajs/vue3` 3.11.0 | Smoke app (shared `smoke-server` routes) |
| [`axum-svelte`](axum-svelte) | `@inertiajs/svelte` 3.11.0 | Smoke app plus `/hello` |
| [`rocket-svelte`](rocket-svelte) | `@inertiajs/svelte` 3.11.0 | Rocket with Handlebars templates |
| [`axum-minimal`](axum-minimal), [`rocket-minimal`](rocket-minimal) | none | Protocol responses without a frontend build |

Build each frontend with `npm install && npm run build` in its frontend
directory, then `cargo run -p <example>` from this directory. Browser tests for
the examples live in [`../e2e`](../e2e).
