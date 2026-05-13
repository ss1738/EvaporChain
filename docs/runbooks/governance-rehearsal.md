# Governance Flag Rehearsal — Operator Runbook

**Lane T1.22** — Network upgrade rehearsal (live flag-flip + rollback). The goal is to prove the cluster can flip a governance flag, observe propagation, and roll back cleanly **before** doing it for real against mainnet.

This runbook pairs with `scripts/governance-flip.sh` (the safe-by-default wrapper) and the chain's `POST /api/governance/param` endpoint (`crates/evaporchain-node/src/api.rs:2627`).

Pairs with: `MAINNET_READINESS.md` T1.22, `docs/runbooks/network-upgrade.md` (binary upgrades), `docs/runbooks/monitoring.md` (observability during the rehearsal).

---

## When to run this rehearsal

Run **at least once** against the live `evaporchain-testnet-1` cluster before:
- The first mainnet doctrine flag flip (e.g. `conservation_enforcement = enforce`)
- Any flip that changes consensus regime (`parent_acceptance_mode = mcc_full`)
- Any flip that affects fee market or finality

The rehearsal is non-destructive: pick a flag at its default value, flip it to another safe value, then flip it back. The chain's behaviour should not change in any user-visible way — the rehearsal validates the **flip mechanism itself**, not the new behaviour.

---

## Pre-flight checklist (do not skip)

1. **Cluster lockstep.** All 5 nodes must report the same `block_height` (or within 1):
   ```bash
   for node in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
     curl -fsS --max-time 5 "http://$node:8081/api/chain" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f\"$node: h={d['block_height']}\")"
   done
   ```
   Expect all 5 heights within 1 of each other. If spread > 5, **abort** — sync issues mean the cluster is not in a fit state for an upgrade rehearsal.

2. **No pending finality stall.** Worst unfinalised gap should be < 10s:
   ```bash
   curl -fsS -H "Authorization: Bearer $EVAPORCHAIN_ADMIN_KEY" \
        http://100.119.53.101:8081/metrics | grep evap_worst_unfinalised_gap_seconds
   ```
   If > 10s, **abort** — finality is degraded and a flag flip would mask real issues.

3. **Conservation audit non-null on every node.**
   ```bash
   for node in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
     curl -fsS --max-time 5 "http://$node:8081/api/four_act" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f\"$node: last_audit={d.get('last_conservation_audit_ok')}\")"
   done
   ```
   All 5 should report a recent timestamp.

4. **Admin auth ready.** `EVAPORCHAIN_ADMIN_KEY` exported in the operator shell. The same value must be set on every node's process env (see `docs/runbooks/monitoring.md`).

5. **Monitoring active.** Grafana dashboard (`scripts/grafana-dashboards/evaporchain-chain.json`) is open in a browser tab. The alert pipeline is wired so post-flip anomalies fire.

6. **Operator buddy in chat.** A second operator should be online for the rehearsal — to call out anomalies you might miss and to be the second pair of eyes on the rollback decision.

7. **Choose the rehearsal flag.** Recommended for first rehearsal:
   - `crooks_mev_settlement_mode`: default `observe` → flip to `observe` (no-op flip) — this rehearses the flip mechanism without changing behaviour. *Lowest-risk first rehearsal.*
   - Once that succeeds, repeat with a behaviour-changing pair, e.g. `crooks_mev_settlement_mode: observe → enforce → observe`.
   - **Do NOT** rehearse `conservation_enforcement: enforce` or `parent_acceptance_mode: mcc_full` on the first rehearsal — those are real regime changes that need a separate run.

---

## Execution

### Step 1 — capture cluster baseline

Take a snapshot of every node's state before the flip:

```bash
mkdir -p /tmp/governance-rehearsal-$(date +%s)
cd /tmp/governance-rehearsal-*

for node in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -fsS --max-time 5 "http://$node:8081/api/governance/flags" > "flags-${node}-pre.json"
  curl -fsS --max-time 5 "http://$node:8081/api/chain"            > "chain-${node}-pre.json"
done
```

All 5 `flags-*-pre.json` should be byte-identical (same flag values across the cluster).

