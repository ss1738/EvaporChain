# EvaporChain — Invention Stack & Doctrine

**Version:** 1.0
**Date:** 2026-04-28
**Status:** Canonical doctrine. Future sessions read this before proposing competing primitives.
**Pairs with:** `INEVITABILITY_STRATEGY.md` (the strategic frame), `frontier/README.md` (the original three primitives), `proposals/energy-stamped-mev-resistance.md` (the first MEV proposal).

This document captures every load-bearing decision made on 2026-04-28 across:
- A fresh-eyes audit (verifying Gap-A fixes + N-01 through N-06 new findings)
- Two SATYA-1 round-trips (locking the invention stack and surfacing the anti-feature manifesto)
- Seven parallel research agents (consensus, ZK/folding, cryptoeconomics, cross-disciplinary, privacy/DA, AI, plus a second round across crypto/apps/topology/light-clients/causal-time/mechanism-design/VM)

**Owner:** Satyawan Singh. **Naming convention:** primitives whose math Satyawan authored or substantially refined may carry "Singh"; primitives assembled from existing literature must cite the original.

---

## 1. The Two Unifying Claims

These are the load-bearing sentences. Every primitive below is a consequence.

### 1.1 The Single-λ Principle

EvaporChain has **one fundamental constant — λ (the decay rate)**. Every layer of the chain — consensus, mempool, time, gas, stake, governance, capabilities, identity, demurrage — is parameterized by it. Like *c* in relativity or *ℏ* in quantum mechanics.

When a sentence about EvaporChain mentions "decay rate," "energy half-life," "epoch evaporation rate," or "refresh interval" — they all collapse to the same λ.

### 1.2 Conservation of Energy as a Chain Invariant

Energy is **never destroyed** on EvaporChain — only redirected:

```
   slashed stake ───┐
   MEV revenue  ────┼─► Refresh Pool ─► auto-refresh of namespace roots
   demurrage    ────┘                    (chain history, beacon, light-cone proofs)
```

Hoarders pay (demurrage). Attackers fund operations (slashing → MEV burn → refresh). Validators earn by producing (Singh-Boltzmann Stake). The protocol funds its own keep-alive.

**Conservation invariant**: total energy budget across {accounts + stake + refresh pool + slashed pool} decreases monotonically only via the global decay term λ — never by destruction in any other transition.

---

## 2. Anti-Feature Manifesto (Whitepaper §1)

EvaporChain explicitly **refuses** three things every other L1 does:

### 2.1 No permanent data storage
Every state object has a half-life. There is no "forever" tier at the protocol level. Storage is leased, not bought. Permanent storage contradicts the core thesis that data has a lifecycle.

### 2.2 No immutable data structures
Immutable data creates a static environment that prevents the natural lifecycle of decay and removal. Every chain that worships immutability is accumulating technical debt that compounds. EvaporChain breaks this orthodoxy.

### 2.3 No bridges to chains without lifecycle alignment
Bridging to chains that don't share the death thesis would dilute the chain's value proposition and import infinite-supply attack vectors (Ronin, Wormhole, Nomad). Wrapped tokens on EvaporChain decay unless re-attested by the origin chain.

These are the three lines that must appear in §1 of the whitepaper.

---

## 3. Framing Decisions

### 3.1 Decay is a metaphor, not a literal physical claim

Confirmed by SATYA-1. Engineering implications:

- **Stop calling decay "thermodynamic" in marketing.** Replace with "algorithmic state lifetime" or "exponential lifetime."
- **Do NOT invoke Landauer's principle (kT ln 2 per bit erased).** That trap forces us to defend physical energy claims we don't make.
- **Causal-set ledger remains physics-inspired** (Sorkin causal-set math is a literal mathematical formalism, not invoking quantum gravity).
- **Whitepaper §1 must explicitly disclaim physical-thermodynamics interpretation** to prevent reviewers from rejecting on physics grounds.

### 3.2 The real adversary is regulators

Not nation-states, not exchanges, not LLM agents — **regulators mandating immutable audit trails** (MiCA, FATF travel rule, GDPR conflicts). This shapes the stack:

- **Provable Retention Proofs** is the design response (selective permanence as a first-class primitive)
- **Decay-Forget Proofs** is its dual (right-to-be-forgotten as a chain primitive)
- **Adaptive governance** must be able to adjust to jurisdictional changes
- **Privacy at retention** — even retained data is privacy-protected unless a specific authority key unlocks

---

## 4. Locked Invention Stack

