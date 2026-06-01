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

## Status (last refresh 2026-06-01)

- **What works:** 25,435+ native tests passing across 163 crate directories (141 active workspace members + 2 excluded WASM crates), zero `unsafe` outside the documented WASM bridge, 3-Mini Tailscale cluster verified end-to-end including real Nova IVC `--prove` chain proofs, snapshot + fast-sync, integrity_hash reproducibility, async fold off the consensus thread, and lockstep finality. 30 catalogue templates wired through typed-init → bind → dispatch → fees → required-keys (anti-regression gate: `every_catalogue_default_binds`). Coq + TLA+ zero-Admitted under their pinned toolchains.
- **Pre-mainnet hardening (cumulative through 2026-05-28):** the 2026-04-27 → 2026-05-07 base layer (oracle auth, governance, contract upgrade, DA encoder, BLS rogue-key validator path, encrypted mempool, BLS key-at-rest EVPL, gossip size, Nova `state_root_to_u64`, nova_proof checkpoint attach, finality-records pollution, persistence-write panic-propagation) **plus** the AUDIT_2026_05_17 closure trail (9 CRITICAL + 14 HIGH + 25 MEDIUM + 13 LOW — Verkle DST CR-1/2/3, VRF chain-id-scoping H-1, address-derivation DST H-2, MMR structural validation H-3, non-validator BLS PoP H-4, DA-cert forgery class Q1-Q3/Q8, Tendermint strict-quorum Q4, antichain stake-weight Q5, DA sampler binding Q6, StateProof sorted-Merkle DST Q7, Nova IVC running-total decay L0-A) **plus** the #469 P0 launch-blocker pack (PRIV-001/002 shielded-tx v1-gating, DA-001 verify_signatures_bound, VM-001 DecayingToken refresh-balance, API-001 wallet master-key fail-closed, ECON-001 slash-redistribute conservation). Per-finding trail at [`AUDIT_SCOPE.md`](AUDIT_SCOPE.md) §6.2 and §6.3.
- **What's still open:** weak-subjectivity checkpoints (V1.1 / post-launch governance flag), formal verification of Nova R1CS (engagement-pending; Coq covers state-decay + LLSA invariant preservation but not the R1CS circuit), GHOST-A paper-drift (resurrection MMR nullifier consume — Paper 1 §3.4 Inv-4), CONS-A conservation gate ChainLambda governance read-path. See [`THREAT_MODEL.md`](THREAT_MODEL.md) §6.1.
- **Distance to mainnet:** code-side is essentially complete; remaining lanes are external (audit T0.12 + auditor selection), operational (multi-validator soak T0.6, `EVAPORCHAIN_COORDINATOR_PK_BYTES` bake-in, tokenomics ceremony 28 Q's), and ship-side polish (mainnet config sprint — see [`MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) for the operator-facing strict-mode launch path).

## Where to go next

| Reading goal | File |
|---|---|
| Detailed component diagrams | `docs/architecture/diagrams/` (Mermaid) |
| Crate-by-crate description | `docs/ARCHITECTURE.md` |
| EvaporScript reference | `docs/EVAPORSCRIPT.md` |
| Cryptographic primitives | `docs/CRYPTO_SPEC.md` |
| Decay & ghost concepts | `docs/concepts/decay.md`, `docs/concepts/ghosts.md` |
| Operational parameters | `docs/PARAMETERS.md` |
| Trust model + adversary | [`docs/THREAT_MODEL.md`](THREAT_MODEL.md) |
| Run a node | [`docs/RUN_A_NODE.md`](RUN_A_NODE.md) |
| **Mainnet launch path (operator-facing)** | [`docs/MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) |
| **Audit scope + closure trail (auditor-facing)** | [`docs/AUDIT_SCOPE.md`](AUDIT_SCOPE.md) |
| Bug bounty (scoping draft) | `docs/BUG_BOUNTY.md` |
| Tokenomics ceremony Q's (28 open) | `docs/TOKENOMICS.md` |
| Genesis ceremony (protocol-level) | `docs/GENESIS_CEREMONY.md` |
| Validator onboarding (post-launch) | `docs/VALIDATOR_ONBOARDING.md` |
| Operational runbooks | `docs/runbooks/*.md` |
| Whitepaper | `research/` (1.2 MB corpus, 188 KB whitepaper, 70 citations) |
| Most recent point-in-time audit snapshot | `AUDIT_2026_05_17.md` (preserved as point-in-time; closure trail in CHANGELOG.md + SESSION_PROGRESS.md) |

---

This doc is intentionally short. Anything that doesn't fit on one page belongs elsewhere.
