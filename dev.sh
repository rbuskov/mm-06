#!/usr/bin/env bash
#
# MM-06 dev launcher. Builds the whole app end to end — the native Rust crates,
# the shipped WASM, and the esbuild-bundled AudioWorklet — then frees the dev
# port and starts Vite on it.
#
# Usage:
#   ./dev.sh              # build everything, serve on http://localhost:5173
#   PORT=4000 ./dev.sh    # use a different port
#
set -euo pipefail

PORT="${PORT:-5173}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

echo "==> Building Rust workspace (native: dsp_core, dsp_wasm, render)"
cargo build

echo "==> Building WASM ship target (dsp_wasm -> web/wasm)"
wasm-pack build crates/dsp_wasm --target web --release --out-dir ../../web/wasm

cd "$ROOT/web"

if [ ! -d node_modules ]; then
  echo "==> Installing web dependencies (first run)"
  npm install
fi

echo "==> Bundling AudioWorklet (esbuild -> web/public/worklet.js)"
npm run build:worklet

echo "==> Freeing port $PORT"
if pids="$(lsof -ti "tcp:$PORT" 2>/dev/null)" && [ -n "$pids" ]; then
  echo "    killing: $pids"
  kill -9 $pids 2>/dev/null || true
  sleep 0.3
else
  echo "    nothing listening on $PORT"
fi

echo "==> Starting Vite on http://localhost:$PORT"
# --strictPort: we just freed the port, so fail loudly rather than silently
# drifting to another one.
exec npx vite --port "$PORT" --strictPort
