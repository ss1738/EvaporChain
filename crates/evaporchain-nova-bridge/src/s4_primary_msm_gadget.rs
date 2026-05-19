//! Audit B-1/B-2 S4a PHASE B — primary (bn256-G1) MSM.
//!
//! ## FINDING (2026-05-19, box-falsified — do not regress)
//!
//! The S4_DESIGN assumption that the primary MSM is "mechanical
//! `ProjectiveVar` reuse with `EmulatedFpVar` coords" is **WRONG**.
//! Empirically (Mini 1 compile, 15 errors):
//!
//! `ark_r1cs_std::groups::curves::short_weierstrass::ProjectiveVar
//! <P, F>` is bounded `F: FieldVar<P::BaseField, BasePrimeField<P>>`.
//! For bn256-G1, `P::BaseField = ark_bn254::Fq` and
//! `BasePrimeField<P> = ark_bn254::Fq`, so the gadget REQUIRES a
//! field var whose constraint field is **Fq**. `EmulatedFpVar<Fq,
//! Fr>` is `FieldVar<Fq, Fr>` (constraint field Fr) — it does NOT
//! satisfy `FieldVar<Fq, Fq>`. **ark's SW `ProjectiveVar` only
//! supports the NATIVE case** (curve base field == circuit field).
//!
//! Consequence: the secondary side (Grumpkin, base field = Fr =
//! circuit field) is native and `ProjectiveVar`-reusable (proven:
//! `s4_msm_gadget`, A.1/A.2 [V]). The **primary side is NOT** — bn256
//! G1 coordinates are Fq, foreign to the BN254-Fr circuit. There is
//! no library drop-in.
//!
//! ## Options (require a design decision, NOT a code patch)
//!
//! 1. **Bespoke non-native SW point gadget**: implement G1 add/double
//!    and the scalar-mul ladder explicitly over `EmulatedFpVar<Fq,
//!    Fr>` (incomplete-formula safe). This is ~S4b-class depth — the
//!    primary side is therefore NOT "days/mechanical"; it is deep.
//! 2. **Avoid in-circuit bn256-G1 arithmetic**: bind the primary
//!    `comm_W` through a different mechanism (e.g. via the transcript
//!    hash already absorbing `comm_W.{x,y}` as Fq-derived field
//!    elements) so no in-circuit primary EC MSM is needed. Requires
//!    re-deriving exactly what soundness property the primary binding
//!    must enforce vs. what S2/S3 already cover.
//! 3. **Curve-cycle trick**: perform primary-instance checks in the
//!    secondary circuit of the cycle where bn256 scalars are native —
//!    a deeper Nova-folding-aware redesign.
//!
//! ## B.0 DECISION (2026-05-19, source-grounded) — Option 1
//!
//! `RecursiveSNARK::verify` (nova-snark 0.68 nova/mod.rs:567–651)
//! calls `is_sat_relaxed` on BOTH `r_U_primary`(ck_primary) AND
//! `r_U_secondary`(ck_secondary); `is_sat_relaxed`
//! (r1cs/mod.rs:447–474) recomputes `U.comm_W == Commit(ck, W)`. The
//! verify hash-check only absorbs `comm_W` coords as field elements —
//! it does NOT verify the MSM relation. ∴ **Option 2 (skip the
//! primary MSM) is UNSOUND** (permits the B-1 forgery). Option 3
//! (wrapper-as-curve-cycle) discards the working S2/S3 single
//! circuit. **Chosen: Option 1** — a bespoke non-native bn256-G1 SW
//! point gadget (`EmulatedFpVar<Fq,Fr>` coords + native-`FpVar<Fr>`-
//! scalar double-and-add ladder). Deep (≈ a second S4b) but bounded
//! and standard.
//!
//! Implementation plan (PHASE B.1→B.3, `MAINNET_REMAINING_WORK_FLOW`):
//!   B.1 non-native SW add/double gadget (incomplete-formula-safe) +
//!       isolated proof vs ark bn256-G1;
//!   B.2 native-scalar double-and-add MSM ladder + decoder/converter;
//!   B.3 `extract_primary_*` + real-fixture binding test.
//!
//! Intentionally NO gadget code yet — B.1 is the next deep unit.
//! See `S4_DESIGN.md` (corrected) + `MAINNET_REMAINING_WORK_FLOW.md`.

