#!/bin/bash
# deploy.sh — Build and deploy EvaporChain testnet to 3 Mac Minis via Tailscale.
# Run from the machine with the source code.
#
# Pass --execute to actually rsync files (default: dry-run, prints commands).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BINARY="${PROJECT_DIR}/target/release/evaporchain-node"

NODES=("satyawan" "ironman" "apsarth")
EXECUTE=false

if [[ "${1:-}" == "--execute" ]]; then
    EXECUTE=true
fi

echo "=== EvaporChain Testnet Deploy ==="
echo ""

if [[ ! -f "${BINARY}" ]]; then
    echo "Binary not found at ${BINARY}"
    echo "Build first: cargo build --release -p evaporchain-node"
    exit 1
fi

for node in "${NODES[@]}"; do
    echo "--- ${node} ---"
    if $EXECUTE; then
        ssh "${node}" "mkdir -p ~/evaporchain-testnet"
        rsync -avz "${BINARY}" "${node}:~/evaporchain-testnet/evaporchain-node"
        rsync -avz "${SCRIPT_DIR}/genesis.json" "${node}:~/evaporchain-testnet/genesis.json"
        rsync -avz "${SCRIPT_DIR}/launch.sh" "${node}:~/evaporchain-testnet/launch.sh"
        ssh "${node}" "chmod +x ~/evaporchain-testnet/launch.sh ~/evaporchain-testnet/evaporchain-node"
        echo "  Deployed."
    else
        echo "  ssh ${node} 'mkdir -p ~/evaporchain-testnet'"
        echo "  rsync -avz ${BINARY} ${node}:~/evaporchain-testnet/evaporchain-node"
        echo "  rsync -avz ${SCRIPT_DIR}/genesis.json ${node}:~/evaporchain-testnet/genesis.json"
        echo "  rsync -avz ${SCRIPT_DIR}/launch.sh ${node}:~/evaporchain-testnet/launch.sh"
        echo "  ssh ${node} 'chmod +x ~/evaporchain-testnet/launch.sh ~/evaporchain-testnet/evaporchain-node'"
    fi
    echo ""
done

if ! $EXECUTE; then
    echo "Dry run. Pass --execute to deploy."
    echo ""
fi

echo "=== Start Nodes ==="
echo ""
echo "  ssh satyawan 'cd ~/evaporchain-testnet && nohup ./launch.sh 1 > node1.log 2>&1 &'"
echo "  ssh ironman  'cd ~/evaporchain-testnet && nohup ./launch.sh 2 > node2.log 2>&1 &'"
echo "  ssh apsarth  'cd ~/evaporchain-testnet && nohup ./launch.sh 3 > node3.log 2>&1 &'"
echo ""
