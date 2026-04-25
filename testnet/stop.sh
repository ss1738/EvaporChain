#!/bin/bash
# stop.sh — Stop EvaporChain testnet nodes on all 3 Mac Minis.

set -uo pipefail

NODES=("satyawan" "ironman" "apsarth")

echo "=== Stopping EvaporChain Testnet ==="
echo ""

for i in "${!NODES[@]}"; do
    node="${NODES[$i]}"
    num=$((i + 1))
    echo "--- Stopping node ${num} on ${node} ---"
    ssh -o ConnectTimeout=5 "${node}" 'pkill -f evaporchain-node 2>/dev/null && echo "  Stopped." || echo "  Not running."'
    echo ""
done

echo "All stop commands sent."
