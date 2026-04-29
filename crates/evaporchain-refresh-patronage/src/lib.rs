//! Refresh-Pool Patronage Covenants.
//!
//! A state object can pledge a **Patronage Covenant**: pre-fund `n` epochs of
//! a voluntary donation (on top of standard rent) into the chain's refresh pool.
//!
//! While the covenant is active the object is **immune from eviction** — the
//! chain's keep-alive scheduler must not non-renew it even under pool pressure.
//!
//! Mechanics
//! ---------
//!
//! 1. [`pledge`] — caller computes `donation_per_epoch` (extra energy above
//!    rent). We draw `donation_per_epoch × epochs` from the namespace's pool
//!    credit and hold it as `pre_funded` inside the covenant.
//!
//! 2. [`honour`] — called once per epoch. Releases one epoch's donation from
//!    `pre_funded` into the global patronage namespace credit (`book.patronage_ns`).
//!    Increments `patronage_score`. If the covenant has expired, returns
//!    `CovenantError::Expired`.
//!
//! 3. [`revoke`] — caller removes the covenant early. Any remaining
//!    `pre_funded` surplus is refunded back to the namespace pool credit.
//!    Patronage score already accrued is retained on the `PatronageCovenant`
//!    returned to the caller for archiving.
//!
//! 4. [`is_immune`] — returns `true` iff an active covenant covers `epoch`.
//!
//! 5. [`patronage_score`] — total energy donated so far by an object.
//!
//! Pre-funding model
//! -----------------
//!
//! Drawing the full donation up-front at pledge time means `honour` never
//! needs to talk to the market (no cross-crate dependency at tick time). The
//! pool credit for the namespace is debited once; the global patronage credit
//! is accrued incrementally each epoch. On revoke, the unused remainder is
//! refunded so the namespace is not penalised for early exit beyond what was
//! already donated.

pub mod book;
pub mod covenant;
pub mod error;

pub use book::PatronageBook;
pub use covenant::PatronageCovenant;
pub use error::CovenantError;

use evaporchain_energy_kernel::{refresh_pool::NamespaceId, RefreshPool};
use evaporchain_types::Energy;

/// Draw `donation_per_epoch × epochs` from `namespace_id`'s pool credit and
/// create an active `PatronageCovenant` registered in `book`.
///
/// Returns the new covenant (for logging; the canonical copy lives in `book`).
pub fn pledge(
    book: &mut PatronageBook,
    pool: &mut RefreshPool,
    object_id: Vec<u8>,
    namespace_id: NamespaceId,
    donation_per_epoch: Energy,
    epochs: u64,
    current_epoch: u64,
) -> Result<PatronageCovenant, CovenantError> {
    if book.get(&object_id).is_some() {
        return Err(CovenantError::AlreadyPledged { object_id });
    }
    if epochs == 0 || donation_per_epoch == 0 {
        return Err(CovenantError::ZeroPledge);
    }
    let pre_funded = donation_per_epoch.saturating_mul(epochs);
    pool.payout(&namespace_id, pre_funded, current_epoch)
        .map_err(CovenantError::Pool)?;
    let covenant = PatronageCovenant {
        object_id: object_id.clone(),
        namespace_id,
        donation_per_epoch,
        created_epoch: current_epoch,
        expires_epoch: current_epoch.saturating_add(epochs),
        pre_funded,
        patronage_score: 0,
        last_honoured_epoch: None,
    };
    book.insert(covenant.clone());
    Ok(covenant)
}

/// Release one epoch of pre-funded donation from the covenant into the global
/// patronage pool credit.
///
/// Idempotent within the same epoch (returns `Ok(0)` if already honoured this
/// epoch). Returns `CovenantError::Expired` if the epoch is past `expires_epoch`.
pub fn honour(
    book: &mut PatronageBook,
    pool: &mut RefreshPool,
    object_id: &[u8],
    epoch: u64,
) -> Result<Energy, CovenantError> {
    let cv = book
        .get_mut(object_id)
        .ok_or_else(|| CovenantError::UnknownCovenant(object_id.to_vec()))?;

    if epoch >= cv.expires_epoch {
        return Err(CovenantError::Expired {
            object_id: object_id.to_vec(),
            expires: cv.expires_epoch,
        });
    }
    // Idempotent within the same epoch — return Ok(0) if already honoured.
    if cv.last_honoured_epoch == Some(epoch) {
        return Ok(0);
    }

    let donation = cv.donation_per_epoch.min(cv.pre_funded);
    if donation == 0 {
        return Ok(0);
    }
    cv.pre_funded = cv.pre_funded.saturating_sub(donation);
    cv.patronage_score = cv.patronage_score.saturating_add(donation);
    cv.last_honoured_epoch = Some(epoch);

    let pat_ns = book.patronage_ns.clone();
    pool.accrue(pat_ns, donation, epoch);
    Ok(donation)
}

