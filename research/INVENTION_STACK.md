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

End of original doctrine.

---

# Amendment 1 — TIER 0: Theorem-Grade Primitives

**Added:** 2026-04-28 (same day, round 2)
**Method:** 4 frontier-mode research agents (stochastic thermodynamics, computational mechanics, self-modifying protocols, far-frontier mathematics), each instructed to surface only primitives a CRYPTO/Nature reviewer would call genuinely unprecedented for an L1.
**Status:** These primitives **outrank Tier 1** in importance because each comes with a *published closed-form theorem* giving EvaporChain a provable claim no other chain can structurally make.

## A1.1 The new headline claim

EvaporChain is the first L1 whose **consensus, fee market, light client, upgrade path, and protocol history are all closed-form solutions of named published theorems, parameterized by a single constant λ.**

## A1.2 The five theorem-grade primitives

Each one binds a *specific* published theorem to EvaporChain via the single-λ. None of them are portable to a chain without a structural decay primitive — that's the moat.

| # | Primitive | Theorem | What EvaporChain alone can state |
|---|---|---|---|
| **T1** | **Maximum-Caliber Consensus (MCC)** — fork-choice rule selecting the chain trajectory that maximizes path-entropy under ⟨ΔE⟩ = λ | Jaynes 1980 (Maximum Caliber); Pressé-Ghosh-Lee-Dill *Rev. Mod. Phys.* 2013 | "Our fork choice is the unique trajectory `argmax exp(−β·E_path)` over candidate chain trajectories — closed form by Lagrange duality on the maximum-entropy program. (Note: a Perron-Frobenius solution would require a strongly connected graph; the LightCone DAG is acyclic, so adjacency is nilpotent and Perron is vacuous. The Lagrangian `argmax` is what's actually shipped.)" |
| **T2** | **Crooks-Singh Fee Equilibrium (CFM)** — closed-form fee distribution `p_eq(f) ∝ exp(−β f) · ρ_mempool(f)` with `β = 1/λ` | Crooks 1999 (Fluctuation Theorem); Jarzynski 1997 | "Our fee market exposes the Crooks identity primitive `log(p_F / p_R) = β·(W − ΔF)` — implemented as `crooks_log_ratio_millibits(p_F, p_R)`. The chain ships the LHS; the RHS-equality test (synthetic forward/reverse trajectory pair, assert equality to fixed-point precision) is open work tracked in `DOCTRINE_PUNCH_LIST.md` Layer 2." |
| **T3** | **Causal-State Light Client (CSLC)** — ε-machine reconstruction of the energy-filtered tx process | Shalizi-Crutchfield 2001 (Optimal Prediction Theorem); Shalizi-Klinkner 2004 (CSSR) | "Our light clients carry the *unique minimal sufficient predictive model* of the energy-surviving tx process. Provably optimal — any model with fewer states cannot be predictively sufficient." |
| **T4** | **Lambda-Locked Self-Amendment (LLSA)** — protocol upgrades require a Coq/Lean term of type `forall s, Inv(s) → Inv(step_new(s))` | Pinned MetaCoq kernel + extraction-to-Rust | "Our upgrades are gated by mechanically-checked invariant-preservation proofs — Tezos has self-amendment without proofs; we are the first chain whose governance is a theorem." |
| **T5** | **Evaporative Protocol Versioning (EPV)** — old protocol versions decay below `E_min` and become *cryptographically un-runnable*; verifier modules pruned by the same λ | Direct consequence of single-λ + state pruning | "Rollback is not socially discouraged — it is *physically impossible*. The verifier modules for old versions have evaporated." |

## A1.3 Theorem-grade supporting primitives

These are the second layer — each a closed-form result, not a heuristic:

| Primitive | Theorem | Use |
|---|---|---|
| **Sanov-Slashing** | Cramér 1938; Sanov 1957 | slash magnitude = stake × KL-rate function I(observed‖honest); replaces ad-hoc percentages with the *exact* large-deviation cost |
| **TUR Liveness Detector** | Barato-Seifert 2015 (Thermodynamic Uncertainty Relation) | falsifiable thermodynamic liveness oracle: `Var(J)/⟨J⟩² ≥ 2/Σ`. Cheap passive monitor. |
| **Cμ-Gate** | Shalizi-Crutchfield identity Cμ ≤ E + hμ | block header carries Cμ; consensus rejects ΔCμ violations (Sybil/spam detector, principled τ from theorem) |
| **MDL-Shard** | Rissanen 1978 (Minimum Description Length) | sharding partition Π* = argmin L(Π) + L(D \| Π); provably optimal not heuristic |
| **Causal-Cone Validator State** | Shalizi 2003 (light-cone sufficient statistics) | upgrades Light-Cone Consensus from heuristic to theorem-backed via the same Optimal Prediction Theorem |
| **Crooks-MEV Refund** | Crooks 1999 fluctuation-theorem ratio | refund formula falls out of CFM; fair restitution = ΔF computed from forward/reverse work distributions |

## A1.4 Far-frontier math — what survived the L1 shipping filter

Out of 14 candidate exotic-math primitives, **5 survived** as ship-now or research-ready, with hard novelty defensible at L1:

| Primitive | Math | Why it ships |
|---|---|---|
| **Authenticated Energy-MERA** | Vidal 2007 / Evenbly-Vidal 2011 — Multi-scale Entanglement Renormalization Ansatz | MERA layer ℓ = states with half-life τ₀·2^ℓ. Disentanglers = decay operator. Energy filtration *is* the MERA RG flow. First state commitment exposing **correlation structure**, not just account scalars. **GATED on empirical entropy measurement** of real chain workloads — this is a 2-week go/no-go study. If chain state has log-correlation, MERA crushes Verkle. If only area-law, downshift to authenticated MPS (still a first). |
| **p-adic ultrametric Merkle** | Hughes 2004 (every ultrametric space embeds in a tree); Khrennikov 1996 | p-adic valuation v_p(x) = energy level. Ultrametric balls form a *strict* tree — perfect Merkle-native geometry. Distinctive, low-risk, ship-now. No other chain has p-adic state metrics. |
| **Tropical Plücker Light Client** | Speyer-Sturmfels 2004 (tropical Grassmannian = phylogenetic trees) | Tropical Plücker coords commit to *entire tree shape* canonically, not just root. Edge weights `−log(remaining energy)` — tropical (min,+) gives multiplicative aggregation = energy-product paths. Clean fit. |
| **Modular-Form Beacon** | Zagier; Eisenstein E_k(τ), modular discriminant Δ(τ) | Per-epoch beacon = (E_4, E_6, Δ) at τ_epoch from VRF. Outputs satisfy known modular equations — aperiodic, hard to fake without solving the modular equation, cheap to verify. q-expansion in q = e^(2πiτ) reframes naturally as e^(−λt). |
| **Braid-Group Sequencer Commitment** | Garside normal form; Dehornoy ordering | tx ordering = braid word in B_n with canonical Garside form. Cheap, unprecedented at L1. *Don't oversell as "anyonic"* — it's just braid-group commitment, no quantum required. |

## A1.5 Strikes confirmed by frontier-mode round

In addition to the original Strikes list (§4.4), these are now also **confirmed dead**:

| Primitive | Reason |
|---|---|
| **Solomonoff oracle / AIXI** | Incomputable; no verifiable approximation; "AI buzzword" risk |
| **Levin universal search as block production** | No useful cutoff; reduces to running the validator anyway |
| **Chaitin Ω as beacon** | Uncomputable → any "Ω-beacon" is hand-wave |
| **NCD (Normalized Compression Distance) fork choice** | Adversarially exploitable with off-the-shelf compressors |
| **Reflection towers (3-Lisp on-chain)** | Wand 1998 broke compositional reasoning under reflective shifts; halting at every level. Skip. |
| **Meta-circular validators** | Bootstrap regress; can't formally ground first interpreter without an external metalanguage |
| **HoTT / Cubical-type-theory contracts** | Decades premature; no production extraction story |
| **Reflexive (self-verifying) headers** | Löb's theorem — ship Mina-style succinctness, skip self-reference |
| **Tropical crypto (KEMs/signatures)** | Cryptanalytic body count too high (Kotov-Ushakov 2018, Brown-Monico 2023) |
| **Geometric (Clifford) algebra state** | Continuous, no security reduction, hashing real multivectors loses the algebra's point |
| **TQFT contracts / motivic integration / F1 / adèles / ∞-cats / λ-rings** | Ship-impossible — no compute backend in 2026, generically #P-hard or computationally undefined |
| **Anyonic braiding for tx ordering primitive** | BQP-complete to simulate. Salvage = braid-group commitment (above) without quantum claim |
| **Carnot validator-reward bound** | Needs two temperatures; EvaporChain has one λ. Forced. |
| **Entropy-production auctions** | Clausius inequality on bids has no clean operational meaning |
| **Process-calculus sharding (π / ambient / join calculus)** | Sharding is already the hardest engineering surface; defer to post-mainnet |

## A1.6 Updated headline launch sentences

The Tier 0 round produced sharper candidates than what was in §5. Replace earlier list with these:

| Sentence | Best for |
|---|---|
| **"EvaporChain — the first blockchain whose consensus, fee market, light client, upgrade path, and history are all closed-form solutions of named theorems, parameterized by one constant λ."** | Technical pitch, paper venue |
| **"EvaporChain — a tensor-network-authenticated, ultrametric, tropically-committed L1 with energy as the renormalization-group flow."** | Math-frontier crowd; one-sentence summary of what's genuinely novel about the math |
| *"The first blockchain with conservation laws."* | Academic |
| *"The blockchain that knows when to die — and pays its own keep-alive."* | Mainstream / VC |

## A1.7 Updated build order — Tier 0 inserted

