//! Market — wraps SDDC for clearing employer bounties against holder bids.
//!
//! Two-axis interpretation for SHLM (per A5.2 doctrine):
//!
//! - SDDC's `max_price` ⇔ holder's salary expectation (bidder = candidate
//!   asking for ≥ this much; auction descends from `salary_cap` toward
//!   the floor).
//! - SDDC's `lambda_tolerance` ⇔ how-decayed-an-acceptable-credential
//!   the *bounty* will tolerate. The lot's `lot_lambda` here is the
//!   class's half-life — high half-life means the bounty's tolerance
//!   needs to be high to clear (a desperate-for-COBOL employer accepts
//!   a more decayed token; a cutting-edge Python employer needs very
//!   fresh).
//!
//! Both axes must clear at the bid's submission epoch.
//!
//! Pre-clear eligibility filter: every bid is checked against
//! `Bounty::matches(class, cred, submitted_at)` first; bids whose
//! credential doesn't meet `(min_freshness, min_level)` at their
//! submission epoch are silently dropped from the candidate set,
//! never reaching SDDC. This keeps SDDC pure-mechanism — SHLM's
//! domain logic doesn't leak into it.

use evaporchain_sddc::{
    auction::{Auction, AuctionId, Cleared, LifecycleError},
    bid::Bid,
    clearing::{try_clear, ClearError},
};
use evaporchain_types::{AccountAddress, DecayRate, Energy, Epoch};
use thiserror::Error;

