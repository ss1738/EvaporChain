#!/usr/bin/env bash
#
# EvaporChain Devnet Stress Test
#
# Launches a 4-node devnet, waits for consensus, sends burst transactions,
# measures block production rate and TPS, tests fault tolerance.
#
# Usage: ./scripts/stress-test.sh
#
# Requires: curl, jq or python3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY="$ROOT_DIR/target/release/evaporchain-node"
LOG_DIR="$ROOT_DIR/logs/stress-test"
NUM_NODES=4
API_BASE_PORT=19001
P2P_BASE_PORT=31001
INTERVAL=1000
BURST_SIZE=50
RESULTS_PASS=true

mkdir -p "$LOG_DIR"

# ── Build ──
echo "━━━ Building EvaporChain node (release mode)... ━━━"
cd "$ROOT_DIR"
cargo build -p evaporchain-node --release 2>&1 | tail -1

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    exit 1
fi

# ── Cleanup ──
PIDS=()
cleanup() {
    echo ""
    echo "━━━ Shutting down stress test nodes... ━━━"
    for pid in "${PIDS[@]+"${PIDS[@]}"}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null
    echo "All nodes stopped."
}
trap cleanup EXIT INT TERM

# ── Launch 4 nodes ──
echo ""
echo "━━━ Launching 4-node devnet ━━━"

for i in $(seq 1 $NUM_NODES); do
    API_PORT=$((API_BASE_PORT + i - 1))
    P2P_PORT=$((P2P_BASE_PORT + i - 1))
    NODE_ID="stress-node-${i}"

    EXTRA_ARGS=""
    if [ "$i" -eq 1 ]; then
        EXTRA_ARGS="--demo"
    else
        EXTRA_ARGS="--bootstrap /ip4/127.0.0.1/tcp/${P2P_BASE_PORT}"
    fi

    "$BINARY" \
        --node-id "$NODE_ID" \
        --port "$P2P_PORT" \
        --api --api-port "$API_PORT" \
        --network \
        --interval "$INTERVAL" \
        --startup-delay 3000 \
        --validator-id "$i" \
        --validators "$NUM_NODES" \
        --stake 1000 \
        $EXTRA_ARGS \
        > "$LOG_DIR/${NODE_ID}.log" 2>&1 &
    PIDS+=($!)
    echo "  Started $NODE_ID (PID $!) — API :${API_PORT}, P2P :${P2P_PORT}"
done

# ── Wait for nodes to be ready ──
echo ""
echo "━━━ Waiting for nodes to come online... ━━━"
MAX_WAIT=30
for i in $(seq 1 $NUM_NODES); do
    PORT=$((API_BASE_PORT + i - 1))
    for attempt in $(seq 1 $MAX_WAIT); do
        HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:${PORT}/health" 2>/dev/null || echo "000")
        if [ "$HTTP_CODE" = "200" ]; then
            echo "  node-${i}: online (attempt ${attempt})"
            break
        fi
        if [ "$attempt" -eq "$MAX_WAIT" ]; then
            echo "  node-${i}: FAILED to start after ${MAX_WAIT}s"
            RESULTS_PASS=false
        fi
        sleep 1
    done
done

# ── Phase 1: Block Production ──
echo ""
echo "━━━ Phase 1: Block Production Check ━━━"
sleep 5  # Let consensus produce some blocks

HEIGHT_1=$(curl -s "http://127.0.0.1:${API_BASE_PORT}/api/status" 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('block_number', d.get('height', 0)))" 2>/dev/null || echo "0")
echo "  Current height: ${HEIGHT_1}"
sleep 5
HEIGHT_2=$(curl -s "http://127.0.0.1:${API_BASE_PORT}/api/status" 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('block_number', d.get('height', 0)))" 2>/dev/null || echo "0")
echo "  Height after 5s: ${HEIGHT_2}"

BLOCKS_PRODUCED=$((HEIGHT_2 - HEIGHT_1))
echo "  Blocks in 5s: ${BLOCKS_PRODUCED}"
BLOCK_RATE=$(echo "scale=1; $BLOCKS_PRODUCED / 5" | bc 2>/dev/null || echo "?")
echo "  Block rate: ~${BLOCK_RATE} blocks/sec"

if [ "$BLOCKS_PRODUCED" -gt 0 ]; then
    echo "  PASS: Blocks being produced"
else
    echo "  FAIL: No blocks produced"
    RESULTS_PASS=false
fi

# ── Phase 2: Transaction Burst ──
echo ""
echo "━━━ Phase 2: Transaction Burst (${BURST_SIZE} txs) ━━━"

HEIGHT_PRE=$(curl -s "http://127.0.0.1:${API_BASE_PORT}/api/status" 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('block_number', d.get('height', 0)))" 2>/dev/null || echo "0")

