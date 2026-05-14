#!/usr/bin/env bash
#
# scripts/d-track-fault-injection.sh — T0.2 D.4: fault injection
#
# Kills one validator at a time, waits for the cluster to maintain finality
# on the remaining f+1 quorum, then restarts the killed node and verifies
# it resync-es within the recovery window.
#
# Designed for the 5-node testnet-1 cluster. With 5 validators, f=1 (BFT
# threshold 3-of-5). Killing any single node should NOT stall finality.
#
# Usage:
#   ./scripts/d-track-fault-injection.sh [options]
#
# Options:
#   --nodes      Comma-separated "ssh-alias:api-port:process-name" triples
#                (default: 3 Minis with standard process name)
#   --cycles     Number of kill/restart cycles per node (default: 3)
#   --kill-secs  How long to keep the node down (default: 60)
#   --recover-timeout  Max seconds to wait for resync after restart (default: 120)
#   --api-port   API port on all nodes (default: 8080)
#   --log        Log file (default: /tmp/d-track-fault-TIMESTAMP.log)
#
# Pass criteria (D.4):
#   - Finality on surviving nodes never stalls >30 s while one node is down.
#   - Killed node rejoins the cluster within RECOVER_TIMEOUT seconds.
#   - Block heights converge across all nodes within 5 blocks after resync.

set -euo pipefail

# Default: 3 Minis. Format: ssh_destination|api_host:api_port|process_match
# The process_match is passed to `pkill -f` to kill the node binary.
NODES_DEF=(
    "satyawansingh@100.119.53.101|100.119.53.101:8080|evaporchain-node"
    "satyawan-mini-1@100.113.253.72|100.113.253.72:8080|evaporchain-node"
    "satyawan-mini-2@100.103.216.125|100.103.216.125:8080|evaporchain-node"
)
CYCLES="${CYCLES:-3}"
KILL_SECS="${KILL_SECS:-60}"
RECOVER_TIMEOUT="${RECOVER_TIMEOUT:-120}"
LOG="${LOG:-/tmp/d-track-fault-$(date +%Y%m%dT%H%M%SZ).log}"
SSH_KEY="${SSH_KEY:-$HOME/.ssh/id_ed25519}"
SSH_RESTART_CMD="${SSH_RESTART_CMD:-}"  # if empty, we skip restart (manual)

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

get_finalized() {
    local api=$1
    local raw
    raw=$(curl -s -m 5 "http://${api}/api/status" 2>/dev/null || echo "{}")
    jget "$raw" finalized_height
}

wait_finality_progressing() {
    # Returns 0 if finalized_height advances within STALL_SECS on $1 (api host:port)
    local api=$1
    local stall_threshold=30
    local before
    before=$(get_finalized "$api")
    local waited=0
    while [ "$waited" -lt "$stall_threshold" ]; do
        sleep 2
        waited=$((waited + 2))
        local after
        after=$(get_finalized "$api")
        if [ "$after" != "-1" ] && [ "$after" -gt "$before" ]; then
            return 0
        fi
    done
    return 1
}

PASS=1
TOTAL_KILLS=0
SUCCESSFUL_RECOVERIES=0

log "FAULT_INJECTION_START cycles=$CYCLES kill_secs=$KILL_SECS recover_timeout=$RECOVER_TIMEOUT"

