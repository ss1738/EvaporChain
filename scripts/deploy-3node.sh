#!/usr/bin/env bash
# EvaporChain 3-Node Tendermint BFT Deployment
# Nodes: Satyawan (100.119.53.101), Apsarth (100.113.253.72), Ironman (100.103.216.125)
#
# Usage:
#   ./scripts/deploy-3node.sh build    # Build release binary on all 3 Minis
#   ./scripts/deploy-3node.sh start    # Start all 3 nodes
#   ./scripts/deploy-3node.sh stop     # Stop all 3 nodes
#   ./scripts/deploy-3node.sh status   # Check if nodes are running
#   ./scripts/deploy-3node.sh logs     # Tail logs from all 3 nodes

set -euo pipefail

SATYAWAN_HOST="satyawan"
APSARTH_HOST="apsarth"
IRONMAN_HOST="ironman"

SATYAWAN_IP="100.119.53.101"
APSARTH_IP="100.113.253.72"
IRONMAN_IP="100.103.216.125"

SATYAWAN_DIR="/Users/satyawansingh/EvaporChain"
APSARTH_DIR="/Users/satyawan-mini-1/EvaporChain"
IRONMAN_DIR="/Users/satyawan-mini-2/EvaporChain"

P2P_PORT=9000
API_BASE_PORT=8080

BINARY="target/release/evaporchain-node"

