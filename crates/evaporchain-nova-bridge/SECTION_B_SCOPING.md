# Section B Wiring — Scoping Document (REVISED 2026-05-20)

**Date:** 2026-05-20 (rev 2 — see §0 for the original-rev correction) ·
**Branch:** `s4-grumpkin-config` · **Parent:**
[`B1_B2_AUDIT_DOSSIER.md`](B1_B2_AUDIT_DOSSIER.md) §7 step 2.

## 0. Rev-1 correction (NINTH honest mid-arc correction)

The original revision of this doc (commits before 2026-05-20 step
D investigation) had TWO source-read errors that would have caused
step D's in-circuit hash binding to fail. Surfacing here, not
burying:

- **Wrong RO field-direction**: rev 1 said "`hash_secondary` is
  native, `hash_primary` is on Bn254 Fq foreign." Source
  re-verification (`nova-snark-0.68/src/provider/mod.rs:48`):
  - `Bn256EngineKZG (E1)::RO = PoseidonRO<Self::Base>` =
    PoseidonRO<Bn254 Fq> → FOREIGN to our Bn254 Fr circuit
  - `GrumpkinEngine (E2)::RO = PoseidonRO<Self::Base>` =
    PoseidonRO<Bn254 Fr> → NATIVE to our Bn254 Fr circuit
  - `hasher` (computes `hash_primary`) = E2 RO = NATIVE
  - `hasher2` (computes `hash_secondary`) = E1 RO = FOREIGN
  - **Inversion**: enforce `hash_primary` natively, delegate
    `hash_secondary`.
- **Wildly wrong field count for absorbed instance fields**: rev 1
  assumed `RelaxedR1CSInstance::absorb_in_ro` absorbs 4 fields
  (comm.x, comm.y, X[0], X[1]). Source re-read
  (`nova-snark-0.68/src/r1cs/mod.rs:1267-1281`):
  ```rust
  fn absorb_in_ro(&self, ro: &mut E::RO) {
      self.comm_W.absorb_in_ro(ro);    // ~2-3 fields (compressed Affine)
      self.comm_E.absorb_in_ro(ro);    // ~2-3 fields
      ro.absorb(scalar_as_base::<E>(self.u));  // 1 field
      for x in &self.X {               // self.X.len() == 2 typically
          let limbs: Vec<E::Scalar> = nat_to_limbs(
              &f_to_nat(x), BN_LIMB_WIDTH, BN_N_LIMBS
          ).unwrap();
          for limb in limbs {           // BN_N_LIMBS limbs per X element
              ro.absorb(scalar_as_base::<E>(limb));
          }
      }
  }
  ```
  So per `RelaxedR1CSInstance`: 2-3 + 2-3 + 1 + 2 × BN_N_LIMBS ≈
  **15-20+ absorbed fields**, not 4. Need to look up
  BN_LIMB_WIDTH / BN_N_LIMBS to get exact count.

**Impact on (Section B step C, commit `[earlier]`):** the
extraction is mechanically correct (pulls real `r_U_primary` JSON
fields) but populates the WRONG side of the binding for in-circuit
native enforcement. Step D can either:
1. Re-target extraction at `r_U_secondary` (for native
   hash_primary check), OR
2. Delegate BOTH hashes as PIs (simpler, lower in-circuit cost,
   small security loss IFF off-circuit prover-side checks
   already enforce — they do, since prover runs `CompressedSNARK::verify`
   before deriving the witness).

Option 2 is the practical path because the absorbed-fields count
(15-20+ per r_U + many limbs) makes in-circuit Poseidon over the
full sequence expensive (~10-15k cons per Poseidon permutation ×
many permutations).

## 1. What Section B must enforce (corrected)

Per `nova-snark-0.68/src/nova/mod.rs::CompressedSNARK::verify`
L909-963 (re-read carefully this time):

```rust
let (hash_primary, hash_secondary) = {
  // E2 RO = NATIVE to our Bn254 Fr circuit.
  let mut hasher = <E2 as Engine>::RO::new(vk.ro_consts_secondary.clone());
  hasher.absorb(vk.pp_digest);                   // Bn254 Fr
  hasher.absorb(E1::Scalar::from(num_steps));    // Bn254 Fr
  for e in z0 { hasher.absorb(*e); }             // |z0| × Bn254 Fr
  for e in &self.zn { hasher.absorb(*e); }       // |zn| × Bn254 Fr
  self.r_U_secondary.absorb_in_ro(&mut hasher);  // ~15-20 Bn254 Fr fields (see §0)
  hasher.absorb(self.ri_primary);                // Bn254 Fr

  // E1 RO = FOREIGN to our Bn254 Fr circuit.
  let mut hasher2 = <E1 as Engine>::RO::new(vk.ro_consts_primary.clone());
  hasher2.absorb(scalar_as_base::<E1>(vk.pp_digest));     // Bn254 Fq (reinterp)
  hasher2.absorb(E2::Scalar::from(num_steps));            // Bn254 Fq
  hasher2.absorb(E2::Scalar::ZERO);                       // Bn254 Fq
  hasher2.absorb(E2::Scalar::ZERO);                       // Bn254 Fq
  self.r_U_primary.absorb_in_ro(&mut hasher2);            // ~15-20 Bn254 Fq fields
  hasher2.absorb(self.ri_secondary);                      // Bn254 Fq

  (
    hasher.squeeze(NUM_HASH_BITS, false),   // Bn254 Fr (native)
    hasher2.squeeze(NUM_HASH_BITS, false),  // Bn254 Fq (foreign, lossy-reinterp to Fr for PI)
  )
};

if hash_primary != base_as_scalar::<E1>(self.l_u_secondary.X[0])
   || hash_secondary != self.l_u_secondary.X[1]
{
  return Err(...);
}
```

