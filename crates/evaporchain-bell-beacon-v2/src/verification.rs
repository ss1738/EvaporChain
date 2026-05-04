//! Verifier-side validation of a `BellCertificate`.

use evaporchain_causal_chsh::chsh::compute_chsh_s_milli;
use evaporchain_causal_chsh::gate::GateThresholds;
use evaporchain_causal_chsh_cartels::coordinated_subset_cartel;
use thiserror::Error;

use crate::certificate::{BellCertificate, GateThresholdsMilli};
use crate::issuance::{derive_cartel_seed, derive_seed};
use crate::observables::{build_samples, ConcurrentPair};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("window range invalid: window_end ({end}) <= window_start ({start})")]
    InvalidWindowRange { start: u64, end: u64 },
    #[error("window is empty — at least one concurrent pair required")]
    EmptyWindow,
    #[error("window does not cover all four CHSH setting-pair buckets")]
    InsufficientCoverage,
    #[error("certificate window range does not match supplied bounds")]
    WindowMismatch,
    #[error("certificate prev_block_hash does not match supplied prev_block_hash")]
    PrevHashMismatch,
    #[error("certificate n_pairs ({cert}) does not match supplied window ({given})")]
    PairCountMismatch { cert: u64, given: u64 },
    #[error("certificate bucket_counts do not match recomputed buckets")]
    BucketCountMismatch,
    #[error("certificate s_honest_milli ({cert}) does not match recomputed ({recomputed})")]
    SHonestMismatch { cert: i64, recomputed: i64 },
    #[error("certificate s_cartel_milli ({cert}) does not match recomputed ({recomputed})")]
    SCartelMismatch { cert: i64, recomputed: i64 },
    #[error("certificate gap_milli ({cert}) does not match recomputed ({recomputed})")]
    GapMismatch { cert: i64, recomputed: i64 },
    #[error("certificate threshold milli-units do not match supplied thresholds")]
    ThresholdMismatch,
    #[error("recomputed gate verdict failed: {0}")]
    GateFailed(String),
    #[error("certificate seed does not match recomputed seed")]
    SeedMismatch,
    #[error("CHSH computation failed: {0}")]
    ChshFailure(String),
}

