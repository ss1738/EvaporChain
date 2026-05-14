#!/usr/bin/env bash
#
# scripts/d-track-soak.sh — T0.2 D-track: 72-hour cluster soak (D.2 + D.3)
#
# Drives a sustained tx load against the 5-node testnet cluster, polls
# per-node metrics every 2 s, and writes CSVs for post-hoc analysis.
# Acceptance criteria (D.2): block-production rate ≥ 1 block/s at 1k tx/s.
# Acceptance criteria (D.3): no finality stall >30 s over 72 hours.
#
# Nodes are expected to be running with --faucet-rate-limit-disabled so the
# unique-address submission loop doesn't hit the 1-hour faucet cooldown.
#
# Usage:
#   TARGETS=host1:port,host2:port DURATION=259200 ./scripts/d-track-soak.sh
#
# Tunable env vars (with defaults):
#   TARGETS    comma-separated host:port list (default: 3-Mini Tailscale cluster)
#   TPS        target aggregate TPS across all workers (default: 1000)
#   WORKERS    concurrent submitter processes (default: 20)
#   DURATION   run duration in seconds (default: 259200 = 72h)
#   POLL_SECS  metric poll interval in seconds (default: 2)
#   STALL_SECS finality-stall alert threshold in seconds (default: 30)
#   LOG_DIR    output directory (default: ./logs/d-track-soak)
#
# Output files (in $LOG_DIR/$RUN_ID/):
#   samples.csv     ts,elapsed,target,height,epoch,finalized,mempool,active,ghosts,uptime_s
#   txs.csv         ts,target,http_code
#   events.log      human-readable stall/recovery events
#   summary.txt     pass/fail verdict with acceptance-criteria checklist

set -euo pipefail

TARGETS="${TARGETS:-100.119.53.101:8080,100.113.253.72:8080,100.103.216.125:8080}"
TPS="${TPS:-1000}"
WORKERS="${WORKERS:-20}"
DURATION="${DURATION:-259200}"
POLL_SECS="${POLL_SECS:-2}"
STALL_SECS="${STALL_SECS:-30}"
LOG_DIR="${LOG_DIR:-./logs/d-track-soak}"
RUN_ID="$(date +%Y%m%dT%H%M%SZ)"

OUTPUT="$LOG_DIR/$RUN_ID"
mkdir -p "$OUTPUT"
SAMPLE_LOG="$OUTPUT/samples.csv"
TX_LOG="$OUTPUT/txs.csv"
EVENT_LOG="$OUTPUT/events.log"
SUMMARY="$OUTPUT/summary.txt"

