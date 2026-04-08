#!/usr/bin/env bash
#
# EvaporChain Multi-Node Devnet Launcher (Tendermint BFT)
#
# Starts 4 validator nodes on the local machine:
#   node-1 (port 9001) — Validator 1 + Demo transactions
#   node-2 (port 9002) — Validator 2
#   node-3 (port 9003) — Validator 3
#   node-4 (port 9004) — Validator 4
#
# Consensus: Tendermint BFT (default) with 2/3 stake finality.
# Nodes connect via bootstrap peers (node-1 address passed to others).
# mDNS is also enabled for additional discovery.
# Press Ctrl+C to stop all nodes.
#
# Usage:
#   ./scripts/launch-devnet.sh              # interleaved output
#   ./scripts/launch-devnet.sh --split      # separate log files per node
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY="$ROOT_DIR/target/release/evaporchain-node"
INTERVAL="${EVAPORCHAIN_INTERVAL:-2000}"
PRODUCER_PORT=9001

# ── Build ──
echo "━━━ Building EvaporChain node (release mode)... ━━━"
cd "$ROOT_DIR"
cargo build -p evaporchain-node --release 2>&1 | tail -1
echo ""

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    exit 1
fi

# ── Cleanup on exit ──
PIDS=()
cleanup() {
    echo ""
    echo "━━━ Shutting down devnet... ━━━"
    for pid in "${PIDS[@]+"${PIDS[@]}"}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null
    echo "All nodes stopped."
}
trap cleanup EXIT INT TERM

# ── Launch mode ──
SPLIT_MODE=false
LOG_DIR=""
if [[ "${1:-}" == "--split" ]]; then
    SPLIT_MODE=true
    LOG_DIR="$ROOT_DIR/logs"
    mkdir -p "$LOG_DIR"
    echo "Log files will be in $LOG_DIR/"
fi

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║      EvaporChain Tendermint BFT Devnet (4 validators)        ║"
echo "║                                                             ║"
echo "║  node-1 (${PRODUCER_PORT}) — Validator 1 + Demo                         ║"
echo "║  node-2 (9002) — Validator 2                                ║"
echo "║  node-3 (9003) — Validator 3                                ║"
echo "║  node-4 (9004) — Validator 4                                ║"
echo "║                                                             ║"
echo "║  Consensus: Tendermint BFT | Stake: ${VALIDATOR_STAKE} per validator    ║"
echo "║  Press Ctrl+C to stop all nodes                             ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# ── Consensus configuration ──
NUM_VALIDATORS=4
VALIDATOR_STAKE=1000

# ── Start node-1 (validator 1) ──
echo "  Starting validator node-1..."
if $SPLIT_MODE; then
    "$BINARY" \
        --node-id "node-1" \
        --port "$PRODUCER_PORT" \
        --network \
        --interval "$INTERVAL" \
        --startup-delay 3000 \
        --validator-id 1 \
        --validators "$NUM_VALIDATORS" \
        --stake "$VALIDATOR_STAKE" \
        --demo \
        > "$LOG_DIR/node-1.log" 2>&1 &
else
    "$BINARY" \
        --node-id "node-1" \
        --port "$PRODUCER_PORT" \
        --network \
        --interval "$INTERVAL" \
        --startup-delay 3000 \
        --validator-id 1 \
        --validators "$NUM_VALIDATORS" \
        --stake "$VALIDATOR_STAKE" \
        --demo &
fi
PIDS+=($!)
echo "  Started node-1 (PID $!) on port ${PRODUCER_PORT} — validator 1 of $NUM_VALIDATORS"

# Give the first node a moment to bind its port
sleep 1

# ── Start validators 2-4 with bootstrap peer ──
BOOTSTRAP="/ip4/127.0.0.1/tcp/${PRODUCER_PORT}"

for i in 2 3 4; do
    PORT=$((9000 + i))
    NODE_ID="node-${i}"

    if $SPLIT_MODE; then
        "$BINARY" \
            --node-id "$NODE_ID" \
            --port "$PORT" \
            --network \
            --interval "$INTERVAL" \
            --startup-delay 3000 \
            --validator-id "$i" \
            --validators "$NUM_VALIDATORS" \
            --stake "$VALIDATOR_STAKE" \
            --bootstrap "$BOOTSTRAP" \
            > "$LOG_DIR/${NODE_ID}.log" 2>&1 &
    else
        "$BINARY" \
            --node-id "$NODE_ID" \
            --port "$PORT" \
            --network \
            --interval "$INTERVAL" \
            --startup-delay 3000 \
            --validator-id "$i" \
            --validators "$NUM_VALIDATORS" \
            --stake "$VALIDATOR_STAKE" \
            --bootstrap "$BOOTSTRAP" &
    fi
    PIDS+=($!)
    echo "  Started $NODE_ID (PID $!) on port $PORT — validator $i of $NUM_VALIDATORS"
done

echo ""
echo "━━━ All 4 nodes launched. Waiting for peer connections (~3s)... ━━━"
echo ""

if $SPLIT_MODE; then
    echo "Tailing all log files (Ctrl+C to stop):"
    echo ""
    tail -f "$LOG_DIR"/node-*.log
else
    # Wait for all background processes
    wait
fi
