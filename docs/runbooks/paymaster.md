# Paymaster Runbook

How to run an `evaporchain-paymaster` sponsorship service so wallets can submit `UserOpTx` transactions where someone other than the sender pays gas. Intended audience: operators running a paymaster (foundation-run for V1, third-party from V1.5+).

**Pairs with:** `docs/MULTI_TOKEN_GAS_OPTIONS.md` (the decision doc — Option B), `crates/evaporchain-paymaster/` (the binary + library), `wallet/src/paymaster.rs` (the wallet client).

**See commits:** `dc89531` (chain-side sponsorship-sig verification), `3ccf4f7` (call_data dispatch), `cd64a3b` (service crate), `2337d63` (wallet client), `85effec` (E2E test).

---

## What the paymaster does

A paymaster is an account funded with EVP that signs sponsorship messages. When a wallet wants to submit a `UserOpTx` and have someone else pay the gas, it:

1. Builds the `UserOpTx` body (sender, sender_nonce, call_data, call_gas_limit) — paymaster fields blank.
2. POSTs that body to `POST /sponsor` on a paymaster service.
3. The paymaster signs the canonical sponsorship payload (chain-id-bound, blake3(call_data)-bound) and stamps four fields onto the UserOp: `paymaster`, `paymaster_nonce`, `paymaster_signature`, `paymaster_public_key`.
4. The wallet posts the now-fully-signed UserOp to the chain's tx endpoint.
5. The chain's `execute_user_op` verifies (a) `blake3(paymaster_public_key) == paymaster` and (b) `HybridVerifier::verify(canonical_payload, paymaster_signature, paymaster_public_key)`. If both pass, gas is debited from the paymaster's balance instead of the sender's.

The paymaster does **not** submit the UserOp to the chain on behalf of the wallet — it only signs sponsorship. Submission and inner-tx execution happen at the chain layer using the wallet's user-side signature.

The paymaster does **not** validate the user's intent. A malformed inner Transaction (e.g. wrong sender, insufficient balance) will land in the block but execute as `tx_failed=1`; the paymaster has paid gas for a no-op. The doc-level recommendation is for paymasters to do **off-band** validation (KYC, anti-spam, per-account budget) before signing — V1 ships unconditionally to keep the binary minimal.

---

## Build

```bash
cd ~/EvaporChain
cargo build --release --bin evaporchain-paymaster
```

Binary lands at `target/release/evaporchain-paymaster`. Pure-Rust, ~12 MB. Per project convention, build on a Mini via SSH — never on the MacBook (`make` and `cargo` are SSH-only on the M4 cluster, see project `CLAUDE.md`).

---

## First-run setup

### 1. Generate a paymaster keypair

```bash
target/release/evaporchain-paymaster \
  --keypair-file paymaster_keypair.json \
  --nonce-file   paymaster_nonce \
  --chain-id     evaporchain-mainnet \
  --listen       127.0.0.1:8088 \
  --generate-keypair-if-missing
```

The `--generate-keypair-if-missing` flag mints a fresh hybrid keypair (Ed25519 + ML-DSA-65) and writes it to `paymaster_keypair.json` if the file doesn't exist. The keypair file format is:

```json
{
  "ecdsa_secret_hex": "...",
  "mldsa_public_hex": "...",
  "mldsa_secret_hex": "..."
}
```

The paymaster's account address is `blake3(public_key_bytes)`. Read it from the log line `paymaster ready paymaster_address=...` or via `GET /info` after the service is up.

**File permissions matter.** A leaked keypair lets anyone drain the paymaster up to its on-chain nonce-allocation horizon. `chmod 600 paymaster_keypair.json` before the service starts. Production deployments should swap the file load for a KMS / secret-manager fetch — `evaporchain-paymaster` exposes `Paymaster::new(HybridKeypair, chain_id, nonce_file)` as a library entrypoint so an operator can write a thin wrapper that pulls the key from elsewhere and never lands it on disk.

### 2. Fund the paymaster account on chain

