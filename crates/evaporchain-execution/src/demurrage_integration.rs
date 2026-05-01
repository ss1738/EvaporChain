//! Per-epoch demurrage sweep — wires `evaporchain-demurrage` into the
//! execution layer.
//!
//! Called once per epoch (gated by `block.epoch > db.get_last_rent_epoch()`,
//! the same guard as `collect_storage_rent`) so accounts are charged
//! holding fees at the end of each epoch.
//!
//! # Conservation invariant
//!
//! For each account the owed amount is debited from `account.balance` and
//! credited to the `RefreshPool` under the account's address as namespace.
//! Σ(account.balance) decreases by exactly Σ(owed); `RefreshPool.total_accrued`
//! increases by the same amount.  The conservation audit in `energy_audit`
//! captures this as a legitimate Demurrage redirect (not a conservation
//! violation — it is the intended per-epoch leak into the refresh pool).
//!
//! Accounts below `DemurrageParams.threshold` pay nothing — small balances
//! are never charged (per §4.1 #6 doctrine).

use evaporchain_demurrage::{demurrage_owed, DemurrageParams};
use evaporchain_energy_kernel::RefreshPool;
use evaporchain_state::db::StateDB;
use tracing::debug;

/// Apply demurrage to every account in the StateDB for one epoch transition.
///
/// Returns the total demurrage collected across all accounts.
pub fn collect_demurrage(
    db: &mut dyn StateDB,
    pool: &mut RefreshPool,
    params: &DemurrageParams,
    last_epoch: u64,
    current_epoch: u64,
) -> u64 {
    if current_epoch <= last_epoch {
        return 0;
    }

    let addrs = db.all_account_addresses();
    let mut total_collected: u64 = 0;

    for addr in addrs {
        let (balance, owed) = {
            let Some(acct) = db.get_account(&addr) else {
                continue;
            };
            let b = acct.balance;
            let o = demurrage_owed(b, last_epoch, current_epoch, params);
            (b, o)
        };

        if owed == 0 {
            continue;
        }

        // Debit the account; clamp to balance (guards against rounding edge cases).
        let actual = owed.min(balance);
        if actual == 0 {
            continue;
        }

        if let Some(acct) = db.get_account_mut(&addr) {
            acct.balance = acct.balance.saturating_sub(actual);
        }

        // Credit the refresh pool under the account address as namespace.
        pool.accrue(addr.to_vec(), actual, current_epoch);
        total_collected = total_collected.saturating_add(actual);
    }

    debug!(
        epoch = current_epoch,
        collected = total_collected,
        "demurrage sweep complete"
    );

    total_collected
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_types::{Account, AccountAddress};

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    fn make_db(balances: &[(u8, u64)]) -> InMemoryStateDB {
        let mut db = InMemoryStateDB::new();
        for &(b, bal) in balances {
            db.put_account(Account {
                address: addr(b),
                balance: bal,
                nonce: 0,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
            });
        }
        db
    }

    #[test]
    fn below_threshold_no_charge() {
        // Default params: threshold = 1024 EPV. Balance below → 0 demurrage.
        let mut db = make_db(&[(1, 512)]);
        let mut pool = RefreshPool::new();
        let params = DemurrageParams::default();
        let collected = collect_demurrage(&mut db, &mut pool, &params, 0, 1);
        assert_eq!(collected, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 512);
    }

    #[test]
    fn above_threshold_charges_and_credits_pool() {
        let mut db = make_db(&[(1, 10_000_000)]);
        let mut pool = RefreshPool::new();
        let params = DemurrageParams::new(100, 1_000); // aggressive for test
        let collected = collect_demurrage(&mut db, &mut pool, &params, 0, 10);
        assert!(
            collected > 0,
            "should charge non-zero demurrage on large balance"
        );
        assert_eq!(pool.total_accrued(), collected);
        let new_bal = db.get_account(&addr(1)).unwrap().balance;
        assert_eq!(new_bal, 10_000_000 - collected);
    }

    #[test]
    fn same_epoch_no_charge() {
        let mut db = make_db(&[(1, 10_000_000)]);
        let mut pool = RefreshPool::new();
        let params = DemurrageParams::default();
        let collected = collect_demurrage(&mut db, &mut pool, &params, 5, 5);
        assert_eq!(collected, 0);
    }

    #[test]
    fn multi_account_independent_charges() {
        let mut db = make_db(&[(1, 10_000_000), (2, 10_000_000), (3, 100)]);
        let mut pool = RefreshPool::new();
        let params = DemurrageParams::new(100, 1_000);
        let collected = collect_demurrage(&mut db, &mut pool, &params, 0, 10);
        // Accounts 1 and 2 should be charged; account 3 (100 < threshold) should not.
        let bal1 = db.get_account(&addr(1)).unwrap().balance;
        let bal2 = db.get_account(&addr(2)).unwrap().balance;
        let bal3 = db.get_account(&addr(3)).unwrap().balance;
        assert!(bal1 < 10_000_000);
        assert!(bal2 < 10_000_000);
        assert_eq!(bal3, 100, "small account should be untouched");
        assert_eq!(pool.total_accrued(), collected);
    }
}
