#!/bin/bash
# status.sh — Check status of all 3 EvaporChain testnet nodes.

set -uo pipefail

declare -A NODES=(
    [1]="satyawan|100.119.53.101|8081"
    [2]="ironman|100.103.216.125|8082"
    [3]="apsarth|100.113.253.72|8083"
)

echo "=== EvaporChain Testnet Status ==="
echo ""

for i in 1 2 3; do
    IFS='|' read -r name ip port <<< "${NODES[$i]}"
    url="http://${ip}:${port}/api/status"
    echo "--- Node ${i}: ${name} (${ip}:${port}) ---"

    response=$(curl -s --connect-timeout 3 --max-time 5 "${url}" 2>/dev/null)
    if [[ $? -ne 0 || -z "$response" ]]; then
        echo "  STATUS: UNREACHABLE"
        echo ""
        continue
    fi

    if command -v jq &>/dev/null; then
        block_height=$(echo "$response" | jq -r '.block_height // "N/A"')
        peer_count=$(echo "$response" | jq -r '.peer_count // "N/A"')
        epoch=$(echo "$response" | jq -r '.epoch // "N/A"')
        uptime=$(echo "$response" | jq -r '.uptime_seconds // "N/A"')
        state_root=$(echo "$response" | jq -r '.state_root // "N/A"')
        echo "  Block Height: ${block_height}"
        echo "  Epoch:        ${epoch}"
        echo "  Peer Count:   ${peer_count}"
        echo "  Uptime:       ${uptime}s"
        echo "  State Root:   ${state_root}"
    else
        echo "  ${response}"
        echo "  (Install jq for formatted output)"
    fi
    echo ""
done
