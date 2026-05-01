// wallet/src/recovery.rs — Dead man's switch + social recovery
//
// Two recovery mechanisms:
//   1. Dead man's switch: auto-transfer if no check-in within deadline
//   2. Social recovery: M-of-N guardians can authorize recovery
//
// All state is persisted locally; on-chain execution is via tx pipeline.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("guardian not found: {0}")]
    GuardianNotFound(String),
    #[error("guardian already exists: {0}")]
    GuardianExists(String),
    #[error("not enough approvals: {0} of {1} required")]
    NotEnoughApprovals(usize, usize),
    #[error("recovery not active")]
    NotActive,
    #[error("recovery already active")]
    AlreadyActive,
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("deadline not reached")]
    DeadlineNotReached,
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Dead man's switch ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwitchStatus {
    Active,
    Triggered,
    Disabled,
    CheckedIn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadManSwitch {
    pub enabled: bool,
    pub beneficiary: String,
    pub check_in_interval_days: u32,
    pub last_check_in: String,
    pub deadline: String,
    pub transfer_amount: Option<u64>,
    pub transfer_all: bool,
    pub status: SwitchStatus,
    pub note: String,
}

impl DeadManSwitch {
    pub fn new(beneficiary: &str, interval_days: u32) -> Result<Self, RecoveryError> {
        if interval_days == 0 {
            return Err(RecoveryError::InvalidConfig(
                "interval must be > 0 days".into(),
            ));
        }
        let now = chrono::Utc::now();
        let deadline = now + chrono::Duration::days(interval_days as i64);
        Ok(Self {
            enabled: true,
            beneficiary: beneficiary.to_string(),
            check_in_interval_days: interval_days,
            last_check_in: now.to_rfc3339(),
            deadline: deadline.to_rfc3339(),
            transfer_amount: None,
            transfer_all: true,
            status: SwitchStatus::Active,
            note: String::new(),
        })
    }

    pub fn with_amount(mut self, amount: u64) -> Self {
        self.transfer_amount = Some(amount);
        self.transfer_all = false;
        self
    }

    pub fn with_note(mut self, note: &str) -> Self {
        self.note = note.to_string();
        self
    }

    /// Check in — reset the deadline
    pub fn check_in(&mut self) {
        let now = chrono::Utc::now();
        self.last_check_in = now.to_rfc3339();
        self.deadline =
            (now + chrono::Duration::days(self.check_in_interval_days as i64)).to_rfc3339();
        self.status = SwitchStatus::CheckedIn;
    }

    /// Is the deadline past?
    pub fn is_triggered(&self, now: &str) -> bool {
        self.enabled && now >= self.deadline.as_str()
    }

    /// Trigger the switch (marks as triggered)
    pub fn trigger(&mut self) -> Result<(), RecoveryError> {
        if !self.enabled {
            return Err(RecoveryError::NotActive);
        }
        self.status = SwitchStatus::Triggered;
        Ok(())
    }

    /// Disable the switch
    pub fn disable(&mut self) {
        self.enabled = false;
        self.status = SwitchStatus::Disabled;
    }

    /// Re-enable the switch
    pub fn enable(&mut self) {
        self.enabled = true;
        self.check_in();
    }

    /// Days remaining until deadline
    pub fn days_remaining(&self) -> i64 {
        if let Ok(deadline) = chrono::DateTime::parse_from_rfc3339(&self.deadline) {
            let now = chrono::Utc::now();
            (deadline.timestamp() - now.timestamp()) / 86400
        } else {
            0
        }
    }
}

