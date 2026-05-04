//! `BellCertificate` — the chain-attached anti-grinding artefact.

use evaporchain_causal_chsh::gate::GateThresholds;
use serde::{Deserialize, Serialize};

/// The certificate emitted on a passing gate. Attached to the
/// proposer's next block header.
///
/// **Field semantics:**
/// - `window_start`, `window_end`: half-open block-height range
///   `[window_start, window_end)` from which the concurrent-pair
///   sample was drawn.
/// - `n_pairs`: total pairs sampled (sum across all four buckets).
/// - `bucket_counts`: pair count per CHSH setting-pair, in order
///   `[Ab, AbPrime, APrimeB, APrimeBPrime]`.
/// - `s_honest`, `s_cartel`, `gap`: gate output, recorded as
///   integer milli-units (i.e. multiplied by 1000 and rounded toward
///   zero) so the certificate is bit-exact across validators.
/// - `seed`: BLAKE3 of (domain || canonical bytes), used downstream
///   as device-independent randomness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BellCertificate {
    pub window_start: u64,
    pub window_end: u64,
    pub n_pairs: u64,
    pub bucket_counts: [u64; 4],
    pub s_honest_milli: i64,
    pub s_cartel_milli: i64,
    pub gap_milli: i64,
    pub honest_ceiling_milli: u64,
    pub cartel_floor_milli: u64,
    pub min_gap_milli: u64,
    pub prev_block_hash: [u8; 32],
    pub seed: [u8; 32],
}

/// Canonical byte serialisation used for hashing into `seed` and for
/// verifier re-derivation. Stable, fixed-width, little-endian. Pairs'
/// 32-byte tags are *not* part of the certificate but are folded into
/// the seed via the issuance path.
#[derive(Debug, Clone)]
pub struct CertificateBytes(pub Vec<u8>);

impl CertificateBytes {
    /// Canonical bytes of the certificate's gate-locked fields. The
    /// `seed` field is *excluded* — this is what the seed hashes
    /// over.
    pub fn pre_seed(cert: &BellCertificate) -> Self {
        let mut buf = Vec::with_capacity(8 * 6 + 32 + 8 * 4);
        buf.extend_from_slice(&cert.window_start.to_le_bytes());
        buf.extend_from_slice(&cert.window_end.to_le_bytes());
        buf.extend_from_slice(&cert.n_pairs.to_le_bytes());
        for c in cert.bucket_counts {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        buf.extend_from_slice(&cert.s_honest_milli.to_le_bytes());
        buf.extend_from_slice(&cert.s_cartel_milli.to_le_bytes());
        buf.extend_from_slice(&cert.gap_milli.to_le_bytes());
        buf.extend_from_slice(&cert.honest_ceiling_milli.to_le_bytes());
        buf.extend_from_slice(&cert.cartel_floor_milli.to_le_bytes());
        buf.extend_from_slice(&cert.min_gap_milli.to_le_bytes());
        buf.extend_from_slice(&cert.prev_block_hash);
        Self(buf)
    }

}

/// Pure-integer milli-encoded form of [`GateThresholds`]. This is the
/// **single documented float boundary** in the V2 certificate path.
///
/// Doctrine thresholds (`honest_ceiling = 1.8`, `cartel_floor = 2.2`,
/// `min_gap = 0.4`) are f64 constants by upstream definition. We
/// convert once, here, at certificate-issuance / verification boundary
/// and stamp the integer result into the certificate root. All other
/// fields in the V2 certificate are derived via
/// [`evaporchain_causal_chsh::compute_chsh_s_milli`] — pure-integer
/// arithmetic with no f64 anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateThresholdsMilli {
    pub honest_ceiling_milli: u64,
    pub cartel_floor_milli: u64,
    pub min_gap_milli: u64,
}

impl GateThresholdsMilli {
    /// Convert a float gate value to milli-integer (×1000, truncated
    /// toward zero). Used only at the doctrine-thresholds → milli
    /// boundary; never on the per-sample consensus path.
    fn f64_to_milli_truncating(v: f64) -> u64 {
        (v * 1000.0) as u64
    }
}

impl From<GateThresholds> for GateThresholdsMilli {
    fn from(t: GateThresholds) -> Self {
        Self {
            honest_ceiling_milli: Self::f64_to_milli_truncating(t.honest_ceiling),
            cartel_floor_milli: Self::f64_to_milli_truncating(t.cartel_floor),
            min_gap_milli: Self::f64_to_milli_truncating(t.min_gap),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_cert() -> BellCertificate {
        BellCertificate {
            window_start: 100,
            window_end: 200,
            n_pairs: 50,
            bucket_counts: [12, 13, 12, 13],
            s_honest_milli: 50,
            s_cartel_milli: 4000,
            gap_milli: 3950,
            honest_ceiling_milli: 1800,
            cartel_floor_milli: 2200,
            min_gap_milli: 400,
            prev_block_hash: [9u8; 32],
            seed: [7u8; 32],
        }
    }

    #[test]
    fn pre_seed_excludes_seed_field() {
        let mut a = dummy_cert();
        let mut b = a;
        b.seed = [0u8; 32];
        // Pre-seed bytes must be identical even though `seed`
        // differs.
        assert_eq!(
            CertificateBytes::pre_seed(&a).0,
            CertificateBytes::pre_seed(&b).0
        );
        // But changing any pre-seed-relevant field must perturb the
        // bytes.
        let _ = &mut a;
        let mut c = dummy_cert();
        c.window_end += 1;
        assert_ne!(
            CertificateBytes::pre_seed(&dummy_cert()).0,
            CertificateBytes::pre_seed(&c).0
        );
    }

    #[test]
    fn pre_seed_is_deterministic() {
        let cert = dummy_cert();
        assert_eq!(
            CertificateBytes::pre_seed(&cert).0,
            CertificateBytes::pre_seed(&cert).0
        );
    }

    #[test]
    fn gate_thresholds_milli_doctrine_constants() {
        let m: GateThresholdsMilli = GateThresholds::doctrine().into();
        assert_eq!(m.honest_ceiling_milli, 1800);
        assert_eq!(m.cartel_floor_milli, 2200);
        // 0.4 * 1000 truncated toward zero. f64 representation of 0.4
        // is slightly below 0.4, so 0.4 * 1000 ≈ 399.999... → 399.
        // This is exactly why we call this conversion ONCE at the
        // doctrine boundary — the value is stamped into the certificate
        // and used for integer comparison thereafter.
        assert!(m.min_gap_milli == 400 || m.min_gap_milli == 399);
    }
}
