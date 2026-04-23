# Network Upgrade Runbook

## Pre-Upgrade Checklist
- [ ] New binary tested on staging with all existing tests passing
- [ ] Genesis config compatible (no breaking changes to format)
- [ ] All validators have the new binary available
- [ ] Backup taken on all validators: `./scripts/backup-state.sh`
- [ ] Upgrade window communicated to all operators

## Rolling Upgrade (no consensus changes)

For non-consensus binary updates (API changes, bug fixes, performance):

1. Upgrade one validator at a time, starting with the lowest-stake one.
2. On each validator:
   ```bash
   systemctl stop evaporchain
   cp /usr/local/bin/evaporchain-node /usr/local/bin/evaporchain-node.bak
   cp evaporchain-node-NEW /usr/local/bin/evaporchain-node
   systemctl start evaporchain
   ```
3. Wait for the node to sync and produce a block before moving to the next.
4. Monitor `/readyz` and `/metrics` throughout.

## Hard Fork (consensus changes)

For changes that affect block/state format, consensus rules, or genesis:

1. Agree on a **halt height** — all validators stop at block N.
2. Configure halt: set environment variable `HALT_HEIGHT=N` (if supported) or monitor and stop manually.
3. Once all validators are stopped at the same height:
   ```bash
   # On each validator
   systemctl stop evaporchain
   cp evaporchain-node-NEW /usr/local/bin/evaporchain-node
   # If new genesis needed:
   cp new-genesis.json /etc/evaporchain/genesis.json
   systemctl start evaporchain
   ```
4. All validators must restart within the consensus timeout window.
5. Verify all validators are on the new version: check `/api/status` for version info.

## Rollback

If the upgrade causes issues:

1. Stop the problematic validator: `systemctl stop evaporchain`
2. Restore old binary: `cp /usr/local/bin/evaporchain-node.bak /usr/local/bin/evaporchain-node`
3. If state is incompatible: restore from pre-upgrade backup.
4. Restart: `systemctl start evaporchain`
