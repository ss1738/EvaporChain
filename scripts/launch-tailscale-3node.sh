#!/bin/bash
# EvaporChain 3-Mini Tailscale cluster launcher.
#
# One per Mini. Reads --validator-id from $1 (1, 2, or 3), figures out
# bootstrap peers for the other two Minis from the static topology
# below, and runs the production tendermint binary with --genesis-config
# pointing at the freshly-built genesis-tailscale-3node.json.
#
# Usage on each Mini:
#   ./scripts/launch-tailscale-3node.sh 1   # on satyawansingh@100.119.53.101
#   ./scripts/launch-tailscale-3node.sh 2   # on satyawan-mini-1@100.113.253.72
#   ./scripts/launch-tailscale-3node.sh 3   # on satyawan-mini-2@100.103.216.125
#
# Genesis JSON, BLS PoPs, BLS pubkeys, and bootstrap peers are pinned
# to commit a37aabe.

set -e

VID="$1"
if [[ "$VID" != "1" && "$VID" != "2" && "$VID" != "3" ]]; then
    echo "Usage: $0 <validator-id-1-2-or-3>"
    exit 1
fi

# Tailscale IPs of all 3 Minis.
IP_1="100.119.53.101"
IP_2="100.113.253.72"
IP_3="100.103.216.125"

# Pick the two peers other than us as bootstrap addresses.
case "$VID" in
    1) PEER_A="$IP_2"; PEER_B="$IP_3" ;;
    2) PEER_A="$IP_1"; PEER_B="$IP_3" ;;
    3) PEER_A="$IP_1"; PEER_B="$IP_2" ;;
esac

BINARY="./target/release/evaporchain-node"
if [ ! -f "$BINARY" ]; then
    echo "Building release binary..."
    cargo build --release -p evaporchain-node
fi

GENESIS="./genesis-tailscale-3node.json"
if [ ! -f "$GENESIS" ]; then
    echo "FATAL: $GENESIS not found. Run \`git pull\` first."
    exit 1
fi

DATA_DIR="$HOME/.evaporchain-tailscale-data"
if [ ! -f "$DATA_DIR/bls_key.bin" ]; then
    echo "FATAL: $DATA_DIR/bls_key.bin not found."
    echo "  Run keygen + write secret bytes per Phase 0.2."
    exit 1
fi

LOG_FILE="$HOME/evaporchain-v$VID.log"
PID_FILE="$HOME/evaporchain-v$VID.pid"

# Kill any stale process on this port for this validator.
pkill -f "validator-id $VID" 2>/dev/null || true
sleep 1

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║   EvaporChain 3-Mini Tailscale Cluster — Validator $VID         ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "  Bootstrap peers: /ip4/$PEER_A/tcp/9000  /ip4/$PEER_B/tcp/9000"
echo "  Data dir:        $DATA_DIR"
echo "  Genesis:         $GENESIS"
echo "  Log:             $LOG_FILE"
echo ""

nohup "$BINARY" \
    --tendermint --network --api --prove \
    --genesis-config "$GENESIS" \
    --validator-id "$VID" --validators 3 \
    --node-id "node-$VID" \
    --port 9000 --api-port "808$VID" \
    --data-dir "$DATA_DIR" \
    --startup-delay 5000 \
    --interval 2000 \
    --bootstrap "/ip4/$PEER_A/tcp/9000" \
    --bootstrap "/ip4/$PEER_B/tcp/9000" \
    > "$LOG_FILE" 2>&1 &

echo $! > "$PID_FILE"
echo "Started validator $VID (PID $(cat $PID_FILE))"
echo ""
echo "  tail -f $LOG_FILE"
echo "  curl http://localhost:808$VID/api/status | jq"
echo "  kill \$(cat $PID_FILE)   # to stop"