| Weeks | Build | Why |
|---|---|---|
| 1–2 | `evaporchain-energy-kernel` (substrate) **+** `evaporchain-mera-gate` (empirical entropy measurement on Ethereum/Solana account-touch data, 2-week go/no-go for MERA) **+** `evaporchain-padic` crate (low-risk parallel) | Substrate + MERA decision + cheap math win |
| 3–5 | Native Demurrage **+** `evaporchain-tropical` (Plücker commitments) | Closes philosophy loop + far-frontier math |
| 4–7 | Singh-Lyapunov Fee Controller **+ CFM closed-form extension +** Sanov-Slashing | Fee-market triple-stack: stability + closed-form equilibrium + theorem-grade slash |
| 6–10 | Singh-Boltzmann Stake **+** Refresh Market **+** TUR Liveness Detector | Validator economics + chain economic engine + falsifiable liveness oracle |
| 8–14 | Light-Cone Consensus **+** Evap-Antichain Mempool **+ MCC fork choice +** Causal-Cone Validator State | Headline consensus stack — all four pieces are theorem-grade together |
| 10–16 | **LLSA** (Coq tooling, MetaCoq + extraction) **+ EPV** (parallel, much simpler) | Self-amendment with invariant proofs + un-rollbackable history |
| 12–18 | Lambda-Fold light client **+ CSLC** (ε-machine reconstruction via CSSR) **+ Cμ-Gate** | Theorem-grade light client + sufficient-statistic state + spam detector |
| 14–18 | (if MERA gate passed) **Authenticated Energy-MERA** crate, χ=4 prototype | Tensor-network state commitment with correlation structure |
| 16–22 | Linear-Affine-Decay VM **+** MDL-Shard | Developer interface + provably-optimal sharding |
| 20–24 | EFH (filtration homology), PRP, Evaporated-Fork Certs, Decay-Lamport Time **+** Modular-form beacon | Light-client + finality + time + randomness |

## A1.8 Open empirical question — the MERA gate

This is the only *go/no-go* gate in the entire Tier-0 stack. Real chain state may NOT have log-correlation structure; if it doesn't, MERA reduces to flat Merkle with χ²× overhead.

**Test:** pull Ethereum mainnet block-by-block account-touch graph for 1M blocks; compute mutual-information matrix; check spectrum.
- **If log-correlation:** MERA goes ahead as the headline state commitment.
- **If only area-law:** downshift to authenticated MPS (1D Matrix Product State) — still a first at L1, just less ambitious.
- **If random:** drop tensor networks entirely; ship Verkle + Energy-Verkle as planned.

This gate runs in parallel with Week 1–2 of the energy-kernel work. Decision drives the Week 14–18 sprint.

## A1.9 Doctrine update for future sessions

Add to §10 doctrine:

9. **Tier 0 outranks Tier 1 in priority.** When choosing what to ship first, prefer Tier 0 theorem-grade primitives where ship-now is feasible.
10. **Don't propose "X with decay"** unless X is a primitive nobody has at L1 *and* decay is structurally required. Cosmetic decay disqualifies.
11. **Cite the specific theorem** for any primitive claiming theorem-grade status. "Lyapunov-stable" without a Lyapunov function is marketing, not engineering.
12. **The MERA gate must pass before MERA ships.** Don't write the whitepaper section assuming success.
13. **Löb's theorem is real.** Don't claim the chain has escaped Gödel. The Coq kernel is an external TCB; document it honestly.

End of Amendment 1.

---

# Amendment 2 — Application Layer & Launch dApp Candidates

**Added:** 2026-04-28 (same day, round 3)
**Method:** 4 application-layer frontier agents (energy-backed stablecoin, cultural/memetic, RWA expiring assets, self-referential primitives), each filtered for "would the New York Times / Wired / Atlantic actually cover this, or is it crypto-incest?"
**Status:** Application-layer addendum to Tier 1/2/3. These are the dApps that demonstrate the chain to non-crypto audiences.

## A2.1 Three categories, three different press lanes

| Category | Lead candidate | Secondary | Audience |
|---|---|---|---|
| **Math / finance** | EnergyCoin (YELLOW pending spec gaps) | — | Financial press, economists, central bankers |
| **Cultural / art** | The Gallery That Forgets | Time Capsules + Memorial Contracts | NYT Arts/Style, art press, mainstream culture |
| **Real-world commercial** | Hour-Block Capacity Tokens (HBCT) | EU ETS Phase tokens | Industrial / energy press, real B2B customers |
| **Philosophical / self-referential** | Mortis + Sentinel + Tombstone (four-act structure) | Refresh-Pool Patronage | New Yorker, Atlantic, MIT Tech Review |

These are not mutually exclusive; they serve different launch goals. A solo founder ships at most one as the headline; the others slot in as supporting moments.

## A2.2 EnergyCoin — verdict YELLOW (ship-after-spec)

The chain's potential unit-of-account currency. Pegged not to USD, not to crypto, but to the chain's **aggregate active energy**. Closed-form value, no oracle, the chain is its own central bank with one constant.

**Critical math reframe:** EnergyCoin is a **Bancor-family bonding curve with state-dependent ratio R(t)=E(t)/N_active, NOT a Terra-family dual-token.** The UST death-spiral physics literally does not apply — there is no second token whose supply expands to defend peg. Mint/redeem is conservative by construction.

**Genuine novelty:** the first production currency whose peg target is computed in closed form from chain state alone. Closer in spirit to Technocracy 1932 energy certificates than to any post-2014 algo-stable.

**Critical branding correction:** EnergyCoin is NOT a USD stablecoin — it is the chain's **unit of account**. Floats against fiat. Same family as gold-pegged coins or REIT shares. Anyone selling it as "stable in dollars" is recreating UST's marketing fraud. The honest pitch: *"The first currency whose value is a closed-form function of one constant — no oracle, no external collateral, no central bank."*

**Three spec gaps that block launch:**
1. **R(t) update semantics under MEV** — must be computed atomically at execution, not submission, with single canonical ordering
2. **Refresh-pool solvency invariant** — must be a *proof*, not assumption. TLA+ / Coq target.
3. **Fee-controller × EnergyCoin coupling** — specify whether mint/redeem ops subtract from E(t) before or after R(t) computation; cross-derivatives with Singh-Lyapunov controller need a 2-page note.

**Build:** spec work in Weeks 1–4, implementation Week 16+ after kernel + Singh-Lyapunov + Refresh Market are in place.

## A2.3 The Gallery That Forgets — fusion of three primitives

One launch artifact, three primitives:

1. **Provably-Mortal NFTs ("Mayflies")** — minted with declared half-life + ZK death certificate. Wallet shows literal countdown.
2. **Decay-as-Performance-Art** — gallery contract; artists deposit works with chosen half-lives; gallery's visual state changes daily. Closing date is *thermodynamic*.
3. **AI-Decay-Art** — generative pieces taking chain-energy as runtime parameter; output literally changes as state evaporates. Basinski's Disintegration Loops on-chain.

**The lineage argument:** places EvaporChain in the lineage of **Banksy / Hirst / Goldsworthy / Abramović / Tibetan sand mandalas / Felix Gonzalez-Torres** — *not* in the lineage of Solana memecoins. Real cultural pedigree.

**The single sentence:**
> *"It is the first thing humans have made that is provably going to die."*

That sentence is what gets EvaporChain out of crypto press and into the rest of the world.

**Build:** 8–10 weeks for primitives, 4–6 months for an actual partner exhibition (Serpentine, Tate, MoMA PS1). Online MVP first, IRL partnership in parallel. Risk: art-world partnerships are slow; mitigation = ship online primitive first, court partner separately.

## A2.4 HBCT (Hour-Block Capacity Tokens) — RWA wedge

Out of 17 RWA candidates, **electricity capacity won all three rankings** (largest 18-month wedge, easiest 6-month demo, most defensible).

**Why electricity:** capacity in hour H decays to 0 at H+1. Single-λ is dimensionally honest, not metaphor. Battery state-of-charge IS a decaying inventory.

| Why HBCT wins |
|---|
| $5T global electricity market; battery-storage segment growing >30% YoY |
| UK Elexon BMRS + ENTSO-E APIs are open → solo founder can ship testnet demo with real GB grid data, no regulator approval needed |
| Existing energy chains (Power Ledger, Energy Web, WePower) handle RECs/PPAs — **none use decay as primitive** |
| Customer (battery aggregators — Octopus Kraken, Habitat Energy) has concrete day-ahead/intraday balancing pain |
| Regulatory bar is light (Ofgem/FERC capacity-market frameworks are utility-token-friendly) |

**The pitch:** *"The first L1 to natively price what physics already prices."*

**Primitive:** `HBCT { delivery_location, hour_slot, mwh_amount }`. Tokens burn at slot close; oracle confirms via smart-meter / settlement-system feed. Battery operators issue forward HBCTs against state-of-charge; aggregators clear day-ahead and intraday.

**Build:** 6–8 weeks for testnet demo with real GB grid data. Production needs balancing-responsible-party partner. Reg score 3 (utility-token-friendly).

**Fallback wedge:** EU ETS Phase-bounded allowances (~€800B/yr turnover). Bigger but reg score 5 (MiFID II treats EUAs as financial instruments since 2018).

## A2.5 The Four-Act Self-Referential Structure

The self-referential agent surfaced something stronger than individual primitives — a *vocabulary* spanning the chain's entire arc:

| Act | Primitive | One sentence |
|---|---|---|
| **Birth** | Genesis with LLSA-checked invariants | "The first blockchain whose constitution is a Coq-checked proof." |
| **Life** | **Sentinel** — autonomic parameter governance via decay-weighted LLSA voting within hard-coded bounds | "EvaporChain is the first chain that governs itself the way a body does — through homeostasis, not legislators." |
| **Small deaths** | **Tombstone** — 32-byte commitment for every fully-evaporated account, written to non-decaying eulogy trie | (The Maya Lin parallel writes itself.) |
| **Final death** | **Mortis** — when refresh pool falls below ε for N epochs, the final state root is auto-minted as a single unowned NFT visible to all light clients forever | *"The first blockchain that signs its own death certificate."* |

