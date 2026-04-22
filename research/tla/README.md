# EvaporChain TLA+ Formal Specification

Formal model of EvaporChain's Tendermint BFT consensus with energy decay,
Rule-Based Consensus (BlockStateCommitment), and DA sampling.

## Files

- `EvaporChainBFT.tla` — Main specification
- `EvaporChainBFT.cfg` — 3-node honest model (matches live cluster)
- `EvaporChainBFT_Byzantine.cfg` — 4-node model with 1 Byzantine validator

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
# Honest 3-node model (fast, ~minutes)
java -jar tla2tools.jar -config EvaporChainBFT.cfg EvaporChainBFT.tla

# Byzantine 4-node model (slower, ~hours)
java -jar tla2tools.jar -config EvaporChainBFT_Byzantine.cfg EvaporChainBFT.tla
```

Or use the TLA+ Toolbox IDE / VS Code TLA+ extension.

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
