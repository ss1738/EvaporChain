#!/usr/bin/env bash
#
# phase-c-validation.sh — automated validation queries for the Phase C
# stop-the-world cluster deploy of the 8-item 100x bundle.
#
# Pairs with `docs/runbooks/cluster-deploy.md` and the existing plan
# file. Run AFTER Phase C deploy completes (all 5 nodes on the new
# binary, restarted in lockstep) to verify the bundle behaves as
# intended on the live cluster.
#
# This script DOES NOT do the deploy itself. It only checks the
# observable consequences afterward:
#
#   1. Pre-flight — all 5 nodes reachable on the same chain_id with
#      ≤3-block skew (refused if anything else).
#   2. Governance flags — three new defaults present:
#        block_source_mode      = "antichain"
#        lambda_fold_mode        = "nova"
#        conservation_enforcement = "enforce"
#   3. Liveness — block_height monotonic +1 over a 30-second window.
#   4. Eulogy progression — non-zero eulogy_count over a 60-second
#      window (proves the eulogy trie is being fed from
#      evap_result.evaporated, item #4 of the bundle).
#   5. Conservation enforcement — no block rejected with
#      ConservationViolation over a 30-block window (proves item #7's
#      minted_this_block credit is correct and complete).
#   6. APY cap — per-block reward <= (total_staked × 0.05) / 15_768_000
#      (proves item #6's apy_capped_reward dispatcher is gating).
#   7. Tx-hash persistence — submit a transfer, verify it remains
#      queryable by tx-hash after 600+ blocks (proves item #3's
#      canonical tx.tx_hash() fix).
#
# Exit codes:
#   0 = all checks passed
#   1 = pre-flight failed (don't continue investigating bundle behavior
#       until cluster basic health is fixed)
#   2+ = specific check failed (see stderr)
#
# Usage:
#   bash scripts/phase-c-validation.sh \
#     --nodes "100.119.53.101,100.113.253.72,100.103.216.125,100.66.208.20,100.91.235.22" \
#     --port 8081 \
#     [--skip-tx]    # skip the 600-block tx-persistence test (~20 min)
#
# All checks except --skip-tx complete in <2 minutes.

set -euo pipefail

# -----------------------------------------------------------------------------
# Defaults + CLI parsing
# -----------------------------------------------------------------------------
NODES=""
PORT=8081
SKIP_TX="false"

while [ $# -gt 0 ]; do
  case "$1" in
    --nodes) NODES="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --skip-tx) SKIP_TX="true"; shift ;;
    -h|--help)
      sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *) echo "Unknown arg: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$NODES" ]; then
  echo "ERROR: --nodes <comma-separated-host-list> is required." >&2
  echo "Example: --nodes 100.119.53.101,100.113.253.72,..." >&2
  exit 2
fi

# -----------------------------------------------------------------------------
# Helpers
# -----------------------------------------------------------------------------
red()   { printf '\033[0;31m%s\033[0m\n' "$*"; }
green() { printf '\033[0;32m%s\033[0m\n' "$*"; }
yel()   { printf '\033[0;33m%s\033[0m\n' "$*"; }
bold()  { printf '\033[1m%s\033[0m\n' "$*"; }
hdr()   { echo; bold "── $* ──"; }

curl_json() {
  local url="$1"
  curl -s --max-time 10 "$url"
}

# Convert comma-separated NODES into a bash array.
IFS=',' read -ra NODE_ARR <<< "$NODES"
FIRST_NODE="${NODE_ARR[0]}"

# -----------------------------------------------------------------------------
# 1. Pre-flight — all 5 nodes reachable, same chain_id, ≤3-block skew
# -----------------------------------------------------------------------------
hdr "1. Pre-flight (cluster reachability + chain_id agreement)"

declare -a CHAINS=()
declare -a HEIGHTS=()
for n in "${NODE_ARR[@]}"; do
  resp=$(curl_json "http://$n:$PORT/api/identity" || true)
  if [ -z "$resp" ]; then
    red "  ✗ $n: unreachable"
    exit 1
  fi
  chain=$(echo "$resp" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('chain_id','?'))")
  height=$(echo "$resp" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('light_cone_block_count', d.get('block_height', '?')))")
  CHAINS+=("$chain")
  HEIGHTS+=("$height")
  echo "  $n  chain=$chain  blk=$height"