**Why this is the strongest narrative play:** every act is structurally impossible on a chain that doesn't decay. Bitcoin promises immortality (and quietly fails); EvaporChain promises mortality (and provably succeeds). The four-act structure is itself the pitch.

**Builds (cheap):**
- Mortis: 2–3 weeks (death predicate + deterministic mint-on-halt rule + light-client visibility)
- Sentinel: 4–5 weeks (LLSA exists; the homeostatic controller is the missing piece)
- Tombstone: 2 weeks (32-byte commitment + eulogy-trie data structure)

Three primitives, all small, all ship-now, all genuinely first-of-their-kind.

## A2.6 Strikes from the application-layer round

Confirmed dead at app layer:

- **Half-Life Memes (decaying total supply)** — crypto-incest, not mainstream
- **Decay-DNS** — crypto-incest only
- **Decay-Voting** — governance press only
- **Living Whitepaper** — blockchain-flavored conceptual art, pretentious
- **Möbius Genesis** — recursive proof at block 0 — decoration, not structural
- **Self-Referential Naming** — pretentious
- **Reflexive Block Headers** — no structural decay synergy, MEV footguns
- **Decay-Photo / Cherry-clone** — consumer-app surface area too risky for solo founder
- **Decay-Native VC equity** — legal mess
- **Decay-Native Will / Testator** — regulatory minefield, defer
- **Decay-Funded UBI** — Sybil failure mode dominates press coverage
- **Decay-Native Public-Goods QF** — Gitcoin shadow, derivative
- **Validator-as-Audience NFTs** — flavor, not structural

RWA strikes:
- **Carbon offsets** — reputation-burned post-2023 Verra collapse
- **DeFi options** — crowded space + CFTC hostile
- **Tokenized treasuries** — already crowded by Ondo / BlackRock BUIDL / Hashnote
- **B2C plays** (loyalty, gift cards, subscriptions) — drag, hard sell
- **Spectrum licenses, water rights, patents** — regulatory bar too high for 6-month sprint

## A2.7 Honest launch recommendation for solo + £100K + 6 months

If you ship one launch dApp:

**Ship HBCT.** Reasons:
- Concrete B2B customer (battery aggregators)
- Open data (GB grid)
- Lowest regulatory bar of the serious candidates
- Decay is dimensionally honest (electricity is the canonical non-storable)
- Demonstrates "the chain is for real-world things that genuinely expire"

If you ship two:

**Ship HBCT + the four-act self-referential structure (Mortis + Sentinel + Tombstone).** The HBCT is the commercial wedge; the four-act structure is the philosophical story. Together they say: "EvaporChain is real engineering for real markets, and it admits its own mortality." That's a complete narrative.

**Defer to V2:**
- EnergyCoin (YELLOW until 3 spec gaps close)
- The Gallery That Forgets (gallery partnerships > 6mo timeline)
- Time Capsules / Memorial Contracts (ship as features once Mortis lands)
- Decay-Native Memecoin (only if you have a separate person handling the meme campaign)

## A2.8 Updated build order — with launch dApp inserted

Insert into the Amendment 1 build order:

| Weeks | Add | Why |
|---|---|---|
| 1–4 | EnergyCoin spec work (3 spec gaps closed in writing) | Decision: ship later or kill |
| 8–14 | **Tombstone** primitive (alongside consensus + mempool) | 2-week add; cheapest profound primitive |
| 14–18 | **Mortis** death predicate + auto-mint rule | Pairs with EFH and PRP timeline |
| 16–20 | **Sentinel** autonomic controller (built on top of LLSA from A1.2) | Whitepaper centerpiece for "chain governs itself" claim |
| 18–24 | **HBCT** primitive + GB grid oracle + testnet demo | Launch wedge implementation |
| 22–26 | (post-mainnet-prep) Refresh-Pool Patronage covenant | Funds the chain's own audits + indexers + docs |

The Gallery That Forgets, Time Capsules, EU ETS, Decay-Native Memecoin, Decay-Native Reputation — all V2 (post-launch).

## A2.9 Doctrine update for application layer

Add to §10 doctrine:

14. **App-layer primitives must clear the "non-crypto press" bar.** If only crypto-Twitter would write about it, it's not a launch primitive — it's a feature.
15. **Lineage matters.** Cultural primitives must connect to a pre-crypto cultural lineage (sand mandalas, memento mori, Maya Lin, conceptual art) to land outside crypto.
16. **HBCT is the operating launch wedge** unless explicitly overridden. Every other dApp sits in the four-act philosophical narrative around HBCT.
17. **EnergyCoin doesn't ship until the 3 spec gaps are closed.** No exceptions; UST's marketing fraud is the failure mode to avoid.
18. **The four-act structure (Mortis / Sentinel / Tombstone / Genesis) is the chain's narrative spine.** Don't lose it in the engineering rush.

End of Amendment 2.

---

# Amendment 3 — Final Synthesis & Doctrine Closure

**Added:** 2026-04-28 (final pass, same day)
**Status:** This is the canonical TL;DR. If you read nothing else, read this. Updates supersede earlier sections where they conflict.
**Research thread closure:** with this amendment, the research phase for the May–Oct 2026 sprint is **closed**. Future sessions read this and start coding; do not propose new primitives unless an existing one is shown to fail.

## A3.1 The total stack — what EvaporChain has after one day's research

| Tier | Count | What it contains |
|---|---|---|
| **Tier 0 — closed-form theorems** | 5 | MCC (Jaynes), CFM (Crooks), CSLC (Shalizi-Crutchfield), LLSA (Coq invariants), EPV (decay-pruned versions) |
| **Tier 0 — supporting theorem-grade** | 6 | Sanov-Slashing, TUR Liveness Detector, Cμ-Gate, MDL-Shard, Causal-Cone Validator State, Crooks-MEV Refund |
| **Far-frontier math survivors** | 5 | Authenticated Energy-MERA (gated), p-adic Merkle, Tropical Plücker Light Client, Modular-Form Beacon, Braid-Group Sequencer Commitment |
| **Tier 1 — launch primitives** | 12 | Light-Cone Consensus, Evap-Antichain Mempool, Decay-Lamport Time, Singh-Lyapunov Fee Controller, Singh-Boltzmann Stake, Native Demurrage→Refresh Pool, Refresh Market, Lambda-Fold, EFH, Evaporated-Fork Certificates, Provable Retention Proofs, Linear-Affine-Decay VM |
| **Tier 2 — V2 primitives** | 19 | (see §4.2) |
| **Tier 3 — app-layer specialized** | ~16 | (see §4.3) |
| **Launch dApp candidates** | 3 | EnergyCoin (YELLOW), HBCT (launch wedge), Gallery That Forgets (V2) |
| **Self-referential narrative spine** | 4 | Genesis (Birth), Sentinel (Life), Tombstone (Small deaths), Mortis (Final death) |
| **Confirmed strikes** | 30+ | (see §4.4 and §A1.5 and §A2.6) |

**Total novel primitives across all tiers: ~70.**

That's more theorem-grade novelty than any L1 launch in history has assembled. It is also more than any solo founder can ship in 6 months. The doctrine doc is now complete enough that the only remaining question is **which subset actually ships**, and that question lives in code, not in more research.

## A3.2 The complete vocabulary of disappearance — the four-act narrative

EvaporChain is the first chain with a complete vocabulary for impermanence. Every act is structurally impossible on a chain that doesn't decay.

```
              BIRTH                    LIFE                   SMALL DEATHS              FINAL DEATH
       ┌──────────────────┐    ┌────────────────────┐    ┌───────────────────┐    ┌──────────────────────┐
       │      Genesis      │    │     Sentinel        │    │     Tombstone      │    │       Mortis          │
       │  with LLSA-checked│    │ autonomic governance│    │  32-byte commitment│    │  death certificate    │
       │  invariants (Coq) │    │ via decay-weighted  │    │  for every evapor- │    │  auto-minted as a     │
       │                   │    │  homeostasis        │    │  ated account      │    │  singleton NFT when   │
       │                   │    │                     │    │                    │    │  refresh pool ≤ ε     │
       └──────────────────┘    └────────────────────┘    └───────────────────┘    └──────────────────────┘
            "constitution         "homeostasis             "Maya Lin                "the chain that signs
             is a proof"           not legislators"          parallel"                its own death cert"
```

Bitcoin promises immortality (and quietly fails). EvaporChain promises mortality (and provably succeeds).

## A3.3 The three press lanes — which audience writes about which thing

| Launch primitive | Lead audience | Sample headline they'd write |
|---|---|---|
| **HBCT** Hour-Block Capacity Tokens | Industrial / energy press, FT, Bloomberg | "The first blockchain to natively price what physics already prices" |
| **Mortis + Sentinel + Tombstone** | New Yorker / Atlantic / Wired / MIT Tech Review | "The blockchain that signs its own death certificate" |
| **EnergyCoin** (post-spec-gap-closure) | Financial Times, Economist, central-bank research | "A currency whose value is a closed-form function of one physical constant" |
| **The Gallery That Forgets** (V2) | NYT Arts, Frieze, ArtForum, NYT Style | "An exhibition that closes by mathematics" |

These lanes are non-overlapping. A solo founder ships **HBCT + the four-act narrative** in the May–Oct sprint; defers EnergyCoin and Gallery to V2.

## A3.4 The honest launch recommendation

**Lead with HBCT. Wrap it in the four-act narrative. Defer EnergyCoin and Gallery.**

