# Section B Wiring — Scoping Document

**Date:** 2026-05-20 · **Branch:** `s4-grumpkin-config` · **Parent:**
[`B1_B2_AUDIT_DOSSIER.md`](B1_B2_AUDIT_DOSSIER.md) §7 step 2.

## 1. What Section B must enforce

Per `nova-snark-0.68/src/nova/mod.rs::CompressedSNARK::verify`
L935-963, Section B is the **output-hash binding** that the verifier
checks immediately after R1CS-instance shape validation and before
the NIFS folds. There are **two** hash equalities:

```rust
let (hash_primary, hash_secondary) = {
  // ── PRIMARY HASH (squeezed on E2 RO = Grumpkin scalar field
  //    = BN254 Fq = NON-NATIVE to our BN254 Fr circuit) ───────
  let mut hasher = <E2 as Engine>::RO::new(vk.ro_consts_secondary.clone());
  hasher.absorb(vk.pp_digest);                  // 1 Bn254 Fr
  hasher.absorb(E1::Scalar::from(num_steps));   // 1 Bn254 Fr
  for e in z0 { hasher.absorb(*e); }            // |z0| Bn254 Fr
  for e in &self.zn { hasher.absorb(*e); }      // |zn| Bn254 Fr
  self.r_U_secondary.absorb_in_ro(&mut hasher); // r_U_secondary fields
  hasher.absorb(self.ri_primary);               // 1 Bn254 Fr

  // ── SECONDARY HASH (squeezed on E1 RO = Bn254 Fr = NATIVE) ──
  let mut hasher2 = <E1 as Engine>::RO::new(vk.ro_consts_primary.clone());
  hasher2.absorb(scalar_as_base::<E1>(vk.pp_digest)); // 1 Bn254 Fr (reinterp)
  hasher2.absorb(E2::Scalar::from(num_steps));        // 1 Bn254 Fr (reinterp)
  hasher2.absorb(E2::Scalar::ZERO);                   // padding
  hasher2.absorb(E2::Scalar::ZERO);                   // padding
  self.r_U_primary.absorb_in_ro(&mut hasher2);        // r_U_primary fields
  hasher2.absorb(self.ri_secondary);                  // 1 Bn254 Fr (reinterp)

  (
    hasher.squeeze(NUM_HASH_BITS, false),   // truncated → ~127 bits
    hasher2.squeeze(NUM_HASH_BITS, false),  // truncated → ~127 bits
  )
};

if hash_primary != base_as_scalar::<E1>(self.l_u_secondary.X[0])
   || hash_secondary != self.l_u_secondary.X[1]
{
  return Err(NovaError::ProofVerifyError { ... });
}
```

## 2. Why this is harder than it first looks

The natural assumption is "two Poseidon hashes ⇒ wire two
Poseidon gadgets." Reality: only ONE side is native to our
circuit's BN254 Fr field.

- **`hash_secondary` (E1 RO):** runs on BN254 Fr (the circuit's
  native field). Native Poseidon RO. Re-uses
  [`section2_gadget::enforce_poseidon_primary`] (already
  byte-correct vs neptune per Phase 2.2 Section 2).
