#!/usr/bin/env bash
set -euo pipefail

# ╔══════════════════════════════════════════════════════════════════════════════╗
# ║  EvaporChain Testnet Deployment Script                                      ║
# ║                                                                              ║
# ║  This script documents exactly how to deploy a 4-node EvaporChain testnet   ║
# ║  on cloud servers. Run each section on the appropriate server.               ║
# ╚══════════════════════════════════════════════════════════════════════════════╝
#
# RECOMMENDED PROVIDER: Hetzner Cloud
#   - Server type: CX21 (2 vCPU, 4GB RAM, 40GB SSD) — ~€5/month each
#   - OS: Ubuntu 22.04 LTS
#   - Region: Nuremberg (nbg1) or Falkenstein (fsn1)
#   - Total cost: ~€20/month for 4 nodes
#
# NETWORK LAYOUT:
#   Node 1 (Leader/API):  port 9001, API on port 3000  ← public-facing
#   Node 2 (Validator):   port 9002
#   Node 3 (Validator):   port 9003
#   Node 4 (Validator):   port 9004
#
# PREREQUISITES:
#   - 4 Hetzner servers with Ubuntu 22.04
#   - SSH access to all 4 servers
#   - Note down the public IPs of all 4 servers

echo "╔══════════════════════════════════════════════════╗"
echo "║  EvaporChain Testnet Deployment                  ║"
echo "╚══════════════════════════════════════════════════╝"

# ─────────────────────────────────────────────────────────────────────────────
# STEP 1: Install Rust and build tools (run on ALL 4 servers)
# ─────────────────────────────────────────────────────────────────────────────

install_dependencies() {
    echo ">>> Installing system dependencies..."
    sudo apt update && sudo apt upgrade -y
    sudo apt install -y build-essential pkg-config libssl-dev git curl

    echo ">>> Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"

    echo ">>> Rust installed: $(rustc --version)"
}

# ─────────────────────────────────────────────────────────────────────────────
# STEP 2: Clone and build (run on ALL 4 servers)
# ─────────────────────────────────────────────────────────────────────────────

build_evaporchain() {
    echo ">>> Cloning EvaporChain..."
    cd "$HOME"
    git clone https://github.com/ss1738/EvaporChain.git || (cd EvaporChain && git pull)
    cd EvaporChain

    echo ">>> Building release binary..."
    cargo build --release -p evaporchain-node

    echo ">>> Build complete: $(ls -lh target/release/evaporchain-node)"
}

# ─────────────────────────────────────────────────────────────────────────────
# STEP 3: Configure and run each node
# ─────────────────────────────────────────────────────────────────────────────

# Replace these with your actual server IPs
NODE1_IP="YOUR_NODE1_IP"
NODE2_IP="YOUR_NODE2_IP"
NODE3_IP="YOUR_NODE3_IP"
NODE4_IP="YOUR_NODE4_IP"

# Run on Node 1 (Leader + API + Dashboard + Faucet):
run_node1() {
    cd "$HOME/EvaporChain"
    ./target/release/evaporchain-node \
        --api \
        --api-port 3000 \
        --port 9001 \
        --node-id validator-1 \
        --network \
        --interval 3000 \
        --bootstrap "/ip4/${NODE2_IP}/tcp/9002" \
        --bootstrap "/ip4/${NODE3_IP}/tcp/9003" \
        --bootstrap "/ip4/${NODE4_IP}/tcp/9004"
}

# Run on Node 2:
run_node2() {
    cd "$HOME/EvaporChain"
    ./target/release/evaporchain-node \
        --port 9002 \
        --node-id validator-2 \
        --network \
        --interval 3000 \
        --bootstrap "/ip4/${NODE1_IP}/tcp/9001" \
        --bootstrap "/ip4/${NODE3_IP}/tcp/9003" \
        --bootstrap "/ip4/${NODE4_IP}/tcp/9004"
}

# Run on Node 3:
run_node3() {
    cd "$HOME/EvaporChain"
    ./target/release/evaporchain-node \
        --port 9003 \
        --node-id validator-3 \
        --network \
        --interval 3000 \
        --bootstrap "/ip4/${NODE1_IP}/tcp/9001" \
        --bootstrap "/ip4/${NODE2_IP}/tcp/9002" \
        --bootstrap "/ip4/${NODE4_IP}/tcp/9004"
}

# Run on Node 4:
run_node4() {
    cd "$HOME/EvaporChain"
    ./target/release/evaporchain-node \
        --port 9004 \
        --node-id validator-4 \
        --network \
        --interval 3000 \
        --bootstrap "/ip4/${NODE1_IP}/tcp/9001" \
        --bootstrap "/ip4/${NODE2_IP}/tcp/9002" \
        --bootstrap "/ip4/${NODE3_IP}/tcp/9003"
}

