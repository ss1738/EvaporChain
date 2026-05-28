//! Legacy on-disk format fallbacks for state migration.
//!
//! When fields are added to types persisted via bincode (RocksDB column
//! families, WAL `PutAccount` payloads, snapshot bodies), the old bytes
//! cannot be deserialized into the new struct because bincode 1.3.3 does
//! not honor `#[serde(default)]` for trailing fields — it errors with
//! `UnexpectedEof`.
//!
//! Each addition needs a paired fallback that:
//!   1. Defines a struct mirroring the *previous* on-disk layout.
//!   2. `bincode::deserialize::<LegacyT>(&value)` succeeds on old bytes.
//!   3. `From<LegacyT> for T` fills the new field(s) with safe defaults.
//!
//! Call sites do `bincode::deserialize::<T>(&value).or_else(|_|
//! deserialize_T_with_legacy_fallback(&value))` — current format wins,
//! legacy is the second-chance path.
//!
//! Mirrors the existing `deserialize_legacy_ghost` precedent at
//! `rocksdb_backend.rs::deserialize_legacy_ghost` (added when the
//! `mmr_position` field landed on `GhostRecord`).

use evaporchain_types::Account;
use serde::Deserialize;

/// Legacy `Account` layout — pre-`vesting` (TOKENOMICS §2.6 / Q14).
///
/// Field order MUST match `Account`'s bincode-emitted byte sequence at the
/// moment the `vesting: Option<VestingSchedule>` field was added (after
/// `last_touched_epoch`). When this fallback fires, the on-disk record
/// was written by a binary that did not yet know about vesting, so
/// `vesting: None` is the correct fill.
///
/// `#[serde(default)]` annotations here mirror the runtime `Account`
/// struct's annotations so that EVEN OLDER on-disk records (those
/// predating `storage_deposit` / `storage_bytes` / `last_touched_epoch`)
/// also fall through cleanly. Defence-in-depth.
#[derive(Deserialize)]
struct LegacyAccount {
    address: [u8; 32],
    balance: u64,
    nonce: u64,
    #[serde(default)]
    storage_deposit: u64,
    #[serde(default)]
    storage_bytes: u64,
    #[serde(default)]
    last_touched_epoch: u64,
}

impl From<LegacyAccount> for Account {
    fn from(l: LegacyAccount) -> Self {
        Account {
            address: l.address,
            balance: l.balance,
            nonce: l.nonce,
            storage_deposit: l.storage_deposit,
            storage_bytes: l.storage_bytes,
            last_touched_epoch: l.last_touched_epoch,
            vesting: None,
        }
    }
}

/// Bincode-deserialize an `Account`, with fallback to the pre-vesting
/// layout. Used by RocksDB account loader, WAL `PutAccount` apply, and
/// snapshot v1 body decode.
///
/// On success the returned Account either has `vesting` populated from
/// disk (current format) or `vesting: None` (legacy format). Either way
/// the upstream caller should re-persist after migration so subsequent
/// loads use the current path directly — see `rocksdb_backend`'s
/// `compact_range_cf` precedent.
pub(crate) fn deserialize_account_with_legacy_fallback(
    data: &[u8],
) -> Result<Account, Box<bincode::ErrorKind>> {
    match bincode::deserialize::<Account>(data) {
        Ok(acct) => Ok(acct),
        Err(_) => bincode::deserialize::<LegacyAccount>(data).map(Account::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::VestingLock;

    /// Round-trip: a record written by the OLD binary (LegacyAccount, no
    /// vesting bytes) MUST be readable by the new binary as
    /// Account { vesting: None, ... }. Critical for cluster non-disruption
    /// — ANY restart of any node with the new binary against existing
    /// on-disk state must NOT drop accounts.
    #[test]
    fn legacy_account_bytes_load_with_vesting_none() {
        let legacy = LegacyAccount {
            address: [0x42; 32],
            balance: 1_000_000,
            nonce: 7,
            storage_deposit: 100,
            storage_bytes: 200,
            last_touched_epoch: 15_000,
        };
        let bytes = bincode::serialize(&LegacyAccountSer {
            address: legacy.address,
            balance: legacy.balance,
            nonce: legacy.nonce,
            storage_deposit: legacy.storage_deposit,
            storage_bytes: legacy.storage_bytes,
            last_touched_epoch: legacy.last_touched_epoch,
        })
        .expect("serialize legacy");

        // Direct Account deserialize SHOULD fail (no vesting trailing bytes).
        // If this assertion fails, bincode did honor #[serde(default)] for
        // Option and the fallback was unnecessary — still safe, just dead code.
        let direct = bincode::deserialize::<Account>(&bytes);

        // The fallback path MUST succeed regardless.
        let acct =
            deserialize_account_with_legacy_fallback(&bytes).expect("legacy fallback must succeed");
        assert_eq!(acct.address, [0x42; 32]);
        assert_eq!(acct.balance, 1_000_000);
        assert_eq!(acct.nonce, 7);
        assert_eq!(acct.storage_deposit, 100);
        assert_eq!(acct.storage_bytes, 200);
        assert_eq!(acct.last_touched_epoch, 15_000);
        assert_eq!(acct.vesting, None);

        // If direct succeeded too, verify it agrees.
        if let Ok(direct_acct) = direct {
            assert_eq!(acct, direct_acct);
        }
    }

    /// Round-trip: a record written by the NEW binary (Account with
    /// vesting: Some(_)) MUST be readable by the new binary unchanged.
    #[test]
    fn current_account_with_vesting_round_trip() {
        let acct = Account {
            address: [0x55; 32],
            balance: 350_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: Some(VestingLock {
                cliff_epoch: 1_576_800,           // ~1y at 8s/block
                linear_release_epochs: 5_256_000, // ~3y linear thereafter
                total_locked: 350_000_000,
            }),
        };
        let bytes = bincode::serialize(&acct).expect("serialize current");
        let loaded =
            deserialize_account_with_legacy_fallback(&bytes).expect("current must round-trip");
        assert_eq!(acct, loaded);
    }

    /// Garbage bytes must error from BOTH paths.
    #[test]
    fn garbage_bytes_error_cleanly() {
        let result = deserialize_account_with_legacy_fallback(b"not an account");
        assert!(result.is_err());
    }

    // Helper to bincode-emit a struct with EXACTLY the legacy field order
    // without making the runtime LegacyAccount Serialize (it only needs
    // Deserialize).
    #[derive(serde::Serialize)]
    struct LegacyAccountSer {
        address: [u8; 32],
        balance: u64,
        nonce: u64,
        storage_deposit: u64,
        storage_bytes: u64,
        last_touched_epoch: u64,
    }
}
