//! Issuing a `BellCertificate` from a window of concurrent pairs.

use blake3::Hasher;
use evaporchain_causal_chsh::chsh::compute_chsh_s_milli;
use evaporchain_causal_chsh::gate::GateThresholds;
use evaporchain_causal_chsh_cartels::coordinated_subset_cartel;
use thiserror::Error;

use crate::certificate::{BellCertificate, CertificateBytes, GateThresholdsMilli};
use crate::observables::{build_samples, ConcurrentPair};

const SEED_DOMAIN: &[u8] = b"evaporchain:bell-beacon:v2:seed\0";
const CARTEL_DOMAIN: &[u8] = b"evaporchain:bell-beacon:v2:cartel\0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BeaconError {
    #[error("window is empty — at least one concurrent pair required")]
    EmptyWindow,
    #[error("window does not cover all four CHSH setting-pair buckets")]
    InsufficientCoverage,
    #[error("window range invalid: window_end ({end}) <= window_start ({start})")]
    InvalidWindowRange { start: u64, end: u64 },
    #[error("CHSH computation failed on honest sample: {0}")]
    HonestChshFailure(String),
    #[error("CHSH computation failed on cartel sample: {0}")]
    CartelChshFailure(String),
    #[error("gate failed under doctrine thresholds: {0}")]
    GateFailed(String),
    #[error("synthetic cartel injection failed: {0}")]
    CartelInjectionFailure(String),
}

/// Issue a `BellCertificate` over a window of concurrent pairs.
///
/// On `Pass`, the certificate is anchored to `prev_block_hash` and
/// the seed hashes (domain || chain_id || pre_seed_bytes || pair_tags).
/// On `Fail`, returns `BeaconError::GateFailed` with the reasons.
///
/// **Audit fix HIGH H2**: `chain_id` is now bound into both the
/// cartel seed and the certificate seed. Two distinct chains (or
/// distinct test-vs-prod harnesses) sharing a `prev_block_hash` value
/// no longer derive the same cartel sample or the same certificate
/// seed.
pub fn issue_certificate(
    chain_id: &str,
    window_start: u64,
    window_end: u64,
    pairs: &[ConcurrentPair],
    thresholds: GateThresholds,
    prev_block_hash: [u8; 32],
) -> Result<BellCertificate, BeaconError> {
    if window_end <= window_start {
        return Err(BeaconError::InvalidWindowRange {
            start: window_start,
            end: window_end,
        });
    }
    if pairs.is_empty() {
        return Err(BeaconError::EmptyWindow);
    }

    let honest = build_samples(pairs).ok_or(BeaconError::InsufficientCoverage)?;

    // Pure-integer honest S in milli-units. NO f64 on this path.
    let s_honest_milli = compute_chsh_s_milli(&honest)
        .map_err(|e| BeaconError::HonestChshFailure(format!("{e}")))?;

    // Synthetic cartel injection over the *same* sample set, seeded
    // by prev_block_hash so different proposers produce identical
    // cartel samples for the same window.
    let cartel_seed = derive_cartel_seed(chain_id, prev_block_hash, window_start, window_end);
    let cartel = coordinated_subset_cartel(&honest, 1, 1, cartel_seed)
        .map_err(|e| BeaconError::CartelInjectionFailure(format!("{e}")))?;

    let s_cartel_milli = compute_chsh_s_milli(&cartel)
        .map_err(|e| BeaconError::CartelChshFailure(format!("{e}")))?;

    // Convert thresholds at the documented f64 boundary.
    let thresholds_milli: GateThresholdsMilli = thresholds.into();

    let gap_milli = s_cartel_milli - s_honest_milli;

    // Integer-only gate evaluation. Mirrors `run_synthetic_gate`
    // semantics exactly but on milli-integer values:
    //   - S_honest_milli >= honest_ceiling_milli  → vacuous
    //   - S_cartel_milli <= cartel_floor_milli    → no discrimination
    //   - gap_milli      <= min_gap_milli         → insufficient SNR
    let mut reasons: Vec<String> = Vec::new();
    if s_honest_milli >= thresholds_milli.honest_ceiling_milli as i64 {
        reasons.push(format!(
            "S_honest_milli ({}) >= honest_ceiling_milli ({}) — inequality vacuous",
            s_honest_milli, thresholds_milli.honest_ceiling_milli
        ));
    }
    if s_cartel_milli <= thresholds_milli.cartel_floor_milli as i64 {
        reasons.push(format!(
            "S_cartel_milli ({}) <= cartel_floor_milli ({}) — fails to discriminate",
            s_cartel_milli, thresholds_milli.cartel_floor_milli
        ));
    }
    if gap_milli <= thresholds_milli.min_gap_milli as i64 {
        reasons.push(format!(
            "gap_milli ({}) <= min_gap_milli ({}) — insufficient SNR",
            gap_milli, thresholds_milli.min_gap_milli
        ));
    }
    if !reasons.is_empty() {
        return Err(BeaconError::GateFailed(reasons.join("; ")));
    }

    let bucket_counts: [u64; 4] = [
        honest.samples_ab.len() as u64,
        honest.samples_ab_prime.len() as u64,
        honest.samples_a_prime_b.len() as u64,
        honest.samples_a_prime_b_prime.len() as u64,
    ];

    let mut cert = BellCertificate {
        window_start,
        window_end,
        n_pairs: pairs.len() as u64,
        bucket_counts,
        s_honest_milli,
        s_cartel_milli,
        gap_milli,
        honest_ceiling_milli: thresholds_milli.honest_ceiling_milli,
        cartel_floor_milli: thresholds_milli.cartel_floor_milli,
        min_gap_milli: thresholds_milli.min_gap_milli,
        prev_block_hash,
        seed: [0u8; 32],
    };
    cert.seed = derive_seed(chain_id, &cert, pairs);

    Ok(cert)
}