### 4.1 TIER 1 — Launch Headline (12 primitives)

All Tier 1 primitives share the single λ. All ship in the May–Oct 2026 sprint.

| # | Primitive | One-liner | Source agent |
|---|---|---|---|
| 1 | **Light-Cone Consensus** | Causal-set partial-order consensus (Sorkin/Pratt). Energy decay gives the time arrow. *Soul of the chain.* | Consensus |
| 2 | **Evap-Antichain Mempool** | Mempool *is* the partial order; producer extends maximal antichains whose total energy clears a threshold. | Causal time |
| 3 | **Decay-Lamport Time** | Clock ticks by energy spent, not wall clock. Decentralized; no NTP, no PoH leader. | Causal time |
| 4 | **Singh-Lyapunov Fee Controller** | First L1 with **provably globally stable** fee market. Lyapunov V(E) = ½(E−E*)² converges *because* of decay. Antifragile under attack. **Whitepaper centerpiece.** | Mechanism design |
| 5 | **Singh-Boltzmann Stake** | Validator stake decays by default; refresh by producing blocks. Kills the stake-and-lease-key-to-MEV pattern. | Mechanism design |
| 6 | **Native Demurrage → Refresh Pool** | Piecewise rate `r(balance) = max(0, λ_base · log(balance/threshold))`. Sink = protocol-controlled refresh pool. Closes the philosophy loop. | Mechanism design |
| 7 | **Refresh Market** | AMM-priced rent per state object. Continuous keep-alive flow becomes the chain's primary economic activity. | Mechanism design |
| 8 | **Lambda-Fold (Energy-Folded Light Client)** | First sublinear-in-active-energy verifier. Nova extension where each fold step folds the energy state. Decade-defining if the math holds. | Light clients |
| 9 | **Evaporative Filtration Homology (EFH)** | Persistent homology with energy as the filtration parameter. Stability theorem (Cohen-Steiner-Edelsbrunner-Harer 2007) gives free tamper-evidence. | Topology |
| 10 | **Evaporated-Fork Certificates** | Negative-finality ZK proof: a fork *cannot* finalize because its energy decayed below threshold. Light clients verify in O(1). | Locked from twin |
| 11 | **Provable Retention Proofs** | Positive-finality dual of #10. Provable retention as a first-class operation. Regulator-survival primitive. | Locked from twin |
| 12 | **Linear-Affine-Decay VM** | Move resources × decay. "Use it or evaporate." Forces liveness as a type-system property. | DP-VM |

### 4.2 TIER 2 — V2 (6–18 months post-launch)

| Primitive | Why V2 |
|---|---|
| **Singh Attractor Consensus** | Folds into Singh-Lyapunov Fee Controller's stability framework |
| **Bell-Certified Beacon** | Device-independent randomness from CHSH Bell tests; consumed by Decay-Lamport Time |
| **Evaporative Pixel (EG-FSS)** | Energy-indexed forward-secure signatures; underwrites Evaporated-Fork Certs at the signature layer |
| **Capability-Decay VM** | KeyKOS/seL4 ocap with decay; default-deny, no `tx.origin` equivalent |
| **DP-Native VM** | First L1 with VM-level (ε,δ)-differential privacy; budget reservoir = decay primitive |
| **Phasing Nullifier Tree (PNT)** | Bounded nullifier sets — kills monotone privacy-chain growth (Tornado/Aztec/Zcash all suffer this) |
| **Cone-Merged Bridges** | Bridges valid only inside intersection of both chains' decay cones; replay-immune by construction |
| **Half-Life Wrapped Asset (HLWA)** | Wrapped tokens decay unless re-attested by origin chain — eliminates infinite-bridge-supply hacks |
| **Decay-Forget Proofs** | GDPR-native — chain *provably cannot* recover a timestamp once decayed past threshold |
| **Wilson-Singh Block Flow (WSBF)** | RG-flow on chain history; old blocks become "effective theory" parameters (12-month moonshot) |
| **Allen-Decay Opcodes** | 13 interval-relation opcodes in EvaporScript (Allen 1983); intervals bounded by energy levels |
| **Total-Programming EvaporScript** | Coq/Agda-style structural totality kills infinite-loop DoS class |
| **Cone-MEV Immunity** | Sandwich attacks become *structurally undefined* under partial-order ordering |
| **Entropic Slashing** | Shannon-weighted slash → energy-aware MEV burn → refresh pool (the conservation triplet) |
| **Singh-Shamir Cells (HLTS)** | Half-life threshold shares; secret recoverable only by surviving high-energy quorum |
| **Cone-locked Capsule (ETLP)** | Energy-gated time-lock encryption; replaces RSW sequentiality with energy-witness |
| **Decay-Stamped Nullifiers (DSN)** | Per-window nullifier accumulators; bounded state for privacy chains |
| **Energy-Bound Fiat-Shamir (EB-FS)** | One-line transcript change; stops cross-fork proof replay |
| **Hot/Cold Stake** | Two-temperature equilibrium per validator; novel split-temperature stake |

