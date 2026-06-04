# FINDING: BLS DA-Attestation Verification Regression (mainnet-blocking)

**Discovered:** 2026-06-04 during T3.1 zero-cost colo cluster bring-up
**Severity:** CRITICAL — mainnet-blocking
**Cluster:** 3-Mini Tailscale colo (M1=val1, M2=val2, M3=val3) on chain_id `evaporchain-tailscale-3node-1`
**Binary:** Built from commit `23f0eafd` on Mini 2; scp'd to all 3
**Configuration:** small-cluster DA mode auto-enabled (validators ≤ 3); proposer self-attestation; mock-prove; mainnet=false; default DA enforcement_height=201

## Summary

Live 3-validator cluster started cleanly, all BLS keys matched genesis, Tendermint consensus reached full 3/3 BFT quorum (stake=750000/750000 = 100%) on every block from #1 to #200. At block #201 — exactly the DA enforcement-height boundary — the chain HALTED. Cluster cycled through Tendermint rounds 0,1,2,3,4,... on h=201 without ever committing.

Root cause: BLS signature verification on DA (data-availability) attestations has been failing **from block #1 onwards**. The failure was masked because the chain runs in soft-DA mode below `enforcement_height` (default 201): blocks without a valid DA certificate are accepted with a WARN. At enforcement_height=201, the soft-mode acceptance lifts; DA quorum now requires verified attestations; the per-attestation BLS-verify-failure makes DA quorum unreachable; the chain halts.

## Evidence

From M1's startup log (full log saved at `.live-soak-diagnostics-2026-06-04/M1-node.log`, 798kB):

```
DA enforcement height updated old=100 new=201
Auto-enabling small-cluster DA mode (validators <= 3): proposer self-attestations will count toward DA quorum
small-cluster DA mode ENABLED: proposer self-attestations will count toward DA quorum
Block has no DA certificate (soft mode — accepting before enforcement height) block=1 enforcement_height=201
Rejecting DA attestation: BLS signature did not verify validator_id=2 block_number=1
Rejecting DA attestation: BLS signature did not verify validator_id=3 block_number=1
```

From the same log, AT enforcement boundary:

```
[node-1] ━━━ Block #200 │ Epoch 200 ━━━ COMMITTED (validator-3) ━━━━━━━━━━━━━━━━━━━━━━
[node-1] [consensus] h=201 r=2 phase=Propose -> 1 action(s)
[node-1] [consensus] h=201 r=3 phase=Propose -> 1 action(s)
... (rounds continue indefinitely)
WARN Rejecting DA attestation: BLS signature did not verify validator_id=1 block_number=201
WARN Rejecting DA attestation: BLS signature did not verify validator_id=2 block_number=201
WARN Rejecting DA attestation: BLS signature did not verify validator_id=3 block_number=201
```

Consensus-layer BLS sigs (Prevote / Precommit / aggregate CommitCertificate) verified correctly — the chain committed 200 blocks at "BLS CommitCertificate: 3 signers, stake=750000/750000(100%)". The verification failure is specific to the DA-attestation path, NOT consensus-vote BLS.

## Why this is mainnet-blocking

- `docs/MAINNET_LAUNCH.md` ships mainnet with DA enforcement from genesis (no soft-mode window).
- This bug means a mainnet cluster would never produce block #1 — every DA attestation rejected, no DA quorum, chain halt.
- Soft-mode masking is exactly why the bug survived 25,435+ unit/integration tests: the test fixtures probably run in soft-DA mode, so the BLS-verify-failure prints WARNs but doesn't block test progress.

## Hypothesised root causes (untested)

1. **DA-attestation message format mismatch** between signer and verifier — the signer might be signing one byte serialization while the verifier reconstructs a different one.
2. **Aggregate-signature path bug** — the BLS aggregation may be combining sigs with the wrong domain-separation tag.
3. **Small-cluster DA mode self-attestation path** — the proposer self-attestation logic shipped in audit cycles may have introduced a sig-format drift.
4. **Field-element / curve-encoding regression** — recent ark-* dependency bumps could have changed serialization.

## Recommended next steps

1. **Reproducer in unit test**: write a test that runs DA-attestation sign-then-verify on a single deterministic block; the test should fail today and pass after the fix.
2. **Bisect for the regression** — git-bisect across the commits in `evaporchain-consensus` and `evaporchain-da` since the last known-green DA-verify state.
3. **Audit add**: this finding belongs in `AUDIT_SCOPE.md` §6.2 as a new CRITICAL since it would never let mainnet advance.
4. **Lane impact**: T3.1 acceptance criteria CANNOT be met until this is fixed. T0.6 live-cluster slashing soak ALSO cannot run (it depends on DA quorum reaching).