/// Domain-separated derivation of the synthetic-cartel seed. Same
/// inputs → same cartel sample → same `s_cartel` across validators.
/// chain_id is length-prefixed to defeat trivial collisions of
/// neighbouring chains with related ids.
pub(crate) fn derive_cartel_seed(
    chain_id: &str,
    prev_block_hash: [u8; 32],
    window_start: u64,
    window_end: u64,
) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(CARTEL_DOMAIN);
    h.update(&(chain_id.len() as u64).to_le_bytes());
    h.update(chain_id.as_bytes());
    h.update(&prev_block_hash);
    h.update(&window_start.to_le_bytes());
    h.update(&window_end.to_le_bytes());
    *h.finalize().as_bytes()
}

/// The certificate seed: BLAKE3(domain || chain_id || pre-seed bytes
/// || sorted pair tags). Sort is by tag bytes lexicographically, so
/// reordering the input pair vector cannot change the seed.
pub(crate) fn derive_seed(
    chain_id: &str,
    cert: &BellCertificate,
    pairs: &[ConcurrentPair],
) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(SEED_DOMAIN);
    h.update(&(chain_id.len() as u64).to_le_bytes());
    h.update(chain_id.as_bytes());
    h.update(&CertificateBytes::pre_seed(cert).0);

    let mut tags: Vec<[u8; 32]> = pairs.iter().map(|p| p.tag).collect();
    tags.sort_unstable();
    for tag in &tags {
        h.update(tag);
    }
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observables::PairStats;

    fn pair(e1: u64, t1: u64, e2: u64, t2: u64, tag_byte: u8) -> ConcurrentPair {
        let mut tag = [0u8; 32];
        tag[31] = tag_byte;
        // Use the tag's other bytes so different (high-entropy) tags
        // hash differently — we'll set a unique high byte too.
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

    #[test]
    fn rejects_invalid_window_range() {
        let err = issue_certificate("test-chain-v1", 200, 100, &balanced_window(), GateThresholds::doctrine(), [0u8; 32])
            .unwrap_err();
        assert!(matches!(err, BeaconError::InvalidWindowRange { .. }));
    }

    #[test]
    fn rejects_empty_window() {
        let err = issue_certificate("test-chain-v1", 100, 200, &[], GateThresholds::doctrine(), [0u8; 32])
            .unwrap_err();
        assert_eq!(err, BeaconError::EmptyWindow);
    }

    #[test]
    fn rejects_single_bucket_window() {
        // All pairs have tag_byte=0 → all bucketed to AB.
        let trace = vec![pair(10, 10, 10, 10, 0); 4];
        let err = issue_certificate("test-chain-v1", 100, 200, &trace, GateThresholds::doctrine(), [0u8; 32])
            .unwrap_err();
        assert_eq!(err, BeaconError::InsufficientCoverage);
    }

    #[test]
    fn issues_on_balanced_window() {
        let cert = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &balanced_window(),
            GateThresholds::doctrine(),
            [9u8; 32],
        )
        .unwrap();
        assert_eq!(cert.window_start, 100);
        assert_eq!(cert.window_end, 200);
        assert_eq!(cert.n_pairs, 16);
        assert_eq!(cert.bucket_counts.iter().sum::<u64>(), 16);
        // Honest S < cartel floor; cartel S = 4 → milli = 4000.
        assert!(cert.s_cartel_milli >= cert.cartel_floor_milli as i64);
        assert!(cert.gap_milli >= cert.min_gap_milli as i64);
        // Seed is non-trivial.
        assert_ne!(cert.seed, [0u8; 32]);
        assert_eq!(cert.prev_block_hash, [9u8; 32]);
    }

    #[test]
    fn determinism_same_inputs_same_certificate() {
        let a = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &balanced_window(),
            GateThresholds::doctrine(),
            [9u8; 32],
        )
        .unwrap();
        let b = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &balanced_window(),
            GateThresholds::doctrine(),
            [9u8; 32],
        )
        .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_prev_block_hash_gives_different_seed() {
        let a = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &balanced_window(),
            GateThresholds::doctrine(),
            [1u8; 32],
        )
        .unwrap();
        let b = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &balanced_window(),
            GateThresholds::doctrine(),
            [2u8; 32],
        )
        .unwrap();
        assert_ne!(a.seed, b.seed);
        assert_ne!(a.prev_block_hash, b.prev_block_hash);
    }

    #[test]
    fn pair_reordering_does_not_change_seed() {
        // Anti-grinding: a malicious proposer cannot pick the seed
        // by reordering the pair vector.
        let mut pairs = balanced_window();
        let a = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &pairs,
            GateThresholds::doctrine(),
            [9u8; 32],
        )
        .unwrap();
        pairs.reverse();
        let b = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &pairs,
            GateThresholds::doctrine(),
            [9u8; 32],
        )
        .unwrap();
        assert_eq!(a.seed, b.seed);
    }
}
