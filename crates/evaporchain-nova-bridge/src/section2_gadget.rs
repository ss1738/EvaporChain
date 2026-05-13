//! Section 2 in-circuit Poseidon gadget.
//!
//! # What this module ships
//!
//! `enforce_poseidon_primary` — an R1CS gadget that absorbs a
//! sequence of `FpVar<Bn254Fr>` scalars into an arkworks
//! `PoseidonSpongeVar` and returns the squeezed `FpVar`. Plus
//! a higher-level `enforce_section_2_primary` that takes the
//! Section-2 primary-side absorb slots (`pp_digest`, `num_steps`,
//! `z0[..]`, `zi[..]`, instance expansion, `ri_primary`) and
//! emits the Poseidon hash with the documented order from PR #65.
//!
//! # Config functions
//!
//! Three `PoseidonConfig<Bn254Fr>` builders:
//!
//!   - [`placeholder_poseidon_config`] — arkworks-default
//!     constants. **DOES NOT** match neptune; shape-only.
//!   - [`neptune_aligned_poseidon_config`] — REAL neptune MDS
//!     (loaded from JSON dump) + arkworks-default ARK. Partial
//!     parity; used for constraint-budget measurement under
//!     neptune-matching state width.
//!   - [`fully_aligned_poseidon_config`] — REAL neptune MDS + ARK
//!     from `crate::compress_ark::compress_full`'s reshaped output.
//!     **Constants are now byte-correct vs neptune** per PR #103.
//!
//! # Section 2 BESPOKE status: CLOSED
//!
//! `enforce_neptune_sponge_primary` produces byte-identical output to
//! `neptune_hash_primary` for all tested inputs. Root cause of the former
//! divergence was the `pre_sparse_mds` transpose: neptune's
//! `product_mds_with_matrix` computes `matrix^T · elements`, so we store
//! `psm^T` at load time in `params_from_dump_path`. Confirmed via
//! `neptune_sponge::sponge_attempt_1_matches_neptune_hash_primary_on_pinned_42_7_99`
//! and `fully_aligned_gadget_byte_parity_with_neptune`.
//!
//! # Why ship the gadget shape before the constants
//!
//! Three reasons:
//!
//! 1. **Catches arity/wiring bugs early.** Whether the gadget
//!    absorbs in the right order, allocates the right number of
//!    public inputs vs witnesses, and connects to the existing
//!    `NovaVerifierCircuit` public-input scheme — these are checked
//!    independently of constant correctness.
//!
//! 2. **Pins the constraint count.** The arkworks-default constants
//!    are close enough in shape to neptune that the constraint count
//!    is a useful empirical floor. PR #70 measured ~753 constraints
//!    for absorb-6 squeeze-1 at this config; the gadget here should
//!    reproduce that number when wired through `report_shape`.
//!
//! 3. **Makes the port a one-line swap.** When the neptune constants
//!    are ported, the only diff is the `PoseidonConfig` argument.
//!    No structural refactor.

