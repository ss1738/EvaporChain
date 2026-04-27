#!/usr/bin/env bash
#
# EvaporChain — sustained-load multi-node stress harness.
#
# Difference vs scripts/stress-test.sh:
#   stress-test.sh  → local 4-node devnet, 50-tx burst, 5-phase
#                     correctness-style harness
#   this script     → drives an EXISTING running cluster (3-Mini Tailscale
#                     by default) at a sustained TPS for a configurable
#                     duration, polls metrics every 2s, writes CSVs for
#                     post-hoc analysis.
#
# Outputs (in $LOG_DIR):
#   <run_id>-samples.csv  ts,elapsed,target,height,epoch,mempool_pending,
#                          active_objects,ghost_count,uptime_s
#   <run_id>-txs.log      ts,target,http_code  (one line per submitted tx)
#
# !!! KNOWN LIMITATION (2026-04-27 first-run):
# The submitter currently posts to /api/faucet which is rate-limited
# server-side ("Try again in 59 minutes"). Run 20260427-205045 against
# a single node returned 1014/1015 HTTP 429 — useful for proving the
# harness loop, useless for measuring real cluster throughput.
# A future revision should switch to a signed transfer or a
# devnet-only bulk-submit endpoint. The harness now warns at end of
# run when >50% of submissions hit 429 so this isn't silent.
#
# Usage:
#   ./scripts/sustained-stress.sh
#
# Tunable env vars (with defaults):
#   TARGETS   = comma-separated host:port list of API endpoints
#               (defaults to the 3-Mini Tailscale cluster on :8080)
#   TPS       = target aggregate transactions per second across all workers
#   DURATION  = total run duration in seconds
#   WORKERS   = number of concurrent submitter processes (round-robin
#               across TARGETS)
#   LOG_DIR   = where to write CSVs
#
# Prereqs: curl, python3 (for JSON extraction).
#
# Caveat: this depends on the cluster reachability. If Tailscale is down
# or a node is unhealthy the probe phase will warn and let you decide
# whether to proceed.

set -euo pipefail

TARGETS="${TARGETS:-100.119.53.101:8080,100.113.253.72:8080,100.103.216.125:8080}"
TPS="${TPS:-50}"
DURATION="${DURATION:-300}"
WORKERS="${WORKERS:-10}"
LOG_DIR="${LOG_DIR:-./logs/sustained-stress}"
RUN_ID="$(date +%Y%m%d-%H%M%S)"

mkdir -p "$LOG_DIR"
SAMPLE_LOG="$LOG_DIR/${RUN_ID}-samples.csv"
TX_LOG="$LOG_DIR/${RUN_ID}-txs.log"

