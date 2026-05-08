# evaporchain-light-client-wasm

**Status: scaffold only. Does not compile for wasm32 today** — blocked on a real architectural refactor of the SDK's dep graph (see "Blocking issue" below).

This crate is the intended browser-side bridge for the [`evaporchain-light-client`](../evaporchain-light-client) SDK. Browsers / mobile WASM runtimes would call into it via `wasm-bindgen` async functions, fetch chain data via `gloo-net::http::Request` (browser fetch), and run BFT BLS + Verkle Pasta-curve Pedersen verification entirely in-browser without trusting the node.

## Design (when it works)

`WasmLightClient` wraps the SDK's `LightClient` and exposes async methods:

- `WasmLightClient.anchor(node_url, genesis_height, current_time)` — fetches a header, BFT-verifies, returns a handle.
- `wlc.sync_to_latest(current_time)` — walks forward height-by-height, BFT-verifying every block.
- `wlc.fetch_and_verify_state_hex(trie_key_hex)` — fetches a state proof, verifies against the trusted state_root.

Sync/async fork resolution: rather than introducing an async `RpcTransport` trait variant in the SDK core (would be a load-bearing breaking change), this crate **bypasses** `RpcTransport` and calls the SDK's pure verifier primitives (`LightClient::ingest_block`, `state_query::verify_state`) directly. Each WASM-exported async method is `await fetch → deserialize → call sync verifier`.

The design is right. The build is what doesn't work.

## Blocking issue (2026-05-08)

`cargo build --target wasm32-unknown-unknown --release` fails on four C-build deps:

```
error: failed to run custom build command for `bzip2-sys v0.1.13+1.0.8`
error: failed to run custom build command for `lz4-sys v1.11.1+lz4-1.10.0`
error: failed to run custom build command for `libz-sys v1.1.28`
error: failed to run custom build command for `blst v0.3.16`
```

### Root cause #1: SDK pulls `evaporchain-consensus` → `evaporchain-state` → RocksDB

The SDK's `evaporchain-light-client` declares `evaporchain-consensus` as a regular (non-optional) dep so it can wrap `LightClientVerifier`. But `evaporchain-consensus` pulls `evaporchain-state` (line 10 of its Cargo.toml), which pulls `rocksdb` for the persistent backend. RocksDB transitively pulls `bzip2-sys`, `lz4-sys`, `libz-sys` for compression — all native-only via `cc`.

The SDK's docstring claim "**WASM-target compatible (with `default-features = false` when consumed)**" is **architectural aspiration, not empirical fact** — the path was never built end-to-end against `wasm32-unknown-unknown`.

### Root cause #2: BFT verification requires `blst` (BLS12-381)

`evaporchain-crypto` line 20: `blst = "0.3"`. blst is a high-performance C library; its build script compiles C source through `cc`. wasm32 needs a C-to-wasm compiler (clang with wasm target) and even then blst's assembly fast paths are x86/ARM-specific. Pure-Rust alternative exists (`bls12_381` from zkcrypto) but the chain's verifier uses blst directly.

## Path to actually shipping browser verification

Two real refactors are required. Either by itself unblocks this crate; together is cleanest.

### A. Extract a `evaporchain-consensus-types` sub-crate

`evaporchain-consensus` today mixes (a) protocol types (`LightBlockHeader`, `CommitCertificate`, `ValidatorSetSnapshot`) with (b) consensus runtime (Tendermint, mempool, fork choice, state-attached). The SDK only needs (a) for verifier composition. A new `evaporchain-consensus-types` crate that contains just the types + the BLS-verifier function would be no_std + wasm-friendly + still consumed by evaporchain-consensus' main crate.

Estimated scope: ~1-2 hours, mechanical extraction.

### B. Abstract BLS backend in `evaporchain-crypto`

Add a feature flag pattern:
```toml
[features]
default = ["bls-native"]
bls-native = ["blst"]
bls-portable = ["bls12_381"]  # pure-Rust, wasm-friendly
```

The verifier function becomes generic over the BLS backend; native consumers keep blst (~10× faster); WASM consumers use bls12_381.

Estimated scope: ~2-3 hours, feature-gate every BLS site + add a wasm regression test.

### C. (Optional) Make `evaporchain-light-client` not pull `evaporchain-state`

Today the SDK pulls `evaporchain-consensus` which transitively pulls state. With (A) done, the SDK can switch to depending on `evaporchain-consensus-types` directly, dropping evaporchain-state from its transitive graph entirely. This is the cleanest end state.

## Cross-references

- `crates/evaporchain-light-client/README.md` — SDK core. Contains the (currently aspirational) WASM compatibility claim.
- `crates/evaporchain-light-client-http/` — native HTTP transport via ureq. Works today; this WASM crate is its browser counterpart.
- `crates/evaporchain-crypto-wasm/` — existing WASM crate; ML-DSA only (no BLS), so doesn't hit the blst issue. Pattern reference for crate layout.
- `crates/evaporchain-consensus/Cargo.toml:10` — the `evaporchain-state` dep that pulls RocksDB.
- `crates/evaporchain-crypto/Cargo.toml:20` — the `blst` dep that won't compile to wasm32.

## What the scaffold contains

- Cargo.toml with the right wasm-bindgen / gloo-net / serde-wasm-bindgen / getrandom-with-js dep set
- src/lib.rs with the full `WasmLightClient` API surface — wasm-bindgen-exported async methods, panic hook, type-safe error mapping
- This README documenting the blocking issue + the two refactors that unblock it

When refactors A + B are done, this crate should build with no further changes. The scaffold is the spec.