case "${1:-help}" in
  build)
    echo "=== Building release binary on all 3 Minis (parallel) ==="
    ssh "$SATYAWAN_HOST" "cargo build --release --manifest-path $SATYAWAN_DIR/Cargo.toml -p evaporchain-node 2>&1 | tail -5" &
    PID1=$!
    ssh "$APSARTH_HOST" "cargo build --release --manifest-path $APSARTH_DIR/Cargo.toml -p evaporchain-node 2>&1 | tail -5" &
    PID2=$!
    ssh "$IRONMAN_HOST" "cargo build --release --manifest-path $IRONMAN_DIR/Cargo.toml -p evaporchain-node 2>&1 | tail -5" &
    PID3=$!

    echo "Waiting for Satyawan build..."
    wait $PID1 && echo "✓ Satyawan built" || echo "✗ Satyawan FAILED"
    echo "Waiting for Apsarth build..."
    wait $PID2 && echo "✓ Apsarth built" || echo "✗ Apsarth FAILED"
    echo "Waiting for Ironman build..."
    wait $PID3 && echo "✓ Ironman built" || echo "✗ Ironman FAILED"
    echo "=== Build complete ==="
    ;;

  start)
    echo "=== Starting 3-node EvaporChain cluster ==="

    # Node 0: Satyawan (validator-id=0, proposer for round 0)
    echo "Starting Satyawan (validator 0)..."
    ssh "$SATYAWAN_HOST" "mkdir -p $SATYAWAN_DIR/data-node0 && \
      nohup $SATYAWAN_DIR/$BINARY \
        --network --api --demo \
        --validator-id 0 --validators 3 --stake 1000 \
        --port $P2P_PORT --api-port $API_BASE_PORT \
        --node-id node-0 \
        --data-dir $SATYAWAN_DIR/data-node0 \
        --bootstrap /ip4/$APSARTH_IP/tcp/$P2P_PORT \
        --bootstrap /ip4/$IRONMAN_IP/tcp/$P2P_PORT \
        > $SATYAWAN_DIR/data-node0/node.log 2>&1 &
      echo \$! > $SATYAWAN_DIR/data-node0/node.pid
      echo 'PID:' \$(cat $SATYAWAN_DIR/data-node0/node.pid)"

    sleep 2

    # Node 1: Apsarth (validator-id=1)
    echo "Starting Apsarth (validator 1)..."
    ssh "$APSARTH_HOST" "mkdir -p $APSARTH_DIR/data-node1 && \
      nohup $APSARTH_DIR/$BINARY \
        --network --api --demo \
        --validator-id 1 --validators 3 --stake 1000 \
        --port $P2P_PORT --api-port $API_BASE_PORT \
        --node-id node-1 \
        --data-dir $APSARTH_DIR/data-node1 \
        --bootstrap /ip4/$SATYAWAN_IP/tcp/$P2P_PORT \
        --bootstrap /ip4/$IRONMAN_IP/tcp/$P2P_PORT \
        > $APSARTH_DIR/data-node1/node.log 2>&1 &
      echo \$! > $APSARTH_DIR/data-node1/node.pid
      echo 'PID:' \$(cat $APSARTH_DIR/data-node1/node.pid)"

    sleep 2

    # Node 2: Ironman (validator-id=2)
    echo "Starting Ironman (validator 2)..."
    ssh "$IRONMAN_HOST" "mkdir -p $IRONMAN_DIR/data-node2 && \
      nohup $IRONMAN_DIR/$BINARY \
        --network --api --demo \
        --validator-id 2 --validators 3 --stake 1000 \
        --port $P2P_PORT --api-port $API_BASE_PORT \
        --node-id node-2 \
        --data-dir $IRONMAN_DIR/data-node2 \
        --bootstrap /ip4/$SATYAWAN_IP/tcp/$P2P_PORT \
        --bootstrap /ip4/$APSARTH_IP/tcp/$P2P_PORT \
        > $IRONMAN_DIR/data-node2/node.log 2>&1 &
      echo \$! > $IRONMAN_DIR/data-node2/node.pid
      echo 'PID:' \$(cat $IRONMAN_DIR/data-node2/node.pid)"

    echo ""
    echo "=== 3-node cluster started ==="
    echo "  Satyawan (v0): http://$SATYAWAN_IP:$API_BASE_PORT/explorer"
    echo "  Apsarth  (v1): http://$APSARTH_IP:$API_BASE_PORT/explorer"
    echo "  Ironman  (v2): http://$IRONMAN_IP:$API_BASE_PORT/explorer"
    echo ""
    echo "Logs:  ./scripts/deploy-3node.sh logs"
    echo "Stop:  ./scripts/deploy-3node.sh stop"
    ;;

  stop)
    echo "=== Stopping all nodes ==="
    ssh "$SATYAWAN_HOST" "cat $SATYAWAN_DIR/data-node0/node.pid 2>/dev/null | xargs kill 2>/dev/null; echo 'Satyawan stopped'" || true
    ssh "$APSARTH_HOST" "cat $APSARTH_DIR/data-node1/node.pid 2>/dev/null | xargs kill 2>/dev/null; echo 'Apsarth stopped'" || true
    ssh "$IRONMAN_HOST" "cat $IRONMAN_DIR/data-node2/node.pid 2>/dev/null | xargs kill 2>/dev/null; echo 'Ironman stopped'" || true
    echo "=== All nodes stopped ==="
    ;;

  status)
    echo "=== Node Status ==="
    echo -n "Satyawan (v0): "
    ssh "$SATYAWAN_HOST" "cat $SATYAWAN_DIR/data-node0/node.pid 2>/dev/null | xargs ps -p 2>/dev/null | grep -q evaporchain && echo 'RUNNING' || echo 'STOPPED'" 2>/dev/null || echo "UNREACHABLE"
    echo -n "Apsarth  (v1): "
    ssh "$APSARTH_HOST" "cat $APSARTH_DIR/data-node1/node.pid 2>/dev/null | xargs ps -p 2>/dev/null | grep -q evaporchain && echo 'RUNNING' || echo 'STOPPED'" 2>/dev/null || echo "UNREACHABLE"
    echo -n "Ironman  (v2): "
    ssh "$IRONMAN_HOST" "cat $IRONMAN_DIR/data-node2/node.pid 2>/dev/null | xargs ps -p 2>/dev/null | grep -q evaporchain && echo 'RUNNING' || echo 'STOPPED'" 2>/dev/null || echo "UNREACHABLE"
    ;;

  logs)
    echo "=== Last 20 lines from each node ==="
    echo "--- Satyawan (v0) ---"
    ssh "$SATYAWAN_HOST" "tail -20 $SATYAWAN_DIR/data-node0/node.log 2>/dev/null" || echo "(no log)"
    echo ""
    echo "--- Apsarth (v1) ---"
    ssh "$APSARTH_HOST" "tail -20 $APSARTH_DIR/data-node1/node.log 2>/dev/null" || echo "(no log)"
    echo ""
    echo "--- Ironman (v2) ---"
    ssh "$IRONMAN_HOST" "tail -20 $IRONMAN_DIR/data-node2/node.log 2>/dev/null" || echo "(no log)"
    ;;

  clean)
    echo "=== Cleaning data directories ==="
    ssh "$SATYAWAN_HOST" "rm -rf $SATYAWAN_DIR/data-node0" && echo "Satyawan cleaned"
    ssh "$APSARTH_HOST" "rm -rf $APSARTH_DIR/data-node1" && echo "Apsarth cleaned"
    ssh "$IRONMAN_HOST" "rm -rf $IRONMAN_DIR/data-node2" && echo "Ironman cleaned"
    ;;

  *)
    echo "Usage: $0 {build|start|stop|status|logs|clean}"
    ;;
esac