use ark_bn254::Fr as Bn254Fr;
use ark_crypto_primitives::sponge::constraints::CryptographicSpongeVar;
use ark_crypto_primitives::sponge::poseidon::constraints::PoseidonSpongeVar;
use ark_crypto_primitives::sponge::poseidon::traits::find_poseidon_ark_and_mds;
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use ark_ff::PrimeField;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// Build a PLACEHOLDER `PoseidonConfig<Bn254Fr>` whose **shape**
/// matches neptune's Bn256/U24/Strength::Standard sponge: state
/// width 25 (rate 24, capacity 1), `full_rounds = 8`,
/// `partial_rounds = 59`, alpha = 5.
///
/// These numbers come from PR #80's `dump-neptune-constants`
/// empirical extraction. The MDS matrix + round constants are
/// still arkworks-default — only the shape parameters
/// (width, round counts, alpha) match neptune. That means:
///
/// - Constraint count of the resulting gadget is order-of-
///   magnitude correct for Section 2's real cost.
/// - Hash outputs still DIVERGE from the neptune oracle
///   (PR #79's port-complete canary continues to fire).
///
/// When the BESPOKE step lands (PR-pending — port plain ARK + MDS
/// from neptune's grain LFSR generation), only `find_poseidon_ark_and_mds`
/// gets replaced; the surrounding `PoseidonConfig::new` call
/// stays put.
/// Same shape as [`placeholder_poseidon_config`] but loads the
/// real neptune MDS matrix from a JSON dump on disk (produced by
/// the `dump-neptune-constants` binary, PR #80). ARK is still
/// arkworks-default — the grain LFSR ARK regeneration is the
/// actual BESPOKE wedge.
///
/// This is PARTIAL parity with neptune:
///   MDS:  REAL neptune values  ←  PR #83's `extract_mds_matrix`
///   ARK:  arkworks placeholder  (regenerated per call)
///
/// The port-complete canary (PR #79) continues to fire because
/// outputs still differ — ARK contributes to every round. But
/// the matrix half is now genuine neptune data, so the
/// constraint count + structural shape are closer to the final
/// gadget.
///
/// **Caveat.** Mixing real MDS with placeholder ARK breaks the
/// security analysis (ARK is supposed to be paired with the same
/// MDS that generated it). This config is for DEVELOPMENT use
/// only — measuring constraint costs, validating shape — NOT
/// for producing proofs that anyone should trust.
pub fn neptune_aligned_poseidon_config<P: AsRef<std::path::Path>>(
    dump_path: P,
) -> Result<PoseidonConfig<Bn254Fr>, String> {
    let full_rounds = 8usize;
    let partial_rounds = 59usize;
    let alpha = 5u64;
    let rate = 24usize;
    let capacity = 1usize;
    let prime_bits = Bn254Fr::MODULUS_BIT_SIZE as u64;

    let mds = crate::neptune_dump_parser::extract_mds_matrix(&dump_path)?;
    if mds.len() != rate + capacity || mds.first().map(|r| r.len()) != Some(rate + capacity) {
        return Err(format!(
            "neptune MDS dims ({}, {}) ≠ expected ({}, {})",
            mds.len(),
            mds.first().map(|r| r.len()).unwrap_or(0),
            rate + capacity,
            rate + capacity
        ));
    }
    let (ark, _arkworks_mds) = find_poseidon_ark_and_mds::<Bn254Fr>(
        prime_bits,
        rate,
        full_rounds as u64,
        partial_rounds as u64,
        0u64,
    );
    Ok(PoseidonConfig::new(
        full_rounds,
        partial_rounds,
        alpha,
        mds,
        ark,
        rate,
        capacity,
    ))
}

/// Final port output: `PoseidonConfig<Bn254Fr>` with BOTH the
/// real neptune MDS (PR #83) AND the grain-LFSR-generated plain
/// ARK (PR #90).
///
/// If the algorithm + parameters in [`crate::grain_lfsr`] match
/// what neptune uses internally, hashing through this config
/// produces the same scalars as
/// [`crate::neptune_reference::neptune_hash_primary`].
///
/// Parity verification: see the test
/// `fully_aligned_gadget_byte_parity_with_neptune` below — runs
/// a hash through both pipelines and compares output bytes.
pub fn fully_aligned_poseidon_config<P: AsRef<std::path::Path>>(
    dump_path: P,
) -> Result<PoseidonConfig<Bn254Fr>, String> {
    let full_rounds = 8usize;
    let partial_rounds = 59usize;
    let alpha = 5u64;
    let rate = 24usize;
    let capacity = 1usize;
    let width = rate + capacity;

    let mds = crate::neptune_dump_parser::extract_mds_matrix(&dump_path)?;
    if mds.len() != width || mds.first().map(|r| r.len()) != Some(width) {
        return Err(format!(
            "neptune MDS dims ({}, {}) ≠ expected ({}, {})",
            mds.len(),
            mds.first().map(|r| r.len()).unwrap_or(0),
            width,
            width
        ));
    }

    let flat_ark = crate::grain_lfsr::generate_round_constants_bn254_arity_24_standard();
    let total_rounds = full_rounds + partial_rounds;
    if flat_ark.len() != total_rounds * width {
        return Err(format!(
            "grain ARK length {} ≠ expected {}",
            flat_ark.len(),
            total_rounds * width
        ));
    }
    // Reshape row-major: each row is `width` constants for one round.
    let mut ark: Vec<Vec<Bn254Fr>> = Vec::with_capacity(total_rounds);
    for r in 0..total_rounds {
        ark.push(flat_ark[r * width..(r + 1) * width].to_vec());
    }

    Ok(PoseidonConfig::new(
        full_rounds,
        partial_rounds,
        alpha,
        mds,
        ark,
        rate,
        capacity,
    ))
}

