#!/usr/bin/env bash
# Builds @inertiajs/core and @inertiajs/react from the pinned, unreleased
# Layers commit (inertiajs/inertia#3236) and packs them into
# e2e/.layers-client/. See docs/compatibility-target.md.
set -euo pipefail

COMMIT=e57a7774430c095ab8616a5ac2b539bd324c26ca
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/e2e/.layers-client"
SRC="$OUT/inertia"
PNPM="npx -y pnpm@11.1.1"

mkdir -p "$OUT"

if [ ! -d "$SRC/.git" ]; then
  git clone --filter=blob:none https://github.com/inertiajs/inertia.git "$SRC"
fi

git -C "$SRC" fetch origin pull/3236/head
git -C "$SRC" checkout --detach "$COMMIT"

cd "$SRC"
$PNPM install --frozen-lockfile --ignore-scripts --filter "@inertiajs/core..." --filter "@inertiajs/react..."
(cd packages/core && $PNPM run build && $PNPM pack --pack-destination "$OUT")
(cd packages/react && $PNPM run build && $PNPM pack --pack-destination "$OUT")

echo "$COMMIT" > "$OUT/COMMIT"
ls "$OUT"/*.tgz