| Reason | What it gives |
|---|---|
| HBCT has a concrete B2B customer (battery aggregators) | Real revenue path |
| GB Elexon BMRS + ENTSO-E APIs are open | Solo founder can ship testnet demo without partner approval |
| Decay is dimensionally honest for electricity (canonical non-storable) | Math story is unassailable |
| Light reg bar (Ofgem/FERC capacity-market frameworks are utility-token-friendly) | No 5+ year regulatory grind |
| Existing energy chains use no decay primitive | First-mover novelty defensible |
| Mortis + Sentinel + Tombstone are cheap (8–10 weeks total) | Adds the philosophical spine almost for free |
| Combined narrative: "real engineering for real markets, admits its own mortality" | Crosses out of crypto press into mainstream |

**Don't ship at launch:**
- **EnergyCoin** — math is sound but 3 spec gaps must close (R(t) MEV semantics, refresh-pool solvency proof, fee-controller cross-derivative note). UST's marketing fraud is the failure mode to avoid. Ship V2.
- **The Gallery That Forgets** — gallery partnerships (Serpentine / Tate / MoMA PS1) take longer than 6 months. Online MVP can ship V2; IRL exhibition in Year 2.
- **Decay-Native Memecoin** — only ship if you have a separate person handling the meme campaign. Don't dilute focus.

## A3.5 The full build order — protocol + launch dApps + narrative spine

Consolidating Amendment 1 §A1.7 + Amendment 2 §A2.8:

| Weeks | Build | Layer |
|---|---|---|
| **1–2** | `evaporchain-energy-kernel` (single λ accumulator, conservation invariant, refresh pool) | Substrate |
| **1–2** | `evaporchain-mera-gate` empirical entropy measurement on Ethereum/Solana account-touch data | Tier 0 decision |
| **1–2** | `evaporchain-padic` crate (low-risk parallel) | Far-frontier |
| **1–4** | EnergyCoin spec work (close the 3 gaps in writing — decision: V2 or kill) | Math |
| **3–5** | Native Demurrage (passive accrual) | Tier 1 |
| **3–6** | `evaporchain-tropical` (Plücker commitments) | Far-frontier |
| **4–7** | Singh-Lyapunov Fee Controller + CFM closed-form extension + Sanov-Slashing | Tier 0 + 1 |
| **6–10** | Singh-Boltzmann Stake + Refresh Market + TUR Liveness Detector | Tier 0 + 1 |
| **8–14** | Light-Cone Consensus + Evap-Antichain Mempool + MCC fork choice + Causal-Cone Validator State | Tier 0 |
| **8–14** | **Tombstone** primitive (alongside above) | Self-referential |
| **10–16** | **LLSA** (MetaCoq + extraction) + EPV (parallel) | Tier 0 |
| **12–18** | Lambda-Fold + CSLC ε-machine + Cμ-Gate | Tier 0 |
| **14–18** | (if MERA gate passed) Authenticated Energy-MERA crate | Far-frontier |
| **14–18** | **Mortis** death predicate + auto-mint rule | Self-referential |
| **16–20** | **Sentinel** autonomic controller built on LLSA | Self-referential |
| **16–22** | Linear-Affine-Decay VM + MDL-Shard | Tier 1 + 0 |
| **18–24** | **HBCT** primitive + GB grid oracle + testnet demo | Launch dApp |
| **20–24** | EFH + PRP + Evaporated-Fork Certs + Decay-Lamport Time + Modular-Form Beacon | Tier 0 + 1 |
| **22–26** | Refresh-Pool Patronage covenant | Self-referential |

26 weeks > 6 months. Realistic launch is end-of-October MVP testnet, late-2026 mainnet candidate, early-2027 mainnet launch.

## A3.6 Updated naming guardrails (final)

Inherit from §A1.9 + §A2.9. Plus:

19. **Closed-form theorem grants you "Singh-X" naming.** No theorem, no name. Lyapunov-stable without a Lyapunov function = marketing.
20. **Cultural primitives must connect to a pre-crypto lineage** (Maya Lin, Banksy, Hirst, Goldsworthy, Abramović, sand mandalas, mono no aware, memento mori). If it doesn't fit a lineage older than crypto, it's crypto-incest.
21. **Every Tier-0 primitive must have its theorem cited in its source code.** Code comment at the type definition: `// Theorem: Shalizi-Crutchfield 2001 Optimal Prediction Theorem (J. Stat. Phys. 104).`
22. **No "thermodynamic" or "quantum" in marketing.** "Algorithmic state lifetime" or "exponential lifetime." Don't invoke Landauer.
23. **MERA gate must pass before MERA ships.** Repeat from A1.9; underline.

## A3.7 What "research is closed" actually means

Three things are off the table:

1. **No more candidate-name brainstorms.** We have ~70 primitives ranked. Adding a 71st is procrastination.
2. **No more agent rounds.** 11 agents have run today. Diminishing returns hard.
3. **No more "but what if we also had X?" exploration.** Pick the X you want from the existing tier table or accept that it's V2.

Three things remain valid as work threads:

1. **Math validation in parallel with coding** — Coq/TLA+ proofs for the conservation invariant, MCC steady state, LLSA invariant preservation. Runs alongside implementation; doesn't block code.
2. **Closing the 3 EnergyCoin spec gaps** — written notes, then decision V2-ship or kill. 1-week task.
3. **The MERA empirical gate** — 2-week measurement on real chain data; drives Week-14 decision.

If a future session wants to propose a new primitive, the bar is: **a published theorem the existing stack does not claim, plus a structural argument why the single-λ is required**. That bar is high deliberately.

## A3.8 The single sentence — final pick

After three rounds of refinement:

> **EvaporChain is the first blockchain whose consensus, fee market, light client, upgrade path, and history are all closed-form solutions of named theorems, parameterized by one constant λ — and the first chain to admit, at genesis, that it can die.**

Two clauses, two ideas, one chain. The first clause is the math. The second is the philosophy. Together they're the pitch.

If forced to one clause for marketing: pick the second.

If forced to one clause for academia: pick the first.

If you have one minute on a podcast: deliver both, in that order, and stop talking.

## A3.9 Closure

Doctrine doc is at version 1.0 + Amendment 1 + Amendment 2 + Amendment 3 = canonical. ~600 lines of Markdown covering every load-bearing decision from a single day of intense research with two SATYA-1 round-trips and 11 parallel research agents.

The next high-value action is one of:

1. Scaffold `evaporchain-energy-kernel` on the Minis
2. Run the MERA empirical entropy gate
3. Close the 3 EnergyCoin spec gaps in writing

If a future Claude session reads this and is asked "should we research more?" the answer is **no — build**. The single λ has been chosen. The four acts have been named. The customer has been identified. The strikes have been buried. Go.

End of Amendment 3.

---

# Amendment 4 — Frontier Ideas & Hypothetical Speculation

**Added:** 2026-04-29
**Status:** Two parts. Part 1 (§A4.1) is a paradigm-level decision that's still on the table for the May–Oct sprint. Part 2 (§A4.2 onward) captures hypothetical frontier ideas not yet proven, not yet in any tier — these are speculative parking spots for *future* consideration, not commitments. Marked clearly so future sessions don't promote them prematurely.

## A4.1 Paradigm-level option — Contractless L1

A paradigm-level alternative to "smart contracts as Turing-complete user code." Decision-ready, not speculative. Choosing this rewrites the smart-contract-layer track of the build order.

### The thesis

EvaporChain ships a small set of **typed, theorem-grade composable primitives** (HBCT, demurrage, refresh, evaporate, retain, fold, decay, slash, ε-compress, ledger-fold, etc.). dApps are composed by *parameterizing and connecting* primitives — not by writing arbitrary Turing-complete code.

| Standard L1 | Contractless EvaporChain |
|---|---|
| Solidity / Move / EvaporScript Turing-complete VM | Small typed library of named primitives |
| User writes arbitrary code, audited per-contract | User selects + parameterizes audited primitives |
| Reentrancy / DoS / overflow as ongoing risk class | Exploit classes literally cannot exist (no user code) |
| dApp = bytecode + state | dApp = JSON/CBOR config of primitive composition + parameters |
| Audit surface = total deployed bytecode | Audit surface = the chain itself, fixed and finite |

**Lineage** (yes, this has precedent): Bitcoin Script (limited stack VM, no Turing-completeness), Cardano Marlowe (financial DSL only), Pact (Kadena, capability-based). None of these are theorem-grade. EvaporChain's primitives would each be backed by a published closed-form theorem — the first L1 of that kind.

### Anti-feature manifesto extension (4th refusal)

If adopted, the manifesto from §2 grows to four refusals:

1. No permanent data storage
2. No immutable data structures
3. No bridges to chains without lifecycle alignment
4. **No Turing-complete user code**

Coherent extension of "the chain that says no."

### Trade-offs

| What it costs you | What it gives you |
|---|---|
| No third-party dApp ecosystem in the Solana / Ethereum sense | Audit surface collapses to a fixed set of crates |
| Expressiveness ceiling fixed by primitive set | Security claim is unbeatable: "zero contract exploits since launch — by design, not by luck" |
| You become responsible for primitive curation (every new primitive needs LLSA-checked invariant proofs) | Doctrine philosophy goes all the way down — single λ, conservation, no permanent storage, no arbitrary code |
| VC narrative is harder ("where do we deploy our app?" "you don't deploy, you compose") | Mainstream press story sharpens: *"The blockchain that won't let you build."* |
| | Build effort lower for solo founder — drops `lad-vm`, `capability-vm`, `dp-vm`, `total-script` from the smart-contract-layer track. Saves ~6–8 weeks. |

### Crate impact if adopted

- **Drop from sprint:** `lad-vm`, `capability-vm`, `dp-vm`, `total-script`. The Allen-Decay opcodes folded into primitive implementations rather than user opcodes.
- **Add:** `evaporchain-composer` — a small composition language (declarative, typed at submission, no execution beyond parameter binding). 1 new crate, ~3 weeks effort.
- **Net:** −4 crates, +1 crate, ~6–8 weeks saved.