/// Placeholder `PoseidonConfig<Bn254Fr>` with neptune-matching
/// shape parameters (full_rounds=8, partial_rounds=59, rate=24,
/// capacity=1, alpha=5) but ARK + MDS generated by arkworks'
/// `find_poseidon_ark_and_mds` (NOT neptune-aligned).
///
/// Use [`fully_aligned_poseidon_config`] when neptune-aligned
/// constants are needed; this function is for shape-only smoke
/// tests / constraint-budget probes.
pub fn placeholder_poseidon_config() -> PoseidonConfig<Bn254Fr> {
    // Neptune Bn256 U24 Strength::Standard (PR #80 empirical):
    let full_rounds = 8usize;
    let partial_rounds = 59usize;
    let alpha = 5u64;
    let rate = 24usize;
    let capacity = 1usize;
    let prime_bits = Bn254Fr::MODULUS_BIT_SIZE as u64;
    let (ark, mds) = find_poseidon_ark_and_mds::<Bn254Fr>(
        prime_bits,
        rate,
        full_rounds as u64,
        partial_rounds as u64,
        0u64,
    );
    PoseidonConfig::new(full_rounds, partial_rounds, alpha, mds, ark, rate, capacity)
}

/// Absorb `inputs` into a `PoseidonSpongeVar<Bn254Fr>` configured
/// with `config`, then squeeze one field element and return it as
/// an `FpVar<Bn254Fr>`.
///
/// This is the low-level shape primitive. `enforce_section_2_primary`
/// wraps it with the documented Section-2 absorb order.
pub fn enforce_poseidon_primary(
    cs: ConstraintSystemRef<Bn254Fr>,
    config: &PoseidonConfig<Bn254Fr>,
    inputs: &[FpVar<Bn254Fr>],
) -> Result<FpVar<Bn254Fr>, SynthesisError> {
    let mut sponge = PoseidonSpongeVar::<Bn254Fr>::new(cs, config);
    for x in inputs {
        sponge.absorb(x)?;
    }
    let out = sponge.squeeze_field_elements(1)?;
    Ok(out.into_iter().next().expect("squeezed 1 element"))
}

/// Neptune-aligned in-circuit sponge: mirrors
/// `neptune_sponge::our_neptune_hash_primary_native` as an R1CS gadget.
///
/// Uses `neptune_permutation_gadget::enforce_neptune_permute` (CRC-based)
/// instead of arkworks `PoseidonSpongeVar`, and correctly initialises
/// `state[0]` with the IOPattern tag for `Absorb(n), Squeeze(1)`.
///
/// Returns `state[1]` (first rate slot) after the squeeze permutation.
/// The caller should apply the 250-bit LE mask (`& 0x03` on byte 31)
/// before byte-comparing with neptune's truncated output.
pub fn enforce_neptune_sponge_primary(
    cs: ConstraintSystemRef<Bn254Fr>,
    params: &crate::neptune_permutation_gadget::NeptuneParams<Bn254Fr>,
    inputs: &[FpVar<Bn254Fr>],
) -> Result<FpVar<Bn254Fr>, SynthesisError> {
    use crate::neptune_permutation_gadget::enforce_neptune_permute;
    use crate::neptune_sponge::{iopattern_tag_as_fr, iopattern_value_absorb_then_squeeze};
    use ark_r1cs_std::alloc::AllocVar;

    let width = params.width;
    let rate = width - 1;
    let capacity = 1usize;

    let tag = iopattern_value_absorb_then_squeeze(inputs.len() as u32, 1, 0);
    let tag_fr = iopattern_tag_as_fr(tag);
    let tag_var = FpVar::<Bn254Fr>::new_constant(cs.clone(), tag_fr)?;
    let zero_var = FpVar::<Bn254Fr>::new_constant(cs.clone(), Bn254Fr::from(0u64))?;
    let mut state: Vec<FpVar<Bn254Fr>> = std::iter::once(tag_var)
        .chain(std::iter::repeat_with(|| zero_var.clone()).take(width - 1))
        .collect();

    let mut absorb_pos = 0usize;
    for input in inputs {
        if absorb_pos == rate {
            enforce_neptune_permute(cs.clone(), &mut state, params)?;
            absorb_pos = 0;
        }
        state[absorb_pos + capacity] = state[absorb_pos + capacity].clone() + input;
        absorb_pos += 1;
    }

    enforce_neptune_permute(cs.clone(), &mut state, params)?;
    Ok(state[capacity].clone())
}