### 4.3 TIER 3 — App-Layer / Specialized (post-V2)

EW-TWAP oracle (energy-weighted TWAP), CL-AMM / Singh Pool (decay-AMM, eliminates mercenary capital), Evaporating Conviction Vote (governance), Half-Life NFT (with retention-tier), Reinforced-Attribute DID (anti-Sybil identity), Energy-Clocked Coverage (insurance), EPA-MMR (sumcheck-folded MMR), Topological Light Clients (PLC, derives from EFH), Ollivier-Ricci Mixing Bound (gossip SLA), Earth-Mover Block Diff (EMBD, MEV detection), Energy-Budgeted Compute Marketplace, Decay-Aware Concurrent VM (Thermal STM), Decay-Sealed Regions (sub-block finality), Folded-State Debugger (rewind tool), Decay-FRI / dFRI (energy-weighted proximity testing), HaPPY-style Holographic Decay Code (research-grade).

### 4.4 STRIKES — confirmed dead, do not pursue

| Primitive | Why it dies |
|---|---|
| Knot-invariant state hashes | Jones polynomial computation is `#P`-hard (Jaeger-Vertigan-Welsh 1990) |
| Sheaf-theoretic smart contracts | 5+ years foundational research; no production tooling |
| Topos-theoretic smart contracts | Ship-impossible at L1 for the foreseeable future |
| Manifold-learning state encoding | Non-canonical embeddings → no consensus across validators |
| Lorentzian causal-set contracts | Speculative; relegate to research appendix |
| AdS/CFT holographic boundary | No conformal symmetry on a blockchain — fails the core requirement. Salvage = HaPPY codes |
| Federated-validation VM | 100–1000× MPC overhead kills throughput |
| Probabilistic-determinism VM | Research-grade only; no reduction to standard BFT |
| Verifiable Decay Encryption (VDecE) | Impossibility result unless trusted committee — degrades to HLTS |
| Phantom-MAC, Decaying-BBS+ | Cosmetic decay only; no structural advantage |
| Persistent-Homology Time on consensus path | PH on streaming posets is too heavy; offline analyzer only |
| Recent-Basket Stable | Decay doesn't help peg; covered by EW-TWAP oracle |
| "Quantum Entanglement Ledger" | Violates no-signalling theorem — bad science. Salvage = Bell-Certified Beacon |
| "Bio-Inspired Adaptive Consensus" | Too broad to be a research contribution. Salvage = Immune Validator Set (V2 candidate) |
| Neural-inspired (Hopfield) consensus | No safety guarantee under Byzantine adversary; recipe for spurious attractors |

---

## 5. Headline Launch Sentence — Candidates

Pick one for technical pitch + one for marketing:

| Sentence | Best for |
|---|---|
| *"The first blockchain with one fundamental constant — every layer decays unless reinforced, with a single rate λ."* | Physics audience, paper venues |
| *"The first blockchain with conservation laws."* | Academic + technical |
| *"The blockchain whose ledger is a partial-order, not a chain."* | Cryptography crowd |
| *"The blockchain that knows when to die — and pays its own keep-alive."* | Mainstream / VC |

**Recommendation:** ship the first sentence as the technical pitch, the fourth as marketing, and use **conservation-of-energy** as the unifying mathematical claim across both papers.

---

## 6. Build Order — May–Oct 2026 Sprint

Different from the original Light-Cone-first plan because the agents revealed which primitives are foundational vs derived. Energy kernel comes first because it's the substrate every Tier-1 primitive shares.

