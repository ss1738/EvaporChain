//! Guardian-based social recovery system for EvaporChain wallet.
//!
//! Allows wallet owners to designate trusted guardians who can collectively
//! authorize recovery of a wallet to a new owner key. Implements:
//!   - Guardian management (add, remove, revoke)
//!   - Threshold-based recovery requests with timelock
//!   - Approval / rejection workflow
//!   - Auto-expiration of stale requests
//!
//! All state is persisted to JSON on disk.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum SocialRecoveryError {
    #[error("guardian not found: {0}")]
    GuardianNotFound(String),

    #[error("guardian already exists: {0}")]
    GuardianAlreadyExists(String),

    #[error("maximum guardians reached ({0})")]
    MaxGuardiansReached(usize),

    #[error("request not found: {0}")]
    RequestNotFound(String),

    #[error("not enough active guardians for threshold ({active} < {threshold})")]
    NotEnoughGuardians { active: usize, threshold: u32 },

    #[error("guardian already approved: {0}")]
    AlreadyApproved(String),

    #[error("guardian already rejected: {0}")]
    AlreadyRejected(String),

    #[error("request not in progress")]
    RequestNotInProgress,

    #[error("request not approved")]
    RequestNotApproved,

    #[error("timelock not passed")]
    TimelockNotPassed,

    #[error("IO error: {0}")]
    Io(String),

    #[error("JSON error: {0}")]
    Json(String),
}