The paymaster's balance is what gets debited per sponsorship. Send EVP from a funded address to `<paymaster_address>` via the standard `POST /api/tx/transfer` endpoint on the chain (or the wallet's `transfer` subcommand). The amount per sponsorship is `call_gas_limit + GAS_USER_OP` (`GAS_USER_OP = 30_000` per `crates/evaporchain-execution/src/lib.rs`). For a paymaster expecting ~1000 transfers/day at `call_gas_limit = 50_000`, budget ~80M EVP/day — top up weekly.

**Don't reuse the paymaster account for anything else.** The on-chain `account.nonce` doubles as the sponsorship counter; mixing a sponsorship paymaster with a normal-tx sender will desync the local `paymaster_nonce` file from chain state and cause every subsequent `/sponsor` to fail.

### 3. Confirm the chain accepts the paymaster's address

```bash
curl http://localhost:8088/info
# {"paymaster_address_hex":"...","next_paymaster_nonce":0,"chain_id":"evaporchain-mainnet"}
```

Cross-check `paymaster_address_hex` against `GET /api/account/<addr>` on the chain — the funded balance should match what you sent in step 2.

---

## Running the service

### CLI flags

| Flag | Required | Default | Purpose |
|---|---|---|---|
| `--keypair-file PATH` | no | `paymaster_keypair.json` | Hybrid keypair JSON |
| `--nonce-file PATH` | no | `paymaster_nonce` | Persisted sponsorship counter (atomic write + fsync per bump) |
| `--chain-id ID` | **yes** | — | Must match the chain's chain_id; sponsorship sigs are chain-id-bound |
| `--listen ADDR` | no | `0.0.0.0:8088` | TCP listen address |
| `--generate-keypair-if-missing` | no | off | First-run convenience; off in prod so a missing file fails loudly |
| `--disable-user-sig-check` | no | off | Day 7 hardening: skip the inbound-UserOp user-signature pre-check. Only safe for testnet / dev — leave OFF in prod (see §Threat: spam-signing) |
| `--per-sender-rps RATE` | no | `5.0` | Per-`UserOp.sender` token-bucket replenish rate. `0` disables the rate limiter |
| `--per-sender-burst N` | no | `10` | Per-sender burst capacity |
| `--audit-log PATH` | no | (off) | Append-only JSON-lines audit log path. One line per successful sponsorship — see §Audit log |

### Endpoints

| Method | Path | Body | Returns |
|---|---|---|---|
| GET | `/healthz` | — | `"ok"` |
| GET | `/info` | — | `{paymaster_address_hex, next_paymaster_nonce, chain_id}` |
| POST | `/sponsor` | `{user_op}` | `{user_op, paymaster_address_hex, paymaster_nonce}` |

### Logs

Default `RUST_LOG=info`. Set `RUST_LOG=evaporchain_paymaster=debug` for per-request detail.

```bash
RUST_LOG=info target/release/evaporchain-paymaster \
  --chain-id evaporchain-mainnet --listen 127.0.0.1:8088
```

---

## Wallet integration

```bash
# 1. Wallet builds half-formed UserOpTx (sender, sender_nonce, call_data, call_gas_limit)
# 2. Wallet POSTs to /sponsor:
curl -X POST http://localhost:8088/sponsor \
  -H 'content-type: application/json' \
  -d @user_op.json
# returns: {user_op, paymaster_address_hex, paymaster_nonce}

# 3. Wallet stamps its own sender signature on the returned user_op (signing message
#    is the standard Transaction::UserOp signing_message — the chain's tx-validity
#    check still requires the user's sig).
# 4. Wallet POSTs the doubly-signed UserOpTx to the chain's tx endpoint.
```

For Rust integration, use `wallet::paymaster::PaymasterClient` (`wallet/src/paymaster.rs`) — async info/sponsor methods backed by `reqwest`.

---

## Live-cluster smoke procedure

Use this to verify a paymaster works against a running cluster before pointing real wallets at it. Operator-driven; do **not** run against the production cluster without an existing funded paymaster account that you control.

