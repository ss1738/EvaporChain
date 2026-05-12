#!/usr/bin/env bash
#
# scripts/dos-flood.sh — DoS Vector 1/2/3 flood harness for T0.7 cluster
# acceptance testing.
#
# Drives controlled tx-submission flood against an EXISTING node's HTTP
# API. Designed for use against the Tier-3 deploy cluster (3 Minis + 2
# Hetzners) once T3.1 is live — does NOT spin up its own nodes.
#
# Modes (matching dos-resistance.md vectors):
#   default        — Vector 1: unique-sender well-formed tx flood
#   --garbage-sigs — Vector 2: malformed-signature flood (pool stays empty)
#   --single-sender — Vector 3: single-sender Sybil isolation (caps at 64)
#
# Usage:
#   scripts/dos-flood.sh --target <host:port> --rate <N/s> --duration <Ns|Nm|Nh>
#                        [--garbage-sigs] [--single-sender] [--out <log>]
#
# Examples:
#   scripts/dos-flood.sh --target 100.119.53.101:8081 --rate 1000  --duration 1h
#   scripts/dos-flood.sh --target 100.119.53.101:8081 --rate 1000  --duration 1h --garbage-sigs
#   scripts/dos-flood.sh --target 100.119.53.101:8081 --rate 1000  --duration 1h --single-sender
#
# Pass criteria — pull these from the target node mid-flood:
#   GET /api/mempool                    → mempool len ≤ MAX_MEMPOOL_SIZE (10_000)
#   GET /api/status                     → block_number monotonically advancing
#   GET /api/network/peers              → no peer score drops below ban threshold
# See docs/runbooks/dos-resistance.md §"Operational acceptance" for the
# full acceptance criteria per vector.

set -euo pipefail

TARGET=""
RATE=0
DURATION=""
MODE="vector1"
OUT="/tmp/dos-flood.$(date -u +%Y%m%dT%H%M%SZ).log"

usage() {
    grep -E '^# ' "$0" | sed 's/^# \{0,1\}//'
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --target)         TARGET="$2"; shift 2 ;;
        --rate)           RATE="$2";   shift 2 ;;
        --duration)       DURATION="$2"; shift 2 ;;
        --garbage-sigs)   MODE="vector2"; shift ;;
        --single-sender)  MODE="vector3"; shift ;;
        --out)            OUT="$2";    shift 2 ;;
        -h|--help)        usage ;;
        *)                echo "Unknown arg: $1"; usage ;;
    esac
done

[ -z "$TARGET" ]   && { echo "--target required";   usage; }
[ "$RATE" -le 0 ]  && { echo "--rate must be > 0";  usage; }
[ -z "$DURATION" ] && { echo "--duration required"; usage; }

# Parse duration suffix (h/m/s) → seconds
case "$DURATION" in
    *h) DURATION_SEC=$(( ${DURATION%h} * 3600 )) ;;
    *m) DURATION_SEC=$(( ${DURATION%m} * 60 )) ;;
    *s) DURATION_SEC="${DURATION%s}" ;;
    *)  DURATION_SEC="$DURATION" ;;
esac

# Compute sleep-per-tx as integer microseconds so we don't drift over
# long runs. Use awk for the floating-point math; bash arithmetic is int-only.
SLEEP_US=$(awk -v r="$RATE" 'BEGIN { printf "%d", 1000000 / r }')

echo "━━━ DoS flood harness ━━━"
echo "  target:     $TARGET"
echo "  mode:       $MODE"
echo "  rate:       $RATE tx/s  (sleep $SLEEP_US µs/tx)"
echo "  duration:   ${DURATION_SEC}s"
echo "  log:        $OUT"
echo

# Probe target is reachable
HTTP=$(curl -s -o /dev/null -w "%{http_code}" --max-time 3 "http://$TARGET/health" 2>/dev/null || echo "000")
if [ "$HTTP" != "200" ]; then
    echo "ERROR: target $TARGET /health returned $HTTP — aborting"
    exit 2
fi

# Capture baseline
BASE_HEIGHT=$(curl -s --max-time 3 "http://$TARGET/api/status" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('block_number', d.get('height',0)))" 2>/dev/null || echo 0)
BASE_MEMPOOL=$(curl -s --max-time 3 "http://$TARGET/api/mempool" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('len',0))" 2>/dev/null || echo 0)
echo "  baseline: block_number=$BASE_HEIGHT mempool_len=$BASE_MEMPOOL"
echo

SINGLE_SENDER_HEX=$(printf '%064x' 42)