START_TIME=$(date +%s)
TX_OK=0
TX_FAIL=0
for t in $(seq 1 $BURST_SIZE); do
    # Submit faucet request (simple transaction)
    RESULT=$(curl -s -X POST "http://127.0.0.1:${API_BASE_PORT}/api/faucet" \
        -H "Content-Type: application/json" \
        -d "{\"address\":\"$(printf '%064x' $t)\"}" 2>/dev/null || echo "error")
    if echo "$RESULT" | grep -qi "error\|fail"; then
        TX_FAIL=$((TX_FAIL + 1))
    else
        TX_OK=$((TX_OK + 1))
    fi
done
END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))
ELAPSED=$((ELAPSED > 0 ? ELAPSED : 1))

echo "  Sent: ${TX_OK} OK, ${TX_FAIL} failed (in ${ELAPSED}s)"
echo "  Submit rate: ~$((TX_OK / ELAPSED)) tx/s"

# Wait for transactions to be included in blocks
sleep 10
HEIGHT_POST=$(curl -s "http://127.0.0.1:${API_BASE_PORT}/api/status" 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('block_number', d.get('height', 0)))" 2>/dev/null || echo "0")
BLOCKS_WITH_TXS=$((HEIGHT_POST - HEIGHT_PRE))
echo "  Blocks during burst: ${BLOCKS_WITH_TXS}"

if [ "$TX_OK" -gt 0 ]; then
    echo "  PASS: Transactions accepted"
else
    echo "  FAIL: No transactions accepted"
    RESULTS_PASS=false
fi

# ── Phase 3: Chain Consistency ──
echo ""
echo "━━━ Phase 3: Chain Consistency ━━━"
"$SCRIPT_DIR/check-consensus.sh" "$API_BASE_PORT" || RESULTS_PASS=false

# ── Phase 4: Fault Tolerance ──
echo ""
echo "━━━ Phase 4: Fault Tolerance (kill 1 of 4 nodes) ━━━"

KILL_IDX=3  # Kill node 3
KILL_PID="${PIDS[$((KILL_IDX - 1))]}"
echo "  Killing node-${KILL_IDX} (PID ${KILL_PID})..."
kill "$KILL_PID" 2>/dev/null || true

sleep 3

# Check remaining nodes still produce blocks
HEIGHT_BEFORE=$(curl -s "http://127.0.0.1:${API_BASE_PORT}/api/status" 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('block_number', d.get('height', 0)))" 2>/dev/null || echo "0")
sleep 5
HEIGHT_AFTER=$(curl -s "http://127.0.0.1:${API_BASE_PORT}/api/status" 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('block_number', d.get('height', 0)))" 2>/dev/null || echo "0")

FAULT_BLOCKS=$((HEIGHT_AFTER - HEIGHT_BEFORE))
echo "  Blocks with 3/4 nodes (5s): ${FAULT_BLOCKS}"

if [ "$FAULT_BLOCKS" -gt 0 ]; then
    echo "  PASS: Chain continues with 3/4 validators (quorum maintained)"
else
    echo "  FAIL: Chain halted after losing 1 validator"
    RESULTS_PASS=false
fi

# ── Phase 5: Node Recovery ──
echo ""
echo "━━━ Phase 5: Node Recovery ━━━"

RECOVER_PORT=$((API_BASE_PORT + KILL_IDX - 1))
P2P_PORT=$((P2P_BASE_PORT + KILL_IDX - 1))
echo "  Restarting node-${KILL_IDX}..."
"$BINARY" \
    --node-id "stress-node-${KILL_IDX}" \
    --port "$P2P_PORT" \
    --api --api-port "$RECOVER_PORT" \
    --network \
    --interval "$INTERVAL" \
    --startup-delay 3000 \
    --validator-id "$KILL_IDX" \
    --validators "$NUM_NODES" \
    --stake 1000 \
    --bootstrap "/ip4/127.0.0.1/tcp/${P2P_BASE_PORT}" \
    > "$LOG_DIR/stress-node-${KILL_IDX}-recovered.log" 2>&1 &
PIDS[$((KILL_IDX - 1))]=$!
echo "  Restarted node-${KILL_IDX} (PID $!)"

# Wait for recovery
sleep 10

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:${RECOVER_PORT}/health" 2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "200" ]; then
    echo "  PASS: Recovered node is back online"
else
    echo "  FAIL: Recovered node did not come back online"
    RESULTS_PASS=false
fi

# ── Final consistency check ──
echo ""
echo "━━━ Final Consistency Check ━━━"
"$SCRIPT_DIR/check-consensus.sh" "$API_BASE_PORT" || RESULTS_PASS=false

# ── Summary ──
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if $RESULTS_PASS; then
    echo "  STRESS TEST: ALL PHASES PASSED"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    exit 0
else
    echo "  STRESS TEST: SOME PHASES FAILED"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    exit 1
fi