IFS=',' read -ra TARGET_ARR <<< "$TARGETS"
NUM_TARGETS=${#TARGET_ARR[@]}

if [ "$NUM_TARGETS" -eq 0 ]; then
    echo "ERROR: TARGETS env var is empty"
    exit 1
fi

# ── Probe ──
echo "━━━ Probing ${NUM_TARGETS} target(s) ━━━"
REACHABLE=0
for t in "${TARGET_ARR[@]}"; do
    HTTP_CODE=$(curl -s -m 5 -o /dev/null -w "%{http_code}" "http://${t}/health" 2>/dev/null || echo "000")
    if [ "$HTTP_CODE" = "200" ]; then
        echo "  OK   $t"
        REACHABLE=$((REACHABLE + 1))
    else
        echo "  WARN $t  (HTTP $HTTP_CODE)"
    fi
done
if [ "$REACHABLE" -eq 0 ]; then
    echo "ERROR: no targets reachable; aborting."
    exit 2
fi

# ── Header ──
echo ""
echo "━━━ Run ${RUN_ID} ━━━"
echo "  Targets:  ${TARGETS}"
echo "  TPS:      ${TPS}"
echo "  Duration: ${DURATION}s"
echo "  Workers:  ${WORKERS}"
echo "  Sample log: ${SAMPLE_LOG}"
echo "  TX log:     ${TX_LOG}"
echo ""

echo "ts,elapsed,target,height,epoch,mempool_pending,active_objects,ghost_count,uptime_s" > "$SAMPLE_LOG"
: > "$TX_LOG"

START_TIME=$(date +%s)

# ── Cleanup on exit ──
PIDS=()
cleanup() {
    for pid in "${PIDS[@]+"${PIDS[@]}"}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# ── JSON field extractor (one-shot python3) ──
jget() {
    # $1 = json string, $2 = key
    python3 -c "import sys,json
try:
    d = json.loads(sys.argv[1])
    print(d.get(sys.argv[2], -1))
except Exception:
    print(-1)" "$1" "$2" 2>/dev/null || echo "-1"
}

# ── Poller (runs in background) ──
poller() {
    while true; do
        NOW=$(date +%s)
        ELAPSED=$((NOW - START_TIME))
        if [ "$ELAPSED" -ge "$DURATION" ]; then
            break
        fi
        for t in "${TARGET_ARR[@]}"; do
            STATUS=$(curl -s -m 3 "http://${t}/api/status" 2>/dev/null || echo "{}")
            MEMPOOL=$(curl -s -m 3 "http://${t}/api/mempool" 2>/dev/null || echo "{}")
            H=$(jget "$STATUS" block_height)
            E=$(jget "$STATUS" epoch)
            AO=$(jget "$STATUS" active_objects)
            GC=$(jget "$STATUS" ghost_count)
            US=$(jget "$STATUS" uptime_seconds)
            MP=$(jget "$MEMPOOL" pending)
            echo "${NOW},${ELAPSED},${t},${H},${E},${MP},${AO},${GC},${US}" >> "$SAMPLE_LOG"
        done
        sleep 2
    done
}

# ── Worker (runs in background, one per WORKER) ──
# Per-worker interval = WORKERS / TPS  (so aggregate hits TPS)
INTERVAL=$(awk "BEGIN { printf \"%.4f\", ${WORKERS} / ${TPS} }")

worker() {
    local wid=$1
    local target_idx=$((wid % NUM_TARGETS))
    local target=${TARGET_ARR[$target_idx]}
    local addr_seed=$(((wid + 1) * 1000000))
    while true; do
        NOW=$(date +%s)
        ELAPSED=$((NOW - START_TIME))
        if [ "$ELAPSED" -ge "$DURATION" ]; then
            break
        fi
        addr_seed=$((addr_seed + 1))
        addr=$(printf '%064x' "$addr_seed")
        CODE=$(curl -s -m 5 -o /dev/null -w "%{http_code}" \
            -X POST "http://${target}/api/faucet" \
            -H 'Content-Type: application/json' \
            -d "{\"address\":\"${addr}\"}" 2>/dev/null || echo "000")
        echo "${NOW},${target},${CODE}" >> "$TX_LOG"
        # sleep accepts fractional seconds on both Linux and macOS
        sleep "$INTERVAL"
    done
}

# ── Launch ──
poller &
PIDS+=($!)
for w in $(seq 1 "$WORKERS"); do
    worker "$w" &
    PIDS+=($!)
done

echo "━━━ Running for ${DURATION}s ━━━"
sleep "$DURATION"
cleanup
trap - EXIT

# ── Summary ──
TOTAL_TX=$(wc -l < "$TX_LOG" | tr -d ' ')
OK_TX=$(grep -c ',200' "$TX_LOG" 2>/dev/null || echo 0)
RATE_LIMITED_TX=$(grep -c ',429' "$TX_LOG" 2>/dev/null || echo 0)
SAMPLES=$(($(wc -l < "$SAMPLE_LOG" | tr -d ' ') - 1))

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Sustained stress complete"
echo "  Run ID:       ${RUN_ID}"
echo "  Submitted:    ${TOTAL_TX} txs (${OK_TX} HTTP 200, ${RATE_LIMITED_TX} HTTP 429)"
echo "  Samples:      ${SAMPLES} rows in ${SAMPLE_LOG}"
echo "  TX log:       ${TX_LOG}"

# Surface rate-limit dominance — the most common reason a run "succeeds"
# but produces no useful stress signal.
if [ "$TOTAL_TX" -gt 0 ]; then
    HALF=$((TOTAL_TX / 2))
    if [ "$RATE_LIMITED_TX" -gt "$HALF" ]; then
        echo ""
        echo "  WARNING: >50% of submissions returned HTTP 429 (rate limited)."
        echo "  The /api/faucet endpoint enforces a per-address cooldown — this"
        echo "  harness is useful for proving the submit/poll loop works but is"
        echo "  not measuring real chain throughput. Switch to a signed transfer"
        echo "  endpoint (or a devnet --no-rate-limit mode) before drawing"
        echo "  conclusions about TPS or block production."
    fi
fi

echo ""
echo "  Quick analysis:"
echo "    head -3 ${SAMPLE_LOG}"
echo "    tail -3 ${SAMPLE_LOG}"
echo "    awk -F, 'NR>1 { print \$3, \$6 }' ${SAMPLE_LOG} | sort | uniq -c"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
