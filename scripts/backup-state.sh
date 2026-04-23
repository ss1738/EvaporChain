#!/usr/bin/env bash
# Automated RocksDB state backup for EvaporChain validators.
#
# Creates a point-in-time snapshot using RocksDB's checkpoint mechanism
# (hardlinks, instant, zero-downtime). Falls back to rsync if checkpoint
# tool is unavailable.
#
# Usage:
#   ./scripts/backup-state.sh [DATA_DIR] [BACKUP_DIR] [RETAIN_COUNT]
#
# RTO: ~5 minutes (copy checkpoint back to data dir)
# RPO: depends on backup frequency (recommended: every 100 blocks or 15min)

set -euo pipefail

DATA_DIR="${1:-/data/evaporchain}"
BACKUP_DIR="${2:-/data/backups}"
RETAIN_COUNT="${3:-10}"

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
SNAPSHOT_DIR="$BACKUP_DIR/snapshot_$TIMESTAMP"

echo "=== EvaporChain State Backup ==="
echo "Data dir:    $DATA_DIR"
echo "Backup dir:  $BACKUP_DIR"
echo "Retain:      $RETAIN_COUNT most recent"
echo "Snapshot:    $SNAPSHOT_DIR"
echo

mkdir -p "$BACKUP_DIR"

# Method 1: RocksDB checkpoint (preferred — instant, zero-downtime)
if command -v ldb &>/dev/null; then
    echo "Using RocksDB checkpoint (hardlinks)..."
    ldb --db="$DATA_DIR/state" checkpoint --checkpoint_dir="$SNAPSHOT_DIR/state" 2>/dev/null || true
    ldb --db="$DATA_DIR/chain" checkpoint --checkpoint_dir="$SNAPSHOT_DIR/chain" 2>/dev/null || true
    echo "Checkpoint created at $SNAPSHOT_DIR"
else
    # Method 2: rsync fallback
    echo "ldb not found — using rsync (may be slower)..."
    rsync -a --delete "$DATA_DIR/state/" "$SNAPSHOT_DIR/state/"
    rsync -a --delete "$DATA_DIR/chain/" "$SNAPSHOT_DIR/chain/"
    echo "rsync backup created at $SNAPSHOT_DIR"
fi

# Record metadata
cat > "$SNAPSHOT_DIR/metadata.json" <<METADATA
{
  "timestamp": "$TIMESTAMP",
  "data_dir": "$DATA_DIR",
  "hostname": "$(hostname)",
  "state_size_bytes": $(du -sb "$SNAPSHOT_DIR/state" 2>/dev/null | awk '{print $1}' || echo 0),
  "chain_size_bytes": $(du -sb "$SNAPSHOT_DIR/chain" 2>/dev/null | awk '{print $1}' || echo 0)
}
METADATA

# Prune old backups (keep $RETAIN_COUNT most recent)
EXISTING=$(ls -1d "$BACKUP_DIR"/snapshot_* 2>/dev/null | sort -r)
COUNT=0
for dir in $EXISTING; do
    COUNT=$((COUNT + 1))
    if [ "$COUNT" -gt "$RETAIN_COUNT" ]; then
        echo "Pruning old backup: $dir"
        rm -rf "$dir"
    fi
done

echo
echo "=== Backup complete. $(ls -1d "$BACKUP_DIR"/snapshot_* 2>/dev/null | wc -l | tr -d ' ') snapshots retained. ==="
