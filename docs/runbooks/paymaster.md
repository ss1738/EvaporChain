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
| `--audit-log-fsync MODE` | no | `per-line` | Audit fix #8a: fsync policy. `per-line` = fail-closed durability (~1k QPS ceiling). `none` = skip fsync, ~10× throughput, OS handles writeback (crash-loss bounded by OS dirty-page schedule, typically 30 s on Linux) |
| `--allow-inner LIST` | no | (trust chain) | Operator-side inner-tx whitelist. Comma-separated values from `transfer`, `call_script`, `call_contract`. Omitted = sponsor any chain-accepted variant. Example: `--allow-inner=transfer` for a transfer-only paymaster. See §Inner-tx whitelist |
| `--idempotency-max-keys N` | no | `1024` | Day 12: idempotency cache size. `0` disables. Wallets sending `Idempotency-Key` retry-safely against this cache |
| `--idempotency-ttl-secs N` | no | `3600` | Day 12: idempotency cache TTL in seconds |
| `--idempotency-persist-path PATH` | no | (off) | Audit fix #6a: persist the idempotency cache to disk. Loaded at startup; re-written atomically on every successful insert. Wallet retries that span a paymaster restart still get cache-replay. Single-process only — multiple paymasters pointing at the same file would race the rename |
| `--chain-rpc-url URL` | no | (off) | Audit fix #3a: chain RPC URL for startup nonce reconciliation. Hits `GET /api/address/<paymaster_addr>` and compares the chain's `account.nonce` against the local `paymaster_nonce` file. Mismatch is logged loudly. See §Startup nonce reconciliation |
| `--strict-reconcile` | no | off | Refuse to start when reconciliation surfaces drift OR fails (RPC error). Production paymasters should set this — sponsoring under drift either creates forever-gaps in the nonce sequence or duplicates already-consumed nonces. Requires `--chain-rpc-url` |
| `--reconcile-interval-secs N` | no | `60` | Audit fix #3b: runtime reconciliation poll interval. `0` disables. Periodic background task hits the same RPC as startup reconciliation; updates `drift_detections_total` / `last_chain_nonce` / `last_reconcile_unix_ms` in `/metrics`. Requires `--chain-rpc-url` |

### Endpoints

| Method | Path | Body | Returns |
|---|---|---|---|
| GET | `/healthz` | — | `"ok"` |
| GET | `/info` | — | `{paymaster_address_hex, next_paymaster_nonce, chain_id, require_user_sig, per_sender_rps, per_sender_burst, audit_log_enabled, audit_log_fsync?, allowed_inner_variants?, idempotency_max_keys, idempotency_ttl_secs}` |
| GET | `/metrics` | — | Prometheus exposition format (see §Metrics) |
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

### Startup nonce reconciliation

After a service restart (graceful or post-crash) or a chain reorg, the local `paymaster_nonce` file can drift from the chain's view of the paymaster's `account.nonce`. Sponsoring while drifted either creates forever-gaps of unusable nonces (if local was right) or duplicates already-consumed nonces (if a prior process crashed mid-write).

`--chain-rpc-url` triggers a startup-time reconciliation:

```bash
evaporchain-paymaster --chain-id evaporchain-mainnet \
  --chain-rpc-url http://node-1:8081 \
  --strict-reconcile
```

Three outcomes (logged at `info!` for aligned, `error!` for the others):

| Outcome | Meaning | Operator action |
|---|---|---|
| `Aligned { nonce: N }` | Local file matches chain. | None — start sponsoring. |
| `LocalAhead { local, chain }` | Paymaster signed sponsorships not yet on-chain. Mempool, dropped network, or reorg. | Investigate which UserOps are unfinalised. Either re-submit them, or accept the gap (truncate local file to `chain` and lose `local - chain` already-allocated-but-unused nonces). |
| `ChainAhead { local, chain }` | Chain has consumed nonces local doesn't know about. Paymaster account misuse OR earlier process state lost. | Stop the service, write `chain` into the nonce file, restart. |

`--strict-reconcile` (off by default) refuses startup on anything but `Aligned`. Production paymasters should set it — silent drift compounds.

