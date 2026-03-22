# EvaporChain Architecture Overview

## Design Philosophy

EvaporChain is built on the principle that blockchain state should be **ephemeral by default**. Rather than storing everything forever (the traditional approach), EvaporChain introduces thermodynamic decay — state that isn't actively maintained loses energy and eventually evaporates.

## Core Layers

### 1. Consensus Layer — Mysticeti DAG-BFT
- Sub-second finality via DAG-based Byzantine Fault Tolerance
- Uncertified DAG structure eliminates certification round-trips
- Leader-based commit rule for minimal latency

### 2. Execution Layer — MoveVM + Block-STM
- Move language with custom `decaying<T, half_life>` type extension
- Block-STM for optimistic parallel transaction execution
- Native energy accounting in the VM

### 3. State Layer — Dual Commitment
- **Active state:** Verkle trie with polynomial commitments (compact proofs)
- **Expired nullifiers:** RSA accumulator (constant-size membership/non-membership proofs)
- State transitions validated by energy decay rules

### 4. Proof Layer — HyperNova IVC Folding
- Each block's state transition is expressed as an R1CS/CCS circuit
- Blocks are *folded* into a running recursive proof via Nova/HyperNova
- The entire chain history compresses into a single proof
- New nodes sync by verifying ONE proof, not replaying history

### 5. Networking Layer — libp2p
- GossipSub for block/transaction propagation
- Erasure-coded shredding for data availability
- ML-DSA post-quantum signatures for future-proofing

## Data Flow

```
Transaction → Mempool → Mysticeti ordering → Block-STM execution
    → State update (Verkle + energy decay) → Nova fold
    → Compressed proof → Propagate to peers
```

## Key Innovation: The Fold

Traditional blockchains grow linearly — every new block adds to the history that must be stored and verified. EvaporChain *folds* each block into a recursive proof. After folding:
- The proof size remains constant regardless of chain length
- Verification time is constant
- Historical block data can be safely discarded
- The chain gets lighter, not heavier