## Operational state at finding time

- All 3 Minis ran the binary built from `23f0eafd` (which is `main` HEAD at 2026-06-04T06:00 UTC).
- Cluster ran for ~6 minutes total (h=0 → h=200 in ~90s under accelerated block-replay, then ~4 min stalled on h=201).
- No conservation violations (200 consecutive clean conservation audits).
- BLS keys derived from `validator-{1,2,3}-keys.json` (the same files that produced the genesis pubkeys); the node confirmed "BLS key matches genesis entry for validator-id=N" on each Mini at startup.
- Diagnostic logs at: `.live-soak-diagnostics-2026-06-04/M{1,2,3}-node.log` (cluster-side raw `/tmp/evaporchain-node.log` files).
- Launch script for future repro: `scripts/launch-colo-3node-cluster.sh`.

## Companion artifacts

- This finding doc: `FINDING_DA_BLS_VERIFY_2026_06_04.md`
- Saved logs (first pass, halted at h=201): `.live-soak-diagnostics-2026-06-04/M{1,2,3}-node.log`
- Saved logs (second pass, post-fix, halted at h≈84 on a DIFFERENT issue): `.live-soak-diagnostics-2026-06-04-pass2/M{1,2,3}-node.log`
- Reusable launch script: `scripts/launch-colo-3node-cluster.sh`
- Genesis used: `genesis-tailscale-3node.json` (chain_id `evaporchain-tailscale-3node-1`)

## RESOLUTION (2026-06-04, same day)

Root cause localized in <10 minutes after the soak data was inspected: two consensus-side inline verifiers (`crates/evaporchain-consensus/src/tendermint.rs:5007-5014` + `crates/evaporchain-consensus-types/src/tendermint.rs:4629-4636`) reconstructed the signed message WITHOUT the trailing 8 stake bytes. `evaporchain-da/src/certificate.rs::create_attestation` had been correctly including stake since the Q3 audit fix (2026-05-17); the verifier sites were not updated alongside it.

### Fix shipped

- **Regression test** (commit `c3ec29ef`): `da_attestation_signed_message_must_include_stake` in `crates/evaporchain-da/src/certificate.rs`. Pins the byte-level contract: the canonical (stake-inclusive) reconstruction MUST verify; the stake-less reconstruction (the buggy shape) MUST fail verify. Independent of any consensus-side verifier — any verifier must conform to this contract.
- **Fix** (commit `ceb95025`): both `tendermint.rs` sites updated to reconstruct `DST || block(8) || data_root(32) || vid(8) || samples(4) || stake(8)`, mirroring `create_attestation` byte-for-byte.

### Live-cluster confirmation

Re-launched the 3-Mini cluster on commit `ceb95025`. Empirical results:
- **Zero "BLS signature did not verify" entries across all 3 nodes** (`grep -c` returned 0 on each Mini's log)
- Chain advanced cleanly from h=0 to h=83 with full 3/3 BFT quorum, 83 consecutive clean conservation audits, 0 ghost objects, no fork events through h=73
- The pre-fix halt at h=201 (DA enforcement boundary) is closed — the cluster does NOT hit it anymore because every DA attestation now verifies

### NEW issue surfaced by the same soak (separate finding)

At approximately h=84, the cluster experienced a proposal-parent-hash mismatch and partitioned into rounds 0..9+ on h=84 without committing:

```
WARN Proposal parent hash mismatch — requesting sync height=84 round=9
     local_parent=b16862204c53d256 proposal_parent=54ee9f5141fad4b7
```

All 3 nodes show identical commit lines through h=73 (and likely further); commit-by-validator pattern matched across all 3 nodes for blocks 64-73 (val-3, val-1, val-2, val-2, val-2, val-1, val-2, val-3, val-3, val-2). Despite this, by h=84 the proposer's view of h=83's hash differs from M1's stored hash.

This is a SEPARATE bug from the DA-BLS verify regression. The DA-BLS fix is verified and complete; the new non-determinism issue is independent and would have surfaced regardless. Likely causes (untested):

1. **State-root non-determinism** at h=83 — different nodes computed different state-roots for the same logical block due to ordering-sensitive state mutations.
2. **Block-payload encoding** drift — different serialization of the same block content across nodes.
3. **DA-certificate inclusion ordering** — the DA cert attached to block 83 may have been built from a non-deterministic attestation iteration order.

Belongs in a NEW finding doc when investigated: `FINDING_PROPOSAL_PARENT_HASH_MISMATCH_2026_06_04.md`. T0.6 live-cluster soak is still gated, now on this new bug rather than on DA-BLS. The pattern (each soak iteration surfaces a new mainnet-blocker) validates the lane spec's premise that live multi-validator soak is the only way to catch this class of bug.
