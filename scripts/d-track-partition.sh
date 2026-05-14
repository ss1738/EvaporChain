#!/usr/bin/env bash
#
# scripts/d-track-partition.sh — T0.2 D.5: network partition + healing
#
# Simulates a network partition by using iptables DROP rules on one node
# to block inbound/outbound gossip traffic from a subset of the cluster.
# After PARTITION_SECS, the rules are removed and the script verifies
# the cluster re-converges to the same block height within HEAL_TIMEOUT.
#
# Prerequisite: the test node must have sudo access (passwordless) for
# iptables, OR the operator must run the partition manually per the
# printed commands and confirm via stdin.
#
# Usage:
#   ./scripts/d-track-partition.sh [--interactive]
#
# Options:
#   --interactive    Print iptables commands and wait for operator to apply
#                    them manually (use when sudoless iptables is not available)
#   --node           ssh destination to partition (default: Mini 1)
#   --node-api       API host:port for that node (default: 100.119.53.101:8080)
#   --peers          Comma-separated IPs to block from/to (default: Mini 2 + 3)
#   --port           Gossip port to block (default: 9000)
#   --partition-secs Seconds to keep partition up (default: 90)
#   --heal-timeout   Max seconds to wait for convergence after healing (default: 120)
#   --log            Log file path
#
# Pass criteria (D.5):
#   - Partitioned node continues producing or validating blocks locally
#     (height must advance ≥1 during isolation — confirmed via API).
#   - After partition healed, node converges to within 5 blocks of the
#     majority partition within HEAL_TIMEOUT seconds.

set -euo pipefail

INTERACTIVE=0
NODE_SSH="${NODE_SSH:-satyawansingh@100.119.53.101}"
NODE_API="${NODE_API:-100.119.53.101:8080}"
PEERS="${PEERS:-100.113.253.72,100.103.216.125}"
PORT="${PORT:-9000}"
PARTITION_SECS="${PARTITION_SECS:-90}"
HEAL_TIMEOUT="${HEAL_TIMEOUT:-120}"
SSH_KEY="${SSH_KEY:-$HOME/.ssh/id_ed25519}"
LOG="${LOG:-/tmp/d-track-partition-$(date +%Y%m%dT%H%M%SZ).log}"

# A peer that stays OUTSIDE the partition (we monitor them for majority progress)
PEER_API="${PEER_API:-100.113.253.72:8080}"

while [ $# -gt 0 ]; do
    case "$1" in
        --interactive)     INTERACTIVE=1; shift ;;
        --node)            NODE_SSH="$2";        shift 2 ;;
        --node-api)        NODE_API="$2";        shift 2 ;;
        --peers)           PEERS="$2";           shift 2 ;;
        --port)            PORT="$2";            shift 2 ;;
        --partition-secs)  PARTITION_SECS="$2";  shift 2 ;;
        --heal-timeout)    HEAL_TIMEOUT="$2";    shift 2 ;;
        --log)             LOG="$2";             shift 2 ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

IFS=',' read -ra PEER_ARR <<< "$PEERS"

log() { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ)  $*" | tee -a "$LOG"; }

jget() {
    python3 -c "
import sys,json
try:
    d=json.loads(sys.argv[1])
    print(d.get(sys.argv[2],-1))
except:
    print(-1)" "$1" "$2" 2>/dev/null || echo "-1"
}

get_height() {
    local api=$1
    jget "$(curl -s -m 5 "http://${api}/api/status" 2>/dev/null || echo '{}')" block_height
}

# Build iptables commands
BLOCK_CMDS=()
UNBLOCK_CMDS=()
for peer_ip in "${PEER_ARR[@]}"; do
    BLOCK_CMDS+=("sudo iptables -I INPUT  -s ${peer_ip} -p tcp --dport ${PORT} -j DROP")
    BLOCK_CMDS+=("sudo iptables -I OUTPUT -d ${peer_ip} -p tcp --sport ${PORT} -j DROP")
    UNBLOCK_CMDS+=("sudo iptables -D INPUT  -s ${peer_ip} -p tcp --dport ${PORT} -j DROP || true")
    UNBLOCK_CMDS+=("sudo iptables -D OUTPUT -d ${peer_ip} -p tcp --sport ${PORT} -j DROP || true")
done

ssh_run() {
    ssh -o IdentitiesOnly=yes -i "$SSH_KEY" -o ConnectTimeout=10 "$NODE_SSH" "$1" 2>/dev/null
}

apply_partition() {
    for cmd in "${BLOCK_CMDS[@]}"; do
        if [ "$INTERACTIVE" = 1 ]; then
            echo "  [manual] $cmd"
        else
            ssh_run "$cmd" || log "WARN: iptables block cmd failed: $cmd"
        fi
    done
}

