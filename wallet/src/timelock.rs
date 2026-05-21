// wallet/src/timelock.rs — Time-locked transactions and vesting schedules
//
// - Create timelocks: funds locked until a specific timestamp
// - Vesting schedules: cliff + linear release over time
// - Cancellable/non-cancellable lock modes
// - Track and claim unlocked portions

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TimelockError {
    #[error("timelock not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("not yet unlocked: unlocks at {0}")]
    NotUnlocked(String),
    #[error("already claimed")]
    AlreadyClaimed,
    #[error("cannot cancel: lock is non-cancellable")]
    NonCancellable,
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Lock types ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockStatus {
    Locked,
    Unlocked,
    PartiallyClaimed,
    FullyClaimed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timelock {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub total_amount: u64,
    pub claimed_amount: u64,
    pub unlock_at: String,
    pub created_at: String,
    pub status: LockStatus,
    pub cancellable: bool,
    pub note: String,
}

impl Timelock {
    pub fn new(
        id: &str,
        sender: &str,
        recipient: &str,
        amount: u64,
        unlock_at: &str,
        cancellable: bool,
    ) -> Self {
        Self {
            id: id.to_string(),
            sender: sender.to_string(),
            recipient: recipient.to_string(),
            total_amount: amount,
            claimed_amount: 0,
            unlock_at: unlock_at.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            status: LockStatus::Locked,
            cancellable,
            note: String::new(),
        }
    }

    pub fn with_note(mut self, note: &str) -> Self {
        self.note = note.to_string();
        self
    }

    pub fn is_unlocked(&self, now: &str) -> bool {
        now >= self.unlock_at.as_str()
    }

    pub fn remaining(&self) -> u64 {
        self.total_amount.saturating_sub(self.claimed_amount)
    }

    pub fn claim(&mut self, now: &str) -> Result<u64, TimelockError> {
        if !self.is_unlocked(now) {
            return Err(TimelockError::NotUnlocked(self.unlock_at.clone()));
        }
        if self.status == LockStatus::FullyClaimed {
            return Err(TimelockError::AlreadyClaimed);
        }
        if self.status == LockStatus::Cancelled {
            return Err(TimelockError::AlreadyClaimed);
        }
        let claimable = self.remaining();
        self.claimed_amount = self.total_amount;
        self.status = LockStatus::FullyClaimed;
        Ok(claimable)
    }

    pub fn cancel(&mut self) -> Result<u64, TimelockError> {
        if !self.cancellable {
            return Err(TimelockError::NonCancellable);
        }
        if self.status == LockStatus::FullyClaimed || self.status == LockStatus::Cancelled {
            return Err(TimelockError::AlreadyClaimed);
        }
        let refund = self.remaining();
        self.status = LockStatus::Cancelled;
        Ok(refund)
    }
}

// ── Vesting schedule ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VestingSchedule {
    pub id: String,
    pub beneficiary: String,
    pub total_amount: u64,
    pub claimed_amount: u64,
    pub start_at: String,
    pub cliff_at: String,
    pub end_at: String,
    pub created_at: String,
    pub status: LockStatus,
    pub note: String,
}

impl VestingSchedule {
    pub fn new(
        id: &str,
        beneficiary: &str,
        total_amount: u64,
        start_at: &str,
        cliff_at: &str,
        end_at: &str,
    ) -> Result<Self, TimelockError> {
        if start_at > cliff_at {
            return Err(TimelockError::InvalidConfig(
                "cliff must be after start".into(),
            ));
        }
        if cliff_at > end_at {
            return Err(TimelockError::InvalidConfig(
                "end must be after cliff".into(),
            ));
        }
        Ok(Self {
            id: id.to_string(),
            beneficiary: beneficiary.to_string(),
            total_amount,
            claimed_amount: 0,
            start_at: start_at.to_string(),
            cliff_at: cliff_at.to_string(),
            end_at: end_at.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            status: LockStatus::Locked,
            note: String::new(),
        })
    }

    pub fn with_note(mut self, note: &str) -> Self {
        self.note = note.to_string();
        self
    }

    /// Calculate how much has vested at a given timestamp (epoch seconds)
    pub fn vested_at(&self, now_epoch: i64) -> u64 {
        let start = parse_epoch(&self.start_at).unwrap_or(0);
        let cliff = parse_epoch(&self.cliff_at).unwrap_or(0);
        let end = parse_epoch(&self.end_at).unwrap_or(0);

        if now_epoch < cliff {
            return 0; // Before cliff: nothing vested
        }
        if now_epoch >= end {
            return self.total_amount; // After end: fully vested
        }
        // Linear vesting between start and end
        let total_duration = end - start;
        if total_duration <= 0 {
            return self.total_amount;
        }
        let elapsed = now_epoch - start;
        let fraction = elapsed as f64 / total_duration as f64;
        (self.total_amount as f64 * fraction) as u64
    }

    /// How much can be claimed right now
    pub fn claimable_at(&self, now_epoch: i64) -> u64 {
        let vested = self.vested_at(now_epoch);
        vested.saturating_sub(self.claimed_amount)
    }

    /// Claim vested tokens
    pub fn claim(&mut self, now_epoch: i64) -> Result<u64, TimelockError> {
        let claimable = self.claimable_at(now_epoch);
        if claimable == 0 {
            if self.claimed_amount >= self.total_amount {
                return Err(TimelockError::AlreadyClaimed);
            }
            return Err(TimelockError::NotUnlocked(self.cliff_at.clone()));
        }
        self.claimed_amount += claimable;
        if self.claimed_amount >= self.total_amount {
            self.status = LockStatus::FullyClaimed;
        } else {
            self.status = LockStatus::PartiallyClaimed;
        }
        Ok(claimable)
    }

    pub fn remaining(&self) -> u64 {
        self.total_amount.saturating_sub(self.claimed_amount)
    }

    pub fn percent_vested(&self, now_epoch: i64) -> f64 {
        if self.total_amount == 0 {
            return 100.0;
        }
        (self.vested_at(now_epoch) as f64 / self.total_amount as f64) * 100.0
    }
}

fn parse_epoch(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp())
}

