//! Post-Quantum Verifiable Random Function (VRF)
//!
//! Lattice-based VRF built on ML-DSA (CRYSTALS-Dilithium3) using the
//! hash-then-sign construction (analogous to ECVRF RFC 9381 but post-quantum).
//!
//! Construction:
//!   VRF.Eval(sk, alpha) -> (output, proof)
//!     sigma = ML-DSA.Sign(sk, DOMAIN_SEP || alpha)
//!     output = BLAKE3(DOMAIN_SEP || alpha || sigma)
//!     proof = sigma
//!
//!   VRF.Verify(pk, alpha, output, proof) -> bool
//!     Check ML-DSA.Verify(pk, DOMAIN_SEP || alpha, proof)
//!     Check output == BLAKE3(DOMAIN_SEP || alpha || proof)
//!
//! Properties:
//! - **Pseudorandomness**: output is unpredictable without sk
//! - **Verifiability**: anyone with pk can verify (output, proof)
//! - **Collision resistance**: BLAKE3 binds output to (alpha, sigma)
//!
//! The ML-DSA signature uses internal rejection sampling so is not
//! perfectly deterministic across calls. Validators must evaluate once
//! per (height, round) and commit. The proof is always verifiable.

use crate::signatures::{MlDsaKeypair, MlDsaVerifier, Signer, Verifier};
use subtle::ConstantTimeEq;

/// Domain separation tag to prevent cross-protocol signature reuse.
const VRF_DOMAIN_SEP: &[u8] = b"EvaporChain_VRF_v1";

/// VRF output: 32 bytes of verifiable pseudorandomness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VrfOutput(pub [u8; 32]);

/// VRF proof: the ML-DSA signature (3293 bytes for Dilithium3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VrfProof(pub Vec<u8>);

/// VRF keypair wrapping an ML-DSA keypair.
/// Validators use this for leader election and randomness generation.
pub struct VrfKeypair {
    inner: MlDsaKeypair,
}

impl VrfKeypair {
    /// Generate a new random VRF keypair.
    pub fn generate() -> Self {
        Self {
            inner: MlDsaKeypair::generate(),
        }
    }

    /// Construct from an existing ML-DSA keypair.
    pub fn from_ml_dsa(keypair: MlDsaKeypair) -> Self {
        Self { inner: keypair }
    }

    /// Construct from raw key bytes.
    pub fn from_bytes(pk: &[u8], sk: &[u8]) -> Result<Self, crate::signatures::MlDsaError> {
        Ok(Self {
            inner: MlDsaKeypair::from_bytes(pk, sk)?,
        })
    }

    /// Evaluate the VRF on input `alpha`, producing (output, proof).
    ///
    /// The input is typically `height || round` for leader election
    /// or `height || "beacon"` for the randomness beacon.
    pub fn evaluate(&self, alpha: &[u8]) -> (VrfOutput, VrfProof) {
        let msg = vrf_message(alpha);
        let sigma = self.inner.sign(&msg);
        let output = vrf_hash(alpha, &sigma);
        (VrfOutput(output), VrfProof(sigma))
    }

    /// Raw public key bytes (1952 bytes).
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.inner.public_key().to_vec()
    }

    /// Raw secret key bytes.
    pub fn secret_key_bytes(&self) -> Vec<u8> {
        self.inner.secret_key().to_vec()
    }
}

/// Verify a VRF output and proof against a public key and input.
///
/// Returns `true` if:
/// 1. The ML-DSA signature (proof) is valid over `DOMAIN_SEP || alpha`
/// 2. The output matches `BLAKE3(DOMAIN_SEP || alpha || proof)`
pub fn vrf_verify(pk: &[u8], alpha: &[u8], output: &VrfOutput, proof: &VrfProof) -> bool {
    let msg = vrf_message(alpha);
    // Step 1: verify the signature
    if !MlDsaVerifier::verify(&msg, &proof.0, pk) {
        return false;
    }
    // Step 2: check output consistency (constant-time to prevent timing side-channel)
    let expected = vrf_hash(alpha, &proof.0);
    bool::from(expected.ct_eq(&output.0))
}

/// Construct the domain-separated message for signing/verification.
fn vrf_message(alpha: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(VRF_DOMAIN_SEP.len() + alpha.len());
    msg.extend_from_slice(VRF_DOMAIN_SEP);
    msg.extend_from_slice(alpha);
    msg
}

/// Compute VRF output hash: BLAKE3(DOMAIN_SEP || alpha || sigma).
fn vrf_hash(alpha: &[u8], sigma: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(VRF_DOMAIN_SEP);
    hasher.update(alpha);
    hasher.update(sigma);
    *hasher.finalize().as_bytes()
}