/// Re-run the gate against the supplied `pairs` and confirm the
/// certificate's bound fields match. Returns `Ok(())` iff every
/// field reproduces exactly under the same inputs.
///
/// **Audit fix HIGH H2**: `chain_id` must match the value the
/// certificate was issued under, since chain_id is folded into both
/// the cartel seed and the certificate seed. Cross-chain replay of a
/// cert is now rejected at the seed-mismatch check.
pub fn verify_certificate(
    chain_id: &str,
    pairs: &[ConcurrentPair],
    prev_block_hash: [u8; 32],
    thresholds: GateThresholds,
    cert: &BellCertificate,
) -> Result<(), VerifyError> {
    if cert.window_end <= cert.window_start {
        return Err(VerifyError::InvalidWindowRange {
            start: cert.window_start,
            end: cert.window_end,
        });
    }
    if pairs.is_empty() {
        return Err(VerifyError::EmptyWindow);
    }
    if cert.prev_block_hash != prev_block_hash {
        return Err(VerifyError::PrevHashMismatch);
    }
    if cert.n_pairs != pairs.len() as u64 {
        return Err(VerifyError::PairCountMismatch {
            cert: cert.n_pairs,
            given: pairs.len() as u64,
        });
    }

    let thresholds_milli: GateThresholdsMilli = thresholds.into();
    if cert.honest_ceiling_milli != thresholds_milli.honest_ceiling_milli
        || cert.cartel_floor_milli != thresholds_milli.cartel_floor_milli
        || cert.min_gap_milli != thresholds_milli.min_gap_milli
    {
        return Err(VerifyError::ThresholdMismatch);
    }

    let honest = build_samples(pairs).ok_or(VerifyError::InsufficientCoverage)?;

    let recomputed_buckets: [u64; 4] = [
        honest.samples_ab.len() as u64,
        honest.samples_ab_prime.len() as u64,
        honest.samples_a_prime_b.len() as u64,
        honest.samples_a_prime_b_prime.len() as u64,
    ];
    if recomputed_buckets != cert.bucket_counts {
        return Err(VerifyError::BucketCountMismatch);
    }

    let s_honest_milli =
        compute_chsh_s_milli(&honest).map_err(|e| VerifyError::ChshFailure(format!("{e}")))?;
    if s_honest_milli != cert.s_honest_milli {
        return Err(VerifyError::SHonestMismatch {
            cert: cert.s_honest_milli,
            recomputed: s_honest_milli,
        });
    }

    let cartel_seed = derive_cartel_seed(chain_id, prev_block_hash, cert.window_start, cert.window_end);
    let cartel = coordinated_subset_cartel(&honest, 1, 1, cartel_seed)
        .map_err(|e| VerifyError::ChshFailure(format!("{e}")))?;
    let s_cartel_milli =
        compute_chsh_s_milli(&cartel).map_err(|e| VerifyError::ChshFailure(format!("{e}")))?;
    if s_cartel_milli != cert.s_cartel_milli {
        return Err(VerifyError::SCartelMismatch {
            cert: cert.s_cartel_milli,
            recomputed: s_cartel_milli,
        });
    }

    let gap_milli = s_cartel_milli - s_honest_milli;
    if gap_milli != cert.gap_milli {
        return Err(VerifyError::GapMismatch {
            cert: cert.gap_milli,
            recomputed: gap_milli,
        });
    }

    // Integer-only gate evaluation (mirrors issuance.rs):
    let mut reasons: Vec<String> = Vec::new();
    if s_honest_milli >= thresholds_milli.honest_ceiling_milli as i64 {
        reasons.push(format!(
            "S_honest_milli ({}) >= honest_ceiling_milli ({})",
            s_honest_milli, thresholds_milli.honest_ceiling_milli
        ));
    }
    if s_cartel_milli <= thresholds_milli.cartel_floor_milli as i64 {
        reasons.push(format!(
            "S_cartel_milli ({}) <= cartel_floor_milli ({})",
            s_cartel_milli, thresholds_milli.cartel_floor_milli
        ));
    }
    if gap_milli <= thresholds_milli.min_gap_milli as i64 {
        reasons.push(format!(
            "gap_milli ({}) <= min_gap_milli ({})",
            gap_milli, thresholds_milli.min_gap_milli
        ));
    }
    if !reasons.is_empty() {
        return Err(VerifyError::GateFailed(reasons.join("; ")));
    }

    let recomputed_seed = derive_seed(chain_id, cert, pairs);
    if recomputed_seed != cert.seed {
        return Err(VerifyError::SeedMismatch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issuance::issue_certificate;
    use crate::observables::PairStats;

    fn pair(e1: u64, t1: u64, e2: u64, t2: u64, tag_byte: u8) -> ConcurrentPair {
        let mut tag = [0u8; 32];
        tag[31] = tag_byte;
        tag[0] = tag_byte;
        ConcurrentPair {
            first: PairStats {
                energy: e1,
                tx_count: t1,
            },
            second: PairStats {
                energy: e2,
                tx_count: t2,
            },
            tag,
        }
    }

    fn balanced_window() -> Vec<ConcurrentPair> {
        let mut out = Vec::new();
        for i in 0..16u8 {
            out.push(pair(
                if i & 1 == 1 { 100 } else { 10 },
                if (i >> 1) & 1 == 1 { 100 } else { 10 },
                if (i >> 2) & 1 == 1 { 100 } else { 10 },
                if (i >> 3) & 1 == 1 { 100 } else { 10 },
                i,
            ));
        }
        out
    }

    fn good_cert() -> (Vec<ConcurrentPair>, [u8; 32], BellCertificate) {
        let pairs = balanced_window();
        let prev = [9u8; 32];
        let cert =
            issue_certificate("test-chain-v1", 100, 200, &pairs, GateThresholds::doctrine(), prev).unwrap();
        (pairs, prev, cert)
    }

    // ── happy path ────────────────────────────────────────────────

    #[test]
    fn round_trip_verifies() {
        let (pairs, prev, cert) = good_cert();
        verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &cert).unwrap();
    }

    // ── tampered fields rejected ──────────────────────────────────

    #[test]
    fn tampered_seed_rejected() {
        let (pairs, prev, mut cert) = good_cert();
        cert.seed[0] ^= 0xff;
        let err = verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &cert).unwrap_err();
        assert_eq!(err, VerifyError::SeedMismatch);
    }

    #[test]
    fn tampered_s_honest_rejected() {
        let (pairs, prev, mut cert) = good_cert();
        cert.s_honest_milli += 100;
        let err = verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &cert).unwrap_err();
        assert!(matches!(err, VerifyError::SHonestMismatch { .. }));
    }

    #[test]
    fn tampered_s_cartel_rejected() {
        let (pairs, prev, mut cert) = good_cert();
        cert.s_cartel_milli -= 1000;
        let err = verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &cert).unwrap_err();
        assert!(matches!(err, VerifyError::SCartelMismatch { .. }));
    }

    #[test]
    fn tampered_gap_rejected() {
        let (pairs, prev, mut cert) = good_cert();
        cert.gap_milli += 50;
        let err = verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &cert).unwrap_err();
        assert!(matches!(err, VerifyError::GapMismatch { .. }));
    }

    #[test]
    fn tampered_n_pairs_rejected() {
        let (pairs, prev, mut cert) = good_cert();
        cert.n_pairs += 1;
        let err = verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &cert).unwrap_err();
        assert!(matches!(err, VerifyError::PairCountMismatch { .. }));
    }

    #[test]
    fn tampered_bucket_counts_rejected() {
        let (pairs, prev, mut cert) = good_cert();
        cert.bucket_counts[0] += 1;
        cert.bucket_counts[1] -= 1;
        let err = verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &cert).unwrap_err();
        assert_eq!(err, VerifyError::BucketCountMismatch);
    }

    #[test]
    fn wrong_prev_block_hash_rejected() {
        let (pairs, _prev, cert) = good_cert();
        let err = verify_certificate(
            "test-chain-v1",
            &pairs,
            [0u8; 32],
            GateThresholds::doctrine(),
            &cert,
        )
        .unwrap_err();
        assert_eq!(err, VerifyError::PrevHashMismatch);
    }

    #[test]
    fn changed_thresholds_rejected() {
        let (pairs, prev, cert) = good_cert();
        let alt = GateThresholds {
            honest_ceiling: 1.5,
            cartel_floor: 2.0,
            min_gap: 0.3,
        };
        let err = verify_certificate("test-chain-v1", &pairs, prev, alt, &cert).unwrap_err();
        assert_eq!(err, VerifyError::ThresholdMismatch);
    }

    #[test]
    fn substituted_pair_set_rejects_via_seed() {
        // Same window count + same prev hash, but a different pair
        // set with the same bucket distribution → s_honest /
        // bucket_counts may match by luck, but the seed cannot.
        let (pairs, prev, cert) = good_cert();
        let mut substituted = pairs.clone();
        // Permute the tags so the bucket counts may stay the same
        // but the exact tag set differs. Replace one tag's byte 0
        // with 0xff to guarantee divergence.
        substituted[0].tag[0] = 0xff;
        let err = verify_certificate("test-chain-v1", &substituted, prev, GateThresholds::doctrine(), &cert)
            .unwrap_err();
        // Either bucket counts differ (because tag[31]&0b11 changes
        // for one pair) or the seed mismatches if buckets stayed the
        // same. Both are valid rejection reasons.
        assert!(matches!(
            err,
            VerifyError::BucketCountMismatch | VerifyError::SeedMismatch
        ));
    }

    #[test]
    fn empty_pairs_rejected() {
        let (_pairs, prev, cert) = good_cert();
        let err = verify_certificate("test-chain-v1", &[], prev, GateThresholds::doctrine(), &cert).unwrap_err();
        assert_eq!(err, VerifyError::EmptyWindow);
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Bell-Certified Beacon V2 binds CHSH-violation gating
        // to the chain. A proposer issues a BellCertificate over a
        // window of concurrent-block pairs anchored to prev_block_hash.
        // The certificate is bit-exact across validators (gate values
        // stored as milli-integers; cartel injection seeded by prev
        // hash; seed hashes over sorted pair tags). Every field in the
        // certificate is verifiable: tampering with any of s_honest,
        // s_cartel, gap, bucket counts, n_pairs, prev_hash, thresholds,
        // or seed causes verify_certificate to reject."

        let (pairs, prev, cert) = good_cert();
        verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &cert).unwrap();

        // 8 tampering vectors, one per field. Already covered by the
        // tests above; press-claim test asserts the public invariant.
        for tampered in [
            {
                let mut c = cert;
                c.seed[5] ^= 1;
                c
            },
            {
                let mut c = cert;
                c.s_honest_milli += 1;
                c
            },
            {
                let mut c = cert;
                c.s_cartel_milli -= 1;
                c
            },
            {
                let mut c = cert;
                c.gap_milli += 1;
                c
            },
            {
                let mut c = cert;
                c.n_pairs -= 1;
                c
            },
            {
                let mut c = cert;
                c.bucket_counts[3] += 1;
                c
            },
            {
                let mut c = cert;
                c.honest_ceiling_milli += 100;
                c
            },
        ] {
            assert!(
                verify_certificate("test-chain-v1", &pairs, prev, GateThresholds::doctrine(), &tampered).is_err()
            );
        }
    }
}
