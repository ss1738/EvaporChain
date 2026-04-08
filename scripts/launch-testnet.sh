#!/bin/bash
# EvaporChain Multi-Node Testnet Launcher
# Launches 4 validators with Tendermint BFT consensus on localhost
#
# Usage: ./scripts/launch-testnet.sh [--demo]

set -e

DEMO_FLAG=""
if [[ "$1" == "--demo" ]]; then
    DEMO_FLAG="--demo"
    echo "🎭 Demo mode: auto-generating transactions"
fi

BINARY="./target/release/evaporchain-node"
if [ ! -f "$BINARY" ]; then
    echo "Building release binary..."
    cargo build --release -p evaporchain-node
fi

# Clean previous data
rm -rf /tmp/evaporchain-v1 /tmp/evaporchain-v2 /tmp/evaporchain-v3 /tmp/evaporchain-v4

# Kill any existing nodes
pkill -f evaporchain-node 2>/dev/null || true
sleep 1

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║      EvaporChain Testnet — 4 Validator Tendermint BFT       ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# P2P ports 30001-30004 (avoiding Docker conflicts on 9000-9001)
# API ports 18001-18004 (avoiding common services on 8080-8083)

echo "Starting Validator 1 (API :18001, P2P :30001)..."
$BINARY \
    --network --api $DEMO_FLAG \
    --validator-id 1 --validators 4 \
    --node-id node-1 \
    --port 30001 --api-port 18001 \
    --data-dir /tmp/evaporchain-v1 \
    --startup-delay 8000 \
    --interval 3000 \
    > /tmp/evaporchain-v1.log 2>&1 &
V1_PID=$!
echo "  PID=$V1_PID"

sleep 2

echo "Starting Validator 2 (API :18002, P2P :30002)..."
$BINARY \
    --network --api \
    --validator-id 2 --validators 4 \
    --node-id node-2 \
    --port 30002 --api-port 18002 \
    --data-dir /tmp/evaporchain-v2 \
    --startup-delay 8000 \
    --interval 3000 \
    --bootstrap /ip4/127.0.0.1/tcp/30001 \
    > /tmp/evaporchain-v2.log 2>&1 &
V2_PID=$!
echo "  PID=$V2_PID"

sleep 2

echo "Starting Validator 3 (API :18003, P2P :30003)..."
$BINARY \
    --network --api \
    --validator-id 3 --validators 4 \
    --node-id node-3 \
    --port 30003 --api-port 18003 \
    --data-dir /tmp/evaporchain-v3 \
    --startup-delay 8000 \
    --interval 3000 \
    --bootstrap /ip4/127.0.0.1/tcp/30001 \
    > /tmp/evaporchain-v3.log 2>&1 &
V3_PID=$!
echo "  PID=$V3_PID"

sleep 2

echo "Starting Validator 4 (API :18004, P2P :30004)..."
$BINARY \
    --network --api \
    --validator-id 4 --validators 4 \
    --node-id node-4 \
    --port 30004 --api-port 18004 \
    --data-dir /tmp/evaporchain-v4 \
    --startup-delay 8000 \
    --interval 3000 \
    --bootstrap /ip4/127.0.0.1/tcp/30001 \
    > /tmp/evaporchain-v4.log 2>&1 &
V4_PID=$!
echo "  PID=$V4_PID"

echo ""
echo "All 4 validators started!"
echo ""
echo "Dashboards:"
echo "  Validator 1: http://localhost:18001"
echo "  Validator 2: http://localhost:18002"
echo "  Validator 3: http://localhost:18003"
echo "  Validator 4: http://localhost:18004"
echo ""
echo "Logs:"
echo "  tail -f /tmp/evaporchain-v1.log"
echo "  tail -f /tmp/evaporchain-v2.log"
echo "  tail -f /tmp/evaporchain-v3.log"
echo "  tail -f /tmp/evaporchain-v4.log"
echo ""
echo "Stop all: kill $V1_PID $V2_PID $V3_PID $V4_PID"
echo ""

# Wait for any to exit
wait -n $V1_PID $V2_PID $V3_PID $V4_PID 2>/dev/null
echo "A validator exited. Stopping all..."
kill $V1_PID $V2_PID $V3_PID $V4_PID 2>/dev/null
