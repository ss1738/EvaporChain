# EvaporChain Frontier Research

Novel primitives that don't exist in any blockchain or academic literature.
This directory holds the three foundational primitives below; the full
canonical doctrine — including the 5 Tier-0 inventions, 7 Tier-0 supporting
theorem-grade primitives, and the 2026-05 doctrine arc (Lambda-Fold Nova,
Crooks-MEV, Light-Cone Full DAG) — lives in `../INVENTION_STACK.md`.

## Foundational primitives (this directory)

| # | Primitive | Status | Tests | Doc |
|---|-----------|--------|-------|-----|
| 1 | [Proof-of-Historical-Availability (PoHA)](01-poha-decaying-da.md) | **Done** | 19 | Decaying DA certificates |
| 2 | [Energy-Annotated Verkle Trie](02-energy-verkle-trie.md) | **Done** | 26 | Self-pruning state tree |
| 3 | [Rule-Based Consensus](03-rule-based-consensus.md) | **Done** | 28 | Consensus over decay rules, not state snapshots |

## Tier-0 invention stack (see `../INVENTION_STACK.md` §A1.2 + §A1.5)

| Primitive | Crate | Status |
|---|---|---|
| Maximum-Caliber Consensus (MCC) — Jaynes Lagrangian fork choice | `evaporchain-mcc` | ✅ Substrate complete |
| Crooks-Singh Fee Equilibrium (CFM) — closed-form fee distribution | `evaporchain-cfm` | ✅ Substrate complete |
| Causal-State Light Client (CSLC) — Shalizi-Crutchfield ε-machines | `evaporchain-cslc` | ⚠ Single-state baseline; Shalizi-Klinkner CSSR is open work |
| Lambda-Locked Self-Amendment (LLSA) — Coq invariant-preservation kernel | `evaporchain-llsa` + `research/proofs/LLSAInvariantPreservation.v` | ✅ Build-verified under Rocq 9.1.1 (M2 closure 2026-05-05); descope path with `MultiAuditorVerifier` k-of-n |
| Evaporative Protocol Versioning (EPV) — verifier modules decay | `evaporchain-epv` | ✅ Substrate complete |

## Tier-0 supporting theorem-grade primitives (`../INVENTION_STACK.md` §A1.3)

| Primitive | Crate | Theorem |
|---|---|---|
| Sanov-Slashing | `evaporchain-sanov-slashing` + `evaporchain-entropic-slashing` | Cramér 1938; Sanov 1957 — large-deviation slash magnitude |
| TUR Liveness Detector | `evaporchain-tur-liveness` | Barato-Seifert 2015 — thermodynamic uncertainty relation |
| Cμ-Gate | `evaporchain-cmu-gate` | Shalizi-Crutchfield Cμ ≤ E + hμ identity |
| MDL-Shard | `evaporchain-mdl-shard` | Rissanen 1978 — minimum description length |
| Causal-Cone Validator State | `evaporchain-causal-cone` | Shalizi 2003 — light-cone sufficient statistics |
| **Crooks-MEV Refund** ✅ Consensus-integrated 2026-05-04 | `evaporchain-mev-detect` + `evaporchain-crooks-mev-refund` | Crooks 1999 fluctuation-theorem ratio; sandwich-attack restitution |
| **Causal-CHSH Cartel Detector** ✅ Empirical gate PASS 2026-05-04 | `evaporchain-causal-chsh` | Bell-Clauser-Horne-Shimony-Holt 1969; first 100% original frontier theorem (S>2 ⇒ cross-validator coordination) |

## 2026-05 doctrine arc

Three frontier primitives shipped end-to-end across May 2026:

- **Lambda-Fold Nova IVC** (`evaporchain-lambda-fold` + `evaporchain-proving::nova`) — sublinear light-client chain verification. Empirically locked at 23 ms @ 100 folds (1.083× of 23 ms @ 10 folds) on M4. See `../../LAMBDA_FOLD_NOVA_PLAN.md`.
- **Crooks-MEV refund pipeline** (`evaporchain-mev-detect` + tendermint integration) — sandwich detection → settlement → stake-slash, governance-flag-gated `observe → enforce`. See `../../CROOKS_MEV_INTEGRATION_PLAN.md`.
- **Light-Cone Full DAG mode** (`evaporchain-light-cone`) — multi-parent blocks, MCC tip selection, antichain finality, Phase 4.4 cross-validator commit-cert digest. See `../../LIGHT_CONE_FULL_DAG_PLAN.md`.

## Build order (this directory)

1. **Energy-Annotated Verkle Trie** — lowest risk, self-contained, new data structure
2. **PoHA** — extends existing `evaporation_da.rs`, strongest publication potential
3. **Rule-Based Consensus** — fixes state root divergence, hardest, most fundamental

## Paper strategy

One unified paper: "Thermodynamic Blockchain Primitives: State Decay as a First-Class Distributed Systems Abstraction"

Contributions: (a) energy-annotated authenticated data structures, (b) decaying data availability certificates, (c) rule-based consensus for time-dependent state.

Adjacent paper-grade results from the broader stack (each independently publishable): Crooks-MEV refund, Lambda-Fold sublinear light-client verification, Causal-CHSH cartel detection on real Ethereum data, MERA real-data gate (VERKLE verdict, §A1.8). See `../INVENTION_STACK.md` for the canonical list.