// ── Store ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimelockStore {
    pub timelocks: HashMap<String, Timelock>,
    pub vestings: HashMap<String, VestingSchedule>,
}

impl TimelockStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_timelock(&mut self, lock: Timelock) -> Result<(), TimelockError> {
        if self.timelocks.contains_key(&lock.id) {
            return Err(TimelockError::AlreadyExists(lock.id.clone()));
        }
        self.timelocks.insert(lock.id.clone(), lock);
        Ok(())
    }

    pub fn add_vesting(&mut self, vesting: VestingSchedule) -> Result<(), TimelockError> {
        if self.vestings.contains_key(&vesting.id) {
            return Err(TimelockError::AlreadyExists(vesting.id.clone()));
        }
        self.vestings.insert(vesting.id.clone(), vesting);
        Ok(())
    }

    pub fn get_timelock(&self, id: &str) -> Option<&Timelock> {
        self.timelocks.get(id)
    }

    pub fn get_vesting(&self, id: &str) -> Option<&VestingSchedule> {
        self.vestings.get(id)
    }

    pub fn get_timelock_mut(&mut self, id: &str) -> Option<&mut Timelock> {
        self.timelocks.get_mut(id)
    }

    pub fn get_vesting_mut(&mut self, id: &str) -> Option<&mut VestingSchedule> {
        self.vestings.get_mut(id)
    }

    pub fn list_timelocks(&self) -> Vec<&Timelock> {
        self.timelocks.values().collect()
    }

    pub fn list_vestings(&self) -> Vec<&VestingSchedule> {
        self.vestings.values().collect()
    }

    /// Get all locks for a given recipient
    pub fn locks_for(&self, recipient: &str) -> Vec<&Timelock> {
        self.timelocks
            .values()
            .filter(|l| l.recipient == recipient)
            .collect()
    }

    /// Get all vestings for a given beneficiary
    pub fn vestings_for(&self, beneficiary: &str) -> Vec<&VestingSchedule> {
        self.vestings
            .values()
            .filter(|v| v.beneficiary == beneficiary)
            .collect()
    }

    /// Total locked amount across all active timelocks
    pub fn total_locked(&self) -> u64 {
        self.timelocks
            .values()
            .filter(|l| l.status == LockStatus::Locked)
            .map(|l| l.remaining())
            .sum()
    }

    /// Total unvested amount across all active vestings
    pub fn total_unvested(&self) -> u64 {
        self.vestings
            .values()
            .filter(|v| v.status != LockStatus::FullyClaimed)
            .map(|v| v.remaining())
            .sum()
    }

    pub fn save(&self, path: &Path) -> Result<(), TimelockError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| TimelockError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| TimelockError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, TimelockError> {
        let data = std::fs::read_to_string(path).map_err(|e| TimelockError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| TimelockError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timelock_new() {
        let lock = Timelock::new("t1", "alice", "bob", 1000, "2099-01-01T00:00:00Z", true);
        assert_eq!(lock.total_amount, 1000);
        assert_eq!(lock.status, LockStatus::Locked);
        assert!(lock.cancellable);
    }

    #[test]
    fn test_timelock_not_yet_unlocked() {
        let lock = Timelock::new("t1", "alice", "bob", 1000, "2099-01-01T00:00:00Z", false);
        assert!(!lock.is_unlocked("2024-01-01T00:00:00Z"));
    }

    #[test]
    fn test_timelock_unlocked() {
        let lock = Timelock::new("t1", "alice", "bob", 1000, "2020-01-01T00:00:00Z", false);
        assert!(lock.is_unlocked("2024-01-01T00:00:00Z"));
    }

    #[test]
    fn test_timelock_claim() {
        let mut lock = Timelock::new("t1", "alice", "bob", 1000, "2020-01-01T00:00:00Z", false);
        let claimed = lock.claim("2024-01-01T00:00:00Z").unwrap();
        assert_eq!(claimed, 1000);
        assert_eq!(lock.status, LockStatus::FullyClaimed);
    }

    #[test]
    fn test_timelock_claim_too_early() {
        let mut lock = Timelock::new("t1", "alice", "bob", 1000, "2099-01-01T00:00:00Z", false);
        assert!(lock.claim("2024-01-01T00:00:00Z").is_err());
    }

    #[test]
    fn test_timelock_double_claim() {
        let mut lock = Timelock::new("t1", "alice", "bob", 1000, "2020-01-01T00:00:00Z", false);
        lock.claim("2024-01-01T00:00:00Z").unwrap();
        assert!(lock.claim("2024-01-01T00:00:00Z").is_err());
    }

    #[test]
    fn test_timelock_cancel() {
        let mut lock = Timelock::new("t1", "alice", "bob", 1000, "2099-01-01T00:00:00Z", true);
        let refund = lock.cancel().unwrap();
        assert_eq!(refund, 1000);
        assert_eq!(lock.status, LockStatus::Cancelled);
    }

    #[test]
    fn test_timelock_cancel_non_cancellable() {
        let mut lock = Timelock::new("t1", "alice", "bob", 1000, "2099-01-01T00:00:00Z", false);
        assert!(lock.cancel().is_err());
    }

    #[test]
    fn test_timelock_with_note() {
        let lock =
            Timelock::new("t1", "a", "b", 100, "2099-01-01T00:00:00Z", false).with_note("salary");
        assert_eq!(lock.note, "salary");
    }

    #[test]
    fn test_vesting_new() {
        let v = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(v.total_amount, 10000);
        assert_eq!(v.claimed_amount, 0);
    }

    #[test]
    fn test_vesting_invalid_cliff_before_start() {
        let res = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2025-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        );
        assert!(res.is_err());
    }

    #[test]
    fn test_vesting_invalid_end_before_cliff() {
        let res = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "2024-06-01T00:00:00Z",
        );
        assert!(res.is_err());
    }

    #[test]
    fn test_vesting_before_cliff() {
        let v = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        // Before cliff: 0 vested
        let epoch = chrono::DateTime::parse_from_rfc3339("2024-03-01T00:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(v.vested_at(epoch), 0);
    }

    #[test]
    fn test_vesting_after_end() {
        let v = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        let epoch = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(v.vested_at(epoch), 10000);
    }

    #[test]
    fn test_vesting_midway() {
        let v = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z", // No cliff
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        // Midpoint: ~50%
        let epoch = chrono::DateTime::parse_from_rfc3339("2024-07-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let vested = v.vested_at(epoch);
        // Should be roughly 4959-5000 depending on exact days
        assert!(vested > 4800 && vested < 5100);
    }

    #[test]
    fn test_vesting_claim() {
        let mut v = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        let epoch = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let claimed = v.claim(epoch).unwrap();
        assert_eq!(claimed, 10000);
        assert_eq!(v.status, LockStatus::FullyClaimed);
    }

    #[test]
    fn test_vesting_partial_claim() {
        let mut v = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        let mid = chrono::DateTime::parse_from_rfc3339("2024-07-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let claimed = v.claim(mid).unwrap();
        assert!(claimed > 0 && claimed < 10000);
        assert_eq!(v.status, LockStatus::PartiallyClaimed);

        // Claim rest
        let end = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let rest = v.claim(end).unwrap();
        assert_eq!(claimed + rest, 10000);
        assert_eq!(v.status, LockStatus::FullyClaimed);
    }

    #[test]
    fn test_vesting_percent() {
        let v = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        let end = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(v.percent_vested(end), 100.0);
    }

    #[test]
    fn test_store_add_timelock() {
        let mut store = TimelockStore::new();
        let lock = Timelock::new("t1", "a", "b", 100, "2099-01-01T00:00:00Z", false);
        store.add_timelock(lock).unwrap();
        assert_eq!(store.list_timelocks().len(), 1);
    }

    #[test]
    fn test_store_add_duplicate() {
        let mut store = TimelockStore::new();
        let lock = Timelock::new("t1", "a", "b", 100, "2099-01-01T00:00:00Z", false);
        store.add_timelock(lock.clone()).unwrap();
        assert!(store.add_timelock(lock).is_err());
    }

    #[test]
    fn test_store_add_vesting() {
        let mut store = TimelockStore::new();
        let v = VestingSchedule::new(
            "v1",
            "alice",
            10000,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        store.add_vesting(v).unwrap();
        assert_eq!(store.list_vestings().len(), 1);
    }

    #[test]
    fn test_store_locks_for() {
        let mut store = TimelockStore::new();
        store
            .add_timelock(Timelock::new(
                "t1",
                "a",
                "bob",
                100,
                "2099-01-01T00:00:00Z",
                false,
            ))
            .unwrap();
        store
            .add_timelock(Timelock::new(
                "t2",
                "a",
                "bob",
                200,
                "2099-01-01T00:00:00Z",
                false,
            ))
            .unwrap();
        store
            .add_timelock(Timelock::new(
                "t3",
                "a",
                "carol",
                300,
                "2099-01-01T00:00:00Z",
                false,
            ))
            .unwrap();
        assert_eq!(store.locks_for("bob").len(), 2);
        assert_eq!(store.locks_for("carol").len(), 1);
    }

    #[test]
    fn test_store_total_locked() {
        let mut store = TimelockStore::new();
        store
            .add_timelock(Timelock::new(
                "t1",
                "a",
                "b",
                100,
                "2099-01-01T00:00:00Z",
                false,
            ))
            .unwrap();
        store
            .add_timelock(Timelock::new(
                "t2",
                "a",
                "b",
                200,
                "2099-01-01T00:00:00Z",
                false,
            ))
            .unwrap();
        assert_eq!(store.total_locked(), 300);
    }

    #[test]
    fn test_store_save_load() {
        let path = std::env::temp_dir().join(format!("evap_timelock_{}.json", std::process::id()));
        let mut store = TimelockStore::new();
        store
            .add_timelock(Timelock::new(
                "t1",
                "a",
                "b",
                100,
                "2099-01-01T00:00:00Z",
                true,
            ))
            .unwrap();
        store.save(&path).unwrap();
        let loaded = TimelockStore::load(&path).unwrap();
        assert_eq!(loaded.list_timelocks().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    // ─── Additional coverage tests ─────────────────────────────────────────

    #[test]
    fn test_timelock_cancel_after_claimed_covers_line_115() {
        let mut lock = Timelock::new("t1", "a", "b", 100, "2020-01-01T00:00:00Z", true);
        lock.claim("2024-01-01T00:00:00Z").unwrap();
        assert!(matches!(
            lock.cancel().unwrap_err(),
            TimelockError::AlreadyClaimed
        ));
    }

    #[test]
    fn test_timelock_cancel_already_cancelled_covers_line_115() {
        let mut lock = Timelock::new("t1", "a", "b", 100, "2099-01-01T00:00:00Z", true);
        lock.cancel().unwrap();
        assert!(matches!(
            lock.cancel().unwrap_err(),
            TimelockError::AlreadyClaimed
        ));
    }

    #[test]
    fn test_timelock_claim_cancelled_lock_covers_line_102() {
        let mut lock = Timelock::new("t1", "a", "b", 100, "2020-01-01T00:00:00Z", true);
        lock.cancel().unwrap();
        assert!(matches!(
            lock.claim("2024-01-01T00:00:00Z").unwrap_err(),
            TimelockError::AlreadyClaimed
        ));
    }

    #[test]
    fn test_vesting_with_note_covers_lines_172_175() {
        let v = VestingSchedule::new(
            "v1",
            "alice",
            1000,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap()
        .with_note("vesting note");
        assert_eq!(v.note, "vesting note");
    }

    #[test]
    fn test_vested_at_zero_total_duration_covers_line_192() {
        // Directly construct to force total_duration = end - start < 0
        // cliff (Jan) < start (Jun) and end (May 31) < start (Jun)
        // so for now in March: now >= cliff, now < end → enters linear branch → total_duration < 0
        let v = VestingSchedule {
            id: "v1".to_string(),
            beneficiary: "alice".to_string(),
            total_amount: 500,
            claimed_amount: 0,
            start_at: "2024-06-01T00:00:00Z".to_string(),
            cliff_at: "2024-01-01T00:00:00Z".to_string(),
            end_at: "2024-05-31T00:00:00Z".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            status: LockStatus::Locked,
            note: String::new(),
        };
        let now = chrono::DateTime::parse_from_rfc3339("2024-03-01T00:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(v.vested_at(now), 500);
    }

    #[test]
    fn test_vesting_claim_before_cliff_covers_line_212() {
        let mut v = VestingSchedule::new(
            "v1",
            "alice",
            1000,
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        let pre_cliff = chrono::DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z")
            .unwrap()
            .timestamp();
        assert!(matches!(
            v.claim(pre_cliff).unwrap_err(),
            TimelockError::NotUnlocked(_)
        ));
    }

    #[test]
    fn test_vesting_claim_already_fully_claimed_covers_line_210() {
        let mut v = VestingSchedule::new(
            "v1",
            "alice",
            1000,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        let after_end = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .timestamp();
        v.claim(after_end).unwrap();
        assert!(matches!(
            v.claim(after_end).unwrap_err(),
            TimelockError::AlreadyClaimed
        ));
    }

    #[test]
    fn test_vesting_remaining_covers_lines_223_225() {
        let mut v = VestingSchedule::new(
            "v1",
            "alice",
            1000,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(v.remaining(), 1000);
        let mid = chrono::DateTime::parse_from_rfc3339("2024-07-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let claimed = v.claim(mid).unwrap();
        assert_eq!(v.remaining(), 1000 - claimed);
    }

    #[test]
    fn test_vesting_percent_zero_total_covers_line_228() {
        let v = VestingSchedule {
            id: "v1".to_string(),
            beneficiary: "alice".to_string(),
            total_amount: 0,
            claimed_amount: 0,
            start_at: "2024-01-01T00:00:00Z".to_string(),
            cliff_at: "2024-01-01T00:00:00Z".to_string(),
            end_at: "2025-01-01T00:00:00Z".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            status: LockStatus::Locked,
            note: String::new(),
        };
        let now = chrono::DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(v.percent_vested(now), 100.0);
    }

    #[test]
    fn test_store_add_vesting_duplicate_covers_line_264() {
        let mut store = TimelockStore::new();
        let v = VestingSchedule::new(
            "v1",
            "alice",
            1000,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        store.add_vesting(v.clone()).unwrap();
        assert!(matches!(
            store.add_vesting(v).unwrap_err(),
            TimelockError::AlreadyExists(_)
        ));
    }

    #[test]
    fn test_store_get_timelock_covers_lines_270_271() {
        let mut store = TimelockStore::new();
        let lock = Timelock::new("t1", "a", "b", 100, "2099-01-01T00:00:00Z", false);
        store.add_timelock(lock).unwrap();
        assert!(store.get_timelock("t1").is_some());
        assert!(store.get_timelock("missing").is_none());
    }

    #[test]
    fn test_store_get_vesting_covers_lines_274_275() {
        let mut store = TimelockStore::new();
        let v = VestingSchedule::new(
            "v1",
            "alice",
            1000,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        store.add_vesting(v).unwrap();
        assert!(store.get_vesting("v1").is_some());
        assert!(store.get_vesting("missing").is_none());
    }

    #[test]
    fn test_store_get_timelock_mut_covers_lines_278_279() {
        let mut store = TimelockStore::new();
        let lock = Timelock::new("t1", "a", "b", 100, "2099-01-01T00:00:00Z", false);
        store.add_timelock(lock).unwrap();
        let m = store.get_timelock_mut("t1").unwrap();
        m.note = "mutated".to_string();
        assert_eq!(store.get_timelock("t1").unwrap().note, "mutated");
    }

    #[test]
    fn test_store_get_vesting_mut_covers_lines_282_283() {
        let mut store = TimelockStore::new();
        let v = VestingSchedule::new(
            "v1",
            "alice",
            1000,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        store.add_vesting(v).unwrap();
        let m = store.get_vesting_mut("v1").unwrap();
        m.note = "mutated".to_string();
        assert_eq!(store.get_vesting("v1").unwrap().note, "mutated");
    }

    #[test]
    fn test_store_vestings_for_covers_lines_303_308() {
        let mut store = TimelockStore::new();
        for i in 0..3_u32 {
            let v = VestingSchedule::new(
                &format!("v{}", i),
                if i < 2 { "alice" } else { "bob" },
                1000,
                "2024-01-01T00:00:00Z",
                "2024-07-01T00:00:00Z",
                "2025-01-01T00:00:00Z",
            )
            .unwrap();
            store.add_vesting(v).unwrap();
        }
        assert_eq!(store.vestings_for("alice").len(), 2);
        assert_eq!(store.vestings_for("bob").len(), 1);
    }

    #[test]
    fn test_store_total_unvested_covers_lines_320_326() {
        let mut store = TimelockStore::new();
        let v = VestingSchedule::new(
            "v1",
            "alice",
            1000,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        store.add_vesting(v).unwrap();
        assert_eq!(store.total_unvested(), 1000);
        let after_end = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .timestamp();
        store.get_vesting_mut("v1").unwrap().claim(after_end).unwrap();
        assert_eq!(store.total_unvested(), 0);
    }

    #[test]
    fn test_store_load_or_default_covers_lines_339_341() {
        let store = TimelockStore::load_or_default(Path::new(
            "/tmp/nonexistent_timelock_doctor_xyzabc.json",
        ));
        assert!(store.timelocks.is_empty());
        assert!(store.vestings.is_empty());
    }
}