// ─────────────────────────────────────────────────────────────────────
// B.1 — non-native bn256-G1 short-Weierstrass point gadget (Option 1).
//
// bn256/BN254 G1: y² = x³ + 3  (a = 0, b = 3). Coordinates live in
// `ark_bn254::Fq`, foreign to the BN254-`Fr` circuit → emulated via
// `EmulatedFpVar<Fq, Fr>`. Generic-case (non-identity, P≠±Q for add,
// y≠0 for double) affine formulas — sufficient for the isolated
// correctness proof; incomplete-formula edge handling (identity /
// doubling / negation) is the B.2 ladder's concern.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_r1cs_std::fields::emulated_fp::EmulatedFpVar;
use ark_r1cs_std::fields::FieldVar;
use ark_relations::r1cs::SynthesisError;

type FqV = EmulatedFpVar<Bn254Fq, Bn254Fr>;

/// In-circuit bn256-G1 affine point (generic, non-identity).
pub struct G1AffineVar {
    pub x: FqV,
    pub y: FqV,
}

/// Point doubling, a=0:  λ = 3x²/2y;  x₃ = λ² − 2x;  y₃ = λ(x − x₃) − y.
/// Requires `p` non-identity and `y ≠ 0` (generic case).
pub fn g1_double(p: &G1AffineVar) -> Result<G1AffineVar, SynthesisError> {
    let three = FqV::constant(Bn254Fq::from(3u64));
    let two = FqV::constant(Bn254Fq::from(2u64));
    let x_sq = &p.x * &p.x;
    let num = &three * &x_sq;
    let den = &two * &p.y;
    let lambda = &num * &den.inverse()?;
    let two_x = &p.x + &p.x;
    let x3 = &(&lambda * &lambda) - &two_x;
    let y3 = &(&lambda * &(&p.x - &x3)) - &p.y;
    Ok(G1AffineVar { x: x3, y: y3 })
}

/// Point addition of DISTINCT points (x₁ ≠ x₂):
/// λ = (y₂−y₁)/(x₂−x₁); x₃ = λ² − x₁ − x₂; y₃ = λ(x₁ − x₃) − y₁.
pub fn g1_add(p: &G1AffineVar, q: &G1AffineVar) -> Result<G1AffineVar, SynthesisError> {
    let num = &q.y - &p.y;
    let den = &q.x - &p.x;
    let lambda = &num * &den.inverse()?;
    let x3 = &(&(&lambda * &lambda) - &p.x) - &q.x;
    let y3 = &(&lambda * &(&p.x - &x3)) - &p.y;
    Ok(G1AffineVar { x: x3, y: y3 })
}

// ── B.2: native-scalar double-and-add ladder over B.1 ────────────────
//
// In-circuit `k·base` for a native `FpVar<Fr>` scalar, MSB-first
// double-and-add: per bit compute `doubled = 2·acc` and
// `added = doubled + base`, then `acc = bit ? added : doubled`
// (always-compute-both + constant-time select). GENERIC-CASE: caller
// supplies MSB-first bits whose leading bit is 1 and whose
// intermediates never hit identity / P=±Q (true for `base=G`, small
// `k` — sufficient for the isolated proof). Full edge-safety
// (leading zeros, identity, degenerate intermediates → complete
// formulas or offset trick) is the B.2-hardening follow, documented
// like B.1's generic→hardening staging.

use ark_r1cs_std::boolean::Boolean;
use ark_r1cs_std::select::CondSelectGadget;

fn g1_select(
    cond: &Boolean<Bn254Fr>,
    t: &G1AffineVar,
    f: &G1AffineVar,
) -> Result<G1AffineVar, SynthesisError> {
    Ok(G1AffineVar {
        x: FqV::conditionally_select(cond, &t.x, &f.x)?,
        y: FqV::conditionally_select(cond, &t.y, &f.y)?,
    })
}

