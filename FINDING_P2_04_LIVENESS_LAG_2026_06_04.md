# FINDING: P2-04 DA-supermajority gate creates a self-stuck proposer at h=202

**Discovered:** 2026-06-04 in 3-Mini Tailscale colo bring-up #3 (post DA-BLS fix + post-cert mutation fix)
**Severity:** MEDIUM — chain progresses through 201 blocks then ONE validator gets stuck; ⅔ honest validators still advance until they need the third
**Status:** OPEN — root cause confirmed; fix candidates listed below; structural fix needs design decision
**Cluster:** 3-Mini Tailscale colo (M1=val1, M2=val2, M3=val3) on commit `6db4aca1`

## Summary

After the two prior CRITICAL bugs (DA-BLS verify regression + post-cert block mutation) were closed, the 3-Mini cluster advanced 201 blocks in lockstep with full BFT quorum. At h=201:

- M1 was the proposer (val-1)
- M2 and M3 committed block #201 (validator-1) by quorum
- **M1 itself did NOT commit block #201** — its own apply path was blocked by the P2-04 DA-supermajority gate (`crates/evaporchain-consensus/src/tendermint.rs:4744-4763` and `:5921-5933`)

M1 then permanently stalled at consensus state `h=201 r=0 phase=Propose`. h=202 traffic from M2/M3 flows into M1 as `[net-msg]` entries but does not advance M1's own consensus state machine. The cluster as a whole cycles h=202 rounds 0..9+ with precommit timeouts because the 3-validator BFT quorum requires all 3 (small-cluster mode permits 2-of-3 for DA but consensus precommit still needs 2-of-3 ACTIVE signers; M1 is silent at h=202).

## Evidence

From `.live-soak-diagnostics-2026-06-04-pass3/M1-node.log`:

```
INFO Advanced to next height height=201 epoch=200
INFO Created proposal height=201 round=0 txs=0 has_data_root=true
[node-1] [consensus] h=201 r=0 phase=Propose -> 3 action(s)
[node-1] [net-msg] h=201 r=0 type=DAAttestation       (peer attestation)
[node-1] [net-msg] h=201 r=0 type=Prevote             (peer prevote)
[node-1] [net-msg] h=201 r=0 type=DAAttestation
[node-1] [net-msg] h=201 r=0 type=DAAttestation
[node-1] [net-msg] h=201 r=0 type=Prevote
[node-1] [net-msg] h=201 r=0 type=DAAttestation
[node-1] [net-msg] h=201 r=0 type=Precommit
[node-1] [net-msg] h=201 r=0 type=Precommit
WARN P2-04: refusing to commit — DA attestation supermajority not reached (msg path) height=201
[node-1] [net-msg] h=201 r=0 type=DAAttestation       (arrives AFTER P2-04 fires)
[node-1] [net-msg] h=201 r=0 type=DAAttestation
[node-1] [net-msg] h=202 r=0 type=DAAttestation       (peers already ahead at h=202)
[node-1] [net-msg] h=202 r=0 type=Proposal
... (no further h=201 [consensus] lines; no Advanced to next height for h=202)
```

Critical signals:
- "Advanced to next height" log appears **200 times** in M1's log (h=2 through h=201) — M1 reached h=201 consensus state and never advanced past it
- "Block #201 ... COMMITTED" log NEVER appears in M1's log
- `[consensus] h=N` log entries only show h=201 r=0 phase=Propose for M1's terminal state
- "P2-04: refusing to commit" warns EXACTLY ONCE — but the warn-suppression dedup (by `(height, round)`) means subsequent ticks could re-check silently. Yet the chain never advances, so either ticks are silently retrying-and-failing forever, OR M1's phase has been somehow moved.

M2 and M3's logs both show "Block #201 ... COMMITTED (validator-1)" — they had local DA quorum and applied.

## Root cause

The P2-04 gate (`tendermint.rs:4744-4763` for tick-path, `:5921-5933` for msg-path) refuses to commit when `enforce_da && data_root.is_some() && !has_da_supermajority(block.number)`. Once `da_enforcement_height` is crossed (default 201), the gate is hard and the validator stays in Precommit phase until DA supermajority is locally observed.

The race that traps M1:

1. M1 proposes block 201 with `data_root.is_some()`.
2. M2 + M3 receive the proposal, attest DA, prevote, precommit.
3. M2 + M3 gossip their DA attestations; some arrive at M1, but not all 2-of-3 stake-weighted before M1's local precommit quorum check fires.
4. M1's per-msg precommit handler (`:5902-5951`) sees 2f+1 precommits + block_hash matches + ... but `!has_da_supermajority(201)` → P2-04 returns + keeps `proposed_block`.
5. The remaining DA attestations arrive at M1 after the per-msg precommit quorum check fires. **Receiving a DA attestation does NOT re-trigger the precommit commit check** — only a new Precommit message or a tick does.
6. No more Precommit messages arrive for h=201 (all 3 validators have already precommitted; the gossip mesh deduplicates).
7. Ticks should retry the Precommit-phase check (`:4701-4778`), and by then DA supermajority is reached. **But empirically, no retry happens.** Either the tick-retry path is dead, or the consensus phase has been moved off Precommit, or `has_da_supermajority(201)` still returns false even after the additional attestations.

