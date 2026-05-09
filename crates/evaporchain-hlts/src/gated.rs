//! Survival-gated reconstruction — composes Layer 1 (energy quorum)
//! with Layer 2 (Shamir Lagrange interpolation).
//!
//! Reconstruction succeeds iff:
//!   1. The chain's energy-survival check ([`crate::quorum_alive`])
//!      reports at least `k` of the `n` registered shares are still
//!      above the chain-set survival threshold at the query epoch.
//!   2. The supplied [`SecretShare`]s — keyed by `idx` matching the
//!      registered [`Share`]s — Lagrange-interpolate to a valid
//!      [`Secret`] (per [`crate::reconstruct`]).
//!
//! This is the API the chain exposes to dApps building on HLTS:
//! "give me the secret IFF the on-chain energy state allows it AND
//! the off-chain shares are valid." Both checks are necessary;
//! either alone is insufficient (energy without shares = no
//! reconstruction; shares without energy = forbidden by the half-
//! life rule).
//!
//! The two layers are **independently composable** so a dApp that
//! wants to bypass either (e.g., test fixtures that skip survival)
//! can reach for [`crate::quorum_alive`] / [`crate::reconstruct`]
//! directly.

use thiserror::Error;

use evaporchain_energy_kernel::ChainLambda;
use evaporchain_types::Energy;

use crate::quorum::quorum_alive;
use crate::secret::{reconstruct, HltsError, Secret, SecretShare};
use crate::share::Share;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReconstructionError {
    /// The on-chain energy quorum is below threshold — fewer than k
    /// shares are still alive. The secret is permanently lost from
    /// this point unless a chain governance action restores energy.
    #[error(
        "energy quorum lost: fewer than k={k} shares above threshold {threshold} \
         at epoch {query_epoch} (alive count: {alive})"
    )]
    QuorumLost {
        k: usize,
        alive: usize,
        threshold: Energy,
        query_epoch: u64,
    },
    /// The supplied secret shares failed Lagrange reconstruction
    /// (insufficient count, duplicate index, or other shape error).
    /// Wraps the underlying [`HltsError`].
    #[error("share reconstruction failed: {0}")]
    BadShares(#[from] HltsError),
}

