use pasta_curves::group::ff::{Field, PrimeField};
use pasta_curves::Fp;
use std::sync::OnceLock;

// ─────────────────────────── HashEngine Trait ────────────────────────────

/// Trait for swappable hash backends.
///
/// Both BLAKE3 (for general state hashing) and Poseidon (for ZK circuits)
/// implement this trait so callers can switch without changing code.
pub trait HashEngine: Send + Sync {
    /// Hash arbitrary-length input to a 32-byte digest.
    fn hash(&self, data: &[u8]) -> [u8; 32];

    /// Hash two 32-byte values together (used for Merkle trees).
    fn hash_two(&self, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut combined = [0u8; 64];
        combined[..32].copy_from_slice(left);
        combined[32..].copy_from_slice(right);
        self.hash(&combined)
    }
}

// ─────────────────────────── BLAKE3 ──────────────────────────────────────

/// BLAKE3 hash engine for general-purpose state root computation.
pub struct Blake3Hasher;

impl HashEngine for Blake3Hasher {
    fn hash(&self, data: &[u8]) -> [u8; 32] {
        blake3_hash(data)
    }
}

/// Compute a BLAKE3 hash of the input data.
pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

// ─────────────────────────── Poseidon ────────────────────────────────────

/// Poseidon hash engine for ZK-circuit-compatible hashing.
///
/// Implements the Poseidon permutation over the Pallas base field (Fp)
/// with parameters: width t=3, rate=2, capacity=1, S-box x^5,
/// R_F=8 full rounds, R_P=56 partial rounds.
///
/// When integrating with Nova proofs over BN256, the field and round
/// constants should be updated to match the proving curve's scalar field.
pub struct PoseidonHasher;

impl HashEngine for PoseidonHasher {
    fn hash(&self, data: &[u8]) -> [u8; 32] {
        poseidon_hash(data)
    }
}

// Poseidon parameters
//
// Security rationale for custom constants vs audited Arkworks:
//
// We use DOMAIN-SEPARATED round constants derived deterministically from
// BLAKE3("EvaporChain_Poseidon_RC_{round}_{element}"). This differs from the
// Arkworks `poseidon` crate which uses the Grain LFSR method from the original
// Poseidon paper (Grassi et al., 2019).
//
// Why custom:
// - Domain separation binds constants to EvaporChain, preventing cross-protocol
//   attacks where an adversary reuses precomputed tables from another deployment.
// - BLAKE3 is a standardized, collision-resistant PRF — the security requirement
//   for round constants is pseudorandomness, which BLAKE3 satisfies.
// - The MDS matrix uses the Cauchy construction (M[i][j] = 1/(x_i + y_j) with
//   disjoint x,y sets), which is the SAME construction used in the Poseidon
//   paper and Arkworks. This guarantees maximal branch number.
//
// Parameter choices (t=3, R_F=8, R_P=56) match the Poseidon authors'
// recommendations for 128-bit security over BLS12-381's scalar field (Table 2,
// https://eprint.iacr.org/2019/458.pdf).
//
// Trade-off: These constants have NOT been audited by a third party. The
// external security audit (see REMAINING_WORK.md) should verify that:
// 1. Round constant generation produces uniform field elements.
// 2. No invariant subspace attacks exist for this specific instantiation.
// 3. The number of partial rounds (56) provides sufficient security margin.
const WIDTH: usize = 3; // state width (t)
const RATE: usize = 2; // absorption rate
const ROUNDS_F: usize = 8; // full rounds
const ROUNDS_P: usize = 56; // partial rounds
const TOTAL_ROUNDS: usize = ROUNDS_F + ROUNDS_P;

struct PoseidonParams {
    round_constants: Vec<[Fp; WIDTH]>,
    mds: [[Fp; WIDTH]; WIDTH],
}

static PARAMS: OnceLock<PoseidonParams> = OnceLock::new();

fn get_params() -> &'static PoseidonParams {
    PARAMS.get_or_init(PoseidonParams::generate)
}