### Decision deferral

This decision **does not need to be made today**. The energy kernel (Week 1–2) is the same code regardless. The fork happens at Week 4–6 once the kernel is running. By then, a few weeks of working code will tell you more about what you want than another doctrine pass would.

If chosen at Week 6: drop the listed VM crates, add `evaporchain-composer`, rewrite the smart-contract-layer track of the build order. If not chosen: keep the existing LAD-VM-centered plan.

## A4.2 Hardware story — silicon, FPGAs, validator power budget

**Status:** V2+ (post-mainnet research). Captured here for the record so future sessions don't repeat the analysis.

### Open questions

1. Does **Lambda-Fold** require custom silicon to hit production-grade verifier latency? (Today: minutes per fold on CPU. Question becomes interesting only after CPU performance is measured against real chain load.)
2. Are **FPGA-accelerated validator nodes** worth a reference design? (Solana ecosystem has explored ASIC; no L1 has a *thermodynamics-aware* hardware spec.)
3. Could EvaporChain publish a **power-budget validator class** — "your node must dissipate no more than X watts per λ-tick" — as a hardware-side anti-Sybil mechanism?

### Why deferred

- ASIC tape-out: ~$1M and 9–12 months. Out of scope for solo + £100K.
- FPGA iteration: 2–3 months per cycle. Could eat the entire 6-month sprint.
- "Decay validators dissipate heat" sounds romantic but doesn't add to Tier 0 theorems. Marketing, not engineering, until measured.
- Capital risk too high pre-launch.

### When it becomes interesting

Post-mainnet, once Lambda-Fold is in production on CPU and load metrics show a clear bottleneck. Then the hardware case writes itself; until then, premature optimization.

## A4.3 Hypothetical frontier ideas — speculative parking spots

**Status: NONE OF THE FOLLOWING ARE COMMITMENTS.** They are speculative ideas captured for future consideration, marked with severity flags. A future session must NOT promote any of these to Tier 1/2/3 without:

- Identifying the specific theorem or formalism that backs the idea
- Verifying the idea is genuinely novel (not already in strikes or other tiers)
- Applying the same single-λ structural-decay filter as Tier 0 / Tier 1
- Re-confirming with Satyawan

Severity flags: **PROMISING** = mathematically grounded, decay synergy plausible, worth a future agent round; **WILD** = intellectually striking but unclear if math transfers; **SPEC** = pure speculation, may not survive scrutiny.

### A4.3.1 Information-Bottleneck Validators **(PROMISING)**

Tishby et al. *Information Bottleneck Method* (1999). Validators learn the **minimum sufficient statistic** for predicting the next block, via the IB principle: maximize I(T;Y) − β·I(X;T) where T is the validator's compressed view, X is full state, Y is next block.

Cousin to **CSLC (Tier 0)** but learning-based rather than CSSR-reconstructed. Could give validators a principled way to *forget* irrelevant past history while maintaining predictive sufficiency.

Energy synergy: structural — β coupling parameter could be set to chain λ.
Math feasibility: research-ready in 12+ months.
Citations: Tishby-Pereira-Bialek 1999; Tishby-Zaslavsky 2015 deep IB.

### A4.3.2 Self-Annealing Validator Set **(PROMISING)**

Simulated annealing applied to validator selection. "Temperature" parameter T(t) decays with chain energy: high T early (exploration), low T late (exploitation). Converges to optimal stake distribution under a global "honest validator likelihood" objective.

Cousin to Singh-Boltzmann Stake but with a *cooling schedule* that is itself a function of λ.

Energy synergy: structural — cooling rate = λ.
Math feasibility: research-ready in 6 months.
Citations: Kirkpatrick-Gelatt-Vecchi 1983; Geman-Geman 1984.

### A4.3.3 Holonomy-Based State Verification **(WILD — STRUCK 2026-04-29)**

> **STRUCK.** No operational security reduction; Fisher-metric variant (ORMB) already in §A1.4 of Amendment 1.

Original entry preserved below for audit trail:



State manifold equipped with a Riemannian connection (Levi-Civita). State changes accumulate **holonomy** when transported around closed loops in state-space. Light clients verify state by walking small loops and checking that the holonomy matches a published reference.

Tamper-evidence at a level beyond hashing — a state perturbation changes the geometry, not just the bytes.

Energy synergy: connection coefficients could be parameterized by λ.
Math feasibility: speculative; needs discrete differential geometry on state graph (Forman-Ricci or Ollivier-Ricci already in §1 of A1.4 as ORMB).
Citations: do Carmo *Riemannian Geometry* 1992; Forman 2003 discrete Morse theory.

### A4.3.4 Causal-Bayesian-Network Smart Contracts **(PROMISING — only if Contractless rejected)**

Judea Pearl's *do-calculus* as native VM primitive operations. Contracts that natively reason counterfactually: "what would the state be if X had not happened?" Smart contracts as causal-Bayesian networks, not state machines.

If Contractless L1 is adopted (§A4.1), this is moot. If not, this could differentiate LAD-VM from Move/EVM.

Energy synergy: weak; structural-decay link is cosmetic.
Math feasibility: research-ready in 12+ months.
Citations: Pearl 1995 *Causal Diagrams for Empirical Research*; Pearl 2009 *Causality* 2nd ed.

### A4.3.5 Stigmergic Consensus **(WILD — STRUCK 2026-04-29)**

> **STRUCK.** No Byzantine safety proof transfers from biological stigmergy math. "Bio-inspired" ≠ provable consensus.

Original entry preserved below for audit trail:



Camazine et al. *Self-Organization in Biological Systems* (2001). Validators leave decay-signed "traces" in the environment that other validators respond to. Consensus emerges without explicit message-passing — bio-inspired but rigorously based on stigmergy theory.

Cousin to gossip protocols but with decay built into trace lifetime.

Energy synergy: structural — trace decay rate = λ.
Math feasibility: research; would need a formal proof of safety/liveness under Byzantine adversary, which doesn't obviously transfer from biological systems.
Citations: Grassé 1959 (original stigmergy); Theraulaz-Bonabeau 1999.

### A4.3.6 Decay-Native Probabilistic Programming **(WILD — STRUCK 2026-04-29)**

> **STRUCK.** zkML production ceiling (~tens-of-millions of params today) makes verifiable Bayesian inference at L1 unrealistic; decay-prior coupling would be cosmetic.

Original entry preserved below for audit trail:



VM where **probabilistic programs** are first-class — contracts express Bayesian inference natively (Stan/Pyro/Gen-style). State transitions are draws from posterior distributions, not deterministic updates. Validators reach consensus on the *distribution*, not the realization.

Closest precedent: nothing at L1. Pyro (Uber) and Gen (MIT) are off-chain.

Energy synergy: weak unless decay parameterizes a prior.
Math feasibility: speculative; verifiable inference at L1 is hard (zkML still has the production ceiling at ~tens-of-millions of params).
Citations: Carpenter et al. 2017 *Stan*; Bingham et al. 2019 *Pyro*; Cusumano-Towner et al. 2019 *Gen*.

### A4.3.7 Information-Geometry Consensus **(WILD — STRUCK 2026-04-29)**

> **STRUCK.** Fisher metric computation at L1 throughput is prohibitive; not realistic as a runtime primitive.

Original entry preserved below for audit trail:



Amari's information geometry. State distributions live on a statistical manifold equipped with the Fisher-Rao metric. Consensus = following geodesics on this manifold; disagreement = geodesic distance.

Cousin to **Singh-Lyapunov Fee Controller** but generalised to consensus state, not just fees.

Energy synergy: structural if energy parameterizes the Fisher metric.
Math feasibility: speculative; computing Fisher metrics at L1 throughput is expensive.
Citations: Amari 1985 *Differential-Geometrical Methods in Statistics*; Amari-Nagaoka 2000.

### A4.3.8 Autopoietic Chain **(PROMISING — partly already shipped)**

Maturana-Varela 1972 *Autopoiesis*. A self-producing, self-maintaining system. The chain produces and maintains its own components: refresh pool funds audits (already in Refresh-Pool Patronage); Sentinel adjusts parameters within bounds (already in Sentinel); LLSA gates upgrades (already in LLSA).

EvaporChain is *already* partly autopoietic. This idea is mostly **renaming what's already there into a coherent biological frame**. Could become a marketing / philosophical framing layer for the four-act narrative spine.

Energy synergy: structural; autopoiesis requires a metabolism, decay supplies it.
Math feasibility: ship-now as a *framing*; the underlying primitives already exist.
Citations: Maturana-Varela 1972, *De máquinas y seres vivos*; English: 1980 *Autopoiesis and Cognition*.

### A4.3.9 Decay-Native Time Crystal **(SPEC — STRUCK 2026-04-29)**

> **STRUCK.** Energy synergy is cosmetic at best; condensed-matter physics doesn't transfer cleanly to L1 dynamics.

Original entry preserved below for audit trail:



Wilczek 2012 (theory); Else et al. 2016 *Floquet Time Crystals* (experimental). Condensed-matter physics phenomenon: a system that breaks time-translation symmetry, exhibiting periodic structure in time without periodic driving.

Could a chain have a built-in long-period recurrence in some metric (e.g., refresh-pool oscillation, validator-set rotation cycle) that is *exact* and emergent rather than imposed? Speculative — unclear what advantage it would confer.

Energy synergy: cosmetic at best.
Math feasibility: SPEC. Likely doesn't transfer cleanly from condensed-matter to L1.
Citations: Wilczek 2012 *Quantum Time Crystals*; Else-Bauer-Nayak 2016 *Floquet Time Crystals*.

### A4.3.10 Quaternionic / Geometric-Algebra State for Specific Sub-Domains **(SPEC — STRUCK 2026-04-29)**

