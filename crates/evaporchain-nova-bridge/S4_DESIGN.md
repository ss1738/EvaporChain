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

> **RESOLVED (S4-0, verified against nova-snark v0.68.0 source):**
> `RelaxedR1CSInstance.comm_W/comm_E = Commitment<E>` =
> `<E::CE as CommitmentEngineTrait>::Commitment`. For the two engines:
> - **E1 `Bn256EngineKZG`** (primary): `type CE =
>   HyperKZGCommitmentEngine`. Its `commit` (hyperkzg.rs:566) is
>   `vartime_multiscalar_mul(v, ck.ck) + h·r` — a **plain MSM** over
>   BN256 G1. Pairing appears in hyperkzg.rs **only** inside the
>   eval-proof `verify` (line 1191), which is the compressing-SNARK
>   opening, **NOT** the running-instance commitment.
> - **E2 `GrumpkinEngine`** (secondary): `type CE =
>   PedersenCommitmentEngine`. `commit` (pedersen.rs:285) is the same
>   form: `vartime_multiscalar_mul(v, ck.ck) + h·r` over Grumpkin.
>
> **Conclusion: S4a is an in-circuit MSM, NOT in-circuit pairing.**
> The in-code "need KZG pairing" comment is **wrong** (it conflated
> the commitment with HyperKZG's evaluation proof). This materially
> de-risks S4a vs the worst case.

Field/curve structure (BN254/Grumpkin 2-cycle) — circuit is over
**BN254 Fr** (= `bn256::Scalar`). Per instance, an MSM
`comm = Σ vᵢ·ckᵢ + h·r` has one native side and one non-native side:

| Instance | Commitment group | Point coords | MSM scalars (`v`) |
|---|---|---|---|
| **Primary** (Bn256/HyperKZG) | BN256 G1 | `bn256::Base` = BN254 **Fq** → *non-native* | primary `W` ∈ `bn256::Scalar` = BN254 **Fr** → *native* |
| **Secondary** (Grumpkin/Pedersen) | Grumpkin | `grumpkin::Base` = BN254 **Fr** → *native* (why Section 2 absorbs `comm_W.x/y` directly as `ArkFr`) | secondary witness ∈ `grumpkin::Scalar` = BN254 **Fq** → *non-native* |

So **no pairing anywhere**; each MSM needs non-native machinery on
exactly one side (primary: non-native G1 point arithmetic, native
scalars; secondary: native point arithmetic, non-native scalars).

Structural note: Section 3 currently enforces the **primary** R1CS;
Section 2 currently binds the **secondary** instance's
`comm_W/comm_E.{x,y}`. Full binding requires S4a on **both** running
instances — bind primary Section-3 `W/E` to the **primary** `comm_W`
(currently not surfaced into the transcript) AND the secondary. This
asymmetry is a real design item, not just an implementation detail.

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
- S4a MSM openings (no pairing): `W` (len ≈ `num_vars` ≈ 9 995) +
  `E` (len ≈ `num_cons`) per instance — ~20 k EC group ops each.
  In-circuit variable-base MSM is ~hundreds of constraints per point
  (double-and-add + non-native side), so plausibly 10⁵–10⁶ constraints
  total. Far cheaper than in-circuit pairing (the de-risk from S4-0).
- S4b non-native secondary R1CS: emulated-Fq for a full second R1CS —
  same order again, likely larger.

Order-of-magnitude: S4 is a multi-100k-to-millions-constraint
addition (MSM-bound, not pairing-bound). Still the true mainnet gate
and not an increment, but S4-0 removes the in-circuit-pairing worst
case.

## 4. Staging (proposed, subject to the §2 MUST-VERIFY)

1. **S4-0 (research, no code) — ✅ DONE.** Commitment model pinned
   against nova-snark v0.68.0 source: both running-instance
   commitments are MSM (`Σ vᵢ·ckᵢ + h·r`), no pairing; native/
   non-native split tabulated in §2. Result recorded inline above
   (this *is* the verified model; no separate appendix needed).
2. **S4-nn:** non-native BN254-**Fq** field gadget (used by secondary
   MSM scalars + S4b's secondary R1CS, and primary G1 point coords).
   Unit-prove against known vectors. Likely the longest pole.
3. **S4a:** in-circuit variable-base MSM opening gadget; bind
   Section-3 `W/E` to `comm_W/comm_E` for **both** running instances
   (incl. surfacing the primary `comm_W` into the transcript — see §2
   structural note). Adversarial test: mismatched `W` vs `comm_W`
   must be unsatisfiable.
4. **S4b:** in-circuit secondary RelaxedR1CS using the S4-nn gadget.
5. **S4-verify (S6 analog):** determinism (setup-shape vs real shape
   still bit-identical with S4 constraints present) **+** adversarial
   (commitment mismatch and secondary-unsat both rejected) on the box,
   real fixture.

Next concrete unit: **S4-nn** (the non-native Fq gadget) — the
foundation both S4a (one side per instance) and S4b rest on. Gated
only on S2b green now (S4-0 done).

## 7. S4-nn / architecture resolution (verified against ark 0.5 source)

A second round of source verification (ark-r1cs-std 0.5 on the box)
collapses most of the bespoke-crypto risk into **library
composition**:

- **Non-native field is library-provided.** `ark_r1cs_std::fields::
  emulated_fp::EmulatedFpVar<TargetF, BaseF>` is a complete emulated
  field gadget (`field_var.rs`, `reduce.rs`, `mul_without_reduce`,
  full `FieldVar` impl). **Do NOT hand-roll CRT/limb non-native
  arithmetic.** S4-nn reduces to *instantiate + validate*
  `EmulatedFpVar<Fq, Fr>` for our operations, not implement it.
- **EC MSM is library-provided and generic.** `ark_r1cs_std::groups::
  curves::short_weierstrass::ProjectiveVar<P: SWCurveConfig, F:
  FieldVar<P::BaseField, BasePrimeField<P>>>` implements `CurveVar`
  with `scalar_mul_le` / `fixed_scalar_mul_le` /
  `precomputed_base_scalar_mul_le`. `F` is the **coordinate** field
  var, chosen per instance:
  - **Secondary** (Grumpkin/Pedersen): `P::BaseField` = Grumpkin base
    = BN254 Fr = circuit field → `ProjectiveVar<GrumpkinCfg,
    FpVar<Fr>>` — **native point arithmetic**; non-native only for the
    scalar bit-decomposition (`EmulatedFpVar<Fq,Fr>` → bits →
    `scalar_mul_le`).
  - **Primary** (BN256 G1/HyperKZG): `P::BaseField` = `bn256::Base` =
    BN254 Fq ≠ Fr → `ProjectiveVar<Bn256Cfg, EmulatedFpVar<Fq,Fr>>` —
    non-native point arithmetic (library), native scalars.
- **No ark Grumpkin/bn256 crate** (nova uses halo2curves). Real
  remaining bespoke work = define ark `SWCurveConfig` for nova's
  Grumpkin and bn256 G1 from their **public** curve constants
  (`COEFF_A`, `COEFF_B`, field assoc types), matching
  halo2curves' coordinate/serialization convention (the same
  decompress `section2_witness` already does for `comm.{x,y}`).

### S4-nn step-1 inputs — Grumpkin `SWCurveConfig` (authoritative, self-checked)

Extracted from halo2curves 0.9.0 `grumpkin::G1Affine::generator()` on
the box (canonical `to_repr`, not Montgomery) and decimal-converted +
on-curve-verified (`y² ≡ x³−17 mod p` holds for the generator):

- `BaseField` (point coords, circuit-native) = `ark_bn254::Fr`
  - modulus `p = 21888242871839275222246405745257275088548364400416034343698204186575808495617`
- `ScalarField` (MSM scalars, non-native) = `ark_bn254::Fq`
- `COEFF_A = 0`
- `COEFF_B = p − 17 = 21888242871839275222246405745257275088548364400416034343698204186575808495600`
- `GENERATOR = (1, 17631683881184975370165255887551781615748388533673675138860)`
- cofactor = 1 (prime order)

Mandatory validation (the non-faked gate, must run on box): a
`#[cfg(test)]` cross-library check — instantiate halo2curves
`grumpkin` generator + `b`, compare canonical bytes to the ark
`GrumpkinConfig` consts; assert ark `GENERATOR` is on-curve and has
the expected prime order; decompress a real-fixture `comm_W` (the
same path `section2_witness` uses) and assert it lies on the
ark-defined curve. Config is NOT trusted until this passes on the box.

### S4-nn step-2 — primary (bn256-G1) config: REUSE ark, no bespoke

Source-verified on the box: `ark_bn254::g1::Config` is a complete
`SWCurveConfig` (`BaseField=ark_bn254::Fq`, `ScalarField=ark_bn254::Fr`,
`COEFF_A=0`, `COEFF_B=3`, `GENERATOR=(1,2)`). halo2curves `bn256` G1
is identical (`G1_A=0`, `G1_B=3`, generator `(1,2)`), and the
constants are tiny (1/2/3) so there is **no Montgomery-vs-canonical
ambiguity** to resolve. **The primary commitment group needs NO
bespoke config — reuse `ark_bn254::g1::Config` directly.** Only a
small cross-library validation test is required (ark vs halo2curves
generator bytes + a real-fixture primary `comm_W` decompresses onto
ark BN254-G1). Net: of the two curve configs, only the **secondary**
(`GrumpkinConfig`) is bespoke; the **primary** is library-provided.
This further shrinks S4-nn.

### S4a real-fixture wiring map (authoritatively pinned, nova 0.68 source)

The MSM gadget (`s4_msm_gadget::pedersen_msm_grumpkin`, box-proven in
isolation) is wired to a real fixture via these **source-verified**
serde paths (the structs ARE the serde JSON shape — not guessed):

- `PublicParams` named fields include `ck_secondary: CommitmentKey<E2>`
  → `serde_json::to_value(pp)["ck_secondary"]`.
- `CommitmentKey<E>` = `{ ck: Vec<AffineGroupElement>, h:
  AffineGroupElement }` →
  - `pp_json["ck_secondary"]["ck"]` = secondary Pedersen MSM bases
  - `pp_json["ck_secondary"]["h"]` = blinding generator
- `RecursiveSNARK` JSON `r_W_secondary` = `RelaxedR1CSWitness<E2>` =
  `{ W:[scalar], r_W:scalar, E:[scalar], r_E:scalar }` →
  - `rs_json["r_W_secondary"]["W"]` = secondary witness vector
  - `rs_json["r_W_secondary"]["r_W"]` = blinding scalar
  - scalars are E2 scalar = Grumpkin scalar = BN254 **Fq**, `0x`
    BE-hex via `EvmCompatSerde` (reuse the `scalar_adapter` /
    `l_u_secondary_extract` decode path).
- `r_U_secondary.comm_W` = the Section-2-bound committed point
  (already extracted by `section2_witness`; `Commitment.comm` has
  `#[serde_as(as = "EvmCompatSerde")]` → its proven hex→decompress
  path).

> **S4a-wiring-0 RESULT (PINNED from a real `serde_json::to_value(pp)`
> box dump — `dump_ck_secondary_shape`, nova 0.68):**
> - `PP_TOP_KEYS` include `ck_secondary` (and `ck_primary`,
>   `r1cs_shape_*`); `digest` absent (confirms `#[serde(skip)]`).
>   `PublicParams`/`CommitmentKey` serde reaches it. ✓
> - `ck_secondary` is an object with keys `["ck", "h"]`.
> - `ck_secondary.ck` is an array of **16384** elements (2¹⁴ Pedersen
>   bases); the MSM uses `ck[..W.len()]` (nova: `ck.len() >= v.len()`).
> - Each `ck[i]` and `h` is a **bare 64-char hex string = 32
>   compressed bytes, NO `0x` prefix** (halo2curves native
>   `AffineGroupElement` Serialize = compressed point). Confirmed
>   distinct from `comm_W`'s `EvmCompatSerde` (`0x`-prefixed) path —
>   the earlier correction was right.
>   - e.g. `ck[0] = "da04349ffe210dacac0edfc90136c939702d7fc021d372d0b3aa0bceee3e9e2b"`
> - **Decoder:** `hex::decode(s)` (no `0x` strip) → 32 bytes →
>   halo2curves `grumpkin::G1Affine` compressed `from_bytes` (same
>   decompression `section2_witness` performs for `comm_W`, just fed
>   bare-hex bytes) → map coords to `ark_bn254::Fr` → `Affine<GrumpkinConfig>`.
> Encoding is now observed, not assumed — extractor is execution
> against a verified format.

**Binding to enforce:** `r_U_secondary.comm_W == Σ Wᵢ·ckᵢ + r_W·h`.
Validation = a real-fixture `#[ignore]` test (real Nova fixture,
S6-class): extract → `pedersen_msm_grumpkin` → assert `== comm_W`;
adversarial: perturb `W[0]` → assert `!=`. Next executable step =
**S4a-wiring-0**: box-dump `serde_json::to_value(pp)["ck_secondary"]`
structure from a real fixture, pin the affine encoding, THEN write
the `ck`/`W` extractor + box-iterate (as `l_u_secondary_extract` was
"verified empirically").

### S4a extractor — complete executable spec (all primitives pinned)

Every decoder is now an exact, codebase-proven reuse. No guessing
remains; this is mechanical execution + box-iterate.

**Point decode (ck[i], h — bare hex; comm_W — `0x` hex):** reuse
`section2_witness.rs:208-223` verbatim pattern —
```
use nova_snark::provider::bn256_grumpkin::grumpkin::Affine as GrumpkinAffine;
use group::GroupEncoding; use crate::scalar_adapter::primary_to_ark_fr;
let s = hex.strip_prefix("0x").unwrap_or(hex);          // handles bare + 0x
let mut arr=[0u8;32]; arr.copy_from_slice(&hex::decode(s)?);
let repr: <GrumpkinAffine as GroupEncoding>::Repr = arr.into();
let a = Option::<GrumpkinAffine>::from(GrumpkinAffine::from_bytes(&repr)).ok_or(..)?;
let pt = ark_ec::short_weierstrass::Affine::<GrumpkinConfig>::new(
            primary_to_ark_fr(a.x), primary_to_ark_fr(a.y));   // validated by grumpkin_config trust gate
```
(`nova_snark` is a normal dep — no `halo2curves` in `[dependencies]`.)

**Scalar decode (W[i], r_W — secondary scalar = BN254 Fq, EvmCompat
`0x` BE-hex):** reuse `l_u_secondary_extract::parse_secondary_scalar_hex`
(proven) → `SecondaryScalar`, then **one new exact converter** in
`scalar_adapter.rs` mirroring `primary_to_ark_fr` but targeting Fq:
```
pub fn secondary_to_ark_fq(s: SecondaryScalar) -> ark_bn254::Fq {
    let repr: [u8;32] = s.to_repr().into();
    ark_bn254::Fq::from_le_bytes_mod_order(&repr)   // EXACT: target IS Fq, value<q
}
```
(NOT `secondary_to_ark_fr_lossy` — that reduces mod Fr, wrong for MSM
scalars. This is the only genuinely new code.)

**Module `s4_secondary_extract.rs`:**
- `extract_secondary_ck(pp_json) -> (Vec<Affine<GrumpkinConfig>>, Affine<GrumpkinConfig>)`
  from `pp_json["ck_secondary"]["ck"]` (array, len 16384) + `["h"]`.
- `extract_secondary_witness(rs_json) -> (Vec<Fq>, Fq)` from
  `rs_json["r_W_secondary"]["W"]` + `["r_W"]`.
- `extract_secondary_comm_w(rs_json) -> Affine<GrumpkinConfig>` from
  `rs_json["r_U_secondary"]["comm_W"]["comm"]` (point decode above).

**Real-fixture `#[ignore]` binding test (S6-class, needs real Nova
fixture):** build `generate_fixture_with_digest` + `canonical_public_params`;
`serde_json::to_value` both; extract; `let n=W.len();`
`pedersen_msm_grumpkin(&W, &ck[..n], &r_W, h)` (W/r_W as
`EmulatedFpVar<Fq,Fr>` witnesses) → assert `== comm_W`. Adversarial:
perturb `W[0]` → assert `!=`. Box-iterate endianness exactly as
`l_u_secondary_extract` was "verified empirically".

### Primary (bn256-G1) MSM analog — pinned from nova 0.68 source

HyperKZG `CommitmentKey<E>` `#[derive(Serialize)]` =
`{ ck: Vec<AffineGroupElement>, h: AffineGroupElement, tau_H: G2Affine }`.
`commit` (hyperkzg.rs:566) = `vartime_multiscalar_mul(v, ck.ck[..]) +
group(ck.h)·r` — **identical MSM form to Pedersen**; `tau_H` is the
verifier key only (NOT in the commitment → ignore).

JSON paths (serde derive): `pp_json["ck_primary"]["ck"|"h"]`;
`rs_json["r_W_primary"]["W"|"r_W"]`;
`rs_json["r_U_primary"]["comm_W"]["comm"]`.

**Field inversion vs secondary** (the only structural differences):
- Scalars `W`/`r_W` = E1 scalar = `bn256::Scalar` = BN254 **Fr** =
  circuit-NATIVE → use existing exact `scalar_adapter::primary_to_ark_fr`
  (NOT `secondary_to_ark_fq`); gadget scalars are **native `FpVar<Fr>`**,
  NOT `EmulatedFpVar`.
- Point group = bn256 G1; coords = `bn256::Base` = BN254 **Fq** =
  non-native → `ProjectiveVar<ark_bn254::g1::Config,
  EmulatedFpVar<Fq,Fr>>`; need a bn256-G1 decoder
  (`nova_snark::provider::bn256_grumpkin::bn256::Affine` +
  `GroupEncoding`, analog of the grumpkin one) and a coord
  `bn256::Base → ark_bn254::Fq` exact converter (analog of
  `primary_to_ark_fr`, target Fq).

So: `pedersen_msm_bn256_g1` gadget (native-scalar / non-native-coord
inverse of `pedersen_msm_grumpkin`) + `extract_primary_*` (mirror of
`s4_secondary_extract`, scalar=`primary_to_ark_fr`, point=bn256-G1
decoder) + real-fixture binding test `r_U_primary.comm_W == Σ Wᵢ·ckᵢ
+ r_W·h`. All paths pinned; no guessing — mechanical once the
secondary binding validates (it gates the shared structure).

### CAPACITY FINDING (2026-05-19, empirical — incident)

The S4a secondary-binding `#[ignore]` test ran an in-circuit
**non-native** Pedersen MSM over the **full real secondary `W`**
(secondary R1CS `num_vars`, ~thousands; each scalar an
`EmulatedFpVar<Fq,Fr>` ≈254-bit decomposition + `scalar_mul_le`).
On the 4 GB Hetzner node-box this exhausted RAM (89 MB free, load
avg 7.6 / 2 vCPU, `sshd` starved — required OOM-kill). **Empirical
proof of the documented S4a/S4b constraint blow-up: full-real-`W`
non-native synthesis does NOT fit in 4 GB.** This is a hardware
boundary, NOT a gadget-correctness failure (the isolated primitive
proofs — `pedersen_msm_grumpkin`, `secondary_to_ark_fq`, S4b
row-sat — are small and box-validated/validatable).

**Scoping decision (honest, recorded):**
- Real-fixture integration tests (`s4_secondary_extract`,
  primary analog, S4b real-witness) MUST cap the MSM/R1CS to a
  **bounded `W` prefix** (e.g. `min(W.len(), N)` with small `N`)
  on the 4 GB box — proves the *binding logic* end-to-end at
  tractable memory.
- **Full-length real-`W`** verification is a SEPARATE heavier run
  on a bigger machine (M4 Mini cluster / `satyawan-1`), tracked as
  its own item — never run on the node-box again (it endangers the
  permanent public node).
- A binding "pass" is ONLY claimable from a run that actually fit
  in memory and printed explicit `... ok` / `N passed`. An
  OOM-died run is NOT a result.

**Net re-scope:** no pairing (S4-0), no hand-rolled non-native field,
no hand-rolled EC gadget. S4 = (a) define 2 curve configs from public
constants, (b) compose `ProjectiveVar` MSM with the right coord-field
var per instance, (c) bind to Section 2/3, (d) secondary R1CS in
`EmulatedFpVar`, (e) S4-verify with real-fixture validation +
adversarial. Still the true mainnet gate and still substantial
(constraint cost in §3 unchanged), but **no bespoke cryptographic
primitives** — the deep risk is now correctness-of-composition +
curve-config exactness, validated empirically (a real fixture's known
`comm_W` must reproduce in-circuit; a wrong `W` must fail).

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