Additionally, M2 and M3 advance to h=202 immediately. Their h=202 traffic reaches M1 but does not influence M1's stuck h=201 state. M1 has no mechanism to detect "I'm a stuck proposer, peers committed my block, I should state-sync the canonical h=201 from a peer."

## Why this is MEDIUM, not CRITICAL

- 2-of-3 honest validators still commit and advance (M2 + M3). The chain itself does not fork or roll back.
- The stuck validator (M1) is the proposer of the affected block, which is a fixed-frequency rotational event in 3-validator clusters — every 3 blocks, the next instance of this race could trap a different validator.
- In a 4+ validator mainnet cluster, BFT quorum is 2f+1 = 3-of-4 (one stuck validator does not block progress). The damage is bounded to the stuck validator's own state — recoverable via restart + state-sync.
- The bug is observed in the small-cluster DA mode auto-enabled for validators ≤ 3. Mainnet validator set will be > 4, so the small-cluster mode would not engage.

## Fix candidates (need design decision)

1. **DA-attestation receive triggers commit re-check.** When a new DA attestation arrives at the message handler, after storing, check whether the current `round_state.phase == Precommit` and `proposed_block.is_some()` and quorum is reached and now DA supermajority is reached → fall through to commit immediately. Minimal-touch fix, well-scoped.

2. **Stuck-proposer state-sync recovery.** When a validator's `(height, phase)` has been unchanged for N ticks AND peers are observed at `height + k` (k ≥ 1), trigger a `RequestSync(height, height + 1)` to pull the committed block from a peer. Generalizes beyond P2-04 — handles other liveness corner-cases too.

3. **Round-robin proposer health: skip self when stuck.** If a validator is the proposer at height H and round 0 fails to commit locally despite peers committing, skip rounds 1+ and request sync. Avoids hammering rounds 1..9 with timeouts.

4. **Raise da_enforcement_height for soak clusters.** Tactical mitigation only — sets the gate's activation point past the soak window. Doesn't fix the underlying race; just delays it. Not appropriate as a permanent solution.

Recommendation: **(1) is the right immediate fix.** It's a 5-10 line change, well-scoped, addresses the exact race, doesn't introduce new liveness assumptions. Combined with (2) as defense-in-depth for future stuck-state scenarios.

## Operational state at finding time

- All 3 Minis ran the binary built from `6db4aca1`
- Cluster ran ~10 minutes total (h=0 → h=201 in ~2 min, then ~8 min stalled on h=202 round-cycling)
- No conservation violations (201 consecutive clean conservation audits on M2/M3, 200 on M1)
- Zero cert-vs-actual-hash mismatch, zero parent-hash mismatch, zero DA verify failures across all 3 nodes — confirms the prior two fixes (`ceb95025` + `6db4aca1`) stayed closed throughout
- Diagnostic logs: `.live-soak-diagnostics-2026-06-04-pass3/M{1,2,3}-node.log` (~800kB each)

## Lane impact

- **T3.1**: still 🟡 PARTIAL — the cluster has clearly proven that the colo path can sustain consensus once non-determinism + signing bugs are out of the way. This last issue is a self-bounded liveness corner-case in 3-validator clusters.
- **T0.6 / T0.2 / T1.17 / T1.18 / T1.19 / T1.23**: still unblocked at the bug level. The h=202 stall is recoverable (restart M1 → state-sync from peers → cluster resumes). Live-cluster soak can proceed with the recovery procedure documented; the structural fix lands separately.
- **Mainnet relevance**: small-cluster DA mode (validators ≤ 3) is explicitly NOT FOR MAINNET (see the `Auto-enabling small-cluster DA mode` warn line). Mainnet's validator set is 4+ where the bug's blast radius is bounded. **This finding is a soak-cluster issue, not a mainnet ship-blocker.**

## Companion artifacts

- This finding doc: `FINDING_P2_04_LIVENESS_LAG_2026_06_04.md`
- Saved logs: `.live-soak-diagnostics-2026-06-04-pass3/M{1,2,3}-node.log` (gitignored)
- Reusable launch script: `scripts/launch-colo-3node-cluster.sh`
- Prior fixes that this finding sits on top of: `ceb95025` (DA-BLS verify), `6db4aca1` (post-cert mutation)
