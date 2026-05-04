//! `Streak` — the on-chain Singh-Streak primitive.
//!
//! Composes a decaying strength scalar in [0, 1] with an
//! `EngagementWindow`-style refresh model (boost on check-in; decay
//! between). Three structural decisions in shape:
//!
//! 1. **Capacity-based math.** The chain stores `cached_strength_bp`
//!    (basis points) + `anchor_epoch`. `strength_at(now)` decays per
//!    half-life. `check_in(boost_bp)` adds to the cached value
//!    (capped at FULL_STRENGTH_BP) and re-anchors to `now`. Integer-
//!    only on the on-chain path.
//!
//! 2. **Lifetime audit trail.** `total_check_ins` and
//!    `lifetime_witness_hash` (BLAKE3 chain over all witness payload
//!    hashes) provide a portable cognitive credential — "I have
//!    provably checked in 312 times across 4 years." Same moat shape
//!    as MnemoChain.
//!
//! 3. **Verifier-closure pattern.** `record(att, device_pubkey,
//!    is_signature_valid, boost_bp, now)` is the only state-changing
//!    op; it gates check-ins on a caller-supplied `is_signature_valid`
//!    closure so the chain's signature scheme can move independently.

use evaporchain_types::{energy_at_epoch, AccountAddress, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bucket::{bucket_for_strength, StrengthBucket};
use crate::witness::{WitnessAttestation, WitnessError};

pub type StreakId = [u8; 32];

/// Maximum strength (1.00 in basis points × 100). The cached
/// strength is capped here so a huge boost can't overshoot.
pub const FULL_STRENGTH_BP: u32 = 10_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StreakError {
    #[error("half_life must be > 0")]
    ZeroHalfLife,
    #[error("boost_bp must be > 0")]
    ZeroBoost,
    #[error("attestation epoch {att} is before anchor_epoch {anchor}")]
    BackwardsTime { att: Epoch, anchor: Epoch },
    #[error("witness: {0}")]
    Witness(#[from] WitnessError),
    #[error("device signature did not verify")]
    BadSignature,
    #[error("attestation streak_id does not match this streak")]
    WrongStreak,
}

/// On-chain streak record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Streak {
    pub id: StreakId,
    pub owner: AccountAddress,
    /// Half-life applied to `cached_strength_bp` per epoch.
    pub half_life: HalfLife,
    /// Strength in basis points (10_000 = 1.00) at `anchor_epoch`.
    pub cached_strength_bp: u32,
    pub anchor_epoch: Epoch,
    /// Lifetime audit trail.
    pub total_check_ins: u64,
    /// BLAKE3 chain over all witness payload_hashes ever recorded.
    /// Reproducible from chain history; portable cognitive credential.
    pub lifetime_witness_hash: [u8; 32],
}

impl Streak {
    /// Mint a new streak. Starts at zero strength — first check-in
    /// produces the initial boost.
    pub fn mint(
        id: StreakId,
        owner: AccountAddress,
        half_life: HalfLife,
        anchor_epoch: Epoch,
    ) -> Result<Self, StreakError> {
        if half_life == 0 {
            return Err(StreakError::ZeroHalfLife);
        }
        Ok(Self {
            id,
            owner,
            half_life,
            cached_strength_bp: 0,
            anchor_epoch,
            total_check_ins: 0,
            lifetime_witness_hash: [0u8; 32],
        })
    }

    /// Strength fraction in [0.0, 1.0] at `epoch_now`. Decays the
    /// cached strength via the standard half-life rule.
    pub fn strength_at(&self, epoch_now: Epoch) -> f64 {
        let elapsed = epoch_now.saturating_sub(self.anchor_epoch);
        let decayed_bp = energy_at_epoch(
            self.cached_strength_bp as u64,
            self.half_life,
            elapsed,
        );
        (decayed_bp as f64 / FULL_STRENGTH_BP as f64).clamp(0.0, 1.0)
    }

    /// Strength in basis points at `epoch_now` — integer counterpart.
    pub fn strength_bp_at(&self, epoch_now: Epoch) -> u32 {
        let elapsed = epoch_now.saturating_sub(self.anchor_epoch);
        let decayed = energy_at_epoch(
            self.cached_strength_bp as u64,
            self.half_life,
            elapsed,
        );
        decayed.min(FULL_STRENGTH_BP as u64) as u32
    }

    /// Qualitative bucket at `epoch_now`.
    pub fn bucket_at(&self, epoch_now: Epoch) -> StrengthBucket {
        bucket_for_strength(self.strength_at(epoch_now))
    }

    /// Record a witness attestation. Decays strength to the
    /// attestation's epoch, adds `boost_bp` (capped at FULL_STRENGTH_BP),
    /// re-anchors. Verifier closure plugs in the signature scheme.
    pub fn record<F>(
        &mut self,
        att: WitnessAttestation,
        boost_bp: u32,
        is_signature_valid: F,
    ) -> Result<(), StreakError>
    where
        F: Fn(&AccountAddress, &[u8], &[u8]) -> bool,
    {
        if boost_bp == 0 {
            return Err(StreakError::ZeroBoost);
        }
        if att.streak_id != self.id {
            return Err(StreakError::WrongStreak);
        }
        if att.epoch < self.anchor_epoch {
            return Err(StreakError::BackwardsTime {
                att: att.epoch,
                anchor: self.anchor_epoch,
            });
        }
        let signing = att.signing_bytes();
        if !is_signature_valid(&self.owner, &signing, &att.signature) {
            return Err(StreakError::BadSignature);
        }
        // Decay current to attestation epoch, then boost.
        let now_bp = self.strength_bp_at(att.epoch);
        let new_bp = now_bp
            .saturating_add(boost_bp)
            .min(FULL_STRENGTH_BP);
        self.cached_strength_bp = new_bp;
        self.anchor_epoch = att.epoch;
        self.total_check_ins = self.total_check_ins.saturating_add(1);
        self.lifetime_witness_hash =
            chain_hash(&self.lifetime_witness_hash, &att.payload_hash);
        Ok(())
    }
}

/// BLAKE3 chain step: H(prev ∥ next ∥ "witnessfit:chain:v1").
fn chain_hash(prev: &[u8; 32], next: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(prev);
    hasher.update(next);
    hasher.update(b"witnessfit:chain:v1");
    let mut out = [0u8; 32];
    out.copy_from_slice(hasher.finalize().as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> StreakId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn always_valid(_a: &AccountAddress, _b: &[u8], _s: &[u8]) -> bool {
        true
    }

    fn att(streak: u8, epoch: Epoch, payload: u8) -> WitnessAttestation {
        WitnessAttestation::new(id(streak), epoch, [payload; 32], vec![payload; 8]).unwrap()
    }

    #[test]
    fn mint_rejects_zero_half_life() {
        let err = Streak::mint(id(1), addr(0xAA), 0, 0).unwrap_err();
        assert_eq!(err, StreakError::ZeroHalfLife);
    }

    #[test]
    fn newly_minted_is_broken() {
        // New streak starts at zero strength — Broken bucket.
        let s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        assert_eq!(s.strength_bp_at(0), 0);
        assert_eq!(s.bucket_at(0), StrengthBucket::Broken);
    }

    #[test]
    fn check_in_boosts_strength() {
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        s.record(att(1, 0, 0xAB), FULL_STRENGTH_BP, always_valid).unwrap();
        assert_eq!(s.strength_bp_at(0), FULL_STRENGTH_BP);
        assert_eq!(s.bucket_at(0), StrengthBucket::Healthy);
        assert_eq!(s.total_check_ins, 1);
    }

    #[test]
    fn strength_decays_between_check_ins() {
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        s.record(att(1, 0, 0xAB), FULL_STRENGTH_BP, always_valid).unwrap();
        // After one half-life, strength ≈ 5000 bp = 0.5.
        let mid = s.strength_at(100);
        assert!(mid < 1.0 && mid > 0.0);
        assert_eq!(s.bucket_at(100), StrengthBucket::Aging);
        // After many half-lives, strength → 0.
        let later = s.strength_at(10_000);
        assert_eq!(later, 0.0);
        assert_eq!(s.bucket_at(10_000), StrengthBucket::Broken);
    }

    #[test]
    fn graceful_decay_not_cliff_failure() {
        // Doctrine claim: existing streak apps cliff to zero on miss;
        // Singh-Streak fades. Build up to full, miss a "day", verify
        // strength is degraded but NOT zero.
        let mut s = Streak::mint(id(1), addr(0xAA), 1000, 0).unwrap();
        s.record(att(1, 0, 0xAB), FULL_STRENGTH_BP, always_valid).unwrap();
        // Miss a single epoch (much less than half-life). Strength
        // should be slightly degraded but well above Broken.
        let after_miss = s.strength_at(1);
        assert!(after_miss > 0.95);
        assert_eq!(s.bucket_at(1), StrengthBucket::Healthy);
    }

    #[test]
    fn check_in_boost_caps_at_full_strength() {
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        s.record(att(1, 0, 0xAB), FULL_STRENGTH_BP, always_valid).unwrap();
        // Boost again at the same epoch — already full, can't go higher.
        s.record(att(1, 0, 0xCD), FULL_STRENGTH_BP, always_valid).unwrap();
        assert_eq!(s.cached_strength_bp, FULL_STRENGTH_BP);
        assert_eq!(s.total_check_ins, 2);
    }

    #[test]
    fn check_in_after_decay_recovers_streak() {
        // Doctrine claim: "missed 2 days, you're at maybe 0.85, single
        // check-in puts you back near full." Build up, decay a bit,
        // check in again, verify recovery.
        let mut s = Streak::mint(id(1), addr(0xAA), 1000, 0).unwrap();
        s.record(att(1, 0, 0xAB), FULL_STRENGTH_BP, always_valid).unwrap();
        // After 2 epochs of silence (much less than half-life=1000):
        let pre = s.strength_at(2);
        assert!(pre < 1.0);
        // Check in again at epoch 2 with a smaller boost (e.g. 5000bp).
        s.record(att(1, 2, 0xCD), 5000, always_valid).unwrap();
        // Strength is back near full.
        let post = s.strength_at(2);
        assert!(post >= pre);
        assert_eq!(s.bucket_at(2), StrengthBucket::Healthy);
    }

    #[test]
    fn wrong_streak_id_rejected() {
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        let err = s
            .record(att(99, 0, 0xAB), FULL_STRENGTH_BP, always_valid)
            .unwrap_err();
        assert_eq!(err, StreakError::WrongStreak);
    }

    #[test]
    fn bad_signature_rejected() {
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        let always_invalid = |_a: &AccountAddress, _b: &[u8], _s: &[u8]| false;
        let err = s
            .record(att(1, 0, 0xAB), FULL_STRENGTH_BP, always_invalid)
            .unwrap_err();
        assert_eq!(err, StreakError::BadSignature);
    }

    #[test]
    fn backwards_time_rejected() {
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 100).unwrap();
        // Anchor at 100; attestation at 50 — invalid.
        let err = s
            .record(att(1, 50, 0xAB), FULL_STRENGTH_BP, always_valid)
            .unwrap_err();
        assert!(matches!(err, StreakError::BackwardsTime { .. }));
    }

    #[test]
    fn zero_boost_rejected() {
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        let err = s.record(att(1, 0, 0xAB), 0, always_valid).unwrap_err();
        assert_eq!(err, StreakError::ZeroBoost);
    }

    #[test]
    fn lifetime_witness_hash_chains_deterministically() {
        let mut a = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        let mut b = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        // Same sequence of attestations ⇒ same lifetime hash.
        for (epoch, payload) in [(0u64, 0x11u8), (1, 0x22), (2, 0x33)] {
            a.record(att(1, epoch, payload), 100, always_valid).unwrap();
            b.record(att(1, epoch, payload), 100, always_valid).unwrap();
        }
        assert_eq!(a.lifetime_witness_hash, b.lifetime_witness_hash);
        assert_ne!(a.lifetime_witness_hash, [0; 32]);
    }

    #[test]
    fn portable_credential_audit_trail_is_attestable() {
        // Doctrine moat: cards become portable cognitive credentials —
        // here, the audit trail. After N check-ins over T epochs, the
        // streak record is a verifiable claim.
        let mut s = Streak::mint(id(1), addr(0xAA), 1000, 0).unwrap();
        for i in 0..10u64 {
            // Offset payload byte by 1 so the first attestation isn't
            // payload=0 (rejected by WitnessAttestation::new).
            s.record(att(1, i, (i + 1) as u8), 1000, always_valid).unwrap();
        }
        assert_eq!(s.total_check_ins, 10);
        assert_ne!(s.lifetime_witness_hash, [0; 32]);
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Doctrine: the moat is decay-as-streak-anchor. Existing apps
        // cliff to zero on a single miss; Singh-Streak fades. Single
        // check-in restores it. Operationalised:
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        // Build up to full strength.
        s.record(att(1, 0, 0xAB), FULL_STRENGTH_BP, always_valid).unwrap();
        assert_eq!(s.bucket_at(0), StrengthBucket::Healthy);
        // Miss "two days" — strength degrades but is NOT zero (no
        // cliff). With half_life=100 and 2 epochs elapsed, strength
        // is ~0.99, still Healthy.
        let after_two = s.strength_at(2);
        assert!(after_two > 0.95);
        assert_eq!(s.bucket_at(2), StrengthBucket::Healthy);
        // The user *can* let the streak fade if they keep missing —
        // after many half-lives, it eventually breaks.
        assert_eq!(s.bucket_at(10_000), StrengthBucket::Broken);
    }

    #[test]
    fn round_trip_serde_streak() {
        let mut s = Streak::mint(id(7), addr(0xAB), 365, 42).unwrap();
        s.record(att(7, 50, 0xAB), 5000, always_valid).unwrap();
        let j = serde_json::to_string(&s).unwrap();
        let back: Streak = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn always_valid(_a: &AccountAddress, _b: &[u8], _s: &[u8]) -> bool {
        true
    }

    proptest! {
        /// Strength is monotone non-increasing between check-ins.
        #[test]
        fn strength_monotone_between_check_ins(
            half_life in 10u64..10_000,
            t_a in 0u64..1_000_000,
            extra in 0u64..1_000_000,
        ) {
            let mut id = [0u8; 32];
            id[0] = 1;
            let mut s = Streak::mint(id, [0xAA; 32], half_life, 0).unwrap();
            // Boost once to FULL.
            let att = WitnessAttestation::new(id, 0, [0xAB; 32], vec![1]).unwrap();
            s.record(att, FULL_STRENGTH_BP, always_valid).unwrap();
            let a = s.strength_at(t_a);
            let b = s.strength_at(t_a.saturating_add(extra));
            prop_assert!(b <= a, "strength went up: {a} → {b}");
        }

        /// Strength always in [0.0, 1.0].
        #[test]
        fn strength_in_unit_range(
            half_life in 10u64..10_000,
            cached in 0u32..(FULL_STRENGTH_BP * 2), // even oversized cached values clamp
            anchor in 0u64..10_000,
            now in 0u64..2_000_000,
        ) {
            let mut id = [0u8; 32];
            id[0] = 1;
            let mut s = Streak::mint(id, [0xAA; 32], half_life, anchor).unwrap();
            s.cached_strength_bp = cached;
            let v = s.strength_at(now);
            prop_assert!((0.0..=1.0).contains(&v));
        }
    }
}