submit_tx() {
    local i="$1"
    local sender_hex
    local body
    case "$MODE" in
        vector1) # well-formed, unique sender per request
            sender_hex=$(printf '%064x' "$i")
            body="{\"address\":\"$sender_hex\"}"
            curl -s -o /dev/null -X POST "http://$TARGET/api/faucet" \
                -H "Content-Type: application/json" -d "$body" --max-time 1 2>/dev/null \
                || echo "fail" >> "$OUT"
            ;;
        vector2) # malformed signature: send to /api/tx/transfer with a
                 # garbage signature blob. The verifier path runs only
                 # when verify_signatures=true at the cluster level.
            sender_hex=$(printf '%064x' "$i")
            local recv_hex
            recv_hex=$(printf '%064x' $(( i + 1000000 )))
            body=$(cat <<JSON
{"from":"$sender_hex","to":"$recv_hex","amount":1,"nonce":0,"signature":"00aa","public_key":"00bb"}
JSON
)
            curl -s -o /dev/null -X POST "http://$TARGET/api/tx/transfer" \
                -H "Content-Type: application/json" -d "$body" --max-time 1 2>/dev/null \
                || echo "fail" >> "$OUT"
            ;;
        vector3) # single-sender flood: same address every tx, unique nonce
            local recv_hex
            recv_hex=$(printf '%064x' $(( i + 1000000 )))
            body="{\"from\":\"$SINGLE_SENDER_HEX\",\"to\":\"$recv_hex\",\"amount\":1,\"nonce\":$i}"
            curl -s -o /dev/null -X POST "http://$TARGET/api/tx/transfer" \
                -H "Content-Type: application/json" -d "$body" --max-time 1 2>/dev/null \
                || echo "fail" >> "$OUT"
            ;;
    esac
}

# Run loop. Each tx-submit is fire-and-forget; we only care about the
# acceptance/rejection accounting on the target's side, not per-tx
# response bodies. The target's /api/mempool counter tells us if the
# admission contract held under the rate.
START=$(date +%s)
END=$(( START + DURATION_SEC ))
SUBMITTED=0
while [ "$(date +%s)" -lt "$END" ]; do
    submit_tx "$SUBMITTED" &
    SUBMITTED=$(( SUBMITTED + 1 ))
    # crude rate gate — micro-sleep
    if [ "$SLEEP_US" -gt 0 ]; then
        # `sleep` accepts fractional seconds on macOS+linux
        sleep "$(awk -v u="$SLEEP_US" 'BEGIN { printf "%.6f", u/1000000 }')"
    fi
    # Drain background curls every 1000 to bound process count
    if [ $(( SUBMITTED % 1000 )) -eq 0 ]; then
        wait
        echo "  $(date -u +%H:%M:%SZ) submitted=$SUBMITTED" | tee -a "$OUT"
    fi
done
wait

echo
echo "━━━ flood complete ━━━"
echo "  submitted: $SUBMITTED in ${DURATION_SEC}s"
echo "  observed rate: $(awk -v s="$SUBMITTED" -v d="$DURATION_SEC" 'BEGIN { printf "%.1f", s/d }') tx/s"

# Capture post-flood snapshot
POST_HEIGHT=$(curl -s --max-time 3 "http://$TARGET/api/status" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('block_number', d.get('height',0)))" 2>/dev/null || echo 0)
POST_MEMPOOL=$(curl -s --max-time 3 "http://$TARGET/api/mempool" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('len',0))" 2>/dev/null || echo 0)
echo "  post-flood: block_number=$POST_HEIGHT mempool_len=$POST_MEMPOOL"

# Quick PASS/FAIL gates per vector
case "$MODE" in
    vector1)
        if [ "$POST_MEMPOOL" -le 10000 ]; then
            echo "  PASS: mempool len ($POST_MEMPOOL) ≤ MAX_MEMPOOL_SIZE (10000)"
        else
            echo "  FAIL: mempool len ($POST_MEMPOOL) > MAX_MEMPOOL_SIZE (10000)"
            exit 1
        fi
        ;;
    vector2)
        if [ "$POST_MEMPOOL" -le "$BASE_MEMPOOL" ]; then
            echo "  PASS: mempool didn't grow (verifier rejected garbage sigs)"
        else
            echo "  FAIL: mempool grew under garbage-sig flood (BASE=$BASE_MEMPOOL POST=$POST_MEMPOOL)"
            exit 1
        fi
        ;;
    vector3)
        # We can't introspect single-sender bucket via HTTP; the gate is
        # MAX_TXS_PER_ACCOUNT = 64. Operator runs `/api/mempool/by_sender`
        # or equivalent and asserts the bucket is capped. Surface a hint.
        echo "  CHECK: query /api/mempool by-sender for $SINGLE_SENDER_HEX"
        echo "         expected bucket size ≤ MAX_TXS_PER_ACCOUNT (64)"
        ;;
esac

[ "$POST_HEIGHT" -gt "$BASE_HEIGHT" ] \
    && echo "  PASS: block production continued ($BASE_HEIGHT → $POST_HEIGHT)" \
    || { echo "  FAIL: consensus stalled under load"; exit 1; }

echo "  log: $OUT"
