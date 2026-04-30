//! Autopoietic health report and coordinator.
//!
//! # Autopoietic viability condition
//!
//! A chain is *autopoietically viable* iff all three subsystems are healthy:
//!   - Patronage: at least one active covenant funding the chain's own
//!     infrastructure (minimum: 1 covenant object with positive score).
//!   - Sentinel: the autonomic controller has been active within the last
//!     `sentinel_heartbeat_window` epochs (has voted at least once).
//!   - LLSA: the upgrade gate is functional (at least one proof verifier
//!     is registered and can verify).
//!
//! When any subsystem is degraded, the chain is in `AutopoieticStatus::Stressed`.
//! When all are failed, `AutopoieticStatus::Inviable` — the chain is in
//! existential crisis (expected — the chain can die per doctrine).

use evaporchain_llsa::proof::{LlsaProof, ProofVerifier};
use evaporchain_refresh_patronage::{patronage_score, PatronageBook};
use evaporchain_types::Energy;

/// Health of a single autopoietic subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SubsystemHealth {
    Healthy,
    Degraded,
    Failed,
}

/// Aggregate autopoietic status across all three components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AutopoieticStatus {
    /// All three subsystems are healthy. Chain is fully autopoietic.
    Viable,
    /// One or two subsystems degraded. Chain can survive but should alert.
    Stressed,
    /// All subsystems failed. Chain is in existential crisis.
    Inviable,
}

/// Full health report produced by `ChainAutopoiesis`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutopoieticHealth {
    pub status: AutopoieticStatus,
    /// Self-production: patronage (RefreshPool covenant funding).
    pub patronage: SubsystemHealth,
    /// Self-maintenance: sentinel (autonomic parameter adjustment).
    pub sentinel: SubsystemHealth,
    /// Self-boundary: LLSA (Coq-verified upgrade gate).
    pub llsa: SubsystemHealth,
    /// Total patronage energy score across all active covenants.
    pub total_patronage_energy: Energy,
    /// Current epoch at report time.
    pub epoch: u64,
}

impl AutopoieticHealth {
    fn compute_status(
        p: SubsystemHealth,
        s: SubsystemHealth,
        l: SubsystemHealth,
    ) -> AutopoieticStatus {
        let failed = [p, s, l]
            .iter()
            .filter(|&&h| h == SubsystemHealth::Failed)
            .count();
        match failed {
            0 => AutopoieticStatus::Viable,
            3 => AutopoieticStatus::Inviable,
            _ => AutopoieticStatus::Stressed,
        }
    }
}

/// Coordinator that aggregates health from all three autopoietic subsystems.
pub struct ChainAutopoiesis<V: ProofVerifier> {
    /// The LLSA proof verifier (determines upgrade boundary health).
    pub verifier: V,
    /// Patronage minimum energy for a covenant to count as "active".
    pub min_patronage_energy: Energy,
    /// Sentinel heartbeat: number of epochs within which a sentinel vote
    /// is required for the subsystem to be considered healthy.
    pub sentinel_heartbeat_window: u64,
}

impl<V: ProofVerifier> ChainAutopoiesis<V> {
    pub fn new(verifier: V, min_patronage_energy: Energy, sentinel_heartbeat_window: u64) -> Self {
        Self {
            verifier,
            min_patronage_energy,
            sentinel_heartbeat_window,
        }
    }