impl PoseidonParams {
    fn generate() -> Self {
        // Generate round constants deterministically from BLAKE3
        let mut round_constants = Vec::with_capacity(TOTAL_ROUNDS);
        for r in 0..TOTAL_ROUNDS {
            let mut rc = [Fp::ZERO; WIDTH];
            for (j, elem) in rc.iter_mut().enumerate() {
                let seed = format!("EvaporChain_Poseidon_RC_{}_{}", r, j);
                let hash = blake3::hash(seed.as_bytes());
                *elem = bytes_to_field(hash.as_bytes());
            }
            round_constants.push(rc);
        }

        // Cauchy MDS matrix: M[i][j] = 1 / (x_i + y_j)
        // where x = {0,1,2}, y = {WIDTH, WIDTH+1, WIDTH+2}
        let mut mds = [[Fp::ZERO; WIDTH]; WIDTH];
        for (i, row) in mds.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                let x = Fp::from_u128(i as u128);
                let y = Fp::from_u128((WIDTH + j) as u128);
                // SAFETY: x ∈ {0,1,2}, y ∈ {3,4,5}, so x+y ∈ {3..7} — never zero in Fp
                *cell = (x + y).invert().unwrap();
            }
        }

        Self {
            round_constants,
            mds,
        }
    }
}

/// Convert 32 bytes to a Pallas base field element.
/// Clears top 2 bits to ensure the value is < p (p ≈ 2^254).
fn bytes_to_field(bytes: &[u8; 32]) -> Fp {
    let mut repr = *bytes;
    // Pallas base field modulus p starts with 0x40... in big-endian.
    // In little-endian repr, byte 31 is the MSB. Clear bits 6,7 so value < p.
    repr[31] &= 0x3F; // Clears bits 6-7 → value < 2^254 < p ≈ 2^254.97
    Fp::from_repr(repr).unwrap()
}

/// Convert a field element back to 32 bytes (little-endian canonical repr).
fn field_to_bytes(elem: &Fp) -> [u8; 32] {
    elem.to_repr()
}

/// Apply the Poseidon permutation to a state of WIDTH field elements.
fn permutation(state: &mut [Fp; WIDTH]) {
    let params = get_params();

    // First half of full rounds
    for r in 0..ROUNDS_F / 2 {
        add_round_constants(state, &params.round_constants[r]);
        full_sbox(state);
        mds_multiply(state, &params.mds);
    }

    // Partial rounds
    for r in (ROUNDS_F / 2)..(ROUNDS_F / 2 + ROUNDS_P) {
        add_round_constants(state, &params.round_constants[r]);
        partial_sbox(state);
        mds_multiply(state, &params.mds);
    }

    // Second half of full rounds
    for r in (ROUNDS_F / 2 + ROUNDS_P)..TOTAL_ROUNDS {
        add_round_constants(state, &params.round_constants[r]);
        full_sbox(state);
        mds_multiply(state, &params.mds);
    }
}

#[inline]
fn add_round_constants(state: &mut [Fp; WIDTH], rc: &[Fp; WIDTH]) {
    for i in 0..WIDTH {
        state[i] += rc[i];
    }
}

/// S-box: x^5 applied to all state elements.
#[inline]
fn full_sbox(state: &mut [Fp; WIDTH]) {
    for s in state.iter_mut() {
        let x2 = s.square();
        let x4 = x2.square();
        *s = x4 * *s;
    }
}

/// Partial S-box: x^5 applied only to state[0].
#[inline]
fn partial_sbox(state: &mut [Fp; WIDTH]) {
    let x2 = state[0].square();
    let x4 = x2.square();
    state[0] = x4 * state[0];
}

/// MDS matrix multiplication.
#[inline]
fn mds_multiply(state: &mut [Fp; WIDTH], mds: &[[Fp; WIDTH]; WIDTH]) {
    let old = *state;
    for (i, row) in mds.iter().enumerate() {
        state[i] = Fp::ZERO;
        for (j, &mds_val) in row.iter().enumerate() {
            state[i] += mds_val * old[j];
        }
    }
}

