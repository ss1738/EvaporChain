# evaporchain-paymaster

Off-chain sponsorship service for EvaporChain `UserOpTx` transactions. Implements the wallet-facing side of [Option B](../../docs/MULTI_TOKEN_GAS_OPTIONS.md) — a paymaster account holds EVP, signs sponsorship messages on a wallet's behalf, and the chain debits gas from the paymaster instead of the user.

## What's in this crate

| Surface | Purpose |
|---|---|
| `Paymaster` (lib) | Holds the keypair, derives its account address (`blake3(pk)`), allocates monotonic sponsorship nonces with crash-safe persistence, signs the canonical sponsorship payload, optionally rate-limits per sender, optionally writes a billing audit log, optionally caches idempotency keys (with cross-restart persistence). |
| `PaymasterConfig` / `AuditFsyncMode` / `InnerVariant` / `SponsorOutcome` (lib) | Operator-facing knobs for hardening (require_user_sig, per_sender_rps/burst, audit_log + audit_log_fsync, allowed_inner_variants, idempotency_max_keys/ttl/persist_path). |
| `SponsorshipRequest` / `SponsorshipResponse` / `PaymasterInfo` (lib) | Wire types matching `wallet::paymaster::PaymasterClient`. |
| `reconcile` module (lib) | Startup + runtime nonce reconciliation against the chain (`check_alignment`, `run_one_cycle`, `NonceAlignment` enum). |
| `evaporchain-paymaster` (bin) | axum HTTP server: `GET /healthz`, `GET /info`, `GET /metrics` (Prometheus), `POST /sponsor`. SIGHUP reopens the audit log; tokio task polls reconciliation. |
| `load_keypair_from_file` / `generate_keypair_to_file` | Convenience helpers for first-run / dev. Production should integrate a KMS instead. |

## Quick start

```bash
cargo build --release --bin evaporchain-paymaster
./target/release/evaporchain-paymaster \
  --chain-id evaporchain-mainnet \
  --listen 127.0.0.1:8088 \
  --chain-rpc-url http://node-1:8081 \
  --strict-reconcile \
  --audit-log /var/log/paymaster.jsonl \
  --idempotency-persist-path /var/lib/paymaster/idempotency.json \
  --generate-keypair-if-missing
```

Read the address from `GET /info`, fund it on-chain, point a wallet at the URL. Production paymasters should set `--strict-reconcile` to refuse startup on chain-vs-local nonce drift; testnet operators may prefer `--audit-log-fsync none` for ~10× throughput at the cost of audit-log durability.

## Library use

```rust
use evaporchain_paymaster::{Paymaster, PaymasterConfig, load_keypair_from_file};

let kp = load_keypair_from_file("paymaster_keypair.json".as_ref())?;
let pm = Paymaster::new_with_config(
    kp,
    "evaporchain-mainnet".to_string(),
    "paymaster_nonce",
    PaymasterConfig::default(), // strict, rate-limited, idempotent
)?;

// `user_op` arrives from a wallet with paymaster fields blank but
// the user signature already stamped (under strict mode).
let outcome = pm.sponsor_idempotent(Some("idempotency-key-123"), &mut user_op)?;
match outcome {
    SponsorOutcome::Fresh { paymaster_nonce } => {
        // First time we saw this key — fresh allocation.
    }
    SponsorOutcome::Replay { paymaster_nonce } => {
        // Wallet retry under same key — cached response replayed.
    }
}
```

The chain enforces consent-to-sponsor by verifying `blake3(paymaster_public_key) == paymaster` AND `HybridVerifier::verify(canonical_payload, paymaster_signature, paymaster_public_key)`. Both checks happen unconditionally in `execute_user_op` regardless of the global `verify_signatures` flag.

## Operator runbook

[`docs/runbooks/paymaster.md`](../../docs/runbooks/paymaster.md) — ~400-line operator guide covering first-run setup, every CLI flag, monitoring (Prometheus alerts), security threats and closures, failure modes, audit log, idempotency, runtime reconciliation, SIGHUP rotation, live-cluster smoke procedure.

## Test surface

```bash
cargo test -p evaporchain-paymaster
```

54 tests covering: nonce monotonicity + persistence, address derivation, sponsorship sig roundtripping under chain rules, idempotency cache (LRU + TTL + cross-restart persistence + replay metric), rate limiter (per-sender bucket + GC), strict-mode user-sig pre-check (4 corners), audit log (write, persist across restart, call_data_hash binding, fsync modes), inner-variant whitelist, /info policy surface (incl. backwards-compat decode), Prometheus exposition format, startup + runtime nonce reconciliation, SIGHUP audit-log reopen.

## Related crates

- `evaporchain-types::UserOpTx` + `paymaster_sponsorship_payload(chain_id)` — the canonical bytes the paymaster signs.
- `evaporchain-execution::SimpleExecutor::execute_user_op` — the chain-side enforcer (`crates/evaporchain-execution/src/lib.rs`).
- `evaporchain-wallet::paymaster::PaymasterClient` — wallet-side async client.

## See also

- Decision doc: [`docs/MULTI_TOKEN_GAS_OPTIONS.md`](../../docs/MULTI_TOKEN_GAS_OPTIONS.md) (Option A vs B vs C)
- E2E test: [`tests/integration/src/paymaster_e2e.rs`](../../tests/integration/src/paymaster_e2e.rs) (4 tests including strict-mode happy-path + tampered-call_data rejection)
- Build arc journal: SESSION_PROGRESS entries `7242e59` (Days 1–5), `0231e75` (Days 6–12B), `1e48720` (audit arc).
- Formal ship log: CHANGELOG.md (audit-arc entry: 7 V1-blocker fixes across 8 commits).