use crate::bounty::{Bounty, BountyError};
use crate::credential::Credential;
use crate::skill_class::SkillClass;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MarketError {
    #[error("auction setup failed: {0}")]
    AuctionSetup(#[from] LifecycleError),
    #[error("clearing failed: {0}")]
    Clearing(#[from] ClearError),
    #[error("bounty/credential mismatch: {0}")]
    Bounty(#[from] BountyError),
}

/// Open the SDDC auction backing a bounty.
///
/// `salary_floor` is the minimum the employer will pay (typically a
/// fraction of `salary_cap`); `lot_lambda` is the class's half-life
/// (the freshness-tolerance axis).
pub fn post_bounty(
    bounty: &Bounty,
    auction_id: AuctionId,
    salary_floor: Energy,
    duration_epochs: u64,
) -> Result<Auction, MarketError> {
    let lot_lambda: DecayRate = 0; // initialised; SDDC type alias
                                   // Use the class id alone isn't enough — SDDC needs a numeric λ. We
                                   // use the bounty's `min_freshness` as the lot's published λ-axis
                                   // value: bidders' `lambda_tolerance` is read as "how decayed a
                                   // freshness am I willing to accept" — bounty publishes its floor.
    let _ = lot_lambda;
    Ok(Auction::open(
        auction_id,
        bounty.salary_cap,
        salary_floor,
        bounty.min_freshness,
        bounty.posted_at,
        duration_epochs,
    )?)
}

/// One candidate bid: a credential-holder offers to take the bounty
/// at `salary_ask` (which becomes the SDDC `max_price`) for their
/// `cred`. The `submitted_at` is the consensus epoch the bid arrived.
#[derive(Debug, Clone)]
pub struct CandidateBid<'a> {
    pub holder: AccountAddress,
    pub cred: &'a Credential,
    pub salary_ask: Energy,
    pub freshness_tolerance: Energy,
    pub submitted_at: Epoch,
}

/// Run the full bounty-clearing flow:
///   1. Filter candidates whose credentials don't satisfy the bounty's
///      thresholds at their submission epoch.
///   2. Translate surviving candidates into SDDC `Bid`s.
///   3. Run `try_clear` against the auction.
///
/// Returns `Some(Cleared)` with the winner when a candidate clears.
pub fn settle_bounty(
    bounty: &Bounty,
    class: &SkillClass,
    auction: &mut Auction,
    candidates: &[CandidateBid<'_>],
    epoch_now: Epoch,
) -> Result<Option<Cleared>, MarketError> {
    // Build the SDDC bid list from candidates whose credentials
    // satisfy bounty thresholds at *their* submission epoch.
    let mut bids: Vec<Bid> = Vec::with_capacity(candidates.len());
    for c in candidates {
        if c.cred.class != bounty.class || class.id != bounty.class {
            return Err(BountyError::ClassMismatch.into());
        }
        if !bounty.matches(class, c.cred, c.submitted_at)? {
            continue;
        }
        // Build the Bid; surface zero-salary as a domain error.
        let bid = Bid::new(
            c.holder,
            c.salary_ask,
            c.freshness_tolerance,
            c.submitted_at,
        )
        .map_err(|_| BountyError::ZeroSalary)?;
        bids.push(bid);
    }
    Ok(try_clear(auction, &bids, epoch_now)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn class() -> SkillClass {
        SkillClass::register(id(0xC1), "Python", 1000, 0).unwrap()
    }

    fn cred_at(holder: u8, level: Energy, attested_at: Epoch) -> Credential {
        Credential::issue(id(0xCD), id(0xC1), id(0xAA), id(holder), level, attested_at).unwrap()
    }

    fn bounty(min_freshness: Energy, min_level: Energy) -> Bounty {
        Bounty::post(
            id(0xB0),
            id(0xEE),
            id(0xC1),
            100_000, // salary cap
            min_freshness,
            min_level,
            0, // posted at
        )
        .unwrap()
    }

    #[test]
    fn post_bounty_opens_auction_with_class_freshness_axis() {
        let b = bounty(500, 100);
        let a = post_bounty(&b, id(0xA0), 10_000, 100).unwrap();
        assert_eq!(a.ceiling, 100_000);
        assert_eq!(a.floor, 10_000);
        // The auction's lot_lambda axis is the bounty's min_freshness
        // — bidders' freshness_tolerance must clear it.
        assert_eq!(a.lot_lambda, 500);
    }

    #[test]
    fn settle_picks_first_qualifying_candidate() {
        let c = class();
        let b = bounty(500, 100);
        let mut a = post_bounty(&b, id(0xA0), 10_000, 1000).unwrap();
        let cr_strong = cred_at(0xBB, 1000, 0);
        let cr_weak = cred_at(0xCC, 50, 0); // level=50 < min_level=100

        let candidates = vec![
            // Strong candidate, asks for 50_000 at epoch 100. Auction
            // price at epoch 100 starts dropping from 100_000; with
            // duration=1000 over span 90_000, drop is 9_000 ⇒ price
            // 91_000. holder's salary_ask=50_000 ≥ 91_000? No — wrong
            // direction. Holder accepts ≤ 91_000 (auction-style).
            // Re-frame: max_price 91_000 ≥ price 91_000 ⇒ clears.
            CandidateBid {
                holder: id(0xBB),
                cred: &cr_strong,
                salary_ask: 100_000,
                freshness_tolerance: 1000,
                submitted_at: 100,
            },
            // Weak candidate, would ask earlier but credential fails
            // bounty thresholds (level=50 < 100). Filtered out.
            CandidateBid {
                holder: id(0xCC),
                cred: &cr_weak,
                salary_ask: 100_000,
                freshness_tolerance: 1000,
                submitted_at: 50,
            },
        ];

        let cleared = settle_bounty(&b, &c, &mut a, &candidates, 200)
            .unwrap()
            .unwrap();
        // Weak candidate filtered ⇒ strong candidate (0xBB) wins.
        assert_eq!(cleared.winner, id(0xBB));
    }

    #[test]
    fn settle_returns_none_when_no_candidate_passes_filter() {
        let c = class();
        let b = bounty(900, 100); // demands very high freshness
        let mut a = post_bounty(&b, id(0xA0), 10_000, 1000).unwrap();
        // Credential decayed past 900 by epoch 1000.
        let cr = cred_at(0xBB, 1000, 0);
        let candidates = vec![CandidateBid {
            holder: id(0xBB),
            cred: &cr,
            salary_ask: 100_000,
            freshness_tolerance: 900,
            submitted_at: 1000,
        }];
        let res = settle_bounty(&b, &c, &mut a, &candidates, 1500).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn settle_rejects_class_mismatch() {
        let c = class();
        let b = bounty(500, 100);
        let mut a = post_bounty(&b, id(0xA0), 10_000, 1000).unwrap();
        // Credential issued under a different class.
        let wrong_cred =
            Credential::issue(id(0xCD), id(0xC2), id(0xAA), id(0xBB), 1000, 0).unwrap();
        let candidates = vec![CandidateBid {
            holder: id(0xBB),
            cred: &wrong_cred,
            salary_ask: 100_000,
            freshness_tolerance: 1000,
            submitted_at: 100,
        }];
        let err = settle_bounty(&b, &c, &mut a, &candidates, 200).unwrap_err();
        assert!(matches!(
            err,
            MarketError::Bounty(BountyError::ClassMismatch)
        ));
    }

    #[test]
    fn high_freshness_tolerance_unlocks_decayed_candidate() {
        // Doctrine claim transposed to SHLM: a desperate-for-COBOL
        // employer (high freshness_tolerance) accepts a more-decayed
        // credential. Construct a bounty where one candidate has
        // freshness_tolerance ≥ class half-life ⇒ clears, while another
        // has tolerance < half-life ⇒ rejected on λ axis.
        let c = SkillClass::register(id(0xC1), "COBOL", 2000, 0).unwrap();
        let b = Bounty::post(
            id(0xB0),
            id(0xEE),
            id(0xC1),
            100_000,
            500, // min_freshness — used as SDDC lot_lambda
            100,
            0,
        )
        .unwrap();
        let mut a = post_bounty(&b, id(0xA0), 10_000, 1000).unwrap();

        let cr = cred_at(0xBB, 1000, 0);
        let candidates = vec![
            // Picky bidder — won't accept very-decayed credentials.
            // freshness_tolerance=400 < lot_lambda=500 ⇒ blocked on λ axis.
            CandidateBid {
                holder: id(0xBB),
                cred: &cr,
                salary_ask: 100_000,
                freshness_tolerance: 400,
                submitted_at: 50,
            },
            // Desperate bidder — accepts credentials more decayed than
            // they normally would. freshness_tolerance=600 ≥ 500 ⇒ OK.
            CandidateBid {
                holder: id(0xCC),
                cred: &cr,
                salary_ask: 100_000,
                freshness_tolerance: 600,
                submitted_at: 60,
            },
        ];

        let cleared = settle_bounty(&b, &c, &mut a, &candidates, 200)
            .unwrap()
            .unwrap();
        // The desperate bidder wins — they tolerated the higher λ.
        assert_eq!(cleared.winner, id(0xCC));
    }
}