# ─────────────────────────────────────────────────────────────────────────────
# STEP 4: systemd service (run on each server, adjust NODE_NUM)
# ─────────────────────────────────────────────────────────────────────────────

install_systemd_service() {
    local NODE_NUM="${1:?Usage: install_systemd_service <1|2|3|4>}"
    local NODE_PORT=$((9000 + NODE_NUM))

    # Determine bootstrap peers (all other nodes)
    local BOOTSTRAP_ARGS=""
    for i in 1 2 3 4; do
        if [ "$i" != "$NODE_NUM" ]; then
            local VAR="NODE${i}_IP"
            local PEER_IP="${!VAR}"
            local PEER_PORT=$((9000 + i))
            BOOTSTRAP_ARGS="$BOOTSTRAP_ARGS --bootstrap /ip4/${PEER_IP}/tcp/${PEER_PORT}"
        fi
    done

    # API flags only for node 1
    local API_FLAGS=""
    if [ "$NODE_NUM" = "1" ]; then
        API_FLAGS="--api --api-port 3000"
    fi

    # DEPLOY-1 (audit 2026-05-16): write the systemd unit to a
    # mktemp file with 0600 perms instead of a predictable, world-
    # readable /tmp path.  Pre-fix any local user on the host could
    # read --data-dir, API port, env vars, and binary path during
    # the brief window between `cat >` and `sudo mv`.  mktemp gives
    # us atomic creation owned by the invoking user; the trap
    # cleans up if the script dies mid-build.
    local SVCTMP
    SVCTMP=$(mktemp -t evaporchain-svc.XXXXXX)
    chmod 600 "$SVCTMP"
    # shellcheck disable=SC2064
    trap "rm -f '$SVCTMP'" EXIT

    cat > "$SVCTMP" << SERVICEEOF
[Unit]
Description=EvaporChain Testnet Node ${NODE_NUM}
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/root/EvaporChain
ExecStart=/root/EvaporChain/target/release/evaporchain-node \\
    --port ${NODE_PORT} \\
    --node-id validator-${NODE_NUM} \\
    --network \\
    --interval 3000 \\
    ${API_FLAGS} \\
    ${BOOTSTRAP_ARGS}
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
SERVICEEOF

    sudo mv "$SVCTMP" /etc/systemd/system/evaporchain.service
    trap - EXIT
    sudo systemctl daemon-reload
    sudo systemctl enable evaporchain
    sudo systemctl start evaporchain

    echo ">>> Node ${NODE_NUM} service installed and started."
    echo ">>> Check status: sudo systemctl status evaporchain"
    echo ">>> View logs:    sudo journalctl -u evaporchain -f"
}

# ─────────────────────────────────────────────────────────────────────────────
# STEP 5: Firewall (run on each server)
# ─────────────────────────────────────────────────────────────────────────────

configure_firewall() {
    local NODE_NUM="${1:?Usage: configure_firewall <1|2|3|4>}"
    local NODE_PORT=$((9000 + NODE_NUM))

    sudo ufw allow 22/tcp        # SSH
    sudo ufw allow ${NODE_PORT}/tcp  # P2P

    if [ "$NODE_NUM" = "1" ]; then
        sudo ufw allow 3000/tcp  # API + Dashboard + Faucet
    fi

    sudo ufw --force enable
    echo ">>> Firewall configured for node ${NODE_NUM}."
}

# ─────────────────────────────────────────────────────────────────────────────
# QUICK START — Run these commands in order on each server
# ─────────────────────────────────────────────────────────────────────────────
#
# On ALL 4 servers:
#   1. SSH in: ssh root@<SERVER_IP>
#   2. Download this script:
#      curl -O https://raw.githubusercontent.com/ss1738/EvaporChain/main/scripts/deploy-testnet.sh
#      chmod +x deploy-testnet.sh
#   3. Edit the NODE*_IP variables at the top of this script
#   4. Run:
#      source deploy-testnet.sh
#      install_dependencies
#      build_evaporchain
#      configure_firewall <NODE_NUM>      # 1, 2, 3, or 4
#      install_systemd_service <NODE_NUM> # 1, 2, 3, or 4
#
# After all 4 are running:
#   - Dashboard:  http://<NODE1_IP>:3000
#   - Faucet:     http://<NODE1_IP>:3000/faucet
#   - API:        http://<NODE1_IP>:3000/api/status
#
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "Script loaded. Available functions:"
echo "  install_dependencies        — Install Rust and build tools"
echo "  build_evaporchain           — Clone and build the node"
echo "  configure_firewall <N>      — Set up UFW firewall for node N"
echo "  install_systemd_service <N> — Install and start systemd service for node N"
echo "  run_node1..run_node4        — Run a node directly (foreground)"
echo ""
echo "Edit NODE1_IP..NODE4_IP before running."
