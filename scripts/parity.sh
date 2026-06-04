#!/usr/bin/env bash
# WASM<->native cross-architecture parity gate (calibration-and-testing-strategy.md §5).
#
# Renders the SAME canonical RenderRequest (crates/dsp_core/examples/parity_dump.rs)
# through two builds of dsp_core::render — native and wasm32 — and diffs the raw
# IEEE-754 bit streams. Tolerance is ZERO: any differing line is a parity bug.
#
# This is the half of §5 that needs a second architecture. It is NOT run by
# `cargo test` because it requires a wasm runtime that may not be installed.
# Run it explicitly:  ./scripts/parity.sh
#
# Requirements (any ONE wasm runtime):
#   - rustup target add wasm32-wasip1   (or wasm32-wasi on older toolchains)
#   - wasmtime  OR  wasmer
# If neither is present the script prints how to obtain them and exits 2 — the
# native streaming-vs-offline gate (`cargo test --test parity`) still stands.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# --- pick a wasm32 WASI target that is installed -------------------------------
WASI_TARGET=""
for t in wasm32-wasip1 wasm32-wasi; do
  if rustup target list --installed 2>/dev/null | grep -qx "$t"; then
    WASI_TARGET="$t"
    break
  fi
done
if [[ -z "$WASI_TARGET" ]]; then
  echo "parity: no wasm32 WASI target installed." >&2
  echo "        run:  rustup target add wasm32-wasip1" >&2
  exit 2
fi

# --- pick a runtime ------------------------------------------------------------
RUNNER=""
if command -v wasmtime >/dev/null 2>&1; then
  RUNNER="wasmtime"
elif command -v wasmer >/dev/null 2>&1; then
  RUNNER="wasmer run"
else
  echo "parity: no wasm runtime found (need wasmtime or wasmer)." >&2
  echo "        install one, e.g.:  curl https://wasmtime.dev/install.sh -sSf | bash" >&2
  exit 2
fi

echo "parity: target=$WASI_TARGET runner=$RUNNER"

NATIVE_OUT="$(mktemp)"
WASM_OUT="$(mktemp)"
trap 'rm -f "$NATIVE_OUT" "$WASM_OUT"' EXIT

# --- native reference ----------------------------------------------------------
echo "parity: rendering native..."
cargo run --release --quiet --example parity_dump > "$NATIVE_OUT"

# --- wasm32 build + run --------------------------------------------------------
echo "parity: building example for $WASI_TARGET..."
cargo build --release --quiet --example parity_dump --target "$WASI_TARGET"
WASM_BIN="target/$WASI_TARGET/release/examples/parity_dump.wasm"
echo "parity: running $WASM_BIN under $RUNNER..."
$RUNNER "$WASM_BIN" > "$WASM_OUT"

# --- diff (zero tolerance) -----------------------------------------------------
if diff -q "$NATIVE_OUT" "$WASM_OUT" >/dev/null; then
  echo "parity: PASS — native and $WASI_TARGET buffers are BIT-IDENTICAL ($(wc -l < "$NATIVE_OUT") samples)."
else
  echo "parity: FAIL — buffers diverge. First differing samples:" >&2
  diff "$NATIVE_OUT" "$WASM_OUT" | head -20 >&2
  exit 1
fi