// ═══════════════════════════════════════════════════════════════════════════
// Sortition (Algorand-style VRF-based committee selection)
// ═══════════════════════════════════════════════════════════════════════════

/// Determines whether a validator is selected as block proposer.
///
/// The VRF output is interpreted as a uniform random value in [0, 2^64).
/// The validator is selected if `vrf_value < threshold` where
/// `threshold = (stake / total_stake) * u64::MAX`.
///
/// This gives each validator a selection probability proportional to stake.
pub fn vrf_leader_check(vrf_output: &VrfOutput, stake: u64, total_stake: u64) -> bool {
    if total_stake == 0 || stake == 0 {
        return false;
    }
    let vrf_value = u64::from_le_bytes(vrf_output.0[..8].try_into().unwrap());
    let threshold = ((stake as u128 * u64::MAX as u128) / total_stake as u128) as u64;
    vrf_value < threshold
}

/// Algorand-style sortition: determines how many committee seats a
/// validator wins based on their VRF output and stake fraction.
///
/// Uses a simple threshold-based approach:
/// - Interpret first 8 bytes of VRF output as uniform u64
/// - Compute expected seats = (stake / total_stake) * expected_committee_size
/// - Use fractional part as probability for one additional seat
///
/// Returns the number of seats won (0 = not on committee).
pub fn sortition(
    vrf_output: &VrfOutput,
    stake: u64,
    total_stake: u64,
    expected_committee_size: u64,
) -> u64 {
    if total_stake == 0 || stake == 0 {
        return 0;
    }

    // Expected number of seats (fixed-point: multiply first to avoid precision loss).
    let expected_x1000 =
        (stake as u128 * expected_committee_size as u128 * 1000) / total_stake as u128;
    let whole_seats = (expected_x1000 / 1000) as u64;
    let fractional_x1000 = (expected_x1000 % 1000) as u64;

    // Use VRF output to decide the fractional seat.
    let vrf_value = u64::from_le_bytes(vrf_output.0[..8].try_into().unwrap());
    let frac_threshold = (fractional_x1000 as u128 * u64::MAX as u128) / 1000;
    let extra = if (vrf_value as u128) < frac_threshold {
        1
    } else {
        0
    };

    whole_seats + extra
}

/// Construct the VRF input for leader election at a given height and round.
pub fn leader_vrf_input(height: u64, round: u32) -> Vec<u8> {
    let mut input = Vec::with_capacity(12);
    input.extend_from_slice(&height.to_le_bytes());
    input.extend_from_slice(&round.to_le_bytes());
    input
}

/// Construct the VRF input for committee sortition at a given height, round, and role.
pub fn sortition_vrf_input(height: u64, round: u32, role: &str) -> Vec<u8> {
    let mut input = Vec::with_capacity(12 + role.len());
    input.extend_from_slice(&height.to_le_bytes());
    input.extend_from_slice(&round.to_le_bytes());
    input.extend_from_slice(role.as_bytes());
    input
}

// ═══════════════════════════════════════════════════════════════════════════
// Randomness Beacon
// ═══════════════════════════════════════════════════════════════════════════

/// On-chain randomness beacon that chains VRF outputs across blocks.
///
/// `beacon[h] = BLAKE3(beacon[h-1] || vrf_output[h])`
///
/// This produces a verifiable, unpredictable, bias-resistant randomness
/// source that smart contracts can use for lotteries, NFT minting, etc.
#[derive(Debug, Clone)]
pub struct RandomnessBeacon {
    /// Current accumulated randomness.
    current: [u8; 32],
    /// Last height that was ingested.
    last_height: u64,
}

impl RandomnessBeacon {
    /// Create a new beacon with genesis seed.
    pub fn new() -> Self {
        Self {
            current: [0u8; 32],
            last_height: 0,
        }
    }

    /// Create a beacon from a known state (e.g., loading from disk).
    pub fn from_state(current: [u8; 32], last_height: u64) -> Self {
        Self {
            current,
            last_height,
        }
    }

    /// Ingest a new block's VRF output and advance the beacon.
    /// Returns the new beacon value.
    pub fn ingest(&mut self, height: u64, vrf_output: &[u8; 32]) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.current);
        hasher.update(vrf_output);
        hasher.update(&height.to_le_bytes());
        self.current = *hasher.finalize().as_bytes();
        self.last_height = height;
        self.current
    }

    /// Get the current beacon value.
    pub fn current(&self) -> [u8; 32] {
        self.current
    }

    /// Last height that was ingested.
    pub fn last_height(&self) -> u64 {
        self.last_height
    }

    /// Derive domain-separated randomness for a specific purpose.
    /// E.g., `derive(b"lottery:contract_42")` for a lottery contract.
    ///
    /// Deterministic: same beacon state + same domain = same output.
    pub fn derive(&self, domain: &[u8]) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"EvaporChain_Beacon_Derive");
        hasher.update(&self.current);
        hasher.update(domain);
        *hasher.finalize().as_bytes()
    }
}

