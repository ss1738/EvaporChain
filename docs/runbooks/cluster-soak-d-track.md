# Cluster soak runbook — T0.2 D-track

**Audience:** operator running the 3-Mini + 2-Hetzner cluster.
**Sister docs:** `cluster-deploy.md` (bring-up), `dos-resistance.md` (per-vector adversarial), `layer4-soak-report.md` (previous run).
**`MAINNET_READINESS.md` lanes addressed:** T0.2 (D.1-D.5), T0.5 (PNT v1 flip + telemetry), T0.6 (slashing-at-scale).

This is the empirical-evidence step that flips the three remaining 🟡 OPEN Tier-0 OPS lanes to ✅ DONE.

---

## 0.  Prereqs

### 0.1  Release binary on every node

Each node needs an up-to-date `evaporchain-node` release binary.  Build on each in parallel:

```bash
ssh ${NODE_USER}@${NODE_IP} \
  "cd ~/EvaporChain && git fetch origin main && git reset --hard origin/main && \
   cargo build --release -p evaporchain-node 2>&1 | tail -5"
```

**Known Mini 1 build failure (2026-05-16):** `libsqlite3-sys` C compile fails with `arm64-apple-macosx`, `mmacosx-version-min=26.4.1`.  Fix:

```bash
# On Mini 1 only — refresh Xcode CLI tools to match macOS 14.4+
xcode-select --install   # or `sudo xcode-select --reset`
# OR pin a lower deployment target
export MACOSX_DEPLOYMENT_TARGET=14.0
cargo clean && cargo build --release -p evaporchain-node
```

### 0.2  Genesis + keys present

```bash
ls -la ~/EvaporChain/genesis-tailscale-5node.json
ls -la ~/.evaporchain-tailscale-data/bls_key.bin    # ← preserve across data-wipes per CLAUDE.md
```

If `bls_key.bin` is missing, regenerate via `evaporchain-cli keygen` BEFORE wiping data dirs; otherwise the node's stake is orphaned.

### 0.3  All 5 nodes' API responding

```bash
for ip in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  echo "=== $ip ==="
  curl -sf -m 5 "http://${ip}:8080/api/status" | jq -c '{height, epoch, chain_id, finalized: .latest_finalized}'
done
```

All 5 should report the same `chain_id`, monotonically advancing `height`, and `finalized > 0` after the first ~10 blocks.

---

## 1.  D-track adversarial (D.1) — 30-60 minutes

**Goal:** exercise the live cluster against Layer-4 adversarial inputs.  For each vector (A.1–A.10), every API endpoint must respond 4xx (never 5xx / panic / hang), finality must continue progressing throughout, and no node restarts involuntarily.

```bash
cd ~/EvaporChain
TARGETS="100.119.53.101:8080,100.113.253.72:8080,100.103.216.125:8080" \
  PRIMARY="100.119.53.101:8080" \
  ./scripts/d-track-adversarial.sh
```

Output → `./logs/d-track-adversarial/$RUN_ID/`:
- `summary.txt` — verdict per vector (pass/fail)
- `*.csv` — finality-progression samples around each attack
- `events.log` — any restart / stall events caught during the run

**Pass criterion:**
- 10/10 vectors return only 4xx codes
- No node restarted (uptime monotonically increases throughout)
- Finality continued progressing within 30s windows around each attack

If any vector fails: capture node logs (`journalctl -u evaporchain -n 1000 --no-pager`) and stop — adversarial pass is a HARD gate before D.2 soak.

---

## 2.  D-track fault injection (D.4) — 30 minutes

```bash
TARGETS="..." ./scripts/d-track-fault-injection.sh
```

Exercises: kill -9 on a node, network partition, disk-full simulation, BLS key rotation under load.  Each must recover within `RECOVERY_SECS=120` and rejoin the canonical tip.

---

## 3.  D-track partition (D.5) — 30 minutes

```bash
TARGETS="..." ./scripts/d-track-partition.sh
```

Partitions 2-of-5 from the rest.  Minority side must NOT finalize (3-of-5 supermajority needed).  Majority side continues.  Partition heals → minority catches up via fast-sync.

---

## 4.  72-hour cluster soak (D.2 + D.3) — 72 hours wall time

**Goal:** sustained 1k TPS load, finality stays ≥ 1 block/s, no finality stall > 30s.

### 4.1  Pre-flight

Each node must run with `--faucet-rate-limit-disabled` so the unique-address tx-submission loop doesn't hit the 1-hour faucet cooldown.  Update systemd unit + restart:

```bash
# On each node:
sudo systemctl edit evaporchain
# Add:
[Service]
Environment=EVAPORCHAIN_FAUCET_RATE_LIMIT_DISABLED=1
# Save, then:
sudo systemctl restart evaporchain
```

### 4.2  Run

```bash
cd ~/EvaporChain
TARGETS="..." \
  TPS=1000 \
  DURATION=259200 \   # 72h = 259200s
  WORKERS=20 \
  STALL_SECS=30 \
  ./scripts/d-track-soak.sh
```

Output → `./logs/d-track-soak/$RUN_ID/`:
- `samples.csv` — per-2s snapshot of every node's height/epoch/finalized/mempool/active/ghosts/uptime
- `txs.csv` — per-tx HTTP code (track 4xx/5xx rate)
- `events.log` — finality-stall + recovery events
- `summary.txt` — pass/fail verdict against D.2 (≥1 blk/s @ 1k TPS) + D.3 (no stall >30s)

