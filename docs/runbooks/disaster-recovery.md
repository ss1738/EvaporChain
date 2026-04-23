# Disaster Recovery Plan

## Recovery Objectives
- **RTO (Recovery Time Objective)**: 15 minutes for single node, 30 minutes for full network
- **RPO (Recovery Point Objective)**: Last backup snapshot (recommended: every 15 minutes)

## Scenario 1: Single Validator Failure

**Impact**: Network continues (BFT tolerates f < n/3). No data loss.

1. Provision a replacement machine with same specs.
2. Install binary and genesis config.
3. Start node with `--bootstrap` pointing to healthy validators.
4. Node automatically syncs missing blocks from peers.
5. Re-register as validator if using on-chain registration.

**Recovery time**: 5-10 minutes.

## Scenario 2: Majority Validator Failure (>1/3 down)

**Impact**: Block production stops. No new transactions processed.

1. Identify surviving validators — they hold the authoritative state.
2. For each failed validator:
   - If machine is recoverable: restart the process.
   - If machine is lost: restore from latest backup on new hardware.
3. Start surviving validators first, then recovered ones.
4. Once 2/3+ validators are online, consensus resumes automatically.

**Recovery time**: 15-30 minutes depending on hardware provisioning.

## Scenario 3: Complete Network Loss

**Impact**: All validators down. Full state reconstruction needed.

1. Provision new machines (minimum 4 for BFT).
2. Restore latest backup on at least one node.
3. Start that node as the seed.
4. Start remaining nodes with `--bootstrap` pointing to the seed.
5. They will sync state from the seed node.
6. Once all validators are caught up, consensus resumes.

**Recovery time**: 30-60 minutes.

## Scenario 4: State Corruption (Byzantine failure)

**Impact**: One or more validators have incorrect state.

1. Stop ALL validators immediately.
2. Identify the last known-good block height from logs/metrics.
3. On each validator, restore the backup closest to (but not after) that height.
4. Restart all validators simultaneously.
5. Investigate root cause before re-enabling the corrupted validator.

## Backup Schedule

| Environment | Frequency | Retention | Method |
|---|---|---|---|
| Dev | Manual | 3 snapshots | `scripts/backup-state.sh` |
| Staging | Every 30min | 10 snapshots | cron + backup script |
| Production | Every 15min | 48 snapshots (12 hours) | cron + backup script + offsite copy |

### Cron example (production)
```cron
*/15 * * * * /opt/evaporchain/scripts/backup-state.sh /data/evaporchain /data/backups 48 >> /var/log/evaporchain-backup.log 2>&1
```

## Testing
- Quarterly: test full restore from backup on staging.
- Monthly: test single-node recovery on staging.
- After every binary upgrade: verify backups are compatible.
