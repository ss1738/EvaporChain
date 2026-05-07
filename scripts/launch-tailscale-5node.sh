#!/bin/bash
# EvaporChain 5-Mini+Hetzner Tailscale cluster launcher.
#
# 5-node BFT cluster: 3 UK Mac Minis + 2 Hetzner CX23 in Helsinki, all
# joined via Tailscale. Each validator bootstraps to ALL 4 other peers
# (not just one neighbor) so the libp2p gossip-mesh forms a full graph.
# That avoids the 2026-05-06 fork-at-h~200 bug, which was caused by an
# ad-hoc launch passing only one --bootstrap per node — Macs never
# discovered each other, only Hetzner.
#
# Usage on each node:
#   ./scripts/launch-tailscale-5node.sh 1   # satyawansingh@100.119.53.101 (Mini-1)
#   ./scripts/launch-tailscale-5node.sh 2   # satyawan-mini-1@100.113.253.72 (Mini-2)
#   ./scripts/launch-tailscale-5node.sh 3   # satyawan-mini-2@100.103.216.125 (Mini-3)
#   ./scripts/launch-tailscale-5node.sh 4   # root@evaporchain-hel-1 (Hetzner-1, 100.66.208.20)
#   ./scripts/launch-tailscale-5node.sh 5   # root@evaporchain-hel-2 (Hetzner-2, 100.91.235.22)

set -e

VID="$1"
if [[ "$VID" != "1" && "$VID" != "2" && "$VID" != "3" && "$VID" != "4" && "$VID" != "5" ]]; then
    echo "Usage: $0 <validator-id-1-through-5>"
    exit 1
fi

# Tailscale IPs of all 5 nodes.
IP_1="100.119.53.101"
IP_2="100.113.253.72"
IP_3="100.103.216.125"
IP_4="100.66.208.20"
IP_5="100.91.235.22"

# Build bootstrap list = all 4 OTHER peers.
ALL_IPS=("$IP_1" "$IP_2" "$IP_3" "$IP_4" "$IP_5")
BOOTSTRAP_ARGS=()
for I in 1 2 3 4 5; do
    if [ "$I" != "$VID" ]; then
        BOOTSTRAP_ARGS+=("--bootstrap" "/ip4/${ALL_IPS[$((I-1))]}/tcp/9000")
    fi
done

BINARY="./target/release/evaporchain-node"
if [ ! -f "$BINARY" ]; then
    echo "FATAL: $BINARY not found. Build with: cargo build --release -p evaporchain-node"
    exit 1
fi

GENESIS="./genesis-tailscale-5node.json"
if [ ! -f "$GENESIS" ]; then
    echo "FATAL: $GENESIS not found in $(pwd)"
    exit 1
fi

DATA_DIR="$HOME/.evaporchain-tailscale-5node-data"
if [ ! -f "$DATA_DIR/bls_key.bin" ]; then
    echo "FATAL: $DATA_DIR/bls_key.bin not found."
    echo "  Convert from validator-$VID-keys.json bundle:"
    echo '    python3 -c "import json; d=json.load(open(\"validator-'"$VID"'-keys.json\")); open(\"'"$DATA_DIR"'/bls_key.bin\",\"wb\").write(bytes.fromhex(d[\"bls\"][\"secret_key\"]))"'
    exit 1
fi

LOG_FILE="$HOME/evaporchain-5node-v$VID.log"
PID_FILE="$HOME/evaporchain-5node-v$VID.pid"

# Kill any stale process for this validator before relaunch.
if [ -f "$PID_FILE" ]; then
    OLD_PID=$(cat "$PID_FILE")
    if ps -p "$OLD_PID" > /dev/null 2>&1; then
        echo "Stopping prior validator $VID (PID $OLD_PID)..."
        kill "$OLD_PID" 2>/dev/null || true
        for i in 1 2 3 4 5; do
            ps -p "$OLD_PID" > /dev/null 2>&1 || break
            sleep 1
        done
        ps -p "$OLD_PID" > /dev/null 2>&1 && kill -9 "$OLD_PID" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
fi

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║   EvaporChain 5-Node Tailscale Cluster — Validator $VID         ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "  Bootstrap peers (all 4 others):"
for ARG in "${BOOTSTRAP_ARGS[@]}"; do
    [[ "$ARG" == /ip4/* ]] && echo "    $ARG"
done
echo "  Data dir:   $DATA_DIR"
echo "  Genesis:    $GENESIS"
echo "  Log:        $LOG_FILE"
echo ""

nohup "$BINARY" \
    --tendermint --network --api --mock-prove \
    --genesis-config "$GENESIS" \
    --validator-id "$VID" --validators 5 \
    --node-id "node-$VID" \
    --port 9000 --api-port 8081 \
    --data-dir "$DATA_DIR" --interval 8000 \
    "${BOOTSTRAP_ARGS[@]}" \
    > "$LOG_FILE" 2>&1 &

NEW_PID=$!
echo "$NEW_PID" > "$PID_FILE"
echo "Started validator $VID — PID $NEW_PID. Tail log with: tail -f $LOG_FILE"
