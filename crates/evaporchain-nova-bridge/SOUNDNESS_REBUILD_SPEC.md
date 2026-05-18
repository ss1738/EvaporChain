# Nova→Groth16 Verifier — Soundness Rebuild Spec (audit B-1/B-2)

**Status:** design (task #27). Grounds the multi-day rebuild. No code change in this doc — it is the engineering precondition for S2–S6 so they don't silently break setup/verify.

## 1. The defect (verified in code, 2026-05-18)

`verifier_circuit.rs::generate_constraints` allocates public inputs (`committed_hash_primary`, `committed_hash_secondary`, `z0`, `zi`) but the **only** code that *binds* them lives inside `if let Some(s2)` (Neptune-hash `enforce_equal`) and `if let Some(s3)` (RelaxedR1CS rows). `dummy()` — the trusted-setup shape (`groth16_wrapper::setup` → `circuit_specific_setup(dummy())`) — sets `section2/3: None`. ⇒ the setup R1CS has **zero binding constraints**; any (hash, z0, zi) satisfies it ⇒ forgeable. Production emitter `fixture-proof-emit.rs` uses the base builder (no sections) — vacuous by its own header admission.

**Core conclusion: B-1 ≡ B-2.** Groth16 keys bind to one exact R1CS. You cannot de-`Option` and attach sections at prove-time — the keys from `setup(dummy())` don't match a section-bearing prover circuit. Soundness requires the **trusted setup itself** to run over the full, fixed-dimension, section-bearing circuit. Therefore the "vacuous circuit" (B-1) and "insecure/incorrect setup" (B-2) are one change.

## 2. CANONICAL_SHAPE — pin these as setup-time constants

The setup, prover, and verifier MUST synthesize an identical constraint system. All dimensions below become compile-time constants (a `CanonicalShape` struct / consts) and are asserted at every boundary:

| Constant | Value (current fixture; pin per chosen production step circuit) | Source |
|---|---|---|
| `STEP_CIRCUIT` | `TrivialIncrementCircuit` (fixture). **Production must fix a real step circuit and re-pin the rows below.** | `recursive_snark_fixture.rs:58` |
| `ARITY` (z0.len == zi.len) | `1` | `TrivialIncrementCircuit::arity` |
| `NUM_STEPS` | fixed chain-advance cadence `N` (setup-time immediate; NOT a public input) | `verifier_circuit.rs` doc |
| `S2_ABSORB_LEN` | `18` (pp_digest, num_steps, z0[..ARITY], zi[..ARITY], l_u_secondary.X[..2], …) | `section2_gadget.rs` doc |
| `S2_NEPTUNE_PARAMS` | fixed width = rate+capacity, fixed MDS/round constants; truncate `NUM_HASH_BITS = 250` | `section2_gadget.rs`; nova-snark |
| `S3_NUM_CONS` | `≈10003` (for `TrivialIncrementCircuit`) | `section3_gadget.rs:16` |
| `S3_NUM_VARS` | `≈9995` | `section3_gadget.rs:16` |
| `S3_NUM_IO` | `2` (`x_primary` = r_U_primary.X[0..2]) | `verifier_circuit.rs` S3 branch |
| `PUBLIC_INPUTS` | `5` = committed_hash_primary + committed_hash_secondary + z0[1] + zi[1] (+1 Groth16 const) | `groth16_wrapper.rs` doc |

**Determinism invariant (CI-checkable):** the section gadgets must emit a constraint count that is a pure function of CANONICAL_SHAPE, independent of witness *values*. Plausible from code (S2 = fixed seq + fixed permutation rounds; S3 = `num_cons` rows over pre-bucketed sparse entries). S6 adds a test asserting `cs.num_constraints()` for `setup_shape()` == for a real-witness circuit.

## 3. Staged plan (each stage gated on the prior; honest multi-day)

- **S2a `setup_shape()`** — replace `dummy()` in the setup path with a constructor that attaches `section2 = Some(_)` and `section3 = Some(_)` placeholder witnesses of *exactly* CANONICAL_SHAPE dimensions. Setup runs over THIS. (Hardest correctness point: placeholder witnesses must produce the identical R1CS to a real one.)
- **S2b mandatory binding** — `generate_constraints`: emit the Neptune-hash `enforce_equal` and RelaxedR1CS-sat rows **unconditionally** (no `Option` gate). `validate_structurally` REJECTS any witness whose dims ≠ CANONICAL_SHAPE (a prover cannot substitute a smaller/absent-binding circuit).
- **S3 prover** — production emitter composes `with_section2`+`with_section3` from `extract_section2/3_witness` (real RecursiveSNARK), dims asserted == CANONICAL_SHAPE before proving.
- **S4 (separate soundness ceiling, NOT in fixed-shape)** — `comm_W`/`comm_E` KZG-commitment binding + secondary R1CS (non-native Grumpkin Fr). Documented deferred; until done the verifier is *binding on primary R1CS + transcript* but NOT commitment-bound. Do not claim "fully sound" before S4.
- **S5 trusted setup (B-2)** — MPC ceremony (Powers-of-Tau + circuit-specific phase 2) over `setup_shape()`. Interim: PR #431 `#[deprecated]` keeps the insecure path from shipping silently. `setup()` insecure path stays until ceremony params land.
- **S6 verification** — round-trip prove/verify over CANONICAL_SHAPE green **+ adversarial: a circuit/proof lacking a valid section witness MUST be unsatisfiable/rejected** (empirically proves the omission hole is closed) + the determinism constraint-count assertion.

## 4. Honest scope ceiling

Even at S6-complete-minus-S4 this is "primary-R1CS + transcript bound, commitment binding deferred" — a major improvement over vacuous, but **not full Nova-accumulator soundness**. S4 (KZG-in-circuit + non-native secondary) is genuinely deep (possibly multi-week) and is the true mainnet gate. This spec deliberately does not pretend S2–S3 alone = sound. Mitigation throughout: `VerkleProofVerifier` is NOT deployed (`Deploy.s.sol` = BLS-quorum), so this is a pre-mainnet correctness rebuild, not a live incident.

## S2a design resolution (2026-05-18 — hardest fork de-risked)

**Question that gated everything:** can the trusted-setup circuit carry `section2`/`section3` at the *exact* prover R1CS shape **without** a real Nova proof? (If not, S2a → S6 are not implementable as scoped.)

**Answer: YES.** Verified from the witness structs + gadgets:

| Field | Role | Source for canonical placeholder |
|---|---|---|
| `Section3.a/b/c_primary`, `num_cons`, `num_vars`, `num_io` | **shape-determining** | the fixed step circuit's R1CS, held in canonical `PublicParams::<E1,E2,TrivialIncrementCircuit>` — deterministic, no proof |
| `Section2.params` | shape-determining | fixed Neptune constants (JSON dump) |
| `Section2.pp_digest` | shape-determining | `pp.digest()` of the canonical `PublicParams` — deterministic |
| `Section3.w_primary/e_primary/u/x`, `Section2.comm_*/u_as_base/x*_limbs/ri_primary` | **value-only** | zeros — `enforce_primary_relaxed_r1cs_sat` emits `num_cons` gates and the Neptune sponge emits fixed rounds *independent of values* (data-independent circuit, verified) |

⇒ setup R1CS ≡ prover R1CS ≡ verifier R1CS is achievable with a value-free placeholder derived purely from the canonical `PublicParams`.

**Concrete implementation path (next, S2a-impl):**
1. `Section3Witness::canonical_shape(pp: &PublicParams)` — mirror `extract_section3_witness`'s A/B/C/dims extraction but from `pp` (R1CS shape) instead of a `RecursiveSNARK`; zero `w/e/x/u`.
2. `Section2Witness::canonical_shape(pp, neptune_params)` — fixed params + `pp.digest()→ArkFr`; zero value fields.
3. `NovaVerifierCircuit::setup_shape()` — `dummy()` arity/z0/zi at `CANONICAL_SHAPE` **with both sections = `Some(canonical_shape(...))`**.
4. `groth16_wrapper::setup` → call `setup_shape()` instead of `dummy()` (the `#[deprecated]` insecure-randomness caveat from PR #431 still applies — S5/MPC unchanged).
5. S2b: drop the `Option` gating in `generate_constraints` so the bindings are unconditional; `validate_structurally` rejects dims ≠ `CANONICAL_SHAPE`.
6. S6 determinism test: `cs.num_constraints()` for `setup_shape()` == for a real-witness circuit.

**Still genuinely multi-day:** S2a-impl needs the canonical `PublicParams` plumbing + mirroring matrix extraction; S4 (KZG comm + secondary R1CS) remains the separate soundness ceiling. This entry only records that the **gating fork is solved** — the path is now concrete and de-risked, not blind.

## S2a-impl design (2026-05-18 — implement-ready, verified against extract_*)

Verified: `extract_section3_witness` pulls **shape from `pp`** (`serde_json::to_value(pp)["r1cs_shape_primary"]` → `num_cons/num_vars/num_io` + `parse_csr` A/B/C, `num_cols = num_vars+1+num_io`, with the `MAX_R1CS_*` caps); values from `rs`. `extract_section2_witness` takes `pp_digest` as a param + `params` from the JSON dump; values from `rs`. ⇒ exact `canonical_shape` bodies:

```rust
// Section3Witness — pure fn of pp; mirrors extract_section3_witness's
// pp-derived half, values zeroed (lengths MUST match: Groth16 keys
// bind #vars/#cons, values are irrelevant to keygen).
pub fn canonical_shape(pp: &PublicParams<E1,E2,TrivialIncrementCircuit>) -> Result<Self, ExtractError> {
    let shape = &serde_json::to_value(pp)?["r1cs_shape_primary"];
    let num_cons = shape["num_cons"].as_u64()… as usize;   // + MAX caps (reuse)
    let num_vars = …; let num_io = …;
    let num_cols = num_vars + 1 + num_io;
    Ok(Self {
        w_primary: vec![ArkFr::zero(); num_vars],
        e_primary: vec![ArkFr::zero(); num_cons],
        u_primary: ArkFr::zero(), x_primary: [ArkFr::zero(); 2],
        a_primary: parse_csr(&shape["A"], num_cons, num_cols)?,
        b_primary: parse_csr(&shape["B"], num_cons, num_cols)?,
        c_primary: parse_csr(&shape["C"], num_cons, num_cols)?,
        num_cons, num_vars, num_io,
    })
}
// Section2Witness — params from the EMBEDDED canonical dump (see below)
pub fn canonical_shape(pp_digest: Scalar1) -> Result<Self, ExtractError> {
    Ok(Self {
        params: params_from_embedded()?,                 // include_bytes! (below)
        pp_digest: primary_to_ark_fr(pp_digest),
        comm_W_x: ArkFr::zero(), comm_W_y: ArkFr::zero(),
        comm_E_x: ArkFr::zero(), comm_E_y: ArkFr::zero(),
        u_as_base: ArkFr::zero(),
        x0_limbs: [ArkFr::zero();4], x1_limbs: [ArkFr::zero();4],
        ri_primary: ArkFr::zero(),
    })
}
```

**Neptune-dump wrinkle + resolution (the real design decision):** `params_from_dump_path` needs a runtime JSON file; `groth16_wrapper::setup` is called by ~15 sites with no path. Requiring a path ripples the `setup()` API + every caller. **Decision: embed the canonical Neptune dump via `include_bytes!("../assets/neptune_bn254_width25.json")`** (commit the dump as a crate asset) → `params_from_embedded()` parses the embedded bytes → `setup_shape()` is fully self-contained, `setup()` signature unchanged, zero caller ripple. (The dump is fixed protocol constants, correctly a compiled-in asset, not config.)

**`setup_shape()` + wiring:**
```rust
impl NovaVerifierCircuit {
  pub fn setup_shape() -> Result<Self, ExtractError> {
    let pp = canonical_public_params();        // deterministic PublicParams::<…TrivialIncrement>::setup
    let mut c = Self::dummy();                  // canonical ARITY z0/zi already
    c.section3 = Some(Section3Witness::canonical_shape(&pp)?);
    c.section2 = Some(Section2Witness::canonical_shape(pp.digest())?);
    Ok(c)
  }
}
// groth16_wrapper::setup: replace `NovaVerifierCircuit::dummy()` with
// `NovaVerifierCircuit::setup_shape()?` — signature unchanged; the
// #[deprecated] insecure-randomness caveat (PR #431) still stands (S5/MPC).
```

**Remaining for S2a-impl (mechanical now, de-risked):** add `assets/neptune_bn254_width25.json` + `params_from_embedded`; expose `canonical_public_params()` (factor from recursive_snark_fixture:103); the two `canonical_shape` fns; `setup_shape()`; one-line `groth16_wrapper` swap. Then S2b (drop `Option` gating + `validate_structurally` dim-reject) and S6 (`cs.num_constraints()` setup==real assertion). Still multi-day incl. the separate S4 KZG/secondary ceiling — this entry makes S2a-impl a coherent mechanical edit, not a blind crypto change.
