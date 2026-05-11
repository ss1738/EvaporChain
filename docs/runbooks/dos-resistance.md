# DoS Resistance — Operator Runbook

**Lane T0.7** — Mempool + signature DoS hardening. This runbook captures the per-validator admission contracts that have been locked in CI plus the operator playbook for cluster-load DoS verification.

## In-CI regression suite

`crates/evaporchain-consensus/tests/dos_resistance.rs` — 6 tests covering 4 of the 7 vectors enumerated in `MAINNET_READINESS.md` T0.7:

| Vector | Test | What it locks |
|---|---|---|
| **1. Tx flooding** | `dos_v1_tx_flood_caps_at_max_size` | Pool stops accepting at exactly `MAX_MEMPOOL_SIZE = 10_000`; overflow rejections == flood − cap. No duplicates leaked from the test fixture. |
| **2. Signature storm** | `dos_v2_signature_storm_pool_stays_empty_under_garbage_sigs` | Under `verify_signatures = true`, every malformed-sig tx is rejected by `HybridVerifier::verify`. Pool remains at len 0 after a 200-tx flood. `rejected_count == flood`. |
| **3. Per-account fairness** | `dos_v3_single_sender_capped_below_global_max` | A single sender flooding 200 unique-nonce txs is capped at `MAX_TXS_PER_ACCOUNT = 64`, well below the 10K global. Sybil-resistance: one identity cannot monopolise the slot budget. |
| **4. Encrypted-mempool reveal flood** | `dos_v4_reveal_too_early_rejected` · `dos_v4_unrevealed_commitments_expire_at_reveal_epoch` · `dos_v4_encrypted_mempool_admission_cap_fires_on_flood` | Reveal-too-early temporal gate fires (RevealTooEarly). Unrevealed commitments expire at their reveal_epoch via process_reveals. **CAP NOW ENFORCED**: `submit_encrypted` rejects when `pending_encrypted == MAX_ENCRYPTED_PENDING` (10_000); 15K-flood test asserts exactly cap accepted, overflow rejected. |

Vector 5 (DAG fork-spam) — ✅ CLOSED 2026-05-11. Multi-validator convergence locked in `crates/evaporchain-consensus/tests/mcc_phase_d.rs`:
- `t0_7_v5_dag_fork_spam_convergence_across_4_validators`: 50 sibling forks injected into 4 validators' DAGs; all 4 agree on candidate_heads, enumerate_candidate_heads (caliber-ordered), authoritative head argmax, propose_parents (capped at `light_cone_max_concurrent_forks=4`), and antichain digest.
- `t0_7_v5_fork_spam_ordering_independence`: same 30 forks observed in forward vs reverse order by two validators; full substrate state converges. Path-independence under gossip jitter.

Vector 6 — ShardSample request flood. ✅ CLOSED 2026-05-11 (commit `8c59fad`, AUDIT-2026-05-11-1/2). Two-gate defense on the libp2p `ShardSample` inbound handler at `crates/evaporchain-network/src/service.rs:1746-1779`, symmetric to BlockSync:
- **Per-peer rate-limit** — `rate_limiter.check_and_increment(&peer)` rejects requests from peers over the gossipsub/sync per-peer budget. A peer that has saturated its other-protocol budget can no longer flood ShardSample.
- **Queries cap** — `MAX_SHARD_QUERIES_PER_REQUEST = 256` per request; overshoot drops the request AND records a peer violation via `ban_list.record_violation(peer)`. Each query forces a Merkle proof construction; cap pins per-request CPU to a bounded constant.

