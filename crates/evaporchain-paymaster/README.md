# evaporchain-paymaster

Off-chain sponsorship service for EvaporChain `UserOpTx` transactions. Implements the wallet-facing side of [Option B](../../docs/MULTI_TOKEN_GAS_OPTIONS.md) — a paymaster account holds EVP, signs sponsorship messages on a wallet's behalf, and the chain debits gas from the paymaster instead of the user.

## What's in this crate

| Surface | Purpose |
|---|---|
| `Paymaster` (lib) | Holds the keypair, derives its account address (`blake3(pk)`), allocates monotonic sponsorship nonces with crash-safe persistence, signs the canonical sponsorship payload. |
| `SponsorshipRequest` / `SponsorshipResponse` / `PaymasterInfo` (lib) | Wire types matching `wallet::paymaster::PaymasterClient`. |
| `evaporchain-paymaster` (bin) | axum HTTP server exposing `GET /healthz`, `GET /info`, `POST /sponsor`. |
| `load_keypair_from_file` / `generate_keypair_to_file` | Convenience helpers for first-run / dev. Production should integrate a KMS instead. |

## Quick start

```bash
cargo build --release --bin evaporchain-paymaster
./target/release/evaporchain-paymaster \
  --chain-id evaporchain-mainnet \
  --listen 127.0.0.1:8088 \
  --generate-keypair-if-missing
```

Read the address from `GET /info`, fund it on-chain, point a wallet at the URL.

## Library use

```rust
use evaporchain_paymaster::{Paymaster, load_keypair_from_file};

let kp = load_keypair_from_file("paymaster_keypair.json".as_ref())?;
let pm = Paymaster::new(kp, "evaporchain-mainnet".to_string(), "paymaster_nonce")?;

// `user_op` arrives from a wallet with paymaster fields blank.
let assigned_nonce = pm.sponsor(&mut user_op)?;
// Now `user_op.paymaster`, `paymaster_nonce`, `paymaster_signature`,
// `paymaster_public_key` are all set.
```

The chain enforces consent-to-sponsor by verifying `blake3(paymaster_public_key) == paymaster` AND `HybridVerifier::verify(canonical_payload, paymaster_signature, paymaster_public_key)`. Both checks happen unconditionally in `execute_user_op` regardless of the global `verify_signatures` flag.

## Operator runbook

[`docs/runbooks/paymaster.md`](../../docs/runbooks/paymaster.md) — first-run setup, deployment, monitoring, security, failure modes, live-cluster smoke procedure.

## Test surface

```bash
cargo test -p evaporchain-paymaster
```

Five tests covering nonce monotonicity + persistence, address derivation, signature roundtripping under chain rules, refusal to overwrite an existing signature.

## Related crates

- `evaporchain-types::UserOpTx` + `paymaster_sponsorship_payload(chain_id)` — the canonical bytes the paymaster signs.
- `evaporchain-execution::SimpleExecutor::execute_user_op` — the chain-side enforcer (`crates/evaporchain-execution/src/lib.rs`, search `"Day 1B"`).
- `evaporchain-wallet::paymaster::PaymasterClient` — wallet-side async client.

## See also

- Decision doc: [`docs/MULTI_TOKEN_GAS_OPTIONS.md`](../../docs/MULTI_TOKEN_GAS_OPTIONS.md) (Option A vs B vs C)
- Verification strategy: [`docs/MULTI_TOKEN_GAS_VERIFICATION.md`](../../docs/MULTI_TOKEN_GAS_VERIFICATION.md)
- E2E test: [`tests/integration/src/paymaster_e2e.rs`](../../tests/integration/src/paymaster_e2e.rs)
