# PoHA (Decaying DA) — Formal Proof Companion

**Companion to** `research/frontier/01-poha-decaying-da.md` (the design rationale) and `research/tla/PoHA.tla` (the TLA+ specification).

**Author:** Satyawan Singh
**Date:** 2026-04-27
**Status:** v0.1 — TLA+ spec drafted, TLC runs pending. Cryptographic-soundness aspects (signature verification, sampling-protocol soundness) are out of TLC scope.

---

## 1. The theorems we are formalising

The frontier doc proposes that DA certificates carry energy and a half-life, that validators re-attest periodically (boosting the certificate's energy with a lightweight signature, no full re-sample required), and that certificates whose energy reaches zero enter a Grace period and eventually become Ghosts.

The TLA+ spec verifies the **state-machine** invariants of this lifecycle. The frontier doc identifies one open security concern explicitly:

> The sampling protocol needs careful security analysis to prevent attestation freeloading (validators claiming to have data they don't).

The state-machine spec partially addresses this via the `ActiveCertsHaveQuorum` invariant — certificates require quorum-stake attesters before they're Active. But the **cryptographic** side (verifying that an attestation truly proves the validator sampled the data) is NOT TLA+-verifiable; it requires sampling-protocol cryptographic analysis. That's flagged as open work in §5.

---

## 2. Properties verified by TLC

The spec encodes five invariants:

| Property | Statement | Source |
|---|---|---|
| `TypeOK` | All variables in declared domains | Type-level |
| `EnergyCappedAtInitial` | `cert_energy[c] <= InitialEnergy[c]` always | Re-attestation cap (`evaporchain-da/poha.rs:cap`) |
| `ActiveCertsHaveQuorum` | Active+Grace certs have attester stake ≥ quorum threshold | Bootstrap seeds full attester set; ReAttest only adds |
| `GhostsAreTerminal` | Ghost certs have energy 0 and never transition back | Action guards exclude Ghost from re-attest path |
| `AttestersAreValidators` | `cert_attesters[c] ⊆ Validators` | Type-level |

These map to specific design choices in `crates/evaporchain-da/poha.rs`:

- **Energy cap.** Production code caps energy at `E_0` on each re-attestation: `E_new = min(E_current + delta, E_0)`. Without this cap, re-attestation could indefinitely extend a certificate's lifetime past its declared half-life, defeating the whole "decaying" semantic.
- **Quorum at Active.** `is_supermajority()` at `da/poha.rs:153` enforces `attested_stake * 3 > total_stake * 2` (strict `>` since Q4 fix). The spec encodes this as a parameterised quorum (default 2/3) that all Active certs must satisfy.
- **Ghost terminality.** The DA storage layer prunes shard data once a cert is Ghost; the data is gone, so re-attestation isn't physically possible. The state machine reflects that by rejecting ReAttest when the cert is Ghost.

If any of these invariants fails under TLC's bounded check, the implementation has a bug.

---

## 3. What the spec abstracts

For TLC tractability, the model deliberately abstracts:

- **Cryptographic signatures.** The spec models attestations as set membership (`cert_attesters[c]: SUBSET Validators`). It does not model BLS aggregate signatures, signature verification, or rogue-key attacks. Those are separate verification surfaces.
- **Real DA sampling.** The spec assumes attestation is "honest" — a validator who appears in `cert_attesters[c]` truly sampled the data. The freeloading attack — where a validator signs an attestation without sampling — is the open security concern §5 addresses.
- **Decay formula.** Production uses bit-shift exponential decay; the spec uses linear `DecayPerEpoch`. Both are monotonically non-increasing in elapsed time, which is what the invariants depend on.
- **Sampling-protocol challenges.** The frontier doc proposes that "each epoch, a random subset of validators re-sample a random subset of DA certificates." The spec models re-attestation as a free action (any validator can re-attest any cert at any time) rather than as a challenge-response protocol. This is appropriate for verifying lifecycle invariants but not the freeloading attack.

These abstractions are appropriate for the property class verified. They would be **wrong** for verifying e.g. soundness of the sampling protocol against rational adversaries.

---

## 4. The state-machine theorem

The TLA+ spec proves (modulo bounded TLC coverage):

> **Theorem (PoHA lifecycle, state-machine):** For any reachable state of the model, the five invariants in §2 hold simultaneously. In particular: (a) energy is bounded by `InitialEnergy[c]`, (b) Active certs have quorum-stake attesters, (c) Ghost certs are terminal, (d) the attester set never references unknown validators.

Combined with the production code's BLS signature verification at `da/certificate.rs` and the supermajority check at `da/poha.rs:153`, this gives a **lifecycle-correct, signature-verified** PoHA implementation — assuming attesters are honest about their sampling claims.

That last assumption is precisely the open security concern.

---

## 5. The open security theorem (NOT in TLC scope)

> **Theorem (PoHA freeloading-resistance, open):** No rational validator coalition with stake `< 1 - 2/3 = 1/3` can produce a valid `DAAttestation` for a certificate whose underlying data they did NOT sample, with non-negligible probability.

This is a **cryptographic-economic** theorem about the sampling protocol design, not the lifecycle. It depends on:

- The sampling-cell-selection seed being unmanipulable by attesters (the seed is `blake3(height || validator_id || sample_index)` — but if validator_id is the attester's own, they can pre-compute which cells they'll be challenged on)
- The cell-proof verification being computationally infeasible to forge
- Honest-majority assumption on attester rationality

Proving this theorem requires:

1. A formal model of the sampling protocol (who samples what, when, with what randomness)
2. Cryptographic-game-based analysis of the attester's success probability under various adversary strategies
3. Comparison to competing DA sampling designs (Celestia's NMT + DAS, EigenDA's quorum thresholds)

Estimated effort: **multi-month research project**, appropriate for a venue like ACM CCS or USENIX Security as a standalone paper. Not something the TLA+ track addresses.

The frontier doc explicitly flags this as needing "careful security analysis"; this proof companion makes the gap explicit so the engaged auditor can scope it in or out.

---

## 6. Composition with the rest of the protocol

PoHA composes with:

- **Tendermint BFT** (`EvaporChainBFT.tla`): Block proposers attach DA attestations to their proposals; the BLS aggregate sig is verified at consensus time. The PoHA spec assumes this verification is sound.
- **Energy-Verkle Trie** (`EnergyVerkleTrie.tla`): Active DA certificates are committed in the trie; their state is governed by the Verkle compression rules. When a cert becomes Ghost, its trie entry is compressed.
- **Rule-Based Consensus** (`RuleBasedConsensus.tla`): DA certificate state queries (e.g., "is this cert Active?") flow through the lazy-eval anchor mechanism. A cert that was Active at the anchor remains queryable as Active by all validators until the next anchor, at which point its current state is re-snapshotted.

The cross-spec composition isn't formally captured in any single TLA+ module — that's the natural follow-up at the integration level.

---

## 7. What's still open

### 7.1 Sampling-protocol soundness (§5)

The freeloading attack analysis. Multi-month research project.

### 7.2 Coq / Lean mechanization

For unbounded statements of the lifecycle invariants. Mirrors the open Coq mechanization for the Rule-Based Consensus integer-decay theorem (proof companion §5.1 there).

### 7.3 Adversarial attestation model

The current spec models all attesters as honest participants. A more thorough model would let attesters arbitrarily refuse to attest, attest falsely, or selectively attest based on bribes. Verifying that the protocol still produces correct lifecycles under adversarial attestation is a real open theorem.

### 7.4 Re-attestation rate / liveness

The spec models re-attestation as a free action. In production, re-attestation must happen frequently enough to keep certs from decaying past the energy threshold. The economic model of "how often must `f < 1/3` honest validators re-attest to keep the chain functioning?" is open.

---

## 8. How an auditor should read this

For an external audit firm engaging with EvaporChain on PoHA:

1. Read `crates/evaporchain-da/poha.rs` and `crates/evaporchain-da/certificate.rs` (the implementation).
2. Read `research/frontier/01-poha-decaying-da.md` (the design rationale).
3. Read this proof companion + `research/tla/PoHA.tla` (the formal model).
4. Run TLC on `PoHA.cfg` and confirm all five invariants pass.
5. Note that the freeloading-resistance theorem (§5) is **open** — explicitly scope whether this is in or out of the engagement.

The TLA+ spec gives a clean lifecycle-correctness baseline. The cryptographic-soundness side is where the real protocol risk lives, and that's open.

---

## 9. References

- `research/frontier/01-poha-decaying-da.md` — design rationale
- `research/tla/PoHA.tla` — formal spec
- `research/tla/PoHA.cfg` — TLC configuration
- `research/tla/EnergyVerkleTrie.tla` — sister formal spec (Frontier #2)
- `research/tla/RuleBasedConsensus.tla` — sister formal spec (Frontier #3)
- `crates/evaporchain-da/poha.rs` — Rust implementation
- `crates/evaporchain-da/certificate.rs` — DA certificate types
- Lamport, L. *Specifying Systems*. Addison-Wesley, 2002.
- Al-Bassam, M., Sonnino, A., Buterin, V. *Fraud and Data Availability Proofs*. arXiv 2018. (DA sampling background.)
- Bagaria, V., Kannan, S., Tse, D. et al. *Coded Merkle Tree: Solving Data Availability Attacks in Blockchains*. Financial Cryptography 2020. (DA sampling background.)

---

**End of v0.1.**

Note for revision: §5's freeloading-resistance theorem statement is informal — the real proof would need a formal cryptographic-game model. Treat as a target-shape, not a final formulation. The engaged auditor should refine.
