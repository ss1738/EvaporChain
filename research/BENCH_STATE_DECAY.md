# State-Decay Benchmark — the credibility unit

**Why this exists.** Seven dated research streams (2024–2026) converged: a solo, no-token, novel-primitive L1 earns relevance the way Reth / SP1 / RISC Zero / early Celestia did — **one irrefutable, reproducible proof artifact tied to the primitive**, not another dApp, token, or BD. This is that artifact for EvaporChain's core thesis (*state bounded by construction via energy-decay*).

## The claim

> Under a sustained deploy workload, EvaporChain's energy-decay primitive holds the aggregate **active-object set and on-disk state bounded by construction** — no rent, no restore tx, no operator action — whereas the *same workload without decay* grows monotonically (the model every other L1 is stuck with: Ethereum unbounded live state, Solana state bonded-forever / 256 GB–1 TB validator RAM, Stellar archival that corrupted mainnet — see macro context below).

## How to run it (one command, self-serve, no ssh)

```bash
EVAPORCHAIN_TX_TOKEN=… bash scripts/bench-state-decay.sh --regime both
# or fully self-serve (it mints its own token):
NODE_URL=http://89.167.52.40:8099 bash scripts/bench-state-decay.sh
```

Same node, same workload (`contracts/evaporscript/bench_object.es`), two regimes isolated **only** by the deploy `half_life`:
- `decay`   — small energy/half_life → objects evaporate.
- `nodecay` — astronomical half_life → objects persist (control).

It samples `GET /api/status` `{active_objects,total_evaporated,data_dir_bytes}` every interval → CSV + a verdict.

## Instrument correction (v1 → v2 — verify-before-claiming caught this)

v1 measured `/api/status.active_objects` / `data_dir_bytes`. **Both were wrong instruments**, proven by live probing: `active_objects` invariantly reads `4` regardless of deploys (it counts the genesis demo objects, *not* deployed EvaporScript contracts); `data_dir_bytes` swings with RocksDB compaction across restarts (514 M → 37 M observed). v2 measures the primitive **directly and correctly**: deploy N objects, **poll each to a terminal tx state and capture its `contract_id`**, then over time count how many of *those specific cids* remain live (`GET /api/script/:id`, not evaporated). Unfooled by genesis objects or restart noise.

## Expected result

| | live_count over samples | gone |
|---|---|---|
| **decay** | landed → **~0** (objects evaporate by physics) | rises to ≈landed |
| **nodecay** (control) | stays ≈ landed (persist) | ≈ 0 |

The **divergence is the proof** — shown empirically, per-object, on a live chain, with no trust in the operator. `WORKLOAD_FAILED` (0 deploys landed) is reported distinctly and is explicitly *not* a primitive result (node/contention, not falsification).

## How to falsify it

Run it. If the `decay` regime's `active_objects` / `data_dir_bytes` *also* grows ≈ monotonically with deploys (i.e. objects don't evaporate), **the core thesis is false**. The benchmark is built to be falsifiable by one curious engineer against the public sandbox without contacting the author — that self-serve falsifiability is the entire point (per the no-token/no-distribution constraint, it is the only honest growth lever).

## Honest scope (no overclaim)

- **Single-node `--mock-consensus` sandbox.** This proves *per-object terminal evaporation aggregating to a bounded active set*. It is **not** a real-BFT, multi-validator, Solana-scale adversarial state-spam test.
- **Bounded by design.** The harness self-limits on `data_dir_bytes` (disk-safe; the `nodecay` control would otherwise fill the box — a real "no resource isolation" caveat for the sandbox). A dramatic large-scale bloat curve needs headroom the sandbox does not safely have; this demonstrates the **divergence trend**, and cites the macro numbers below for the at-scale picture rather than faking them on a 38 GB box.
- **Macro context (external, dated, sourced):** Ethereum killed Verkle + deferred state-expiry (EF "Protocol Priorities 2026", 2026-02-18) — live state still unbounded; Solana abandoned rent → state bonded forever, 256 GB–1 TB validator RAM; Stellar is the only L1 with native archival and a 2025-09 eviction bug corrupted mainnet for 35 days. No L1 has shipped *decay*; only relocation/bonding/witness-shifting.

## Independent of #27

This proves the **core energy-decay thesis** and is unrelated to the ZK-bridge soundness blocker (B-1/B-2, task #27, deep-staged). It is buildable, runnable, and credible now.
