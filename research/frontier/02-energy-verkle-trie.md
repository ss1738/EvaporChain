# Primitive 2: Energy-Annotated Verkle Trie

## Problem

Every blockchain's state trie grows forever. Ethereum has tried to ship state expiry since 2021 and failed. Solana's rent mechanism became a no-op (rent-exempt accounts). The fundamental issue: Merkle/Verkle tries have no concept of time or importance — every leaf is equal.

EvaporChain already removes evaporated objects from state, but the trie structure doesn't exploit the energy information. A subtree where every object is nearly dead still occupies the same proof space as a subtree of hot, active objects.

## What Exists

- **Ethereum state trie:** Grows ~50GB/year. No pruning of active state.
- **Verkle tries (Ethereum roadmap):** Smaller proofs than Merkle, but still no temporal awareness.
- **LSM trees (databases):** Hot/cold data separation via compaction levels. Not cryptographic.
- **Persistent data structures (Driscoll et al., 1989):** Version-aware but not energy-aware.
- **EvaporChain (current):** Verkle trie in `crates/evaporchain-state/`. Objects removed on evaporation. No energy annotation on internal nodes.

## The Idea

Augment each internal Verkle node with energy metadata. The trie becomes self-aware of which regions are hot and which are cold.

### Data Structure

Each internal node stores:
```
VerkleNode {
    commitment: [u8; 32],      // existing: IPA/KZG commitment
    children: Vec<Child>,       // existing
    max_energy: u64,            // NEW: max energy of any leaf in subtree
    min_half_life: u64,         // NEW: shortest half-life in subtree
    leaf_count: u32,            // NEW: number of active leaves
    last_activity_epoch: u64,   // NEW: most recent update in subtree
}
```

### Operations

**Insert/Update (bottom-up propagation):**
When a leaf is inserted or its energy changes, propagate upward:
```
parent.max_energy = max(child.max_energy for child in children)
parent.min_half_life = min(child.min_half_life for child in children)
parent.leaf_count = sum(child.leaf_count for child in children)
parent.last_activity_epoch = max(child.last_activity_epoch for child in children)
```

**Subtree Compression:**
When `node.max_energy == 0` (all leaves are dead/evaporated):
```
CompressedNode {
    commitment: [u8; 32],       // commitment over the dead subtree
    leaf_count: u32,            // how many ghosts are under here
    last_activity_epoch: u64,   // when the last leaf died
}
```

The compressed node replaces the entire subtree. Proof size drops from O(depth * branching) to O(1) for cold regions.

**Subtree Queries:**
- "Give me all objects with energy > X" — skip subtrees where max_energy <= X
- "What fraction of state is cold?" — sum leaf_counts of compressed nodes / total
- "Proof of non-existence for evaporated object" — compressed node + MMR proof

### Properties

1. **Self-shrinking:** As objects cool and evaporate, subtrees compress. The trie physically shrinks.
2. **Proof efficiency:** Proofs for hot objects are normal-sized. Proofs that traverse cold regions are smaller (compressed nodes skip entire subtrees).
3. **State health metrics:** The trie's energy distribution is immediately readable from internal nodes. No full scan needed.
4. **Backward compatible:** A standard Verkle trie is an energy-annotated trie where all energies are infinity. The annotation is purely additive.

## Build Plan

1. Add energy metadata fields to internal Verkle nodes
2. Implement bottom-up propagation on insert/update/delete
3. Implement subtree compression when max_energy hits zero
4. Implement decompression (resurrection — when someone refreshes a ghost under a compressed node)
5. Benchmark: proof sizes, trie size over time, compression ratio
6. Property tests: prove that an energy-annotated trie produces identical commitments to a standard trie for the same leaf set

## Existing Foundation

- `crates/evaporchain-state/` — current Verkle trie implementation
- Evaporation engine already tracks energy per object
- MMR accumulator for evaporated object hashes

## Difficulty

2-3 months. Low risk. Well-defined data structure problem. The hardest part is handling decompression correctly (when a ghost under a compressed subtree gets resurrected).

## Publication Potential

High. "Energy-annotated authenticated data structures" is a new class. No prior work combines cryptographic commitments with temporal/energy metadata for automatic pruning. Target: AFT (Advances in Financial Technologies) or short paper at CCS/NDSS.

## Key Insight

This isn't just "delete old stuff." It's a data structure where the shape of the tree reflects the thermodynamic state of the system. Hot regions are fully expanded. Cold regions collapse. The trie breathes.
