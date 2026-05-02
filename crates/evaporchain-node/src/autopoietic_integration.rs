// Pre-wired API: this module is intentionally kept exported but not yet
// invoked from main.rs. Once the consensus loop calls `NodeAutopoiesis::check`
// (per INVENTION_STACK.md §A4.3.8) the dead-code allowance can be lifted.
#![allow(dead_code)]

//! Autopoietic health integration — wires `evaporchain-autopoietic` into
//! the node boot and block-production loop.
//!
//! Per INVENTION_STACK.md §A4.3.8 the chain is autopoietically viable iff
//! all three subsystems are healthy:
//!   - Patronage  — RefreshPool covenants funding chain maintenance.
//!   - Sentinel   — autonomic controller voted within heartbeat window.
//!   - LLSA       — Coq-verified upgrade gate is functional.
//!
//! # Integration
//!
//! `NodeAutopoiesis::check(book, covenant_ids, last_sentinel_vote, epoch)`
//! produces a full `AutopoieticHealth` report.  The node calls this once per
//! epoch boundary (same gate as `collect_storage_rent`) and emits a warning
//! if the chain is `Stressed` or `Inviable`.
//!
//! At genesis a `NullVerifier` is used — it always returns `Ok(())` so LLSA
//! reports Healthy until a real Coq verifier is registered via governance.

use evaporchain_autopoietic::autopoiesis::{
    AutopoieticHealth, AutopoieticStatus, ChainAutopoiesis, SubsystemHealth,
};
use evaporchain_llsa::proof::ProofVerifier;
use evaporchain_refresh_patronage::PatronageBook;
use tracing::{debug, info, warn};

/// Minimum patronage energy for the chain to be considered self-funded.
/// Genesis placeholder: 1 unit means the chain is nominally funded as long
/// as at least one covenant with any positive score exists.
pub const MIN_PATRONAGE_ENERGY: u64 = 1;

/// Sentinel heartbeat window: 10 epochs.  If no sentinel vote arrives
/// within 10 epochs the subsystem is marked Degraded.
pub const SENTINEL_HEARTBEAT_WINDOW: u64 = 10;

/// Null proof verifier for genesis: accepts all proofs.
/// Replaced by governance when the real Coq verifier is deployed.
pub struct NullVerifier;

impl ProofVerifier for NullVerifier {
    fn verify(
        &self,
        _proof: &evaporchain_llsa::proof::LlsaProof,
        _expected_invariant: evaporchain_llsa::proof::InvariantId,
        _expected_amendment: evaporchain_llsa::proof::AmendmentHash,
    ) -> Result<(), evaporchain_llsa::proof::ProofError> {
        Ok(())
    }
}

/// Node-level autopoietic coordinator using the `NullVerifier`.
pub struct NodeAutopoiesis {
    inner: ChainAutopoiesis<NullVerifier>,
}

impl NodeAutopoiesis {
    pub fn new() -> Self {
        Self {
            inner: ChainAutopoiesis::new(
                NullVerifier,
                MIN_PATRONAGE_ENERGY,
                SENTINEL_HEARTBEAT_WINDOW,
            ),
        }
    }

    /// Run the full autopoietic health check and log the result.
    pub fn check(
        &self,
        book: &PatronageBook,
        covenant_ids: &[Vec<u8>],
        last_sentinel_vote_epoch: Option<u64>,
        epoch: u64,
    ) -> AutopoieticHealth {
        let report = self
            .inner
            .health_report(book, covenant_ids, last_sentinel_vote_epoch, epoch);
        log_health_report(&report);
        report
    }
}

impl Default for NodeAutopoiesis {
    fn default() -> Self {
        Self::new()
    }
}

/// Log the autopoietic health report at the appropriate tracing level.
pub fn log_health_report(report: &AutopoieticHealth) {
    let patronage = subsystem_str(report.patronage);
    let sentinel = subsystem_str(report.sentinel);
    let llsa = subsystem_str(report.llsa);

    match report.status {
        AutopoieticStatus::Viable => debug!(
            epoch = report.epoch,
            patronage,
            sentinel,
            llsa,
            total_patronage = report.total_patronage_energy,
            "autopoietic health: Viable"
        ),
        AutopoieticStatus::Stressed => warn!(
            epoch = report.epoch,
            patronage,
            sentinel,
            llsa,
            total_patronage = report.total_patronage_energy,
            "autopoietic health: Stressed — one or more subsystems degraded"
        ),
        AutopoieticStatus::Inviable => warn!(
            epoch = report.epoch,
            patronage, sentinel, llsa, "autopoietic health: Inviable — all subsystems failed"
        ),
    }
}

fn subsystem_str(h: SubsystemHealth) -> &'static str {
    match h {
        SubsystemHealth::Healthy => "healthy",
        SubsystemHealth::Degraded => "degraded",
        SubsystemHealth::Failed => "failed",
    }
}

/// One-shot genesis health check: logs the initial autopoietic state.
/// Called during node startup before the first block is produced.
pub fn genesis_health_check(epoch: u64) -> AutopoieticHealth {
    let node = NodeAutopoiesis::new();
    let book = PatronageBook::new(b"evaporchain-patronage".to_vec());
    let report = node.check(&book, &[], None, epoch);
    info!(
        status = ?report.status,
        epoch,
        "genesis autopoietic health check complete"
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_health_is_stressed_no_patronage() {
        let report = genesis_health_check(0);
        // No covenants at genesis → patronage Failed → Stressed.
        assert_eq!(report.status, AutopoieticStatus::Stressed);
        assert_eq!(report.patronage, SubsystemHealth::Failed);
    }

    #[test]
    fn null_verifier_gives_healthy_llsa() {
        let report = genesis_health_check(0);
        assert_eq!(report.llsa, SubsystemHealth::Healthy);
    }

    #[test]
    fn no_sentinel_gives_failed_sentinel() {
        let report = genesis_health_check(0);
        assert_eq!(report.sentinel, SubsystemHealth::Failed);
    }

    #[test]
    fn recent_sentinel_vote_gives_healthy_sentinel() {
        let node = NodeAutopoiesis::new();
        let book = PatronageBook::new(b"evaporchain-patronage".to_vec());
        let report = node.check(&book, &[], Some(5), 10); // 10 - 5 = 5 ≤ window(10)
        assert_eq!(report.sentinel, SubsystemHealth::Healthy);
    }

    #[test]
    fn stale_sentinel_vote_gives_degraded_sentinel() {
        let node = NodeAutopoiesis::new();
        let book = PatronageBook::new(b"evaporchain-patronage".to_vec());
        let report = node.check(&book, &[], Some(0), 100); // 100 - 0 = 100 > window(10)
        assert_eq!(report.sentinel, SubsystemHealth::Degraded);
    }
}
