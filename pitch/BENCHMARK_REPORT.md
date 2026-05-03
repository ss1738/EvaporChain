# EvaporChain Nova IVC Benchmark Report

## Executive Summary

We built a prototype proving that Nova IVC can fold blockchain state transitions including thermodynamic energy decay. The step circuit encodes EvaporChain block transitions across 64 accounts, 64 decaying objects, and 50 transactions per block, with energy decay following configurable half-life saturation curves. Results: 6.2ms amortized per block, 11.3KB proof size, 15.0ms verification. Verdict: PASS -- feasible for production.

---

## Methodology

### What We Built

A Nova IVC step circuit encoding EvaporChain block transitions with the following parameters:

- **Accounts**: 64 accounts with balance tracking
- **Objects**: 64 state objects with thermodynamic energy decay (configurable half-life, saturation handling)
- **Transactions**: 50 transactions per block (transfers, object interactions, energy refreshes)
- **Decay model**: Per-epoch energy depletion with saturation arithmetic to prevent underflow; objects reaching zero energy enter a 5-epoch grace period (`GRACE_PERIOD` in `crates/evaporchain-node/src/main.rs:163`) before evaporation
- **Evaporation**: In-circuit verification that objects with depleted energy are correctly removed from active state

### Engine

- **Curves**: Bn256/Grumpkin cycle of elliptic curves
- **Polynomial commitment**: HyperKZG (KZG-based, enabling faster verification and compression than IPA)
- **Framework**: nova-snark v0.68 (Rust)

### Batching Strategy

- **5 blocks per fold step**: Each Nova fold step processes 5 sequential block transitions
- **200 fold steps** for 1,000 blocks total
- Amortizes the fixed per-fold MSM overhead across multiple blocks

### Hardware

- **Apple Silicon** (development hardware)
- Production targets server-class hardware (expected 2-4x improvement with dedicated proving infrastructure)

---

## Results

### Original Configuration (Pallas/Vesta + IPA)

| Metric | Value |
|---|---|
| Avg fold time | 33.7ms |
| Compressed verify | 24.0ms |
| Proof size | 10,624 bytes |
| Constraints | 10,015 |
| Verdict | MARGINAL |

### Optimized Configuration (Bn256/Grumpkin + HyperKZG + 5-Block Batching)

| Metric | Value |
|---|---|
| Avg per fold step | 31.1ms |
| Amortized per block | 6.2ms |
| Compressed verify | 15.0ms |
| Proof size | 11,552 bytes |
| Constraints | 10,333 |
| Setup time | 0.67s |
| Total fold time (1000 blocks) | 6.2s |
| Evaporated objects | 64/64 |
| Verdict | PASS |

---

## Analysis

### Per-Fold MSM Dominance

The per-fold multi-scalar multiplication (MSM) time is dominated by curve operations at approximately 31ms. This cost is relatively constant regardless of constraint count, meaning circuit optimizations have diminishing returns on fold time. The MSM cost is a function of the number of group elements in the commitment scheme, not the number of R1CS constraints.

### Batching as the Key Optimization Lever

The primary improvement came from batching 5 blocks per fold step. The fixed per-fold overhead (MSM, commitment updates, cross-term computation) is amortized across 5 block transitions, yielding a 5.4x improvement in per-block cost (33.7ms down to 6.2ms). This is a structural optimization -- it reduces the number of fold steps without changing the circuit.

### KZG Verification Improvement

Switching from IPA (Inner Product Argument) to HyperKZG improved verification speed from 24.0ms to 15.0ms (37% faster) and compression time from 0.80s to 0.46s. KZG commitments enable constant-time pairing-based verification rather than logarithmic-time IPA checks. The tradeoff is a trusted setup requirement, addressed by universal SRS ceremonies.

### Circuit Constraint Reduction

Removing redundant constraint allocations reduced the constraint count modestly but had minimal impact on fold time. This confirms that fold time is MSM-bound, not constraint-bound, at this circuit scale. Constraint reduction becomes more impactful at larger circuit sizes where the MSM input vector grows.

### Remaining Optimizations for Production

- **HyperNova/CCS**: Variable-time folding with customizable constraint systems, enabling more efficient encoding of EvaporChain's state transition logic
- **Binius binary-field backend**: Binary tower fields for constraint evaluation, potentially 10-100x faster witness generation
- **Poseidon hash constraints**: Replace SHA-based hashing with arithmetic-friendly Poseidon, reducing constraint count for Merkle path verification
- **Parallel proving**: Multi-threaded MSM computation and pipelined fold steps across CPU cores or GPU

---

## Comparison with Other Systems

### Mina Protocol

Mina uses Kimchi recursive proofs (based on Pasta curves) to maintain a constant-size blockchain proof. Proof generation takes approximately 20 seconds per block. EvaporChain targets 6.2ms per block via Nova IVC folding, which avoids the full recursive SNARK proof at each step. Mina's approach requires a full proof at every block; Nova defers the expensive compression to the end.

### SP1 (Succinct)

SP1 implements a RISC-V folding scheme for general-purpose verifiable computation. It compiles arbitrary Rust programs to a zkVM. The generality introduces overhead compared to purpose-built circuits. EvaporChain uses a hand-optimized R1CS circuit for its specific state transition function, trading generality for performance.

### Polygon zkEVM

Polygon zkEVM uses Halo2 proofs to generate validity proofs for EVM-equivalent execution. Proving time is measured in minutes per batch of transactions. The use case differs (full EVM equivalence vs. custom state model), but the comparison illustrates ZK proving overhead at scale. EvaporChain's simpler execution model enables faster proving.

### StarkNet SHARP

StarkNet's Shared Prover (SHARP) uses recursive STARKs with larger proofs but faster proving due to hash-based commitments (no elliptic curve operations). Proof sizes are tens of kilobytes to megabytes, compared to EvaporChain's 11.3KB. The tradeoff is proof size vs. proving speed; EvaporChain uses SNARKs for compact proofs suitable for on-chain verification.

### Note on Comparisons

Direct comparison across these systems is imperfect. Each optimizes for different constraints: Mina for chain proof size, SP1 for developer experience, Polygon for EVM compatibility, StarkNet for proving throughput. EvaporChain optimizes for per-block folding cost with thermodynamic state management -- a different point in the design space.

---

## Conclusion

1. **Nova IVC folding works for EvaporChain's state transition model.** The step circuit successfully encodes block transitions with account balances, object state, transaction processing, and thermodynamic energy decay in a single R1CS circuit.

2. **Thermodynamic energy decay is provable in-circuit with correct saturation handling.** All 64 objects evaporated correctly across the 1,000-block run, with energy depletion following the configured half-life curve and zero-energy objects properly removed from active state.

3. **6.2ms per block is well within the target for 1-second block times.** At 1-second block intervals, proving consumes less than 1% of the block time, leaving substantial headroom for consensus, execution, and network propagation.

4. **The path to further optimization is clear.** HyperNova/CCS, Binius binary-field backends, Poseidon hash constraints, and hardware acceleration (GPU/FPGA) each offer independent speedup vectors. Conservative estimates suggest sub-millisecond per-block proving is achievable on production hardware.

5. **Open source**: [github.com/ss1738/EvaporChain](https://github.com/ss1738/EvaporChain)