/// Compute a Poseidon hash of arbitrary-length input.
///
/// Uses sponge construction: absorb input as field elements (31 bytes each),
/// then squeeze one element as the digest.
///
/// **M-4 / M-5 (audit 2026-05-17) — for ZK-circuit field-element
/// compression ONLY. NOT a general-purpose cryptographic hash.**
///
/// This implementation lacks the hardening a general-purpose Poseidon
/// requires:
///
/// 1. **No length prefix.** Two distinct inputs that hash to the same
///    elements vec (e.g. empty input and a 31-byte input that decodes
///    to `Fp::ZERO`) absorb to identical state. Length-vs-content
///    ambiguity, not a collision proper, but real for short inputs.
/// 2. **No capacity-distinguishing IV.** A general sponge uses a
///    non-zero IV in the capacity portion to bind the construction's
///    domain. Here the state starts at all-zero.
/// 3. **No padding-discriminator** ("10*" per Grassi et al. 2019 §4.3).
///    The last absorbed chunk's zero-pad is indistinguishable from a
///    chunk that genuinely ends in zeros.
/// 4. **No DST across uses.** Two crates calling `poseidon_hash(x)`
///    for two different purposes produce the same digest → cross-
///    purpose collision.
///
/// EvaporChain uses Poseidon only inside the Nova IVC circuit's
/// field-element compression where BOTH sides of the equation use the
/// same shape and the four hazards above are irrelevant. For any other
/// caller — chain commitments, signatures, message hashing — use
/// `blake3_hash` with an explicit DST instead. See `accumulator.rs`
/// or `signatures.rs` for the canonical DST pattern.
pub fn poseidon_hash(data: &[u8]) -> [u8; 32] {
    // Split input into 31-byte chunks → field elements (31 bytes to stay < p)
    let mut elements = Vec::new();
    for chunk in data.chunks(31) {
        let mut padded = [0u8; 32];
        padded[..chunk.len()].copy_from_slice(chunk);
        // 31 bytes max → byte 31 is always 0 → value < 2^248 < p, always valid
        elements.push(Fp::from_repr(padded).unwrap());
    }
    if elements.is_empty() {
        elements.push(Fp::ZERO);
    }

    // Sponge: absorb elements in pairs (rate=2), then squeeze
    let mut state = [Fp::ZERO; WIDTH];

    for chunk in elements.chunks(RATE) {
        for (i, elem) in chunk.iter().enumerate() {
            state[i] += *elem;
        }
        permutation(&mut state);
    }

    // Squeeze: output state[0]
    field_to_bytes(&state[0])
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── BLAKE3 tests ──

    #[test]
    fn test_blake3_hash_deterministic() {
        let data = b"evaporchain";
        let h1 = blake3_hash(data);
        let h2 = blake3_hash(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_blake3_hash_different_inputs() {
        let h1 = blake3_hash(b"hello");
        let h2 = blake3_hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_blake3_empty_input() {
        let h = blake3_hash(b"");
        assert_ne!(h, [0u8; 32]);
    }

    #[test]
    fn test_blake3_hasher_trait() {
        let hasher = Blake3Hasher;
        let h1 = hasher.hash(b"test");
        let h2 = blake3_hash(b"test");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_blake3_hash_two() {
        let hasher = Blake3Hasher;
        let left = blake3_hash(b"left");
        let right = blake3_hash(b"right");
        let combined = hasher.hash_two(&left, &right);
        assert_ne!(combined, left);
        assert_ne!(combined, right);

        // Deterministic
        assert_eq!(combined, hasher.hash_two(&left, &right));

        // Order matters
        assert_ne!(combined, hasher.hash_two(&right, &left));
    }

    // ── Poseidon tests ──

    #[test]
    fn test_poseidon_hash_deterministic() {
        let data = b"evaporchain";
        let h1 = poseidon_hash(data);
        let h2 = poseidon_hash(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_poseidon_hash_different_inputs() {
        let h1 = poseidon_hash(b"hello");
        let h2 = poseidon_hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_poseidon_empty_input() {
        let h = poseidon_hash(b"");
        // Empty input should produce a non-zero hash
        assert_ne!(h, [0u8; 32]);
    }

    #[test]
    fn test_poseidon_hasher_trait() {
        let hasher = PoseidonHasher;
        let h1 = hasher.hash(b"test");
        let h2 = poseidon_hash(b"test");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_poseidon_large_input() {
        // Input larger than one field element (>31 bytes)
        let data = vec![0xABu8; 100];
        let h = poseidon_hash(&data);
        assert_ne!(h, [0u8; 32]);

        // Deterministic for large inputs too
        assert_eq!(h, poseidon_hash(&data));
    }

    #[test]
    fn test_poseidon_differs_from_blake3() {
        let data = b"same input";
        let ph = poseidon_hash(data);
        let bh = blake3_hash(data);
        assert_ne!(
            ph, bh,
            "Poseidon and BLAKE3 should produce different outputs"
        );
    }

    // ── HashEngine interchangeability ──

    #[test]
    fn test_hash_engines_swappable() {
        let data = b"state root input";

        fn compute_root(engine: &dyn HashEngine, data: &[u8]) -> [u8; 32] {
            engine.hash(data)
        }

        let b3 = compute_root(&Blake3Hasher, data);
        let pos = compute_root(&PoseidonHasher, data);

        // Both produce 32-byte outputs
        assert_eq!(b3.len(), 32);
        assert_eq!(pos.len(), 32);

        // But with different values (different algorithms)
        assert_ne!(b3, pos);
    }

    // ── Field conversion tests ──

    #[test]
    fn test_bytes_to_field_roundtrip() {
        let mut bytes = [0u8; 32];
        bytes[0] = 42;
        bytes[1] = 0xFF;
        let elem = bytes_to_field(&bytes);
        let out = field_to_bytes(&elem);
        // After clearing top bits, roundtrip should match
        let mut expected = bytes;
        expected[31] &= 0x3F;
        assert_eq!(out, expected);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Property-Based Tests (Audit Hardening)
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// BLAKE3: identical inputs always produce identical outputs.
        #[test]
        fn blake3_deterministic(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let h1 = blake3_hash(&data);
            let h2 = blake3_hash(&data);
            prop_assert_eq!(h1, h2);
        }

        /// BLAKE3: different inputs produce different outputs (collision resistance).
        #[test]
        fn blake3_collision_resistance(
            a in proptest::collection::vec(any::<u8>(), 1..512),
            b in proptest::collection::vec(any::<u8>(), 1..512),
        ) {
            prop_assume!(a != b);
            let ha = blake3_hash(&a);
            let hb = blake3_hash(&b);
            prop_assert_ne!(ha, hb);
        }

        /// Poseidon: identical inputs always produce identical outputs.
        #[test]
        fn poseidon_deterministic(data in proptest::collection::vec(any::<u8>(), 0..256)) {
            let h1 = poseidon_hash(&data);
            let h2 = poseidon_hash(&data);
            prop_assert_eq!(h1, h2);
        }

        /// Poseidon: output is always 32 bytes, never all zeros for non-empty input.
        #[test]
        fn poseidon_output_valid(data in proptest::collection::vec(any::<u8>(), 1..256)) {
            let h = poseidon_hash(&data);
            prop_assert_eq!(h.len(), 32);
            prop_assert_ne!(h, [0u8; 32]);
        }

        /// Field element roundtrip: bytes → field → bytes is stable after first conversion.
        #[test]
        fn field_roundtrip(bytes in any::<[u8; 32]>()) {
            let elem = bytes_to_field(&bytes);
            let out = field_to_bytes(&elem);
            // A second roundtrip must be identical (idempotent).
            let elem2 = bytes_to_field(&out);
            let out2 = field_to_bytes(&elem2);
            prop_assert_eq!(out, out2, "double roundtrip must be stable");
        }

        /// HashEngine trait: both implementations produce 32-byte output.
        #[test]
        fn hash_engines_produce_32_bytes(data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let b3 = Blake3Hasher.hash(&data);
            let pos = PoseidonHasher.hash(&data);
            prop_assert_eq!(b3.len(), 32);
            prop_assert_eq!(pos.len(), 32);
        }

        /// hash_two is not commutative (order matters).
        #[test]
        fn hash_two_order_matters(
            a in any::<[u8; 32]>(),
            b in any::<[u8; 32]>(),
        ) {
            prop_assume!(a != b);
            let h1 = Blake3Hasher.hash_two(&a, &b);
            let h2 = Blake3Hasher.hash_two(&b, &a);
            prop_assert_ne!(h1, h2);
        }
    }
}