/// Reconstruct the secret iff the chain-side energy quorum is met.
///
/// `chain_shares` are the on-chain energy-bookkeeping records (one
/// per registered share-holder). `secret_shares` are the actual
/// cryptographic shares, supplied off-chain by the share-holders
/// for this reconstruction. They're matched by `idx`.
///
/// Behavior:
/// - If quorum fails: return [`ReconstructionError::QuorumLost`].
///   Lagrange is NOT attempted — the secret is gone.
/// - If quorum passes but Lagrange fails (insufficient secret
///   shares, duplicate indices, etc.): return
///   [`ReconstructionError::BadShares`].
/// - If both pass: return the recovered [`Secret`].
pub fn reconstruct_if_alive(
    chain_shares: &[Share],
    secret_shares: &[SecretShare],
    k: usize,
    chain_lambda: ChainLambda,
    query_epoch: u64,
    threshold: Energy,
) -> Result<Secret, ReconstructionError> {
    // Layer 1: energy quorum gate.
    let alive = crate::survival::count_alive(chain_shares, chain_lambda, query_epoch, threshold);
    if !quorum_alive(chain_shares, k, chain_lambda, query_epoch, threshold) {
        return Err(ReconstructionError::QuorumLost {
            k,
            alive,
            threshold,
            query_epoch,
        });
    }
    // Layer 2: Lagrange reconstruction from the supplied secret
    // shares. The chain doesn't dictate WHICH secret shares the
    // dApp uses — any k of them work; the energy gate just
    // confirms that the on-chain state hasn't lost the quorum.
    let secret = reconstruct(secret_shares, k)?;
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::{deal, DealRng};
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    fn rng_for(tag: &str) -> DealRng {
        let mut seed = [0u8; 32];
        let bytes = tag.as_bytes();
        seed[..bytes.len().min(32)].copy_from_slice(&bytes[..bytes.len().min(32)]);
        DealRng::from_seed(seed)
    }

    /// Build matched chain_shares + secret_shares from a single deal.
    /// All shares get the same energy + observed_epoch by default;
    /// individual tests adjust to simulate decay scenarios.
    fn deal_and_match(
        secret_value: u64,
        n: usize,
        k: usize,
        energy: Energy,
        observed_epoch: u64,
        tag: &str,
    ) -> (Vec<Share>, Vec<SecretShare>) {
        let mut rng = rng_for(tag);
        let secret = Secret::from_u64(secret_value);
        let secret_shares = deal(secret, n, k, &mut rng).unwrap();
        let chain_shares: Vec<Share> = secret_shares
            .iter()
            .map(|ss| Share::new(ss.idx, energy, observed_epoch))
            .collect();
        (chain_shares, secret_shares)
    }

    #[test]
    fn fresh_shares_reconstruct_cleanly() {
        let (chain, shares) = deal_and_match(0xCAFE, 5, 3, 1000, 0, "fresh");
        let secret = reconstruct_if_alive(&chain, &shares, 3, lambda(), 0, 1).unwrap();
        assert_eq!(secret.to_u64(), 0xCAFE);
    }

    #[test]
    fn quorum_lost_returns_error_without_attempting_lagrange() {
        // Set energy low + threshold high so the quorum gate fails.
        let (chain, shares) = deal_and_match(0xCAFE, 5, 3, 100, 0, "quorum-lost");
        let err = reconstruct_if_alive(
            &chain, &shares, 3, lambda(), 0, /* threshold */ 200,
        )
        .expect_err("quorum below threshold must reject");
        match err {
            ReconstructionError::QuorumLost {
                k,
                alive,
                threshold,
                ..
            } => {
                assert_eq!(k, 3);
                assert_eq!(alive, 0);
                assert_eq!(threshold, 200);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn quorum_met_but_bad_shares_returns_lagrange_error() {
        // Quorum is fine; supply only 2 of 3 required shares.
        let (chain, shares) = deal_and_match(0xCAFE, 5, 3, 1000, 0, "bad-shares");
        let err = reconstruct_if_alive(&chain, &shares[..2], 3, lambda(), 0, 1)
            .expect_err("k=3 but 2 shares must reject");
        match err {
            ReconstructionError::BadShares(HltsError::InsufficientShares { have, k }) => {
                assert_eq!(have, 2);
                assert_eq!(k, 3);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn decay_to_below_threshold_eventually_locks_secret() {
        // 5 shares with energy 1000, half-life 100 epochs (default
        // ChainLambda above). At epoch 300 (3 half-lives), each share's
        // energy is ~125 (1000 / 8). Threshold 200 → all dead → quorum
        // lost.
        let (chain, shares) = deal_and_match(0xDEAD, 5, 3, 1000, 0, "decay");
        // Fresh: works.
        assert!(reconstruct_if_alive(&chain, &shares, 3, lambda(), 0, 200).is_ok());
        // Aged 300 epochs: locked.
        let err = reconstruct_if_alive(&chain, &shares, 3, lambda(), 300, 200)
            .expect_err("aged shares below threshold must reject");
        assert!(matches!(err, ReconstructionError::QuorumLost { .. }));
    }

    #[test]
    fn sparse_alive_quorum_with_specific_shares_still_works() {
        // 5 chain shares with mixed energies — only 3 alive at threshold
        // 500. Reconstruction with the 3 alive shares (the only ones
        // the share-holders bothered to keep) works.
        let mut rng = rng_for("sparse");
        let secret = Secret::from_u64(0xBEEF);
        let secret_shares = deal(secret, 5, 3, &mut rng).unwrap();
        let chain_shares = vec![
            Share::new(1, 1000, 0), // alive
            Share::new(2, 100, 0),  // dead
            Share::new(3, 1000, 0), // alive
            Share::new(4, 50, 0),   // dead
            Share::new(5, 1000, 0), // alive
        ];
        // Quorum: 3 of 5 above threshold 500 → quorum met.
        // Provide the 3 alive shares (idx 1, 3, 5).
        let alive_secret = vec![
            secret_shares[0],
            secret_shares[2],
            secret_shares[4],
        ];
        let recovered = reconstruct_if_alive(&chain_shares, &alive_secret, 3, lambda(), 0, 500)
            .unwrap();
        assert_eq!(recovered.to_u64(), 0xBEEF);
    }
}
