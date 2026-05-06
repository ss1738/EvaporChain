# EvaporChain — The Impossible Research Stack

**Date:** 2026-05-06
**Author:** Satyawan Singh
**Scope:** The five frontier research lanes that no other L1 can occupy, plus the single mechanized theorem that anchors the academic legacy of EvaporChain.
**Pairs with:** `DOCTRINE_PUNCH_LIST.md`, `INVENTION_STACK.md`, `AUDIT_2026_05_06.md`

> **Mission:** Build the only blockchain in history whose consensus, decay, finality, privacy, and governance are simultaneously physics-grounded, mechanically verified, and forward-secret by construction. Five research lanes, all unprecedented, converging in one mechanized theorem.

---

## Table of Contents

1. [Mission Statement](#1-mission-statement)
2. [The Five Lanes — Mind Map](#2-the-five-lanes--mind-map)
3. [Branch 1 — Physics-Native Computing](#3-branch-1--physics-native-computing)
4. [Branch 2 — Mechanized Mathematics](#4-branch-2--mechanized-mathematics)
5. [Branch 3 — Quantum-Inspired Cryptography](#5-branch-3--quantum-inspired-cryptography)
6. [Branch 4 — Decay-Native Architecture](#6-branch-4--decay-native-architecture)
7. [Branch 5 — zk-Native Execution](#7-branch-5--zk-native-execution)
8. [Cross-Pollination — Where Lanes Intersect](#8-cross-pollination--where-lanes-intersect)
9. [The Big Theorem — Singh's Decay-BFT Safety + Liveness](#9-the-big-theorem--singhs-decay-bft-safety--liveness)
10. [Asymmetric Advantages](#10-asymmetric-advantages)
11. [Six-Month Execution Map](#11-six-month-execution-map)
12. [Day-1 Starting Point](#12-day-1-starting-point)

---

## 1. Mission Statement

Most L1 blockchains compete on speed, scale, or composability. EvaporChain occupies an orthogonal position: **the chain that obeys physics**. State decays by the second law. Forks evaporate via causal-set ordering. Cartels collapse under Bell's theorem. Governance is a theorem proven in Coq. The chain gets lighter, not heavier.

The objective of this document is to enumerate the five research lanes that **no other blockchain team can pursue from their starting position**, and to converge them on a single mechanized result — Singh's Decay-BFT Safety + Liveness Theorem — that becomes the academic crown jewel of the EvaporChain corpus.

This is not a feature roadmap. It is a research program.

---

## 2. The Five Lanes — Mind Map

```
                                    THE IMPOSSIBLE STACK
                                            │
        ┌──────────────────┬──────────────┬─┴──────────────┬──────────────────┐
        │                  │              │                │                  │
   [1] PHYSICS-       [2] MECHANIZED   [3] QUANTUM-     [4] DECAY-         [5] zk-NATIVE
   NATIVE             MATHEMATICS      INSPIRED         NATIVE              EXECUTION
   COMPUTING                           CRYPTO           ARCHITECTURE
        │                  │              │                │                  │
   ┌────┼────┐         ┌───┼───┐     ┌────┼────┐      ┌────┼────┐         ┌────┼────┐
   │    │    │         │   │   │     │    │    │      │    │    │         │    │    │
 Maxwell Bekenstein   E2E  Singh Anti-  GHZ  Bell-  Lattice Forward Mortal Memento  zk-     Recursive Self-
 Demon  Bound         Coq  Attractor chain KS   Cert.  IVC over Secret  AI    Contracts Evapor- Decaying  Verifying
 VM            Wheeler-Safety+ Theorem  Finality Cont. Random- Decay  Privacy Agents          Script  Proofs   Contracts
              Feynman Liveness         Theorem extual ness     State  via                                  
              Retro-                          Bell           Decay                                     
              causal
        │                  │              │                │                  │
        └──────────────────┴──────────────┼────────────────┴──────────────────┘
                                          │
                                  CROSS-POLLINATION
                                          │
              ┌───────────────────┬───────┴───────┬───────────────────┐
              │                   │               │                   │
        Decay × Coq         Bell × Coq      zk × Decay         Holographic ×
        (safety proof for   (cartel impos-   (forward          Decay (state
         evaporating         sibility theo-   secrecy via       collapses to
         system)             rem, machine-    thermodynamic     surface area)
                             checked)         erasure)
                                          │
                                  THE BIG THEOREM
                                          │
                       SINGH'S DECAY-BFT SAFETY+LIVENESS THEOREM
                  Mechanized Coq proof, end-to-end, with energy decay,
                   under partial synchrony, with adversarial validators
```

---

## 3. Branch 1 — Physics-Native Computing

**Thesis:** Apply real physics laws (thermodynamics, relativity, quantum mechanics) as binding constraints on a blockchain VM, with mathematical proofs of consistency.

| Sub-branch | Prior art | What's new | What we build | Output |
|---|---|---|---|---|
| **Maxwell's Demon VM** | Landauer's principle (1961), Bennett's reversible computing (1973). Never applied to a blockchain VM. | Each opcode either reduces state entropy or pays kT ln 2 per bit erased. Programs that violate the second law abort at consensus. | `evaporchain-maxwell-vm` crate. Per-opcode entropy accounting. | Paper at IEEE LICS or POPL. Named result: **Singh-Bennett Reversible-Execution Theorem**. |
| **Bekenstein-Bounded State** | Bekenstein bound (1972) — info bounded by surface area. Has never been applied to chain state. | Total state ≤ 2π·R·E / (ℏ·c·ln 2) where R is "chain radius", E is energy budget. State that violates the bound fails consensus. | `evaporchain-bekenstein-state` crate. State-size enforcement at consensus level via holographic principle. | Paper at FC or PETS. Named result: **Bekenstein-Singh State Bound**. |
| **Wheeler-Feynman retrocausal txs** | Wheeler-Feynman absorber theory (1945). Never used in distributed systems. | Tx confirmation requires causal-future absorber acknowledgment. Mathematically equivalent to delayed finality, framed as retrocausal. | `evaporchain-absorber` crate. Pre-confirmation via game-theoretic absorber proofs. | Paper at PODC. Named result: **Wheeler-Feynman-Singh Pre-Confirmation**. |
| **Singh-Tsirelson Tight Bound** | Tsirelson (1980) proved quantum max for CHSH = 2√2. Never derived for resource-bounded validator coalitions. | Prove the maximum S-value achievable by a cartel with bounded computation/communication budget — analog to Tsirelson but for distributed consensus. | Mathematical proof + numerical validation. Updates Causal-CHSH gate to use tight bound. | Paper at CRYPTO or ICALP. Named result: **Singh-Tsirelson Cartel Bound**. |

**Effort for the full branch:** 4–6 months in parallel.

---

## 4. Branch 2 — Mechanized Mathematics

**Thesis:** End-to-end mechanized proofs of full L1 safety + liveness with novel state models. Tezos and Cardano have Coq for sub-systems; nobody has the full theorem.

| Sub-branch | Prior art | What's new | What we build | Output |
|---|---|---|---|---|
| **End-to-End Coq Safety+Liveness** | Tezos has Coq for individual ops. Casper FFG has informal proof. Algorand has handwritten proof. **Nobody has machine-checked end-to-end safety+liveness for a thermodynamically-decaying BFT.** | One Coq theorem: ∀ state s₀, network N, honest_supermajority(N) → Safety(s₀,N) ∧ Liveness(s₀,N). Includes DAG semantics + decay invariants. | `EvaporChainSafetyLiveness.v` (~2K lines). Build on existing 5 .v files + LLSAInvariantPreservation. | Paper at **CAV** or **POPL** — top-tier verification venues. Named result: **Singh's Decay-BFT Theorem**. |
| **Singh Attractor Convergence Theorem** | MCC fork-choice exists in your code; no formal proof. | Prove `argmax exp(−β·E_path)` converges to unique trajectory under bounded adversarial stake. Closed-form Lagrangian. | Coq proof on top of `evaporchain-mcc`. | Paper at **TCC** or **EC**. |
| **Antichain Finality Theorem** | Hashgraph has informal "famous witnesses" finality. Aleph has a partial proof. **Nobody has formal proof for antichain finalization.** | Prove that closing antichain ∩ ≥2f+1 precommits is safe under DAG semantics with cross-fork equivocation detection. | Coq proof for `try_finalize_antichain` in `evaporchain-light-cone`. | Paper at **PODC** or **DISC**. Named result: **Singh-Sorkin Antichain Theorem**. |
| **Crooks-Singh Fluctuation Theorem** | Crooks (1999) is for thermodynamic systems. **Never derived for distributed consensus.** | Prove the MEV refund pmf converges to fluctuation-theorem distribution under bounded mempool latency. | Mathematical paper + simulation. | Paper at **CCS** or **FC**. |

**Effort for full branch:** End-to-end Coq alone is 4–5 months. Other 3 are 1–2 months each in parallel.

---

## 5. Branch 3 — Quantum-Inspired Cryptography

**Thesis:** Use higher-order quantum bounds and Bell theorems as cryptographic primitives in a real chain.

| Sub-branch | Prior art | What's new | What we build | Output |
|---|---|---|---|---|
| **GHZ + KS contextuality detectors** | Causal-CHSH ships in your code. **Higher-order Bell tests (GHZ, KS) have never been deployed in a chain.** | 3-party validator collusion via Mermin's GHZ inequality. Contextuality-based fraud detection via Kochen-Specker. | `evaporchain-ghz-detector`, `evaporchain-ks-contextuality` crates. | Two papers at **USENIX Security** or **CCS**. |
| **Bell-Certified Randomness Beacon hardening** | Coin-flipping protocols (Blum 1983), randao (Eth). **Nobody has Bell-certified randomness in a production L1.** | Randomness with proof that it's free of cartel manipulation up to Tsirelson bound. Stronger than randao. | Hardening of existing `evaporchain-bell-beacon` with formal proof of certifiable purity. | Paper at **CRYPTO** or **EUROCRYPT**. |
| **Lattice-based IVC over decaying state** | Nova IVC uses BN256/Pallas (vulnerable to Shor). HyperNova uses lattices in research. **Nobody has lattice IVC binding to decaying state commitments.** | Lambda-Fold variant using BabyBear (M31 prime) or lattice-based folding (Latticefold). | New crate `evaporchain-lambda-fold-pq`. Coexists with existing curve-based variant. | Paper at **CRYPTO**. |
| **Holographic Light Client** | Mina has 22KB recursive proofs. **Nobody has true O(1) commitment + O(log n) verify for full chain history.** | Compress entire chain into 32-byte SNARK + log-depth Verkle path. Verifiable on a smartphone in <10ms. | New crate `evaporchain-holographic-client`. | Paper at **CCS** or **NDSS**. |

**Effort for full branch:** 6–8 months parallel for full branch. Holographic light client alone is 3–4 months.

---

## 6. Branch 4 — Decay-Native Architecture

**Thesis:** Take "thermodynamic decay" beyond storage rent — apply it to identity, computation, secrecy, ownership, governance. Build the first system where impermanence is the default, not the exception.

| Sub-branch | Prior art | What's new | What we build | Output |
|---|---|---|---|---|
| **Forward-secret privacy via thermodynamic erasure** | Zcash, Aleo, Aztec have privacy. **None have decay-bound privacy (state + commitment evaporates after window).** | Shielded balances decay; once decayed, even commitment + nullifier are GC'd. Forward-secrecy by design. | New crate `evaporchain-shielded-decay`. Promotes Shield to default. | Paper at **PETS** or **USENIX Security**. Named result: **Forward-Secret Privacy via Decay**. |
| **Mortal AI agent identity** | Bittensor (subnet validators), Story Protocol (IP NFTs). **No L1 has thermodynamically-bounded AI agent identity.** | Each agent = on-chain entity with energy budget. Reputation = thermodynamic state. Inactive agents evaporate. Cartel detection extends naturally. | New crate `evaporchain-agent`. | Paper at **AAAI** or **NeurIPS** crypto-AI workshop. |
| **Memento contracts (commit-and-forget)** | Time-locked tx (Bitcoin OP_CLTV). **Nobody has chain-native contracts that REVEAL only after expiry of state.** | Contracts that hold sealed state, reveal only when conditions trigger (e.g., wallet inactive 5 years → execute will). | EvaporScript primitive `Memento { trigger, reveal }`. | Paper at **FC** or **CCS**. |
| **Resurrection markets** | Dust transactions. **Nobody has formalized markets for selective revival of evaporated state.** | Decentralized market where bidders pay to selectively resurrect ghost-state with proof of original commitment. Game theory + cryptographic auth. | New crate `evaporchain-resurrection`. | Paper at **EC** (Economics & Computation). |

**Effort for full branch:** 4–6 months.

---

## 7. Branch 5 — zk-Native Execution

**Thesis:** Make every smart contract a zk-circuit by default, with native recursive folding across executions, all on top of a decaying state model.

| Sub-branch | Prior art | What's new | What we build | Output |
|---|---|---|---|---|
| **zk-EvaporScript** | Aleo has Leo (zk language). Cairo for StarkNet. **Nobody has zk-native VM with thermodynamic decay primitives.** | Every EvaporScript opcode → R1CS gadget. Execution produces Nova-foldable proof. Recursive composition for cross-contract calls. | New crate `evaporchain-zk-script` with gadgets for all 65 opcodes. | Paper at **CRYPTO**. |
| **Recursive decaying proofs** | Nova IVC (Kothapalli-Setty 2022). **Nobody has IVC where the proof itself binds to decaying state.** | Lambda-Fold proofs that decay with the state they certify. After n epochs, the proof is no longer reconstructible — even by the prover. | Extension of existing `evaporchain-lambda-fold`. | Paper at **EUROCRYPT**. |
| **Self-verifying smart contracts** | Standard contracts emit logs, off-chain verifies. **Nobody has contracts that include their own correctness proof in their state.** | Contract state includes `proof: Vec<u8>` field; every state transition produces fresh proof; contract self-audits at every call. | EvaporScript primitive `Verifiable<T>` that enforces proof-of-correctness on state mutation. | Paper at **POPL**. |
| **Reversible VM (Bennett-style)** | Bennett 1973 reversible computing. **Never deployed in a blockchain.** | EvaporScript variant where every operation has an inverse. State transitions are time-reversal-symmetric until commit. Enables clean rollback semantics + entropy accounting. | New crate `evaporchain-reversible-vm`. | Paper at **LICS** or **POPL**. |

**Effort for full branch:** 4–6 months. zk-EvaporScript alone is 3–4 months.

---

## 8. Cross-Pollination — Where Lanes Intersect

The most original work happens at the intersections of the lanes.

### A. Decay × Coq → "The Decay-BFT Theorem"

The end-to-end safety+liveness proof for an evaporating system. Existing BFT proofs assume state is monotonically growing. **Nobody has proven safety + liveness when state can vanish mid-flight.** This is mathematically subtle: standard BFT proofs use induction on block height, but with decay, the inductive invariant changes shape every epoch. Requires new proof technique. **Top-tier paper at CAV or POPL.**

### B. Bell × Coq → "Cartel Impossibility Theorem"

Mechanically prove that no validator coalition can exceed S = Tsirelson bound under your causal-set + Light-Cone semantics. This combines Bell theorem semantics (probabilistic) with Coq mechanization (deterministic). Requires probabilistic Coq variants (e.g., MathComp Probability). **Joint paper at ITP + CRYPTO.**

### C. zk × Decay → "Forward-Secret Recursive Proofs"

A Nova-folded proof whose verifier circuit becomes mathematically impossible to reconstruct after the certified state evaporates. Cryptographic forward-secrecy via thermodynamic erasure. **Foundational paper at EUROCRYPT.**

### D. Holographic × Decay → "The Surface-Area Chain"

State complexity bounded by chain "surface area" (active perimeter of the state graph). As state evaporates, perimeter shrinks proportionally. Mathematically: chain entropy ≤ A(boundary)/4 in Planck units (analogy to black hole entropy). **Speculative but rigorous if pursued.**

---

## 9. The Big Theorem — Singh's Decay-BFT Safety + Liveness

### Why this is the single most important deliverable

This is the one result that, if shipped, defines the chain in academic literature for 20 years. Every future paper on energy-decay blockchains will cite Singh 2026.

### What it actually proves

```coq
Theorem decay_bft_safety_liveness :
  forall (s₀ : SystemState) (N : NetworkModel),
    partial_synchrony N →
    honest_supermajority N (2/3 + 1) →
    energy_conservation s₀ →
    decay_bound s₀ →
    (Safety s₀ N) ∧ (Liveness s₀ N).
Proof.
  intros s₀ N Hps Hsuper Hcons Hdecay.
  split.
  - apply safety_under_decay; assumption.
  - apply liveness_under_decay; assumption.
Qed.
```

Where:
- `Safety` = no two honest validators ever commit conflicting blocks at the same DAG height
- `Liveness` = under partial synchrony, every transaction is eventually included or definitively rejected
- `energy_conservation` = total energy across compartments is non-increasing modulo decay
- `decay_bound` = energy decay follows the canonical `energy_at_epoch` function

### Why this is "impossible" today

1. **Existing BFT proofs assume monotonic state** — Castro-Liskov, Tendermint, Algorand all assume state grows. Yours shrinks.
2. **DAG semantics complicate inductive arguments** — Light-Cone has multi-parent blocks, antichain finality. Standard linear-chain proofs don't apply.
3. **Decay introduces time-varying invariants** — the same state at different epochs has different energy budgets. Proof techniques must handle this.
4. **No prior art** — Tezos has Coq for ops; Cardano has Haskell formal methods; nobody has machine-checked safety+liveness for a chain that intentionally forgets.

### How to actually do it (six phases over five months)

**Phase 1 (Month 1–2): Model the system in Coq.**
- `Inductive Phase := Propose | Prevote | Precommit | Commit.`
- `Inductive Vote := PrevoteFor | PrecommitFor | Nil.`
- `Inductive State := mkState { height: nat; round: nat; phase: Phase; ... }.`
- `Inductive transition : State → Action → State → Prop := ...` (the BFT rules)

**Phase 2 (Month 2–3): Prove safety.**
- Lemma: quorum intersection (2f+1 ∩ 2f+1 ⊇ f+1 honest)
- Lemma: lock safety (locked validators don't equivocate)
- Lemma: cross-fork equivocation detection (your existing infrastructure)
- Theorem: safety = induction on height + DAG ordering

**Phase 3 (Month 3–4): Prove liveness.**
- Lemma: synchronous round eventually arrives (partial synchrony)
- Lemma: honest proposer eventually selected (VRF leader rotation)
- Lemma: timeout guarantees progress
- Theorem: liveness = bounded round count to commit

**Phase 4 (Month 4–5): Add decay invariant.**
- Lemma: energy conservation preserved across all transitions (you have similar in `LLSAInvariantPreservation.v`)
- Lemma: decay doesn't violate quorum (validators with active stake suffice)
- Lemma: state evaporation doesn't break safety (evaporated state can't conflict)

**Phase 5 (Month 5): Add DAG semantics.**
- Lemma: antichain finality is safe (your existing `is_antichain` primitive)
- Lemma: multi-parent blocks preserve causal ordering
- Theorem: decay-BFT under DAG.

**Phase 6 (Month 5–6): Polish + paper.**
- Coq build clean under Rocq 9.1.1 + extracted to OCaml for performance verification
- 30-page paper for CAV / POPL
- arXiv preprint

### Why this is feasible solo in 6 months

You already have:
- `LLSAInvariantPreservation.v` — the proof skeleton
- 5 .v files compiling under Rocq 9.1.1
- `evaporchain-light-cone::concurrency::is_antichain`
- `evaporchain-types::energy_at_epoch` (canonical decay)
- `evaporchain-consensus` working implementation

You need to add: ~2K lines of Coq + 30 pages of paper. **Hard but doable** for someone with your math + Coq + protocol depth.

---

## 10. Asymmetric Advantages

These are research lanes that other L1 teams **literally cannot pursue** given their constraints:

1. **Solana, Sui, Aptos cannot prove decay-BFT** — they don't have decay in their model. The proof technique doesn't apply.
2. **Aleo, Aztec, Zcash cannot do Bell-theorem cartel detection** — they don't have causal-set DAG semantics required for the construction.
3. **Tezos has Coq culture but no DAG, no decay, no Bell** — their proof system doesn't extend to your primitives.
4. **Ethereum has scale but not formal-methods culture** — their consensus is too entrenched to retrofit Coq end-to-end.
5. **Mina has succinct proofs but not decay** — their commitment scheme doesn't compose with thermodynamic state.

**Every one of the 5 branches is unreachable from any other L1's starting position.** That is the asymmetric advantage. Not "we are 2× faster" but "we occupy a position no one else can move to from where they are."

---

## 11. Six-Month Execution Map

| Month | Branch | Deliverable | Output |
|---|---|---|---|
| **May 2026** | Branch 2 + 3 | Start Singh's Decay-BFT proof. arXiv all 4 existing primitives (Causal-CHSH, Lambda-Fold, Light-Cone, EvaporChain). Begin GHZ extension. | 4 arXiv preprints |
| **June 2026** | Branch 2 | Safety proof complete in Coq. GHZ contextuality math hardened. | Safety Coq file ✓ |
| **July 2026** | Branch 2 + 3 | Liveness proof complete in Coq. Submit GHZ + Lambda-Fold papers to USENIX/CRYPTO. | Liveness Coq file ✓ |
| **August 2026** | Branch 2 + 5 | Add decay invariant to Decay-BFT proof. Begin zk-EvaporScript (arithmetic gadgets). | Decay-BFT v1 ✓ |
| **September 2026** | Branch 2 + 5 | Add DAG semantics to Decay-BFT. zk-EvaporScript memory + control gadgets. | Singh's Decay-BFT v2 ✓ |
| **October 2026** | Branch 2 + 4 | Polish proof + paper. Forward-secret privacy upgrade ships. | CAV submission. PETS submission. |

End of sprint:
- **2 top-tier mechanized theorems shipped** (Decay-BFT, Antichain Finality)
- **5 papers submitted across 4 venues** (CAV, USENIX Security, CRYPTO, PETS)
- **zk-EvaporScript prototype** with arithmetic + memory + control gadgets
- **Forward-secret privacy live** on testnet
- **All 4 founding primitives on arXiv** with priority date locked

---

## 12. Day-1 Starting Point

**Begin the Decay-BFT Coq proof.** Why: 4–5 months is the longest single deliverable. Start now or it does not ship by end of sprint. Everything else can fit around it.

**First step:** write the labeled-transition-system model in Coq. Approximately 200 lines. Defines `State`, `Action`, `transition`. Build it, verify it typechecks under Rocq 9.1.1.

**File:** `research/proofs/EvaporChainSafetyLiveness.v`
**CoqProject:** add the file to `research/coq/_CoqProject` so CI builds it.
**Status:** **STARTED 2026-05-06.**

---

## Appendix — Cross-references

| Section | File | Purpose |
|---|---|---|
| Existing Coq corpus | `research/coq/EnergyDecayMonotonicity.v` | Base decay lemma |
| | `research/coq/EnergyVerkleCompression.v` | Compression preservation |
| | `research/coq/PoHAFreeloading.v` | DA freeloading resistance |
| | `research/coq/LazyEagerEquivalence.v` | Rule-Based Consensus determinism |
| | `research/proofs/LLSAInvariantPreservation.v` | Conservation invariant under amendments |
| **NEW** | `research/proofs/EvaporChainSafetyLiveness.v` | **The Big Theorem (this document anchors it)** |
| Doctrine | `research/INVENTION_STACK.md` | Tier-0 + Tier-0-supporting primitives |
| | `DOCTRINE_PUNCH_LIST.md` | Layered build plan |
| Audit | `AUDIT_2026_05_06.md` | End-to-end audit — gaps + actuals |
| | `FULL_AUDIT_2026_04_24.md` | 12-agent audit baseline |

---

**End of document.**

Tomorrow: write the labeled-transition-system model in `EvaporChainSafetyLiveness.v`. Compile clean under Rocq 9.1.1 on Mini 1. That is day 1 of the impossible.
