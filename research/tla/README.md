# EvaporChain TLA+ Formal Specification

Formal model of EvaporChain's Tendermint BFT consensus with energy decay,
Rule-Based Consensus (BlockStateCommitment), and DA sampling.

## Files

- `EvaporChainBFT.tla` — Main specification
- `EvaporChainBFT.cfg` — 3-node honest model (matches live cluster). MaxRound=3, MaxHeight=3.
- `EvaporChainBFT_Tiny.cfg` — Minimal model (3 nodes, MaxRound=1, MaxHeight=2). Completes fast (~1.5 min with deadlock detection); use for quick iteration on safety properties.
- `EvaporChainBFT_Small.cfg` — Mid-size honest model (3 nodes, MaxRound=2, MaxHeight=2). Fits in memory; completes in minutes.
- `EvaporChainBFT_Byzantine.cfg` — 4-node model with 1 Byzantine validator (n=4, f=1). Slowest config; reserve for V1-prep verification.

## What's modeled

- Full Tendermint round: Propose → Prevote → Precommit → Commit
- Lock mechanism (locked_block / locked_round)
- Quorum logic: 2f+1 = (2n+2)/3
- Timeout-driven round advancement
- Max-round forced empty block commit
- Round-skip on future-round messages
- Byzantine validators (arbitrary votes, equivocating proposals)
- Equivocation detection and slashing
- BlockStateCommitment in every block header
- DA sampling attestation tracking

## Safety properties (invariants)

1. **Agreement** — No two honest validators commit different blocks at the same height
2. **Validity** — Only proposed blocks are committed
3. **CommitRequiresQuorum** — Commit requires 2f+1 precommits
4. **LockSafety** — Locks prevent conflicting quorums across rounds
5. **EquivocationDetected** — Double-signing is always caught and slashed
6. **StateCommitmentIntegrity** — Every committed block has a state commitment

## Liveness properties (temporal)

1. **Progress** — Honest validators at the same height eventually advance
2. **DAEventualAttestation** — Committed blocks eventually get 2f+1 DA attestations

## Running

Install TLA+ tools (tla2tools.jar) then:

```bash
# Tiny model (3 nodes, MaxRound=1, MaxHeight=2 — fastest, ~1.5 min with deadlock checking)
java -jar tla2tools.jar -config EvaporChainBFT_Tiny.cfg EvaporChainBFT.tla

# Small model (3 nodes, MaxRound=2, MaxHeight=2 — minutes)
java -jar tla2tools.jar -config EvaporChainBFT_Small.cfg EvaporChainBFT.tla

# Default 3-node model (matches live cluster — minutes to hours)
java -jar tla2tools.jar -config EvaporChainBFT.cfg EvaporChainBFT.tla

# Byzantine 4-node model (slower, ~hours)
java -jar tla2tools.jar -config EvaporChainBFT_Byzantine.cfg EvaporChainBFT.tla
```

Or use the TLA+ Toolbox IDE / VS Code TLA+ extension.

## On TLC "deadlock" reports (resolved 2026-05-05)

All four .cfg files set `CHECK_DEADLOCK FALSE`. Every action in
`EvaporChainBFT.tla` is guarded by `height[v] <= MaxHeight`, so once
all honest validators have committed up to MaxHeight, height advances
to MaxHeight + 1 and no action is enabled. TLC's default behaviour
flags this as a "deadlock" — but it is the *intended* terminal state
of bounded model checking, not a real bug.

The 2026-04-30 counter-example traces (`EvaporChainBFT_TTrace_*.tla`,
plus the dated subdirectories under `states/`) were emitted when
TLC's deadlock-detection fired on this terminal state. Inspection
of the trace state (all validators at height = MaxHeight + 1, all
committed = full sequence of BlockA, slashed = {}, equivocations =
{}) confirms every safety invariant — Agreement, Validity,
CommitRequiresQuorum, LockSafety, EquivocationDetected,
StateCommitmentIntegrity, TypeOK — is satisfied at the "deadlock"
state. The traces are retained as historical artefact.

**Trade-off:** with `CHECK_DEADLOCK FALSE`, TLC explores the full
state space without early termination, so runtime grows. Tiny config
(`MaxRound=1, MaxHeight=2`) generates ~12M+ distinct states and
takes ~10-30 min in a full exhaustive run; with deadlock detection
enabled it stopped at depth 20 in ~1.5 min. For active development
during the May-Oct 2026 sprint, prefer running the smaller configs;
the Byzantine config is post-V1 verification work.

## Mapping to Rust implementation

| TLA+ concept | Rust location |
|---|---|
| Phase enum | `tendermint.rs:150` Phase { Propose, Prevote, Precommit, Commit } |
| Quorum calculation | `tendermint.rs:484` `(n * 2 + 2) / 3` |
| Lock mechanism | `tendermint.rs:237-242` locked_block/locked_round |
| Prevote quorum check | `tendermint.rs:1510` check_prevote_quorum() |
| Precommit quorum check | `tendermint.rs:1530` check_precommit_quorum() |
| Advance round | `tendermint.rs:1548` advance_round() |
| Max round force commit | `tendermint.rs:1597` MAX_ROUNDS_PER_HEIGHT |
| Equivocation detection | `tendermint.rs:870` proposals_seen tracking |
| DA sampling | `tendermint.rs:574` perform_da_sampling() |
| BlockStateCommitment | `tendermint.rs:516` state_function_commitment in block_hash |
