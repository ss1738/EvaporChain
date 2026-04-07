// wallet/src/escrow.rs — On-chain escrow system for EvaporChain wallet
//
// - Create escrows between buyer and seller with optional arbiter
// - Fund, release, refund, and dispute flows
// - Milestone-based partial releases
// - Fee calculation and event audit trail
// - Expiry detection and stats

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EscrowError {
    #[error("escrow not found: {0}")]
    NotFound(String),
    #[error("escrow already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid status: expected {expected}, got {actual}")]
    InvalidStatus { expected: String, actual: String },
    #[error("milestone not found: {0}")]
    MilestoneNotFound(String),
    #[error("milestone not completed: {0}")]
    MilestoneNotCompleted(String),
    #[error("milestone already released: {0}")]
    MilestoneAlreadyReleased(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Enums ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EscrowStatus {
    Created,
    Funded,
    Released,
    Refunded,
    Disputed,
    Resolved,
    Expired,
}

impl std::fmt::Display for EscrowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "Created"),
            Self::Funded => write!(f, "Funded"),
            Self::Released => write!(f, "Released"),
            Self::Refunded => write!(f, "Refunded"),
            Self::Disputed => write!(f, "Disputed"),
            Self::Resolved => write!(f, "Resolved"),
            Self::Expired => write!(f, "Expired"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisputeResolution {
    ReleaseToBuyer,
    ReleaseToSeller,
    Split(u64, u64),
}

// ── Structs ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Escrow {
    pub id: String,
    pub buyer: String,
    pub seller: String,
    pub arbiter: Option<String>,
    pub token: String,
    pub amount: u64,
    pub fee_bps: u32,
    pub status: EscrowStatus,
    pub created_at: String,
    pub funded_at: Option<String>,
    pub expires_at: String,
    pub released_at: Option<String>,
    pub description: String,
    pub milestones: Vec<Milestone>,
    pub dispute_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub description: String,
    pub amount: u64,
    pub completed: bool,
    pub released: bool,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowEvent {
    pub escrow_id: String,
    pub event_type: String,
    pub timestamp: String,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowStats {
    pub total_escrows: usize,
    pub active_escrows: usize,
    pub released: usize,
    pub refunded: usize,
    pub disputed: usize,
    pub total_volume: u64,
    pub total_fees: u64,
}

// ── Manager ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EscrowManager {
    pub escrows: HashMap<String, Escrow>,
    pub events: Vec<EscrowEvent>,
}

impl EscrowManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_escrow(&mut self, escrow: Escrow) -> Result<(), EscrowError> {
        if self.escrows.contains_key(&escrow.id) {
            return Err(EscrowError::AlreadyExists(escrow.id.clone()));
        }
        let id = escrow.id.clone();
        self.escrows.insert(id.clone(), escrow);
        self.events.push(EscrowEvent {
            escrow_id: id,
            event_type: "created".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: "Escrow created".to_string(),
        });
        Ok(())
    }

    pub fn fund_escrow(&mut self, id: &str) -> Result<(), EscrowError> {
        let escrow = self
            .escrows
            .get_mut(id)
            .ok_or_else(|| EscrowError::NotFound(id.to_string()))?;
        if escrow.status != EscrowStatus::Created {
            return Err(EscrowError::InvalidStatus {
                expected: "Created".to_string(),
                actual: escrow.status.to_string(),
            });
        }
        escrow.status = EscrowStatus::Funded;
        escrow.funded_at = Some(chrono::Utc::now().to_rfc3339());
        self.events.push(EscrowEvent {
            escrow_id: id.to_string(),
            event_type: "funded".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: "Escrow funded".to_string(),
        });
        Ok(())
    }

    pub fn release_escrow(&mut self, id: &str) -> Result<(), EscrowError> {
        let escrow = self
            .escrows
            .get_mut(id)
            .ok_or_else(|| EscrowError::NotFound(id.to_string()))?;
        if escrow.status != EscrowStatus::Funded {
            return Err(EscrowError::InvalidStatus {
                expected: "Funded".to_string(),
                actual: escrow.status.to_string(),
            });
        }
        escrow.status = EscrowStatus::Released;
        escrow.released_at = Some(chrono::Utc::now().to_rfc3339());
        self.events.push(EscrowEvent {
            escrow_id: id.to_string(),
            event_type: "released".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: "Escrow released to seller".to_string(),
        });
        Ok(())
    }

    pub fn refund_escrow(&mut self, id: &str) -> Result<(), EscrowError> {
        let escrow = self
            .escrows
            .get_mut(id)
            .ok_or_else(|| EscrowError::NotFound(id.to_string()))?;
        if escrow.status != EscrowStatus::Funded {
            return Err(EscrowError::InvalidStatus {
                expected: "Funded".to_string(),
                actual: escrow.status.to_string(),
            });
        }
        escrow.status = EscrowStatus::Refunded;
        self.events.push(EscrowEvent {
            escrow_id: id.to_string(),
            event_type: "refunded".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: "Escrow refunded to buyer".to_string(),
        });
        Ok(())
    }

    pub fn dispute_escrow(&mut self, id: &str, reason: &str) -> Result<(), EscrowError> {
        let escrow = self
            .escrows
            .get_mut(id)
            .ok_or_else(|| EscrowError::NotFound(id.to_string()))?;
        if escrow.status != EscrowStatus::Funded {
            return Err(EscrowError::InvalidStatus {
                expected: "Funded".to_string(),
                actual: escrow.status.to_string(),
            });
        }
        escrow.status = EscrowStatus::Disputed;
        escrow.dispute_reason = Some(reason.to_string());
        self.events.push(EscrowEvent {
            escrow_id: id.to_string(),
            event_type: "disputed".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: format!("Dispute raised: {}", reason),
        });
        Ok(())
    }

    pub fn resolve_dispute(
        &mut self,
        id: &str,
        resolution: DisputeResolution,
    ) -> Result<(), EscrowError> {
        let escrow = self
            .escrows
            .get_mut(id)
            .ok_or_else(|| EscrowError::NotFound(id.to_string()))?;
        if escrow.status != EscrowStatus::Disputed {
            return Err(EscrowError::InvalidStatus {
                expected: "Disputed".to_string(),
                actual: escrow.status.to_string(),
            });
        }
        escrow.status = EscrowStatus::Resolved;
        let detail = match &resolution {
            DisputeResolution::ReleaseToBuyer => "Resolved: released to buyer".to_string(),
            DisputeResolution::ReleaseToSeller => "Resolved: released to seller".to_string(),
            DisputeResolution::Split(buyer_amt, seller_amt) => {
                format!("Resolved: split buyer={} seller={}", buyer_amt, seller_amt)
            }
        };
        self.events.push(EscrowEvent {
            escrow_id: id.to_string(),
            event_type: "resolved".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: detail,
        });
        Ok(())
    }

    pub fn complete_milestone(
        &mut self,
        escrow_id: &str,
        milestone_id: &str,
    ) -> Result<(), EscrowError> {
        let escrow = self
            .escrows
            .get_mut(escrow_id)
            .ok_or_else(|| EscrowError::NotFound(escrow_id.to_string()))?;
        let milestone = escrow
            .milestones
            .iter_mut()
            .find(|m| m.id == milestone_id)
            .ok_or_else(|| EscrowError::MilestoneNotFound(milestone_id.to_string()))?;
        milestone.completed = true;
        milestone.completed_at = Some(chrono::Utc::now().to_rfc3339());
        self.events.push(EscrowEvent {
            escrow_id: escrow_id.to_string(),
            event_type: "milestone_completed".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: format!("Milestone {} completed", milestone_id),
        });
        Ok(())
    }

    pub fn release_milestone(
        &mut self,
        escrow_id: &str,
        milestone_id: &str,
    ) -> Result<u64, EscrowError> {
        let escrow = self
            .escrows
            .get_mut(escrow_id)
            .ok_or_else(|| EscrowError::NotFound(escrow_id.to_string()))?;
        let milestone = escrow
            .milestones
            .iter_mut()
            .find(|m| m.id == milestone_id)
            .ok_or_else(|| EscrowError::MilestoneNotFound(milestone_id.to_string()))?;
        if !milestone.completed {
            return Err(EscrowError::MilestoneNotCompleted(
                milestone_id.to_string(),
            ));
        }
        if milestone.released {
            return Err(EscrowError::MilestoneAlreadyReleased(
                milestone_id.to_string(),
            ));
        }
        milestone.released = true;
        let amount = milestone.amount;
        self.events.push(EscrowEvent {
            escrow_id: escrow_id.to_string(),
            event_type: "milestone_released".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: format!("Milestone {} released: {} tokens", milestone_id, amount),
        });
        Ok(amount)
    }

    pub fn get_escrow(&self, id: &str) -> Option<&Escrow> {
        self.escrows.get(id)
    }

    pub fn active_escrows(&self) -> Vec<&Escrow> {
        self.escrows
            .values()
            .filter(|e| e.status == EscrowStatus::Created || e.status == EscrowStatus::Funded)
            .collect()
    }

    pub fn expired_escrows(&self) -> Vec<String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.escrows
            .values()
            .filter(|e| {
                e.expires_at <= now
                    && e.status != EscrowStatus::Expired
                    && e.status != EscrowStatus::Released
                    && e.status != EscrowStatus::Refunded
                    && e.status != EscrowStatus::Resolved
            })
            .map(|e| e.id.clone())
            .collect()
    }

    pub fn escrows_by_buyer(&self, buyer: &str) -> Vec<&Escrow> {
        self.escrows
            .values()
            .filter(|e| e.buyer == buyer)
            .collect()
    }

    pub fn escrows_by_seller(&self, seller: &str) -> Vec<&Escrow> {
        self.escrows
            .values()
            .filter(|e| e.seller == seller)
            .collect()
    }

    pub fn calculate_fee(&self, escrow_id: &str) -> Result<u64, EscrowError> {
        let escrow = self
            .escrows
            .get(escrow_id)
            .ok_or_else(|| EscrowError::NotFound(escrow_id.to_string()))?;
        Ok(escrow.amount * escrow.fee_bps as u64 / 10000)
    }

    pub fn recent_events(&self, n: usize) -> Vec<&EscrowEvent> {
        self.events.iter().rev().take(n).collect()
    }

    pub fn stats(&self) -> EscrowStats {
        let total_escrows = self.escrows.len();
        let active_escrows = self
            .escrows
            .values()
            .filter(|e| e.status == EscrowStatus::Created || e.status == EscrowStatus::Funded)
            .count();
        let released = self
            .escrows
            .values()
            .filter(|e| e.status == EscrowStatus::Released)
            .count();
        let refunded = self
            .escrows
            .values()
            .filter(|e| e.status == EscrowStatus::Refunded)
            .count();
        let disputed = self
            .escrows
            .values()
            .filter(|e| e.status == EscrowStatus::Disputed)
            .count();
        let total_volume: u64 = self.escrows.values().map(|e| e.amount).sum();
        let total_fees: u64 = self
            .escrows
            .values()
            .filter(|e| e.status == EscrowStatus::Released || e.status == EscrowStatus::Resolved)
            .map(|e| e.amount * e.fee_bps as u64 / 10000)
            .sum();
        EscrowStats {
            total_escrows,
            active_escrows,
            released,
            refunded,
            disputed,
            total_volume,
            total_fees,
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), EscrowError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| EscrowError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| EscrowError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, EscrowError> {
        let data = std::fs::read_to_string(path).map_err(|e| EscrowError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| EscrowError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_escrow(id: &str) -> Escrow {
        Escrow {
            id: id.to_string(),
            buyer: "alice".to_string(),
            seller: "bob".to_string(),
            arbiter: Some("charlie".to_string()),
            token: "EVAP".to_string(),
            amount: 10_000,
            fee_bps: 250,
            status: EscrowStatus::Created,
            created_at: chrono::Utc::now().to_rfc3339(),
            funded_at: None,
            expires_at: "2099-01-01T00:00:00+00:00".to_string(),
            released_at: None,
            description: "Test escrow".to_string(),
            milestones: vec![
                Milestone {
                    id: "m1".to_string(),
                    description: "Phase 1".to_string(),
                    amount: 5000,
                    completed: false,
                    released: false,
                    completed_at: None,
                },
                Milestone {
                    id: "m2".to_string(),
                    description: "Phase 2".to_string(),
                    amount: 5000,
                    completed: false,
                    released: false,
                    completed_at: None,
                },
            ],
            dispute_reason: None,
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("escrow_test_{}_{}", std::process::id(), name))
    }

    #[test]
    fn test_create_escrow() {
        let mut mgr = EscrowManager::new();
        let escrow = make_escrow("e1");
        mgr.create_escrow(escrow).unwrap();
        assert!(mgr.get_escrow("e1").is_some());
    }

    #[test]
    fn test_create_duplicate_escrow() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        assert!(mgr.create_escrow(make_escrow("e1")).is_err());
    }

    #[test]
    fn test_fund_escrow() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        assert_eq!(mgr.get_escrow("e1").unwrap().status, EscrowStatus::Funded);
        assert!(mgr.get_escrow("e1").unwrap().funded_at.is_some());
    }

    #[test]
    fn test_fund_escrow_wrong_status() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        assert!(mgr.fund_escrow("e1").is_err());
    }

    #[test]
    fn test_release_escrow() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        mgr.release_escrow("e1").unwrap();
        assert_eq!(mgr.get_escrow("e1").unwrap().status, EscrowStatus::Released);
        assert!(mgr.get_escrow("e1").unwrap().released_at.is_some());
    }

    #[test]
    fn test_release_escrow_wrong_status() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        assert!(mgr.release_escrow("e1").is_err());
    }

    #[test]
    fn test_refund_escrow() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        mgr.refund_escrow("e1").unwrap();
        assert_eq!(
            mgr.get_escrow("e1").unwrap().status,
            EscrowStatus::Refunded
        );
    }

    #[test]
    fn test_refund_escrow_wrong_status() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        assert!(mgr.refund_escrow("e1").is_err());
    }

    #[test]
    fn test_dispute_escrow() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        mgr.dispute_escrow("e1", "Item not delivered").unwrap();
        assert_eq!(
            mgr.get_escrow("e1").unwrap().status,
            EscrowStatus::Disputed
        );
        assert_eq!(
            mgr.get_escrow("e1").unwrap().dispute_reason.as_deref(),
            Some("Item not delivered")
        );
    }

    #[test]
    fn test_dispute_wrong_status() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        assert!(mgr.dispute_escrow("e1", "reason").is_err());
    }

    #[test]
    fn test_resolve_dispute() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        mgr.dispute_escrow("e1", "Bad quality").unwrap();
        mgr.resolve_dispute("e1", DisputeResolution::ReleaseToSeller)
            .unwrap();
        assert_eq!(
            mgr.get_escrow("e1").unwrap().status,
            EscrowStatus::Resolved
        );
    }

    #[test]
    fn test_resolve_dispute_split() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        mgr.dispute_escrow("e1", "Partial delivery").unwrap();
        mgr.resolve_dispute("e1", DisputeResolution::Split(4000, 6000))
            .unwrap();
        assert_eq!(
            mgr.get_escrow("e1").unwrap().status,
            EscrowStatus::Resolved
        );
    }

    #[test]
    fn test_resolve_not_disputed() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        assert!(mgr
            .resolve_dispute("e1", DisputeResolution::ReleaseToBuyer)
            .is_err());
    }

    #[test]
    fn test_complete_milestone() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.complete_milestone("e1", "m1").unwrap();
        let escrow = mgr.get_escrow("e1").unwrap();
        let m = escrow.milestones.iter().find(|m| m.id == "m1").unwrap();
        assert!(m.completed);
        assert!(m.completed_at.is_some());
    }

    #[test]
    fn test_release_milestone() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.complete_milestone("e1", "m1").unwrap();
        let amount = mgr.release_milestone("e1", "m1").unwrap();
        assert_eq!(amount, 5000);
        let escrow = mgr.get_escrow("e1").unwrap();
        let m = escrow.milestones.iter().find(|m| m.id == "m1").unwrap();
        assert!(m.released);
    }

    #[test]
    fn test_release_milestone_not_completed() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        assert!(mgr.release_milestone("e1", "m1").is_err());
    }

    #[test]
    fn test_release_milestone_already_released() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.complete_milestone("e1", "m1").unwrap();
        mgr.release_milestone("e1", "m1").unwrap();
        assert!(mgr.release_milestone("e1", "m1").is_err());
    }

    #[test]
    fn test_active_escrows() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.create_escrow(make_escrow("e2")).unwrap();
        mgr.fund_escrow("e2").unwrap();
        mgr.release_escrow("e2").unwrap();
        let active = mgr.active_escrows();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "e1");
    }

    #[test]
    fn test_escrows_by_buyer_and_seller() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        assert_eq!(mgr.escrows_by_buyer("alice").len(), 1);
        assert_eq!(mgr.escrows_by_seller("bob").len(), 1);
        assert_eq!(mgr.escrows_by_buyer("nobody").len(), 0);
    }

    #[test]
    fn test_calculate_fee() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        // 10000 * 250 / 10000 = 250
        let fee = mgr.calculate_fee("e1").unwrap();
        assert_eq!(fee, 250);
    }

    #[test]
    fn test_recent_events() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        let events = mgr.recent_events(1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "funded");
    }

    #[test]
    fn test_stats() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.create_escrow(make_escrow("e2")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        mgr.release_escrow("e1").unwrap();
        let stats = mgr.stats();
        assert_eq!(stats.total_escrows, 2);
        assert_eq!(stats.active_escrows, 1); // e2 is Created
        assert_eq!(stats.released, 1);
        assert_eq!(stats.total_volume, 20_000);
        assert_eq!(stats.total_fees, 250); // only released e1
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load.json");
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        mgr.fund_escrow("e1").unwrap();
        mgr.save(&path).unwrap();

        let loaded = EscrowManager::load(&path).unwrap();
        assert_eq!(loaded.escrows.len(), 1);
        assert_eq!(
            loaded.get_escrow("e1").unwrap().status,
            EscrowStatus::Funded
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing() {
        let path = temp_path("nonexistent.json");
        let mgr = EscrowManager::load_or_default(&path);
        assert!(mgr.escrows.is_empty());
    }

    #[test]
    fn test_escrow_not_found() {
        let mgr = EscrowManager::new();
        assert!(mgr.get_escrow("nope").is_none());
        assert!(mgr.calculate_fee("nope").is_err());
    }

    #[test]
    fn test_milestone_not_found() {
        let mut mgr = EscrowManager::new();
        mgr.create_escrow(make_escrow("e1")).unwrap();
        assert!(mgr.complete_milestone("e1", "m99").is_err());
        assert!(mgr.release_milestone("e1", "m99").is_err());
    }
}
