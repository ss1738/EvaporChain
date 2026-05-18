# S4 Design — Commitment Binding + Secondary RelaxedR1CS

Audit B-1/B-2, the true mainnet gate. Stage-1 deliverable: scope, not
code. S4 implementation is gated on S2b being box-verified green and
is explicitly multi-week (`SOUNDNESS_REBUILD_SPEC.md §4`).

This document is deliberately honest about what is **established** vs
what **must be verified against nova-snark source before any code**.
Asserting the wrong commitment model would invalidate the whole S4
build, so those points are flagged, not guessed.

## 1. Where S4 sits

Verified today (S2a + S2b, box-proven):

- **S2a:** Groth16 keys are bound to one R1CS shape; `setup_shape()`'s
  R1CS is bit-identical (S6) to a real prover circuit.
- **S2b:** Section 2 (Neptune transcript hash) and Section 3 (primary
  RelaxedR1CS satisfiability) bindings are *mandatory* — a circuit
  lacking either is unsatisfiable; a real-fixture proof verifies.

What is bound now:

- **Section 3:** prover knows `W, E, u` such that for every primary
  R1CS row `i`: `(Az)_i·(Bz)_i == u·(Cz)_i + E_i`, with `A,B,C` baked
  as circuit constants from `PublicParams`, `z = [W, u, X[0], X[1]]`.
- **Section 2:** Neptune hash binds `pp.digest, num_steps, z0, zi,
  r_U_secondary.comm_W.{x,y}, comm_E.{x,y}, u_as_base, X-limbs,
  ri_primary` to the public input `committed_hash_primary`.

## 2. The gap (the "partial soundness" caveat)

Two distinct holes remain. Closing them is S4.

### S4a — commitment binding

Section 3 proves a `W` satisfies the primary R1CS. Section 2 binds
`comm_W.{x,y}` (and `comm_E`) into the transcript hash. **Nothing
in-circuit ties the `W` used in Section 3 to the `comm_W` bound in
Section 2.** A malicious prover can satisfy Section 3 with one `W`
while the transcript commits to a `comm_W` that opens to a different
`W'`. The missing constraint is the commitment *opening*:

```
comm_W == Commit(ck, W)        comm_E == Commit(ck, E)
```

enforced inside the R1CS.

> **MUST VERIFY (blocking, before any S4a code):** which commitment
> scheme backs the *running-instance* `comm_W`/`comm_E`. In Nova these
> are commitments under the engine's `CommitmentEngine` (Pedersen-style
> MSM over the commitment group), **distinct** from the HyperKZG (E1)
> / IPA (E2) *evaluation* engines used only by the final compressing
> SNARK. If it is Pedersen-MSM, S4a is an in-circuit multi-scalar
> multiplication — **no pairing**. If any KZG opening is actually on
> the binding path, S4a needs in-circuit pairing (far deeper). The
> in-code doc-comment says "need KZG pairing"; that is **unverified**
> and may be imprecise. Resolve against `nova-snark` source
> (`CommitmentEngine` for `Bn256EngineKZG` / `GrumpkinEngine`, and
> exactly which commitment `RelaxedR1CSInstance.comm_W` carries)
> before committing to an approach.

Field/curve structure (BN254/Grumpkin 2-cycle), assuming Pedersen-MSM:

- The primary circuit is over **BN254 Fr**.
- `r_U_secondary.comm_W` is a **Grumpkin** point. Grumpkin's base
  field = BN254 Fr → its **coordinates are native** in the circuit
  (this is why Section 2 can absorb `comm_W.x/y` directly as `ArkFr`).
- But the **MSM scalars** are the committed vector entries; Grumpkin's
  scalar field = BN254 **Fq** ≠ Fr → scalar handling for an in-circuit
  Grumpkin MSM is **non-native**. This is the documented pain and
  couples S4a to S4b's non-native machinery.

### S4b — secondary RelaxedR1CS satisfiability

Nova's `is_sat_relaxed` requires **both** the primary and the
secondary accumulator's relaxed R1CS to be satisfied. Section 3 does
only the primary. The secondary R1CS is over Grumpkin's scalar field
= **BN254 Fq**, non-native to the BN254-Fr Groth16 circuit. Verifying
it in-circuit requires emulated-Fq arithmetic gadgets (non-native
field) for the full secondary `(Az)(Bz) == u(Cz)+E` system — very
deep, very expensive.

## 3. Constraint-cost reality

- Section 3 primary today: ~`num_cons` ≈ 10 003 mult gates.
- S4a Pedersen-MSM opening of `W` (len ≈ `num_vars` ≈ 9 995) +
  `E` (len ≈ `num_cons`): an MSM of ~20 k group ops with non-native
  scalars — plausibly 10⁵–10⁶ constraints depending on window/strategy.
- S4b non-native secondary R1CS: emulated-Fq for a full second R1CS —
  same order again, likely larger.

Order-of-magnitude: S4 is a multi-100k-to-millions-constraint
addition. This is why it is the true mainnet gate and not an
increment.

## 4. Staging (proposed, subject to the §2 MUST-VERIFY)

1. **S4-0 (research, no code):** read nova-snark source; pin the exact
   commitment scheme + which curve/field each of `comm_W`, `comm_E`,
   secondary-R1CS lives in. Produce a one-page "verified model"
   appendix to this doc. *Nothing downstream is correct until this is
   done.*
2. **S4-nn:** non-native BN254-Fq field gadget (shared by S4a scalars
   and S4b). Unit-prove against known vectors. Likely the longest pole.
3. **S4a:** in-circuit commitment-opening gadget; bind Section-3 `W/E`
   to Section-2 `comm_W/comm_E`. Adversarial test: mismatched
   `W` vs `comm_W` must be unsatisfiable.
4. **S4b:** in-circuit secondary RelaxedR1CS using the S4-nn gadget.
5. **S4-verify (S6 analog):** determinism (setup-shape vs real shape
   still bit-identical with S4 constraints present) **+** adversarial
   (commitment mismatch and secondary-unsat both rejected) on the box,
   real fixture.

## 5. Honest ceiling (do not overstate)

Until S4 lands, the verifier is **primary-R1CS + transcript bound,
commitment binding deferred** — a major improvement over the vacuous
circuit, but **NOT full Nova-accumulator soundness**. Do not describe
B-1/B-2 as "closed" or the verifier as "fully sound" before S4-verify
is green.

Mitigation (current, factual): this circuit / `VerkleProofVerifier`
is **not deployed** — `Deploy.s.sol` wires BLS-quorum verification —
so this is a pre-mainnet correctness rebuild, not a live incident.

## 6. Relation to S5

S5 (MPC ceremony, B-2 insecure randomness) is orthogonal to S4 and can
proceed independently; it replaces `groth16_wrapper::setup`'s insecure
`circuit_specific_setup` with Powers-of-Tau + circuit-specific phase 2
over `setup_shape()`. Neither S4 nor S5 alone closes B-1/B-2; both are
required for a mainnet claim.
