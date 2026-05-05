# EvaporChain Wallet — Browser Extension

Post-quantum browser-extension wallet for EvaporChain. Manages keys, signs transactions, exposes `window.evaporchain` to dApps, and renders live energy-decay state for owned objects.

## Status

Pre-1.0 (`v0.1.0`). Chrome MV3 build works; cross-browser polish pending.

## Stack

- **Chrome Manifest V3** (background service worker + popup + content script + injected provider)
- **React 19** + Zustand for popup state
- **Vite** + TypeScript
- **`@noble/hashes` + `@scure/bip39`** for mnemonics
- **ML-DSA Dilithium3** signing via the WASM build of `evaporchain-crypto-wasm` (compiled from the Rust workspace; see `scripts/build-wasm.sh` and `scripts/verify-wasm.mjs`)
- **Playwright** for E2E

## Build

```bash
cd extension

# Build the ML-DSA WASM binding (one-time per Rust change)
npm run build:wasm     # invokes scripts/build-wasm.sh
npm run verify:wasm    # checks the WASM artifact against scripts/wasm-build-versions.json

# Build the extension itself
npm install
npm run build          # writes dist/

# Watch mode for development
npm run dev
```

## Reproducible builds

The signing-critical path (ML-DSA Dilithium3 via `evaporchain-crypto-wasm`) ships with a deterministic build pipeline so any reviewer can rebuild the WASM from source and verify it matches the artifact in `dist/`:

- `scripts/build-wasm.sh` — pinned Rust toolchain + `wasm-pack` invocation
- `scripts/wasm-build-versions.json` — version-pin manifest (toolchain, deps, output hash)
- `scripts/verify-wasm.mjs` — verifier that rebuilds, hashes, and compares against the manifest
- `npm run verify:wasm` — runs the verifier in CI and locally before every release

This is the user-protective property an auditor cares about: *the wallet a user installs from the Chrome Web Store is bit-identical to the wallet rebuilt from this repo at the tagged commit.* See [`scripts/README.md`](./scripts/README.md) for the full pipeline.

## Load into Chrome

1. `chrome://extensions` → enable **Developer mode**.
2. Click **Load unpacked** → pick `extension/dist/`.
3. Pin the **EvaporChain Wallet** action to the toolbar.

## Test

```bash
npm test               # vitest unit tests
npm run test:e2e       # Playwright E2E (requires a built dist)
npm run lint
```

## What it ships

| Surface | Path |
|---|---|
| Popup UI (account list, send, receive, decay viewer) | `src/popup/` |
| Content script + injected provider (`window.evaporchain`) | `src/content/`, `src/provider/` |
| Service worker (request routing, signing) | `src/background/` |
| ML-DSA + BLAKE3 + key derivation | `src/crypto/` |
| dApp-facing message protocol | `src/provider/` |

## How dApps talk to it

dApps consume the wallet via the [`@evaporchain/wallet-sdk`](../wallet-sdk/) package, which detects `window.evaporchain` and wraps the connect / sign / send flow in a typed API plus React hooks. See `wallet-sdk/README.md`.

## License

MIT