> **STRUCK.** Already in the main strikes list (§4.4). Niche-domain salvage doesn't justify a parking spot here.

Original entry preserved below for audit trail:



Already in the strikes list as a general state primitive (continuous, no security reduction). But for *specific sub-domains* (e.g., physical-world simulation oracles, robotics on-chain, IoT spatial reasoning), Clifford algebra could be a useful native type. Wouldn't replace the main state model; would be a typed namespace for specific use cases.

Energy synergy: weak.
Math feasibility: research-ready as off-chain analytic, speculative as on-chain primitive.
Citations: Hestenes 1966; Doran-Lasenby 2003 *Geometric Algebra for Physicists*.

### A4.3.11 Renormalization-Group Consensus Phase Map **(PROMISING — extends WSBF)**

Already in Tier 2 as Wilson-Singh Block Flow (WSBF). The hypothetical extension: produce a **phase diagram** of consensus regimes under varying λ, validator count, and adversary fraction. Map "fixed points" of the RG flow to operational regimes (liveness-stable, safety-stable, frozen, chaotic). First-of-its-kind diagnostic for L1 operators.

Energy synergy: structural; RG flow is generated by λ.
Math feasibility: research-ready in 12 months as analytic tooling; not a runtime primitive.
Citations: Wilson 1971; Cardy 1996 *Scaling and Renormalization in Statistical Physics*.

### A4.3.12 Decay-Native Topological Quantum Error Correction Inspired Validation **(WILD — STRUCK 2026-04-29)**

> **STRUCK.** Weak decay synergy; "inspired by topological QEC" ≠ the actual math. Falls below the theorem-grade bar.

Original entry preserved below for audit trail:



Borrow from topological QEC (Kitaev surface codes, Bombin color codes). Not quantum hardware — the *combinatorial structure* of topological codes (anyons, lattice gauge theory) for arranging validator votes such that local errors don't propagate.