| Weeks | Build | Why this order |
|---|---|---|
| 1–2 | **`evaporchain-energy-kernel`** crate — single λ accumulator, conservation invariant, refresh pool data structures | Substrate for everything; one λ, one accumulator |
| 3–5 | **Native Demurrage** — passive accrual on `last_touched_height` | Trivial; unlocks Refresh Market and refresh pool |
| 4–7 | **Singh-Lyapunov Fee Controller** — Lyapunov stability theorem + impl | Whitepaper centerpiece; must complete before launch |
| 6–10 | **Singh-Boltzmann Stake** + **Refresh Market** | Same kernel; defines validator economics + economic engine |
| 8–14 | **Light-Cone Consensus** + **Evap-Antichain Mempool** | Paired structurally — mempool *is* the partial order |
| 12–18 | **Lambda-Fold** light client | Decade-defining moonshot; needs the rest as substrate |
| 16–22 | **Linear-Affine-Decay VM** | Wraps the whole stack as a developer interface |
| 20–24 | **EFH**, **Provable Retention Proofs**, **Evaporated-Fork Certificates**, **Decay-Lamport Time** | Light-client + regulator + finality + time stories |

Tier 2 primitives spool up after Week 24.

---

## 7. Source Attribution

For each Tier-1 primitive, the agent or session that surfaced it:

| Primitive | Origin |
|---|---|
| Light-Cone Consensus | Consensus agent (round 1) — Sorkin/Pratt synthesis |
| Evap-Antichain Mempool | Causal-time agent (round 2) — Narwhal × decay |
| Decay-Lamport Time | Causal-time agent (round 2) — Lamport × VDF × thermodynamic accounting |
| Singh-Lyapunov Fee Controller | Mechanism-design agent (round 2) — Lyapunov drift on EIP-1559 |
| Singh-Boltzmann Stake | Mechanism-design agent (round 2) — Boltzmann distribution over validator activity |
| Native Demurrage | Mechanism-design agent (round 2) + SATYA-1 Q2 | piecewise rate, refresh-pool sink |
| Refresh Market | Mechanism-design agent (round 2) — Merton-style stochastic control |
| Lambda-Fold | Light-client agent (round 2) — Nova/HyperNova × energy scalar |
| EFH | Topology agent (round 2) — energy filtration on persistence diagrams |
| Evaporated-Fork Certs | SATYA-1 Q4 (round 1) — confirmed as "soul" |
| Provable Retention Proofs | SATYA-1 Q5 (round 1) — regulator-survival design |
| Linear-Affine-Decay VM | DP-VM agent (round 2) — Move × Wadler-Girard linear logic × decay |

---

## 8. Open Questions

| Question | Status |
|---|---|
| **Demurrage rate** — exact `λ_base` and `threshold` for the piecewise log curve | Mechanism agent proposed shape; numbers TBD via simulation |
| **Headline sentence** — pick one for technical pitch, one for marketing | 4 candidates above; not yet locked |
| **Light-Cone Consensus latency target** — must hold under Sui/Aptos DEX workload before launch | Adversarial benchmark suite to be built in M5/M6 (twin's plan) |
| **Bell-Certified Beacon hardware attestation** — which CHSH source vendor / fallback chain | V2 question; research-mode |
| **Total chain energy budget at genesis** — sets all other constants | Tokenomics ceremony question |
| **Conservation invariant proof** — formal Lyapunov-style proof that total energy is monotone-decreasing | TLA+ / Coq target during the sprint |

---

## 9. Cross-References Within Repo

- Anti-feature manifesto → **whitepaper §1**, replaces current intro
- Single-λ principle → **whitepaper §2**, foundational claim
- Conservation of energy → **whitepaper §3**, the unifying theorem to prove
- Build order → drives `REMAINING_WORK.md` punch list during the sprint
- Strikes list → checked against any future "I had an idea" proposal
- Source attribution → kept honest about which inventions are assembled vs original

When in doubt, this document overrides earlier ones. Earlier docs (`INEVITABILITY_STRATEGY.md`, `frontier/README.md`, `proposals/`) remain valid for their narrower scope but defer to the rankings here.

---

## 10. Doctrine for Future Sessions

A future Claude session reading this doc should:

1. **Not propose competing primitives** to Tier 1 — extend or refine these.
2. **Not rename** without re-confirming with Satyawan + (if available) SATYA-1.
3. **Not invoke "thermodynamic"** in marketing copy. Use "algorithmic state lifetime" or "exponential lifetime."
4. **Not suggest "quantum" or "neural"** in primitive names without specific mechanisms.
5. **Verify any Tier-1 primitive before recommending it** — if a memory file says it's done, check the code.
6. **Treat strikes (§4.4) as binding** — re-litigation requires new evidence, not re-argument.
7. **Build, don't paper.** When deciding what to do next, prefer crate scaffolding over paper drafts. Papers come from running code.
8. **Run cargo on the Minis, never the MacBook.** (Per `feedback_no_local_builds.md`.)

End of doctrine.