impl Default for RandomnessBeacon {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vrf_evaluate_and_verify() {
        let kp = VrfKeypair::generate();
        let alpha = b"height=100,round=0";
        let (output, proof) = kp.evaluate(alpha);

        assert!(vrf_verify(&kp.public_key_bytes(), alpha, &output, &proof));
    }

    #[test]
    fn test_vrf_wrong_alpha_rejects() {
        let kp = VrfKeypair::generate();
        let (output, proof) = kp.evaluate(b"height=100");

        assert!(!vrf_verify(
            &kp.public_key_bytes(),
            b"height=200",
            &output,
            &proof
        ));
    }

    #[test]
    fn test_vrf_wrong_key_rejects() {
        let kp1 = VrfKeypair::generate();
        let kp2 = VrfKeypair::generate();
        let alpha = b"test";
        let (output, proof) = kp1.evaluate(alpha);

        assert!(!vrf_verify(&kp2.public_key_bytes(), alpha, &output, &proof));
    }

    #[test]
    fn test_vrf_tampered_output_rejects() {
        let kp = VrfKeypair::generate();
        let alpha = b"test";
        let (mut output, proof) = kp.evaluate(alpha);
        output.0[0] ^= 0xFF;

        assert!(!vrf_verify(&kp.public_key_bytes(), alpha, &output, &proof));
    }

    #[test]
    fn test_vrf_tampered_proof_rejects() {
        let kp = VrfKeypair::generate();
        let alpha = b"test";
        let (output, mut proof) = kp.evaluate(alpha);
        proof.0[0] ^= 0xFF;

        assert!(!vrf_verify(&kp.public_key_bytes(), alpha, &output, &proof));
    }

    #[test]
    fn test_vrf_different_inputs_different_outputs() {
        let kp = VrfKeypair::generate();
        let (out1, _) = kp.evaluate(b"input_a");
        let (out2, _) = kp.evaluate(b"input_b");

        assert_ne!(out1, out2);
    }

    #[test]
    fn test_vrf_output_is_32_bytes() {
        let kp = VrfKeypair::generate();
        let (output, _) = kp.evaluate(b"test");
        assert_eq!(output.0.len(), 32);
    }

    #[test]
    fn test_vrf_proof_is_3293_bytes() {
        let kp = VrfKeypair::generate();
        let (_, proof) = kp.evaluate(b"test");
        assert_eq!(proof.0.len(), 3293);
    }

    #[test]
    fn test_vrf_from_bytes_roundtrip() {
        let kp = VrfKeypair::generate();
        let pk = kp.public_key_bytes();
        let sk = kp.secret_key_bytes();

        let kp2 = VrfKeypair::from_bytes(&pk, &sk).unwrap();
        let alpha = b"roundtrip";
        let (output, proof) = kp2.evaluate(alpha);

        assert!(vrf_verify(&pk, alpha, &output, &proof));
    }

    // ─── Leader Check Tests ────────────────────────────────────────────

    #[test]
    fn test_leader_check_zero_stake_never_selected() {
        let kp = VrfKeypair::generate();
        let (output, _) = kp.evaluate(b"test");
        assert!(!vrf_leader_check(&output, 0, 1000));
    }

    #[test]
    fn test_leader_check_full_stake_always_selected() {
        // If stake == total_stake, threshold == u64::MAX, so always selected.
        let kp = VrfKeypair::generate();
        let (output, _) = kp.evaluate(b"test");
        assert!(vrf_leader_check(&output, 1000, 1000));
    }

    #[test]
    fn test_leader_check_statistical_fairness() {
        // With 50% stake, roughly 50% of random VRF outputs should pass.
        let kp = VrfKeypair::generate();
        let mut selected = 0;
        let trials = 200;

        for i in 0..trials {
            let input = format!("trial_{}", i);
            let (output, _) = kp.evaluate(input.as_bytes());
            if vrf_leader_check(&output, 500, 1000) {
                selected += 1;
            }
        }

        // Expect ~100 selections. Allow wide margin for randomness.
        assert!(
            selected > 50 && selected < 150,
            "Expected ~100 selections out of {}, got {}",
            trials,
            selected
        );
    }

