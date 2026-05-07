//! `SplitLegacy` — multi-recipient Legacy edge with weighted shares.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SplitId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SplitError {
    #[error("split spec is empty — at least one recipient required")]
    EmptySplit,
    #[error("split shares sum to {sum}bp but must sum to exactly 10_000bp (= 100%)")]
    SplitDoesNotSumToFull { sum: u64 },
    #[error("zero share for recipient at index {0}")]
    ZeroShare(usize),
    #[error("duplicate recipient {0:?} in split spec")]
    DuplicateRecipient([u8; 32]),
    #[error("self-recipient: source cannot dedicate to themselves")]
    SelfRecipient,
    #[error("legacy not yet inverted — source still alive")]
    NotInverted,
    #[error("legacy already fully distributed: total share paid = 10_000bp")]
    FullyDistributed,
    #[error("unknown recipient {0:?} for this split")]
    UnknownRecipient([u8; 32]),
    #[error("recipient {0:?} has already curated this dedication")]
    AlreadyCurated([u8; 32]),
    #[error("recipient {0:?} is not the curating survivor")]
    NotRecipient([u8; 32]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitState {
    /// Source still alive; the split is dormant.
    Pending,
    /// Source certified dead; Dedications produced and active.
    Inverted { died_at_epoch: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Curation {
    Pending,
    Accepted,
    Rejected,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitDedication {
    pub recipient: [u8; 32],
    pub share_bp: u64,
    pub curation: Curation,
    /// Whether this share has been counted into the legacy's
    /// total-distributed accounting. Survivors who reject still
    /// count (their slot is "claimed" — the dedication exists
    /// on chain in their preferred state).
    pub claimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitLegacy {
    pub id: SplitId,
    pub source: [u8; 32],
    pub dedications: Vec<SplitDedication>,
    pub state: SplitState,
    pub declared_at_epoch: u64,
    pub total_share_paid_bp: u64,
}

impl SplitLegacy {
    /// Declare a new Legacy edge with split spec. Caller passes
    /// `(recipient, share_bp)` pairs that must sum to exactly
    /// 10_000.
    pub fn declare(
        id: SplitId,
        source: [u8; 32],
        spec: Vec<([u8; 32], u64)>,
        declared_at_epoch: u64,
    ) -> Result<Self, SplitError> {
        if spec.is_empty() {
            return Err(SplitError::EmptySplit);
        }
        let mut sum: u64 = 0;
        let mut seen = std::collections::BTreeSet::new();
        for (i, (recipient, share_bp)) in spec.iter().enumerate() {
            if *share_bp == 0 {
                return Err(SplitError::ZeroShare(i));
            }
            if !seen.insert(*recipient) {
                return Err(SplitError::DuplicateRecipient(*recipient));
            }
            if *recipient == source {
                return Err(SplitError::SelfRecipient);
            }
            sum = sum
                .checked_add(*share_bp)
                .ok_or(SplitError::SplitDoesNotSumToFull { sum: u64::MAX })?;
        }
        if sum != 10_000 {
            return Err(SplitError::SplitDoesNotSumToFull { sum });
        }
        let dedications: Vec<SplitDedication> = spec
            .into_iter()
            .map(|(recipient, share_bp)| SplitDedication {
                recipient,
                share_bp,
                curation: Curation::Pending,
                claimed: false,
            })
            .collect();
        Ok(Self {
            id,
            source,
            dedications,
            state: SplitState::Pending,
            declared_at_epoch,
            total_share_paid_bp: 0,
        })
    }

    /// Source-death certified. Splits the Legacy into N
    /// Dedications atomically.
    pub fn certify_source_death(&mut self, died_at_epoch: u64) {
        self.state = SplitState::Inverted { died_at_epoch };
    }

    /// Survivor curates their share. Marks it Accepted/Rejected/
    /// Hidden and counts it as claimed (regardless of curation).
    pub fn curate(&mut self, recipient: [u8; 32], choice: Curation) -> Result<(), SplitError> {
        if !matches!(self.state, SplitState::Inverted { .. }) {
            return Err(SplitError::NotInverted);
        }
        let ded = self
            .dedications
            .iter_mut()
            .find(|d| d.recipient == recipient)
            .ok_or(SplitError::UnknownRecipient(recipient))?;
        if ded.claimed {
            return Err(SplitError::AlreadyCurated(recipient));
        }
        if matches!(choice, Curation::Pending) {
            // Setting back to Pending is a no-op; treat as
            // not-yet-claimed.
            ded.curation = choice;
            return Ok(());
        }
        ded.curation = choice;
        ded.claimed = true;
        self.total_share_paid_bp = self
            .total_share_paid_bp
            .checked_add(ded.share_bp)
            .ok_or(SplitError::FullyDistributed)?;
        Ok(())
    }

    pub fn is_fully_distributed(&self) -> bool {
        self.total_share_paid_bp >= 10_000
    }

    pub fn share_for(&self, recipient: [u8; 32]) -> Option<u64> {
        self.dedications
            .iter()
            .find(|d| d.recipient == recipient)
            .map(|d| d.share_bp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(b: u8) -> SplitId {
        SplitId([b; 32])
    }
    fn alice() -> [u8; 32] {
        [0xAA; 32]
    }
    fn bob() -> [u8; 32] {
        [0xBB; 32]
    }
    fn carol() -> [u8; 32] {
        [0xCC; 32]
    }
    fn dan() -> [u8; 32] {
        [0xDD; 32]
    }

    fn fresh_balanced() -> SplitLegacy {
        // 60/30/10 split.
        SplitLegacy::declare(
            sid(1),
            alice(),
            vec![(bob(), 6_000), (carol(), 3_000), (dan(), 1_000)],
            0,
        )
        .unwrap()
    }

    // ── declaration validation ───────────────────────────────────

    #[test]
    fn declare_balanced_succeeds() {
        let l = fresh_balanced();
        assert_eq!(l.dedications.len(), 3);
        assert!(matches!(l.state, SplitState::Pending));
        assert_eq!(l.total_share_paid_bp, 0);
    }

    #[test]
    fn empty_split_rejected() {
        let err = SplitLegacy::declare(sid(1), alice(), vec![], 0).unwrap_err();
        assert_eq!(err, SplitError::EmptySplit);
    }

    #[test]
    fn split_below_full_rejected() {
        let err = SplitLegacy::declare(sid(1), alice(), vec![(bob(), 5_000), (carol(), 4_000)], 0)
            .unwrap_err();
        assert!(matches!(
            err,
            SplitError::SplitDoesNotSumToFull { sum: 9_000 }
        ));
    }

    #[test]
    fn split_above_full_rejected() {
        let err = SplitLegacy::declare(sid(1), alice(), vec![(bob(), 6_000), (carol(), 5_000)], 0)
            .unwrap_err();
        assert!(matches!(
            err,
            SplitError::SplitDoesNotSumToFull { sum: 11_000 }
        ));
    }

    #[test]
    fn zero_share_rejected() {
        let err = SplitLegacy::declare(sid(1), alice(), vec![(bob(), 10_000), (carol(), 0)], 0)
            .unwrap_err();
        assert!(matches!(err, SplitError::ZeroShare(1)));
    }

    #[test]
    fn duplicate_recipient_rejected() {
        let err = SplitLegacy::declare(sid(1), alice(), vec![(bob(), 5_000), (bob(), 5_000)], 0)
            .unwrap_err();
        assert_eq!(err, SplitError::DuplicateRecipient(bob()));
    }

    #[test]
    fn self_recipient_rejected() {
        let err = SplitLegacy::declare(sid(1), alice(), vec![(alice(), 10_000)], 0).unwrap_err();
        assert_eq!(err, SplitError::SelfRecipient);
    }

    #[test]
    fn single_recipient_full_share_succeeds() {
        let l = SplitLegacy::declare(sid(1), alice(), vec![(bob(), 10_000)], 0).unwrap();
        assert_eq!(l.dedications.len(), 1);
        assert_eq!(l.dedications[0].share_bp, 10_000);
    }

    // ── inversion + curation ─────────────────────────────────────

    #[test]
    fn cannot_curate_before_inversion() {
        let mut l = fresh_balanced();
        let err = l.curate(bob(), Curation::Accepted).unwrap_err();
        assert_eq!(err, SplitError::NotInverted);
    }

    #[test]
    fn curate_accepted_marks_claimed_and_increments_total() {
        let mut l = fresh_balanced();
        l.certify_source_death(100);
        l.curate(bob(), Curation::Accepted).unwrap();
        assert_eq!(l.total_share_paid_bp, 6_000);
        assert!(
            l.dedications
                .iter()
                .find(|d| d.recipient == bob())
                .unwrap()
                .claimed
        );
    }

    #[test]
    fn curate_rejected_still_marks_claimed() {
        // Rejection still consumes the slot — a survivor who
        // rejects can't later change to accept and "regain" the
        // share.
        let mut l = fresh_balanced();
        l.certify_source_death(100);
        l.curate(bob(), Curation::Rejected).unwrap();
        assert_eq!(l.total_share_paid_bp, 6_000);
    }

    #[test]
    fn curate_hidden_marks_claimed() {
        let mut l = fresh_balanced();
        l.certify_source_death(100);
        l.curate(bob(), Curation::Hidden).unwrap();
        assert_eq!(l.total_share_paid_bp, 6_000);
    }

    #[test]
    fn curate_unknown_recipient_rejected() {
        let mut l = fresh_balanced();
        l.certify_source_death(100);
        let stranger = [0xEE; 32];
        let err = l.curate(stranger, Curation::Accepted).unwrap_err();
        assert_eq!(err, SplitError::UnknownRecipient(stranger));
    }

    #[test]
    fn double_curation_rejected() {
        let mut l = fresh_balanced();
        l.certify_source_death(100);
        l.curate(bob(), Curation::Accepted).unwrap();
        let err = l.curate(bob(), Curation::Hidden).unwrap_err();
        assert_eq!(err, SplitError::AlreadyCurated(bob()));
    }

    #[test]
    fn pending_curation_does_not_count() {
        // Setting curation to Pending is a no-op; not claimed.
        let mut l = fresh_balanced();
        l.certify_source_death(100);
        l.curate(bob(), Curation::Pending).unwrap();
        assert_eq!(l.total_share_paid_bp, 0);
        assert!(
            !l.dedications
                .iter()
                .find(|d| d.recipient == bob())
                .unwrap()
                .claimed
        );
    }

    // ── full distribution ────────────────────────────────────────

    #[test]
    fn all_three_curations_fully_distribute() {
        let mut l = fresh_balanced();
        l.certify_source_death(100);
        l.curate(bob(), Curation::Accepted).unwrap();
        l.curate(carol(), Curation::Hidden).unwrap();
        l.curate(dan(), Curation::Rejected).unwrap();
        assert_eq!(l.total_share_paid_bp, 10_000);
        assert!(l.is_fully_distributed());
    }

    // ── share_for query ──────────────────────────────────────────

    #[test]
    fn share_for_returns_recipient_share() {
        let l = fresh_balanced();
        assert_eq!(l.share_for(bob()), Some(6_000));
        assert_eq!(l.share_for(carol()), Some(3_000));
        assert_eq!(l.share_for(dan()), Some(1_000));
        assert_eq!(l.share_for([0xEE; 32]), None);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "GraveGraph V2 ships split Dedications. A holder
        // declares a Legacy edge with a SplitSpec — per-recipient
        // basis-point share summing to exactly 10_000 — and on
        // certified death the single edge inverts into N weighted
        // Dedications. Each survivor curates their share
        // independently; the cumulative paid-out tracking
        // prevents a slot from being claimed twice."

        let mut l = SplitLegacy::declare(
            sid(1),
            alice(),
            vec![(bob(), 5_000), (carol(), 3_500), (dan(), 1_500)],
            0,
        )
        .unwrap();

        // Pre-death: cannot curate.
        assert!(matches!(
            l.curate(bob(), Curation::Accepted),
            Err(SplitError::NotInverted)
        ));

        // Source dies; legacy inverts.
        l.certify_source_death(100);

        // Three independent curations.
        l.curate(bob(), Curation::Accepted).unwrap();
        l.curate(carol(), Curation::Rejected).unwrap();
        l.curate(dan(), Curation::Hidden).unwrap();

        // All shares accounted for.
        assert_eq!(l.total_share_paid_bp, 10_000);
        assert!(l.is_fully_distributed());

        // Bob cannot curate again — slot consumed.
        assert!(matches!(
            l.curate(bob(), Curation::Hidden),
            Err(SplitError::AlreadyCurated(_))
        ));
    }

    proptest::proptest! {
        #[test]
        fn property_balanced_split_always_declares(
            n in 1usize..6usize,
            seed in 1u64..1000u64,
        ) {
            // Generate a balanced split that sums to 10_000 by
            // distributing 10_000 across n recipients.
            let base = 10_000 / (n as u64);
            let remainder = 10_000 - base * (n as u64);
            let mut spec: Vec<([u8; 32], u64)> = Vec::new();
            for i in 0..n {
                let recipient = [(i as u8 + 0x10); 32];
                let extra = if (i as u64) < remainder { 1 } else { 0 };
                spec.push((recipient, base + extra));
            }
            let result = SplitLegacy::declare(sid(seed as u8), alice(), spec, 0);
            proptest::prop_assert!(result.is_ok());
        }
    }
}