Different from existing strike (#5 Anyonic Braiding Tx Ordering — that was for ordering, this is for *error correction* in voting topology).

Energy synergy: weak; would need contrived coupling.
Math feasibility: research-grade; promising in principle, no L1 has tried.
Citations: Kitaev 2003 *Fault-tolerant quantum computation by anyons*; Bombin-Martin-Delgado 2006.

## A4.3.13 Curation pass — 2026-04-29

Seven of the twelve speculative ideas were struck after the doctrine's first re-read. Surviving speculation list (the only items future sessions may consider promoting, with appropriate rigor):

| # | Survivor | Severity | Why it survived |
|---|---|---|---|
| A4.3.1 | **Information-Bottleneck Validators** (Tishby) | PROMISING | β coupling = λ; structural decay synergy; mathematically grounded |
| A4.3.2 | **Self-Annealing Validator Set** | PROMISING | Cooling rate = λ; clean structural fit; standard simulated-annealing math |
| A4.3.4 | **Causal-Bayesian-Network Contracts** (Pearl) | PROMISING — *only if Contractless L1 is rejected* | Counterfactual reasoning as VM primitive; dies under Contractless paradigm |
| A4.3.8 | **Autopoietic Chain** (Maturana-Varela) | PROMISING — *partly already shipped* | Coherent biological framing for Refresh-Pool Patronage + Sentinel + LLSA |
| A4.3.11 | **RG Consensus Phase Map** (extends WSBF) | PROMISING | Diagnostic tooling; extends a Tier-2 primitive that's already locked |

Struck items with reasoning:

| # | Idea | Reason for strike |
|---|---|---|
| A4.3.3 | Holonomy-Based State Verification | No operational security reduction; Fisher-metric variant (ORMB) already in §A1.4 |
| A4.3.5 | Stigmergic Consensus | No Byzantine safety proof from biological stigmergy math |
| A4.3.6 | Decay-Native Probabilistic Programming | zkML production ceiling; decay-prior coupling is cosmetic |
| A4.3.7 | Information-Geometry Consensus | Fisher metric at L1 throughput is prohibitive |
| A4.3.9 | Decay-Native Time Crystal | SPEC; energy synergy cosmetic |
| A4.3.10 | Quaternionic / Geometric-Algebra State | Already in strikes (§4.4) |
| A4.3.12 | Topological QEC Validation | Weak decay synergy; "inspired by" ≠ the actual math |

The struck entries remain visible in §A4.3 for audit trail (so a future session can see what was considered and why it was rejected) but **must not be promoted to any tier**. They are cumulative strikes on top of the main strikes list (§4.4 + §A1.5 + §A2.6).

## A4.4 Discipline for these speculations

A future Claude session reading this section must obey:

24. **None of §A4.3 is locked.** Treat as a parking lot.
25. **No primitive moves from §A4.3 to Tier 1/2/3 without** (a) the specific theorem cited, (b) the structural-decay test passed, (c) Satyawan's confirmation.
26. **The Contractless L1 paradigm (§A4.1) is decision-ready.** Defer the choice to Week 4–6, but don't add it to a tier until then.
27. **Hardware story (§A4.2) is V2+.** Don't propose it for the May–Oct sprint.
28. **Don't propose a new speculative primitive in this section** unless it passes the same filter as Tier 0: published theorem + structural-decay requirement.
29. **The 7 entries struck on 2026-04-29 (§A4.3.13) are dead.** Do not re-litigate. Reasoning is preserved with each entry; the bar to revive is the same as Tier 0 (published theorem + structural-decay test + Satyawan's explicit confirmation).

End of Amendment 4.

---

# Amendment 5 — Singh-Named Application-Layer Frontier (5 Domains)

**Added:** 2026-04-29
**Method:** 5 parallel application-layer frontier agents, each filtered for "Singh-namable + structural-decay + would land in non-crypto press." 100+ candidates considered, 25 survivors locked.
**Status:** Application-layer addendum to Amendment 2. These are personally-namable primitives spanning smart contracts, marketplaces, NFTs, wallet UX, and consumer apps. **All "Singh"-prefixed primitives below are personal inventions where Satyawan Singh authored or substantially refined the math/mechanism — naming guardrail #19 from §A3.6 applies.**

---

## A5.1 Smart Contract Paradigms — three lock-grade primitives

These are paradigm-redefining; not "VM with feature X" but genuine alternative formal foundations.

### Singh Strategy Machines (SSM) — game-semantic contracts

**Formalism:** Hyland-Ong-Nickau arenas + Abramsky-Jagadeesan-Malacaria innocent strategies. A contract is an **arena** A; a program is an **innocent strategy** σ : A. Execution = play between Proponent (contract code) and Opponent (callers / adversary / environment). PCF-completeness via HO games.

**Decay synergy — STRUCTURAL:** Game arenas have *justification pointers* (a move justifies later moves). Energy budget = bounded depth of the justification tree. **A move is legal iff its justifier still has positive λ-residual.** Decay is the *visibility condition* on plays. Unjustified-by-decayed-move ⇒ illegal play ⇒ rejected at strategy level.

**Pitch:** *"the contract is a proof you can win against any adversary, mechanically."*

**Build:** crate `singh-strategy-vm`. Restricted fragment (innocent + well-bracketed + single-threaded) shippable in 2026.

**Citations:** Hyland-Ong "On Full Abstraction for PCF" 2000; AJM 2000; Ghica-McCusker IPA 2003.

### Singh-Bennett Asymmetric VM (SBAV) — reversible compute, decay as sole entropy export

**Formalism:** Bennett 1973 reversible TM; Janus reversible imperative language (Yokoyama-Glück 2007); Landauer 1961 (irreversibility ⇔ entropy export ⇔ energy cost).

**Decay synergy — STRUCTURAL (philosophically the cleanest in the entire stack):** every classical opcode is reversible (zero-energy in the limit); **only `decay(λ)` exports entropy and is the unique irreversible primitive.** Not analogy — Landauer literally. State at block t is bit-for-bit recoverable from t+k *except for the λ-decay trace*, which is the chain's thermodynamic arrow.

**Pitch:** *"the first computer system where the laws of thermodynamics dictate which operations cost gas."*

**Build:** crate `sbav-vm`. Janus + R-WHILE compile cleanly. Reversible ledger uses append-only-with-undo log = the chain *is* the trace. `op DECAY(addr, lambda) -> Entropy` marked `#[irreversible]` and the *only* opcode lacking a `Reversible` impl.

**Citations:** Bennett 1973; Yokoyama-Glück PEPM 2007; Danos-Krivine CONCUR 2004; Landauer 1961.

### Singh-Girard Bang-Whimper Types (SGB) — linear logic !/? exponentials at L1

**Formalism:** Girard 1987 linear logic. `!T` admits duplication (decay-immune); `?T` admits silent loss (decay-eligible); de Morgan duals. Light Linear Logic (Girard 1998) and Soft Linear Logic (Lafont 2004) give *polytime* fragments.

**Decay synergy — STRUCTURAL:** the duality is exact. λ-decay *is* the categorical weakening rule applied automatically. Existing affine/linear chains (Move, Sui) only use `!`'s shadow; `?` has never been exposed as a contract type because no chain had a principled "decay" primitive to pair with it.

**Pitch:** *"first system where ephemerality is a type, not a runtime convention."* Type theorists have been waiting 30 years for industrial use of `?`.

**Build:** crate `sgb-types`. SLL/DLAL fragment compiles via Mackie's Geometry-of-Interaction Machine; runtime = GoIM token machine. Gives polytime gas bounds *for free* (Lafont 2004) — kills whole DoS classes.

**Citations:** Girard TCS 1987; Girard "Light Linear Logic" 1998; Lafont SLL 2004; Mackie GoIM POPL 1995.

### Strikes from this round (smart contract)

Lambek-Scott CCC contracts, process algebras (CSP/CCS/π) as contract primitives, realisability (Kleene K1/K2), Total FP (Cardano-style; not novel), modal/epistemic contracts, uniqueness types (subsumed by SGB), CPS-as-VM (engineering choice not paradigm), BPMN workflows, string diagrams (notation only), higher-order contracts, refinement types (Move Prover already does this), logic-programming contracts.

---

## A5.2 Marketplaces — six primitives with structural-decay clearing

### Singh Future-Self Vault (SFSV) — sell your future self's claim

User posts an energy-denominated commitment that pays out to their *own future address* when a *decay-state predicate* clears. Secondary market clears third parties bidding for that future claim at a discount. Energy-decay is the unforgeable clock no validator collusion can manipulate.

**Pitch:** *"You can now sell your future self's money — and your future self can't sue."*

**Build:** 6 weeks. Cheapest survivor; mainstream-press friendly. **Build first.**

### Singh Decay-Dutch Continuous Auction (SDDC) — joint clearing on (price, λ)

Bidders commit not just price but their *willingness to hold*; high-λ-tolerant bidders win at lower prices. **Foundational mechanism**, not standalone market — underlies SAP, SCL, SFSV, SHLM. Build once, reuse everywhere.

**Build:** 3 weeks. Build alongside SFSV.

### Singh Skill Half-Life Market (SHLM) — credentials that expire by skill-specific λ

Skill credentials decay at skill-specific rates (Python λ ≈ 18mo, COBOL λ ≈ 8yr, prompt-engineering λ ≈ 4mo). Holders refresh via micro-assessments. Employers post bounties for fresh-skill tokens above decay threshold.

**Pitch:** *"Your Python skills now expire — and employers can see the timestamp."* Rides AI-displacement wave perfectly. $50B B2B TAM (recruiter market).

**Build:** 12 weeks; biggest commercial market of the set.

### Singh Attention Pool (SAP) — Attention Quanta with cognitive-decay-tied pricing

Each verified human wallet emits Attention Quanta (AQ) per minute, capped, decaying at λ_attn ≈ 45min (empirical forgetting curve). Advertisers/creators bid energy into a pool that pays out only against AQs whose decay-state is above threshold at attestation moment. **A 5-minute-old AQ is worth more than a 25-min one** because the human is statistically still cognitively-engaged.

**Pitch:** *"For the first time, your attention has a half-life — and a price."* NYT Business desk material.

**Build:** 8 weeks. Hardest part = gaze-attestation TEE circuit.

### Singh Capability Lease (SCL) — permission market with structural revocation

Capability tuples (subject, verb, object, λ_cap) minted as fungible-non-transferable leases. At expiry, the underlying right snaps back atomically — no revocation tx, no race. Listed on a CL-AMM-style book.

**Customer:** DAOs delegating treasury auth (Gnosis Safe pain), MEV searchers leasing per-block builder permissions, AI agents needing time-boxed wallet authority.

**Pitch:** *"The first blockchain where permissions can't outlive their purpose."*

**Build:** 10 weeks. B2DAO + B2-AI-agent native.

### Singh Counter-Decay Insurance (SCDI) — premium grows with policy age

Counter-conventional: premiums increase as policy ages because insured event is "the asset hasn't decayed yet." Defer to V2 — niche, B2DAO only.

### Strikes from this round (marketplaces)

Prediction markets with decay, reputation markets, attestation markets, time-slice markets (5), bonding-curve-decay, subscription secondary, cancellation, sentiment, calendar, reverse marketplaces, provenance, decay sport-betting, insurance reverse-decay (collapses to HBCT), capacity-beyond-electricity (collides with HBCT/ESL), memory markets (storage half = ESL).

---

## A5.3 NFT Primitives — five lock-grade decay-native art primitives

### Singh-Sabi (Patina Tokens) — NFTs that age toward "ruined-beautiful"

Mint deposits fixed energy budget. As λ-decay drains, *visual entropy* increases on a deterministic curve: edges fray, palette desaturates, surface accrues procedural cracks/foxing/staining seeded by tokenId. Decay is *aesthetically tuned* — never reaches zero, reaches a "ruined-beautiful" asymptote (~15% energy floor). Owner cannot pause; only witness.

**Cultural lineage:** wabi-sabi (Sen no Rikyū, 16th c.); kintsugi; Tarkovsky's *Stalker* set decay; Basinski's *Disintegration Loops*; Banksy's *Love is in the Bin*.

**Pitch:** *"A blockchain that lets art age like paper."*

**Build:** 6 weeks. Decay shader (GLSL/WGSL) + deterministic procedural cracks + on-chain energy hook.

### Singh-Posthuma (Sealed Testaments) — confessional NFTs revealed on certified death

Mint commits encrypted payload. Decryption key held by threshold-secret-sharing committee. Decay suspended while issuer is verifiably alive. On confirmed death, committee reveals key → payload becomes public → λ-decay begins on the now-public NFT → fades to permanent on-chain marker.

**Cultural lineage:** Catholic confessional seal; Pessoa's trunk; Kafka's Brod betrayal; Joan Didion's *Year of Magical Thinking*.

**Pitch:** *"the first NFT that's a deathbed confession."*

**Build:** 12 weeks. Death-oracle is the painful primitive; threshold crypto is solved (FROST). Highest mainstream-press potency of the NFT set. *New Yorker*-grade.

### Singh-Migrant (Wanderwrits) — NFTs that die if held still

Each NFT has *resting threshold* (~30 days). Energy decays normally; transfer to a *new* wallet refunds a fraction. Stay still past threshold → λ doubles; past 60 days → quadruples. Must keep moving through novel hands or it evaporates.

**Cultural lineage:** Trobriand kula ring (Malinowski 1922); chain letters; geocaching; the Olympic torch; Marcel Mauss's *The Gift* (1925).

**Pitch:** *"the NFT that dies if you keep it."*

**Build:** 4 weeks. Cheapest viral wildcard.

### Singh-Heir (Patrilithic Tokens) — kin-graph heirloom NFTs (renamed from "Singh-Lineage" to resolve wallet collision)

Mint binds NFT to a *kin-graph* DAG of attested kinship relations. Generational transfer (parent→child edge) refreshes 80% energy. Non-kin transfer refreshes 0%. Across ~3 generations of dormancy, evaporates.

**Cultural lineage:** primogeniture; Japanese daimyō sword inheritance; Torah scrolls; signet rings; Mann's *Buddenbrooks*.

**Build:** 10 weeks. Kin-graph contract is a real research artefact (bilateral-attestation, succession-on-death). Defer to Year 2.

### Singh-Resonance (Vital-Sign NFTs) — engagement-coupled decay

λ inversely coupled to engagement (views, on-chain reactions, transfers, derivative mints). Loved art slows toward immortality; ignored art accelerates toward zero.

**Pitch:** maps directly to attention-economy critique (Tristan Harris, Jenny Odell *How to Do Nothing*). Risk: looks like "Black Mirror but on chain" — needs careful framing as critique.

**Build:** 8 weeks (4 if Lens-equivalent social graph already exists in-stack).

### Strikes from this round (NFTs)

Penalty NFTs, Pheromone NFTs, Metabolic NFTs, Counterfactual NFTs, Genealogical NFTs, Witness NFTs, Ouroboros NFTs, Decay-Ranked Curation NFTs, Memento NFTs, Time-Capsule Souvenir NFTs, Kintsugi NFTs (subsumed by Singh-Sabi).

---

## A5.4 Wallet UX Paradigms — three-paradigm stack

### Singh-Triage (EvaporWallet-Triage) — wallet opens on inbox, not balance

Wallet opens not on a balance — on an **inbox**. "3 items decay today" with swipe actions: Refresh / Let Die / Archive-to-ghost. Below the fold: "Tomorrow (7), This Week (24), Healthy (137)." Pull-to-refresh literally refreshes the top item's chain energy. Balance is a secondary tab.

**Cultural lineage:** Superhuman email triage; Things 3 Today view; Linear inbox; Hey.com Imbox/Feed split. **No crypto wallet has shipped this aesthetic.**

**Pitch:** *"the wallet that finally treats crypto like adult software."*

**Build:** 6–8 weeks. **Ship first** — highest design-press leverage, lowest build cost, most defensible aesthetic moat.

### Singh-Lineage (EvaporWallet-Lineage) — graduated dormancy-based inheritance

Wallet has a second screen called Lineage: family tree showing primary key + designated successors with dormancy thresholds. *"If I'm silent 90 days, my daughter's key gains 25% authority; 180 days, 50%; 365 days, full."* Per-asset posthumous designation. Real-time visual of your own digital mortality.

**Cultural lineage:** Apple Legacy Contact (closest precedent — but reactive, not graduated); Google Inactive Account Manager.

**Pitch:** *"crypto solves inheritance."* Highest mainstream-press potency of the wallet set — FT, NYT personal finance, Atlantic.

**Build:** 10–14 weeks. Legal-UX surface is the long pole, not the code.

### Singh-Heartbeat (EvaporWallet-Pulse) — ambient pulse encoding wallet vital signs

Persistent ambient signal (visual + haptic) encoding aggregate wallet energy as a pulse rate. Healthy: slow 60bpm green pulse. Decay below threshold: arrhythmic red. Apple Watch complication shows sparkline of wallet's heartbeat over 24h.

**Cultural lineage:** Apple Watch heart-rate haptics; Tesla heartbeat-on-screen; Nest Leaf icon; Oura Ring rest signal.

**Pitch:** *"your crypto wallet has a pulse."*

**Build:** 5–7 weeks. Wired Gear section bait.

### Singh-Counsel (EvaporWallet-Conversational) — chat-first AI wallet (defer)

Primary interface is a chat thread with an LLM agent that has read access to full decay state. Proposes refresh prioritisation, energy-budget allocation, dormancy planning. **Ship last** — every wallet will claim "AI" by Q3 2026; differentiation hinges on decay-native framing.

**Build:** 8–10 weeks. Defer until Triage establishes the aesthetic.

### Strikes from this round (wallet)

Bleeding Wallet (anxiety-porn), Refresh-Calendar (component not paradigm), standalone Decay-Native Notifications (feature inside Heartbeat), Decay-Aware Default-Refresh (settings panel), Time-of-Day Wallet (depends on chain adopting one-block-per-day), Decay-Pricing UI (micro-feature), Phantom Limb Wallet (morbid for daily driver), Decay Gamification (commodity), Decay Photography (niche), Hardware-only Decay Wallet (accessory not flagship), Privacy-by-Decay UX (feature inside Counsel), Decay-Counterfactual Wallet (research tool), Memorial Wallet (folds into Lineage), Visual State Compression UI (inscrutable to non-technicals).

---

## A5.5 Consumer Apps — four lock-grade non-crypto-press primitives

### Singh Letter / ChildKey — sealed letters unlocked by recipient's age

Parents seal text/voice/photo/video to a child, **locked by age-of-recipient (not date)**. Chain holds encrypted blob; decryption key materializes when child's verified DID reaches unlock age. Parent dies? Seal still opens on schedule. *"Letter to my daughter at 18, recorded when she was 3, opens whether I'm alive or not, and no Google Drive admin can lose it."*

**Decay synergy: inverted decay** — chain runs decay backward to compute "energy-time-to-unlock." Same primitive, opposite sign. Genuinely novel.

**Customer:** new parents (3.6M US births/yr, 600K UK), grandparents, terminally ill patients.

**Cultural lineage:** FutureMe.org (~3M users); *Letters Against Depression*; *To My Future Daughter* (Maria Shriver bestseller); Encore hospice letters.

**Pitch:** *"Today Show segment writes itself."*

**Build:** 3–4 months. Encryption is straightforward (threshold cryptography on validator quorum). iOS app polish is the long pole.

**Singh names:** Singh Letter (primitive), ChildKey (unlock-by-age key derivation), Singh Vault (blob layer). **Build first.**

### MnemoChain / Singh Curve — Anki on-chain with FSRS forgetting curves

Every flashcard is an on-chain object with half-life equal to its scientifically-modeled forgetting curve (Ebbinghaus → SM-2 → FSRS, Wozniak/Ye 2023). When card's "memory energy" decays past threshold, surfaces for review. Correct recall *re-energizes* with longer half-life; wrong recall collapses it. The chain literally **is** the spaced-repetition scheduler.

**Decay synergy:** structural — this is the *only* candidate where decay isn't metaphor, it's the literal cognitive-science primitive. Without decay, you have Quizlet.

**The moat:** cards become **portable cognitive credentials.** "I have provably reviewed Spanish vocab card #4471 across 312 sessions over 4 years" is a real attestation a university or employer can verify. Anki decks aren't portable; MnemoChain decks are.

**Customer:** 20M Anki users globally (med students, language learners, MCAT/USMLE/LSAT prep), 80M Duolingo MAU. $5B+ education credentials market.

**Cultural lineage:** Anki (Damien Elmes 2006); SuperMemo (Wozniak 1985); Duolingo. None on-chain. None portable.

**Pitch:** *"Anki that proves you studied."* Med Twitter would adopt this in a week.

**Build:** 4–6 months. FSRS is open source (MIT). Hardest part is mobile UX, not consensus.

**Singh names:** Singh Curve (per-card decay function), Mnemo Trie (on-chain card store).

### WitnessFit / Singh Streak — wearable + chain attestation streaks

Apple Watch / Oura / Whoop / Fitbit pushes daily attestation to chain (workout completed, 8h sleep, 10K steps). Streak token has decay = 1 day; miss → token collapses. Optional: stake EnergyCoin on your own streak.

**The moat:** streak credential is **portable** across wearable vendors (impossible today — switching from Fitbit to Apple Watch loses your streak).

**Customer:** quantified-self / r/getdisciplined / Andrew Huberman audience. ~15M Strava users, 100M+ wearable owners.

**Cultural lineage:** Streaks (iOS app), Duolingo streaks, Strava, Beeminder.

**Build:** 5–7 months. HealthKit, Google Fit, Oura API integrations are the long pole.

**Singh names:** Singh Streak (primitive), WitnessFit (product), Decay-Bond (staking variant).

### GraveGraph / Singh Mortis — mortality-aware social network

Every profile has visible biological-age halo. Posts, photos, friend-tokens decay at rates indexed to user's age — 25-year-old's posts last decades, 80-year-old's posts decay in months and UI shows it. On verified death, profile auto-converts to Memorial Contract, undecayed artifacts crystallize permanently. **The network is aware that everyone on it will die, and treats it as default UX rather than an edge case Facebook patches in 2015.**

**Customer:** 35–65 demographic. People who've watched Facebook auto-suggest a dead parent for "people you may know."

**Cultural lineage:** none at scale. *We Are Not Really Strangers* (viral 2020 card game); *Death Over Dinner* movement.

**Pitch:** *New Yorker* essay primitive. Slow burn. Highest cultural prestige; build last after others establish credibility.

**Build:** 8–10 months. Death oracle is the hard part — partnership with UK GRO / US SSA Death Master File or family-attested 2-of-3 multisig.

**Singh names:** Singh Mortis (decay-rate function indexed to actuarial tables), GraveGraph (the social graph).

### Tier A consumer (build but later)

- **Singh Snap** (decay-photo) — ship as SDK, not consumer product (Snap Inc would crush a solo founder)
- **Singh Letters** (pen-pal chain) — slow Web cultural prestige play
- **Singh Wabi-Post** (vintage tweets) — defer until Twitter-scale base exists
- Singh Trip (travel capsule) — folded into ChildKey

### Strikes from this round (consumer)

Dating with decay-consent (Hinge/Bumble network effects insurmountable; build consent primitive as RA-DID extension instead), messaging (Signal won; decay is feature not moat), daily meditation (Calm/Headspace are content businesses), generic habit tracking (subsumed by WitnessFit), decay music streaming (Spotify won; no licensing path), decay job board (LinkedIn/Indeed insurmountable), decay reading (publisher DRM nightmare), friendship wallet (tbh/Path/Peach all failed), decay fan communities (Discord/Reddit won), MMO roleplay (game studios won't integrate).

---

## A5.6 Crate impact — additional ~25 crates

If everything in A5 ships, the crate count grows from ~62 (sprint target) to ~87 (post-A5 sprint target). Honest read: **A5 is too much for a 6-month solo sprint.** Realistic Q3 2026 + V2 split:

**Sprint May–Oct 2026 (additions to existing A1-A3 build order):**

| Weeks | Add | Why |
|---|---|---|
| 4–7 | `singh-strategy-vm` foundations (research-ready, not full impl) | SSM is the academic-press claim |
| 8–14 | `sgb-types` (linear logic !/?) | Replaces or augments LAD-VM if Contractless paradigm not chosen |
| 8–14 | `sbav-vm` reversible core | Pairs with SGB; together = headline academic story |
| 12–18 | `sddc` (Decay-Dutch auction) — foundational mechanism | Reused by every marketplace below |
| 14–18 | `sfsv` (Future-Self Vault) — first launch dApp candidate | Cheapest, most viral marketplace |
| 16–22 | `singh-sabi` NFT primitive | Cultural launch moment |
| 18–22 | `singh-triage` wallet UX | Daily-driver UX upgrade |
| 18–24 | `singh-letter` / ChildKey | Mainstream consumer app |
| 22–26 | `singh-heartbeat` + `singh-lineage` wallet | Ambient + inheritance UX |

**V2 (post-launch, Year 2):** SHLM, SAP, SCL, SCDI, Singh-Posthuma, Singh-Migrant, Singh-Heir, Singh-Resonance, Singh-Counsel, MnemoChain, WitnessFit, GraveGraph.

## A5.7 Naming guardrails — collision resolution + new rules

30. **Singh-Lineage** = wallet-UX paradigm (graduated dormancy-based inheritance). The NFT primitive originally proposed under this name is renamed **Singh-Heir** (kin-graph heirloom NFT). No further reuse of "Lineage."
31. **Singh Letter / ChildKey / Singh Vault** are three names for sub-components of one primitive (sealed-by-age-of-recipient letters). When referring to the whole thing, use ChildKey as the consumer-facing brand. When referring to the protocol primitive, use Singh Letter.
32. **MnemoChain / Singh Curve / Mnemo Trie** — same pattern. Consumer brand: MnemoChain. Decay-rate function: Singh Curve. Storage: Mnemo Trie.
33. **WitnessFit / Singh Streak / Decay-Bond** — same pattern. Consumer: WitnessFit. Primitive: Singh Streak. Staking variant: Decay-Bond.
34. **GraveGraph / Singh Mortis** — Consumer brand: GraveGraph. Decay-rate function: Singh Mortis.

## A5.8 Honest commercial recommendation — pick ONE launch consumer app

A5 surfaces four lock-grade consumer primitives. Solo + 6 months means picking ONE for the launch consumer wedge:

| Pick | If you optimize for | Reasoning |
|---|---|---|
| **ChildKey** | Mainstream press impact + emotional virality | Today Show segment writes itself; lowest crypto-vibe risk; 3–4 month build |
| **MnemoChain** | Real B2B revenue + portable-credential moat | Med students will pay; FSRS is open source; portable credentials is a defensible moat |
| **WitnessFit** | Largest TAM (100M+ wearable owners) | But weakest moat (Streaks-the-app already won the UX); needs portability angle to differentiate |
| **GraveGraph** | Long-game cultural prestige | Slow burn; defer until other primitives establish credibility |

My read: **ChildKey first** — fastest to ship, highest emotional pull, mainstream-press friendly, no enterprise sales cycle. Then MnemoChain in V2 once the chain has audience.

End of Amendment 5.