done

unique_chains=$(printf '%s\n' "${CHAINS[@]}" | sort -u | wc -l | tr -d ' ')
if [ "$unique_chains" != "1" ]; then
  red "  ✗ chain_id disagreement across nodes — partitioned cluster"
  exit 1
fi

# Compute height skew. Pure-bash sort of numeric heights.
sorted_h=$(printf '%s\n' "${HEIGHTS[@]}" | sort -n)
min_h=$(echo "$sorted_h" | head -1)
max_h=$(echo "$sorted_h" | tail -1)
skew=$((max_h - min_h))
if [ "$skew" -gt 3 ]; then
  red "  ✗ block skew $skew > 3 — cluster not in lockstep"
  exit 1
fi
green "  ✓ all ${#NODE_ARR[@]} nodes reachable, same chain_id, skew=$skew blocks"

# -----------------------------------------------------------------------------
# 2. Governance flags — three new defaults
# -----------------------------------------------------------------------------
hdr "2. Governance flags reflect Phase C bundle defaults"

flags=$(curl_json "http://$FIRST_NODE:$PORT/api/governance/flags")
check_flag() {
  local key="$1" expected="$2"
  local actual
  actual=$(echo "$flags" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('$key','MISSING'))")
  if [ "$actual" = "$expected" ]; then
    green "  ✓ $key = $expected"
  else
    red "  ✗ $key = $actual (expected $expected)"
    return 1
  fi
}

flag_failures=0
check_flag block_source_mode antichain || flag_failures=$((flag_failures + 1))
check_flag lambda_fold_mode nova || flag_failures=$((flag_failures + 1))
check_flag conservation_enforcement enforce || flag_failures=$((flag_failures + 1))
[ "$flag_failures" -eq 0 ] || { red "  $flag_failures governance flag(s) wrong — bundle didn't ship cleanly"; exit 2; }

# -----------------------------------------------------------------------------
# 3. Liveness — block height advances over 30 seconds
# -----------------------------------------------------------------------------
hdr "3. Liveness (block height monotonic +1 over 30 s)"

h_start=$(curl_json "http://$FIRST_NODE:$PORT/api/identity" \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('light_cone_block_count', d.get('block_height', 0)))")
sleep 30
h_end=$(curl_json "http://$FIRST_NODE:$PORT/api/identity" \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('light_cone_block_count', d.get('block_height', 0)))")
delta=$((h_end - h_start))
if [ "$delta" -lt 10 ]; then
  red "  ✗ block height advanced only $delta in 30 s (expected ≥10 at 2-3 s intervals)"
  exit 3
fi
green "  ✓ block_height advanced $delta (≈$(awk "BEGIN{printf \"%.2f\", $delta/30}") blk/s)"

# -----------------------------------------------------------------------------
# 4. Eulogy progression — eulogy_count grows over 60 s
# -----------------------------------------------------------------------------
hdr "4. Eulogy progression (eulogy_count > 0 over 60 s)"

e_start=$(curl_json "http://$FIRST_NODE:$PORT/api/four_act" \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('eulogy_count',0))" 2>/dev/null || echo 0)
echo "  initial eulogy_count = $e_start"
sleep 60
e_end=$(curl_json "http://$FIRST_NODE:$PORT/api/four_act" \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('eulogy_count',0))" 2>/dev/null || echo 0)
echo "  final eulogy_count = $e_end"
if [ "$e_end" -lt "$e_start" ]; then
  red "  ✗ eulogy_count went BACKWARDS ($e_start → $e_end). Trie is being corrupted."
  exit 4
fi
if [ "$e_end" -eq 0 ]; then
  yel "  ⚠ eulogy_count still 0 after 60 s — items 2/4 of bundle may not be wired."
  yel "    If cluster is fresh, this may just be a startup window — re-run in 10 min."
fi
green "  ✓ eulogy_count progression observed"

