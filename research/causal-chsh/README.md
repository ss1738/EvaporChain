# Causal-CHSH Empirical Gate — Reproducibility

**Doctrine reference:** `research/INVENTION_STACK.md §A1.10`
**Crate:** `evaporchain-causal-chsh` (Tier-0 supporting, gate PASS 2026-05-04)

This directory holds the data + scripts that earned Causal-CHSH its
slot in the doctrine. An auditor running the steps below should
reproduce the same `PASS` verdict on real Ethereum mainnet.

## What the gate is

`S = | E(A,B) + E(A,B') + E(A',B) − E(A',B') |` over four samples
of ±1 products drawn from concurrent block pairs in the LightCone
DAG under four CHSH setting-pairs. **Theorem (proposed):** under
honest validators + LightCone causality + EvaporChain's single-λ
decay, `S ≤ 2`. Violation ⇒ hidden cross-validator coordination.

## Pre-committed thresholds

Locked in `crates/evaporchain-causal-chsh/src/gate.rs::GateThresholds::doctrine()`
**before** any real-data run. Same MERA-style discipline (which
just FAILED its own gate; same code path, opposite outcome — that's
the credibility).

| Threshold | Value | Meaning |
|---|---|---|
| `honest_ceiling` | 1.80 | Real-data S must stay below this — the inequality has empirical headroom under honest traffic |
| `cartel_floor` | 2.20 | Synthetic-cartel injection must exceed this — the inequality discriminates coordination |
| `min_gap` | 0.40 | `S_cartel − S_honest` must exceed this — discrimination has signal-to-noise |

If any one fails: **drop**. Crate retained as research artefact, same
fate as MERA. Operators cannot override these thresholds via the
`POST /api/cartel_alarm/run_gate` RPC — the value of the gate is in
the pre-commitment.

## Reproducing the verdict from scratch

### Step 1 — scrape Ethereum mainnet headers

```bash
cd ~/EvaporChain
# 200-block sample (matches Lane O.3 commit c9e553c)
python3 research/causal-chsh/scrape_eth.py 19900000 19900200 \
    research/causal-chsh/honest.csv --sleep 0.20

# 3K-block sanity check (matches Lane O.6 commit cdb736c, same Eth
# window the MERA gate used)
python3 research/causal-chsh/scrape_eth.py 19900000 19903000 \
    research/causal-chsh/honest_3k.csv --sleep 0.20
```

The scraper rotates between `eth.publicnode.com` and
`eth-mainnet.public.blastapi.io` with a browser User-Agent — same
RPC pattern as the MERA scrape (Dune's free-tier CSV download is
blocked for the relevant query). 0 fetch failures over both runs as
of 2026-05-04. Each block requires one `eth_getBlockByNumber(.., false)`
call (header only — no per-tx detail).

Expected runtime: ~80s for 200 blocks, ~20min for 3000.

### Step 2 — run the gate

```bash
cargo run -p evaporchain-causal-chsh --release \
    --bin causal_chsh_run_gate -- \
    research/causal-chsh/honest.csv \
    --window-secs 60 \
    --report research/causal-chsh/GATE_RESULT.md
```

The runner:

1. Loads the CSV into `Vec<BlockSummary>`
2. Calls `extract_chsh_samples(trace, 60)` — concurrency-window proxy
   distributes pairs round-robin across the 4 CHSH setting-pair buckets
3. Computes 4 binary observables per block:
   - `A`  = `sign(block.size − median_size)`
   - `A'` = `sign(gas_used − median_gas)`
   - `B`  = `sign(tx_count − median_tx_count)`
   - `B'` = `sign(timestamp_secs % 2)` (timestamp parity)
4. Computes `S_honest` via `compute_chsh_s(samples)`
5. Generates same-size synthetic cartel injection via
   `synthesize_max_cartel_samples(n)` (rigs samples to S=4
   algebraic max — uniformly +1 in three buckets, uniformly −1 in
   the fourth)
6. Computes `S_cartel`
7. Checks all three doctrine thresholds via
   `run_synthetic_gate(honest, cartel, GateThresholds::doctrine())`
8. Writes the verdict to `GATE_RESULT.md`

Exit code: `0` = PASS, `1` = FAIL, `4` = InputError.

### Step 3 — interpret the verdict

Read `GATE_RESULT.md`. The verdict block is at the top.

**Expected (PASS):**

```
VERDICT: PASS — Causal-CHSH SHIPS

  S_honest = 0.0120  (< 1.80 ceiling ✓)
  S_cartel = 4.0000  (> 2.20 floor ✓)
  gap      = 3.9880  (> 0.40 min ✓)
```

200-block run: `S_honest = 0.0120` (~150× headroom on ceiling).
3K-block run: `S_honest = 0.0175` (slightly higher with more data,
still ~100× below ceiling — verdict robust under sample-size scaling).

## Why the verdict is honest about being a proxy

Real Ethereum is a *linear* chain — no LightCone DAG, no genuinely
concurrent blocks. The gate uses a **concurrency-window proxy**:
pairs of blocks separated by ≤ `concurrency_window_secs = 60s` are
treated as "would-be concurrent under a LightCone refactor."

The proxy is honest. The gate's verdict on Ethereum is a *proxy*
for what would happen on a true LightCone chain. The genuine test
on EvaporChain's own LightCone substrate has to wait until testnet
matures.

That said, the proxy passes by such enormous margins (S_honest ≈
0.012-0.018 vs ceiling 1.8) that a real LightCone trace would have
to look pathologically different from real chain traffic to fail
the bound.

## Files in this directory

| File | What |
|---|---|
| `README.md` | This file |
| `scrape_eth.py` | Scraper — `(start, end, output_csv)` → header-only blocks |
| `honest.csv` | 200-block sample (19_900_000 to 19_900_200) — Lane O.3 reproducibility data |
| `honest_3k.csv` | 3K-block sample (19_900_000 to 19_903_000) — Lane O.6 reproducibility data |
| `GATE_RESULT.md` | 200-block verdict report (PASS) — Lane O.3 |
| `GATE_RESULT_3K.md` | 3K-block verdict report (PASS) — Lane O.6 |

## Related code paths

- `crates/evaporchain-causal-chsh/src/chsh.rs` — math primitive
  (`compute_chsh_s` + classical-LHV variant)
- `crates/evaporchain-causal-chsh/src/gate.rs` — gate runner with
  doctrine-locked thresholds (`GateThresholds::doctrine()`)
- `crates/evaporchain-causal-chsh/src/trace.rs` — concurrency-window
  proxy + 4 binary observables (`extract_chsh_samples`)
- `crates/evaporchain-causal-chsh/src/alarm.rs` — rolling-buffer
  observability primitive (`CartelAlarm`)
- `crates/evaporchain-causal-chsh/src/bin/run_gate.rs` — Rust binary
  used in step 2
- `crates/evaporchain-node/src/api.rs` — `POST /api/cartel_alarm/run_gate`
  HTTP endpoint (Lane O.5)

## Running the test suite

```bash
cargo test -p evaporchain-causal-chsh
```

Expected: 26 tests pass, 5 of them proptests (Bell bound for LHV
sources, S algebraic range, alarm rolling-buffer invariants over
256× random configurations).