**Runtime reconciliation** (audit fix #3b) is also wired: `--reconcile-interval-secs N` (default `60`) spawns a background tokio task that re-runs the alignment check every N seconds. Outcomes:

- Aligned: silent (steady-state).
- Drift (LocalAhead / ChainAhead): `error!` log + `drift_detections_total` counter increments + `last_chain_nonce` gauge updates.
- RPC unreachable: `error!` log; gauges NOT updated, so `last_reconcile_unix_ms` going stale becomes the alert signal.

**Suggested Prometheus alerts** (drift / staleness):
- `increase(evaporchain_paymaster_drift_detections_total[5m]) > 0` — page; reorg or paymaster-account misuse.
- `evaporchain_paymaster_last_chain_nonce != evaporchain_paymaster_next_nonce` — drift visible directly.
- `time() * 1000 - evaporchain_paymaster_last_reconcile_unix_ms > 180000` — chain RPC unreachable for >3 min.

The runtime poller does NOT auto-pause `/sponsor` on drift — operator response is external. Auto-pause-on-drift is V1.5 (the wallet would need a clear retry-after message; the binary can't decide that policy alone).

### Idempotency (wallet retries)

Wallets retry `/sponsor` on network blips. Without idempotency, the second call allocates a fresh paymaster_nonce + signs a second time — the wallet ends up holding two distinct UserOps for what was logically one sponsorship, the chain accepts the first and rejects the second, and the paymaster has burned gas budget on a UserOp that won't land.

The Day 12 fix: wallets send an `Idempotency-Key: <opaque>` HTTP header on every `/sponsor`. The paymaster keeps a bounded LRU cache (`idempotency_max_keys`, default `1024`) with per-entry TTL (`idempotency_ttl_secs`, default `3600` = 1h). Same key → return the cached `SponsorshipResponse` byte-for-byte (same paymaster_nonce, same sig). New key → process normally, cache the result.

```bash
# Wallet generates a UUID per logical sponsorship and sends it.
curl -X POST http://localhost:8088/sponsor \
  -H 'idempotency-key: 8ec40a3f-...' \
  -H 'content-type: application/json' \
  -d @user_op.json
```

Operationally:
- **Failed sponsorships are NOT cached.** `AlreadySigned` / rate-limited / invalid user sig errors don't poison the key — a wallet retry with a clean UserOp under the same key gets fresh handling.
- **TTL is generous by default** (1h). Tune higher if wallets retry across longer windows (laptop sleep mid-flight); lower if the paymaster's nonce horizon is short.
- **`0` disables.** Set `--idempotency-max-keys=0` to opt out entirely; clients sending `Idempotency-Key` simply don't see the cache.
- **/info exposes `idempotency_max_keys` + `idempotency_ttl_secs`** so wallets can decide whether to bother computing and sending keys.
- **`evaporchain_paymaster_idempotent_replays_total` counter** — see §Metrics. Sustained high replay rate signals either flaky wallet → paymaster networking or a wallet bug retrying without backoff.
- **Persistence** (audit fix #6a): set `--idempotency-persist-path PATH` to keep the cache across paymaster restarts. The file is JSON-encoded `PersistedCache { entries, insertion_order }`; loaded at startup with TTL-expired entries dropped; re-written atomically (temp file + rename) on every successful insert. Single-process semantics — running multiple paymaster processes against the same path would race the rename and lose entries; cross-process via shared DB is V1.5+.

#### Limitations the cache does NOT cover

- **Restart loses the cache.** The cache is in-memory (HashMap + insertion-order VecDeque on the `Paymaster` struct) — there's no on-disk persistence. A restart wipes the entire `idempotency_max_keys` window. A wallet retry that spans the restart gets a fresh `paymaster_nonce`, not a replay. If your operational profile involves frequent restarts (e.g. canary deploys, k8s pod cycling) and you depend on idempotency, prefer a longer wallet-side dedupe window (so the wallet can detect the duplicate response itself) over relying on the cache as the sole defense.
- **Concurrent retries with the same key can both miss.** The cache lock is dropped between the lookup and the populate, so two retries arriving in flight at the same time can both run the full sponsor path and both allocate distinct `paymaster_nonces`. The second's response wins the cache slot; the first's nonce is "lost" — i.e., not replayable. The paymaster has spent gas budget on both. Wallets should retry sequentially with backoff to avoid this; the chain itself only accepts one of the two regardless. This is a known polish backlog item — fixable with per-key in-flight locking.

### `/info` policy surface

Wallets read `GET /info` before submitting `/sponsor` to discover the paymaster's address, next-nonce, chain_id, AND its operator policy. A wallet that pre-checks policy can fail a doomed request locally (e.g. wallet doesn't have a hybrid keypair → paymaster requires user-sig → reject locally rather than burn the round-trip).

```bash
curl http://localhost:8088/info
```

```json
{
  "paymaster_address_hex": "ab12...",
  "next_paymaster_nonce": 1234,
  "chain_id": "evaporchain-mainnet",
  "require_user_sig": true,
  "per_sender_rps": 5.0,
  "per_sender_burst": 10,
  "audit_log_enabled": true,
  "allowed_inner_variants": ["transfer", "call_script"]
}
```

`allowed_inner_variants` is omitted entirely when the paymaster trusts the chain's whitelist (no operator narrowing). The audit-log PATH is intentionally NOT exposed — operational hygiene; only whether logging is on.

Wire backwards-compat: every policy field has `serde(default)`. A wallet built post-Day-11 hitting an old paymaster sees the policy fields filled with permissive-baseline defaults (`require_user_sig: false`, `per_sender_rps: 0.0`, etc.) — the wallet should treat that as "unknown policy; submit and see".

### Inner-tx whitelist

By default the paymaster sponsors any inner Transaction variant the chain accepts (currently `Transfer`, `CallScript`, `CallContract` — see `crates/evaporchain-execution/src/lib.rs:execute_user_op`). Operators can narrow this with `--allow-inner=<comma-separated>`:

```bash
# Transfer-only paymaster — won't subsidize contract calls
evaporchain-paymaster ... --allow-inner=transfer

# Allow Transfer + CallScript but not CallContract
evaporchain-paymaster ... --allow-inner=transfer,call_script
```

Why an operator would narrow:
- **Specialization.** A "stablecoin micro-tip paymaster" only needs Transfer; rejecting CallScript / CallContract limits the blast radius if a key wallet compromise tries to redirect sponsorship into expensive contract loops.
- **Billing simplicity.** Transfer call_data is fixed-shape; contract calls vary in gas. An operator pricing per-sponsorship in fiat may want to reject the variable cases until they have variable pricing wired.
- **Compliance.** A regulated paymaster might be cleared to subsidize value transfers but not arbitrary contract calls.

Empty `call_data` (gas-only sponsorship — bumping a sender's nonce without doing anything else) is **always** allowed regardless of this setting; there's no inner intent to classify.

Rejection surfaces as `400 Bad Request` with `inner variant 'X' is not allowed by this paymaster`. The wallet should respect the policy or pick a different paymaster — wallets enumerate paymaster policies via `GET /info` (future V1.5 hardening will add `allowed_inner_variants` to `/info`).

### Metrics

`GET /metrics` returns the standard Prometheus exposition format (Content-Type `text/plain; version=0.0.4`). Scrape with Prometheus / vmagent / vector / DataDog / etc. on the standard 15s cadence — the response is cheap (atomic counter loads + a mutex-guarded HashMap len read).

Surface:

| Metric | Type | Description |
|---|---|---|
| `evaporchain_paymaster_sponsorships_total{status=...}` | counter | Number of `/sponsor` requests by outcome. Status labels: `ok`, `already_signed`, `invalid_user_sig`, `rate_limited`, `nonce_io`, `audit_io`, `other`. All 7 emit (even at 0) so `rate()` / `increase()` over a status selector never NaN's |
| `evaporchain_paymaster_next_nonce` | gauge | Next sponsorship nonce that will be assigned — should advance monotonically; flat for several minutes implies the paymaster is idle |
| `evaporchain_paymaster_active_senders` | gauge | Number of senders held in the rate-limiter HashMap (active or in flight, before idle GC). Spikes here often correlate with `rate_limited` ticks |
| `evaporchain_paymaster_uptime_seconds` | gauge | Process uptime since paymaster construction. Restarts reset to 0 |

Suggested alerts:
- `rate(evaporchain_paymaster_sponsorships_total{status="invalid_user_sig"}[5m]) > 1` — sustained malformed sigs from wallets; investigate.
- `rate(evaporchain_paymaster_sponsorships_total{status="rate_limited"}[5m]) > 1` — sustained throttling; either bump `--per-sender-rps` or investigate which senders are flooding.
- `rate(evaporchain_paymaster_sponsorships_total{status=~"nonce_io|audit_io"}[1m]) > 0` — disk problems; page operator.
- `delta(evaporchain_paymaster_next_nonce[10m]) == 0 AND active_senders > 0` — the paymaster is rejecting every request despite traffic.
- `evaporchain_paymaster_uptime_seconds < 60 AND on(instance) prev value > 60` — recent restart.

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
- Each line is followed by an explicit `fsync(2)` (under the default `--audit-log-fsync per-line` mode) so a crash never loses a sponsored entry. Disk-write latency adds ~1 ms to `/sponsor` p99 on standard SSDs. Throughput ceiling: ~1k sponsorships/sec.
- `--audit-log-fsync none` skips the explicit `sync_all`. The line still hits the OS page cache via `write_all`; durability is at the kernel's writeback discretion (typically ~30 s of dirty-page lag). Crash-loss bound is undefined in absolute terms but bounded by that lag. Throughput ceiling: ~10k sponsorships/sec on the same hardware. Operators using this mode SHOULD have a redundancy story — mirrored audit log on a second disk, downstream fanout to a remote sink (e.g. Kafka), etc. — so the local file isn't the only audit copy.
- Group-commit (the safer middle option — every N ms or N pending writes, fsync once for the batch, all writers durable on return) is not yet implemented. Tracked as audit fix #8b for V1.5+.
- **Fail-closed:** an audit-log IO error (full disk, no permission) returns `503 paymaster IO`. The paymaster does NOT silently sponsor without writing the line — that would be a billing-reconciliation hole. Operators who need to keep sponsoring through audit-log outages should rotate the log path with `--audit-log` before unblocking.
- The file is opened with `O_APPEND` so concurrent writers (across worker threads in this process) are kernel-serialised at line boundaries. Multiple paymaster processes pointing at the same audit file work too — the per-line atomicity is OS-enforced for sub-PIPE_BUF writes.
- Rotate via:

  ```bash
  mv audit.jsonl audit-$(date +%F).jsonl
  kill -HUP <pid>
  ```

  The paymaster receives SIGHUP, closes the old fd (the renamed file's inode stays alive on the kernel side until the last reference drops), and re-opens the configured path with `O_APPEND + create`. Subsequent sponsorships land in the fresh `audit.jsonl`. If the reopen fails (path unwritable, permissions, etc.) the service logs `SIGHUP — audit log reopen failed` and subsequent sponsorships fail with `503 audit IO` until the operator fixes the underlying problem.

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
