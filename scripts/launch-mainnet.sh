#!/usr/bin/env bash
#
# EvaporChain Mainnet Genesis Launcher
#
# Initializes and launches a validator node from genesis-mainnet.json.
#
# Usage:
#   ./scripts/launch-mainnet.sh --validator-id 1 [--data-dir /path] [--api-port 8080]
#
# Prerequisites:
#   1. Build:   cargo build -p evaporchain-node --release
#   2. Keygen:  cargo run -p evaporchain-cli -- keygen --output keys.json
#   3. Verify:  cargo run -p evaporchain-cli -- genesis validate genesis-mainnet.json
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY="$ROOT_DIR/target/release/evaporchain-node"
GENESIS="$ROOT_DIR/genesis-mainnet.json"

# ── Parse arguments ──
VALIDATOR_ID="${VALIDATOR_ID:-1}"
DATA_DIR="${DATA_DIR:-$ROOT_DIR/data/validator-$VALIDATOR_ID}"
API_PORT="${API_PORT:-8080}"
P2P_PORT="${P2P_PORT:-9000}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --validator-id) VALIDATOR_ID="$2"; shift 2 ;;
        --data-dir)     DATA_DIR="$2"; shift 2 ;;
        --api-port)     API_PORT="$2"; shift 2 ;;
        --p2p-port)     P2P_PORT="$2"; shift 2 ;;
        *)              echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# ── Preflight checks ──
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found. Build first:"
    echo "  cargo build -p evaporchain-node --release"
    exit 1
fi

if [ ! -f "$GENESIS" ]; then
    echo "ERROR: Genesis config not found at $GENESIS"
    exit 1
fi

# ── Validate genesis (offline) ──
echo "Validating genesis config..."
cargo run -p evaporchain-cli --quiet -- genesis validate "$GENESIS"
echo ""

# ── Create data directory ──
mkdir -p "$DATA_DIR"

# ── Extract bootstrap peers from genesis ──
BOOTSTRAP_ARGS=""
PEERS=$(python3 -c "
import json, sys
with open('$GENESIS') as f:
    cfg = json.load(f)
for p in cfg.get('bootstrap_peers', []):
    print(p)
" 2>/dev/null || true)

for peer in $PEERS; do
    BOOTSTRAP_ARGS="$BOOTSTRAP_ARGS --bootstrap $peer"
done

# ── Launch ──
echo "============================================================"
echo "  EvaporChain Mainnet Node"
echo "  Validator ID:  $VALIDATOR_ID"
echo "  Data dir:      $DATA_DIR"
echo "  API port:      $API_PORT"
echo "  P2P port:      $P2P_PORT"
echo "  Genesis:       $GENESIS"
echo "============================================================"
echo ""

exec "$BINARY" \
    --node-id "validator-$VALIDATOR_ID" \
    --port "$P2P_PORT" \
    --api-port "$API_PORT" \
    --network \
    --api \
    --data-dir "$DATA_DIR" \
    --genesis-config "$GENESIS" \
    --validator-id "$VALIDATOR_ID" \
    --validators 4 \
    --startup-delay 3000 \
    $BOOTSTRAP_ARGS
