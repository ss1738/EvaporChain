#!/usr/bin/env bash
#
# EvaporChain Multi-Node Devnet Launcher
#
# Starts 4 nodes on the local machine:
#   node-1 (port 9001) — Producer with --demo (generates transactions)
#   node-2 (port 9002) — Follower (syncs blocks from peers)
#   node-3 (port 9003) — Follower
#   node-4 (port 9004) — Follower
#
# All nodes auto-discover each other via mDNS.
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
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null
    echo "All nodes stopped."
}
trap cleanup EXIT INT TERM

# ── Launch mode ──
SPLIT_MODE=false
if [[ "${1:-}" == "--split" ]]; then
    SPLIT_MODE=true
    LOG_DIR="$ROOT_DIR/logs"
    mkdir -p "$LOG_DIR"
    echo "Log files will be in $LOG_DIR/"
fi

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║         EvaporChain Multi-Node Devnet (4 nodes)             ║"
echo "║                                                             ║"
echo "║  node-1 (9001) — Producer + Demo                            ║"
echo "║  node-2 (9002) — Follower                                   ║"
echo "║  node-3 (9003) — Follower                                   ║"
echo "║  node-4 (9004) — Follower                                   ║"
echo "║                                                             ║"
echo "║  Block interval: ${INTERVAL}ms | mDNS auto-discovery              ║"
echo "║  Press Ctrl+C to stop all nodes                             ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# ── Start nodes ──

start_node() {
    local node_id="$1"
    local port="$2"
    shift 2
    local extra_args=("$@")

    if $SPLIT_MODE; then
        "$BINARY" \
            --node-id "$node_id" \
            --port "$port" \
            --network \
            --interval "$INTERVAL" \
            "${extra_args[@]}" \
            > "$LOG_DIR/${node_id}.log" 2>&1 &
    else
        "$BINARY" \
            --node-id "$node_id" \
            --port "$port" \
            --network \
            --interval "$INTERVAL" \
            "${extra_args[@]}" &
    fi
    PIDS+=($!)
    echo "  Started $node_id (PID $!) on port $port ${extra_args[*]:-}"
}

# Node 1: Producer with demo mode
start_node "node-1" 9001 --demo

# Small delay so node-1 starts listening before followers try to discover
sleep 1

# Nodes 2-4: Followers
start_node "node-2" 9002
start_node "node-3" 9003
start_node "node-4" 9004

echo ""
echo "━━━ All 4 nodes launched. Waiting for mDNS discovery... ━━━"
echo ""

if $SPLIT_MODE; then
    echo "Tailing all log files (Ctrl+C to stop):"
    echo ""
    tail -f "$LOG_DIR"/node-*.log
else
    # Wait for all background processes
    wait
fi
