#!/bin/bash
# EvaporChain 3-Node Cluster — Tailscale Mac Mini Deployment
#
# Deploys a 3-validator Tendermint BFT cluster across 3 Mac Minis
# connected via Tailscale VPN.
#
# Usage:
#   ./scripts/deploy-tailscale.sh setup    — sync code & build on all Minis
#   ./scripts/deploy-tailscale.sh start    — start all 3 validators
#   ./scripts/deploy-tailscale.sh stop     — stop all validators
#   ./scripts/deploy-tailscale.sh status   — check health of all nodes
#   ./scripts/deploy-tailscale.sh logs <N> — tail logs for node N (1-3)
#   ./scripts/deploy-tailscale.sh clean    — wipe data dirs (fresh start)
#   ./scripts/deploy-tailscale.sh test     — run cross-node integration tests

set -euo pipefail

# ─────────────────────── Tailscale IPs ──────────────────────────────────
MINI1_IP="100.119.53.101"    # satyawans-mac-mini
MINI2_IP="100.113.253.72"    # satyawan-mini-2
MINI3_IP="100.103.216.125"   # satyawan-mini-2s-mac-mini

MINI_IPS=("$MINI1_IP" "$MINI2_IP" "$MINI3_IP")
MINI_NAMES=("mini-1" "mini-2" "mini-3")
MINI_USERS=("satyawansingh" "satyawan-mini-1" "satyawan-mini-2")
MINI_HOMES=("/Users/satyawansingh" "/Users/satyawan-mini-1" "/Users/satyawan-mini-2")

# ─────────────────────── Ports ──────────────────────────────────────────
P2P_PORT=9000
API_PORT=8080

# ─────────────────────── Paths ──────────────────────────────────────────
BINARY="target/release/evaporchain-node"
GENESIS="genesis-tailscale-3node.json"

# ─────────────────────── SSH ────────────────────────────────────────────
SSH_OPTS="-o ConnectTimeout=5 -o StrictHostKeyChecking=no"

ssh_mini() {
    local idx=$1
    shift
    ssh $SSH_OPTS "${MINI_USERS[$idx]}@${MINI_IPS[$idx]}" "$@"
}

remote_dir() { echo "${MINI_HOMES[$1]}/EvaporChain"; }
data_dir() { echo "${MINI_HOMES[$1]}/evaporchain-data/validator-$(($1+1))"; }
log_dir() { echo "${MINI_HOMES[$1]}/evaporchain-logs"; }

# ─────────────────────── Commands ───────────────────────────────────────

cmd_setup() {
    echo "══════════════════════════════════════════════════════"
    echo "  Setting up 3-node cluster on Tailscale Minis"
    echo "══════════════════════════════════════════════════════"

    for i in 0 1 2; do
        local rdir=$(remote_dir $i)
        local ddir=$(data_dir $i)
        local ldir=$(log_dir $i)
        echo ""
        echo "── ${MINI_NAMES[$i]} (${MINI_IPS[$i]}, user=${MINI_USERS[$i]}) ──"

        # Check SSH
        if ! ssh_mini $i "echo ok" >/dev/null 2>&1; then
            echo "  SKIP: SSH failed"
            continue
        fi

        # Sync repo
        echo "  Syncing code..."
        rsync -az --delete \
            --exclude target \
            --exclude .git \
            --exclude evaporchain-data \
            -e "ssh $SSH_OPTS" \
            "$(dirname "$0")/../" \
            "${MINI_USERS[$i]}@${MINI_IPS[$i]}:${rdir}/"

        # Build
        echo "  Building release binary (this may take a while)..."
        ssh_mini $i "cd ${rdir} && cargo build --release -p evaporchain-node 2>&1 | tail -3"

        # Create dirs
        ssh_mini $i "mkdir -p ${ddir} ${ldir}"

        echo "  Done."
    done

    echo ""
    echo "Setup complete."
}