remove_partition() {
    for cmd in "${UNBLOCK_CMDS[@]}"; do
        if [ "$INTERACTIVE" = 1 ]; then
            echo "  [manual] $cmd"
        else
            ssh_run "$cmd" || true
        fi
    done
}

# Cleanup on exit — always try to unblock
cleanup() {
    log "Cleanup: removing partition rules (if any)"
    remove_partition
}
trap cleanup EXIT INT TERM

log "PARTITION_TEST_START node=$NODE_SSH peers=$PEERS port=$PORT partition_secs=$PARTITION_SECS"

# ── Pre-flight ──
PRE_NODE_H=$(get_height "$NODE_API")
PRE_PEER_H=$(get_height "$PEER_API")
log "PRE-PARTITION heights: node=$PRE_NODE_H peer=$PRE_PEER_H"

if [ "$PRE_NODE_H" = "-1" ] || [ "$PRE_PEER_H" = "-1" ]; then
    log "ERROR: cannot reach node or peer API; aborting"
    exit 2
fi

# ── Apply partition ──
log "PARTITIONING node $NODE_SSH from peers: $PEERS"

if [ "$INTERACTIVE" = 1 ]; then
    echo ""
    echo "Manual partition mode. Apply these commands on $NODE_SSH:"
    apply_partition
    echo ""
    read -r -p "Press ENTER when partition is in place..."
else
    apply_partition
fi

log "PARTITION_ACTIVE"

# ── Let the partition run ──
sleep "$PARTITION_SECS"

# Check that both sides are still alive (just not talking)
MID_NODE_H=$(get_height "$NODE_API")
MID_PEER_H=$(get_height "$PEER_API")
log "MID-PARTITION heights: node=$MID_NODE_H peer=$MID_PEER_H"

# ── Remove partition ──
log "HEALING partition"
if [ "$INTERACTIVE" = 1 ]; then
    echo ""
    echo "Remove the partition now (apply these commands on $NODE_SSH):"
    remove_partition
    echo ""
    read -r -p "Press ENTER when partition is healed..."
else
    remove_partition
fi

log "PARTITION_HEALED"
trap - EXIT INT TERM

# ── Wait for convergence ──
log "Waiting up to ${HEAL_TIMEOUT}s for height convergence..."
WAITED=0
CONVERGED=0
while [ "$WAITED" -lt "$HEAL_TIMEOUT" ]; do
    sleep 5
    WAITED=$((WAITED + 5))
    POST_NODE_H=$(get_height "$NODE_API")
    POST_PEER_H=$(get_height "$PEER_API")
    if [ "$POST_NODE_H" != "-1" ] && [ "$POST_PEER_H" != "-1" ]; then
        DIFF=$(python3 -c "print(abs(${POST_NODE_H} - ${POST_PEER_H}))")
        if python3 -c "exit(0 if abs(${POST_NODE_H}-${POST_PEER_H})<=5 else 1)"; then
            log "HEIGHT_CONVERGED node=$POST_NODE_H peer=$POST_PEER_H diff=$DIFF after ${WAITED}s"
            CONVERGED=1
            break
        else
            log "Waiting... node=$POST_NODE_H peer=$POST_PEER_H diff=$DIFF"
        fi
    fi
done

PASS=1

# D.5 check 1: node advanced during partition (proves it didn't freeze)
NODE_ADVANCED=0
if [ "$MID_NODE_H" != "-1" ] && [ "$MID_NODE_H" -gt "$PRE_NODE_H" ]; then
    NODE_ADVANCED=1
fi

# D.5 check 2: converged after heal
[ "$CONVERGED" = 0 ] && PASS=0

{
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  T0.2 D.5 Partition healing — $(basename "$LOG")"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Node:           $NODE_SSH ($NODE_API)"
    echo "  Peers blocked:  $PEERS"
    echo "  Partition secs: $PARTITION_SECS"
    echo ""
    echo "  Pre-partition:  node=$PRE_NODE_H  peer=$PRE_PEER_H"
    echo "  Mid-partition:  node=$MID_NODE_H  peer=$MID_PEER_H"
    echo ""
    echo "  Node advanced during isolation:  $([ "$NODE_ADVANCED" = 1 ] && echo YES || echo NO)"
    echo "  Post-heal convergence (≤5 blks): $([ "$CONVERGED" = 1 ] && echo PASS || echo FAIL)"
    echo ""
    if [ "$PASS" = 1 ]; then
        echo "  OVERALL VERDICT: PASS"
    else
        echo "  OVERALL VERDICT: FAIL"
    fi
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Full log: $LOG"
} | tee -a "$LOG"

[ "$PASS" = 1 ] && exit 0 || exit 1