/// Remove a covenant early. Refunds remaining `pre_funded` to the namespace
/// pool credit. Returns the archived covenant (patronage_score preserved).
pub fn revoke(
    book: &mut PatronageBook,
    pool: &mut RefreshPool,
    object_id: &[u8],
    epoch: u64,
) -> Result<PatronageCovenant, CovenantError> {
    let cv = book
        .remove(object_id)
        .ok_or_else(|| CovenantError::UnknownCovenant(object_id.to_vec()))?;
    if cv.pre_funded > 0 {
        pool.accrue(cv.namespace_id.clone(), cv.pre_funded, epoch);
    }
    Ok(cv)
}

/// True iff the object has an active covenant covering `epoch`.
pub fn is_immune(book: &PatronageBook, object_id: &[u8], epoch: u64) -> bool {
    book.get(object_id)
        .map(|cv| epoch < cv.expires_epoch)
        .unwrap_or(false)
}

/// Total energy donated so far (score retained across revoke via the returned
/// archived covenant; this queries only active covenants).
pub fn patronage_score(book: &PatronageBook, object_id: &[u8]) -> Energy {
    book.get(object_id)
        .map(|cv| cv.patronage_score)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::RefreshPool;

    fn ns(b: u8) -> NamespaceId {
        vec![b; 4]
    }
    fn obj(b: u8) -> Vec<u8> {
        vec![b; 8]
    }
    fn fresh_book() -> PatronageBook {
        PatronageBook::new(ns(0xff)) // global patronage ns
    }
    fn pool_with(ns_id: NamespaceId, amount: Energy) -> RefreshPool {
        let mut p = RefreshPool::new();
        p.accrue(ns_id, amount, 0);
        p
    }

    // ── pledge ────────────────────────────────────────────────────────────────

    #[test]
    fn pledge_draws_from_pool_and_registers() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        pledge(&mut book, &mut pool, obj(1), ns(1), 100, 10, 0).unwrap();
        // 100 × 10 = 1000 drawn
        assert_eq!(pool.total_accrued(), 1_000_000 - 1_000);
        let cv = book.get(&obj(1)).unwrap();
        assert_eq!(cv.pre_funded, 1_000);
        assert_eq!(cv.expires_epoch, 10);
        assert_eq!(cv.patronage_score, 0);
    }

    #[test]
    fn pledge_duplicate_rejected() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        pledge(&mut book, &mut pool, obj(1), ns(1), 100, 10, 0).unwrap();
        let err = pledge(&mut book, &mut pool, obj(1), ns(1), 100, 5, 1).unwrap_err();
        assert!(matches!(err, CovenantError::AlreadyPledged { .. }));
    }

    #[test]
    fn pledge_insufficient_pool_credit_rejected() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 50);
        // 100 × 10 = 1000 > 50
        let err = pledge(&mut book, &mut pool, obj(1), ns(1), 100, 10, 0).unwrap_err();
        assert!(matches!(err, CovenantError::Pool(_)));
        assert!(book.is_empty()); // not registered
    }

    #[test]
    fn pledge_zero_epochs_rejected() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        let err = pledge(&mut book, &mut pool, obj(1), ns(1), 100, 0, 0).unwrap_err();
        assert!(matches!(err, CovenantError::ZeroPledge));
    }

    #[test]
    fn pledge_zero_donation_rejected() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        let err = pledge(&mut book, &mut pool, obj(1), ns(1), 0, 10, 0).unwrap_err();
        assert!(matches!(err, CovenantError::ZeroPledge));
    }

    // ── honour ────────────────────────────────────────────────────────────────

    #[test]
    fn honour_releases_one_epoch_to_patronage_ns() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        pledge(&mut book, &mut pool, obj(1), ns(1), 100, 10, 0).unwrap();
        let donated = honour(&mut book, &mut pool, &obj(1), 1).unwrap();
        assert_eq!(donated, 100);
        // patronage ns credit
        let pat_ns = book.patronage_ns.clone();
        assert!(pool.total_accrued() > 0);
        // check pre_funded decreased
        assert_eq!(book.get(&obj(1)).unwrap().pre_funded, 1_000 - 100);
        assert_eq!(book.get(&obj(1)).unwrap().patronage_score, 100);
        // payout from patronage ns to verify it accrued
        pool.payout(&pat_ns, 100, 2).unwrap();
    }

    #[test]
    fn honour_at_expiry_epoch_returns_expired() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        pledge(&mut book, &mut pool, obj(1), ns(1), 100, 3, 0).unwrap();
        // expires_epoch = 3; epoch 3 is expired
        let err = honour(&mut book, &mut pool, &obj(1), 3).unwrap_err();
        assert!(matches!(err, CovenantError::Expired { .. }));
    }

    #[test]
    fn honour_unknown_object_returns_error() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        let err = honour(&mut book, &mut pool, &obj(99), 1).unwrap_err();
        assert!(matches!(err, CovenantError::UnknownCovenant(_)));
    }

    #[test]
    fn honour_multiple_epochs_accumulates_score() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        pledge(&mut book, &mut pool, obj(1), ns(1), 200, 5, 0).unwrap();
        for ep in 1..=4 {
            honour(&mut book, &mut pool, &obj(1), ep).unwrap();
        }
        assert_eq!(book.get(&obj(1)).unwrap().patronage_score, 800);
        assert_eq!(book.get(&obj(1)).unwrap().pre_funded, 200);
    }

    // ── revoke ────────────────────────────────────────────────────────────────

    #[test]
    fn revoke_refunds_remaining_pre_funded() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        pledge(&mut book, &mut pool, obj(1), ns(1), 100, 10, 0).unwrap();
        honour(&mut book, &mut pool, &obj(1), 1).unwrap(); // 100 donated
        let archived = revoke(&mut book, &mut pool, &obj(1), 2).unwrap();
        // 900 refunded to ns(1)
        assert_eq!(archived.patronage_score, 100);
        assert!(book.is_empty());
        // ns(1) credit = 1_000_000 - 1_000 + 900 = 999_900 (minus what went to patronage_ns)
        // pool ns(1) gets 900 back; total pool = 999_900 + 100 (in patronage_ns)
        assert_eq!(pool.total_accrued(), 1_000_000 - 100); // only donated amount is "spent"
    }

    #[test]
    fn revoke_unknown_object_returns_error() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        let err = revoke(&mut book, &mut pool, &obj(99), 0).unwrap_err();
        assert!(matches!(err, CovenantError::UnknownCovenant(_)));
    }

    // ── is_immune ─────────────────────────────────────────────────────────────

    #[test]
    fn is_immune_true_while_active() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        pledge(&mut book, &mut pool, obj(1), ns(1), 100, 5, 0).unwrap();
        assert!(is_immune(&book, &obj(1), 0));
        assert!(is_immune(&book, &obj(1), 4));
        assert!(!is_immune(&book, &obj(1), 5)); // expires_epoch exclusive
        assert!(!is_immune(&book, &obj(99), 1)); // no covenant at all
    }

    // ── expire_all ────────────────────────────────────────────────────────────

    #[test]
    fn expire_all_removes_expired_covenants() {
        let mut book = fresh_book();
        let mut pool = pool_with(ns(1), 1_000_000);
        pledge(&mut book, &mut pool, obj(1), ns(1), 100, 3, 0).unwrap();
        pledge(&mut book, &mut pool, obj(2), ns(1), 100, 10, 0).unwrap();
        let expired = book.expire_all(5); // epoch 5 → expires_epoch ≤ 5
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].object_id, obj(1));
        assert_eq!(book.len(), 1); // obj(2) still active
    }

    // ── conservation invariant ────────────────────────────────────────────────

    #[test]
    fn conservation_honour_then_revoke() {
        let mut book = fresh_book();
        let initial = 10_000u64;
        let mut pool = pool_with(ns(1), initial);
        pledge(&mut book, &mut pool, obj(1), ns(1), 50, 20, 0).unwrap();
        // honour 3 epochs
        for ep in 1..=3 {
            honour(&mut book, &mut pool, &obj(1), ep).unwrap();
        }
        revoke(&mut book, &mut pool, &obj(1), 4).unwrap();
        // 3 × 50 = 150 in patronage_ns; rest back in ns(1).
        // total_accrued should equal initial (energy conserved).
        assert_eq!(pool.total_accrued(), initial);
    }
}