Regression test: `crates/evaporchain-network/src/service.rs::tests::shard_query_cap_is_capped_at_256` pins the cap value + verifies a serialized request at the cap stays <64 KB (well under libp2p's 1 MB JSON ceiling).

Vectors 7 (gas exhaustion) and 8 (memory blow-up via large blobs) are covered elsewhere — `block_stm` tests + `test_global_byte_cap_rejects_when_pool_would_overflow` in `mempool.rs` respectively.

## Operational acceptance — cluster-load DoS verification

T0.7 acceptance criterion: harness runs ≥1hr at each load level without admission-contract violation. This is OPERATOR-DRIVEN and depends on T3.1 (Phase C cluster deploy). Pre-flight:

1. Cluster is in lockstep at the same block_height + state_root.
2. `verify_signatures = true` flipped on all validators.
3. `/api/network/peers` shows zero ghost-score entries.
4. Baseline: capture 30-minute idle metrics (mempool size, rejected_count, CPU load per node).

### Vector 1 — tx flooding

Drive 1k → 10k → 100k tx/s sustained from 4 client nodes:

```bash
scripts/dos-flood.sh --target 100.119.53.101:8081 --rate 1000   --duration 1h
scripts/dos-flood.sh --target 100.119.53.101:8081 --rate 10000  --duration 1h
scripts/dos-flood.sh --target 100.119.53.101:8081 --rate 100000 --duration 1h
```

Pass criteria per load level:
- All cluster nodes' mempool `len ≤ 10_000` throughout
- `rejected_count` grows linearly with overflow rate
- Block production cadence stays within 2× of baseline (no consensus-stall under load)
- No node CPU sustains 100% for >5 min

### Vector 2 — signature storm

Drive 1k malformed-sig tx/s for 1hr:

```bash
scripts/dos-flood.sh --target 100.119.53.101:8081 --rate 1000 --duration 1h --garbage-sigs
```

Pass criteria:
- Every node's pool stays at `len ≈ 0` (no garbage tx accepted)
- `rejected_count` grows at 1k/s exactly
- CPU load proportional to verifier throughput; no degradation

### Vector 3 — single-sender exhaustion

Drive 1k tx/s from a single sender (Sybil isolation):

```bash
scripts/dos-flood.sh --target 100.119.53.101:8081 --rate 1000 --duration 1h --single-sender
```

Pass criteria:
- That sender's account-tx-count caps at 64 immediately
- Remaining bandwidth (10K − 64 = 9936 slots) stays available for other senders
- A second client at 100 tx/s from a different sender achieves baseline acceptance rate

### Vector 6 — ShardSample request flood

Drive a malicious light-client peer that sends shard-sample requests at the per-peer rate limit (gossipsub/sync budget) AND with `queries.len() > 256` payloads:

```bash
cargo run --release --bin shard-sample-flood -- \
  --target-addr /ip4/100.119.53.101/tcp/9000/p2p/<peer-id> \
  --chain-id evaporchain-tailscale-5node-1 \
  --rate 1000 \
  --queries-per-request 1024 \
  --duration 1h
```

The harness lives at `crates/evaporchain-network/src/bin/shard-sample-flood.rs` and reuses the production `P2pNetworkService` codec so there's no risk of wire-format drift between flood-side and validator-side.

Pass criteria per target node:
- `rate_limiter` rejects log lines fire at the configured per-peer ceiling (warn: `Rate-limited peer … dropping shard-sample request`).
- `ban_list.record_violation` fires every time `queries.len() > 256` (warn: `Peer … sent N shard queries, cap is 256 — recording violation`).
- Peer crosses the `ban_threshold` and ends up in `banned` map after the runbook-documented number of violations.
- `cargo test -p evaporchain-network shard_query_cap_is_capped_at_256` stays green; the runtime gate matches the regression test.
- Target node's CPU does NOT spike for ShardSample serving — proof-generation throughput pinned by the 256-query cap.

### Failure-mode triage

If admission contract is violated mid-test:
1. Capture `/api/network/peers`, `/api/mempool`, `/api/chain` snapshots.
2. Save validator logs + RocksDB column-family stats.
3. File issue with: vector ID, load level, time-to-violation, captured snapshots.
4. Halt the test; do NOT continue probing higher load levels until the violation is understood.

## Cross-references

- `MAINNET_READINESS.md` T0.7
- `crates/evaporchain-consensus/tests/dos_resistance.rs`
- `crates/evaporchain-consensus/src/mempool.rs` — admission gate implementation
- `crates/evaporchain-network/src/service.rs` — libp2p ShardSample + BlockSync inbound gates
- `AUDIT_2026_05_11.md` — AUDIT-2026-05-11-1/2 ShardSample DoS findings + fix
- `evaporchain_verification_track_2026_05_02.md` — historic per-(height,round) suppression bug
