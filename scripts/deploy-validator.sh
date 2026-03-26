#!/bin/bash
# Deploy a single EvaporChain validator to a remote server.
#
# Usage: ./scripts/deploy-validator.sh <validator-id> <server-ip> <p2p-port> <api-port>
#
# Example:
#   ./scripts/deploy-validator.sh 1 37.27.1.1 9000 8080
#
# Prerequisites:
#   - SSH key at ~/.ssh/evaporchain
#   - Rust installed on the server
#   - The server has the EvaporChain repo at /root/EvaporChain/

set -e

VALIDATOR_ID="${1:?Usage: deploy-validator.sh <validator-id> <server-ip> <p2p-port> <api-port>}"
SERVER_IP="${2:?Missing server IP}"
P2P_PORT="${3:-9000}"
API_PORT="${4:-8080}"
SSH_KEY="$HOME/.ssh/evaporchain"
REMOTE_DIR="/root/EvaporChain"
DATA_DIR="/var/lib/evaporchain/validator-${VALIDATOR_ID}"

VALIDATORS=4
BOOTSTRAP_PEERS=""
for i in $(seq 1 $VALIDATORS); do
    if [ "$i" != "$VALIDATOR_ID" ]; then
        PORT=$((8999 + i))
        BOOTSTRAP_PEERS="$BOOTSTRAP_PEERS --bootstrap /ip4/${SERVER_IP}/tcp/${PORT}"
    fi
done

echo "Deploying Validator $VALIDATOR_ID to $SERVER_IP:$P2P_PORT (API :$API_PORT)"

# Sync code
echo "Syncing code..."
rsync -az --exclude target --exclude .git \
    -e "ssh -i $SSH_KEY" \
    "$(dirname "$0")/../" \
    "root@${SERVER_IP}:${REMOTE_DIR}/"

# Build on server
echo "Building on server..."
ssh -i "$SSH_KEY" "root@${SERVER_IP}" "
    source ~/.cargo/env
    cd ${REMOTE_DIR}
    cargo build --release -p evaporchain-node
"

# Create data directory
ssh -i "$SSH_KEY" "root@${SERVER_IP}" "mkdir -p ${DATA_DIR}"

# Create systemd service
UNIT_NAME="evaporchain-v${VALIDATOR_ID}"
echo "Creating systemd service: ${UNIT_NAME}"
ssh -i "$SSH_KEY" "root@${SERVER_IP}" "cat > /etc/systemd/system/${UNIT_NAME}.service << 'UNIT'
[Unit]
Description=EvaporChain Validator ${VALIDATOR_ID}
After=network.target

[Service]
Type=simple
ExecStart=${REMOTE_DIR}/target/release/evaporchain-node \\
    --tendermint --network --api --demo \\
    --validator-id ${VALIDATOR_ID} --validators ${VALIDATORS} \\
    --node-id node-${VALIDATOR_ID} \\
    --port ${P2P_PORT} --api-port ${API_PORT} \\
    --data-dir ${DATA_DIR} \\
    --startup-delay 5000 \\
    ${BOOTSTRAP_PEERS}
Restart=on-failure
RestartSec=5
WorkingDirectory=${REMOTE_DIR}
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
UNIT"

# Enable and start
ssh -i "$SSH_KEY" "root@${SERVER_IP}" "
    systemctl daemon-reload
    systemctl enable ${UNIT_NAME}
    systemctl restart ${UNIT_NAME}
    sleep 2
    systemctl is-active ${UNIT_NAME}
"

echo "Validator $VALIDATOR_ID deployed and running!"
echo "  Dashboard: http://${SERVER_IP}:${API_PORT}"
echo "  P2P:       /ip4/${SERVER_IP}/tcp/${P2P_PORT}"
echo "  Logs:      ssh -i $SSH_KEY root@${SERVER_IP} journalctl -u ${UNIT_NAME} -f"