/// `k·base`, MSB-first double-and-add. `bits_msb_first[0]` must be 1
/// (generic-case contract); `acc` initialised to `base`.
pub fn g1_scalar_mul(
    base: &G1AffineVar,
    bits_msb_first: &[Boolean<Bn254Fr>],
) -> Result<G1AffineVar, SynthesisError> {
    let mut acc = G1AffineVar {
        x: base.x.clone(),
        y: base.y.clone(),
    };
    for bit in &bits_msb_first[1..] {
        let doubled = g1_double(&acc)?;
        let added = g1_add(&doubled, base)?;
        acc = g1_select(bit, &added, &doubled)?;
    }
    Ok(acc)
}

// ── B.2b: primary Pedersen/HyperKZG MSM = Σ scalarᵢ·baseᵢ + r·h ───────
//
// Composes the proven B.2 ladder over each (base, scalar) + the
// blind term, accumulating with the proven B.1 `g1_add`. Accumulator
// is initialised to the FIRST term (NOT identity) to stay in B.1's
// generic add domain. `scalar_bits[i]` / `blind_bits` are MSB-first
// with leading 1 (B.2 contract). Edge-safe arbitrary-`W` form =
// B.2-hardening follow.
pub fn pedersen_msm_bn256_g1(
    scalar_bits: &[Vec<Boolean<Bn254Fr>>],
    bases: &[G1AffineVar],
    blind_bits: &[Boolean<Bn254Fr>],
    h: &G1AffineVar,
) -> Result<G1AffineVar, SynthesisError> {
    assert_eq!(
        scalar_bits.len(),
        bases.len(),
        "pedersen_msm_bn256_g1: scalars/bases length mismatch"
    );
    assert!(!bases.is_empty(), "MSM needs ≥1 base");
    let mut acc = g1_scalar_mul(&bases[0], &scalar_bits[0])?;
    for (sb, b) in scalar_bits[1..].iter().zip(&bases[1..]) {
        let term = g1_scalar_mul(b, sb)?;
        acc = g1_add(&acc, &term)?;
    }
    let blind_term = g1_scalar_mul(h, blind_bits)?;
    g1_add(&acc, &blind_term)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{short_weierstrass::Projective, AffineRepr, CurveGroup};
    use ark_r1cs_std::{
        alloc::AllocVar, boolean::Boolean, convert::ToBitsGadget,
        fields::fp::FpVar, R1CSVar,
    };
    use ark_relations::r1cs::ConstraintSystem;

    /// THE B.1 PRIMITIVE PROOF: in-circuit non-native bn256-G1
    /// double + add equal out-of-circuit ark bn256-G1, CS satisfied.
    /// Generic points only (G, 2G, 3G are distinct & non-identity).
    #[test]
    fn nonnative_bn256_g1_double_add_match_ark() {
        let g_aff = ark_bn254::G1Affine::generator();
        let g = Projective::from(g_aff);
        let two_g = (g + g).into_affine();
        let three_g = (g + g + g).into_affine();

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mkfq = |v: Bn254Fq| FqV::new_witness(cs.clone(), || Ok(v)).unwrap();
        let gx = g_aff.x().unwrap();
        let gy = g_aff.y().unwrap();
        let gp = G1AffineVar { x: mkfq(gx), y: mkfq(gy) };

        // double: 2G
        let d = g1_double(&gp).expect("g1_double");
        // add (distinct): G + 2G = 3G
        let twog_v = G1AffineVar {
            x: mkfq(two_g.x().unwrap()),
            y: mkfq(two_g.y().unwrap()),
        };
        let s = g1_add(&gp, &twog_v).expect("g1_add");

        assert!(cs.is_satisfied().expect("is_satisfied"), "CS must be satisfied");
        assert_eq!(d.x.value().unwrap(), two_g.x().unwrap(), "2G.x");
        assert_eq!(d.y.value().unwrap(), two_g.y().unwrap(), "2G.y");
        assert_eq!(s.x.value().unwrap(), three_g.x().unwrap(), "3G.x");
        assert_eq!(s.y.value().unwrap(), three_g.y().unwrap(), "3G.y");
    }

    /// THE B.2 PRIMITIVE PROOF: the native-scalar double-and-add
    /// ladder over B.1 computes `k·G` matching ark. `k=5` (`101`b,
    /// MSB-first leading 1; intermediates G,2G,4G,5G all distinct
    /// non-identity → generic-safe). Scalar bits come from a real
    /// native `FpVar<Fr>` witness (the variable-scalar path).
    #[test]
    fn nonnative_bn256_g1_scalar_mul_matches_ark() {
        let g_aff = ark_bn254::G1Affine::generator();
        let g = Projective::from(g_aff);
        let k = 5u64;
        let expected = (g * Bn254Fr::from(k)).into_affine();

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mkfq = |v: Bn254Fq| FqV::new_witness(cs.clone(), || Ok(v)).unwrap();
        let gp = G1AffineVar {
            x: mkfq(g_aff.x().unwrap()),
            y: mkfq(g_aff.y().unwrap()),
        };

        // Scalar from a real native FpVar<Fr> witness → bits.
        let kv = FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(k)))
            .unwrap();
        let bits_le = kv.to_bits_le().unwrap(); // LSB-first
        // k=5 fits in 3 bits; take low 3, reverse → MSB-first [1,0,1].
        let mut msb: Vec<Boolean<Bn254Fr>> = bits_le[..3].to_vec();
        msb.reverse();

        let out = g1_scalar_mul(&gp, &msb).expect("g1_scalar_mul");

        assert!(cs.is_satisfied().expect("is_satisfied"), "CS must be satisfied");
        assert_eq!(out.x.value().unwrap(), expected.x().unwrap(), "5G.x");
        assert_eq!(out.y.value().unwrap(), expected.y().unwrap(), "5G.y");
    }

    /// THE B.2b PRIMITIVE PROOF: the full primary MSM
    /// `Σ sᵢ·baseᵢ + r·h` in-circuit equals out-of-circuit ark
    /// bn256-G1. Bases [G, 2G], scalars [4, 5], h=3G, blind r=7
    /// (all 3-bit, MSB=1; intermediates generic-safe).
    /// Expected = 4·G + 5·(2G) + 7·(3G) = 35G.
    #[test]
    fn nonnative_bn256_g1_msm_matches_ark() {
        let g_aff = ark_bn254::G1Affine::generator();
        let g = Projective::from(g_aff);
        let two_g = (g + g).into_affine();
        let three_g = (g + g + g).into_affine();
        let expected =
            (g * Bn254Fr::from(4u64) + (g + g) * Bn254Fr::from(5u64)
                + (g + g + g) * Bn254Fr::from(7u64))
            .into_affine();

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mkfq = |v: Bn254Fq| FqV::new_witness(cs.clone(), || Ok(v)).unwrap();
        let pt = |a: ark_bn254::G1Affine| G1AffineVar {
            x: mkfq(a.x().unwrap()),
            y: mkfq(a.y().unwrap()),
        };
        // MSB-first 3-bit vec from a real native FpVar<Fr> witness.
        let bits3 = |k: u64| -> Vec<Boolean<Bn254Fr>> {
            let kv =
                FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(k)))
                    .unwrap();
            let mut b = kv.to_bits_le().unwrap()[..3].to_vec();
            b.reverse();
            b
        };

        let bases = [pt(g_aff), pt(two_g)];
        let scalar_bits = [bits3(4), bits3(5)];
        let h_v = pt(three_g);
        let blind_bits = bits3(7);

        let out = pedersen_msm_bn256_g1(&scalar_bits, &bases, &blind_bits, &h_v)
            .expect("pedersen_msm_bn256_g1");

        assert!(cs.is_satisfied().expect("is_satisfied"), "CS must be satisfied");
        assert_eq!(out.x.value().unwrap(), expected.x().unwrap(), "MSM.x = 35G.x");
        assert_eq!(out.y.value().unwrap(), expected.y().unwrap(), "MSM.y = 35G.y");
    }
}