```bash
# On a Mini (binary already built):
mkdir -p ~/paymaster-data && cd ~/paymaster-data

# 1. Generate keypair, start service.
~/EvaporChain/target/release/evaporchain-paymaster \
  --chain-id evaporchain-mainnet \
  --listen 127.0.0.1:8088 \
  --generate-keypair-if-missing &
PM_PID=$!

# 2. Read the paymaster address.
PM_ADDR=$(curl -s http://localhost:8088/info | jq -r .paymaster_address_hex)
echo "paymaster: 0x$PM_ADDR"

# 3. Fund it from a wallet you control (replace SENDER, NODE).
~/EvaporChain/target/release/evaporchain-wallet transfer \
  --from $SENDER --to "0x$PM_ADDR" --amount 100000000 --node $NODE
# Wait for finality (~6s).

# 4. Build a minimal sponsored Transfer via curl + jq, POST to /sponsor.
# (See docs/MULTI_TOKEN_GAS_OPTIONS.md §4 step 3 for the full request shape.)

# 5. Submit the returned UserOp to the chain. Check the recipient credit + the
#    paymaster's balance both moved in the same block.

# 6. Tear down.
kill $PM_PID
```

The deferred Day 4 "live cluster smoke" item from the build is captured here. The in-process E2E test (`tests/integration/src/paymaster_e2e.rs`) covers the same wire path under a real `execute_block`; this procedure adds real-consensus, real-network delivery on top.

---

## Operations

### Restart

The `paymaster_nonce` file is atomically written on every successful `/sponsor`. A restart picks up the next-nonce from the file, so the in-memory counter resumes exactly where the previous instance left off. **Never** edit the nonce file by hand — if it desyncs from chain state, the service will start producing sponsorship sigs the chain rejects (`InvalidNonce`).

If you need to reset (e.g. fresh paymaster account), delete both `paymaster_nonce` and `paymaster_keypair.json` and re-run with `--generate-keypair-if-missing`. The new paymaster has a different on-chain address; fund it separately.

### Monitoring

- `GET /healthz` for liveness probes.
- `GET /info` exposes `next_paymaster_nonce`. The on-chain `account.nonce` for the paymaster address should always be `next_paymaster_nonce - <in-flight sponsored UserOps not yet finalized>`. A persistent gap > a few blocks signals dropped UserOps or a chain reorg.
- Tail the paymaster log for `sponsor failed` lines — every entry is a wallet bug or a wallet-side abuse attempt (e.g. resubmitting an `AlreadySigned` UserOp).

### Audit log

When `--audit-log PATH` is set, every successful sponsorship appends one line to the file in JSON-lines format:

```json
{"ts_unix_ms":1715200000000,"sender":"0x01..","paymaster_nonce":42,"call_gas_limit":50000,"call_data_hash":"0xab..","chain_id":"evaporchain-mainnet"}
```

Use cases:
- **Billing reconciliation.** A paymaster charging users in another token (USDC, ETH, fiat) matches the audit lines against off-chain payments. The `(sender, paymaster_nonce)` pair is unique; `call_data_hash` is bit-identical to what the chain's `paymaster_sponsorship_payload` binds, so audit lines correspond 1:1 to on-chain sponsored UserOps that landed.
- **Forensics.** A paymaster operator suspecting abuse can grep the log by `sender` to see all sponsorships from a given address.
- **Stuck-tx triage.** Cross-reference `paymaster_nonce` in the audit line against the chain's account.nonce for the paymaster address — gaps signal sponsored UserOps that the chain rejected (would otherwise have advanced the nonce).

Operational notes:
- Each line is followed by an explicit `fsync(2)` so a crash never loses a sponsored entry. Disk-write latency adds ~1 ms to `/sponsor` p99 on standard SSDs.
- **Fail-closed:** an audit-log IO error (full disk, no permission) returns `503 paymaster IO`. The paymaster does NOT silently sponsor without writing the line — that would be a billing-reconciliation hole. Operators who need to keep sponsoring through audit-log outages should rotate the log path with `--audit-log` before unblocking.
- The file is opened with `O_APPEND` so concurrent writers (across worker threads in this process) are kernel-serialised at line boundaries. Multiple paymaster processes pointing at the same audit file work too — the per-line atomicity is OS-enforced for sub-PIPE_BUF writes.
- Rotate via `mv audit.jsonl audit.jsonl.$(date +%F)` then `kill -HUP` (currently no-op — daemon would re-open on SIGHUP in a future v1.5 hardening). For now: stop the service, rotate, restart.

### Concurrency

