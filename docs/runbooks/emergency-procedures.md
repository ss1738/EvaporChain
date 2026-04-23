# Emergency Procedures Runbook

## Consensus Stalled (no new blocks)

**Alert**: `ConsensusStalled` — no blocks in 5 minutes

1. Check all validator logs: `ssh <validator> journalctl -u evaporchain -n 100`
2. Check peer counts: `curl http://<validator>:8080/readyz`
3. If peer count = 0: network issue. Check firewall, Tailscale/VPN status.
4. If peers connected but no blocks: check for consensus round timeouts in logs.
5. Restart validators one at a time (never all at once): `systemctl restart evaporchain`
6. If still stalled after restart: check if validators have diverged. Compare state roots.

## Validator Down

**Alert**: `ValidatorDown` — instance unreachable for 1+ minute

1. SSH to the machine. Check process: `systemctl status evaporchain`
2. Check disk: `df -h /data` — if >90% full, run backup + prune.
3. Check OOM: `dmesg | grep -i oom`
4. Restart: `systemctl restart evaporchain`
5. Verify recovery: `curl http://localhost:8080/readyz`
6. Check it catches up: block height should climb within 30 seconds.

## State Corruption

Symptoms: node crashes on startup with RocksDB errors, or produces blocks with wrong state root.

1. Stop the node: `systemctl stop evaporchain`
2. Check if backup exists: `ls /data/backups/`
3. Restore from latest backup:
   ```bash
   rm -rf /data/evaporchain/state /data/evaporchain/chain
   cp -r /data/backups/snapshot_LATEST/state /data/evaporchain/state
   cp -r /data/backups/snapshot_LATEST/chain /data/evaporchain/chain
   ```
4. Restart: `systemctl start evaporchain`
5. Node will sync missing blocks from peers.

## Network Partition

Symptoms: some validators see different block heights, gossip messages not reaching all nodes.

1. Check each validator's peer list: `curl http://<v>:8080/api/status`
2. Ensure all validators can reach each other on port 26656.
3. If using Tailscale: `tailscale status` on each machine.
4. Tendermint BFT tolerates f < n/3 failures. With 4 validators, 1 can be partitioned.
5. If >1 partitioned: blocks will stop. Fix network before restarting.

## Disk Space Critical (>90%)

**Alert**: `DiskSpaceCritical`

1. Run backup: `./scripts/backup-state.sh /data/evaporchain /data/backups 5`
2. Check for old logs: `journalctl --vacuum-size=500M`
3. If RocksDB state is large: trigger compaction via API (if available) or restart node.
4. Consider expanding disk or adding storage.
