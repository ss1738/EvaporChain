# EvaporChain — Spec One-Pager

A skim-readable summary. For depth see `docs/ARCHITECTURE.md`, `docs/CRYPTO_SPEC.md`, `docs/EVAPORSCRIPT.md`, and `docs/concepts/`.

---

## What it is

EvaporChain is a **Layer 1 blockchain whose state expires by default**. Every on-chain object has an energy budget that depletes over time, and unused objects evaporate into a cryptographic ghost record. Combined with Nova IVC recursive proof folding, the chain gets *lighter* over time, not heavier.

Differentiator vs other L1s: most chains assume state is permanent unless deleted. EvaporChain inverts the default — state is ephemeral unless explicitly refreshed. Storage rent is **not** an aftermarket optimization; it's the protocol primitive.

## Core primitives

1. **Energy decay.** Every object has `(E₀, τ)`. Energy decays per block as `E(t) = E₀ × 2^(−t/τ)`. When `E ≤ 0`, the object evaporates and only its nullifier remains in the MMR. Decay curves are configurable per object (Linear, Exponential, Asymptotic).
2. **Tendermint BFT consensus.** Stake-weighted 2/3 quorum, BLS12-381 aggregate signatures, equivocation slashing, view-change with exponential backoff. Trusted-checkpoint long-range defense.
3. **Post-quantum signatures.** ML-DSA Dilithium3 (NIST FIPS 204) for transactions; BLS12-381 for consensus. Hybrid ECDSA+ML-DSA verifier available for transition compatibility.
4. **Nova IVC recursive proofs.** Block validity folded into a single proof; chain history compresses rather than accumulates.
5. **EvaporScript VM.** Stack-based, 44 opcodes (`compiler.rs:11 enum Op`), all gas-metered. Reentrancy guarded at execution (`MAX_CALL_DEPTH = 64`) and at script (`MAX_CALL_DEPTH = 8`). Stack ≤ 1024, memory ≤ 4 MiB, loop iterations ≤ 100K.
6. **2D erasure-coded data availability.** Reed-Solomon over BLS12-381 field, namespace Merkle tree, light-client sampling, BLS supermajority DA certificates.
7. **Privacy layer.** Note tree (depth 20), Pedersen commitments with balance binding, deterministic nullifier derivation, zero-knowledge spend proofs.
8. **Block-STM parallel execution.** MVCC-based, deterministic serial-fallback path, checked arithmetic on every balance/nonce update.
9. **Energy-Verkle Trie.** Active-state commitments via Verkle tree; expired-state commitments via MMR. Single state-root binds both.

## Differentiating ideas

- **Thermodynamic state.** Storage cost is intrinsic, not extrinsic. Removes the perpetual-state bloat problem that plagues every chain over a long enough timeline.
- **Refresh-or-die.** Users opt into permanence by spending energy to refresh. Anything not refreshed eventually disappears, freeing blockspace and validator state.
- **Ghost records.** Evaporated objects leave an audit trail (nullifier in MMR) without keeping their data. Enables later cross-chain claims, audit, and dispute resolution without state-bloat penalty.
- **Folded history.** Nova IVC means the entire chain validity collapses into one verifier query. Light clients are O(1) in chain length.

## Comparison with neighbouring L1s

| Property | EvaporChain | Ethereum | Celestia | Cosmos SDK chain |
|---|---|---|---|---|
| State growth model | Decays automatically | Permanent (rent in flight) | DA only | Permanent |
| Consensus | Tendermint BFT | PoS Casper FFG | Tendermint | Tendermint |
| DA primitive | 2D erasure + PoHA | Blob market (EIP-4844) | 2D erasure (canonical) | Block bytes |
| Proof system | Nova IVC | KZG (separate provers) | None canonical | None canonical |
| Privacy | Native (note tree) | L2 (Aztec, Tornado) | None | IBC-add-on |
| PQ signatures | Yes (ML-DSA) | No (ECDSA) | No | No |
| State expiry | Default (decay curves) | Proposed (statelessness) | n/a | None |

## Stack at a glance

| Layer | Implementation |
|---|---|
| Language | Rust (85 workspace crates: 18 core + 60 substrate + 7 Tier-2 starts) |
| Smart contracts | Template contracts + EvaporScript (custom 44-opcode VM) |
| Consensus | Tendermint BFT + BLS aggregation + VRF leader election |
| Execution | Block-STM parallel + serial fallback + PID fee controller |
| State | Energy-Verkle trie (active) + MMR (expired) + RocksDB + WAL |
| Crypto | BLAKE3, ML-DSA Dilithium3, BLS12-381, Poseidon, Pallas, Nova |
| Networking | libp2p (GossipSub, Kademlia, Noise, TLS 1.3) |
| API | Axum HTTP + JSON-RPC + WebSocket events + dashboard |
| Wallets | TypeScript SDK, mobile (React Native), browser extension |

## Status (last refresh 2026-05-11)

- **What works:** 25,435+ native tests passing across 147 workspace crates (16 core + 131 substrate / supporting), zero `unsafe`, 3-Mini Tailscale cluster verified end-to-end including real Nova IVC `--prove` chain proofs, snapshot + fast-sync, integrity_hash reproducibility, async fold off the consensus thread, and lockstep finality.
- **Hardening since 2026-04-27:** oracle authentication closed (HybridVerifier against validator-set lookup), governance closed (stake-weighted vote, quorum, param-range validation, timelock), contract upgrade closed (`governance_approved` gate), DA encoder wired (`build_block_da_inputs(txs)` → identical proposal-time and serve-time `data_root`), BLS rogue-key closed (PoP enforced at `add_validator()` and at genesis), encrypted mempool integrated, BLS key-at-rest encrypted (Argon2id + XChaCha20-Poly1305 EVPL format), gossip size unified at 4 MB, Nova `state_root_to_u64` truncation fixed, nova_proof attaches at checkpoint boundaries, finality-records pollution closed (6 layered guards in `FinalityTracker::on_block_finalized_with_active`), persistence-write panic-propagation closed (`fatal_persistence_error` graceful-exit pattern across all RocksDB write sites).
- **What's still open:** weak-subjectivity checkpoints, Block-STM contention path under high write conflict, empty-block `data_root` handling, formal verification of Nova R1CS. See `audit/end_to_end_audit_2026_04_27.md` and `docs/THREAT_MODEL.md` (the 2026-04-27 supplement was folded into the base on 2026-05-07).
- **Distance to mainnet:** code-side hardening near-complete; remaining work is operational (weak-subjectivity, Block-STM polish) plus external validation.

## Where to go next

| Reading goal | File |
|---|---|
| Detailed component diagrams | `docs/architecture/diagrams/` (Mermaid) |
| Crate-by-crate description | `docs/ARCHITECTURE.md` |
| EvaporScript reference | `docs/EVAPORSCRIPT.md` |
| Cryptographic primitives | `docs/CRYPTO_SPEC.md` |
| Decay & ghost concepts | `docs/concepts/decay.md`, `docs/concepts/ghosts.md` |
| Operational parameters | `docs/PARAMETERS.md` |
| Trust model + adversary | `docs/THREAT_MODEL.md` |
| Run a node | `docs/RUN_A_NODE.md` |
| Operational runbooks | `docs/runbooks/*.md` |
| Whitepaper | `research/` (1.2 MB corpus, 188 KB whitepaper, 70 citations) |
| Current audit state | `audit/end_to_end_audit_2026_04_27.md` |

---

This doc is intentionally short. Anything that doesn't fit on one page belongs elsewhere.