### 4.3  Monitoring during the soak

Tail in a separate tmux pane:

```bash
tail -f logs/d-track-soak/$RUN_ID/events.log
```

If `events.log` records a stall > `STALL_SECS`: investigate IMMEDIATELY.  The 72hr soak is restartable, but a single 30+ s stall = D.3 fail.

Acceptable yellow signals during the run:
- mempool depth oscillating up to ~5000 txs (normal under 1k TPS / 5s reveal-delay)
- per-tx 429 rate up to 5% (faucet bucket eviction working as designed)
- `ghosts` count growing — proves evaporation is firing

Red signals — pause + investigate:
- mempool depth > 9000 (approaching MAX_ENCRYPTED_PENDING cap)
- per-tx 5xx rate > 0
- any node restart (`uptime_s` resets)
- `finalized` not advancing on any node for > 30s

---

## 5.  T0.5 — PNT v1 governance flip

Once D-track adversarial + 72hr soak are green, run T0.5:

### 5.1  Set the fork-epoch

Pick a fork epoch ~12 hours in the future (gives all nodes time to catch the flip):

```bash
FORK_EPOCH=$(curl -sf http://$PRIMARY/api/status | jq '.epoch + 43200')   # ~12hr at 1s block
```

### 5.2  Flip the protocol_version governance param

```bash
curl -X POST "http://$PRIMARY/api/governance/param" \
  -H "Authorization: Bearer ${EVAPORCHAIN_ADMIN_KEY}" \
  -d "{\"name\":\"pnt_protocol_version\",\"value\":\"1\",\"fork_epoch\":${FORK_EPOCH}}"
```

### 5.3  Storage-growth telemetry

Monitor PNT bucket sizes from each node for 24 hours post-flip:

```bash
for ip in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -sf "http://${ip}:8080/api/pnt/stats" | jq -c '{nullifier_count, bucket_count, oldest_epoch}'
done
```

Acceptance: bucket counts grow linearly with throughput; no bucket is older than the configured retention window.

---

## 6.  T0.6 — Slashing-at-scale empirical

The 5 adversarial scenarios in `crates/evaporchain-consensus/tests/slashing_at_scale.rs` are unit-test green (2026-05-14).  T0.6 lifts them to live cluster:

```bash
# 5-node cluster with one node configured as Byzantine
BYZANTINE_NODE=2 ./scripts/d-track-slashing.sh
```

(Note: as of 2026-05-16 the slashing-at-scale script lives in `crates/evaporchain-consensus/tests/slashing_at_scale.rs`; a wrapper shell script for cluster-side soak is a follow-on.  Operator can drive the 5 scenarios manually from that test file's structure.)

---

## 7.  Lane-flip reporting

Once each D-track section is green, update `MAINNET_READINESS.md`:

- T0.2 → ✅ DONE (`done-as-of: <commit>`)
- T0.5 → ✅ DONE (operator step complete)
- T0.6 → ✅ DONE (cluster soak passed)

Also update `SESSION_PROGRESS.md` with an entry:

```
## YYYY-MM-DD (session N) — Cluster soak D-track + T0.5 + T0.6 ✅ DONE

**Focus:** Run the D-track adversarial + 72hr soak + PNT v1 flip on the 5-node cluster.
**Empirical results:** 10/10 adversarial vectors green; 72hr soak finalized N blocks at 1k TPS sustained; PNT v1 storage telemetry within bucket-retention window.
**MAINNET_READINESS.md lanes flipped:** T0.2 ✅, T0.5 ✅, T0.6 ✅.
**Cross-references:** `logs/d-track-{adversarial,soak}/$RUN_ID/summary.txt`.
```

---

## 8.  Failure recovery

If any acceptance criterion fails:

1. Capture cluster state: `for ip in ...; do curl -sf "http://${ip}:8080/api/status" > "logs/state-${ip}.json"; done`
2. Pull journal logs from each node: `journalctl -u evaporchain --since "30 minutes ago" --no-pager > logs/journal-${ip}.log`
3. Stop the soak: `pkill -INT -f d-track-soak.sh`
4. Open a `failure-report.md` under `logs/d-track-$RUN_ID/` summarising:
   - Vector / phase where it failed
   - Per-node state snapshot at failure
   - Hypothesis on root cause (e.g., known M-class audit item, unmerged PR, etc.)
5. Either fix the issue OR mark the lane as 🔴 BLOCKED on the fix and notify the next session.

External-audit kickoff (T0.12) is BLOCKED until all three lanes (T0.2/T0.5/T0.6) are ✅ DONE.

---

## 9.  Estimated wall time

| Phase | Wall time | Active operator time |
|---|---|---|
| Prereqs (build + verify) | 30 min | 30 min |
| D.1 adversarial | 30-60 min | 15 min (monitor + verdict) |
| D.4 fault injection | 30 min | 10 min |
| D.5 partition | 30 min | 10 min |
| D.2 + D.3 soak (72h) | 72 hours | ~1 hour total (monitoring) |
| T0.5 PNT flip + 24h telemetry | 24+ hours | 30 min |
| T0.6 slashing-at-scale | 2-4 hours | 1 hour |
| Lane-flip reporting | 30 min | 30 min |

**Total active operator time: ~4 hours spread across ~5 days wall time.**