The internal nonce counter is `Mutex`-guarded across (allocate, persist, sign). Per-request latency is dominated by the ML-DSA sign step (~1 ms on M4); under contention, requests serialize on the mutex. Throughput ceiling is ~1000 sponsorships/sec per process; horizontally scaling requires multiple paymaster accounts (one per process), since two processes sharing a nonce file would race.

---

## Security

### Threat: forged-paymaster drain

**Closed by `dc89531`.** The chain enforces `paymaster_signature` verification before debiting the paymaster's balance. Without that fix, any user could submit a UserOp with `paymaster: <victim>` and drain a victim. With the fix, the paymaster's hybrid signature is required, and `blake3(paymaster_public_key)` must derive to the paymaster address — so only the paymaster's actual key holder can authorize debits.

### Threat: tampered call_data after sponsorship

**Closed by the canonical sponsorship payload.** The payload binds `blake3(call_data)`. Swapping the inner Transaction after the paymaster signs invalidates the signature. The Day 4 E2E test (`paymaster_e2e_rejects_tampered_call_data_at_chain_layer` in `tests/integration/src/paymaster_e2e.rs`) verifies this end-to-end.

### Threat: sponsorship nonce replay

**Closed by `paymaster_nonce`** (Phase 4.1, 2026-05-03). The chain checks the paymaster account's on-chain nonce equals `tx.paymaster_nonce` and bumps it. Replaying a UserOp with the same paymaster_nonce fails `InvalidNonce`. The `paymaster_nonce` file ensures the service never re-issues a nonce after restart.

### Threat: keypair theft

The keypair file is the security boundary. A leaked keypair lets the attacker drain the paymaster up to the unspent-funds balance. Mitigations:

- `chmod 600 paymaster_keypair.json`, owner-only.
- Run the paymaster as a non-root user.
- Cold-store the keypair generation; production should integrate with a KMS rather than a flat file. `Paymaster::new(HybridKeypair, ...)` accepts an in-memory keypair so the file path is optional at the library level.
- Rotate paymaster addresses periodically — fund a new paymaster, redirect wallet `--paymaster-url`, retire the old one.

### Threat: spam-signing

A malicious wallet can flood `/sponsor` and burn through the paymaster's balance unless the service rejects bad/abusive requests before signing. The Day 7 hardening closes this in two layers (both on by default in production builds):

**Layer 1 — Mandatory user-signature pre-check** (`require_user_sig: true`, default). The paymaster verifies the inbound `UserOpTx`'s user-side signature against the same canonical message the chain checks (`Transaction::UserOp(user_op).signing_message(chain_id)`) BEFORE allocating a sponsorship nonce or signing. A wallet sending malformed sigs gets rejected with `400` and pays no paymaster gas. Disable only for testnet via `--disable-user-sig-check`.

The pre-check also enforces that the user signed for THIS paymaster: the service overwrites `user_op.paymaster` with its own address before the signature check, so a UserOp signed for some-other-paymaster fails verification and rejects.

**Layer 2 — Per-sender token-bucket rate limit** (`per_sender_rps: 5.0`, `per_sender_burst: 10`, defaults). Each `UserOp.sender` gets its own bucket; new senders start full. Replenish rate `--per-sender-rps` (sponsorships/sec) and burst `--per-sender-burst` are tunable. Set `--per-sender-rps 0` to disable. Hits surface as `429 Too Many Requests` so wallets can back off cleanly. Idle buckets GC'd after 10 min so the HashMap stays bounded by active senders, not historical total.

**Still V1.5 / operator-specific (not in the binary):**

- Whitelist of permitted inner-tx variants per paymaster operator (chain enforces a global whitelist; operators may want a tighter subset).
- Off-band payment-confirmation gate (the Option B "user reimburses paymaster in token X" flow — needs a billing system per operator).

---

## Competing paymasters

Multiple paymasters can run simultaneously against the same chain. Wallets pick which paymaster to use per UserOp via the `paymaster` address they stamp on the UserOp body. There's no on-chain registry of paymasters in V1; discovery is out-of-band (DNS, wallet config, foundation directory).

To run a competing paymaster:

1. Generate a fresh keypair (steps above).
2. Fund the new paymaster account with EVP.
3. Run the binary with your own `--listen` and `--chain-id`.
4. Publish the paymaster's URL + address (e.g. on a public web page or via a wallet config endpoint).
5. Wallets that prefer your paymaster point their config at your URL.