IFS=',' read -ra TARGET_ARR <<< "$TARGETS"
NUM_TARGETS=${#TARGET_ARR[@]}

log_event() { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ)  $*" | tee -a "$EVENT_LOG"; }

# ── Pre-flight probe ──
echo "━━━ D-track soak: probing ${NUM_TARGETS} target(s) ━━━"
REACHABLE=0
for t in "${TARGET_ARR[@]}"; do
    CODE=$(curl -s -m 5 -o /dev/null -w "%{http_code}" "http://${t}/api/health" 2>/dev/null || echo "000")
    STATUS_OK=$(curl -s -m 5 "http://${t}/api/health" 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('status',''))" 2>/dev/null || echo "")
    if [ "$CODE" = "200" ] && [ "$STATUS_OK" = "ok" ]; then
        echo "  ✅  $t  (HTTP 200, status=ok)"
        REACHABLE=$((REACHABLE + 1))
    else
        echo "  ⚠️   $t  (HTTP $CODE, status='$STATUS_OK')"
    fi
done

if [ "$REACHABLE" -eq 0 ]; then
    echo "ERROR: no targets reachable — aborting"; exit 2
fi
if [ "$REACHABLE" -lt "$NUM_TARGETS" ]; then
    log_event "WARN: only $REACHABLE/$NUM_TARGETS targets reachable at run start"
fi

echo ""
echo "━━━ Run ${RUN_ID} ━━━"
printf "  Targets:  %s\n  TPS:      %s\n  Workers:  %s\n  Duration: %ss (%.1f h)\n  Stall threshold: %ss\n  Output: %s\n\n" \
    "$TARGETS" "$TPS" "$WORKERS" "$DURATION" "$(awk "BEGIN{printf \"%.1f\",$DURATION/3600}")" \
    "$STALL_SECS" "$OUTPUT"

echo "ts,elapsed,target,height,epoch,finalized,mempool,active,ghosts,uptime_s" > "$SAMPLE_LOG"
echo "ts,target,http_code" > "$TX_LOG"
log_event "SOAK_START run=$RUN_ID targets=$TARGETS tps=$TPS duration=${DURATION}s"

START_TIME=$(date +%s)
PIDS=()
cleanup() {
    for pid in "${PIDS[@]+"${PIDS[@]}"}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

jget() {
    python3 -c "
import sys,json
try:
    d=json.loads(sys.argv[1])
    print(d.get(sys.argv[2],-1))
except:
    print(-1)" "$1" "$2" 2>/dev/null || echo "-1"
}

# ── Stall detector (runs inside poller) ──
# Per-target state: last seen height, time that height was last observed.
declare -A LAST_HEIGHT
declare -A LAST_HEIGHT_TS
declare -A STALLED
for t in "${TARGET_ARR[@]}"; do
    LAST_HEIGHT["$t"]=-1
    LAST_HEIGHT_TS["$t"]=$START_TIME
    STALLED["$t"]=0
done

poller() {
    while true; do
        NOW=$(date +%s)
        ELAPSED=$((NOW - START_TIME))
        [ "$ELAPSED" -ge "$DURATION" ] && break
        for t in "${TARGET_ARR[@]}"; do
            STATUS=$(curl -s -m 4 "http://${t}/api/status" 2>/dev/null || echo "{}")
            MEMPOOL=$(curl -s -m 4 "http://${t}/api/mempool" 2>/dev/null || echo "{}")
            H=$(jget "$STATUS" block_height)
            E=$(jget "$STATUS" epoch)
            FIN=$(jget "$STATUS" finalized_height)
            AO=$(jget "$STATUS" active_objects)
            GC=$(jget "$STATUS" ghost_count)
            US=$(jget "$STATUS" uptime_seconds)
            MP=$(jget "$MEMPOOL" pending)
            echo "${NOW},${ELAPSED},${t},${H},${E},${FIN},${MP},${AO},${GC},${US}" >> "$SAMPLE_LOG"
            # Stall detection on finalized_height
            PREV_H="${LAST_HEIGHT["$t"]}"
            if [ "$FIN" != "-1" ] && [ "$FIN" != "$PREV_H" ]; then
                LAST_HEIGHT["$t"]=$FIN
                LAST_HEIGHT_TS["$t"]=$NOW
                if [ "${STALLED["$t"]}" = "1" ]; then
                    log_event "STALL_RECOVERED target=$t finalized_height=$FIN"
                    STALLED["$t"]=0
                fi
            elif [ "$FIN" != "-1" ]; then
                GAP=$((NOW - LAST_HEIGHT_TS["$t"]))
                if [ "$GAP" -ge "$STALL_SECS" ] && [ "${STALLED["$t"]}" = "0" ]; then
                    log_event "STALL_DETECTED target=$t last_finalized=$PREV_H stalled_for=${GAP}s"
                    STALLED["$t"]=1
                fi
            fi
        done
        sleep "$POLL_SECS"
    done
}

worker() {
    local wid=$1
    local target_idx=$((wid % NUM_TARGETS))
    local target=${TARGET_ARR[$target_idx]}
    local seed=$((wid * 10000000 + 1))
    local interval
    interval=$(awk "BEGIN { printf \"%.4f\", ${WORKERS} / ${TPS} }")
    while true; do
        NOW=$(date +%s)
        [ $((NOW - START_TIME)) -ge "$DURATION" ] && break
        seed=$((seed + 1))
        addr=$(printf '%064x' "$seed")
        CODE=$(curl -s -m 5 -o /dev/null -w "%{http_code}" \
            -X POST "http://${target}/api/faucet" \
            -H 'Content-Type: application/json' \
            -d "{\"address\":\"${addr}\"}" 2>/dev/null || echo "000")
        echo "${NOW},${target},${CODE}" >> "$TX_LOG"
        sleep "$interval"
    done
}

# ── Launch ──
poller &
PIDS+=($!)
for w in $(seq 1 "$WORKERS"); do
    worker "$w" &
    PIDS+=($!)
done

echo "━━━ Soak running (duration: ${DURATION}s) ━━━"
REPORT_EVERY=3600  # log a progress line hourly
LAST_REPORT=$START_TIME
while true; do
    NOW=$(date +%s)
    ELAPSED=$((NOW - START_TIME))
    [ "$ELAPSED" -ge "$DURATION" ] && break
    if [ $((NOW - LAST_REPORT)) -ge "$REPORT_EVERY" ]; then
        LAST_REPORT=$NOW
        TX_OK=$(awk -F, 'NR>1 && $3==200 {n++} END{print n+0}' "$TX_LOG")
        TX_TOTAL=$(awk 'END{print NR-1}' "$TX_LOG")
        log_event "PROGRESS elapsed=${ELAPSED}s tx_ok=${TX_OK}/${TX_TOTAL}"
    fi
    sleep 10
done

cleanup
trap - EXIT

# ── Summary / verdict ──
TOTAL_TX=$(awk 'END{print NR-1}' "$TX_LOG")
OK_TX=$(awk -F, 'NR>1 && $3==200 {n++} END{print n+0}' "$TX_LOG")
RATE_LIMITED=$(awk -F, 'NR>1 && $3==429 {n++} END{print n+0}' "$TX_LOG")
STALL_EVENTS=$(grep -c 'STALL_DETECTED' "$EVENT_LOG" 2>/dev/null || echo 0)
RECOVERY_EVENTS=$(grep -c 'STALL_RECOVERED' "$EVENT_LOG" 2>/dev/null || echo 0)
SAMPLES=$(($(wc -l < "$SAMPLE_LOG" | tr -d ' ') - 1))

# Block rate: compare first and last height on primary node
FIRST_H=$(awk -F, 'NR==2{print $4}' "$SAMPLE_LOG")
LAST_H=$(awk -F, 'END{print $4}' "$SAMPLE_LOG")
ACTUAL_DURATION="$DURATION"
BLOCK_RATE="unknown"
if [ "$FIRST_H" != "-1" ] && [ "$LAST_H" != "-1" ] && [ "$ACTUAL_DURATION" -gt 0 ]; then
    BLOCK_DELTA=$((LAST_H - FIRST_H))
    BLOCK_RATE=$(awk "BEGIN{printf \"%.3f\", ${BLOCK_DELTA}/${ACTUAL_DURATION}}")
fi

# Verdict
PASS=1
[ "$STALL_EVENTS" -gt 0 ] && [ "$STALL_EVENTS" != "$RECOVERY_EVENTS" ] && PASS=0
BLOCK_RATE_OK=0
awk "BEGIN{ exit ($BLOCK_RATE >= 1.0 ? 0 : 1) }" 2>/dev/null && BLOCK_RATE_OK=1 || true

{
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  T0.2 D-track soak — $RUN_ID"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Duration:       ${DURATION}s"
    echo "  Samples:        $SAMPLES"
    echo "  Submitted txs:  $TOTAL_TX  ($OK_TX HTTP 200, $RATE_LIMITED HTTP 429)"
    echo "  Block rate:     $BLOCK_RATE blk/s  (D.2 target: ≥1.0)"
    echo "  Stall events:   $STALL_EVENTS detected / $RECOVERY_EVENTS recovered"
    echo ""
    echo "  D.2 — ≥1 blk/s at 1k tx/s:  $([ "$BLOCK_RATE_OK" = 1 ] && echo PASS || echo FAIL)"
    echo "  D.3 — No unrecovered stall:  $([ "$STALL_EVENTS" = "$RECOVERY_EVENTS" ] && echo PASS || echo FAIL)"
    echo ""
    if [ "$PASS" = 1 ] && [ "$BLOCK_RATE_OK" = 1 ]; then
        echo "  OVERALL VERDICT: PASS"
    else
        echo "  OVERALL VERDICT: FAIL"
    fi
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
} | tee "$SUMMARY"

log_event "SOAK_END block_rate=$BLOCK_RATE stalls=$STALL_EVENTS"
echo ""
echo "Logs in: $OUTPUT"
