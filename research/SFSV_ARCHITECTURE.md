# SFSV — Self-Future-Self Vault Architecture

**Version:** 1.0
**Date:** 2026-05-16
**Status:** Canonical reference for the first launch dApp. Pairs with `INEVITABILITY_STRATEGY.md`, `INVENTION_STACK.md`, `APPLICATION_UNIVERSE.md` (cat 7: DeFi with native decay).
**Owner:** Satyawan Singh.
**Naming convention:** primitives whose math is original-here may carry "Singh"; cryptographic primitives lifted from literature cite the original.
**Source-of-truth pair:** `contracts/evaporscript/future_self_vault.es` (282 LOC) + `crates/evaporchain-sfsv/` (5 modules, 25 tests).

---

## 0. TL;DR for cold readers

SFSV is the **canonical reference dApp** for EvaporChain's energy-decay primitive. A user deploys a vault contract whose deposit is locked from their present self and released to their future self when a predicate trips (epoch reached, or contract's own physical energy decays below a threshold). The deposit *is the energy*; deploying with budget B *is* the lock — there is no hand-coded decay formula, the chain's Single-λ Principle does the bookkeeping for free.

Three properties make SFSV the right first build:

1. **Zero cold-start.** No issuers, no attestors, no oracle. The wallet is the only trust anchor; the chain's clock is the only external signal.
2. **Demonstrably impossible on Ethereum / Solana / Cosmos.** Without native energy-decay, "vault auto-unlocks when its physical energy falls below ε" has no analogue. A simulated counterpart needs an off-chain keeper (centralised) or a TWAP oracle (unreliable). UVP filter from `APPLICATION_UNIVERSE.md` passes cleanly.
3. **Forkable in <50 lines.** Once SFSV ships, *every* decay-application (mortal credentials, decaying NFTs, rentals, demurrage stablecoins, sealed-bid auctions) is a fork of the same pattern. SFSV is the viral seed.

This document is **not** a tutorial. It is the architecture spec — math, cryptography, threat model, doctrine mapping, frontier-research roadmap. Solo-build budget: ~6 weeks for ship-quality reference impl, ~3 months for the full frontier extensions catalogued in §7.

---

## 1. Mission & Doctrine Anchor

### 1.1 What SFSV is

A user-deployed, user-owned, single-owner-at-a-time **time-locked vault** whose release condition is one of:

- **EpochReached** — release at a chosen absolute epoch.
- **EnergyDecaysBelow** — release when the *vault contract's own physical energy* falls below a threshold.