// ── Social recovery ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guardian {
    pub address: String,
    pub name: String,
    pub added_at: String,
    pub last_active: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryStatus {
    Inactive,
    Pending,
    Approved,
    Executed,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRequest {
    pub id: String,
    pub new_owner: String,
    pub initiated_by: String,
    pub initiated_at: String,
    pub expires_at: String,
    pub approvals: Vec<String>,
    pub rejections: Vec<String>,
    pub status: RecoveryStatus,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialRecovery {
    pub guardians: HashMap<String, Guardian>,
    pub threshold: usize,
    pub recovery_delay_hours: u32,
    pub requests: Vec<RecoveryRequest>,
}

impl SocialRecovery {
    pub fn new(threshold: usize, delay_hours: u32) -> Result<Self, RecoveryError> {
        if threshold == 0 {
            return Err(RecoveryError::InvalidConfig("threshold must be > 0".into()));
        }
        Ok(Self {
            guardians: HashMap::new(),
            threshold,
            recovery_delay_hours: delay_hours,
            requests: Vec::new(),
        })
    }

    /// Add a guardian
    pub fn add_guardian(&mut self, address: &str, name: &str) -> Result<(), RecoveryError> {
        if self.guardians.contains_key(address) {
            return Err(RecoveryError::GuardianExists(address.into()));
        }
        self.guardians.insert(
            address.to_string(),
            Guardian {
                address: address.to_string(),
                name: name.to_string(),
                added_at: chrono::Utc::now().to_rfc3339(),
                last_active: None,
            },
        );
        Ok(())
    }

    /// Remove a guardian
    pub fn remove_guardian(&mut self, address: &str) -> Result<Guardian, RecoveryError> {
        self.guardians
            .remove(address)
            .ok_or_else(|| RecoveryError::GuardianNotFound(address.into()))
    }

    /// List guardians
    pub fn list_guardians(&self) -> Vec<&Guardian> {
        self.guardians.values().collect()
    }

    /// Initiate a recovery request
    pub fn initiate_recovery(
        &mut self,
        new_owner: &str,
        initiated_by: &str,
    ) -> Result<String, RecoveryError> {
        // Check initiator is a guardian
        if !self.guardians.contains_key(initiated_by) {
            return Err(RecoveryError::GuardianNotFound(initiated_by.into()));
        }

        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::hours(self.recovery_delay_hours as i64 + 168); // delay + 7 days

        let id = format!("rec-{}", self.requests.len() + 1);
        self.requests.push(RecoveryRequest {
            id: id.clone(),
            new_owner: new_owner.to_string(),
            initiated_by: initiated_by.to_string(),
            initiated_at: now.to_rfc3339(),
            expires_at: expires.to_rfc3339(),
            approvals: vec![initiated_by.to_string()],
            rejections: Vec::new(),
            status: RecoveryStatus::Pending,
            note: String::new(),
        });

        // Mark guardian active
        if let Some(g) = self.guardians.get_mut(initiated_by) {
            g.last_active = Some(now.to_rfc3339());
        }

        // Check if threshold already met (initiator counts as approval)
        if let Some(req) = self.requests.iter_mut().find(|r| r.id == id) {
            if req.approvals.len() >= self.threshold {
                req.status = RecoveryStatus::Approved;
            }
        }

        Ok(id)
    }

    /// Approve a recovery request
    pub fn approve(
        &mut self,
        request_id: &str,
        guardian_address: &str,
    ) -> Result<bool, RecoveryError> {
        if !self.guardians.contains_key(guardian_address) {
            return Err(RecoveryError::GuardianNotFound(guardian_address.into()));
        }

        let req = self
            .requests
            .iter_mut()
            .find(|r| r.id == request_id)
            .ok_or(RecoveryError::NotActive)?;

        if req.status != RecoveryStatus::Pending {
            return Err(RecoveryError::NotActive);
        }

        if !req.approvals.contains(&guardian_address.to_string()) {
            req.approvals.push(guardian_address.to_string());
        }

        // Mark guardian active
        if let Some(g) = self.guardians.get_mut(guardian_address) {
            g.last_active = Some(chrono::Utc::now().to_rfc3339());
        }

        // Check if threshold met
        if req.approvals.len() >= self.threshold {
            req.status = RecoveryStatus::Approved;
            return Ok(true);
        }

        Ok(false)
    }

    /// Reject a recovery request
    pub fn reject(
        &mut self,
        request_id: &str,
        guardian_address: &str,
    ) -> Result<(), RecoveryError> {
        if !self.guardians.contains_key(guardian_address) {
            return Err(RecoveryError::GuardianNotFound(guardian_address.into()));
        }

        let req = self
            .requests
            .iter_mut()
            .find(|r| r.id == request_id)
            .ok_or(RecoveryError::NotActive)?;

        if !req.rejections.contains(&guardian_address.to_string()) {
            req.rejections.push(guardian_address.to_string());
        }

        // If majority rejects, mark as rejected
        let total = self.guardians.len();
        if req.rejections.len() > total / 2 {
            req.status = RecoveryStatus::Rejected;
        }

        Ok(())
    }

    /// Mark a request as executed
    pub fn execute(&mut self, request_id: &str) -> Result<&RecoveryRequest, RecoveryError> {
        let req = self
            .requests
            .iter_mut()
            .find(|r| r.id == request_id)
            .ok_or(RecoveryError::NotActive)?;

        if req.status != RecoveryStatus::Approved {
            return Err(RecoveryError::NotEnoughApprovals(
                req.approvals.len(),
                self.threshold,
            ));
        }

        req.status = RecoveryStatus::Executed;
        Ok(req)
    }

    /// Get active (pending) recovery requests
    pub fn pending_requests(&self) -> Vec<&RecoveryRequest> {
        self.requests
            .iter()
            .filter(|r| r.status == RecoveryStatus::Pending)
            .collect()
    }

    /// Check if threshold is achievable with current guardian count
    pub fn is_valid(&self) -> bool {
        self.guardians.len() >= self.threshold
    }

    /// Approvals needed for a given request
    pub fn approvals_needed(&self, request_id: &str) -> Option<usize> {
        self.requests
            .iter()
            .find(|r| r.id == request_id)
            .map(|r| self.threshold.saturating_sub(r.approvals.len()))
    }
}

// ── Recovery store ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecoveryStore {
    pub dead_man_switch: Option<DeadManSwitch>,
    pub social_recovery: Option<SocialRecovery>,
}

impl RecoveryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn save(&self, path: &Path) -> Result<(), RecoveryError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| RecoveryError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| RecoveryError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, RecoveryError> {
        let data = std::fs::read_to_string(path).map_err(|e| RecoveryError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| RecoveryError::Json(e.to_string()))
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
    fn test_dead_man_switch_new() {
        let dms = DeadManSwitch::new("evap1beneficiary", 30).unwrap();
        assert!(dms.enabled);
        assert_eq!(dms.check_in_interval_days, 30);
        assert!(dms.transfer_all);
    }

    #[test]
    fn test_dead_man_switch_zero_interval() {
        assert!(DeadManSwitch::new("evap1x", 0).is_err());
    }

    #[test]
    fn test_dead_man_switch_with_amount() {
        let dms = DeadManSwitch::new("evap1x", 30).unwrap().with_amount(5000);
        assert_eq!(dms.transfer_amount, Some(5000));
        assert!(!dms.transfer_all);
    }

    #[test]
    fn test_dead_man_switch_check_in() {
        let mut dms = DeadManSwitch::new("evap1x", 30).unwrap();
        let old_deadline = dms.deadline.clone();
        std::thread::sleep(std::time::Duration::from_millis(10));
        dms.check_in();
        assert_ne!(dms.deadline, old_deadline);
        assert_eq!(dms.status, SwitchStatus::CheckedIn);
    }

    #[test]
    fn test_dead_man_switch_not_triggered() {
        let dms = DeadManSwitch::new("evap1x", 30).unwrap();
        assert!(!dms.is_triggered(&chrono::Utc::now().to_rfc3339()));
    }

    #[test]
    fn test_dead_man_switch_triggered() {
        let dms = DeadManSwitch::new("evap1x", 30).unwrap();
        assert!(dms.is_triggered("2099-01-01T00:00:00Z"));
    }

    #[test]
    fn test_dead_man_switch_trigger() {
        let mut dms = DeadManSwitch::new("evap1x", 30).unwrap();
        dms.trigger().unwrap();
        assert_eq!(dms.status, SwitchStatus::Triggered);
    }

    #[test]
    fn test_dead_man_switch_disable_enable() {
        let mut dms = DeadManSwitch::new("evap1x", 30).unwrap();
        dms.disable();
        assert!(!dms.enabled);
        assert_eq!(dms.status, SwitchStatus::Disabled);
        dms.enable();
        assert!(dms.enabled);
    }

    #[test]
    fn test_dead_man_switch_days_remaining() {
        let dms = DeadManSwitch::new("evap1x", 30).unwrap();
        let remaining = dms.days_remaining();
        assert!(remaining >= 29 && remaining <= 30);
    }

    #[test]
    fn test_social_recovery_new() {
        let sr = SocialRecovery::new(2, 24).unwrap();
        assert_eq!(sr.threshold, 2);
        assert_eq!(sr.recovery_delay_hours, 24);
    }

    #[test]
    fn test_social_recovery_zero_threshold() {
        assert!(SocialRecovery::new(0, 24).is_err());
    }

    #[test]
    fn test_add_guardian() {
        let mut sr = SocialRecovery::new(2, 24).unwrap();
        sr.add_guardian("evap1guard1", "Alice").unwrap();
        sr.add_guardian("evap1guard2", "Bob").unwrap();
        assert_eq!(sr.list_guardians().len(), 2);
    }

    #[test]
    fn test_add_duplicate_guardian() {
        let mut sr = SocialRecovery::new(2, 24).unwrap();
        sr.add_guardian("evap1guard1", "Alice").unwrap();
        assert!(sr.add_guardian("evap1guard1", "Alice2").is_err());
    }

    #[test]
    fn test_remove_guardian() {
        let mut sr = SocialRecovery::new(1, 24).unwrap();
        sr.add_guardian("evap1guard1", "Alice").unwrap();
        let g = sr.remove_guardian("evap1guard1").unwrap();
        assert_eq!(g.name, "Alice");
        assert!(sr.guardians.is_empty());
    }

    #[test]
    fn test_remove_guardian_not_found() {
        let mut sr = SocialRecovery::new(1, 24).unwrap();
        assert!(sr.remove_guardian("nope").is_err());
    }

    #[test]
    fn test_initiate_recovery() {
        let mut sr = SocialRecovery::new(2, 24).unwrap();
        sr.add_guardian("evap1g1", "G1").unwrap();
        sr.add_guardian("evap1g2", "G2").unwrap();
        let id = sr.initiate_recovery("evap1newowner", "evap1g1").unwrap();
        assert_eq!(sr.pending_requests().len(), 1);
        assert_eq!(sr.approvals_needed(&id), Some(1)); // 2 threshold, 1 approval (initiator)
    }

    #[test]
    fn test_initiate_recovery_non_guardian() {
        let mut sr = SocialRecovery::new(2, 24).unwrap();
        assert!(sr.initiate_recovery("evap1new", "evap1rando").is_err());
    }

    #[test]
    fn test_approve_recovery() {
        let mut sr = SocialRecovery::new(2, 24).unwrap();
        sr.add_guardian("evap1g1", "G1").unwrap();
        sr.add_guardian("evap1g2", "G2").unwrap();
        let id = sr.initiate_recovery("evap1new", "evap1g1").unwrap();
        let approved = sr.approve(&id, "evap1g2").unwrap();
        assert!(approved); // 2/2 threshold met
        let req = sr.requests.iter().find(|r| r.id == id).unwrap();
        assert_eq!(req.status, RecoveryStatus::Approved);
    }

    #[test]
    fn test_approve_not_enough() {
        let mut sr = SocialRecovery::new(3, 24).unwrap();
        sr.add_guardian("evap1g1", "G1").unwrap();
        sr.add_guardian("evap1g2", "G2").unwrap();
        sr.add_guardian("evap1g3", "G3").unwrap();
        let id = sr.initiate_recovery("evap1new", "evap1g1").unwrap();
        let approved = sr.approve(&id, "evap1g2").unwrap();
        assert!(!approved); // 2/3 — not enough yet
    }

    #[test]
    fn test_reject_recovery() {
        let mut sr = SocialRecovery::new(2, 24).unwrap();
        sr.add_guardian("evap1g1", "G1").unwrap();
        sr.add_guardian("evap1g2", "G2").unwrap();
        sr.add_guardian("evap1g3", "G3").unwrap();
        let id = sr.initiate_recovery("evap1new", "evap1g1").unwrap();
        sr.reject(&id, "evap1g2").unwrap();
        sr.reject(&id, "evap1g3").unwrap();
        let req = sr.requests.iter().find(|r| r.id == id).unwrap();
        assert_eq!(req.status, RecoveryStatus::Rejected);
    }

    #[test]
    fn test_execute_recovery() {
        let mut sr = SocialRecovery::new(1, 0).unwrap();
        sr.add_guardian("evap1g1", "G1").unwrap();
        let id = sr.initiate_recovery("evap1new", "evap1g1").unwrap();
        // Already has 1 approval (initiator), threshold=1
        let req = sr.execute(&id).unwrap();
        assert_eq!(req.status, RecoveryStatus::Executed);
    }

    #[test]
    fn test_execute_not_approved() {
        let mut sr = SocialRecovery::new(2, 24).unwrap();
        sr.add_guardian("evap1g1", "G1").unwrap();
        sr.add_guardian("evap1g2", "G2").unwrap();
        let id = sr.initiate_recovery("evap1new", "evap1g1").unwrap();
        assert!(sr.execute(&id).is_err());
    }

    #[test]
    fn test_is_valid() {
        let mut sr = SocialRecovery::new(2, 24).unwrap();
        assert!(!sr.is_valid()); // No guardians
        sr.add_guardian("evap1g1", "G1").unwrap();
        assert!(!sr.is_valid()); // Only 1, need 2
        sr.add_guardian("evap1g2", "G2").unwrap();
        assert!(sr.is_valid()); // 2 >= 2
    }

    #[test]
    fn test_recovery_store_save_load() {
        let path = std::env::temp_dir().join(format!("evap_recovery_{}.json", std::process::id()));
        let mut store = RecoveryStore::new();
        store.dead_man_switch = Some(DeadManSwitch::new("evap1x", 30).unwrap());
        store.save(&path).unwrap();
        let loaded = RecoveryStore::load(&path).unwrap();
        assert!(loaded.dead_man_switch.is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_recovery_store_load_or_default() {
        let path = std::env::temp_dir().join("evap_recovery_noexist.json");
        let store = RecoveryStore::load_or_default(&path);
        assert!(store.dead_man_switch.is_none());
        assert!(store.social_recovery.is_none());
    }
}
