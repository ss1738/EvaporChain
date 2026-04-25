#!/bin/bash
# launch.sh — Start an EvaporChain testnet validator node.
# Usage: ./launch.sh <node_number>
#   node_number: 1 (satyawan), 2 (ironman), or 3 (apsarth)

set -euo pipefail

NODE_NUM="${1:?Usage: ./launch.sh <1|2|3>}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
GENESIS="${SCRIPT_DIR}/genesis.json"
BINARY="${SCRIPT_DIR}/evaporchain-node"
if [ ! -f "${BINARY}" ]; then
    BINARY="${SCRIPT_DIR}/../target/release/evaporchain-node"
fi

case "$NODE_NUM" in
    1) NODE_NAME="satyawan"; NODE_IP="100.119.53.101" ;;
    2) NODE_NAME="ironman";  NODE_IP="100.103.216.125" ;;
    3) NODE_NAME="apsarth";  NODE_IP="100.113.253.72"  ;;
    *) echo "Error: node_number must be 1, 2, or 3"; exit 1 ;;
esac

VALIDATOR_ID="$NODE_NUM"
P2P_PORT=$((9000 + NODE_NUM - 1))
API_PORT=$((8080 + NODE_NUM))
DATA_DIR="${HOME}/evaporchain-data/node-${NODE_NUM}"

echo "=== EvaporChain Testnet Node ${NODE_NUM} (${NODE_NAME}) ==="
echo "  Validator ID: ${VALIDATOR_ID}"
echo "  P2P Port:     ${P2P_PORT}"
echo "  API Port:     ${API_PORT}"
echo "  Data Dir:     ${DATA_DIR}"
echo "  Genesis:      ${GENESIS}"
echo ""

mkdir -p "${DATA_DIR}"

# Build bootstrap peer list (all nodes except self)
BOOTSTRAP_ARGS=""
IPS="100.119.53.101 100.103.216.125 100.113.253.72"
IDX=0
for ip in $IPS; do
    IDX=$((IDX + 1))
    if [ "$IDX" != "$NODE_NUM" ]; then
        PEER_PORT=$((9000 + IDX - 1))
        BOOTSTRAP_ARGS="${BOOTSTRAP_ARGS} --bootstrap /ip4/${ip}/tcp/${PEER_PORT}"
    fi
done

export RUST_LOG=info

exec "${BINARY}" \
    --network \
    --api \
    --node-id "node-${NODE_NUM}" \
    --validator-id "${VALIDATOR_ID}" \
    --validators 3 \
    --port "${P2P_PORT}" \
    --api-port "${API_PORT}" \
    --data-dir "${DATA_DIR}" \
    --genesis-config "${GENESIS}" \
    --interval 3000 \
    ${BOOTSTRAP_ARGS}
