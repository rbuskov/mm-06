# WASM↔native parity gate

> Implements `calibration-and-testing-strategy.md` §5 — the assumption the whole
> "test natively, ship wasm" approach rests on. The two builds of `dsp_core`
> must agree **bit-for-bit**; the core is engineered for this (vendored `mathx`
> polynomials instead of libm, fast-math / FP-reordering disabled in both builds
> via `[profile.release]` in the workspace `Cargo.toml`).

**Tolerance: ZERO.** Comparison is on `f32::to_bits()` — a single differing bit
is a parity bug. The usual culprit is a transcendental that bypassed `mathx`, or
an FP-reordering flag leaking into one build.

There are two halves, mirroring §5.

---

## 1. Streaming-vs-offline equivalence — native, runs in `cargo test`

The shipped worklet (`dsp_wasm::Engine`) pushes `TrigEvent`s **one at a time** as
control frames arrive; the offline `dsp_core::render()` takes the **whole**
`events` list up front. Both schedulings of the same events must produce a
byte-identical buffer. This also proves the 8-byte protocol marshalling layer
(`dsp_core::protocol`) is bit-transparent.

This is a pure native test — no second architecture needed — and it is the
hard gate:

```
cargo test --test parity
```

Tests in `crates/dsp_core/tests/parity.rs`:

| test | proves |
|------|--------|
| `streaming_matches_offline_bit_identical` | one-frame-per-event-at-onset (via `protocol::apply_messages`) == `render()` whole-list, bit-identical |
| `protocol_path_matches_direct_api_bit_identical` | driving via 8-byte protocol frames == driving via the direct Rust API (`Engine::trigger`/setters), with a non-default control surface — the wire layer perturbs nothing |
| `streaming_block_size_invariant_bit_identical` | the streaming path is itself deterministic across render quanta |

These run as part of the normal workspace `cargo test` and MUST stay green.

---

## 2. Cross-architecture diff — native vs. `wasm32`, run manually

This is the half that needs a **second architecture**: render the same canonical
`RenderRequest` through `dsp_core::render` built for native and for `wasm32`, and
diff the raw bit streams.

- Fixture + dumper: `crates/dsp_core/examples/parity_dump.rs` — renders one
  canonical request and writes `%08x` of each sample's bits to stdout. It uses
  only `std` stdout I/O, so the **same source** compiles to native and to
  `wasm32-wasip1`.
- Driver: `scripts/parity.sh` — builds and runs both, diffs with zero tolerance.

Run it:

```
./scripts/parity.sh
```

### Requirements

- A wasm32 WASI target: `rustup target add wasm32-wasip1` (older toolchains:
  `wasm32-wasi`). The script auto-detects whichever is installed.
- A wasm runtime: **`wasmtime`** or **`wasmer`**. Install wasmtime with
  `curl https://wasmtime.dev/install.sh -sSf | bash`.

If the target or a runtime is missing, `scripts/parity.sh` prints what to install
and exits `2` (it does not fail the build). The native gate in §1 still stands on
its own.

### Status on this machine (2026-06-05)

- `wasm32-wasip1` target: **installed** (added during implementation).
- `crates/dsp_core/examples/parity_dump.rs` **compiles** for `wasm32-wasip1`
  (`target/wasm32-wasip1/release/examples/parity_dump.wasm` builds clean).
- `wasmtime` / `wasmer`: **NOT installed**, and installing a new runtime was out
  of scope for this task (no offline runtime available, piped installers
  disallowed). The cross-architecture bit-diff is therefore **scaffolded but not
  executed here** — everything up to the runtime invocation works; only the final
  `wasmtime parity_dump.wasm | diff` step is pending a runtime.

Once a runtime is present (e.g. in CI), `./scripts/parity.sh` runs the full
cross-architecture diff with no further changes.
