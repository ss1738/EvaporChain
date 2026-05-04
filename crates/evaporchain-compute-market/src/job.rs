//! `Job` — one compute-market entry + lifecycle.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct JobId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum JobError {
    #[error("zero bounty")]
    ZeroBounty,
    #[error("zero deadline")]
    ZeroDeadline,
    #[error("zero proof_energy_floor")]
    ZeroFloor,
    #[error("zero bond_required")]
    ZeroBond,
    #[error("job already in terminal state {0:?}")]
    Terminal(JobState),
    #[error("job is not in Open state — current: {0:?}")]
    NotOpen(JobState),
    #[error("job is not in Claimed state — current: {0:?}")]
    NotClaimed(JobState),
    #[error("claim epoch {claim} > deadline {deadline}")]
    ClaimAfterDeadline { claim: u64, deadline: u64 },
    #[error("submission epoch {sub} > deadline {deadline}")]
    SubmitAfterDeadline { sub: u64, deadline: u64 },
    #[error("proof_energy {energy} below floor {floor} — stale worker")]
    StaleProof { energy: u64, floor: u64 },
    #[error("worker bond {provided} < bond_required {required}")]
    BondBelowRequired { provided: u128, required: u128 },
    #[error("not the claimant: caller is not the worker who claimed this job")]
    NotClaimant,
    #[error("expire called before deadline {deadline} (now={now})")]
    NotYetExpired { now: u64, deadline: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Open,
    Claimed { worker: [u8; 32], claimed_at: u64 },
    Paid { worker: [u8; 32], paid_at: u64 },
    Slashed { reason_at: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub buyer: [u8; 32],
    pub spec_hash: [u8; 32],
    pub bounty_micros: u128,
    pub bond_required_micros: u128,
    pub posted_at_epoch: u64,
    /// Absolute deadline = posted_at + deadline_epochs.
    pub deadline_epoch: u64,
    pub proof_energy_floor: u64,
    pub state: JobState,
    /// Worker's locked bond once claimed; 0 in Open / settled states.
    pub locked_bond_micros: u128,
}

impl Job {
    pub fn post(
        id: JobId,
        buyer: [u8; 32],
        spec_hash: [u8; 32],
        bounty_micros: u128,
        bond_required_micros: u128,
        deadline_epochs: u64,
        proof_energy_floor: u64,
        posted_at_epoch: u64,
    ) -> Result<Self, JobError> {
        if bounty_micros == 0 {
            return Err(JobError::ZeroBounty);
        }
        if bond_required_micros == 0 {
            return Err(JobError::ZeroBond);
        }
        if deadline_epochs == 0 {
            return Err(JobError::ZeroDeadline);
        }
        if proof_energy_floor == 0 {
            return Err(JobError::ZeroFloor);
        }
        Ok(Self {
            id,
            buyer,
            spec_hash,
            bounty_micros,
            bond_required_micros,
            posted_at_epoch,
            deadline_epoch: posted_at_epoch.saturating_add(deadline_epochs),
            proof_energy_floor,
            state: JobState::Open,
            locked_bond_micros: 0,
        })
    }

    /// Claim the job. Locks `bond_provided` (must be ≥ required).
    /// Caller becomes the claimant.
    pub fn claim(
        &mut self,
        worker: [u8; 32],
        bond_provided: u128,
        now: u64,
    ) -> Result<(), JobError> {
        if !matches!(self.state, JobState::Open) {
            return Err(JobError::NotOpen(self.state));
        }
        if now > self.deadline_epoch {
            return Err(JobError::ClaimAfterDeadline {
                claim: now,
                deadline: self.deadline_epoch,
            });
        }
        if bond_provided < self.bond_required_micros {
            return Err(JobError::BondBelowRequired {
                provided: bond_provided,
                required: self.bond_required_micros,
            });
        }
        self.state = JobState::Claimed { worker, claimed_at: now };
        self.locked_bond_micros = bond_provided;
        Ok(())
    }

    /// Submit a result. On success, transitions to Paid; bond is
    /// released back to the worker (caller drains
    /// `locked_bond_micros` and adds bounty).
    ///
    /// Returns `(payout_to_worker, payout_to_buyer)` —
    /// `(bond + bounty, 0)` on success, the chain transfers.
    pub fn submit(
        &mut self,
        caller: [u8; 32],
        proof_energy: u64,
        now: u64,
    ) -> Result<(u128, u128), JobError> {
        let (worker, _claimed_at) = match self.state {
            JobState::Claimed { worker, claimed_at } => (worker, claimed_at),
            _ => return Err(JobError::NotClaimed(self.state)),
        };
        if caller != worker {
            return Err(JobError::NotClaimant);
        }

        // Stale-proof check FIRST — structural rejection before
        // deadline check.
        if proof_energy < self.proof_energy_floor {
            return Err(JobError::StaleProof {
                energy: proof_energy,
                floor: self.proof_energy_floor,
            });
        }
        if now > self.deadline_epoch {
            return Err(JobError::SubmitAfterDeadline {
                sub: now,
                deadline: self.deadline_epoch,
            });
        }

        let payout_worker = self
            .locked_bond_micros
            .checked_add(self.bounty_micros)
            .ok_or_else(|| {
                // Theoretical: payout overflows u128. Treat as
                // terminal slash on the worker side as a defensive
                // measure — but realistically u128 will not overflow.
                JobError::Terminal(JobState::Slashed { reason_at: now })
            })?;
        self.locked_bond_micros = 0;
        self.state = JobState::Paid { worker, paid_at: now };
        Ok((payout_worker, 0))
    }

    /// Sweep: any Claimed job past its deadline is slashed.
    /// Returns the (bond_to_buyer, bounty_to_buyer) amounts.
    /// Open jobs past deadline are also closed: bounty refunded
    /// to buyer, no bond to slash.
    pub fn expire_overdue(
        &mut self,
        now: u64,
    ) -> Result<(u128, u128), JobError> {
        if now <= self.deadline_epoch {
            return Err(JobError::NotYetExpired {
                now,
                deadline: self.deadline_epoch,
            });
        }
        match self.state {
            JobState::Claimed { .. } => {
                // Slash bond to buyer, refund bounty to buyer.
                let bond = self.locked_bond_micros;
                let bounty = self.bounty_micros;
                self.locked_bond_micros = 0;
                self.state = JobState::Slashed { reason_at: now };
                Ok((bond, bounty))
            }
            JobState::Open => {
                // No worker — refund bounty only.
                let bounty = self.bounty_micros;
                self.state = JobState::Slashed { reason_at: now };
                Ok((0, bounty))
            }
            _ => Err(JobError::Terminal(self.state)),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.state, JobState::Paid { .. } | JobState::Slashed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jid(b: u8) -> JobId { JobId([b; 32]) }
    fn buyer() -> [u8; 32] { [0xBB; 32] }
    fn worker() -> [u8; 32] { [0xCC; 32] }
    fn worker2() -> [u8; 32] { [0xEE; 32] }

    fn fresh() -> Job {
        Job::post(
            jid(1),
            buyer(),
            [0x11; 32],
            1_000_000,    // bounty 1.0
            100_000,      // bond 0.1
            100,          // deadline 100 epochs
            500,          // proof_energy_floor
            0,
        )
        .unwrap()
    }

    // ── construction ─────────────────────────────────────────────

    #[test]
    fn post_initializes_open() {
        let j = fresh();
        assert!(matches!(j.state, JobState::Open));
        assert_eq!(j.deadline_epoch, 100);
        assert_eq!(j.locked_bond_micros, 0);
    }

    #[test]
    fn post_rejects_zero_params() {
        assert_eq!(Job::post(jid(1), buyer(), [0;32], 0, 1, 1, 1, 0).unwrap_err(), JobError::ZeroBounty);
        assert_eq!(Job::post(jid(1), buyer(), [0;32], 1, 0, 1, 1, 0).unwrap_err(), JobError::ZeroBond);
        assert_eq!(Job::post(jid(1), buyer(), [0;32], 1, 1, 0, 1, 0).unwrap_err(), JobError::ZeroDeadline);
        assert_eq!(Job::post(jid(1), buyer(), [0;32], 1, 1, 1, 0, 0).unwrap_err(), JobError::ZeroFloor);
    }

    // ── claim ────────────────────────────────────────────────────

    #[test]
    fn claim_locks_bond_and_records_worker() {
        let mut j = fresh();
        j.claim(worker(), 100_000, 50).unwrap();
        assert!(matches!(j.state, JobState::Claimed { worker: w, claimed_at: 50 } if w == worker()));
        assert_eq!(j.locked_bond_micros, 100_000);
    }

    #[test]
    fn claim_below_required_bond_rejected() {
        let mut j = fresh();
        let err = j.claim(worker(), 50_000, 50).unwrap_err();
        assert!(matches!(err, JobError::BondBelowRequired { .. }));
    }

    #[test]
    fn claim_after_deadline_rejected() {
        let mut j = fresh();
        let err = j.claim(worker(), 100_000, 200).unwrap_err();
        assert!(matches!(err, JobError::ClaimAfterDeadline { .. }));
    }

    #[test]
    fn claim_on_already_claimed_rejected() {
        let mut j = fresh();
        j.claim(worker(), 100_000, 50).unwrap();
        let err = j.claim(worker2(), 100_000, 60).unwrap_err();
        assert!(matches!(err, JobError::NotOpen(_)));
    }

    // ── submit ───────────────────────────────────────────────────

    #[test]
    fn fresh_submit_pays_bond_plus_bounty() {
        let mut j = fresh();
        j.claim(worker(), 100_000, 10).unwrap();
        let (payout_worker, payout_buyer) = j.submit(worker(), 1000, 50).unwrap();
        assert_eq!(payout_worker, 1_100_000); // bond 100k + bounty 1M
        assert_eq!(payout_buyer, 0);
        assert!(matches!(j.state, JobState::Paid { .. }));
    }

    #[test]
    fn submit_below_proof_floor_rejected() {
        let mut j = fresh();
        j.claim(worker(), 100_000, 10).unwrap();
        let err = j.submit(worker(), 100, 50).unwrap_err(); // floor 500
        assert!(matches!(err, JobError::StaleProof { .. }));
        // State still Claimed.
        assert!(matches!(j.state, JobState::Claimed { .. }));
    }

    #[test]
    fn submit_after_deadline_rejected() {
        let mut j = fresh();
        j.claim(worker(), 100_000, 10).unwrap();
        let err = j.submit(worker(), 1000, 200).unwrap_err();
        assert!(matches!(err, JobError::SubmitAfterDeadline { .. }));
    }

    #[test]
    fn submit_by_non_claimant_rejected() {
        let mut j = fresh();
        j.claim(worker(), 100_000, 10).unwrap();
        let err = j.submit(worker2(), 1000, 50).unwrap_err();
        assert_eq!(err, JobError::NotClaimant);
    }

    #[test]
    fn submit_on_open_job_rejected() {
        let mut j = fresh();
        let err = j.submit(worker(), 1000, 50).unwrap_err();
        assert!(matches!(err, JobError::NotClaimed(_)));
    }

    #[test]
    fn stale_proof_check_runs_before_deadline_check() {
        // Worker submits past deadline AND with stale proof.
        // Stale-proof should be the surfaced error (it runs first).
        let mut j = fresh();
        j.claim(worker(), 100_000, 10).unwrap();
        let err = j.submit(worker(), 100, 200).unwrap_err();
        assert!(matches!(err, JobError::StaleProof { .. }));
    }

    // ── expire overdue ───────────────────────────────────────────

    #[test]
    fn expire_open_job_refunds_bounty_only() {
        let mut j = fresh();
        let (bond, bounty) = j.expire_overdue(101).unwrap();
        assert_eq!(bond, 0);
        assert_eq!(bounty, 1_000_000);
        assert!(matches!(j.state, JobState::Slashed { .. }));
    }

    #[test]
    fn expire_claimed_job_slashes_bond_refunds_bounty() {
        let mut j = fresh();
        j.claim(worker(), 100_000, 10).unwrap();
        let (bond, bounty) = j.expire_overdue(200).unwrap();
        assert_eq!(bond, 100_000); // bond → buyer
        assert_eq!(bounty, 1_000_000); // bounty → buyer
        assert!(matches!(j.state, JobState::Slashed { .. }));
    }

    #[test]
    fn expire_before_deadline_rejected() {
        let mut j = fresh();
        let err = j.expire_overdue(50).unwrap_err();
        assert!(matches!(err, JobError::NotYetExpired { .. }));
    }

    #[test]
    fn expire_terminal_job_rejected() {
        let mut j = fresh();
        j.claim(worker(), 100_000, 10).unwrap();
        j.submit(worker(), 1000, 50).unwrap();
        let err = j.expire_overdue(200).unwrap_err();
        assert!(matches!(err, JobError::Terminal(_)));
    }

    // ── doctrine claim ───────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Energy-Budgeted Compute Marketplace structurally
        // prevents grief-by-claiming. A worker who claims a job
        // and disappears loses their bond at deadline. A worker
        // who submits stale proof — even on time — gets rejected.
        // Honest workers with fresh proof get bounty + bond back."
        //
        // Three workers:
        // - Honest: claims fast, submits fresh proof in time.
        // - Grief: claims, never submits, deadline passes.
        // - Late: claims, submits with stale (decayed) proof.

        // Honest path
        let mut j_honest = fresh();
        j_honest.claim(worker(), 100_000, 10).unwrap();
        let (worker_payout, _) = j_honest.submit(worker(), 1000, 50).unwrap();
        assert_eq!(worker_payout, 1_100_000); // bond + bounty

        // Grief path
        let mut j_grief = fresh();
        j_grief.claim(worker(), 100_000, 10).unwrap();
        // Worker disappears. Buyer expires after deadline.
        let (bond_to_buyer, bounty_to_buyer) = j_grief.expire_overdue(200).unwrap();
        assert_eq!(bond_to_buyer, 100_000);
        assert_eq!(bounty_to_buyer, 1_000_000);

        // Late-with-stale-proof path
        let mut j_stale = fresh();
        j_stale.claim(worker(), 100_000, 10).unwrap();
        // Worker submits in time but with energy below floor (decayed).
        let err = j_stale.submit(worker(), 100, 50).unwrap_err();
        assert!(matches!(err, JobError::StaleProof { .. }));
        // After deadline, buyer expires.
        let (bond_to_buyer, bounty_to_buyer) = j_stale.expire_overdue(200).unwrap();
        assert_eq!(bond_to_buyer, 100_000);
        assert_eq!(bounty_to_buyer, 1_000_000);
    }

    proptest::proptest! {
        #[test]
        fn property_only_claimant_can_submit(
            attacker_byte in 0u8..255u8,
        ) {
            let attacker = [attacker_byte; 32];
            // Skip the case where attacker happens to be the worker.
            if attacker == worker() {
                return Ok(());
            }
            let mut j = fresh();
            j.claim(worker(), 100_000, 10).unwrap();
            let err = j.submit(attacker, 1000, 50).unwrap_err();
            proptest::prop_assert_eq!(err, JobError::NotClaimant);
        }

        #[test]
        fn property_proof_below_floor_always_rejects(
            energy in 0u64..499u64, // floor is 500
            now in 11u64..100u64,
        ) {
            let mut j = fresh();
            j.claim(worker(), 100_000, 10).unwrap();
            let result = j.submit(worker(), energy, now);
            let is_stale = matches!(result, Err(JobError::StaleProof { .. }));
            proptest::prop_assert!(is_stale);
        }
    }
}