### Step 2 — flip the flag forward

```bash
./scripts/governance-flip.sh crooks_mev_settlement_mode enforce
```

The wrapper:
1. Captures the current value (for rollback hint)
2. Runs the relevant readiness script (`crooks-mev-readiness.py` for this flag)
3. Prints the rollback command and prompts for confirmation
4. POSTs to `/api/governance/param`
5. Watches propagation across the cluster for 30s

Pass criteria (the script exits 0 only if all are met):
- Readiness check returned green
- POST succeeded (HTTP 200)
- All 5 nodes report the new value within 30s

### Step 3 — observe for 5 minutes

Watch the dashboard. Specifically:

- **Block height advances on all 5 nodes** at the pre-flip cadence (≥ 1 block / 5s)
- **No alert from the starter set fires** (no height stall, no peer loss, no finality stall, no phase drift)
- **Cluster height spread stays < 3** (some natural jitter from the flip itself is OK; sustained spread is not)
- **`evap_finality_gap_seconds`** doesn't spike past 10s on any node

If anything looks wrong, **stop the rehearsal and proceed to rollback immediately** (Step 5).

### Step 4 — sample mid-flip state

```bash
for node in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -fsS --max-time 5 "http://$node:8081/api/governance/flags" > "flags-${node}-post.json"
done
diff <(sort flags-100.119.53.101-post.json) <(sort flags-100.113.253.72-post.json)
```

The new flag value should be byte-identical across all 5 nodes. Any disagreement means propagation didn't complete — investigate before rolling back.

### Step 5 — roll back

```bash
./scripts/governance-flip.sh crooks_mev_settlement_mode observe
```

Same wrapper, inverse value. The script will prompt for confirmation and watch propagation again. Rolling back is exactly the same operation as flipping forward — there is no "rollback mode" that does anything special.

### Step 6 — validate post-rehearsal state

Re-run Step 1 with the `-post-rollback` suffix:

```bash
for node in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -fsS --max-time 5 "http://$node:8081/api/governance/flags" > "flags-${node}-post-rollback.json"
done
```

`flags-*-post-rollback.json` should match `flags-*-pre.json` byte-for-byte. If they don't, the chain has settled to a different config than where it started — that's a real finding to triage.

Hold the cluster for another **5 minutes** and watch the dashboard. The chain should be back to its baseline state with no residual alerts.

---

## Post-rehearsal report

Capture in `docs/runbooks/governance-rehearsal-log.md` (or wherever the operator team keeps run logs):

| Field | Value |
|---|---|
| Date | YYYY-MM-DD |
| Flag rehearsed | `<flag_name>` |
| Forward value | `<value>` |
| Cluster height at flip | h=… |
| Propagation time (s) | … |
| Alerts fired (if any) | none / … |
| Rollback successful | ✅ / ❌ |
| Anomalies observed | … |
| Next rehearsal candidate | `<flag_name>` |

A successful rehearsal is a single row with no anomalies. A failed rehearsal — alerts fired, propagation stalled, byte-mismatch on post-rollback — is a real bug to fix before mainnet flips this flag.

---

## What this runbook does NOT do

- Does not cover **binary upgrades** — that's `docs/runbooks/network-upgrade.md`.
- Does not cover **genesis amendments** — that's T1.23 / `MAINNET_READINESS.md`.
- Does not cover **mainnet flips** — once the rehearsal is green across all the flags we care about, mainnet flips follow the same protocol but with a buddy + on-call + rollback path explicitly approved up-front.

---

## Cross-references

- `MAINNET_READINESS.md` T1.22
- `scripts/governance-flip.sh` — the wrapper this runbook drives
- `scripts/mcc-readiness.py` / `scripts/crooks-mev-readiness.py` — readiness checks
- `crates/evaporchain-node/src/api.rs:2627` — `post_governance_param` source of truth
- `crates/evaporchain-node/src/api.rs:2659` — `get_governance_flags` (used in Step 1)
- `docs/runbooks/monitoring.md` — Grafana dashboard + alert rules to watch during the rehearsal
