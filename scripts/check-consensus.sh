#!/usr/bin/env bash
#
# EvaporChain Consensus Health Check
#
# Queries all 4 devnet nodes and compares their latest block height
# and state to detect forks or inconsistencies.
#
# Usage: ./scripts/check-consensus.sh [base_port]
#   base_port defaults to 19001 (API ports for devnet nodes)

set -euo pipefail

BASE_PORT="${1:-19001}"
NUM_NODES=4
PASS=true

echo "━━━ EvaporChain Consensus Check ━━━"
echo ""

declare -a HEIGHTS
declare -a STATE_ROOTS
declare -a STATUSES

for i in $(seq 1 $NUM_NODES); do
    PORT=$((BASE_PORT + i - 1))
    URL="http://127.0.0.1:${PORT}"

    # Check health
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "${URL}/health" 2>/dev/null || echo "000")
    if [ "$HTTP_CODE" != "200" ]; then
        echo "  node-${i} (port ${PORT}): OFFLINE (HTTP ${HTTP_CODE})"
        STATUSES+=("offline")
        HEIGHTS+=("0")
        STATE_ROOTS+=("none")
        PASS=false
        continue
    fi

    # Get status
    STATUS=$(curl -s "${URL}/api/status" 2>/dev/null || echo "{}")
    HEIGHT=$(echo "$STATUS" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('block_number', d.get('height', 0)))" 2>/dev/null || echo "0")
    STATE_ROOT=$(echo "$STATUS" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('state_root', 'unknown'))" 2>/dev/null || echo "unknown")

    echo "  node-${i} (port ${PORT}): height=${HEIGHT}  state_root=${STATE_ROOT:0:16}..."
    STATUSES+=("online")
    HEIGHTS+=("$HEIGHT")
    STATE_ROOTS+=("$STATE_ROOT")
done

echo ""

# Check consistency
ONLINE_COUNT=0
MAX_HEIGHT=0
for i in $(seq 0 $((NUM_NODES - 1))); do
    if [ "${STATUSES[$i]}" = "online" ]; then
        ONLINE_COUNT=$((ONLINE_COUNT + 1))
        if [ "${HEIGHTS[$i]}" -gt "$MAX_HEIGHT" ] 2>/dev/null; then
            MAX_HEIGHT="${HEIGHTS[$i]}"
        fi
    fi
done

echo "Online nodes: ${ONLINE_COUNT}/${NUM_NODES}"
echo "Max height: ${MAX_HEIGHT}"

# Check for height divergence (allow 2 blocks tolerance)
HEIGHT_SPREAD=0
for i in $(seq 0 $((NUM_NODES - 1))); do
    if [ "${STATUSES[$i]}" = "online" ]; then
        DIFF=$((MAX_HEIGHT - HEIGHTS[$i]))
        if [ "$DIFF" -gt "$HEIGHT_SPREAD" ]; then
            HEIGHT_SPREAD=$DIFF
        fi
    fi
done

if [ "$HEIGHT_SPREAD" -gt 2 ]; then
    echo "WARNING: Height spread is ${HEIGHT_SPREAD} blocks (tolerance: 2)"
    PASS=false
else
    echo "Height spread: ${HEIGHT_SPREAD} blocks (OK)"
fi

# Check for state root divergence among nodes at the same height
# (simplified: just check if all online nodes have same state root)
UNIQUE_ROOTS=$(printf '%s\n' "${STATE_ROOTS[@]}" | grep -v "^none$" | sort -u | wc -l | tr -d ' ')
if [ "$UNIQUE_ROOTS" -gt 1 ]; then
    echo "WARNING: State root divergence detected — possible fork!"
    PASS=false
else
    echo "State roots: consistent (OK)"
fi

echo ""
if $PASS; then
    echo "✓ Consensus check PASSED"
    exit 0
else
    echo "✗ Consensus check FAILED"
    exit 1
fi