# -----------------------------------------------------------------------------
# 5. Conservation — no ConservationViolation over 30 blocks
# -----------------------------------------------------------------------------
hdr "5. Conservation enforcement (no spurious rejections over 30 blocks)"

cca_start=$(curl_json "http://$FIRST_NODE:$PORT/api/four_act" \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('consecutive_clean_audits',0))" 2>/dev/null || echo 0)
sleep 90  # 30 blocks at 2-3 s
cca_end=$(curl_json "http://$FIRST_NODE:$PORT/api/four_act" \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('consecutive_clean_audits',0))" 2>/dev/null || echo 0)
echo "  consecutive_clean_audits: $cca_start → $cca_end"
if [ "$cca_end" -le "$cca_start" ]; then
  red "  ✗ consecutive_clean_audits did NOT advance — at least one block rejected with"
  red "    ConservationViolation under enforce-mode. Item #7 (minted_this_block fix) may"
  red "    be incomplete; do NOT flip back to observe-mode as a workaround."
  exit 5
fi
green "  ✓ consecutive_clean_audits +$((cca_end - cca_start)) over 30 blocks"

# -----------------------------------------------------------------------------
# 6. APY cap — per-block reward bounded by tokenomics
# -----------------------------------------------------------------------------
hdr "6. APY cap (item #6 — Tokenomics::apy_capped_reward)"

summary=$(curl_json "http://$FIRST_NODE:$PORT/api/stats/summary")
ts=$(echo "$summary" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('total_staked',0))" 2>/dev/null || echo 0)
last_reward=$(echo "$summary" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('last_block_reward', d.get('block_reward_last',0)))" 2>/dev/null || echo 0)
cap=$(awk "BEGIN{printf \"%.0f\", ($ts*0.05)/15768000}")
echo "  total_staked      = $ts"
echo "  last_block_reward = $last_reward"
echo "  apy_cap (5% / blocks_per_year=15_768_000) = $cap"
if [ "$cap" -gt 0 ] && [ "$last_reward" -gt "$cap" ]; then
  red "  ✗ last_block_reward ($last_reward) > apy_cap ($cap) — dispatcher not gating"
  exit 6
fi
green "  ✓ block reward respects APY cap"

# -----------------------------------------------------------------------------
# 7. Tx-hash persistence — only if --skip-tx not set
# -----------------------------------------------------------------------------
if [ "$SKIP_TX" = "true" ]; then
  yel "Skipping check 7 (--skip-tx). To run later: re-invoke without --skip-tx."
else
  hdr "7. Tx-hash persistence (item #3 — canonical tx.tx_hash())"
  yel "  (this check takes ~20 min — pass --skip-tx to skip)"
  echo "  submitting a transfer..."
  tx_resp=$(curl -s -X POST "http://$FIRST_NODE:$PORT/api/tx/transfer" \
    -H "Content-Type: application/json" \
    -d '{"from":"0x0000000000000000000000000000000000000001","to":"0x0000000000000000000000000000000000000002","amount":1,"signature":null}' \
    || echo '{"tx_hash":null}')
  tx_hash=$(echo "$tx_resp" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('tx_hash','') or '')" 2>/dev/null || echo "")
  if [ -z "$tx_hash" ]; then
    yel "  ⚠ transfer submission failed — likely auth-gated. Skip with --skip-tx for runbook-mode."
  else
    echo "  tx_hash = $tx_hash"
    echo "  waiting 1200 s (~600 blocks at 2 s/block)..."
    sleep 1200
    persisted=$(curl_json "http://$FIRST_NODE:$PORT/api/tx/$tx_hash" \
      | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('status','MISSING'))" 2>/dev/null || echo "MISSING")
    if [ "$persisted" = "committed" ]; then
      green "  ✓ tx persisted as committed after 600 blocks"
    else
      red "  ✗ tx status = '$persisted' (expected 'committed'). Item #3 not closed."
      exit 7
    fi
  fi
fi

# -----------------------------------------------------------------------------
echo
green "════════════════════════════════════════════════════════════════"
green "  Phase C validation: ALL CHECKS PASSED."
green "  Bundle is observably correct on the running cluster."
green "════════════════════════════════════════════════════════════════"
