use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum PermissionManagerError {
    #[error("permission not found: {0}")]
    PermissionNotFound(String),
    #[error("duplicate permission: {0}")]
    DuplicatePermission(String),
    #[error("approval not found: {0}")]
    ApprovalNotFound(String),
    #[error("spend limit exceeded: {0}")]
    SpendLimitExceeded(String),
    #[error("permission expired: {0}")]
    PermissionExpired(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PermissionType {
    ReadBalance,
    SignTransaction,
    SendTokens,
    DeployContract,
    ManageNFT,
    AccessPrivateKey,
    ConnectDapp,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PermissionStatus3 {
    Granted,
    Denied,
    Pending,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalStatus {
    Approved,
    Rejected,
    Pending,
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub id: String,
    pub dapp_id: String,
    pub permission_type: PermissionType,
    pub status: PermissionStatus3,
    pub granted_at: Option<String>,
    pub expires_at: Option<String>,
    pub max_uses: Option<u64>,
    pub use_count: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendLimit {
    pub dapp_id: String,
    pub contract_address: Option<String>,
    pub token: String,
    pub max_per_tx: u64,
    pub max_daily: u64,
    pub spent_today: u64,
    pub last_reset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub dapp_id: String,
    pub permission_type: PermissionType,
    pub reason: String,
    pub status: ApprovalStatus,
    pub requested_at: String,
    pub resolved_at: Option<String>,
    pub resolver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionStats3 {
    pub total_permissions: usize,
    pub granted: usize,
    pub denied: usize,
    pub pending: usize,
    pub expired: usize,
    pub revoked: usize,
    pub total_spend_limits: usize,
    pub total_approvals: usize,
    pub pending_approvals: usize,
}

// ---------------------------------------------------------------------------
// PermissionManager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionManager {
    pub permissions: HashMap<String, Permission>,
    pub spend_limits: HashMap<String, SpendLimit>,
    pub approvals: Vec<ApprovalRequest>,
}

impl PermissionManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -- permission CRUD ----------------------------------------------------

    pub fn grant_permission(&mut self, mut perm: Permission) -> Result<(), PermissionManagerError> {
        if self.permissions.contains_key(&perm.id) {
            return Err(PermissionManagerError::DuplicatePermission(perm.id.clone()));
        }
        perm.status = PermissionStatus3::Granted;
        perm.granted_at = Some(Utc::now().to_rfc3339());
        self.permissions.insert(perm.id.clone(), perm);
        Ok(())
    }

    pub fn revoke_permission(&mut self, id: &str) -> Result<(), PermissionManagerError> {
        let perm = self
            .permissions
            .get_mut(id)
            .ok_or_else(|| PermissionManagerError::PermissionNotFound(id.to_string()))?;
        perm.status = PermissionStatus3::Revoked;
        Ok(())
    }

    pub fn deny_permission(&mut self, id: &str) -> Result<(), PermissionManagerError> {
        let perm = self
            .permissions
            .get_mut(id)
            .ok_or_else(|| PermissionManagerError::PermissionNotFound(id.to_string()))?;
        perm.status = PermissionStatus3::Denied;
        Ok(())
    }

    pub fn check_permission(&self, dapp_id: &str, perm_type: &PermissionType) -> bool {
        self.permissions.values().any(|p| {
            p.dapp_id == dapp_id
                && p.permission_type == *perm_type
                && p.status == PermissionStatus3::Granted
                && !Self::is_expired(p)
                && !Self::is_max_uses_exceeded(p)
        })
    }

    pub fn use_permission(&mut self, id: &str) -> Result<(), PermissionManagerError> {
        let perm = self
            .permissions
            .get_mut(id)
            .ok_or_else(|| PermissionManagerError::PermissionNotFound(id.to_string()))?;

        if Self::is_expired(perm) {
            perm.status = PermissionStatus3::Expired;
            return Err(PermissionManagerError::PermissionExpired(id.to_string()));
        }

        if let Some(max) = perm.max_uses {
            if perm.use_count >= max {
                return Err(PermissionManagerError::SpendLimitExceeded(format!(
                    "max uses ({max}) reached for permission {id}"
                )));
            }
        }

        perm.use_count += 1;
        Ok(())
    }

    // -- spend limits -------------------------------------------------------

    pub fn set_spend_limit(&mut self, limit: SpendLimit) {
        self.spend_limits.insert(limit.dapp_id.clone(), limit);
    }

    pub fn check_spend(&self, dapp_id: &str, amount: u64) -> Result<(), PermissionManagerError> {
        let limit = self.spend_limits.get(dapp_id).ok_or_else(|| {
            PermissionManagerError::PermissionNotFound(format!("spend limit for {dapp_id}"))
        })?;

        if amount > limit.max_per_tx {
            return Err(PermissionManagerError::SpendLimitExceeded(format!(
                "amount {amount} exceeds max per-tx {}",
                limit.max_per_tx
            )));
        }

        if limit.spent_today + amount > limit.max_daily {
            return Err(PermissionManagerError::SpendLimitExceeded(format!(
                "amount {amount} would exceed daily limit {}",
                limit.max_daily
            )));
        }

        Ok(())
    }

    pub fn record_spend(
        &mut self,
        dapp_id: &str,
        amount: u64,
    ) -> Result<(), PermissionManagerError> {
        self.check_spend(dapp_id, amount)?;
        let limit = self.spend_limits.get_mut(dapp_id).ok_or_else(|| {
            PermissionManagerError::PermissionNotFound(format!("spend limit for {dapp_id}"))
        })?;
        limit.spent_today += amount;
        Ok(())
    }

    pub fn reset_daily_spend(&mut self, dapp_id: &str) -> Result<(), PermissionManagerError> {
        let limit = self.spend_limits.get_mut(dapp_id).ok_or_else(|| {
            PermissionManagerError::PermissionNotFound(format!("spend limit for {dapp_id}"))
        })?;
        limit.spent_today = 0;
        limit.last_reset = Utc::now().to_rfc3339();
        Ok(())
    }

    // -- approval workflow --------------------------------------------------

    pub fn request_approval(&mut self, req: ApprovalRequest) {
        self.approvals.push(req);
    }

    pub fn approve_request(
        &mut self,
        id: &str,
        resolver: &str,
    ) -> Result<(), PermissionManagerError> {
        let req = self
            .approvals
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| PermissionManagerError::ApprovalNotFound(id.to_string()))?;
        req.status = ApprovalStatus::Approved;
        req.resolved_at = Some(Utc::now().to_rfc3339());
        req.resolver = Some(resolver.to_string());
        Ok(())
    }

    pub fn reject_request(
        &mut self,
        id: &str,
        resolver: &str,
    ) -> Result<(), PermissionManagerError> {
        let req = self
            .approvals
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| PermissionManagerError::ApprovalNotFound(id.to_string()))?;
        req.status = ApprovalStatus::Rejected;
        req.resolved_at = Some(Utc::now().to_rfc3339());
        req.resolver = Some(resolver.to_string());
        Ok(())
    }

    // -- queries ------------------------------------------------------------

    pub fn permissions_for_dapp(&self, dapp_id: &str) -> Vec<&Permission> {
        self.permissions
            .values()
            .filter(|p| p.dapp_id == dapp_id)
            .collect()
    }

    pub fn pending_approvals(&self) -> Vec<&ApprovalRequest> {
        self.approvals
            .iter()
            .filter(|r| r.status == ApprovalStatus::Pending)
            .collect()
    }

    pub fn granted_permissions(&self) -> Vec<&Permission> {
        self.permissions
            .values()
            .filter(|p| p.status == PermissionStatus3::Granted)
            .collect()
    }

    #[allow(clippy::field_reassign_with_default)]
    pub fn stats(&self) -> PermissionStats3 {
        let mut s = PermissionStats3::default();
        s.total_permissions = self.permissions.len();
        for p in self.permissions.values() {
            match p.status {
                PermissionStatus3::Granted => s.granted += 1,
                PermissionStatus3::Denied => s.denied += 1,
                PermissionStatus3::Pending => s.pending += 1,
                PermissionStatus3::Expired => s.expired += 1,
                PermissionStatus3::Revoked => s.revoked += 1,
            }
        }
        s.total_spend_limits = self.spend_limits.len();
        s.total_approvals = self.approvals.len();
        s.pending_approvals = self
            .approvals
            .iter()
            .filter(|r| r.status == ApprovalStatus::Pending)
            .count();
        s
    }

    // -- persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), PermissionManagerError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, PermissionManagerError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    // -- helpers (private) --------------------------------------------------

    fn is_expired(perm: &Permission) -> bool {
        if let Some(ref exp) = perm.expires_at {
            if let Ok(dt) = exp.parse::<chrono::DateTime<Utc>>() {
                return Utc::now() > dt;
            }
        }
        false
    }

    fn is_max_uses_exceeded(perm: &Permission) -> bool {
        if let Some(max) = perm.max_uses {
            return perm.use_count >= max;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process;

    fn test_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!(
            "evaporchain_perm_test_{}_{}.json",
            process::id(),
            name
        ))
    }

    fn make_permission(id: &str, dapp: &str, ptype: PermissionType) -> Permission {
        Permission {
            id: id.to_string(),
            dapp_id: dapp.to_string(),
            permission_type: ptype,
            status: PermissionStatus3::Pending,
            granted_at: None,
            expires_at: None,
            max_uses: None,
            use_count: 0,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn make_spend_limit(dapp: &str, per_tx: u64, daily: u64) -> SpendLimit {
        SpendLimit {
            dapp_id: dapp.to_string(),
            contract_address: None,
            token: "EVAP".to_string(),
            max_per_tx: per_tx,
            max_daily: daily,
            spent_today: 0,
            last_reset: Utc::now().to_rfc3339(),
        }
    }

    fn make_approval(id: &str, dapp: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: id.to_string(),
            dapp_id: dapp.to_string(),
            permission_type: PermissionType::SendTokens,
            reason: "need to send".to_string(),
            status: ApprovalStatus::Pending,
            requested_at: Utc::now().to_rfc3339(),
            resolved_at: None,
            resolver: None,
        }
    }

    // 1
    #[test]
    fn test_grant_permission() {
        let mut mgr = PermissionManager::new();
        let perm = make_permission("p1", "dapp1", PermissionType::ReadBalance);
        mgr.grant_permission(perm).unwrap();
        let stored = mgr.permissions.get("p1").unwrap();
        assert_eq!(stored.status, PermissionStatus3::Granted);
        assert!(stored.granted_at.is_some());
    }

    // 2
    #[test]
    fn test_revoke_permission() {
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::ReadBalance))
            .unwrap();
        mgr.revoke_permission("p1").unwrap();
        assert_eq!(
            mgr.permissions.get("p1").unwrap().status,
            PermissionStatus3::Revoked
        );
    }

    // 3
    #[test]
    fn test_deny_permission() {
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::ReadBalance))
            .unwrap();
        mgr.deny_permission("p1").unwrap();
        assert_eq!(
            mgr.permissions.get("p1").unwrap().status,
            PermissionStatus3::Denied
        );
    }

    // 4
    #[test]
    fn test_duplicate_permission() {
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::ReadBalance))
            .unwrap();
        let res = mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::ReadBalance));
        assert!(res.is_err());
    }

    // 5
    #[test]
    fn test_check_permission_granted() {
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission(
            "p1",
            "dapp1",
            PermissionType::SignTransaction,
        ))
        .unwrap();
        assert!(mgr.check_permission("dapp1", &PermissionType::SignTransaction));
    }

    // 6
    #[test]
    fn test_check_permission_denied() {
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission(
            "p1",
            "dapp1",
            PermissionType::SignTransaction,
        ))
        .unwrap();
        mgr.deny_permission("p1").unwrap();
        assert!(!mgr.check_permission("dapp1", &PermissionType::SignTransaction));
    }

    // 7
    #[test]
    fn test_check_permission_not_found() {
        let mgr = PermissionManager::new();
        assert!(!mgr.check_permission("dapp1", &PermissionType::ReadBalance));
    }

    // 8
    #[test]
    fn test_use_permission_with_max_uses() {
        let mut mgr = PermissionManager::new();
        let mut perm = make_permission("p1", "dapp1", PermissionType::SendTokens);
        perm.max_uses = Some(2);
        mgr.grant_permission(perm).unwrap();

        mgr.use_permission("p1").unwrap();
        mgr.use_permission("p1").unwrap();
        let res = mgr.use_permission("p1");
        assert!(res.is_err());
    }

    // 9
    #[test]
    fn test_use_permission_unlimited() {
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::SendTokens))
            .unwrap();
        for _ in 0..100 {
            mgr.use_permission("p1").unwrap();
        }
        assert_eq!(mgr.permissions.get("p1").unwrap().use_count, 100);
    }

    // 10
    #[test]
    fn test_set_spend_limit() {
        let mut mgr = PermissionManager::new();
        mgr.set_spend_limit(make_spend_limit("dapp1", 100, 1000));
        assert!(mgr.spend_limits.contains_key("dapp1"));
    }

    // 11
    #[test]
    fn test_check_spend_within_limit() {
        let mut mgr = PermissionManager::new();
        mgr.set_spend_limit(make_spend_limit("dapp1", 100, 1000));
        assert!(mgr.check_spend("dapp1", 50).is_ok());
    }

    // 12
    #[test]
    fn test_check_spend_over_daily() {
        let mut mgr = PermissionManager::new();
        let mut limit = make_spend_limit("dapp1", 100, 1000);
        limit.spent_today = 950;
        mgr.set_spend_limit(limit);
        let res = mgr.check_spend("dapp1", 100);
        assert!(res.is_err());
    }

    // 13
    #[test]
    fn test_check_spend_per_tx_exceeded() {
        let mut mgr = PermissionManager::new();
        mgr.set_spend_limit(make_spend_limit("dapp1", 100, 1000));
        let res = mgr.check_spend("dapp1", 200);
        assert!(res.is_err());
    }

    // 14
    #[test]
    fn test_record_spend() {
        let mut mgr = PermissionManager::new();
        mgr.set_spend_limit(make_spend_limit("dapp1", 100, 1000));
        mgr.record_spend("dapp1", 50).unwrap();
        assert_eq!(mgr.spend_limits.get("dapp1").unwrap().spent_today, 50);
        mgr.record_spend("dapp1", 50).unwrap();
        assert_eq!(mgr.spend_limits.get("dapp1").unwrap().spent_today, 100);
    }

    // 15
    #[test]
    fn test_reset_daily_spend() {
        let mut mgr = PermissionManager::new();
        mgr.set_spend_limit(make_spend_limit("dapp1", 100, 1000));
        mgr.record_spend("dapp1", 80).unwrap();
        mgr.reset_daily_spend("dapp1").unwrap();
        assert_eq!(mgr.spend_limits.get("dapp1").unwrap().spent_today, 0);
    }

    // 16
    #[test]
    fn test_request_approval() {
        let mut mgr = PermissionManager::new();
        mgr.request_approval(make_approval("a1", "dapp1"));
        assert_eq!(mgr.approvals.len(), 1);
    }

    // 17
    #[test]
    fn test_approve_request() {
        let mut mgr = PermissionManager::new();
        mgr.request_approval(make_approval("a1", "dapp1"));
        mgr.approve_request("a1", "admin").unwrap();
        let req = mgr.approvals.iter().find(|r| r.id == "a1").unwrap();
        assert_eq!(req.status, ApprovalStatus::Approved);
        assert_eq!(req.resolver.as_deref(), Some("admin"));
    }

    // 18
    #[test]
    fn test_reject_request() {
        let mut mgr = PermissionManager::new();
        mgr.request_approval(make_approval("a1", "dapp1"));
        mgr.reject_request("a1", "admin").unwrap();
        let req = mgr.approvals.iter().find(|r| r.id == "a1").unwrap();
        assert_eq!(req.status, ApprovalStatus::Rejected);
    }

    // 19
    #[test]
    fn test_permissions_for_dapp() {
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::ReadBalance))
            .unwrap();
        mgr.grant_permission(make_permission("p2", "dapp1", PermissionType::SendTokens))
            .unwrap();
        mgr.grant_permission(make_permission("p3", "dapp2", PermissionType::ReadBalance))
            .unwrap();
        assert_eq!(mgr.permissions_for_dapp("dapp1").len(), 2);
        assert_eq!(mgr.permissions_for_dapp("dapp2").len(), 1);
    }

    // 20
    #[test]
    fn test_pending_approvals_and_granted_permissions() {
        let mut mgr = PermissionManager::new();
        mgr.request_approval(make_approval("a1", "dapp1"));
        mgr.request_approval(make_approval("a2", "dapp1"));
        mgr.approve_request("a1", "admin").unwrap();
        assert_eq!(mgr.pending_approvals().len(), 1);

        mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::ReadBalance))
            .unwrap();
        mgr.grant_permission(make_permission("p2", "dapp1", PermissionType::SendTokens))
            .unwrap();
        mgr.revoke_permission("p2").unwrap();
        assert_eq!(mgr.granted_permissions().len(), 1);
    }

    // 21
    #[test]
    fn test_stats() {
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::ReadBalance))
            .unwrap();
        mgr.grant_permission(make_permission("p2", "dapp1", PermissionType::SendTokens))
            .unwrap();
        mgr.deny_permission("p2").unwrap();
        mgr.set_spend_limit(make_spend_limit("dapp1", 100, 1000));
        mgr.request_approval(make_approval("a1", "dapp1"));

        let s = mgr.stats();
        assert_eq!(s.total_permissions, 2);
        assert_eq!(s.granted, 1);
        assert_eq!(s.denied, 1);
        assert_eq!(s.total_spend_limits, 1);
        assert_eq!(s.total_approvals, 1);
        assert_eq!(s.pending_approvals, 1);
    }

    // 22 (bonus: persistence roundtrip)
    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut mgr = PermissionManager::new();
        mgr.grant_permission(make_permission("p1", "dapp1", PermissionType::ReadBalance))
            .unwrap();
        mgr.set_spend_limit(make_spend_limit("dapp1", 100, 1000));
        mgr.request_approval(make_approval("a1", "dapp1"));

        mgr.save(&path).unwrap();
        let loaded = PermissionManager::load(&path).unwrap();

        assert_eq!(loaded.permissions.len(), 1);
        assert_eq!(loaded.spend_limits.len(), 1);
        assert_eq!(loaded.approvals.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    // 23 (bonus: load_or_default on missing file)
    #[test]
    fn test_load_or_default_missing() {
        let path = test_path("nonexistent");
        let mgr = PermissionManager::load_or_default(&path);
        assert_eq!(mgr.permissions.len(), 0);
    }
}