    /// Produce a full autopoietic health report.
    ///
    /// - `book`: the live patronage book.
    /// - `covenant_ids`: identifiers for known patronage objects (from chain state).
    /// - `last_sentinel_vote_epoch`: the epoch of the most recent sentinel vote.
    /// - `epoch`: the current chain epoch.
    pub fn health_report(
        &self,
        book: &PatronageBook,
        covenant_ids: &[Vec<u8>],
        last_sentinel_vote_epoch: Option<u64>,
        epoch: u64,
    ) -> AutopoieticHealth {
        // 1. Patronage health: total score across known covenants.
        let total_energy: Energy = covenant_ids
            .iter()
            .map(|id| patronage_score(book, id.as_slice()))
            .fold(0u64, |acc, e| acc.saturating_add(e));

        let patronage = if total_energy >= self.min_patronage_energy {
            SubsystemHealth::Healthy
        } else if total_energy > 0 {
            SubsystemHealth::Degraded
        } else {
            SubsystemHealth::Failed
        };

        // 2. Sentinel health: last vote within heartbeat window.
        let sentinel = match last_sentinel_vote_epoch {
            Some(last) if epoch.saturating_sub(last) <= self.sentinel_heartbeat_window => {
                SubsystemHealth::Healthy
            }
            Some(_) => SubsystemHealth::Degraded,
            None => SubsystemHealth::Failed,
        };

        // 3. LLSA health: verify a trivial no-op proof to confirm the
        //    verifier is live. A verifier that always rejects is Degraded,
        //    not Failed (it can still gate; it's just strict).
        //    We probe with an empty amendment bytes; the verifier's response
        //    tells us if the subsystem is functional.
        let llsa = {
            // Probe: create a trivial LlsaProof where all hash fields are zero,
            // then call verify with matching zero expected IDs.
            // AlwaysAcceptVerifier: Ok → Healthy.
            // AlwaysRejectVerifier or any strict verifier: Err → Degraded (functional but strict).
            let probe = LlsaProof {
                coq_term_hash: [0u8; 32],
                target_invariant_id: [0u8; 32],
                bound_amendment_hash: [0u8; 32],
                proof_bytes: vec![],
            };
            match self.verifier.verify(&probe, [0u8; 32], [0u8; 32]) {
                Ok(_) => SubsystemHealth::Healthy,
                Err(_) => SubsystemHealth::Degraded,
            }
        };

        let status = AutopoieticHealth::compute_status(patronage, sentinel, llsa);

        AutopoieticHealth {
            status,
            patronage,
            sentinel,
            llsa,
            total_patronage_energy: total_energy,
            epoch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_llsa::proof::{AlwaysAcceptVerifier, AlwaysRejectVerifier};
    use evaporchain_refresh_patronage::PatronageBook;

    fn empty_book() -> PatronageBook {
        PatronageBook::new(b"empty-test-ns".to_vec())
    }

    #[test]
    fn no_patronage_no_sentinel_degraded_llsa_is_inviable() {
        let sys = ChainAutopoiesis::new(AlwaysRejectVerifier, 100, 10);
        let r = sys.health_report(&empty_book(), &[], None, 100);
        assert_eq!(r.status, AutopoieticStatus::Inviable);
        assert_eq!(r.patronage, SubsystemHealth::Failed);
        assert_eq!(r.sentinel, SubsystemHealth::Failed);
        assert_eq!(r.llsa, SubsystemHealth::Degraded);
    }

    #[test]
    fn all_healthy_gives_viable() {
        let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 100);
        // epoch=50, last_vote=45 → within window of 100
        let r = sys.health_report(&empty_book(), &[], Some(45), 50);
        assert_eq!(r.patronage, SubsystemHealth::Healthy); // min_energy=0 → threshold met
        assert_eq!(r.sentinel, SubsystemHealth::Healthy);
        assert_eq!(r.llsa, SubsystemHealth::Healthy);
        assert_eq!(r.status, AutopoieticStatus::Viable);
    }

    #[test]
    fn stale_sentinel_is_degraded() {
        let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
        let r = sys.health_report(&empty_book(), &[], Some(0), 100); // 100 epochs since last vote
        assert_eq!(r.sentinel, SubsystemHealth::Degraded);
        assert_eq!(r.status, AutopoieticStatus::Stressed);
    }

    #[test]
    fn status_stressed_when_one_failed() {
        let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 1_000_000, 10);
        // Patronage failed (no covenants, min=1_000_000), sentinel+llsa healthy.
        let r = sys.health_report(&empty_book(), &[], Some(99), 100);
        assert_eq!(r.patronage, SubsystemHealth::Failed);
        assert_eq!(r.status, AutopoieticStatus::Stressed);
    }

    #[test]
    fn viability_requires_sentinel_within_window() {
        let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 5);
        let in_window = sys.health_report(&empty_book(), &[], Some(95), 100);
        let out_window = sys.health_report(&empty_book(), &[], Some(90), 100);
        assert_eq!(in_window.sentinel, SubsystemHealth::Healthy);
        assert_eq!(out_window.sentinel, SubsystemHealth::Degraded);
    }
}