- **`hash_primary` (E2 RO):** runs on Bn254 Fq (Grumpkin's scalar
  field = BN254's base field). **NON-NATIVE** to the Bn254 Fr
  circuit. Requires either:
  - An **`EmulatedFpVar<Bn254Fq, Bn254Fr>` Poseidon RO gadget**
    (heavy; arkworks doesn't ship this; multi-week build), OR
  - A **delegation trick**: include `hash_primary` as a public
    input to the Groth16 wrap (so the verifier sees it directly)
    and check the equality on the off-circuit side that computed
    the proof. This is sound IFF the proof generation also pins
    `hash_primary` correctly — which it does, because the off-
    circuit Rust runs the SAME `CompressedSNARK::verify` to
    generate the proof inputs.

The delegation trick is the practical path for Section B.
Building a non-native Poseidon RO gadget is multi-week effort
that buys very little since the hash_primary value is already
checked off-circuit at proof-generation time.

## 3. Public-input layout (when Section B is wired)

Section A currently allocates **zero `new_input` calls**
(`pis = []`). Section B adds:

| Index | Bn254 Fr value | Comes from |
|---|---|---|
| 0 | `hash_secondary` (claimed) | `compressed_snark.l_u_secondary.X[1]` |
| 1 | `hash_primary_reinterp` | `base_as_scalar::<E1>(compressed_snark.l_u_secondary.X[0])` |
| 2 | `pp_digest` | `vk.pp_digest` |
| 3 | `num_steps` | `compressed_snark` ambient |
| 4 | `ri_secondary` | `compressed_snark.ri_secondary` |
| 5..5+|z0| | `z0[..]` | Caller |
| 5+|z0|..5+|z0|+|zn| | `zn[..]` | `compressed_snark.zn[..]` |

Section B in-circuit:

```rust
// PRE: scalars are public inputs (allocated as new_input).
let computed = enforce_poseidon_primary(
    cs.clone(),
    &neptune_config,
    &[
        pp_digest,
        num_steps,
        Bn254Fr::ZERO, Bn254Fr::ZERO,      // E2 padding (reinterp)
        // r_U_primary fields absorbed (constant: per nova-snark
        // R1CSInstance::absorb_in_ro = comm.x, comm.y, X[0], X[1]
        // + flag, all already in public inputs as r_U_primary_* —
        // adds 4 more PIs above),
        ri_secondary,
    ],
)?;
// Enforce computed == hash_secondary_claimed.
computed.enforce_equal(&hash_secondary_pi)?;
```

And the corresponding off-circuit delegation for hash_primary:
- Off-circuit: compute `hash_primary` exactly as nova-snark does
  (via E2 RO) → emit as public input #1 (reinterpreted via
  `base_as_scalar::<E1>`).
- In-circuit: just `new_input` it. No in-circuit Poseidon check
  for hash_primary.
- Soundness: the proof generator MUST run the same hash off-
  circuit. A malicious prover who lies about hash_primary would
  immediately fail the off-circuit `CompressedSNARK::verify` they
  used to derive the proof inputs.

## 4. Constraint cost estimate

- `enforce_poseidon_primary` on ~10 absorbs at the existing config:
  ~2,500 cons per Poseidon permutation × ⌈(10+1)/8⌉ = 1-2
  permutations ⇒ ~5k cons.
- Public-input expansion: ~7 new_inputs above ⇒ +7 instance vars.
- Equality enforcement: ~1 cons each ⇒ +2 cons.
- **Total Section B: ~5k cons** (constant — independent of n_aux).

Negligible vs the ~41.5M cons Section A — adding Section B keeps
the dossier's "~43.5M cons full circuit at n_aux=16,384" claim
intact.

## 5. Test plan (when implemented)

1. **Positive:** real `CompressedSNARK` proof from canonical pp +
   real (z0, zn) → off-circuit `verify` succeeds → all 7 public
   inputs threaded → CS satisfied.
2. **Negative #1:** tamper `hash_secondary_pi` → CS UNSAT
   (Poseidon-equality breaks).
3. **Negative #2:** tamper `hash_primary_pi` → off-circuit
   verify rejects BEFORE proof generation (catches it at the
   delegation boundary).
4. **Non-vacuity:** swap two of the absorbed fields (e.g. pp_digest
   ↔ ri_secondary) → CS UNSAT.

## 6. What this scoping doc deliberately does NOT do

- Implement the Section B gate (multi-day; this doc is the spec).
- Change `RecursionDeciderCircuit`'s current public-input layout
  (would break (e)-1 / (e)-2 fixtures; needs coordinated change).
- Build a non-native Poseidon RO gadget (multi-week, and the
  delegation trick avoids needing it).

## 7. Next concrete iteration after this scoping

A. Add `section_b_wired: bool` flag + Section B field structure
   to `RecursionDeciderCircuit` (no enforcement yet — interface
   only).
B. Update `RecursionDeciderCircuit::setup_shape` and
   `section_a_only` to handle the new fields as no-ops when
   `section_b_wired=false` (preserves all existing tests + (e)
   fixtures).
C. Add `section_a_and_b_with_real_pp(...)` constructor that takes
   a real CompressedSNARK proof + pp and builds the full
   public-inputs.
D. Wire the Section B in-circuit Poseidon check + equality.
E. Add Section B tests (positive + 2 negatives + non-vacuity).
F. Re-extrapolate the cons cost and update dossier §3/§4/§7.

A-B is one iteration. C-D is one iteration. E-F is one iteration.
Three iterations from now Section B is fully closed.