// ──────────────────────────── Types ────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianStatus {
    Active,
    Pending,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecoveryStatus {
    NotStarted,
    InProgress,
    Approved,
    Rejected,
    Completed,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guardian {
    pub id: String,
    pub name: String,
    pub address: String,
    pub public_key: String,
    pub status: GuardianStatus,
    pub added_at: String,
    pub last_confirmed: Option<String>,
    pub trust_score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRequest {
    pub id: String,
    pub requester: String,
    pub new_owner_key: String,
    pub status: RecoveryStatus,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub timelock_until: String,
    pub approvals: Vec<GuardianApproval>,
    pub rejections: Vec<String>,
    pub threshold: u32,
    pub required_approvals: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianApproval {
    pub guardian_id: String,
    pub timestamp: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryConfig {
    pub threshold: u32,
    pub timelock_hours: u64,
    pub max_guardians: usize,
    pub auto_expire_hours: u64,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            threshold: 2,
            timelock_hours: 48,
            max_guardians: 7,
            auto_expire_hours: 168,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialRecoveryStats {
    pub total_guardians: usize,
    pub active_guardians: usize,
    pub total_requests: usize,
    pub completed_recoveries: usize,
    pub rejected_recoveries: usize,
    pub pending_requests: usize,
}

// ──────────────────────────── Manager ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SocialRecoveryManager {
    pub guardians: HashMap<String, Guardian>,
    pub requests: HashMap<String, RecoveryRequest>,
    pub config: RecoveryConfig,
}

impl SocialRecoveryManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Guardian management ──────────────────────────────────────

    pub fn add_guardian(&mut self, guardian: Guardian) -> Result<(), SocialRecoveryError> {
        if self.guardians.contains_key(&guardian.id) {
            return Err(SocialRecoveryError::GuardianAlreadyExists(
                guardian.id.clone(),
            ));
        }
        if self.guardians.len() >= self.config.max_guardians {
            return Err(SocialRecoveryError::MaxGuardiansReached(
                self.config.max_guardians,
            ));
        }
        self.guardians.insert(guardian.id.clone(), guardian);
        Ok(())
    }

    pub fn remove_guardian(&mut self, id: &str) -> Result<Guardian, SocialRecoveryError> {
        self.guardians
            .remove(id)
            .ok_or_else(|| SocialRecoveryError::GuardianNotFound(id.to_string()))
    }

    pub fn get_guardian(&self, id: &str) -> Option<&Guardian> {
        self.guardians.get(id)
    }

    pub fn active_guardians(&self) -> Vec<&Guardian> {
        self.guardians
            .values()
            .filter(|g| g.status == GuardianStatus::Active)
            .collect()
    }

    pub fn revoke_guardian(&mut self, id: &str) -> Result<(), SocialRecoveryError> {
        let guardian = self
            .guardians
            .get_mut(id)
            .ok_or_else(|| SocialRecoveryError::GuardianNotFound(id.to_string()))?;
        guardian.status = GuardianStatus::Revoked;
        Ok(())
    }

    // ── Recovery workflow ────────────────────────────────────────

    pub fn initiate_recovery(
        &mut self,
        requester: &str,
        new_owner_key: &str,
    ) -> Result<String, SocialRecoveryError> {
        let active_count = self.active_guardians().len();
        if (active_count as u32) < self.config.threshold {
            return Err(SocialRecoveryError::NotEnoughGuardians {
                active: active_count,
                threshold: self.config.threshold,
            });
        }

        let now = chrono::Utc::now();
        let timelock_until = now + chrono::Duration::hours(self.config.timelock_hours as i64);
        // Two consecutive initiate_recovery calls inside the same nanosecond
        // (test_pending_requests does exactly that) must produce distinct
        // ids — otherwise HashMap::insert silently collides. Mix in a
        // process-wide monotonic counter.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("recovery_{}_{}", now.timestamp_nanos_opt().unwrap_or(0), n);

        let request = RecoveryRequest {
            id: id.clone(),
            requester: requester.to_string(),
            new_owner_key: new_owner_key.to_string(),
            status: RecoveryStatus::InProgress,
            created_at: now.to_rfc3339(),
            completed_at: None,
            timelock_until: timelock_until.to_rfc3339(),
            approvals: Vec::new(),
            rejections: Vec::new(),
            threshold: self.config.threshold,
            required_approvals: self.config.threshold,
        };

        self.requests.insert(id.clone(), request);
        Ok(id)
    }

    pub fn approve_recovery(
        &mut self,
        request_id: &str,
        guardian_id: &str,
        signature: &str,
    ) -> Result<(), SocialRecoveryError> {
        // Verify guardian exists and is active
        let guardian = self
            .guardians
            .get(guardian_id)
            .ok_or_else(|| SocialRecoveryError::GuardianNotFound(guardian_id.to_string()))?;
        if guardian.status != GuardianStatus::Active {
            return Err(SocialRecoveryError::GuardianNotFound(
                guardian_id.to_string(),
            ));
        }

        let request = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| SocialRecoveryError::RequestNotFound(request_id.to_string()))?;

        if request.status != RecoveryStatus::InProgress {
            return Err(SocialRecoveryError::RequestNotInProgress);
        }

        // Check for duplicate approval
        if request
            .approvals
            .iter()
            .any(|a| a.guardian_id == guardian_id)
        {
            return Err(SocialRecoveryError::AlreadyApproved(
                guardian_id.to_string(),
            ));
        }

        let approval = GuardianApproval {
            guardian_id: guardian_id.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            signature: signature.to_string(),
        };
        request.approvals.push(approval);

        // Auto-approve if threshold met and timelock passed
        if request.approvals.len() as u32 >= request.required_approvals {
            let timelock_dt = chrono::DateTime::parse_from_rfc3339(&request.timelock_until)
                .unwrap_or_else(|_| chrono::Utc::now().into());
            if chrono::Utc::now() >= timelock_dt {
                request.status = RecoveryStatus::Approved;
            }
        }

        Ok(())
    }

    pub fn reject_recovery(
        &mut self,
        request_id: &str,
        guardian_id: &str,
    ) -> Result<(), SocialRecoveryError> {
        let _guardian = self
            .guardians
            .get(guardian_id)
            .ok_or_else(|| SocialRecoveryError::GuardianNotFound(guardian_id.to_string()))?;

        let request = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| SocialRecoveryError::RequestNotFound(request_id.to_string()))?;

        if request.status != RecoveryStatus::InProgress {
            return Err(SocialRecoveryError::RequestNotInProgress);
        }

        if request.rejections.contains(&guardian_id.to_string()) {
            return Err(SocialRecoveryError::AlreadyRejected(
                guardian_id.to_string(),
            ));
        }

        request.rejections.push(guardian_id.to_string());
        Ok(())
    }

    pub fn complete_recovery(&mut self, request_id: &str) -> Result<(), SocialRecoveryError> {
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| SocialRecoveryError::RequestNotFound(request_id.to_string()))?;

        if request.status != RecoveryStatus::Approved {
            return Err(SocialRecoveryError::RequestNotApproved);
        }

        request.status = RecoveryStatus::Completed;
        request.completed_at = Some(chrono::Utc::now().to_rfc3339());
        Ok(())
    }

    pub fn cancel_recovery(&mut self, request_id: &str) -> Result<(), SocialRecoveryError> {
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| SocialRecoveryError::RequestNotFound(request_id.to_string()))?;

        if request.status != RecoveryStatus::InProgress {
            return Err(SocialRecoveryError::RequestNotInProgress);
        }

        request.status = RecoveryStatus::Rejected;
        Ok(())
    }

    pub fn check_timelock(&self, request_id: &str) -> Result<bool, SocialRecoveryError> {
        let request = self
            .requests
            .get(request_id)
            .ok_or_else(|| SocialRecoveryError::RequestNotFound(request_id.to_string()))?;

        let timelock_dt = chrono::DateTime::parse_from_rfc3339(&request.timelock_until)
            .unwrap_or_else(|_| chrono::Utc::now().into());
        Ok(chrono::Utc::now() >= timelock_dt)
    }

    pub fn expire_stale_requests(&mut self) -> Vec<String> {
        let now = chrono::Utc::now();
        let expire_duration = chrono::Duration::hours(self.config.auto_expire_hours as i64);
        let mut expired = Vec::new();

        for (id, request) in self.requests.iter_mut() {
            if request.status != RecoveryStatus::InProgress {
                continue;
            }
            let created = chrono::DateTime::parse_from_rfc3339(&request.created_at)
                .unwrap_or_else(|_| now.into())
                .with_timezone(&chrono::Utc);
            if now - created >= expire_duration {
                request.status = RecoveryStatus::TimedOut;
                expired.push(id.clone());
            }
        }

        expired
    }

    pub fn pending_requests(&self) -> Vec<&RecoveryRequest> {
        self.requests
            .values()
            .filter(|r| r.status == RecoveryStatus::InProgress)
            .collect()
    }

    pub fn update_config(&mut self, config: RecoveryConfig) {
        self.config = config;
    }

    pub fn stats(&self) -> SocialRecoveryStats {
        let total_guardians = self.guardians.len();
        let active_guardians = self.active_guardians().len();
        let total_requests = self.requests.len();
        let completed_recoveries = self
            .requests
            .values()
            .filter(|r| r.status == RecoveryStatus::Completed)
            .count();
        let rejected_recoveries = self
            .requests
            .values()
            .filter(|r| r.status == RecoveryStatus::Rejected)
            .count();
        let pending_requests = self
            .requests
            .values()
            .filter(|r| r.status == RecoveryStatus::InProgress)
            .count();

        SocialRecoveryStats {
            total_guardians,
            active_guardians,
            total_requests,
            completed_recoveries,
            rejected_recoveries,
            pending_requests,
        }
    }

    // ── Persistence ──────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), SocialRecoveryError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| SocialRecoveryError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| SocialRecoveryError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, SocialRecoveryError> {
        let data =
            std::fs::read_to_string(path).map_err(|e| SocialRecoveryError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| SocialRecoveryError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Helper ───────────────────────────────────────────────────────────

#[cfg(test)]
fn make_guardian(id: &str) -> Guardian {
    Guardian {
        id: id.to_string(),
        name: format!("Guardian {}", id),
        address: format!("evap1_{}", id),
        public_key: format!("pk_{}", id),
        status: GuardianStatus::Active,
        added_at: chrono::Utc::now().to_rfc3339(),
        last_confirmed: None,
        trust_score: 80,
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_guardian(id: &str) -> Guardian {
        make_guardian(id)
    }

    fn seeded_manager() -> SocialRecoveryManager {
        let mut mgr = SocialRecoveryManager::new();
        mgr.add_guardian(test_guardian("alice")).unwrap();
        mgr.add_guardian(test_guardian("bob")).unwrap();
        mgr.add_guardian(test_guardian("carol")).unwrap();
        mgr
    }

    // ── Guardian tests ───────────────────────────────────────────

    #[test]
    fn test_add_guardian() {
        let mut mgr = SocialRecoveryManager::new();
        let g = test_guardian("alice");
        assert!(mgr.add_guardian(g).is_ok());
        assert_eq!(mgr.guardians.len(), 1);
    }

    #[test]
    fn test_add_duplicate_guardian() {
        let mut mgr = SocialRecoveryManager::new();
        mgr.add_guardian(test_guardian("alice")).unwrap();
        let result = mgr.add_guardian(test_guardian("alice"));
        assert!(result.is_err());
    }

    #[test]
    fn test_add_guardian_max_reached() {
        let mut mgr = SocialRecoveryManager::new();
        for i in 0..7 {
            mgr.add_guardian(test_guardian(&format!("g{}", i))).unwrap();
        }
        let result = mgr.add_guardian(test_guardian("overflow"));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_guardian() {
        let mut mgr = SocialRecoveryManager::new();
        mgr.add_guardian(test_guardian("alice")).unwrap();
        let removed = mgr.remove_guardian("alice").unwrap();
        assert_eq!(removed.id, "alice");
        assert!(mgr.guardians.is_empty());
    }

    #[test]
    fn test_remove_nonexistent_guardian() {
        let mut mgr = SocialRecoveryManager::new();
        assert!(mgr.remove_guardian("nobody").is_err());
    }

    #[test]
    fn test_get_guardian() {
        let mut mgr = SocialRecoveryManager::new();
        mgr.add_guardian(test_guardian("alice")).unwrap();
        let g = mgr.get_guardian("alice");
        assert!(g.is_some());
        assert_eq!(g.unwrap().name, "Guardian alice");
    }

    #[test]
    fn test_active_guardians() {
        let mut mgr = seeded_manager();
        mgr.revoke_guardian("carol").unwrap();
        let active = mgr.active_guardians();
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn test_revoke_guardian() {
        let mut mgr = SocialRecoveryManager::new();
        mgr.add_guardian(test_guardian("alice")).unwrap();
        mgr.revoke_guardian("alice").unwrap();
        let g = mgr.get_guardian("alice").unwrap();
        assert_eq!(g.status, GuardianStatus::Revoked);
    }

    #[test]
    fn test_revoke_nonexistent_guardian() {
        let mut mgr = SocialRecoveryManager::new();
        assert!(mgr.revoke_guardian("nobody").is_err());
    }

    // ── Recovery workflow tests ──────────────────────────────────

    #[test]
    fn test_initiate_recovery() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk_123").unwrap();
        assert!(id.starts_with("recovery_"));
        assert_eq!(mgr.requests.len(), 1);
        let req = mgr.requests.get(&id).unwrap();
        assert_eq!(req.status, RecoveryStatus::InProgress);
        assert_eq!(req.requester, "user1");
    }

    #[test]
    fn test_initiate_recovery_not_enough_guardians() {
        let mut mgr = SocialRecoveryManager::new();
        mgr.add_guardian(test_guardian("alice")).unwrap();
        // threshold is 2, only 1 active guardian
        let result = mgr.initiate_recovery("user1", "new_pk");
        assert!(result.is_err());
    }

    #[test]
    fn test_approve_recovery() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        mgr.approve_recovery(&id, "alice", "sig_alice").unwrap();
        let req = mgr.requests.get(&id).unwrap();
        assert_eq!(req.approvals.len(), 1);
        assert_eq!(req.approvals[0].guardian_id, "alice");
    }

    #[test]
    fn test_approve_recovery_duplicate() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        mgr.approve_recovery(&id, "alice", "sig").unwrap();
        let result = mgr.approve_recovery(&id, "alice", "sig2");
        assert!(result.is_err());
    }

    #[test]
    fn test_approve_recovery_nonexistent_guardian() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        let result = mgr.approve_recovery(&id, "nobody", "sig");
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_recovery() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        mgr.reject_recovery(&id, "alice").unwrap();
        let req = mgr.requests.get(&id).unwrap();
        assert_eq!(req.rejections.len(), 1);
    }

    #[test]
    fn test_reject_recovery_duplicate() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        mgr.reject_recovery(&id, "alice").unwrap();
        let result = mgr.reject_recovery(&id, "alice");
        assert!(result.is_err());
    }

    #[test]
    fn test_complete_recovery_not_approved() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        let result = mgr.complete_recovery(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_cancel_recovery() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        mgr.cancel_recovery(&id).unwrap();
        let req = mgr.requests.get(&id).unwrap();
        assert_eq!(req.status, RecoveryStatus::Rejected);
    }

    #[test]
    fn test_cancel_non_in_progress_recovery() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        mgr.cancel_recovery(&id).unwrap();
        // Try to cancel again — now it's Rejected, not InProgress
        let result = mgr.cancel_recovery(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_timelock() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "new_pk").unwrap();
        // Timelock is 48h in the future, should not have passed
        let passed = mgr.check_timelock(&id).unwrap();
        assert!(!passed);
    }

    #[test]
    fn test_pending_requests() {
        let mut mgr = seeded_manager();
        mgr.initiate_recovery("user1", "pk1").unwrap();
        mgr.initiate_recovery("user2", "pk2").unwrap();
        assert_eq!(mgr.pending_requests().len(), 2);
    }

    // ── Config and stats ─────────────────────────────────────────

    #[test]
    fn test_update_config() {
        let mut mgr = SocialRecoveryManager::new();
        let new_config = RecoveryConfig {
            threshold: 3,
            timelock_hours: 72,
            max_guardians: 10,
            auto_expire_hours: 336,
        };
        mgr.update_config(new_config);
        assert_eq!(mgr.config.threshold, 3);
        assert_eq!(mgr.config.max_guardians, 10);
    }

    #[test]
    fn test_stats() {
        let mut mgr = seeded_manager();
        mgr.initiate_recovery("user1", "pk1").unwrap();
        let id2 = mgr.initiate_recovery("user2", "pk2").unwrap();
        mgr.cancel_recovery(&id2).unwrap();

        let stats = mgr.stats();
        assert_eq!(stats.total_guardians, 3);
        assert_eq!(stats.active_guardians, 3);
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.pending_requests, 1);
        assert_eq!(stats.rejected_recoveries, 1);
        assert_eq!(stats.completed_recoveries, 0);
    }

    // ── Persistence tests ────────────────────────────────────────

    #[test]
    fn test_save_and_load() {
        let path =
            std::env::temp_dir().join(format!("evap_social_recovery_{}.json", std::process::id()));
        let mut mgr = seeded_manager();
        mgr.initiate_recovery("user1", "pk1").unwrap();
        mgr.save(&path).unwrap();

        let loaded = SocialRecoveryManager::load(&path).unwrap();
        assert_eq!(loaded.guardians.len(), 3);
        assert_eq!(loaded.requests.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = std::env::temp_dir().join(format!(
            "evap_social_recovery_noexist_{}.json",
            std::process::id()
        ));
        let mgr = SocialRecoveryManager::load_or_default(&path);
        assert!(mgr.guardians.is_empty());
        assert!(mgr.requests.is_empty());
        assert_eq!(mgr.config.threshold, 2);
    }

    #[test]
    fn test_default_config() {
        let config = RecoveryConfig::default();
        assert_eq!(config.threshold, 2);
        assert_eq!(config.timelock_hours, 48);
        assert_eq!(config.max_guardians, 7);
        assert_eq!(config.auto_expire_hours, 168);
    }

    // ─── Additional coverage tests ────────────────────────────────────────────

    #[test]
    fn test_approve_revoked_guardian_covers_lines_255_257() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "pk").unwrap();
        mgr.revoke_guardian("alice").unwrap();
        // Alice is now Revoked, not Active → GuardianNotFound
        let err = mgr.approve_recovery(&id, "alice", "sig").unwrap_err();
        assert!(matches!(err, SocialRecoveryError::GuardianNotFound(_)));
    }

    #[test]
    fn test_approve_non_in_progress_covers_line_266() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "pk").unwrap();
        mgr.cancel_recovery(&id).unwrap(); // status → Rejected
        let err = mgr.approve_recovery(&id, "bob", "sig").unwrap_err();
        assert!(matches!(err, SocialRecoveryError::RequestNotInProgress));
    }

    #[test]
    fn test_approve_threshold_met_past_timelock_covers_lines_289_293() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "pk").unwrap();
        // Push timelock into the past so auto-approve fires
        mgr.requests.get_mut(&id).unwrap().timelock_until = "2020-01-01T00:00:00Z".to_string();
        // threshold=2 by default; two approvals reach it
        mgr.approve_recovery(&id, "alice", "sig1").unwrap();
        mgr.approve_recovery(&id, "bob", "sig2").unwrap();
        let req = mgr.requests.get(&id).unwrap();
        assert_eq!(req.status, RecoveryStatus::Approved);
    }

    #[test]
    fn test_reject_non_in_progress_covers_line_315() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "pk").unwrap();
        mgr.cancel_recovery(&id).unwrap(); // status → Rejected
        let err = mgr.reject_recovery(&id, "bob").unwrap_err();
        assert!(matches!(err, SocialRecoveryError::RequestNotInProgress));
    }

    #[test]
    fn test_complete_recovery_success_covers_lines_336_340() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "pk").unwrap();
        // Force approved status directly so we bypass timelock
        mgr.requests.get_mut(&id).unwrap().status = RecoveryStatus::Approved;
        mgr.complete_recovery(&id).unwrap();
        let req = mgr.requests.get(&id).unwrap();
        assert_eq!(req.status, RecoveryStatus::Completed);
        assert!(req.completed_at.is_some());
    }

    #[test]
    fn test_expire_stale_requests_covers_lines_368_387() {
        let mut mgr = seeded_manager();
        let id = mgr.initiate_recovery("user1", "pk").unwrap();
        // Push created_at far into the past (>168h default)
        let old_ts = (chrono::Utc::now() - chrono::Duration::hours(200)).to_rfc3339();
        mgr.requests.get_mut(&id).unwrap().created_at = old_ts;
        let expired = mgr.expire_stale_requests();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], id);
        assert_eq!(mgr.requests.get(&id).unwrap().status, RecoveryStatus::TimedOut);
    }
}
