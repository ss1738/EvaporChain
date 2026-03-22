# EvaporChain

**A blockchain where state expires by default and the entire chain history compresses into a single recursive proof.**

## Core Innovation

EvaporChain introduces thermodynamic state decay — every piece of on-chain state has an energy budget that depletes over time. Unused state evaporates automatically. Combined with HyperNova recursive proof folding, the chain gets *lighter* over time, not heavier.

## Status: Research → Prototype

- [x] Phase 1: Complete blockchain landscape research (511 KB)
- [x] Phase 2: Frontier research — novel consensus, cryptographic weapons, impossible architectures (195 KB)
- [x] Phase 3: Architecture convergence, selection, stress testing (290 KB)
- [x] Whitepaper: Full technical specification (188 KB, 70 citations)
- [ ] **Fold-a-Block Prototype: GO/NO-GO TEST** ← We are here
- [ ] Project scaffold
- [ ] MoveVM fork with `decaying<T>`
- [ ] Dual commitment (Verkle + RSA accumulator)
- [ ] Mysticeti consensus integration
- [ ] 4-node devnet

## Technical Stack

| Layer | Choice |
|-------|--------|
| Language | Rust |
| Smart Contracts | Move (with `decaying<T, half_life>` extension) |
| Consensus | Mysticeti DAG-BFT |
| Execution | MoveVM + Block-STM parallel execution |
| ZK Proofs | HyperNova/CCS folding + Binius backend |
| State | Verkle trie (active) + RSA accumulator (expired nullifiers) |
| Signatures | ML-DSA (post-quantum) + BLS12-381 (consensus) |
| Hashing | BLAKE3 (general) + Poseidon (ZK circuits) |
| Networking | libp2p + GossipSub + erasure-coded shredding |

## Key Metrics (Targets)

| Metric | Target | Hard Fail |
|--------|--------|-----------|
| Fold time per block | <10ms | >50ms |
| Final SNARK proof size | <2KB | >10KB |
| Consensus finality | <1 second | >3 seconds |
| State evaporation latency | <1ms | >10ms |

## Research

See `/research` for the complete 1.2MB research corpus covering consensus mechanisms, cryptographic foundations, execution environments, privacy, interoperability, tokenomics, governance, AI×blockchain convergence, and security analysis.

## License

MIT
