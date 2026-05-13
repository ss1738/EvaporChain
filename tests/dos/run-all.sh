#!/usr/bin/env bash
# tests/dos/run-all.sh — T0.7 cluster acceptance runner.
# Requires T3.1 (Phase C cluster deploy). All vectors run sequentially.
# Usage: tests/dos/run-all.sh --target <host:port> --duration <Nh>
set -euo pipefail

TARGET=""
DURATION="1h"

while [ $# -gt 0 ]; do
    case "$1" in
        --target)   TARGET="$2";   shift 2 ;;
        --duration) DURATION="$2"; shift 2 ;;
        *) echo "Unknown: $1"; exit 1 ;;
    esac
done
[ -z "$TARGET" ] && { echo "--target required"; exit 1; }

echo "=== T0.7 DoS Acceptance Suite ==="
echo "Target: $TARGET  Duration: $DURATION"
echo ""

echo "--- Vector 1: tx flood ---"
scripts/dos-flood.sh --target "$TARGET" --rate 1000   --duration "$DURATION"
scripts/dos-flood.sh --target "$TARGET" --rate 10000  --duration "$DURATION"
scripts/dos-flood.sh --target "$TARGET" --rate 100000 --duration "$DURATION"

echo "--- Vector 2: signature storm ---"
scripts/dos-flood.sh --target "$TARGET" --rate 1000 --duration "$DURATION" --garbage-sigs

echo "--- Vector 3: single-sender exhaustion ---"
scripts/dos-flood.sh --target "$TARGET" --rate 1000 --duration "$DURATION" --single-sender

echo "=== All vectors complete. Review logs per dos-resistance.md ==="