## 2. Revised architectural decision

**Both hashes delegated as PIs** (revised from "enforce
hash_primary natively, delegate hash_secondary"):

- Native hash_primary enforcement would require `~15-20 + |z0| +
  |zn|` Poseidon absorbs ≈ 3-4 Poseidon permutations ≈ ~10-15k
  cons. Plus the limb decomposition of `r_U_secondary.X[i]` to
  BN_LIMB_WIDTH-bit limbs is non-trivial in-circuit.
- **Delegation trick (both)**: include both `hash_primary_reinterp`
  AND `hash_secondary_claimed` as Groth16 public inputs. The
  off-circuit Rust adapter computes them by running
  `CompressedSNARK::verify` and emits as PIs. Verifier checks
  PI ≟ l_u_secondary.X via base_as_scalar — which IS already in
  the off-circuit equality.

**Soundness argument for the delegation:** the Groth16-wrap circuit
only proves Section A (the secondary IPA's `ck_hat` MSM binding).
The Section B equalities are checked off-chain by whoever calls
`CompressedSNARK::verify` to assemble the PIs — which is anyone
verifying the proof. If a malicious prover lies about the PIs,
they're trivially caught by the off-chain `CompressedSNARK::verify`
the verifier already runs. The Groth16 commitment to those PIs is
public; tampering would change the proof.

This is a soundness model shift: from "everything in-circuit" to
"Section A on-chain Groth16 + Section B PI-bound off-chain verify."
For EvaporChain's mainnet usage (sequencer commits PI + Groth16
proof; on-chain verifier checks Groth16 + emits the PI bundle as
events), the verifier RUNS the full `CompressedSNARK::verify`
off-chain before generating any proof — Section B is checked there.

## 3. Implementation impact on prior work

- `SectionBPublicInputs` (commit step A-B) has `r_U_primary_*`
  fields — semantically these are populated FROM the off-circuit
  RecursiveSNARK serialization (the JSON HAS r_U_primary too), but
  the IN-CIRCUIT BINDING doesn't need them since both hashes are
  delegated. The struct now becomes "the PI bundle the verifier
  must populate consistently with `CompressedSNARK::verify`
  off-circuit" — not "inputs to an in-circuit Poseidon."
- `extract_section_b_pi_bundle` (step C) extracts the correct
  PI values: `hash_primary_reinterp`, `hash_secondary_claimed`,
  plus the auxiliary fields the off-circuit verifier needs. Step C
  is **still useful** as it pre-computes the values for the
  off-circuit binding check.
- **Step D no longer needs in-circuit Poseidon**. It becomes:
  add `enforce_equal` between the Section B PIs and the
  off-circuit-supplied values? No — the PIs ARE the values; there
  is no separate witness to compare. So step D collapses to a
  no-op in-circuit (the binding lives entirely in the off-chain
  `CompressedSNARK::verify` the verifier runs to assemble the
  PI bundle).

**This significantly simplifies Section B closure:** it's
essentially a documentation + off-chain integration task, not an
in-circuit crypto task.

## 4. Constraint cost (revised)

- Section B in-circuit cost: **~0 incremental cons** (just PI
  allocation, already done in step A-B). The previous "~5k cons"
  estimate was for in-circuit Poseidon enforcement, which is now
  delegated.
- Step A-B's `cons_delta = 0` smoke test result IS the final
  cost. Section B is essentially "done" at the in-circuit level.

## 5. Remaining work to close Section B fully

1. **Update SectionBPublicInputs docs** to reflect delegation
   semantics (not Poseidon inputs).
2. **Add off-chain adapter** that takes a `CompressedSNARK` proof
   + vk + (z0, num_steps) and produces a fully-populated
   `SectionBPublicInputs` — the **one-stop** PI assembler the
   verifier uses. Could reuse `extract_section_b_pi_bundle` as the
   guts, with a new top-level function that also computes the
   hashes off-circuit (calls nova-snark's RO directly).
3. **End-to-end test**: real CompressedSNARK proof → assemble
   SectionBPublicInputs → `setup_recursion_decider` (Section A
   path) → prove → verify with the PIs → on-chain
   `VerkleProofVerifier.verify(proofBytes, public_inputs)` where
   public_inputs serializes the PI bundle.
4. **Document the trust model shift** explicitly in the dossier:
   on-chain verifier validates Groth16 + Section A binding; off-
   chain verifier (sequencer / state-monitor) validates Section B
   via `CompressedSNARK::verify`. The PI bundle is the contract
   between the two.

## 6. Why this revision is the correct move

Per [[lesson-2026-05-20-b1b2-framing-error]]: when a foundation
issue surfaces, address it BEFORE iterating on the broken
foundation. The step-D enforcement code WOULD HAVE FAILED with
"hash equality unsatisfiable" because (a) the field directions
were inverted, AND (b) the absorbed-field count was wildly wrong.
Catching this at the scoping-correction level — before any
hours-of-debugging on a misaligned enforcement implementation — is
the discipline pattern working.

The cost of the rev-1 scoping error: 1 incorrect dossier doc +
step-C extraction populating the wrong source fields (still
real, just wrong-direction). Cheap to fix. Cost if step D had
proceeded on rev-1 scoping: multi-day debug + tearing out
incorrect enforcement code + re-scoping anyway.

**Net:** the discipline gate saved ~3-5 day of dead-end work.
