#!/bin/bash
# EvaporChain Testnet Health Check
# Queries all 4 validators and verifies consensus

set -e

PORTS=(18001 18002 18003 18004)
echo "╔══════════════════════════════════════════════════╗"
echo "║  EvaporChain Testnet Health Check                ║"
echo "╚══════════════════════════════════════════════════╝"
echo ""

# Collect status from all nodes
max_height=0
max_root=""
nodes_at_max=0
alive=0

for i in "${!PORTS[@]}"; do
    port=${PORTS[$i]}
    vid=$((i + 1))
    data=$(curl -s -m 2 http://localhost:${port}/api/status 2>/dev/null) || true

    if [ -z "$data" ] || echo "$data" | grep -q "error\|detail"; then
        echo "  Validator $vid (:${port})  DOWN"
        continue
    fi

    height=$(echo "$data" | python3 -c "import sys,json;print(json.load(sys.stdin)['block_height'])" 2>/dev/null)
    epoch=$(echo "$data" | python3 -c "import sys,json;print(json.load(sys.stdin)['epoch'])" 2>/dev/null)
    peers=$(echo "$data" | python3 -c "import sys,json;print(json.load(sys.stdin)['peer_count'])" 2>/dev/null)
    root=$(echo "$data" | python3 -c "import sys,json;print(json.load(sys.stdin)['state_root'])" 2>/dev/null)
    objects=$(echo "$data" | python3 -c "import sys,json;print(json.load(sys.stdin)['active_objects'])" 2>/dev/null)
    ghosts=$(echo "$data" | python3 -c "import sys,json;print(json.load(sys.stdin)['ghost_count'])" 2>/dev/null)

    alive=$((alive + 1))
    echo "  Validator $vid (:${port})  Block: $height  Epoch: $epoch  Peers: $peers  Objects: $objects  Ghosts: $ghosts"

    if [ "$height" -gt "$max_height" ] 2>/dev/null; then
        max_height=$height
        max_root=$root
        nodes_at_max=1
    elif [ "$height" -eq "$max_height" ] 2>/dev/null; then
        if [ "$root" = "$max_root" ]; then
            nodes_at_max=$((nodes_at_max + 1))
        fi
    fi
done

echo ""
echo "─────────────────────────────────────────────────"
echo "  Nodes alive:     $alive / 4"
echo "  Max block:       $max_height"
echo "  Nodes at max:    $nodes_at_max"
if [ "$nodes_at_max" -ge 3 ]; then
    echo "  State root:      ${max_root:0:32}..."
    echo "  Status:          CONSENSUS OK (${nodes_at_max}/4 agree)"
elif [ "$max_height" -gt 0 ]; then
    echo "  Status:          PROGRESSING (nodes catching up)"
else
    echo "  Status:          NO BLOCKS YET"
fi
echo "─────────────────────────────────────────────────"
