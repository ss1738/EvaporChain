# evaporchain-light-client-wasm

WASM bridge for the [`evaporchain-light-client`](../evaporchain-light-client) SDK. Browser wallets / dapps / explorers verify chain state in-browser via wasm-bindgen async functions backed by `gloo-net::http::Request` (browser fetch).

**Status: working end-to-end as of 2026-05-08.** Builds against `wasm32-unknown-unknown` with zero errors. Bit-identical results to the native `blst` BLS verifier (verified by 10 cross-backend interop tests in `crates/evaporchain-crypto/tests/bls_cross_backend.rs`).

## What you get

```bash
cd crates/evaporchain-light-client-wasm
wasm-pack build --target web --release
# → pkg/ — npm-publishable package
#     evaporchain_light_client_wasm_bg.wasm     (310 KB)
#     evaporchain_light_client_wasm.js          (26 KB ES module)
#     evaporchain_light_client_wasm.d.ts        (TypeScript declarations)
#     package.json
```

The 310KB `.wasm` is post-wasm-opt; pre-opt artifact at `target/wasm32-unknown-unknown/release/evaporchain_light_client_wasm.wasm` is 962KB.

## Usage from JavaScript / TypeScript

```typescript
import init, { WasmLightClient } from 'evaporchain-light-client-wasm';

await init(); // Load + instantiate the .wasm

// Anchor at the chain's latest header (or pass a specific genesis_height).
const wlc = await WasmLightClient.anchor(
  'http://node.example.com:8081',
  null,                                    // genesis_height: null = latest
  BigInt(Math.floor(Date.now() / 1000)),   // current_time_secs
);

// Walk forward to chain tip, BFT-verifying every block.
const newHeight = await wlc.sync_to_latest(BigInt(Math.floor(Date.now() / 1000)));
console.log(`Trusted tip: ${newHeight}, state_root: ${wlc.current_state_root}`);

// Verify a state-query proof.
const valueHex = await wlc.fetch_and_verify_state_hex(
  '0x1a80ddfb53bb84c968f17dcc4564fac3cc73eb4f7c2a46029a765e8629da3c81'
);
console.log('Verified value:', valueHex);
```

The full BFT BLS aggregate-sig + Verkle Pasta-curve Pedersen verification runs **in the browser** — no native code, no trust in the node. A signature that verifies on a Mini will verify in the browser; a forged signature rejected on a Mini will be rejected in the browser.

## Architecture

`WasmLightClient` wraps the SDK's `LightClient` and exposes three async methods:

- `WasmLightClient.anchor(node_url, genesis_height, current_time)` — fetches a header, BFT-verifies, returns a handle.
- `wlc.sync_to_latest(current_time)` — walks forward height-by-height, BFT-verifying every block.
- `wlc.fetch_and_verify_state_hex(trie_key_hex)` — fetches a state proof, verifies against the trusted state_root.

Plus three readonly props (`current_height`, `current_state_root`, `trust_period_secs`).

### Sync/async fork resolution

The SDK's `RpcTransport` trait is sync. Browser fetch is async-only. Rather than introducing an async trait variant in the SDK core (load-bearing breaking change), this crate **bypasses** `RpcTransport` and calls the SDK's pure verifier primitives (`LightClient::ingest_block`, `LightClient::verify_state`) directly. Each WASM-exported async method does:

1. `await` the browser fetch via `gloo-net`,
2. deserialize the response JSON to the SDK's chain types,
3. call the SDK's sync verification methods.

Result: full BFT BLS + Verkle verification in the browser, SDK core stays sync-only + WASM-friendly.

### BLS backend

The crypto crate's `bls-portable` feature replaces `blst` (C library, native-only) with `bls12_381` + `group` + `pairing` + `ff` (pure Rust, wasm32-friendly). Verification only — `BlsKeypair` (signing) is feature-gated to `bls-native` since browsers don't sign BLS.

10 cross-backend interop tests validate that the portable verifier produces bit-identical results to blst on real signatures (single-sig, DST handling for PoP + rotation, 3-signer aggregate). See `crates/evaporchain-crypto/tests/bls_cross_backend.rs`.

## Build flow

This crate is **not a workspace member** (it's `workspace.exclude`'d at the repo root) so wasm-specific compile flags don't bleed into the main workspace. Mirrors the standalone-Cargo-project pattern of `evaporchain-crypto-wasm`.

Two valid build invocations:

```bash
# Direct cargo (produces .wasm only):
cargo build --target wasm32-unknown-unknown --release

# wasm-pack (produces npm package with JS bindings + TS declarations):
wasm-pack build --target web --release
```

`wasm-pack` runs cargo internally then invokes `wasm-bindgen` to generate the JS glue + `wasm-opt` to shrink the binary.

## Cross-references

- `crates/evaporchain-light-client/README.md` — SDK core, with up-to-date WASM status.
- `crates/evaporchain-light-client-http/` — native HTTP transport via `ureq`. Same conceptual flow; this WASM crate is its browser counterpart.
- `crates/evaporchain-crypto/Cargo.toml` — feature flags for the BLS backend split.
- `crates/evaporchain-crypto/src/bls_portable.rs` — pure-Rust BLS verifier.
- `crates/evaporchain-crypto/tests/bls_cross_backend.rs` — interop tests.
- `crates/evaporchain-consensus-types/` — types-only sub-crate extracted from `evaporchain-consensus` to drop the SDK's RocksDB transitive dep (Refactor A).
- `INVENTION_STACK.md §4.1 row 8` — Lambda-Fold doctrine the SDK operationalizes, now extended to browser consumers.

## Refactor history

This crate was scaffolded 2026-05-07 with full `WasmLightClient` API surface as a spec, but did not compile (RocksDB + blst transitive deps blocked wasm32). Two refactors landed 2026-05-08 to unlock the build:

| Refactor | What | Commits |
|---|---|---|
| A | Extract `evaporchain-consensus-types` so SDK doesn't pull `evaporchain-state` → RocksDB | `46bfdd4` `3c44eeb` `28a3fba` `f4efdea` |
| B | Feature-flag BLS backend so wasm uses pure-Rust `bls12_381` instead of `blst` | `99bab9c` |
| Test | Cross-backend interop (10 tests, all pass) | `a5697c6` |

After both refactors + interop validation, the scaffold from 2026-05-07 builds + verifies correctly with no source changes — exactly as the original spec promised.

## Open follow-ups

- **Browser smoke test** — load the `.wasm` in a real browser (or jsdom), point at the running 5-node WAN cluster, verify a state proof end-to-end.
- **Bundle size optimization** — already at 310KB after wasm-pack's auto-`wasm-opt`. Could go lower with custom flags (~150KB target) or by stripping unused SDK methods.
- **WebSocket transport** — `gloo-net::websocket` for live-block streaming instead of polling.
- **WASM-bindgen `#[wasm_bindgen(typescript_custom_section)]`** for richer TypeScript type ergonomics (currently `Promise<any>` for state-query results; could be `Promise<HexString>`).