There's no ranking, fees-on-chain, or "preferred paymaster" mechanism — V1 keeps it simple. If a competitive market emerges, V2+ can add an on-chain paymaster registry with fee bidding (similar to ERC-4337's bundler ecosystem).

---

## Pricing policy

V1 paymasters sign **unconditionally**. The chain debits `call_gas_limit + GAS_USER_OP` EVP from the paymaster per sponsored tx. The paymaster recovers cost from the user out-of-band (off-chain payment in USDC/ETH/another token, subscription, foundation subsidy, etc.).

Recommended V1 policies:

- **Foundation paymaster:** 1:1 EVP (sponsorship is free for testnet/early-mainnet; foundation absorbs cost as growth subsidy).
- **Third-party USDC paymaster:** wallet sends USDC to paymaster operator's address (Ethereum / L2 / off-chain); paymaster's automated reconciliation script confirms payment, then signs the UserOp. **The reconciliation script must run before the `POST /sponsor` returns** — otherwise the paymaster signs first and gets stiffed.

V1 does not provide a built-in payment-confirmation path. Operators wire that themselves between their billing system and the paymaster service.

---

## Failure modes

| Symptom | Likely cause | Fix |
|---|---|---|
| `/sponsor` returns 400 with `paymaster_signature must include` | Wallet sent a UserOp with `paymaster_signature` already set (`AlreadySigned`) | Wallet bug — do not pre-stamp paymaster fields on /sponsor calls |
| `/sponsor` returns 400 with `user signature missing or invalid` | Strict mode (`require_user_sig: true`) rejected the inbound UserOp | Wallet must stamp a valid user-side signature over `Transaction::UserOp(user_op).signing_message(chain_id)` BEFORE posting to `/sponsor`. The signed message must include `paymaster = <this paymaster's address>`. See `wallet::paymaster::sign_user_op_as_sender` for the canonical helper |
| `/sponsor` returns 429 `rate limited` | Per-sender token bucket exhausted | Wallet should back off (default refill is 5 sponsorships/sec). Operators can tune via `--per-sender-rps` / `--per-sender-burst` or disable with `--per-sender-rps 0` |
| `/sponsor` returns 503 `paymaster IO` | Disk full, nonce file unwritable, or audit log unwritable | Free disk; check filesystem permissions on both the nonce file and the audit-log file's parent directory |
| Chain rejects every UserOp with `InvalidNonce { expected: N, got: M }` | `paymaster_nonce` file desynced from chain state | Stop service, query chain `account.nonce` for paymaster address, write that value into the nonce file, restart |
| Chain rejects with `paymaster_signature verification failed` | `--chain-id` doesn't match the chain's chain_id | Restart service with the correct chain-id |
| Chain rejects with `does not derive to paymaster address` | Wallet stamped a wrong `paymaster` address | Wallet should leave `paymaster: None` on /sponsor and let the service fill it in (the service overwrites regardless) |
| Service crashes on startup with "paymaster_nonce file is malformed" | Manual edit / disk corruption | Inspect the file. If empty, delete it (counter reseeds at 0 — but only safe if chain `account.nonce` is also 0). Otherwise, write the correct integer. |
| `cargo build` errors with `evaporchain-paymaster` not found | Workspace member entry missing | Confirm `crates/evaporchain-paymaster` is in `Cargo.toml` `members =` list |

---

## Related docs

- `docs/MULTI_TOKEN_GAS_OPTIONS.md` — Option A vs B vs C decision; this runbook is the operational arm of Option B.
- `docs/runbooks/cluster-deploy.md` — how to roll a new node binary; same procedure applies if you need to deploy paymaster alongside a chain upgrade.
- `crates/evaporchain-paymaster/src/lib.rs` — library API (`Paymaster::new`, `Paymaster::sponsor`, `load_keypair_from_file`, `generate_keypair_to_file`).
- `wallet/src/paymaster.rs` — wallet client (`PaymasterClient::info`, `PaymasterClient::sponsor`, `build_unsigned_user_op`).
- `tests/integration/src/paymaster_e2e.rs` — E2E test that exercises the full pipeline (HTTP → execute_block) in-process; useful as a reference flow for wallets.
