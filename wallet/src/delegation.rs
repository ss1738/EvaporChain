// wallet/src/delegation.rs — Token delegation with revocable allowances
//
// Delegate spending authority to another address with:
//   - Per-delegation spending cap
//   - Expiry timestamps
//   - Revocation at any time
//   - Usage tracking and audit trail

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DelegationError {
    #[error("delegation not found: {0}")]
    NotFound(String),
    #[error("delegation already exists: {0}")]
    AlreadyExists(String),
    #[error("delegation expired")]
    Expired,
    #[error("delegation revoked")]
    Revoked,
    #[error("spending cap exceeded: requested {0}, remaining {1}")]
    CapExceeded(u64, u64),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Delegation types ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DelegationStatus {
    Active,
    Expired,
    Revoked,
    Exhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DelegationType {
    Transfer,
    Staking,
    Governance,
    ContractCall,
    Any,
}

impl DelegationType {
    pub fn name(&self) -> &'static str {
        match self {
            DelegationType::Transfer => "transfer",
            DelegationType::Staking => "staking",
            DelegationType::Governance => "governance",
            DelegationType::ContractCall => "contract_call",
            DelegationType::Any => "any",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "transfer" => Some(DelegationType::Transfer),
            "staking" => Some(DelegationType::Staking),
            "governance" => Some(DelegationType::Governance),
            "contract_call" | "contract" => Some(DelegationType::ContractCall),
            "any" | "all" => Some(DelegationType::Any),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendRecord {
    pub amount: u64,
    pub timestamp: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delegation {
    pub id: String,
    pub owner: String,
    pub delegate: String,
    pub delegation_type: DelegationType,
    pub spending_cap: u64,
    pub spent: u64,
    pub per_tx_limit: Option<u64>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub status: DelegationStatus,
    pub note: String,
    pub spend_history: Vec<SpendRecord>,
}

impl Delegation {
    pub fn new(
        id: &str,
        owner: &str,
        delegate: &str,
        delegation_type: DelegationType,
        spending_cap: u64,
    ) -> Self {
        Self {
            id: id.to_string(),
            owner: owner.to_string(),
            delegate: delegate.to_string(),
            delegation_type,
            spending_cap,
            spent: 0,
            per_tx_limit: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
            status: DelegationStatus::Active,
            note: String::new(),
            spend_history: Vec::new(),
        }
    }

    pub fn with_expiry(mut self, expires_at: &str) -> Self {
        self.expires_at = Some(expires_at.to_string());
        self
    }

    pub fn with_per_tx_limit(mut self, limit: u64) -> Self {
        self.per_tx_limit = Some(limit);
        self
    }

    pub fn with_note(mut self, note: &str) -> Self {
        self.note = note.to_string();
        self
    }

    pub fn remaining(&self) -> u64 {
        self.spending_cap.saturating_sub(self.spent)
    }

    pub fn utilization_percent(&self) -> f64 {
        if self.spending_cap == 0 {
            return 100.0;
        }
        (self.spent as f64 / self.spending_cap as f64) * 100.0
    }

    pub fn is_expired(&self, now: &str) -> bool {
        match &self.expires_at {
            Some(exp) => now >= exp.as_str(),
            None => false,
        }
    }

    pub fn is_active(&self, now: &str) -> bool {
        self.status == DelegationStatus::Active && !self.is_expired(now)
    }

    /// Try to spend against this delegation
    pub fn spend(&mut self, amount: u64, description: &str) -> Result<(), DelegationError> {
        let now = chrono::Utc::now().to_rfc3339();

        if self.status == DelegationStatus::Revoked {
            return Err(DelegationError::Revoked);
        }
        if self.is_expired(&now) {
            self.status = DelegationStatus::Expired;
            return Err(DelegationError::Expired);
        }
        if let Some(limit) = self.per_tx_limit {
            if amount > limit {
                return Err(DelegationError::CapExceeded(amount, limit));
            }
        }
        if amount > self.remaining() {
            return Err(DelegationError::CapExceeded(amount, self.remaining()));
        }

        self.spent += amount;
        self.spend_history.push(SpendRecord {
            amount,
            timestamp: now,
            description: description.to_string(),
        });

        if self.spent >= self.spending_cap {
            self.status = DelegationStatus::Exhausted;
        }

        Ok(())
    }

    /// Revoke the delegation
    pub fn revoke(&mut self) -> Result<(), DelegationError> {
        if self.status == DelegationStatus::Revoked {
            return Err(DelegationError::Revoked);
        }
        self.status = DelegationStatus::Revoked;
        Ok(())
    }

    /// Increase the spending cap
    pub fn increase_cap(&mut self, additional: u64) {
        self.spending_cap += additional;
        if self.status == DelegationStatus::Exhausted && self.remaining() > 0 {
            self.status = DelegationStatus::Active;
        }
    }
}

// ── Delegation store ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DelegationStore {
    pub delegations: HashMap<String, Delegation>,
}

impl DelegationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, delegation: Delegation) -> Result<(), DelegationError> {
        if self.delegations.contains_key(&delegation.id) {
            return Err(DelegationError::AlreadyExists(delegation.id.clone()));
        }
        self.delegations.insert(delegation.id.clone(), delegation);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Delegation> {
        self.delegations.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Delegation> {
        self.delegations.get_mut(id)
    }

    pub fn remove(&mut self, id: &str) -> Result<Delegation, DelegationError> {
        self.delegations
            .remove(id)
            .ok_or_else(|| DelegationError::NotFound(id.into()))
    }

    pub fn list(&self) -> Vec<&Delegation> {
        self.delegations.values().collect()
    }

    /// Active delegations (not expired, not revoked)
    pub fn active(&self) -> Vec<&Delegation> {
        let now = chrono::Utc::now().to_rfc3339();
        self.delegations
            .values()
            .filter(|d| d.is_active(&now))
            .collect()
    }

    /// Delegations granted by an owner
    pub fn by_owner(&self, owner: &str) -> Vec<&Delegation> {
        self.delegations
            .values()
            .filter(|d| d.owner == owner)
            .collect()
    }

    /// Delegations received by a delegate
    pub fn by_delegate(&self, delegate: &str) -> Vec<&Delegation> {
        self.delegations
            .values()
            .filter(|d| d.delegate == delegate)
            .collect()
    }

    /// Total delegated (remaining) across all active delegations for an owner
    pub fn total_delegated(&self, owner: &str) -> u64 {
        let now = chrono::Utc::now().to_rfc3339();
        self.delegations
            .values()
            .filter(|d| d.owner == owner && d.is_active(&now))
            .map(|d| d.remaining())
            .sum()
    }

    /// Revoke all delegations for an owner (emergency)
    pub fn revoke_all(&mut self, owner: &str) -> usize {
        let mut count = 0;
        for d in self.delegations.values_mut() {
            if d.owner == owner && d.status == DelegationStatus::Active {
                d.status = DelegationStatus::Revoked;
                count += 1;
            }
        }
        count
    }

    /// Purge expired/revoked delegations
    pub fn purge_inactive(&mut self) -> usize {
        let now = chrono::Utc::now().to_rfc3339();
        let before = self.delegations.len();
        self.delegations.retain(|_, d| d.is_active(&now));
        before - self.delegations.len()
    }

    pub fn save(&self, path: &Path) -> Result<(), DelegationError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| DelegationError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| DelegationError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, DelegationError> {
        let data = std::fs::read_to_string(path).map_err(|e| DelegationError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| DelegationError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_delegation(id: &str) -> Delegation {
        Delegation::new(id, "alice", "bob", DelegationType::Transfer, 1000)
    }

    #[test]
    fn test_delegation_new() {
        let d = make_delegation("d1");
        assert_eq!(d.spending_cap, 1000);
        assert_eq!(d.spent, 0);
        assert_eq!(d.remaining(), 1000);
        assert_eq!(d.status, DelegationStatus::Active);
    }

    #[test]
    fn test_delegation_with_expiry() {
        let d = make_delegation("d1").with_expiry("2099-01-01T00:00:00Z");
        assert!(d.expires_at.is_some());
        assert!(!d.is_expired(&chrono::Utc::now().to_rfc3339()));
    }

    #[test]
    fn test_delegation_with_per_tx_limit() {
        let d = make_delegation("d1").with_per_tx_limit(100);
        assert_eq!(d.per_tx_limit, Some(100));
    }

    #[test]
    fn test_delegation_spend() {
        let mut d = make_delegation("d1");
        d.spend(300, "payment 1").unwrap();
        assert_eq!(d.spent, 300);
        assert_eq!(d.remaining(), 700);
        assert_eq!(d.spend_history.len(), 1);
    }

    #[test]
    fn test_delegation_spend_multiple() {
        let mut d = make_delegation("d1");
        d.spend(300, "p1").unwrap();
        d.spend(400, "p2").unwrap();
        assert_eq!(d.spent, 700);
        assert_eq!(d.spend_history.len(), 2);
    }

    #[test]
    fn test_delegation_spend_exceeds_cap() {
        let mut d = make_delegation("d1");
        assert!(d.spend(1001, "too much").is_err());
    }

    #[test]
    fn test_delegation_spend_exceeds_per_tx() {
        let mut d = make_delegation("d1").with_per_tx_limit(100);
        assert!(d.spend(101, "too much per tx").is_err());
        d.spend(100, "ok").unwrap();
    }

    #[test]
    fn test_delegation_exhausted() {
        let mut d = make_delegation("d1");
        d.spend(1000, "all").unwrap();
        assert_eq!(d.status, DelegationStatus::Exhausted);
        assert!(d.spend(1, "more").is_err());
    }

    #[test]
    fn test_delegation_revoke() {
        let mut d = make_delegation("d1");
        d.revoke().unwrap();
        assert_eq!(d.status, DelegationStatus::Revoked);
        assert!(d.spend(1, "nope").is_err());
    }

    #[test]
    fn test_delegation_double_revoke() {
        let mut d = make_delegation("d1");
        d.revoke().unwrap();
        assert!(d.revoke().is_err());
    }

    #[test]
    fn test_delegation_expired() {
        let mut d = make_delegation("d1").with_expiry("2020-01-01T00:00:00Z");
        assert!(d.spend(1, "past expiry").is_err());
        assert_eq!(d.status, DelegationStatus::Expired);
    }

    #[test]
    fn test_delegation_increase_cap() {
        let mut d = make_delegation("d1");
        d.spend(1000, "all").unwrap();
        assert_eq!(d.status, DelegationStatus::Exhausted);
        d.increase_cap(500);
        assert_eq!(d.spending_cap, 1500);
        assert_eq!(d.status, DelegationStatus::Active);
        d.spend(500, "more").unwrap();
    }

    #[test]
    fn test_delegation_utilization() {
        let mut d = make_delegation("d1");
        assert_eq!(d.utilization_percent(), 0.0);
        d.spend(500, "half").unwrap();
        assert_eq!(d.utilization_percent(), 50.0);
    }

    #[test]
    fn test_delegation_is_active() {
        let d = make_delegation("d1");
        assert!(d.is_active(&chrono::Utc::now().to_rfc3339()));
    }

    #[test]
    fn test_delegation_type_from_str() {
        assert_eq!(
            DelegationType::from_str("transfer"),
            Some(DelegationType::Transfer)
        );
        assert_eq!(
            DelegationType::from_str("staking"),
            Some(DelegationType::Staking)
        );
        assert_eq!(DelegationType::from_str("any"), Some(DelegationType::Any));
        assert_eq!(DelegationType::from_str("nope"), None);
    }

    #[test]
    fn test_store_add_get() {
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        assert!(store.get("d1").is_some());
    }

    #[test]
    fn test_store_add_duplicate() {
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        assert!(store.add(make_delegation("d1")).is_err());
    }

    #[test]
    fn test_store_remove() {
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        let d = store.remove("d1").unwrap();
        assert_eq!(d.id, "d1");
        assert!(store.get("d1").is_none());
    }

    #[test]
    fn test_store_by_owner() {
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        store
            .add(Delegation::new(
                "d2",
                "carol",
                "bob",
                DelegationType::Any,
                500,
            ))
            .unwrap();
        assert_eq!(store.by_owner("alice").len(), 1);
    }

    #[test]
    fn test_store_by_delegate() {
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        assert_eq!(store.by_delegate("bob").len(), 1);
        assert_eq!(store.by_delegate("carol").len(), 0);
    }

    #[test]
    fn test_store_total_delegated() {
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        store
            .add(Delegation::new(
                "d2",
                "alice",
                "carol",
                DelegationType::Transfer,
                500,
            ))
            .unwrap();
        assert_eq!(store.total_delegated("alice"), 1500);
    }

    #[test]
    fn test_store_revoke_all() {
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        store
            .add(Delegation::new(
                "d2",
                "alice",
                "carol",
                DelegationType::Any,
                500,
            ))
            .unwrap();
        let count = store.revoke_all("alice");
        assert_eq!(count, 2);
        assert_eq!(store.active().len(), 0);
    }

    #[test]
    fn test_store_active() {
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        let mut d2 = make_delegation("d2");
        d2.status = DelegationStatus::Revoked;
        store.delegations.insert("d2".into(), d2);
        assert_eq!(store.active().len(), 1);
    }

    #[test]
    fn test_store_save_load() {
        let path = std::env::temp_dir().join(format!("evap_deleg_{}.json", std::process::id()));
        let mut store = DelegationStore::new();
        store.add(make_delegation("d1")).unwrap();
        store.save(&path).unwrap();
        let loaded = DelegationStore::load(&path).unwrap();
        assert_eq!(loaded.list().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_store_load_or_default() {
        let store = DelegationStore::load_or_default(Path::new("/tmp/noexist_deleg.json"));
        assert!(store.delegations.is_empty());
    }
}
