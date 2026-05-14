# Layer 4 D-track Soak Report (T0.2)

**Status:** ⏳ PENDING EXECUTION  
**Cluster:** evaporchain-tailscale-5node-1 (3 Minis + 2 Hetzners)  
**Acceptance:** soak report committed here closes T0.2.

---

## Execution summary

| Sub-task | Script | Status | Date | Verdict |
|---|---|---|---|---|
| D.1 — adversarial sweep | `scripts/dos-flood.sh` | ⏳ | — | — |
| D.2 — ≥1 blk/s at 1k tx/s | `scripts/d-track-soak.sh` (first 5 min) | ⏳ | — | — |
| D.3 — 72hr stability soak | `scripts/d-track-soak.sh DURATION=259200` | ⏳ | — | — |
| D.4 — fault injection | `scripts/d-track-fault-injection.sh` | ⏳ | — | — |
| D.5 — partition healing | `scripts/d-track-partition.sh` | ⏳ | — | — |

---

## D.1 — Adversarial sweep

**Command:**
```bash
# Run on Mini 1 against each target node:
./scripts/dos-flood.sh --target 100.119.53.101:8080 --rate 1000 --duration 5m
./scripts/dos-flood.sh --target 100.119.53.101:8080 --rate 1000 --duration 5m --garbage-sigs
./scripts/dos-flood.sh --target 100.119.53.101:8080 --rate 1000 --duration 5m --single-sender
```

**Pass criteria** (from `docs/runbooks/dos-resistance.md`):
- Vector 1: mempool ≤ MAX_MEMPOOL_SIZE (10,000); blocks continue at ≥1/s
- Vector 2: mempool stays near 0 (malformed sigs rejected pre-pool)
- Vector 3: single-sender capped at 64 pending; no peer ban fires on honest peers

| Vector | Mempool max | Block rate (blk/s) | Verdict |
|---|---|---|---|
| V1 (flood) | — | — | — |
| V2 (garbage sigs) | — | — | — |
| V3 (single-sender) | — | — | — |

**Notes:**

```
[operator fills in observations]
```

---

## D.2 + D.3 — Performance profile + 72hr soak

**Command:**
```bash
# Quick perf check (5 min):
TPS=1000 DURATION=300 ./scripts/d-track-soak.sh

# 72hr soak:
TPS=1000 DURATION=259200 ./scripts/d-track-soak.sh
```

**Pass criteria (D.2):** block rate ≥ 1.0 blk/s at 1000 tx/s sustained.  
**Pass criteria (D.3):** no unrecovered finality stall in 72hr window.

### D.2 — Perf profile result

| Metric | Observed | Target | Pass? |
|---|---|---|---|
| Block rate (blk/s) | — | ≥ 1.0 | — |
| TX success rate (%) | — | ≥ 50% | — |
| Peak mempool depth | — | ≤ 10,000 | — |
| Max finality lag (s) | — | ≤ 30 | — |

### D.3 — 72hr soak result

| Metric | Observed | Target | Pass? |
|---|---|---|---|
| Run duration (h) | — | 72 | — |
| Total txs submitted | — | — | — |
| Finality stall events | — | 0 | — |
| Unrecovered stalls | — | 0 | — |
| Final block height | — | — | — |
| Block rate over window (blk/s) | — | ≥ 1.0 | — |

**CSV outputs:** `logs/d-track-soak/<run_id>/samples.csv`, `txs.csv`, `summary.txt`

**Notes:**

```
[operator fills in — stall events, recovery times, anomalies]
```

---

## D.4 — Fault injection

**Command:**
```bash
# SSH_RESTART_CMD must be set to the node restart command on each host.
# Example for systemd-managed node:
SSH_RESTART_CMD="sudo systemctl restart evaporchain-node" \
    CYCLES=3 KILL_SECS=60 \
    ./scripts/d-track-fault-injection.sh
```

**Pass criteria:**
- Finality on surviving nodes never stalls >30s while one node is down.
- Killed node rejoins within 120s.
- Block heights converge within 5 blocks after resync.

| Node | Cycles | Kills | Recoveries | Max stall (s) | Heights converged | Verdict |
|---|---|---|---|---|---|---|
| Mini 1 | 3 | — | — | — | — | — |
| Mini 2 | 3 | — | — | — | — | — |
| Mini 3 | 3 | — | — | — | — | — |

**Log:** `/tmp/d-track-fault-<timestamp>.log`

**Notes:**

```
[operator fills in — any nodes that failed to restart, timing anomalies]
```

---

## D.5 — Partition healing

**Command:**
```bash
# Requires passwordless sudo iptables on Mini 1, or use --interactive:
PARTITION_SECS=90 HEAL_TIMEOUT=120 ./scripts/d-track-partition.sh

# Interactive (operator applies iptables rules manually):
./scripts/d-track-partition.sh --interactive
```

**Pass criteria:**
- Partitioned node advances height during isolation (doesn't freeze).
- After healing: node converges within 5 blocks of majority in ≤120s.

| Test | Partition duration (s) | Node height delta during isolation | Post-heal convergence (s) | Verdict |
|---|---|---|---|---|
| Mini 1 ↔ {Mini 2, Mini 3} | 90 | — | — | — |

**Log:** `/tmp/d-track-partition-<timestamp>.log`

**Notes:**

```
[operator fills in]
```

---

## Overall T0.2 verdict

| Sub-task | Verdict |
|---|---|
| D.1 — Adversarial sweep | ⏳ |
| D.2 — ≥1 blk/s perf | ⏳ |
| D.3 — 72hr soak | ⏳ |
| D.4 — Fault injection | ⏳ |
| D.5 — Partition healing | ⏳ |
| **OVERALL** | **⏳ PENDING** |

**Completed by:** [operator name / session ID]  
**Completion date:** —  
**Final commit:** — (operator commits this file when all sub-tasks are PASS)

---

## Reference

| Script | Location | Purpose |
|---|---|---|
| `dos-flood.sh` | `scripts/dos-flood.sh` | D.1 adversarial flood (3 vectors) |
| `d-track-soak.sh` | `scripts/d-track-soak.sh` | D.2+D.3 sustained load + 72hr soak |
| `d-track-fault-injection.sh` | `scripts/d-track-fault-injection.sh` | D.4 kill/restart cycles |
| `d-track-partition.sh` | `scripts/d-track-partition.sh` | D.5 network partition + healing |

DoS runbook: `docs/runbooks/dos-resistance.md`  
Monitoring runbook: `docs/runbooks/monitoring.md`  
Cluster deploy: `docs/runbooks/cluster-deploy.md`