cmd_start() {
    echo "══════════════════════════════════════════════════════"
    echo "  Starting 3-validator Tendermint BFT cluster"
    echo "══════════════════════════════════════════════════════"

    for i in 0 1 2; do
        local vid=$((i + 1))
        local rdir=$(remote_dir $i)
        local ddir=$(data_dir $i)
        local ldir=$(log_dir $i)
        echo ""
        echo "── Starting validator ${vid} on ${MINI_NAMES[$i]} (${MINI_IPS[$i]}) ──"

        if ! ssh_mini $i "echo ok" >/dev/null 2>&1; then
            echo "  SKIP: SSH failed"
            continue
        fi

        # Check if already running
        if ssh_mini $i "pgrep -f evaporchain-node" >/dev/null 2>&1; then
            echo "  Already running — stop first with: $0 stop"
            continue
        fi

        # Build bootstrap peer list (all other nodes)
        local bootstrap=""
        for j in 0 1 2; do
            if [ $j -ne $i ]; then
                bootstrap="${bootstrap} --bootstrap /ip4/${MINI_IPS[$j]}/tcp/${P2P_PORT}"
            fi
        done

        # Start node
        ssh_mini $i "cd ${rdir} && nohup ./${BINARY} \
            --network --api --demo \
            --validator-id ${vid} --validators 3 \
            --node-id node-${vid} \
            --port ${P2P_PORT} --api-port ${API_PORT} \
            --data-dir ${ddir} \
            --genesis-config ${rdir}/${GENESIS} \
            --startup-delay 8000 \
            --interval 3000 \
            ${bootstrap} \
            > ${ldir}/validator-${vid}.log 2>&1 &"

        echo "  Started (PID on remote)."
    done

    echo ""
    echo "All validators started. Waiting 15s for peer discovery..."
    sleep 15
    cmd_status
}

cmd_stop() {
    echo "Stopping all validators..."
    for i in 0 1 2; do
        if ssh_mini $i "pkill -f evaporchain-node" 2>/dev/null; then
            echo "  ${MINI_NAMES[$i]}: stopped"
        else
            echo "  ${MINI_NAMES[$i]}: not running (or SSH failed)"
        fi
    done
}

cmd_status() {
    echo "══════════════════════════════════════════════════════"
    echo "  Cluster Status"
    echo "══════════════════════════════════════════════════════"
    echo ""

    for i in 0 1 2; do
        local ip="${MINI_IPS[$i]}"
        local name="${MINI_NAMES[$i]}"

        printf "  %-8s (%s): " "$name" "$ip"

        local status
        status=$(curl -s --connect-timeout 3 "http://${ip}:${API_PORT}/api/status" 2>/dev/null)
        if [ -z "$status" ]; then
            echo "OFFLINE"
            continue
        fi

        local height peers state_root
        height=$(echo "$status" | python3 -c "import json,sys; print(json.load(sys.stdin).get('block_height',0))" 2>/dev/null || echo "?")
        peers=$(echo "$status" | python3 -c "import json,sys; print(json.load(sys.stdin).get('peer_count',0))" 2>/dev/null || echo "?")
        state_root=$(echo "$status" | python3 -c "import json,sys; r=json.load(sys.stdin).get('state_root',''); print(r[:16]+'...' if len(r)>16 else r)" 2>/dev/null || echo "?")

        echo "height=${height}  peers=${peers}  root=${state_root}"
    done

    echo ""
}

cmd_logs() {
    local node_num="${1:?Usage: $0 logs <1|2|3>}"
    local idx=$((node_num - 1))
    local ldir=$(log_dir $idx)
    echo "Tailing logs for validator ${node_num} on ${MINI_NAMES[$idx]}..."
    ssh_mini $idx "tail -f ${ldir}/validator-${node_num}.log"
}

cmd_clean() {
    echo "Wiping data directories on all Minis..."
    for i in 0 1 2; do
        local ddir=$(data_dir $i)
        if ssh_mini $i "rm -rf ${ddir}" 2>/dev/null; then
            echo "  ${MINI_NAMES[$i]}: cleaned"
        else
            echo "  ${MINI_NAMES[$i]}: failed (or SSH failed)"
        fi
    done
    echo "All data wiped. Next start will be a fresh genesis."
}

cmd_test() {
    echo "Running cross-node integration tests..."
    bash "$(dirname "$0")/cross_node_test.sh"
}

# ─────────────────────── Main ───────────────────────────────────────────

case "${1:-help}" in
    setup)  cmd_setup ;;
    start)  cmd_start ;;
    stop)   cmd_stop ;;
    status) cmd_status ;;
    logs)   cmd_logs "${2:-}" ;;
    clean)  cmd_clean ;;
    test)   cmd_test ;;
    *)
        echo "EvaporChain Tailscale Cluster Manager"
        echo ""
        echo "Usage: $0 <command>"
        echo ""
        echo "Commands:"
        echo "  setup   — sync code & build release binary on all Minis"
        echo "  start   — start 3-validator Tendermint cluster"
        echo "  stop    — stop all validators"
        echo "  status  — check health of all nodes"
        echo "  logs N  — tail logs for validator N (1-3)"
        echo "  clean   — wipe data dirs for fresh start"
        echo "  test    — run cross-node integration tests"
        ;;
esac
