#!/bin/bash
# Per-Mini wipe-and-restart for the 3-Mini Tailscale cluster.
#
# Used to switch consensus modes (e.g. mock-prove → prove) where the
# new binary's verifier won't accept block data produced by the old
# binary. Preserves bls_key.bin (needed for the validator's identity)
# but wipes everything else in the data dir.
#
# Usage on each Mini:
#   ./scripts/restart-tailscale-3node.sh <validator-id-1-2-or-3>

set -e

VID="$1"
if [[ "$VID" != "1" && "$VID" != "2" && "$VID" != "3" ]]; then
    echo "Usage: $0 <validator-id-1-2-or-3>"
    exit 1
fi

DATA_DIR="$HOME/.evaporchain-tailscale-data"
PID_FILE="$HOME/evaporchain-v$VID.pid"
LOG_FILE="$HOME/evaporchain-v$VID.log"

# Step 1: stop the running node (if any).
if [ -f "$PID_FILE" ]; then
    OLD_PID=$(cat "$PID_FILE")
    if ps -p "$OLD_PID" > /dev/null 2>&1; then
        echo "Stopping validator $VID (PID $OLD_PID)..."
        kill "$OLD_PID" 2>/dev/null || true
        # Give it a moment to flush state before SIGKILL.
        for i in 1 2 3 4 5; do
            ps -p "$OLD_PID" > /dev/null 2>&1 || break
            sleep 1
        done
        if ps -p "$OLD_PID" > /dev/null 2>&1; then
            kill -9 "$OLD_PID" 2>/dev/null || true
        fi
    fi
    rm -f "$PID_FILE"
fi
# Belt-and-braces: kill any stray validator-N process the PID file
# missed.
pkill -f "validator-id $VID" 2>/dev/null || true
sleep 1

# Step 2: preserve bls_key.bin, wipe rest of data dir.
if [ -f "$DATA_DIR/bls_key.bin" ]; then
    cp "$DATA_DIR/bls_key.bin" "$HOME/.bls_key.bin.tmp"
    chmod 600 "$HOME/.bls_key.bin.tmp"
fi
echo "Wiping $DATA_DIR (preserving bls_key.bin)..."
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"
if [ -f "$HOME/.bls_key.bin.tmp" ]; then
    mv "$HOME/.bls_key.bin.tmp" "$DATA_DIR/bls_key.bin"
    chmod 600 "$DATA_DIR/bls_key.bin"
fi

# Step 3: rotate the log so we don't conflate runs.
if [ -f "$LOG_FILE" ]; then
    mv "$LOG_FILE" "$LOG_FILE.$(date +%Y%m%dT%H%M%S).bak"
fi

# Step 4: re-launch via the canonical launcher.
exec "$(dirname "$0")/launch-tailscale-3node.sh" "$VID"