    // ─── Sortition Tests ───────────────────────────────────────────────

    #[test]
    fn test_sortition_zero_stake_zero_seats() {
        let kp = VrfKeypair::generate();
        let (output, _) = kp.evaluate(b"test");
        assert_eq!(sortition(&output, 0, 1000, 20), 0);
    }

    #[test]
    fn test_sortition_full_stake_gets_all_seats() {
        let kp = VrfKeypair::generate();
        let (output, _) = kp.evaluate(b"test");
        // 100% stake with committee size 20 => 20 seats guaranteed
        assert_eq!(sortition(&output, 1000, 1000, 20), 20);
    }

    #[test]
    fn test_sortition_proportional() {
        // 25% stake, committee size 20 => expect ~5 seats
        let kp = VrfKeypair::generate();
        let mut total_seats = 0u64;
        let trials = 200;

        for i in 0..trials {
            let input = format!("sort_{}", i);
            let (output, _) = kp.evaluate(input.as_bytes());
            total_seats += sortition(&output, 250, 1000, 20);
        }

        let avg = total_seats as f64 / trials as f64;
        assert!(
            avg > 3.0 && avg < 7.0,
            "Expected avg ~5.0 seats, got {}",
            avg
        );
    }

    // ─── Randomness Beacon Tests ───────────────────────────────────────

    #[test]
    fn test_beacon_ingest_changes_value() {
        let mut beacon = RandomnessBeacon::new();
        let initial = beacon.current();
        let vrf_out = [0xABu8; 32];
        beacon.ingest(1, &vrf_out);
        assert_ne!(beacon.current(), initial);
    }

    #[test]
    fn test_beacon_deterministic() {
        let mut b1 = RandomnessBeacon::new();
        let mut b2 = RandomnessBeacon::new();
        let vrf1 = [1u8; 32];
        let vrf2 = [2u8; 32];

        b1.ingest(1, &vrf1);
        b1.ingest(2, &vrf2);

        b2.ingest(1, &vrf1);
        b2.ingest(2, &vrf2);

        assert_eq!(b1.current(), b2.current());
    }

    #[test]
    fn test_beacon_order_matters() {
        let mut b1 = RandomnessBeacon::new();
        let mut b2 = RandomnessBeacon::new();
        let vrf1 = [1u8; 32];
        let vrf2 = [2u8; 32];

        b1.ingest(1, &vrf1);
        b1.ingest(2, &vrf2);

        b2.ingest(1, &vrf2);
        b2.ingest(2, &vrf1);

        assert_ne!(b1.current(), b2.current());
    }

    #[test]
    fn test_beacon_derive_domain_separation() {
        let mut beacon = RandomnessBeacon::new();
        beacon.ingest(1, &[42u8; 32]);

        let r1 = beacon.derive(b"lottery");
        let r2 = beacon.derive(b"nft_mint");
        assert_ne!(r1, r2);
    }

    #[test]
    fn test_beacon_derive_deterministic() {
        let mut beacon = RandomnessBeacon::new();
        beacon.ingest(1, &[42u8; 32]);

        let r1 = beacon.derive(b"lottery");
        let r2 = beacon.derive(b"lottery");
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_beacon_height_tracking() {
        let mut beacon = RandomnessBeacon::new();
        assert_eq!(beacon.last_height(), 0);
        beacon.ingest(5, &[1u8; 32]);
        assert_eq!(beacon.last_height(), 5);
        beacon.ingest(10, &[2u8; 32]);
        assert_eq!(beacon.last_height(), 10);
    }

    #[test]
    fn test_leader_vrf_input_deterministic() {
        let a = leader_vrf_input(100, 0);
        let b = leader_vrf_input(100, 0);
        assert_eq!(a, b);
        let c = leader_vrf_input(100, 1);
        assert_ne!(a, c);
    }

    #[test]
    fn test_sortition_vrf_input_includes_role() {
        let a = sortition_vrf_input(100, 0, "prevote");
        let b = sortition_vrf_input(100, 0, "precommit");
        assert_ne!(a, b);
    }

    #[test]
    fn test_vrf_empty_alpha() {
        let kp = VrfKeypair::generate();
        let (output, proof) = kp.evaluate(b"");
        assert!(vrf_verify(&kp.public_key_bytes(), b"", &output, &proof));
    }

    #[test]
    fn test_vrf_large_alpha() {
        let kp = VrfKeypair::generate();
        let alpha = vec![0xCDu8; 10_000];
        let (output, proof) = kp.evaluate(&alpha);
        assert!(vrf_verify(&kp.public_key_bytes(), &alpha, &output, &proof));
    }
}