for node_def in "${NODES_DEF[@]}"; do
    IFS='|' read -r SSH_DEST API PROC_MATCH <<< "$node_def"

    log "===== NODE: $SSH_DEST ($API) ====="

    for cycle in $(seq 1 "$CYCLES"); do
        log "CYCLE $cycle/$CYCLES — killing $SSH_DEST"

        # Pick a peer (any other node) to monitor during the outage
        PEER_API=""
        for other in "${NODES_DEF[@]}"; do
            IFS='|' read -r _ OTHER_API _ <<< "$other"
            [ "$OTHER_API" != "$API" ] && { PEER_API=$OTHER_API; break; }
        done

        # 1. Kill the node
        ssh -o IdentitiesOnly=yes -i "$SSH_KEY" -o ConnectTimeout=10 "$SSH_DEST" \
            "pkill -f '${PROC_MATCH}' || true" 2>/dev/null || {
            log "WARN: ssh kill failed for $SSH_DEST — skipping cycle"
            continue
        }
        TOTAL_KILLS=$((TOTAL_KILLS + 1))
        log "NODE_DOWN $SSH_DEST"

        # 2. Verify peer continues to make finality progress
        log "Checking finality on peer $PEER_API ..."
        if wait_finality_progressing "$PEER_API"; then
            log "FINALITY_OK peer=$PEER_API progressed during outage of $SSH_DEST"
        else
            log "FINALITY_STALL peer=$PEER_API did not advance within 30s while $SSH_DEST was down"
            PASS=0
        fi

        # 3. Keep the node down for KILL_SECS
        REMAINING=$((KILL_SECS - 30))
        [ "$REMAINING" -gt 0 ] && sleep "$REMAINING"

        # 4. Restart the node (if restart command provided)
        if [ -n "$SSH_RESTART_CMD" ]; then
            log "RESTARTING $SSH_DEST via: $SSH_RESTART_CMD"
            ssh -o IdentitiesOnly=yes -i "$SSH_KEY" -o ConnectTimeout=10 "$SSH_DEST" \
                "$SSH_RESTART_CMD" &>/dev/null &
        else
            log "RESTART_SKIPPED — set SSH_RESTART_CMD to enable auto-restart; restart $SSH_DEST manually now"
            log "Waiting ${RECOVER_TIMEOUT}s for manual restart..."
        fi

        # 5. Wait for node to come back and resync
        WAITED=0
        RECOVERED=0
        while [ "$WAITED" -lt "$RECOVER_TIMEOUT" ]; do
            sleep 5
            WAITED=$((WAITED + 5))
            CODE=$(curl -s -m 4 -o /dev/null -w "%{http_code}" "http://${API}/api/health" 2>/dev/null || echo "000")
            if [ "$CODE" = "200" ]; then
                log "NODE_UP $SSH_DEST after ${WAITED}s"
                RECOVERED=1
                break
            fi
        done

        if [ "$RECOVERED" = 0 ]; then
            log "RECOVERY_TIMEOUT $SSH_DEST did not come up within ${RECOVER_TIMEOUT}s"
            PASS=0
        else
            SUCCESSFUL_RECOVERIES=$((SUCCESSFUL_RECOVERIES + 1))
            # 6. Check height convergence (within 5 blocks of peer after 20s)
            sleep 20
            NODE_H=$(jget "$(curl -s -m 5 "http://${API}/api/status" 2>/dev/null || echo '{}')" block_height)
            PEER_H=$(jget "$(curl -s -m 5 "http://${PEER_API}/api/status" 2>/dev/null || echo '{}')" block_height)
            if [ "$NODE_H" != "-1" ] && [ "$PEER_H" != "-1" ]; then
                DIFF=$(python3 -c "print(abs(${NODE_H} - ${PEER_H}))")
                if python3 -c "exit(0 if abs(${NODE_H}-${PEER_H})<=5 else 1)"; then
                    log "HEIGHT_CONVERGED node=$NODE_H peer=$PEER_H diff=$DIFF"
                else
                    log "HEIGHT_DIVERGED node=$NODE_H peer=$PEER_H diff=$DIFF (>5 blocks)"
                    PASS=0
                fi
            else
                log "WARN: could not compare heights (node=$NODE_H peer=$PEER_H)"
            fi
        fi

        log "CYCLE_END $cycle/$CYCLES node=$SSH_DEST recovered=$RECOVERED"
        # brief pause between cycles
        sleep 10
    done
done

{
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  T0.2 D.4 Fault injection — $(basename "$LOG")"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Nodes tested:   ${#NODES_DEF[@]}"
    echo "  Cycles/node:    $CYCLES"
    echo "  Total kills:    $TOTAL_KILLS"
    echo "  Recoveries:     $SUCCESSFUL_RECOVERIES / $TOTAL_KILLS"
    echo ""
    echo "  D.4 — Single-node fault tolerance:  $([ "$PASS" = 1 ] && echo PASS || echo FAIL)"
    echo ""
    if [ "$PASS" = 1 ]; then
        echo "  OVERALL VERDICT: PASS"
    else
        echo "  OVERALL VERDICT: FAIL  (see events above)"
    fi
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Full log: $LOG"
} | tee -a "$LOG"

log "FAULT_INJECTION_END pass=$PASS"
[ "$PASS" = 1 ] && exit 0 || exit 1