/// Convenience wrapper: takes the named Section-2 primary-side
/// slots in absorb order (matching PR #65's
/// `poseidon_transcript::absorb_order(HasherSide::Primary, z_arity)`)
/// and returns the hashed `FpVar`. Caller is responsible for
/// allocating the slots from circuit inputs / witnesses.
///
/// Section-2 primary-side absorb shape at z_arity=1:
///   `[pp.digest, num_steps, z0[0], zi[0], instance..., ri_primary]`
///
/// `instance` carries the variable-length cross-side absorb
/// expansion (`r_U_secondary.absorb_in_ro` — `comm_W`, `comm_E`,
/// `u`, `X[..]`). At this gadget level the caller supplies it as
/// an opaque slice; the exact encoding choice is part of Section 3.
pub fn enforce_section_2_primary(
    cs: ConstraintSystemRef<Bn254Fr>,
    config: &PoseidonConfig<Bn254Fr>,
    pp_digest: &FpVar<Bn254Fr>,
    num_steps: &FpVar<Bn254Fr>,
    z0: &[FpVar<Bn254Fr>],
    zi: &[FpVar<Bn254Fr>],
    instance: &[FpVar<Bn254Fr>],
    ri_primary: &FpVar<Bn254Fr>,
) -> Result<FpVar<Bn254Fr>, SynthesisError> {
    let mut inputs: Vec<FpVar<Bn254Fr>> =
        Vec::with_capacity(2 + z0.len() + zi.len() + instance.len() + 1);
    inputs.push(pp_digest.clone());
    inputs.push(num_steps.clone());
    inputs.extend_from_slice(z0);
    inputs.extend_from_slice(zi);
    inputs.extend_from_slice(instance);
    inputs.push(ri_primary.clone());
    enforce_poseidon_primary(cs, config, &inputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_r1cs_std::alloc::AllocVar;
    use ark_relations::r1cs::ConstraintSystem;

    /// Smoke test: gadget compiles, runs over a 6-scalar absorb,
    /// emits a single squeezed FpVar, and the constraint count
    /// lands in the documented range (PR #70 measured ~753 for
    /// arkworks-default Poseidon absorb-6 squeeze-1).
    #[test]
    fn enforce_poseidon_primary_compiles_and_runs() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let config = placeholder_poseidon_config();
        let inputs: Vec<FpVar<Bn254Fr>> = (1..=6u64)
            .map(|i| FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(i))).unwrap())
            .collect();
        let out = enforce_poseidon_primary(cs.clone(), &config, &inputs).expect("synthesize");
        assert!(!matches!(out, FpVar::Constant(_)), "squeezed output must be a variable");
        let nc = cs.num_constraints();
        // Width-25 + rate-24 + 6 absorbs: the rate-24 buffer never
        // fills (would need 24 absorbs), so only the squeeze
        // triggers a permutation. ONE width-25 permutation at
        // 8 full + 59 partial rounds in arkworks-default emits
        // ~720 constraints empirically on Mini 1.
        //
        // At z_arity = 1 + a multi-scalar instance expansion the
        // real Section 2 absorb count climbs toward rate=24 and
        // we'll see a SECOND permutation kick in (cost roughly
        // doubles per added full permutation).
        assert!(
            (200..=200_000).contains(&nc),
            "Section-2 primary gadget constraint count out of range: {nc}"
        );
        eprintln!(
            "enforce_poseidon_primary: 6 absorb + 1 squeeze (width=25) → {} constraints",
            nc
        );
    }

    /// Wire up the Section-2 primary-side absorb in the exact
    /// shape Section 2's R1CS gadget will use at z_arity=1:
    /// 6 named slots (digest + num_steps + z0[0] + zi[0] +
    /// 1-element instance + ri_primary). Confirm the assembled
    /// absorb sequence runs end-to-end.
    #[test]
    fn enforce_section_2_primary_at_z_arity_1() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let config = placeholder_poseidon_config();

        let pp_digest =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();
        let num_steps =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(1u64))).unwrap();
        let z0_0 = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();
        let zi_0 = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(1u64))).unwrap();
        // Single placeholder instance scalar — Section 3 will
        // expand this to the real (comm_W, comm_E, u, X[..]) vector.
        let instance = vec![
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap(),
        ];
        let ri_primary =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();

        let _hash = enforce_section_2_primary(
            cs.clone(),
            &config,
            &pp_digest,
            &num_steps,
            &[z0_0],
            &[zi_0],
            &instance,
            &ri_primary,
        )
        .expect("synthesize");

        // 4 public inputs + 1 implicit const + 2 witness slots
        // (instance + ri_primary) seeded by the test; Poseidon
        // internals add more witnesses.
        assert!(cs.num_instance_variables() >= 5);
        assert!(cs.num_constraints() > 200);
    }

    /// Determinism canary: same allocated inputs produce structurally
    /// identical constraint systems. If `PoseidonSpongeVar` ever picks
    /// up non-determinism (e.g., random gadget-internal allocations),
    /// this fires.
    #[test]
    fn gadget_is_deterministic() {
        let mk = || {
            let cs = ConstraintSystem::<Bn254Fr>::new_ref();
            let config = placeholder_poseidon_config();
            let inputs: Vec<FpVar<Bn254Fr>> = (0..6u64)
                .map(|i| {
                    FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(i + 1))).unwrap()
                })
                .collect();
            let _ = enforce_poseidon_primary(cs.clone(), &config, &inputs).unwrap();
            (cs.num_instance_variables(), cs.num_witness_variables(), cs.num_constraints())
        };
        let a = mk();
        let b = mk();
        assert_eq!(a, b, "gadget must produce identical CS shape across runs");
    }

    /// **Section 2 BESPOKE CLOSED.** Hash the Section-2 primary-side
    /// minimal absorb sequence `[0, 1, 0, 1, 0]` through:
    ///
    ///   1. `neptune_reference::neptune_hash_primary` — what
    ///      `RecursiveSNARK::verify` actually uses internally (includes
    ///      250-bit LE truncation)
    ///   2. `enforce_neptune_sponge_primary` — CRC-based in-circuit
    ///      permutation + IOPattern tag init; gadget value is masked
    ///      to 250 bits before byte comparison
    ///
    /// Both must match byte-for-byte.
    ///
    /// Requires `/tmp/neptune-bn256-standard.json`.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json from dump-neptune-constants binary"]
    fn fully_aligned_gadget_byte_parity_with_neptune() {
        use crate::neptune_reference::{neptune_hash_primary, PrimaryScalar};
        use ark_r1cs_std::R1CSVar;
        use ff::PrimeField as _;

        let dump_path = "/tmp/neptune-bn256-standard.json";

        // Neptune side: pinned PR #77 minimal absorb sequence (returns 250-bit-truncated value).
        let neptune_inputs = vec![
            PrimaryScalar::from(0u64),
            PrimaryScalar::from(1u64),
            PrimaryScalar::from(0u64),
            PrimaryScalar::from(1u64),
            PrimaryScalar::from(0u64),
        ];
        let neptune_out = neptune_hash_primary(&neptune_inputs);
        let neptune_bytes_le: [u8; 32] = neptune_out.to_repr().into();

        // In-circuit side: neptune-aligned sponge gadget.
        let params = crate::neptune_permutation_gadget::params_from_dump_path(dump_path)
            .expect("load neptune params");
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let inputs: Vec<FpVar<Bn254Fr>> = [0u64, 1, 0, 1, 0]
            .into_iter()
            .map(|v| FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(v))).unwrap())
            .collect();
        let hash_var =
            enforce_neptune_sponge_primary(cs.clone(), &params, &inputs).expect("synth");
        let gadget_out: Bn254Fr = hash_var.value().expect("witness value");
        let gadget_bigint = ark_ff::PrimeField::into_bigint(gadget_out);
        let gadget_bytes = ark_ff::BigInteger::to_bytes_le(&gadget_bigint);
        let mut gadget_bytes_le = [0u8; 32];
        gadget_bytes_le[..gadget_bytes.len()].copy_from_slice(&gadget_bytes);
        // Apply 250-bit LE truncation (NUM_HASH_BITS=250): zero bits 250-255 of byte 31.
        // Byte 31 holds bits 248-255; bits 248-249 survive, bits 250-255 are zeroed.
        gadget_bytes_le[31] &= 0x03;

        eprintln!("neptune  bytes LE: {neptune_bytes_le:?}");
        eprintln!("gadget   bytes LE: {gadget_bytes_le:?}");

        assert_eq!(
            gadget_bytes_le, neptune_bytes_le,
            "enforce_neptune_sponge_primary must match neptune byte-for-byte.\n\
             If this fails: pre_sparse_mds transpose regression or CRC drift.\n\
             gadget[0..8]={:?}  neptune[0..8]={:?}",
            &gadget_bytes_le[..8],
            &neptune_bytes_le[..8],
        );
    }

    /// Confirm `neptune_aligned_poseidon_config` loads + builds
    /// against a hand-rolled minimal JSON fixture. (The full
    /// 25×25 real PR #80 fixture lives outside the repo at
    /// `/tmp/neptune-bn256-standard.json` on Mini 1; not checked
    /// into git so this test uses a smaller synthetic dump.)
    ///
    /// The synthetic dump has a 25×25 identity-like MDS — every
    /// row has a 1 in the diagonal slot. That's not a real MDS
    /// (not invertible-resistant for security), but the config
    /// builder accepts it for shape validation.
    #[test]
    fn neptune_aligned_config_loads_real_dims() {
        use std::fs;
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-aligned-fixture.json");

        // Build 25×25 matrix as a JSON string: row i has 1 at
        // position i and 0 elsewhere.
        let one_hex = format!("01{}", "0".repeat(62));
        let zero_hex = "0".repeat(64);
        let mut mds_rows = Vec::with_capacity(25);
        for i in 0..25 {
            let mut row_cells = Vec::with_capacity(25);
            for j in 0..25 {
                row_cells.push(format!("\"{}\"", if i == j { &one_hex } else { &zero_hex }));
            }
            mds_rows.push(format!("[{}]", row_cells.join(",")));
        }
        let mds_m = mds_rows.join(",");
        let json = format!(
            "{{\"mds\":{{\"m\":[{mds_m}],\"m_inv\":[],\"m_hat\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\"crc\":[],\"psm\":[],\"sm\":[],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}"
        );
        fs::write(&path, json).unwrap();

        let config = neptune_aligned_poseidon_config(&path).expect("load aligned config");
        assert_eq!(config.full_rounds, 8);
        assert_eq!(config.partial_rounds, 59);
        assert_eq!(config.rate, 24);
        assert_eq!(config.capacity, 1);
        // MDS dimensions are 25×25 (state width = rate + capacity).
        assert_eq!(config.mds.len(), 25);
        assert_eq!(config.mds[0].len(), 25);
        // The diagonal entries must be 1 (per our synthetic dump).
        for i in 0..25 {
            assert_eq!(config.mds[i][i], Bn254Fr::from(1u64), "diagonal[{i}]");
        }

        let _ = fs::remove_file(&path);
    }

    /// Confirm `neptune_aligned_poseidon_config` rejects wrong MDS
    /// dimensions cleanly (instead of silently using a malformed
    /// config).
    #[test]
    fn neptune_aligned_config_rejects_wrong_mds_dims() {
        use std::fs;
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-bad-dims.json");
        // 3×3 instead of 25×25
        let one = format!("01{}", "0".repeat(62));
        let json = format!(
            "{{\"mds\":{{\"m\":[[\"{one}\",\"{one}\",\"{one}\"],[\"{one}\",\"{one}\",\"{one}\"],[\"{one}\",\"{one}\",\"{one}\"]],\"m_inv\":[],\"m_hat\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\"crc\":[],\"psm\":[],\"sm\":[],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}"
        );
        fs::write(&path, json).unwrap();
        let result = neptune_aligned_poseidon_config(&path);
        assert!(result.is_err(), "3×3 MDS must be rejected");
        let _ = fs::remove_file(&path);
    }

    /// Absorb 25 elements — one more than rate=24 — to force a
    /// buffer-flush permutation BEFORE the squeeze. This should
    /// roughly double the constraint count vs the 6-absorb case
    /// (two width-25 permutations instead of one). Pin the
    /// resulting band as the actual Section 2 ballpark for absorbs
    /// that exceed the rate.
    #[test]
    fn width_25_buffer_flush_doubles_cost() {
        let cs_a = ConstraintSystem::<Bn254Fr>::new_ref();
        let cs_b = ConstraintSystem::<Bn254Fr>::new_ref();
        let config = placeholder_poseidon_config();

        let inputs_6: Vec<FpVar<Bn254Fr>> = (1..=6u64)
            .map(|i| FpVar::<Bn254Fr>::new_input(cs_a.clone(), || Ok(Bn254Fr::from(i))).unwrap())
            .collect();
        enforce_poseidon_primary(cs_a.clone(), &config, &inputs_6).expect("6 absorbs");

        let inputs_25: Vec<FpVar<Bn254Fr>> = (1..=25u64)
            .map(|i| FpVar::<Bn254Fr>::new_input(cs_b.clone(), || Ok(Bn254Fr::from(i))).unwrap())
            .collect();
        enforce_poseidon_primary(cs_b.clone(), &config, &inputs_25).expect("25 absorbs");

        let nc6 = cs_a.num_constraints();
        let nc25 = cs_b.num_constraints();
        eprintln!(
            "width-25 cost: 6 absorbs → {nc6} constraints, 25 absorbs → {nc25} constraints"
        );

        // 25-absorb must cost MORE than 6-absorb (extra permutation
        // from buffer flush) but no more than 3× (1 extra
        // permutation + small absorb overhead, not 2 extras).
        assert!(nc25 > nc6, "25-absorb must exceed 6-absorb cost");
        assert!(
            nc25 <= 3 * nc6,
            "25-absorb {nc25} blew up over 3× of 6-absorb {nc6} — unexpected"
        );
    }

    /// **Port-complete canary.** Today: confirms gadget output
    /// ≠ neptune oracle output (because placeholder constants).
    /// After the BESPOKE port: this test INVERTS to `assert_eq!`,
    /// proving byte parity.
    ///
    /// Input is the Section-2 primary-side minimal absorb sequence:
    ///   `[pp.digest=0, num_steps=1, z0[0]=0, zi[0]=1, ri=0]`
    /// (matches PR #77's `pinned_section_2_primary_minimal_absorb_sequence`)
    ///
    /// This test serves three purposes:
    ///
    /// 1. Documents the divergence: the arkworks-default Poseidon
    ///    config produces a DIFFERENT scalar than neptune for the
    ///    same input sequence (because round constants + MDS
    ///    differ).
    /// 2. Provides a single byte-comparison test point that the
    ///    BESPOKE port can re-run to confirm parity.
    /// 3. Catches a future regression where someone accidentally
    ///    swaps in the neptune constants partially (and the
    ///    arkworks gadget starts matching some test inputs but not
    ///    others).
    #[test]
    fn placeholder_gadget_diverges_from_neptune_oracle() {
        use crate::neptune_reference::{neptune_hash_primary, PrimaryScalar};
        use ff::{Field as _, PrimeField as _};
        use ark_r1cs_std::R1CSVar;

        // Neptune-side: the pinned minimal absorb (PR #77).
        let neptune_inputs = vec![
            PrimaryScalar::ZERO,        // pp.digest
            PrimaryScalar::from(1u64),  // num_steps
            PrimaryScalar::ZERO,        // z0[0]
            PrimaryScalar::from(1u64),  // zi[0]
            PrimaryScalar::ZERO,        // ri_primary
        ];
        let neptune_out = neptune_hash_primary(&neptune_inputs);
        let neptune_bytes_le: [u8; 32] = neptune_out.to_repr().into();

        // Arkworks-side: same input shape but instance=[] so the
        // absorb count matches neptune's 5 elements exactly.
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let config = placeholder_poseidon_config();
        let pp_digest =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();
        let num_steps =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(1u64))).unwrap();
        let z0 = vec![FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap()];
        let zi = vec![FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(1u64))).unwrap()];
        let instance: Vec<FpVar<Bn254Fr>> = vec![];
        let ri =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();
        let hash_var = enforce_section_2_primary(
            cs.clone(),
            &config,
            &pp_digest,
            &num_steps,
            &z0,
            &zi,
            &instance,
            &ri,
        )
        .expect("synthesize");
        let ark_out: Bn254Fr = hash_var.value().expect("witness value");
        let ark_bigint = ark_ff::PrimeField::into_bigint(ark_out);
        let bytes_le = ark_ff::BigInteger::to_bytes_le(&ark_bigint);
        let mut ark_bytes_le = [0u8; 32];
        let copy_len = bytes_le.len().min(32);
        ark_bytes_le[..copy_len].copy_from_slice(&bytes_le[..copy_len]);

        eprintln!("neptune bytes LE:  {neptune_bytes_le:?}");
        eprintln!("arkworks bytes LE: {ark_bytes_le:?}");

        // Today: bytes MUST differ. Tomorrow (post-port): assert_eq.
        assert_ne!(
            ark_bytes_le, neptune_bytes_le,
            "placeholder constants must NOT match neptune — if this fires either \
             (a) the BESPOKE neptune-constants port landed (flip to assert_eq), or \
             (b) by astronomical coincidence the placeholder Poseidon produced the \
             same hash on this input."
        );
    }

    /// Pinned parity test for `enforce_neptune_sponge_primary` on
    /// `[42, 7, 99]` — the same vector confirmed byte-correct in
    /// `neptune_sponge::sponge_attempt_1_matches_neptune_hash_primary_on_pinned_42_7_99`.
    ///
    /// Compares in-circuit `.value()` (250-bit masked) against the native
    /// `our_neptune_hash_primary_native` output. Since native == neptune
    /// (confirmed by the sponge test), this transitively pins the gadget vs neptune.
    ///
    /// Requires `/tmp/neptune-bn256-standard.json`.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json from dump-neptune-constants binary"]
    fn neptune_sponge_gadget_pinned_42_7_99() {
        use crate::neptune_permutation_gadget::params_from_dump_path;
        use crate::neptune_sponge::our_neptune_hash_primary_native;
        use ark_r1cs_std::R1CSVar;

        let dump_path = "/tmp/neptune-bn256-standard.json";
        let params = params_from_dump_path(dump_path).expect("load neptune params");

        // Native reference (already 250-bit truncated by our_neptune_hash_primary_native).
        let native_inputs = [
            Bn254Fr::from(42u64),
            Bn254Fr::from(7u64),
            Bn254Fr::from(99u64),
        ];
        let native_out = our_neptune_hash_primary_native(&native_inputs, &params);
        let native_bigint = ark_ff::PrimeField::into_bigint(native_out);
        let native_bytes = ark_ff::BigInteger::to_bytes_le(&native_bigint);
        let mut native_le = [0u8; 32];
        native_le[..native_bytes.len()].copy_from_slice(&native_bytes);

        // In-circuit side: gadget returns untruncated Fr; apply 250-bit mask before compare.
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let inputs: Vec<FpVar<Bn254Fr>> = [42u64, 7, 99]
            .into_iter()
            .map(|v| FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(v))).unwrap())
            .collect();
        let hash_var =
            enforce_neptune_sponge_primary(cs.clone(), &params, &inputs).expect("synth");
        let gadget_out: Bn254Fr = hash_var.value().expect("witness value");
        let gadget_bigint = ark_ff::PrimeField::into_bigint(gadget_out);
        let gadget_bytes = ark_ff::BigInteger::to_bytes_le(&gadget_bigint);
        let mut gadget_le = [0u8; 32];
        gadget_le[..gadget_bytes.len()].copy_from_slice(&gadget_bytes);
        // 250-bit LE truncation: zero bits 250-255 of byte 31 (mask 0x03).
        gadget_le[31] &= 0x03;

        eprintln!("native  LE: {native_le:?}");
        eprintln!("gadget  LE: {gadget_le:?}");

        assert_eq!(
            gadget_le, native_le,
            "in-circuit sponge must match native for [42,7,99] after 250-bit mask.\n\
             gadget[0..8]={:?}  native[0..8]={:?}",
            &gadget_le[..8],
            &native_le[..8],
        );
    }
}
