# Axum + React + Vite

A complete `inertia_rs` application on the official `@inertiajs/react` 3.11.0
client. It exercises every server-side protocol feature:

| Page | Features |
| --- | --- |
| `/` Dashboard | Shared props, shared once prop, async deferred props (two groups, one rescued with `/?fail=1`), optional prop, live props (value broadcast and reload broadcast), big integers in flash |
| `/users` | Infinite scroll (`scrollProps`, merge intent, `matchPropsOn`), flash after redirects |
| `/users/create` | `useForm` validation errors (redirect back) and flash |
| `/users/{id}` | Deferred prop, `DELETE` with `303` |
| `/fragment`, `/external` | Fragment redirect, external location |
| anything else | Inertia-rendered 404 via `error_page` |

## Production build (with SSR)

```sh
cd examples/axum-react/frontend
npm install
npm run build                 # client bundle -> ../public/build, SSR bundle -> bootstrap/ssr
npm run ssr                   # Node SSR server on 127.0.0.1:13714

# in another terminal, from examples/
INERTIA_SSR_URL=http://127.0.0.1:13714 cargo run -p axum-react
# open http://127.0.0.1:3000
```

Without `INERTIA_SSR_URL` the app renders on the client. If the SSR server is
down, pages fall back to client rendering and the failure is logged.

## Development (Vite HMR, SSR through the Vite dev server)

```sh
cd examples/axum-react/frontend && npx vite          # VITE_PORT=5199 to change the port
# from examples/
VITE_DEV_SERVER=http://localhost:5173 INERTIA_SSR_DEV=1 cargo run -p axum-react
```

`INERTIA_SSR_DEV=1` posts page objects to `/__inertia_ssr` on the Vite dev
server, which `@inertiajs/vite` exposes for the `src/app.jsx` entry.

## Live props

`src/liveTransport.js` is a custom `LiveTransport` for the crate's SSE
`Broadcaster` (`/inertia/live`). Open the dashboard in two windows and place
an order: "broadcast value" updates other windows from the `__inertia.props`
envelope without a request; "broadcast reload" makes other windows reload
`lastOrderAt` (the sender is skipped through `X-Socket-Id`).

## Experimental Layers

Layers (inertiajs/inertia#3236) are unreleased. The `layers` feature adds an
edit layer (`/users/{id}/edit`, closes with flash) and an interstitial detour
(`/users/{id}/secure-edit` → `/confirm-password`).

```sh
../../scripts/build-layers-client.sh      # from examples/axum-react; builds the pinned client
cd frontend-layers && npm install && npm run build && npm run ssr   # SSR on 13715
# from examples/
FRONTEND_BUILD_DIR=public/build-layers INERTIA_SSR_URL=http://127.0.0.1:13715 \
  cargo run -p axum-react --features layers
```

With the stable 3.11 client the same routes render as ordinary pages, because
the new page-object fields are optional.

## Browser tests

```sh
cd e2e && npm install && npx playwright install chromium
E2E_FRAMEWORKS=react npx playwright test   # starts the SSR server and the app
```
