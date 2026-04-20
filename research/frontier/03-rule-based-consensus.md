# Primitive 3: Rule-Based Consensus for Time-Dependent State

## Problem

All production BFT protocols (Tendermint, HotStuff, Jolteon, Narwhal/Bullshark) assume state is a deterministic snapshot. Validators execute the same transactions and arrive at identical state. This breaks when state is a function of time.

EvaporChain's state depends on when you evaluate it. The same object queried at epoch E and E+1 has different energy. With discrete epochs this works (all validators evaluate at the same epoch). But it creates problems:
- State roots already diverge between nodes (observed: Mini2 ghost_count=4330 vs Mini3=4290)
- Variable block times become impossible (validators must agree on exact evaluation time)
- DAG-based consensus (where blocks are produced asynchronously) is incompatible
- Light clients must know the exact epoch to verify any proof

## What Exists

- **Tendermint/CometBFT:** Deterministic state machine replication. State = f(genesis, tx_1, tx_2, ..., tx_n). No time dependence.
- **Timed automata (Alur & Dill, 1994):** Formal model for time-dependent systems. Used in real-time verification (UPPAAL). Never applied to blockchain.
- **CTMCs (Continuous-Time Markov Chains):** Used in stochastic model checking (PRISM). Not blockchain.
- **Chainlink OCR:** Nodes agree on observation rules for oracle data. Closest analogy but limited to oracles, not base-layer state.

No published work on "state as a continuous function of time" in blockchain consensus.

## The Idea

Validators agree on **decay rules** and **anchor states**, not on every intermediate state value.

### State Function Commitment

Instead of committing to a state root (which changes every instant), validators commit to a **state function**:

```
StateCommitment {
    anchor_root: [u8; 32],     // Verkle root at anchor epoch
    anchor_epoch: u64,          // when the anchor was computed
    decay_rules_hash: [u8; 32], // hash of the decay parameter set
    active_count: u64,          // objects alive at anchor epoch
}
```

Any verifier can derive the state at any epoch >= anchor_epoch:
```
energy(object, epoch) = anchor_energy * 2^(-(epoch - anchor_epoch) / half_life)
```

### Lazy Evaluation

State is not eagerly computed every epoch. Instead:
- **At anchor epochs** (e.g., every 100 blocks): full state is materialized, Verkle root computed, validators reach consensus on the anchor.
- **Between anchors:** state is computed lazily on demand. Any query includes the query epoch, and the node computes the answer from the last anchor + decay rules.
- **Consensus rounds** only commit to: (block_hash, tx_merkle_root, anchor_ref, data_root). No per-block state root.

### Why This Works for Decay

EvaporChain's decay formula is deterministic given (initial_energy, half_life, elapsed_epochs). Two validators applying the same formula to the same anchor state at the same query epoch MUST get the same answer. The formal property:

```
For all objects O, epochs E >= anchor_epoch:
  lazy_eval(O, E, anchor_state) == eager_eval(O, E, full_state_at_E)
```

This holds for exponential decay. It also holds for any decay function that is:
1. Deterministic (no randomness)
2. Monotonically decreasing
3. Depends only on (initial_energy, half_life, elapsed_time)

### What This Enables

- **Variable block times:** Blocks don't need to be produced every epoch. Validators can produce blocks at different rates without state divergence.
- **DAG-based consensus:** Asynchronous block production becomes possible because state agreement only happens at anchors.
- **Efficient light clients:** A light client only needs the anchor state + decay rules to verify any proof. No need to sync every block.
- **Reduced bandwidth:** No state root in every block header. State roots only at anchor points.

## Formal Proof Sketch

**Theorem:** Lazy evaluation is equivalent to eager evaluation for monotone decay functions.

**Proof sketch:**
1. Let S_0 be the anchor state at epoch E_0.
2. For any object O with energy E_O and half_life H_O at epoch E_0:
   - Eager evaluation at epoch E: apply decay at each intermediate epoch E_0+1, E_0+2, ..., E
   - Lazy evaluation at epoch E: compute E_O * 2^(-(E - E_0) / H_O) directly
3. Since exponential decay is multiplicative: the product of per-epoch decay factors equals the direct computation.
4. QED for exponential decay. Generalizes to any function where f(t1+t2) = f(t1) * f(t2).

The key constraint: decay must be a **semigroup homomorphism** over time. Exponential decay satisfies this. Linear decay does not (but EvaporChain uses exponential).

## Build Plan

1. Define `StateCommitment` struct with anchor semantics
2. Implement anchor epoch selection (configurable interval, e.g., every 100 blocks)
3. Modify block header: replace per-block state_root with anchor_ref
4. Implement lazy state evaluation with caching
5. Modify Tendermint to reach consensus on anchors instead of per-block state
6. Add epoch-parameterized state proof verification
7. Formal specification in TLA+ or similar
8. Property tests: lazy vs eager evaluation equivalence across 10K+ random scenarios

## Existing Foundation

- `crates/evaporchain-consensus/src/tendermint.rs` — current BFT implementation
- `crates/evaporchain-state/` — state management with Verkle trie
- Evaporation engine with exponential decay (bit-shift formula)

## Difficulty

3-4 months implementation + 1-2 months formal proof. Medium-high risk. The formal proof is the hard part — specifically handling edge cases around object creation/deletion between anchors, and resurrection of objects that crossed the ghost threshold between anchor points.

## Publication Potential

High. "Consensus over state functions rather than state values" is a novel formalization. Target: PODC (Principles of Distributed Computing) or DISC (Distributed Computing) as a short paper, or FC (Financial Cryptography) as a full paper.

## The State Root Divergence Fix

This primitive directly explains and fixes the observed divergence between Mini2 (ghost_count=4330) and Mini3 (ghost_count=4290). The divergence happens because each node processes evaporation at slightly different wall-clock times. With anchor-based consensus, all nodes agree on the exact anchor state and derive the same values.
