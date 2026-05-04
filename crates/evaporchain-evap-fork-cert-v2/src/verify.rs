//! V2 verifier.

use crate::cert::{CertV2Error, EvaporatedForkCertV2};
use crate::prove::compute_witness_v2;

/// Verify a V2 certificate. Pure function of the certificate alone.
/// Three rejection paths:
/// - `NotEvaporated` if `decayed_energy ≥ threshold`.
/// - `CausalityViolation` if `seed_anchor_epoch > evaluated_at_epoch`.
/// - `WitnessMismatch` if any cert field has been tampered with.
pub fn verify_evaporated_cert_v2(cert: &EvaporatedForkCertV2) -> Result<(), CertV2Error> {
    if cert.seed_anchor_epoch > cert.evaluated_at_epoch {
        return Err(CertV2Error::CausalityViolation {
            anchor: cert.seed_anchor_epoch,
            eval: cert.evaluated_at_epoch,
        });
    }
    if cert.decayed_energy >= cert.threshold {
        return Err(CertV2Error::NotEvaporated {
            decayed: cert.decayed_energy,
            threshold: cert.threshold,
        });
    }
    let derived = compute_witness_v2(
        cert.fork_root,
        cert.evaluated_at_epoch,
        cert.total_seed_energy,
        cert.decayed_energy,
        cert.threshold,
        &cert.bell_seed_anchor,
        cert.seed_anchor_epoch,
    );
    if derived != cert.witness {
        return Err(CertV2Error::WitnessMismatch {
            derived,
            claimed: cert.witness,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prove::prove_fork_evaporated_v2;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_evap_fork_cert::ForkBlock;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    fn one_block() -> Vec<ForkBlock> {
        vec![ForkBlock {
            seed_energy: 1000,
            observed_epoch: 0,
        }]
    }

    fn good_cert() -> EvaporatedForkCertV2 {
        // Threshold 600 > decayed 500 → evaporated.
        prove_fork_evaporated_v2([1u8; 32], &one_block(), lambda(), 100, 600, [9u8; 32], 0).unwrap()
    }

    // ── happy path ────────────────────────────────────────────────

    #[test]
    fn round_trip_verifies() {
        verify_evaporated_cert_v2(&good_cert()).unwrap();
    }

    // ── tampering rejected ────────────────────────────────────────

    #[test]
    fn tampered_witness_rejected() {
        let mut cert = good_cert();
        cert.witness[0] ^= 0xff;
        let err = verify_evaporated_cert_v2(&cert).unwrap_err();
        assert!(matches!(err, CertV2Error::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_anchor_rejected() {
        let mut cert = good_cert();
        cert.bell_seed_anchor[0] ^= 0xff;
        let err = verify_evaporated_cert_v2(&cert).unwrap_err();
        assert!(matches!(err, CertV2Error::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_anchor_epoch_rejected() {
        let mut cert = good_cert();
        // Lower anchor epoch — still ≤ eval epoch (no causality
        // violation), so the rejection comes from the witness.
        cert.seed_anchor_epoch = if cert.seed_anchor_epoch > 0 {
            cert.seed_anchor_epoch - 1
        } else {
            cert.seed_anchor_epoch + 1
        };
        // If we incremented, we may have crossed eval — verify path:
        if cert.seed_anchor_epoch > cert.evaluated_at_epoch {
            let err = verify_evaporated_cert_v2(&cert).unwrap_err();
            assert!(matches!(err, CertV2Error::CausalityViolation { .. }));
        } else {
            let err = verify_evaporated_cert_v2(&cert).unwrap_err();
            assert!(matches!(err, CertV2Error::WitnessMismatch { .. }));
        }
    }

    #[test]
    fn tampered_fork_root_rejected() {
        let mut cert = good_cert();
        cert.fork_root[0] ^= 0xff;
        let err = verify_evaporated_cert_v2(&cert).unwrap_err();
        assert!(matches!(err, CertV2Error::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_decayed_rejected() {
        let mut cert = good_cert();
        cert.decayed_energy -= 1;
        let err = verify_evaporated_cert_v2(&cert).unwrap_err();
        assert!(matches!(err, CertV2Error::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_threshold_rejected() {
        let mut cert = good_cert();
        cert.threshold += 1;
        let err = verify_evaporated_cert_v2(&cert).unwrap_err();
        assert!(matches!(err, CertV2Error::WitnessMismatch { .. }));
    }

    // ── semantic rejections ───────────────────────────────────────

    #[test]
    fn not_evaporated_rejected() {
        // Threshold below decayed — fork has not evaporated.
        let cert = prove_fork_evaporated_v2(
            [1u8; 32],
            &one_block(),
            lambda(),
            100,
            10, // threshold << decayed (500)
            [9u8; 32],
            0,
        )
        .unwrap();
        let err = verify_evaporated_cert_v2(&cert).unwrap_err();
        assert!(matches!(err, CertV2Error::NotEvaporated { .. }));
    }

    #[test]
    fn causality_violation_rejected_by_verifier() {
        // Hand-crafted cert with anchor_epoch > eval_epoch.
        // We can't construct via prover (it rejects), so build by
        // hand from a good cert and post-mutate.
        let mut cert = good_cert();
        cert.seed_anchor_epoch = cert.evaluated_at_epoch + 1;
        let err = verify_evaporated_cert_v2(&cert).unwrap_err();
        assert!(matches!(err, CertV2Error::CausalityViolation { .. }));
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Evaporated-Fork Certificate V2 binds each fork-cert
        // to a chain-supplied 32-byte seed anchor (typically a
        // BellCertificate.seed) plus its issuance epoch. A tactical
        // forker who knows the half-life cannot pre-compute the V2
        // witness because the anchor isn't published until the chain
        // confirms blocks at the eval epoch. Verifier rejects on
        // tampering of any field, on causality violation
        // (seed_anchor_epoch > evaluated_at_epoch), and on the
        // semantic check decayed_energy < threshold."

        // Round-trip a good cert.
        verify_evaporated_cert_v2(&good_cert()).unwrap();

        // Tampering vectors all rejected.
        for tampered in [
            {
                let mut c = good_cert();
                c.witness[0] ^= 1;
                c
            },
            {
                let mut c = good_cert();
                c.bell_seed_anchor[5] ^= 1;
                c
            },
            {
                let mut c = good_cert();
                c.fork_root[10] ^= 1;
                c
            },
            {
                let mut c = good_cert();
                c.decayed_energy += 1;
                c
            },
            {
                let mut c = good_cert();
                c.threshold = c.decayed_energy; // makes NotEvaporated trip
                c
            },
            {
                let mut c = good_cert();
                c.seed_anchor_epoch = c.evaluated_at_epoch + 1;
                c
            },
        ] {
            assert!(verify_evaporated_cert_v2(&tampered).is_err());
        }
    }
}