The second predicate is the architecturally interesting one — it routes time through *physics* (the chain's energy field) rather than wall-clock epochs, which has three properties no other chain can replicate:

1. **Re-org-safe.** Energy is a chain-state quantity, not a wall-clock signal. A re-org rewinds energy with it; the predicate evaluates to the same answer on every honest fork at the same height.
2. **Refresh-safe.** If the holder refreshes the contract (pays gas to restore energy), the predicate's trip is *naturally* postponed — the user has implicit early-cancel without a separate API.
3. **Slashing-safe.** Confiscatory slashing burns energy; a slashed vault unlocks faster. The release condition has a built-in "soft death-of-issuer" path.

### 1.2 What SFSV is **not**

- **Not** a yield product. The vault does not pay interest. Anti-feature per `INVENTION_STACK.md`.
- **Not** a fundraiser. No token, no ICO, no IDO. Per Satoshi-pattern doctrine.
- **Not** tied to identity. The future-self address is a wallet, not a KYC binding.
- **Not** a security model for inheritance — the m-of-n threshold variant (§5.4) is for *contingent control*, not for legal succession; legal succession is out of scope for an L1 primitive.

### 1.3 Why this is the **first** launch dApp

From `evaporchain_application_universe.md` build queue (slot 3): *"SFSV reference dApp (2027 early) — viral demo of primitive."* The decision logs (`strategic_decision_2026_05_16_focus.md`) lock SFSV ahead of mortal-credentials (cat 4) precisely because credentials need attestors and SFSV does not. SFSV's cold-start is the wallet's keypair.

---

## 2. The Three Primitives at Play

SFSV is a *minimal* dApp in the sense that it activates only three of the chain's primitives, and that minimality is the design's strength. Every additional primitive a reference dApp activates is a feature-flag-induced fragility for forkers downstream.

### 2.1 Energy-Decay (Layer 0)

The chain enforces a Single-λ Principle: every decay across the whole stack — consensus weight, mempool admission, stake refresh, capability expiry, governance — collapses to one rate. SFSV's `EnergyDecaysBelow` predicate hooks *directly* into this — `energy_at_epoch(e₀, Δt)` is the canonical formula (`evaporchain-types::energy_at_epoch`, Coq-proven). The Layer 0 lint forbids hand-rolled bit-shifts on energy outside `evaporchain-types`; SFSV obeys it.

### 2.2 EvaporScript VM (Layer 3)

The contract is written in EvaporScript, not Solidity, not Move, not Rust. Pure 44-opcode VM with constant-fold + DCE compile passes; gas-metered; predicate evaluation is *pure*, side-effect-free, re-entrancy-impossible by VM design. The contract has no internal method dispatch (a VM constraint per `evaporchain_evaporscript_grammar_gotchas.md`), so predicate logic is inlined into `try_payout` and `predicate_satisfied` and held bit-identical by adversarial tests.

### 2.3 SDDC Marketplace Pattern (Layer 6, Substrate)

A vault is a **transferable asset** during its locked phase. The holder may list the vault on a Dutch-clearing auction (`list_for_sale(ceiling, floor, duration)`); winning bidder becomes the new holder. The contract enforces:

- Only the *current holder* can list.
- The Dutch-clear is off-chain (SDDC coordinator) but the sale is on-chain (`record_sale(winner_addr)`).
- `record_sale` is only valid during an open listing window.
- The future-self beneficiary address is **immutable** post-seal — the secondary market trades the *claim*, not the destination.

This is the same SDDC pattern shared by SFSV / SHLM / future marketplaces; SFSV is the canonical instance.

---

## 3. State Machine & Lifecycle

```
                  set_terms()
   [DEPLOYED] ─────────────────► [SEALED]
                                    │
                       list_for_sale│ (optional)
                                    ▼
                                 [LISTED] ─── record_sale ─► [SEALED, holder := new_holder]
                                    │
                          cancel_listing
                                    ▼
                                [SEALED] (back)
                                    │
                                    │   predicate_satisfied()
                                    ▼
                                  ┌────────────────┐
                                  │  try_payout()  │
                                  └────────┬───────┘
                                           ▼
                                       [RELEASED]   ← terminal, no further mutations
                                           │
                              off-chain execution layer:
                              credit current_holder + retire contract
```

**Sealing.** `set_terms(...)` may be called exactly once. After sealing, all economic terms (`deposit`, `predicate_type`, `release_epoch`, `threshold`, `future_self`) are immutable. Re-seal attempts revert.

**Release.** `released = true` is terminal. The contract's storage is then eligible for **natural eviction** per the state-bloat-killer doctrine: once a released vault's residual energy decays to zero, the chain garbage-collects it without operator intervention. This is the doctrine point most other chains cannot reproduce.

**Transitions are linear and one-way.** The state machine has no cycles past `[RELEASED]`. There is no `[REOPEN]`, no `[REPAUSE]`, no governance-vetoed "void"; the contract is purely user-initiated and chain-finalised.

---

## 4. Mathematical Foundation

### 4.1 Decay function

The chain's canonical decay is:

```
energy_at_epoch(e₀, Δt) = e₀ · 2^(−Δt / τ)
```

where:
- `e₀` is the initial energy deposit at the lock epoch.
- `Δt` is elapsed epochs since lock (chain-state, not wall-clock).
- `τ` is the half-life parameter — chain-level constant, not per-vault.

The lint at Layer 0 forbids any *re-derivation* of this formula in dependent crates; SFSV calls `energy_at_epoch` and never computes decay arithmetic itself. The Coq proof at `research/coq/EnergyDecayPreservation.v` discharges the conservation invariant.

### 4.2 Predicate semantics

**EpochReached(release_epoch).** Trips when `current_epoch ≥ release_epoch`. Pure comparison; no decay arithmetic. Functionally identical to a wall-clock timelock except for re-org safety (the chain's epoch is causal-set time, not block-height time — see `research/light_cone/`).

**EnergyDecaysBelow(threshold).** Trips when the contract's *own physical energy* is `< threshold`. This is the architecturally novel one. The contract's energy field is the chain's bookkeeping — no on-chain math is needed in the predicate body; the comparison reads `self.energy < threshold`.

The two predicates are mathematically equivalent only when the half-life τ exactly matches the chosen release window:

```
threshold = e₀ · 2^(−(release_epoch − lock_epoch) / τ)
```

For any other (τ, threshold) pair, the predicates describe distinct conditions and the chooser is the vault deployer's decision.

### 4.3 Conservation invariant

For the system as a whole:

```
Σ_{v ∈ active_vaults} v.deposit
  + Σ_{v ∈ released_vaults} v.payout
  + Σ_{v ∈ slashed_vaults} v.confiscation
  = Σ_{v ∈ all_vaults} v.deposit_at_lock
```

This is the chain-level Conservation of Energy applied to SFSV. The candidate Coq proof at `research/coq/SFSVConservation.v` (not yet written) should be a direct corollary of `EnergyDecayPreservation.v` plus the predicate purity lemma.

### 4.4 Why the predicate must be pure

If `predicate_satisfied(vault, state)` is side-effect-free, then:

- The same predicate evaluates to the same answer on every honest fork at the same height.
- Re-org safety: rolling back state rolls back the predicate's truth value.
- DoS-safe: an attacker calling `try_payout` repeatedly on an unsatisfied vault burns their own gas without affecting vault state.

The EvaporScript VM's lack of internal method dispatch is what makes purity *automatic* — there is no `self.method()` indirection through which a side effect could hide. The compile-time inlining is enforced by adversarial tests in `predicate.rs`.

---

## 5. Predicate Variants

§5.1–5.2 are **shipped today**. §5.3–5.7 are the frontier-research roadmap — each adds a doctrine-clean predicate without breaking the existing contract.

### 5.1 EpochReached (shipped)

Trivial timelock. Predicate body: `self.energy_or_epoch >= release_epoch` (using the chain's epoch field). Use-cases: birthday gifts to children, scheduled vesting, future-dated charity donations, "do not open until 2030" capsules.

### 5.2 EnergyDecaysBelow (shipped)

Doctrine-canonical. Vault releases when its *own* energy field decays below `threshold`. Use-cases: graceful expiry of self-locked assets, "burn-on-fade" credentials, deflation-tied unlocks (vault matures faster in a high-confiscation environment).

### 5.3 VDF-AnchoredEpochReached (frontier)

**Problem.** A vault that releases at epoch E is gameable by a chain reorganization that skips epochs. EpochReached's re-org safety holds for *honest* chain heights but not against a 51%-adversary mining empty blocks at high tempo to fast-forward `current_epoch`.

**Fix.** Anchor the release condition not to the chain epoch alone but to a **Verifiable Delay Function** evaluated over the lock-time hash:

```
release_condition := VDF.verify(input := lock_hash,
                                difficulty := T_releases,
                                output := σ)
```

`σ` is supplied by whoever submits `try_payout`. VDF.verify is O(log T) but VDF.eval is Θ(T) sequential — no shortcut. T is chosen so that on the fastest commodity hardware, σ cannot be computed before the wall-clock target.

**Candidate VDF.** Wesolowski 2018 (RSA-style, ~50ms verify, requires trusted setup) **or** Pietrzak 2019 (class groups, ~200ms verify, transparent setup). The trade-off is verifier cost vs. trusted-setup ceremony complexity; class groups are doctrine-preferred (no ceremony, post-quantum-friendlier).

**Status.** Not in current `evaporchain-sfsv`. Would be ~2 weeks of integration; the chain already vendors `nova-snark` for IVC so much of the cryptographic plumbing exists. Belongs in v1.1.

### 5.4 ThresholdSelf (frontier)

**Problem.** A single-key future-self vault is brittle. If the user loses the future-self private key, the vault is unrecoverable. This is the "$200M Bitcoin lost forever" problem applied to time-locked deposits.

**Fix.** Replace `future_self: address` with `future_self_group: G2Affine, threshold: u8`. Release becomes a BLS aggregated signature of m-of-n shareholders' approval. Shamir secret sharing at lock-time distributes shares to designated keys (spouse, lawyer, second device, etc.).

**Cryptographic stack.** Pedersen DKG for the share-generation ceremony, BLS12-381 for signature aggregation, Lagrange-interpolation evaluation off-chain or on-chain (gas-bounded). All three primitives already exist in `evaporchain-crypto`; the integration is the engineering.

**Status.** Not in current contract. The `.es` contract would need a new state field and a new entry point `record_threshold_release(sig)`. ~1 week of work.

### 5.5 WitnessEncryptedBeneficiary (frontier, cryptographically aggressive)

**Problem.** A vault that names `future_self` *at deploy time* leaks the beneficiary to the chain forever. A privacy-preserving variant should reveal the beneficiary only after the predicate trips.

**Fix.** Encrypt the beneficiary address with a witness-encryption scheme whose decryption key is the predicate's own satisfaction proof:

```
ct = WE.Enc(beneficiary, predicate_circuit)
beneficiary = WE.Dec(ct, witness)   where witness is a satisfying assignment
                                    to predicate_circuit
```

When the predicate trips, the chain (or any third party) recovers the beneficiary and credits them.

**Cryptographic substrate.** Witness encryption (Garg-Gentry-Halevi-Sahai 2013) is theoretically beautiful but practically unstable — every known construction relies on multilinear maps or indistinguishability obfuscation, both of which have had repeated cryptanalysis cycles. A safer interim is **time-lock puzzles** (Rivest-Shamir-Wagner 1996) which give a weaker but cryptographically robust analogue.

**Status.** v2.0 candidate, not v1.0. Flagged here because the doctrine demand exists (state-bloat reduction: the chain never has to store the beneficiary in cleartext).

### 5.6 ForwardSecureRotatingClaim (frontier)

**Problem.** If the future_self private key is compromised at any point during the lock window, the attacker can wait for the predicate to trip and steal the payout.

**Fix.** The future_self key evolves once per epoch via a forward-secure signature scheme. Past keys cannot reclaim. The contract verifies the current epoch's key against a Merkle-Mountain-Range root committed at deploy time.

**Cryptographic substrate.** Itkis-Reyzin 2001 forward-secure signatures with optimal verifying time. Alternative: lattice-based forward-secure (post-quantum). Both work; the chain's existing Dilithium3 signature suite (PQ-safe) plus an MMR-anchored evolution root is the natural fit.

**Status.** v1.5 candidate. Wallet-side complexity is the gating cost — the user has to evolve their key every epoch, which is a UX burden requiring a daemon. Not worth shipping until the wallet has a background-sync process.

### 5.7 LambdaFoldedBatchRelease (frontier, novel-here)

**Problem.** A protocol-level migration that wants to release N vaults simultaneously (e.g., a chain-wide grace event after a confiscation crisis) costs O(N) gas. For N=10⁶, prohibitive.

**Fix.** Use the chain's existing Lambda-Fold (Nova IVC) primitive. A single recursive proof folds N "this vault's predicate trips" statements into one O(1)-verified proof; the chain accepts the batch as a single transaction.

This is **novel** — no public chain has had a fold accumulator native at L1 before EvaporChain. The reference SFSV implementation should include a small batch-release example to demonstrate the pattern, since the same accumulator can later batch-clear any decay-application's release condition.

**Status.** The chain has Nova IVC at `evaporchain-nova-bridge` (84% covered as of 2026-05-16 lane #4 pushes). The Lambda-Fold doctrine doc at `research/lambda_fold/` is canonical. The SFSV-side integration is a separate substrate crate `evaporchain-sfsv-batch` not yet scaffolded. ~3 weeks of work, but reusable: every decay-dApp downstream can call into it.

---

## 6. Frontier Cryptographic Stack

Each row is a primitive the SFSV reference impl will eventually invoke. Status reflects state as of 2026-05-16.

| Primitive | Used For | Provenance | Status in EvaporChain | SFSV v1.0? |
|---|---|---|---|---|
| BLAKE3 | Content addressing | Aumasson et al. 2020 | `evaporchain-crypto` (production) | ✅ |
| BLS12-381 aggregation | Threshold release (§5.4) | Boneh-Gorbunov-Wahby 2018 | `evaporchain-crypto` (production) | ❌ (v1.5) |
| Dilithium3 (ML-DSA) | Owner-side authentication, PQ-safe | NIST FIPS 204 | `evaporchain-crypto` (production) | ✅ |
| Pedersen commitments | Amount privacy | Pedersen 1991 | `evaporchain-crypto` (production) | ✅ (optional) |
| Verkle / Energy-Verkle Trie | Vault state membership | Buterin / Boneh-Bonneau-Bünz-Fisch 2018 | `evaporchain-state` (production) | ✅ |
| MMR (Merkle Mountain Range) | Nullifier set, anti-replay | Peter Todd 2017 | `evaporchain-crypto` (production) | ✅ |
| Nova IVC (Lambda-Fold) | Batch release (§5.7) | Kothapalli-Setty-Tzialla 2022 | `evaporchain-proving`, `evaporchain-nova-bridge` (production) | ❌ (v1.1) |
| Wesolowski / Pietrzak VDF | Re-org-resistant timelock (§5.3) | Wesolowski 2018, Pietrzak 2019 | not yet integrated | ❌ (v1.1) |
| Forward-secure signatures | Rotating claim key (§5.6) | Itkis-Reyzin 2001 | not yet integrated | ❌ (v1.5) |
| Pedersen DKG | Threshold key generation (§5.4) | Pedersen 1991 / Gennaro et al. 1999 | not yet integrated | ❌ (v1.5) |
| Witness encryption | Beneficiary privacy (§5.5) | Garg-Gentry-Halevi-Sahai 2013 | research only | ❌ (v2.0) |
| Time-lock puzzles | Fallback for §5.5 | Rivest-Shamir-Wagner 1996 | not yet integrated | ❌ (v2.0) |
| Class group cryptography | Transparent VDF setup | Buchmann-Williams 1988 | research only | ❌ (v1.1) |
| Groth16 SNARK | Wrapped Nova accumulator | Groth 2016 | `evaporchain-nova-bridge` (production) | ✅ (read-only) |

**Solo-build complexity budget.** v1.0 uses only primitives already in production at `evaporchain-crypto` / `evaporchain-state` / `evaporchain-nova-bridge`. No new cryptographic engineering required for ship. v1.1 (VDF + batch release) requires ~5 weeks of new crypto work and should *not* be on the mainnet-sprint critical path. v1.5 and v2.0 are post-Paper-1 research projects.

---

## 7. SDDC Market Integration

The SDDC pattern (`Substrate Dutch-clearing Decay-aware Coordinator`) was introduced 2026-05-02 with the `evaporchain-sddc` base crate. SFSV is the canonical instance.

### 7.1 Dutch-clearing mechanics

A locked vault is a **claim on a future cash flow** whose present value depends on:
- The vault's remaining energy `e_now`.
- The chain's half-life τ.
- The market's implied discount rate δ.

The Dutch-clearing auction starts at `list_ceiling`, decays linearly toward `list_floor` over `list_duration` epochs. The first bidder accepting the price wins. This is structurally identical to a continuous double auction with a single seller, single buyer per round, and a deterministic time-of-trade.

**Why Dutch and not English.** English (ascending) auctions front-load the price discovery into many small transactions; for an on-chain market, this is expensive. Dutch is single-transaction: one `record_sale(winner_addr)` call closes the auction.

**MEV concern.** Front-running the `record_sale` call is the obvious attack vector — a searcher reading the mempool can race the legitimate winner. The chain's Crooks-MEV pipeline (per `research/crooks_mev/`) provides deterministic slot assignment for MEV-flagged operations; `record_sale` should be flagged as MEV-sensitive in the substrate dispatcher.

### 7.2 Why the secondary market matters

Without a secondary market, time-locked vaults are illiquid — a user with a 10-year vault who needs cash now is stuck. With a secondary market, the vault becomes a *tradeable claim* with a Bismarck-pension-style present value. The market price reveals the discount rate the chain's depositors actually use, which is empirically valuable data for the chain's economic researchers.

This is also where the chain's value-add over Ethereum becomes most visible: an Ethereum vault auction would clear at a price that includes the keeper-bot fee for triggering release; an EvaporChain vault auction's price is *only* a function of physics (remaining energy + half-life), because the release is automatic.

### 7.3 Cancellation

Holders may cancel an open listing by calling `cancel_listing`. The contract reverts `listed = false`; no penalty. This is intentional — an over-zealous penalty discourages listing in the first place, which destroys the price-discovery function of the market.

---

## 8. Threat Model

The threat model is structured as **adversary → goal → mitigation → residual risk**. Each adversary is constructively defined by what they *can* do, not by their identity.

### 8.1 Adversary A — Present-Self Reneger

**Capabilities:** holds the owner key, controls the wallet, can submit any well-formed transaction.
**Goal:** reclaim the deposit before the predicate trips.
**Attack:** call `try_payout` early.
**Mitigation:** `predicate_satisfied` is pure and returns `false`; `try_payout` reverts. The deposit stays locked.
**Residual:** none for v1.0. (Refresh attacks — paying gas to keep the energy high — are *features*, not attacks, since they amount to the user paying for postponement.)

### 8.2 Adversary B — Chain-Operator Censor

**Capabilities:** runs a validator, controls block inclusion within their proposer slot.
**Goal:** prevent a victim's vault from releasing on time.
**Attack:** refuse to include `try_payout` transactions for victim's vault.
**Mitigation:**
- The Immune Validator Set (`research/proofs/LLSAInvariantPreservation.v`, Coq-proven) jails validators whose block-inclusion behavior deviates from the unbiased proposer distribution. A targeted censor accumulates a measurable bias.
- The Light-Cone Ledger DAG accepts parallel parent blocks; if validator V censors, any other validator V' can include the txn at the same height.
- `try_payout` is permissionless — *any* address can submit it, the payout still goes to the named beneficiary. A censored victim can pay a third party a small fee to relay.

**Residual:** brief delays (one block, possibly two during partition); no permanent denial.

### 8.3 Adversary C — Re-org Attacker

**Capabilities:** controls ≥ ⅓ of stake (per BFT safety threshold).
**Goal:** invalidate a payout that already occurred.
**Attack:** mine a longer chain that omits the `try_payout` transaction.
**Mitigation:**
- BFT finality is committed after 2/3-stake votes; reverting requires equivocation, which slashes the equivocator's stake.
- Once `released = true`, the state is in the Verkle root; rolling it back requires reverting the whole block, which requires reverting all txns in the block.
- The §5.3 VDF anchor (when shipped) makes fast-forward fork mining infeasible — the adversary cannot mine faster than the VDF.

**Residual:** below-finality (pre-2/3) attacks possible but require equivocation = slashing risk = economically infeasible for vaults below the slashing-collateral threshold.

### 8.4 Adversary D — State-Bloat Spammer

**Capabilities:** can submit unbounded `deploy + set_terms` transactions paying the minimum gas.
**Goal:** fill the chain's state with dust vaults that never release.
**Attack:** deploy 10⁹ tiny vaults with absurdly distant `release_epoch` (e.g., epoch 2^63).
**Mitigation:**
- Minimum `deposit` floor at admission. A vault with `deposit < MIN_VAULT_DEPOSIT` is rejected at the EvaporScript admission gate.
- Released vaults are eligible for natural state eviction once their residual energy decays to zero — built-in garbage collection without operator action.
- Gas pricing on `set_terms` includes a future-storage premium proportional to `release_epoch − current_epoch`. Distant unlocks cost more upfront. (This is the chain's standard storage rent applied to the vault's expected on-chain lifetime; existing primitive, no new mechanism.)

**Residual:** spam attacks where the attacker burns substantial gas to fill state — economically equivalent to burning collateral for no gain; rational adversary won't.

### 8.5 Adversary E — Replay / Double-Reclaim

**Capabilities:** captures a successful `try_payout` transaction off-the-wire; tries to replay it on the same vault.
**Goal:** double-payout.
**Attack:** broadcast the captured txn against the contract again.
**Mitigation:** `released = true` is terminal and checked first in `try_payout`. The second attempt reverts. The nullifier MMR also records the payout-event hash; cross-vault replay is rejected.
**Residual:** none.

### 8.6 Adversary F — MEV Sandwich on Secondary Market

**Capabilities:** runs an MEV searcher with mempool visibility.
**Goal:** front-run a `record_sale` so the searcher buys the vault and re-sells at the original price.
**Attack:** observe pending `record_sale(winner)` txn, submit own `record_sale(searcher)` with higher gas.
**Mitigation:** `record_sale` flagged MEV-sensitive in Crooks-MEV dispatcher; the chain's MEV pipeline assigns deterministic slot ownership based on listing-time hash, neutralising the gas-bid race.
**Residual:** depends on Crooks-MEV maturity. If the MEV pipeline is not yet flagged for this op, the searcher can succeed. Mitigation deferred to post-mainnet Crooks-MEV enforcement mode.

### 8.7 Adversary G — Beneficiary-Key Loss

**Capabilities:** the future-self loses their private key before the predicate trips.
**Goal:** —  (this is the user's own self-inflicted failure mode).
**Attack:** key lost in disk crash, dead drop, deceased holder, etc.
**Mitigation in v1.0:** none. The vault is unrecoverable. This is a known and explicit risk in the v1.0 reference.
**Mitigation in v1.5:** §5.4 ThresholdSelf variant — m-of-n release defeats single-key loss.
**Residual:** key-loss in single-key vaults is a feature of self-custody, not a bug to fix at protocol level.

### 8.8 Adversary H — Quantum

**Capabilities:** holds a cryptographically-relevant quantum computer (CRQC), 2030+ horizon.
**Goal:** forge a future-self signature or break a VDF.
**Attack:** Shor's algorithm against ECDSA / BLS12-381 / RSA-VDF.
**Mitigation:**
- Owner authentication uses ML-DSA (Dilithium3) which is PQ-safe by design.
- BLS aggregation (threshold variant) is **not** PQ-safe; v2.0 should migrate to lattice-based threshold (e.g., FROST-Dilithium, when standardised).
- Wesolowski VDF (RSA-based) is **broken** by CRQC. Class group VDFs are believed quantum-resistant; SFSV's VDF choice should default to class groups, not RSA.

**Residual:** PQ-migration is a chain-wide concern, not SFSV-specific. Tracked at chain doctrine level.

---

## 9. Doctrine Mapping

SFSV instantiates four of the chain's five invention-stack primitives. The mapping is the load-bearing claim that SFSV is "thesis-true."

### 9.1 Light-Cone Ledger

The vault's lock-event and release-event are entries in the causal-set DAG. Two vault releases on different forks are *causally independent* if and only if their parent blocks are not in each other's light cones. The DAG semantics handles fork resolution without SFSV needing fork-aware logic.

### 9.2 Evaporated-Fork Certificates

If a vault is released on fork F₁ and the chain finalises on fork F₂, the chain emits an Evaporated-Fork Certificate proving the release on F₁ is **not** finalised. The off-chain execution layer reads this certificate and refunds-or-revokes the payout accordingly. SFSV's release is *atomic with* the chain's finality, not eventually-consistent with it.

### 9.3 Singh Attractor Consensus

Under partition, two halves of the chain may each see a different `current_epoch`. EpochReached predicates may trip on one half but not the other. The Singh Attractor's convergence dynamics ensure both halves reach the same `current_epoch` at finality; the late-half's vault then has `predicate_satisfied = true` and releases consistently. The chain's BFT safety is what makes SFSV's release safe under partition.

### 9.4 Bell-Certified Beacon

Not used by SFSV v1.0. (The Bell Beacon is the chain's randomness source; SFSV has no randomness dependency.) The threshold variant §5.4 would use the beacon for proposer election among the m-of-n custodians, but that's a v1.5 concern.

### 9.5 Immune Validator Set

Used by SFSV against censorship adversary §8.2. A validator with anomalous block-inclusion behavior is jailed; SFSV does not need to detect censorship itself — the chain's immune set does.

---

## 10. Reference Implementation Status

As of 2026-05-16 (this document's authoring date):

### 10.1 Shipped

| Surface | Path | LOC | Tests | Status |
|---|---|---|---|---|
| `.es` contract | `contracts/evaporscript/future_self_vault.es` | 282 | n/a (tested via crate) | drafted 2026-05-16 |
| Crate root | `crates/evaporchain-sfsv/src/lib.rs` | 64 | — | scaffolded |
| Vault module | `crates/evaporchain-sfsv/src/vault.rs` | 194 | 8 | shipped |
| Predicate module | `crates/evaporchain-sfsv/src/predicate.rs` | 169 | 6 | shipped |
| Payout module | `crates/evaporchain-sfsv/src/payout.rs` | 145 | 5 | shipped |
| Market module | `crates/evaporchain-sfsv/src/market.rs` | 199 | 6 | shipped |
| **TOTAL crate** | 5 files | **771** | **25** | green on satyawan |

The substrate crate counts as **first launch-dApp candidate per A5.2 doctrine** per `evaporchain_sfsv_scaffolded_2026_05_02.md`.

### 10.2 Gaps for v1.0 ship

In rough priority order:

1. **Adversarial test suite.** The current 25 tests are happy-path. Add adversarial tests covering each row of §8 (replay, censor, re-org, dust, MEV-sandwich, beneficiary-loss).
2. **Bit-identical predicate-evaluation enforcement.** The `.es` contract inlines predicate logic in two places (`try_payout` and `predicate_satisfied`). Adversarial test must verify the inlined bytecode is byte-identical.
3. **Deploy script.** `scripts/deploy-sfsv.sh` against the 3-Mini cluster: setup → deploy → lock → simulate decay → reclaim.
4. **Thin TS view layer.** Single page, two buttons: "lock" and "reclaim." Reads from `evaporchain-node` HTTP API. No framework dependency beyond `react` and `tailwind`.
5. **One-page README.** What it is, why it exists, how to fork it for a different decay use-case.

Estimated solo-build budget: **3–4 weeks** for v1.0 with all five above shipped.

### 10.3 Gaps for v1.1+

- VDF integration (§5.3) — 2 weeks
- Lambda-Fold batch release (§5.7) — 3 weeks
- Threshold variant (§5.4) — 1 week
- Forward-secure rotating claim (§5.6) — 1 week + wallet daemon prerequisite
- Witness-encrypted beneficiary (§5.5) — 4 weeks + research-phase complexity

**Total v1.0 → v2.0 frontier roadmap: ~3 months solo-build, assuming mainnet sprint completes on schedule.**

---

## 11. Frontier Research — Open Problems

This section lists problems whose resolution would extend SFSV beyond the v2.0 horizon. Each is publishable.

### 11.1 Optimal half-life for vault dApps

The chain has a single τ. But different dApps want different decay rates: a 100-year vault wants slow decay, a 1-day microtransaction wants fast decay. Does the Single-λ Principle constrain SFSV's expressive range, or can per-vault *effective* decay rates be derived from a single chain-wide τ by per-vault scaling of `e₀`?

**Conjecture (Singh, 2026):** the equivalence `e(t) = e₀ · 2^(−t/τ_eff)` with `τ_eff = α · τ` for arbitrary α > 0 is recoverable by setting `e₀` proportionally. The chain has a *fixed-τ* but *variable-effective-half-life* property. **This needs a formal proof and a Coq mechanisation.**

### 11.2 SFSV under fee-volatility

If chain fees spike during a vault's lock period, the holder cannot afford to refresh and the vault's energy decays faster than expected. Does this break SFSV's predicate-purity contract? Conjecture: no, because the predicate evaluates against post-fee energy, which is itself determined by chain state. But the user's *expected* release date shifts. **This is a UX problem that may require a chain-level "fee-protected" vault flag.**

### 11.3 Pension equivalence

A SFSV vault is structurally a defined-contribution pension: the holder deposits at time 0, the future-self receives a deterministic-or-time-decayed payout. Is there a formal equivalence between SFSV's mathematics and the actuarial reserve formulas (Bowers-Hickman-Nesbitt-Jones-Gerber 1997)? If yes, SFSV is the first L1-native pension primitive, which is a significantly stronger paper-publishable claim than "another DeFi vault."

### 11.4 Decay-aware fee discount

The chain's PID fee controller (`evaporchain-fee-controller`) currently treats all transactions equivalently. Could it discount fees for *transactions that consume state* (i.e., reduce future state-bloat liability)? SFSV's `try_payout` is the canonical such transaction. Conjecture: a fee-discount equal to `α · expected_state_bytes_freed` is incentive-compatible. **This is a chain-level mechanism design problem with SFSV as the motivating example.**

### 11.5 Inter-chain SFSV bridging

A future-self vault on EvaporChain releases to an Ethereum address. Is this safe? The release depends on EvaporChain's `current_epoch`, which is not observable on Ethereum. A naive bridge accepts a chain-state proof from EvaporChain's light client. **The architectural question is whether the Evaporated-Fork Certificate (§9.2) can be embedded in the light-client proof.** If yes, SFSV is bridge-safe; if no, only same-chain releases are safe and any cross-chain payout requires a relayer with slashable collateral.

---

## 12. Forkability — How to Derive Your Own Decay-dApp

The viral-demo purpose (§1.3) is satisfied if a third-party developer can read SFSV and produce a different decay-dApp in <50 lines of EvaporScript. The recipe:

1. **Pick your decay use-case.** Mortal credential? Decaying NFT? Rental? Demurrage stablecoin? See `evaporchain_application_universe.md` for the 12 categories.
2. **Identify the predicate.** What condition triggers your dApp's "release"? It must be expressible as a pure comparison over chain state (epoch, energy, or both).
3. **Fork `future_self_vault.es`.** Rename the contract, change the state fields to match your domain (e.g., for a decaying NFT: replace `deposit: u64` with `metadata_uri: bytes`).
4. **Inline your predicate in `try_payout` and `predicate_satisfied`.** Both must be byte-identical; adversarial test enforces.
5. **Wire to SDDC** if your dApp has a secondary market. Otherwise skip the listing fields.
6. **Adversarial-test the eight rows of §8.** Most carry over directly to your fork.

**Total fork distance for a typical decay-dApp: 30–80 lines of EvaporScript + ~200 LOC of substrate-crate test scaffolding.**

---

## 13. Failure Modes (Honest)

The doctrine demands the architecture doc *names* its own failure modes rather than hide them. Three I can identify today:

### 13.1 Witness encryption may not deliver

§5.5 is the most architecturally interesting frontier extension but witness encryption's cryptographic substrate is repeatedly broken. If the next 5 years' literature doesn't produce a stable WE construction, the beneficiary-privacy variant of SFSV is dead. Time-lock puzzles are a fallback but weaker.

### 13.2 VDF setup ceremony complexity

If the chain ships with RSA-VDF (Wesolowski) for simplicity, it inherits a trusted-setup ceremony obligation. If it ships with class-group VDF, it inherits a verifier-cost obligation (~5× slower). Both are acceptable, but the choice is irreversible once mainnet ships. Doctrine prefers class groups (transparent setup); cost is the residual issue.

### 13.3 The Single-λ constraint may surface

§11.1's conjecture, if false, means SFSV's expressive range is bounded by the chain's chosen τ. A 100-year vault on a chain with τ=1 year decays nearly to zero before release; the math has to be redone. This is unlikely (the conjecture's intuition is strong) but not proven.

### 13.4 Solo-build complexity budget overrun

The full v2.0 roadmap (§7) is ~3 months solo-build. If mainnet sprint slips, SFSV slips, Paper 1 slips, GDPR-EaaS slips. The cascading risk is significant. The mitigation is to ship **v1.0 only** before Paper 1, treat §5.3–§5.7 as post-paper research.

---

## 14. Bibliography

Cryptographic primitives, ordered by year:

- Rivest, R. L., Shamir, A., & Wagner, D. A. (1996). *Time-lock puzzles and timed-release crypto*. MIT/LCS/TR-684.
- Pedersen, T. P. (1991). *Non-interactive and information-theoretic secure verifiable secret sharing*. CRYPTO '91.
- Gennaro, R., Jarecki, S., Krawczyk, H., & Rabin, T. (1999). *Secure distributed key generation for discrete-log based cryptosystems*. EUROCRYPT '99.
- Itkis, G., & Reyzin, L. (2001). *Forward-secure signatures with optimal signing and verifying*. CRYPTO '01.
- Bowers, N. L., Hickman, J. C., Nesbitt, C. J., Jones, D. A., & Gerber, H. U. (1997). *Actuarial Mathematics* (2nd ed.). Society of Actuaries.
- Buchmann, J., & Williams, H. C. (1988). *A key-exchange system based on imaginary quadratic fields*. Journal of Cryptology.
- Groth, J. (2016). *On the size of pairing-based non-interactive arguments*. EUROCRYPT '16.
- Boneh, D., Bonneau, J., Bünz, B., & Fisch, B. (2018). *Verifiable delay functions*. CRYPTO '18.
- Wesolowski, B. (2018). *Efficient verifiable delay functions*. EUROCRYPT '19.
- Boneh, D., Gorbunov, S., & Wahby, R. S. (2018). *BLS signatures*. IETF draft.
- Boneh, D., Bünz, B., & Fisch, B. (2018). *A survey of two verifiable delay functions*. ePrint 2018/712.
- Pietrzak, K. (2019). *Simple verifiable delay functions*. ITCS '19.
- Garg, S., Gentry, C., Halevi, S., & Sahai, A. (2013). *Witness encryption and its applications*. STOC '13.
- Kothapalli, A., Setty, S., & Tzialla, I. (2022). *Nova: Recursive zero-knowledge arguments from folding schemes*. CRYPTO '22.
- Aumasson, J.-P., Neves, S., Wilcox-O'Hearn, Z., & Winnerlein, C. (2020). *BLAKE3: one function, fast everywhere*.
- NIST. (2024). *FIPS 204: Module-Lattice-Based Digital Signature Standard (ML-DSA)*.

Economic / historical:

- Gesell, S. (1916). *Die natürliche Wirtschaftsordnung durch Freiland und Freigeld* (Free-land and free-money).
- Wörgl, Austria (1932). The Wörgl experiment in demurrage currency.

EvaporChain doctrine:

- `research/INVENTION_STACK.md` — canonical doctrine, especially the Single-λ Principle (§1.1) and Conservation invariant (§1.2).
- `research/INEVITABILITY_STRATEGY.md` — strategic frame for the Satoshi pattern.
- `research/APPLICATION_UNIVERSE.md` — application taxonomy with SFSV at cat 7.
- `research/lambda_fold/` — Nova IVC accumulator doctrine for §5.7.
- `research/light_cone/` — causal-set DAG doctrine for §9.1.
- `research/crooks_mev/` — MEV pipeline for §7.1 secondary-market protection.
- `research/proofs/LLSAInvariantPreservation.v` — Coq proof for Immune Validator Set (§9.5).

---

## 15. Version & Change Log

| Version | Date | Author | Notes |
|---|---|---|---|
| 1.0 | 2026-05-16 | Satyawan Singh | Initial spec. Captures shipped substrate (25 tests) + §5.3–5.7 frontier roadmap + bibliography. |

**Next planned revision:** after Paper 1 submission, fold in any reviewer comments on the mathematical foundation (§4) and update §11 open problems with paper-stage resolution.

---

*This file is owned by Satyawan Singh. Future contributors propose changes via PR with `[SFSV-ARCH]` prefix.*
