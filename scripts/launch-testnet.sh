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

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║      EvaporChain Testnet — 4 Validator Tendermint BFT       ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# Validator 1 (API on port 8080, P2P on port 9000)
echo "Starting Validator 1 (API :8080, P2P :9000)..."
$BINARY \
    --tendermint --network --api $DEMO_FLAG \
    --validator-id 1 --validators 4 \
    --node-id node-1 \
    --port 9000 --api-port 8080 \
    --data-dir /tmp/evaporchain-v1 \
    --startup-delay 3000 \
    --bootstrap /ip4/127.0.0.1/tcp/9001 \
    --bootstrap /ip4/127.0.0.1/tcp/9002 \
    --bootstrap /ip4/127.0.0.1/tcp/9003 \
    > /tmp/evaporchain-v1.log 2>&1 &
V1_PID=$!
echo "  PID=$V1_PID"

sleep 1

# Validator 2 (API on port 8081, P2P on port 9001)
echo "Starting Validator 2 (API :8081, P2P :9001)..."
$BINARY \
    --tendermint --network --api $DEMO_FLAG \
    --validator-id 2 --validators 4 \
    --node-id node-2 \
    --port 9001 --api-port 8081 \
    --data-dir /tmp/evaporchain-v2 \
    --startup-delay 3000 \
    --bootstrap /ip4/127.0.0.1/tcp/9000 \
    --bootstrap /ip4/127.0.0.1/tcp/9002 \
    --bootstrap /ip4/127.0.0.1/tcp/9003 \
    > /tmp/evaporchain-v2.log 2>&1 &
V2_PID=$!
echo "  PID=$V2_PID"

sleep 1

# Validator 3 (API on port 8082, P2P on port 9002)
echo "Starting Validator 3 (API :8082, P2P :9002)..."
$BINARY \
    --tendermint --network --api $DEMO_FLAG \
    --validator-id 3 --validators 4 \
    --node-id node-3 \
    --port 9002 --api-port 8082 \
    --data-dir /tmp/evaporchain-v3 \
    --startup-delay 3000 \
    --bootstrap /ip4/127.0.0.1/tcp/9000 \
    --bootstrap /ip4/127.0.0.1/tcp/9001 \
    --bootstrap /ip4/127.0.0.1/tcp/9003 \
    > /tmp/evaporchain-v3.log 2>&1 &
V3_PID=$!
echo "  PID=$V3_PID"

sleep 1

# Validator 4 (API on port 8083, P2P on port 9003)
echo "Starting Validator 4 (API :8083, P2P :9003)..."
$BINARY \
    --tendermint --network --api $DEMO_FLAG \
    --validator-id 4 --validators 4 \
    --node-id node-4 \
    --port 9003 --api-port 8083 \
    --data-dir /tmp/evaporchain-v4 \
    --startup-delay 3000 \
    --bootstrap /ip4/127.0.0.1/tcp/9000 \
    --bootstrap /ip4/127.0.0.1/tcp/9001 \
    --bootstrap /ip4/127.0.0.1/tcp/9002 \
    > /tmp/evaporchain-v4.log 2>&1 &
V4_PID=$!
echo "  PID=$V4_PID"

echo ""
echo "All 4 validators started!"
echo ""
echo "Dashboards:"
echo "  Validator 1: http://localhost:8080"
echo "  Validator 2: http://localhost:8081"
echo "  Validator 3: http://localhost:8082"
echo "  Validator 4: http://localhost:8083"
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
