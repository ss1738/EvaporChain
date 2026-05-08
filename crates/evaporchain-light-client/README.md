# evaporchain-light-client

Light Client SDK for [EvaporChain](https://github.com/ss1738/EvaporChain). Verify chain state from the outside — wallets, dapps, bridges, explorers, embedded verifiers — without running a full validator.

## What it does

Three verification layers, composed into a single `LightClient`:

| Layer | Mechanism | Cost | Feature |
|---|---|---|---|
| **BFT commit-certificate** | BLS aggregate signature over a Tendermint-style `CommitCertificate` proving ≥2/3 stake attested. Trust-period tracking per ICS-007. | Constant per block | Always on |
| **Nova-IVC sublinear validity** | Single Nova-IVC fold-attestation verifies block validity at any chain length in a fixed-size proof. Light client holds only `vk_bytes` (~few KB). | ~23 ms regardless of chain length (1.083× of 10 folds, locked Phase 6.1) | `nova` feature |
| **Verkle state-query** | Pasta-curve Pedersen-commitment proofs (`EnergyVerkleProof`) bound to the trusted state root. | Constant per query | Always on |

## Quickstart

```rust
use evaporchain_light_client::{LightClient, RpcTransport};
use evaporchain_light_client_http::HttpTransport;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let transport = HttpTransport::new("http://my-node.example.com:8081");
let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)?
    .as_secs();

// Anchor at a height you trust (e.g., genesis, or a checkpoint).
let genesis = transport.fetch_header_at(15190)?;
let mut lc = LightClient::new(genesis, now, /* vk_bytes */ None);

// Walk forward to chain tip, BFT-verifying every step.
lc.sync_to_latest(&transport, now)?;

// Query state. The proof is verified against the trusted state root.
let key = [0u8; 32]; // your trie key
let value = lc.fetch_and_verify_state(&transport, &key, /* expected */ None)?;
println!("verified value: {value:?}");
# Ok(()) }
```

## Crates in this family

| Crate | Purpose |
|---|---|
| `evaporchain-light-client` (this) | SDK core. Verifier composition, transport-agnostic. WASM-friendly. |
| `evaporchain-light-client-http` | HTTP transport via `ureq`. Drop-in for native consumers. |
| `evaporchain-light-client-cli` | `evaporchain-light-client` binary — operator CLI + worked example. |

The core has **no HTTP client and no async runtime**. Bring your own `RpcTransport` impl for non-HTTP transports (browser fetch, mobile FFI, gRPC, websocket, etc.).

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `nova` | off | Adds Nova-IVC sublinear block-validity verification via `evaporchain-lambda-fold` + `evaporchain-proving`. Without it, you still get BFT + Verkle — useful and ~10× lighter on dependencies. |

## WASM target — aspirational, NOT yet shipped

The core's design intent is wasm32-friendly (no async-trait, no native-only deps in evaporchain-light-client itself), and `default-features = []`. **However, the dep graph pulls `evaporchain-consensus` → `evaporchain-state` → RocksDB, AND `evaporchain-crypto` → `blst` (C library) — neither compiles for `wasm32-unknown-unknown` today.**

Two architectural refactors are needed to actually ship browser verification:
1. Extract `evaporchain-consensus-types` (types-only, no state-DB).
2. Abstract BLS backend in `evaporchain-crypto` (blst native, `bls12_381` wasm).

A scaffolded WASM bridge crate exists at [`evaporchain-light-client-wasm`](../evaporchain-light-client-wasm) — full `WasmLightClient` API surface + gloo-net fetch wiring. Does not compile yet; serves as the spec for what the refactors enable. See that crate's README for the full diagnosis + proposed refactor sequence.

## Verification semantics

- **BFT verification** is mandatory and unconditional. Every header ingested via `ingest_block` is rejected unless the BLS aggregate sig verifies against the validator-set's public keys with ≥2/3 stake.
- **State proofs** verify against the trusted tip's `state_root` — the same root the BFT layer just attested. A node lying about state will be caught at proof verification time.
- **Trust-period expiry** follows ICS-007 / Tendermint convention. Default 14 days; configurable via `LightClient::with_trust_period`.
- **Parent-hash chaining is intentionally not enforced** at the SDK boundary. Authentication relies on the BLS aggregate sig, not a hash chain. (Chain producer-side `block.parent_hash` uses a recursive blake3 formula different from `cert.block_hash` — discovered during 5-node WAN cluster validation.)

## Cross-references

- `INVENTION_STACK.md §4.1 row 8` — Lambda-Fold doctrine the SDK operationalizes.
- `LAMBDA_FOLD_NOVA_PLAN.md` — Phase 5 Tendermint-side Nova integration consumed here.
- `crates/evaporchain-consensus/src/light_client.rs` — the BFT verifier this SDK wraps.
- `crates/evaporchain-proving/src/nova.rs` — the Nova verifier this SDK wraps.
- `docs/runbooks/light-client-cli.md` — operator runbook for the CLI binary.

## Stability

Workspace-versioned alongside the rest of EvaporChain. Pre-mainnet — semver guarantees apply post-1.0. Trait shapes (`RpcTransport`, `LightClient` constructors) are stable as of the 5-node WAN validation; new methods may be added.

## License

Workspace-licensed.
