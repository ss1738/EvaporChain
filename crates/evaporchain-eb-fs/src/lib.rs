//! Energy-Bound Fiat-Shamir (EB-FS).
//!
//! Per `research/INVENTION_STACK.md` §4.2 (Tier 2):
//!
//! > **Energy-Bound Fiat-Shamir (EB-FS)** — One-line transcript
//! > change; stops cross-fork proof replay.
//!
//! Standard Fiat-Shamir: `challenge = H(transcript)`. EB-FS:
//! `challenge = H(transcript || epoch_energy)`. Binding the chain's
//! per-epoch aggregate energy into the transcript means a proof
//! generated on fork A cannot be replayed on fork B — the two forks'
//! epoch-energy budgets differ, so the challenge differs, so the
//! proof's response no longer satisfies the verifier's check.
//!
//! Substrate exposes the wrapper. Real ZK-proof systems plug their
//! transcript bytes through `eb_fs_challenge` instead of the bare
//! Fiat-Shamir hash.

use evaporchain_types::Energy;

const EB_FS_TAG: &[u8] = b"evaporchain-eb-fs";

/// `EB-FS challenge = blake3("evaporchain-eb-fs" || transcript ||
/// epoch || epoch_energy)`. Returns 32 bytes; consumers reduce mod
/// their proof system's challenge field.
pub fn eb_fs_challenge(transcript: &[u8], epoch: u64, epoch_energy: Energy) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(EB_FS_TAG);
    h.update(transcript);
    h.update(&epoch.to_le_bytes());
    h.update(&epoch_energy.to_le_bytes());
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_under_same_inputs() {
        let c1 = eb_fs_challenge(b"hello", 5, 1_000);
        let c2 = eb_fs_challenge(b"hello", 5, 1_000);
        assert_eq!(c1, c2);
    }

    #[test]
    fn different_transcript_different_challenge() {
        let c1 = eb_fs_challenge(b"hello", 5, 1_000);
        let c2 = eb_fs_challenge(b"world", 5, 1_000);
        assert_ne!(c1, c2);
    }

    #[test]
    fn different_epoch_different_challenge() {
        let c1 = eb_fs_challenge(b"x", 5, 1_000);
        let c2 = eb_fs_challenge(b"x", 6, 1_000);
        assert_ne!(c1, c2);
    }

    #[test]
    fn different_epoch_energy_different_challenge() {
        // The cross-fork-replay defence: two forks at the same epoch
        // with different aggregate energy → different challenge →
        // proofs cannot replay across forks.
        let c1 = eb_fs_challenge(b"x", 5, 1_000);
        let c2 = eb_fs_challenge(b"x", 5, 1_001);
        assert_ne!(c1, c2);
    }

    #[test]
    fn empty_transcript_well_defined() {
        let _ = eb_fs_challenge(b"", 0, 0);
    }
}
