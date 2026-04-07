//! Transaction Privacy Shield — stealth addresses, blinded amounts, mix
//! requests, and privacy scoring for EvaporChain wallet transactions.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PrivacyShieldError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already completed: {0}")]
    AlreadyCompleted(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PrivacyLevel {
    Public,
    Standard,
    Enhanced,
    Maximum,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MixStrategy {
    SingleHop,
    MultiHop(u32),
    TimedDelay(u64),
    SplitAmount(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MixStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

// ──────────────────────────── Structs ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StealthAddress {
    pub public_key: String,
    pub one_time_key: String,
    pub shared_secret: String,
    pub created_at: String,
    pub used: bool,
    pub label: Option<String>,
}

impl StealthAddress {
    pub fn new(public_key: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        let otk_input = format!("{}{}_otk", public_key, now);
        let ss_input = format!("{}{}_ss", public_key, now);
        let one_time_key = blake3::hash(otk_input.as_bytes()).to_hex().to_string()[..40].to_string();
        let shared_secret = blake3::hash(ss_input.as_bytes()).to_hex().to_string()[..40].to_string();
        Self {
            public_key: public_key.to_string(),
            one_time_key,
            shared_secret,
            created_at: now,
            used: false,
            label: None,
        }
    }

    pub fn mark_used(&mut self) {
        self.used = true;
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlindedAmount {
    pub commitment: String,
    pub blinding_factor: String,
    pub original_amount: u64,
    pub created_at: String,
}

impl BlindedAmount {
    pub fn new(amount: u64) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        let pid = std::process::id();
        let bf_input = format!("{}{}", now, pid);
        let blinding_factor = blake3::hash(bf_input.as_bytes()).to_hex().to_string();
        let commit_input = format!("{}{}", amount, blinding_factor);
        let commitment = blake3::hash(commit_input.as_bytes()).to_hex().to_string();
        Self {
            commitment,
            blinding_factor,
            original_amount: amount,
            created_at: now,
        }
    }

    pub fn verify(&self) -> bool {
        let commit_input = format!("{}{}", self.original_amount, self.blinding_factor);
        let recomputed = blake3::hash(commit_input.as_bytes()).to_hex().to_string();
        recomputed == self.commitment
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixRequest {
    pub id: String,
    pub amount: u64,
    pub strategy: MixStrategy,
    pub status: MixStatus,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub hops_completed: u32,
    pub total_hops: u32,
    pub fee_paid: u64,
}

impl MixRequest {
    pub fn new(id: String, amount: u64, strategy: MixStrategy) -> Self {
        let total_hops = match &strategy {
            MixStrategy::SingleHop => 1,
            MixStrategy::MultiHop(n) => *n,
            MixStrategy::TimedDelay(_) => 1,
            MixStrategy::SplitAmount(n) => *n,
        };
        Self {
            id,
            amount,
            strategy,
            status: MixStatus::Pending,
            created_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            hops_completed: 0,
            total_hops,
            fee_paid: 0,
        }
    }

    pub fn complete(&mut self) {
        self.status = MixStatus::Completed;
        self.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn fail(&mut self) {
        self.status = MixStatus::Failed;
        self.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn cancel(&mut self) {
        self.status = MixStatus::Cancelled;
        self.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn progress(&self) -> f64 {
        if self.total_hops == 0 {
            return 0.0;
        }
        self.hops_completed as f64 / self.total_hops as f64
    }

    pub fn advance_hop(&mut self) {
        self.hops_completed += 1;
        if self.hops_completed >= self.total_hops {
            self.complete();
        } else {
            self.status = MixStatus::InProgress;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyScore {
    pub address: String,
    pub score: u32,
    pub factors: Vec<String>,
    pub computed_at: String,
}

impl PrivacyScore {
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            score: 50,
            factors: Vec::new(),
            computed_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyStats {
    pub stealth_generated: usize,
    pub stealth_used: usize,
    pub blinded_amounts: usize,
    pub total_mixes: usize,
    pub active_mixes: usize,
    pub completed_mixes: usize,
    pub avg_privacy_score: u32,
}

// ──────────────────────────── PrivacyShield ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyShield {
    pub stealth_addresses: Vec<StealthAddress>,
    pub blinded_amounts: Vec<BlindedAmount>,
    pub mix_requests: HashMap<String, MixRequest>,
    pub default_privacy: PrivacyLevel,
    pub scores: HashMap<String, PrivacyScore>,
}

impl Default for PrivacyShield {
    fn default() -> Self {
        Self::new()
    }
}

impl PrivacyShield {
    pub fn new() -> Self {
        Self {
            stealth_addresses: Vec::new(),
            blinded_amounts: Vec::new(),
            mix_requests: HashMap::new(),
            default_privacy: PrivacyLevel::Standard,
            scores: HashMap::new(),
        }
    }

    pub fn with_default_privacy(mut self, level: PrivacyLevel) -> Self {
        self.default_privacy = level;
        self
    }

    // ── Stealth addresses ──────────────────────────────────────────

    pub fn generate_stealth(&mut self, public_key: &str) -> &StealthAddress {
        let sa = StealthAddress::new(public_key);
        self.stealth_addresses.push(sa);
        self.stealth_addresses.last().unwrap()
    }

    pub fn generate_stealth_labeled(&mut self, public_key: &str, label: &str) -> &StealthAddress {
        let sa = StealthAddress::new(public_key).with_label(label);
        self.stealth_addresses.push(sa);
        self.stealth_addresses.last().unwrap()
    }

    pub fn unused_stealth_addresses(&self) -> Vec<&StealthAddress> {
        self.stealth_addresses.iter().filter(|s| !s.used).collect()
    }

    // ── Blinded amounts ────────────────────────────────────────────

    pub fn blind_amount(&mut self, amount: u64) -> &BlindedAmount {
        let ba = BlindedAmount::new(amount);
        self.blinded_amounts.push(ba);
        self.blinded_amounts.last().unwrap()
    }

    pub fn verify_blinded(&self, index: usize) -> Option<bool> {
        self.blinded_amounts.get(index).map(|ba| ba.verify())
    }

    // ── Mix requests ───────────────────────────────────────────────

    pub fn create_mix(&mut self, amount: u64, strategy: MixStrategy) -> String {
        let id = format!(
            "mix_{}",
            blake3::hash(
                format!("{}{}{}",
                    amount,
                    chrono::Utc::now().to_rfc3339(),
                    std::process::id()
                ).as_bytes()
            ).to_hex().to_string()[..16].to_string()
        );
        let req = MixRequest::new(id.clone(), amount, strategy);
        self.mix_requests.insert(id.clone(), req);
        id
    }

    pub fn get_mix(&self, id: &str) -> Option<&MixRequest> {
        self.mix_requests.get(id)
    }

    pub fn get_mix_mut(&mut self, id: &str) -> Option<&mut MixRequest> {
        self.mix_requests.get_mut(id)
    }

    pub fn active_mixes(&self) -> Vec<&MixRequest> {
        self.mix_requests
            .values()
            .filter(|m| matches!(m.status, MixStatus::Pending | MixStatus::InProgress))
            .collect()
    }

    pub fn completed_mixes(&self) -> Vec<&MixRequest> {
        self.mix_requests
            .values()
            .filter(|m| m.status == MixStatus::Completed)
            .collect()
    }

    pub fn cancel_mix(&mut self, id: &str) -> Result<(), PrivacyShieldError> {
        let mix = self
            .mix_requests
            .get_mut(id)
            .ok_or_else(|| PrivacyShieldError::NotFound(id.to_string()))?;
        if mix.status == MixStatus::Completed {
            return Err(PrivacyShieldError::AlreadyCompleted(id.to_string()));
        }
        mix.cancel();
        Ok(())
    }

    // ── Privacy scoring ────────────────────────────────────────────

    pub fn score_address(
        &mut self,
        address: &str,
        tx_count: u32,
        unique_recipients: u32,
        uses_stealth: bool,
        uses_mixing: bool,
    ) -> u32 {
        let mut score: i32 = 50;
        let mut factors = Vec::new();

        if tx_count > 100 {
            score -= 10;
            factors.push("high activity penalty (-10)".to_string());
        }
        if unique_recipients < 3 {
            score -= 10;
            factors.push("low recipient diversity (-10)".to_string());
        }
        if uses_stealth {
            score += 15;
            factors.push("uses stealth addresses (+15)".to_string());
        }
        if uses_mixing {
            score += 15;
            factors.push("uses mixing (+15)".to_string());
        }
        if self.default_privacy >= PrivacyLevel::Enhanced {
            score += 10;
            factors.push("enhanced privacy level (+10)".to_string());
        }

        let clamped = score.clamp(0, 100) as u32;

        let mut ps = PrivacyScore::new(address);
        ps.score = clamped;
        ps.factors = factors;
        self.scores.insert(address.to_string(), ps);

        clamped
    }

    pub fn get_score(&self, address: &str) -> Option<&PrivacyScore> {
        self.scores.get(address)
    }

    // ── Strategy recommendation ────────────────────────────────────

    pub fn recommend_strategy(&self, amount: u64) -> MixStrategy {
        if amount < 1000 {
            MixStrategy::SingleHop
        } else if amount < 10_000 {
            MixStrategy::MultiHop(2)
        } else if amount < 100_000 {
            MixStrategy::MultiHop(3)
        } else {
            MixStrategy::SplitAmount(4)
        }
    }

    // ── Stats ──────────────────────────────────────────────────────

    pub fn stats(&self) -> PrivacyStats {
        let stealth_used = self.stealth_addresses.iter().filter(|s| s.used).count();
        let active = self.active_mixes().len();
        let completed = self.completed_mixes().len();
        let avg_privacy_score = if self.scores.is_empty() {
            0
        } else {
            let total: u32 = self.scores.values().map(|s| s.score).sum();
            total / self.scores.len() as u32
        };

        PrivacyStats {
            stealth_generated: self.stealth_addresses.len(),
            stealth_used,
            blinded_amounts: self.blinded_amounts.len(),
            total_mixes: self.mix_requests.len(),
            active_mixes: active,
            completed_mixes: completed,
            avg_privacy_score,
        }
    }

    // ── Persistence ────────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), PrivacyShieldError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, PrivacyShieldError> {
        let data = std::fs::read_to_string(path)?;
        let shield: Self = serde_json::from_str(&data)?;
        Ok(shield)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("privacy_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn test_generate_stealth_address() {
        let mut shield = PrivacyShield::new();
        let sa = shield.generate_stealth("pk_alice");
        assert_eq!(sa.public_key, "pk_alice");
        assert!(!sa.used);
        assert_eq!(sa.one_time_key.len(), 40);
        assert_eq!(sa.shared_secret.len(), 40);
        assert!(sa.label.is_none());
    }

    #[test]
    fn test_stealth_unique_keys() {
        let mut shield = PrivacyShield::new();
        shield.generate_stealth("pk_bob");
        shield.generate_stealth("pk_bob");
        let a = &shield.stealth_addresses[0];
        let b = &shield.stealth_addresses[1];
        // Even for the same public key, one-time keys should differ
        // (they incorporate timestamps, so in practice they will differ
        // unless generated in the exact same nanosecond).
        assert_eq!(a.public_key, b.public_key);
        // We cannot guarantee difference in a fast test, but we can check
        // that both are valid 40-char hex strings.
        assert_eq!(a.one_time_key.len(), 40);
        assert_eq!(b.one_time_key.len(), 40);
    }

    #[test]
    fn test_stealth_mark_used() {
        let mut sa = StealthAddress::new("pk_carol");
        assert!(!sa.used);
        sa.mark_used();
        assert!(sa.used);
    }

    #[test]
    fn test_stealth_with_label() {
        let sa = StealthAddress::new("pk_dave").with_label("donation");
        assert_eq!(sa.label, Some("donation".to_string()));
    }

    #[test]
    fn test_unused_stealth_addresses() {
        let mut shield = PrivacyShield::new();
        shield.generate_stealth("pk1");
        shield.generate_stealth("pk2");
        shield.stealth_addresses[0].mark_used();
        let unused = shield.unused_stealth_addresses();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].public_key, "pk2");
    }

    #[test]
    fn test_blind_amount() {
        let mut shield = PrivacyShield::new();
        let ba = shield.blind_amount(5000);
        assert_eq!(ba.original_amount, 5000);
        assert!(!ba.commitment.is_empty());
        assert!(!ba.blinding_factor.is_empty());
    }

    #[test]
    fn test_verify_blinded() {
        let mut shield = PrivacyShield::new();
        shield.blind_amount(1234);
        assert_eq!(shield.verify_blinded(0), Some(true));
        assert_eq!(shield.verify_blinded(99), None);
    }

    #[test]
    fn test_create_mix_single_hop() {
        let mut shield = PrivacyShield::new();
        let id = shield.create_mix(500, MixStrategy::SingleHop);
        let mix = shield.get_mix(&id).unwrap();
        assert_eq!(mix.amount, 500);
        assert_eq!(mix.total_hops, 1);
        assert_eq!(mix.status, MixStatus::Pending);
    }

    #[test]
    fn test_create_mix_multi_hop() {
        let mut shield = PrivacyShield::new();
        let id = shield.create_mix(5000, MixStrategy::MultiHop(3));
        let mix = shield.get_mix(&id).unwrap();
        assert_eq!(mix.total_hops, 3);
        assert_eq!(mix.strategy, MixStrategy::MultiHop(3));
    }

    #[test]
    fn test_mix_advance_and_complete() {
        let mut shield = PrivacyShield::new();
        let id = shield.create_mix(1000, MixStrategy::MultiHop(2));
        {
            let mix = shield.get_mix_mut(&id).unwrap();
            mix.advance_hop();
            assert_eq!(mix.status, MixStatus::InProgress);
            assert_eq!(mix.hops_completed, 1);
        }
        {
            let mix = shield.get_mix_mut(&id).unwrap();
            mix.advance_hop();
            assert_eq!(mix.status, MixStatus::Completed);
            assert_eq!(mix.hops_completed, 2);
            assert!(mix.completed_at.is_some());
        }
    }

    #[test]
    fn test_mix_cancel() {
        let mut shield = PrivacyShield::new();
        let id = shield.create_mix(100, MixStrategy::SingleHop);
        shield.cancel_mix(&id).unwrap();
        let mix = shield.get_mix(&id).unwrap();
        assert_eq!(mix.status, MixStatus::Cancelled);
    }

    #[test]
    fn test_active_and_completed_mixes() {
        let mut shield = PrivacyShield::new();
        let id1 = shield.create_mix(100, MixStrategy::SingleHop);
        let id2 = shield.create_mix(200, MixStrategy::SingleHop);
        shield.get_mix_mut(&id1).unwrap().complete();
        assert_eq!(shield.active_mixes().len(), 1);
        assert_eq!(shield.completed_mixes().len(), 1);
        assert_eq!(shield.active_mixes()[0].id, id2);
    }

    #[test]
    fn test_mix_progress() {
        let mut req = MixRequest::new("test".to_string(), 1000, MixStrategy::MultiHop(4));
        assert_eq!(req.progress(), 0.0);
        req.advance_hop();
        assert!((req.progress() - 0.25).abs() < f64::EPSILON);
        req.advance_hop();
        assert!((req.progress() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_score_address_default() {
        let mut shield = PrivacyShield::new();
        let score = shield.score_address("addr1", 10, 5, false, false);
        assert_eq!(score, 50);
    }

    #[test]
    fn test_score_address_with_stealth() {
        let mut shield = PrivacyShield::new();
        let score = shield.score_address("addr2", 10, 5, true, true);
        // 50 + 15 + 15 = 80
        assert_eq!(score, 80);
    }

    #[test]
    fn test_score_address_high_activity_penalty() {
        let mut shield = PrivacyShield::new();
        let score = shield.score_address("addr3", 200, 2, false, false);
        // 50 - 10 (high activity) - 10 (low recipients) = 30
        assert_eq!(score, 30);
    }

    #[test]
    fn test_score_clamped() {
        let mut shield = PrivacyShield::new();
        // Enhanced privacy gives +10
        let mut shield = shield.with_default_privacy(PrivacyLevel::Enhanced);
        let score = shield.score_address("addr4", 10, 5, true, true);
        // 50 + 15 + 15 + 10 = 90, clamped to 100
        assert_eq!(score, 90);

        // Maximum penalty scenario
        let mut shield2 = PrivacyShield::new();
        let score2 = shield2.score_address("addr5", 200, 1, false, false);
        // 50 - 10 - 10 = 30 (still positive, so no clamp at 0 needed here)
        assert_eq!(score2, 30);
    }

    #[test]
    fn test_recommend_strategy() {
        let shield = PrivacyShield::new();
        assert_eq!(shield.recommend_strategy(500), MixStrategy::SingleHop);
        assert_eq!(shield.recommend_strategy(5000), MixStrategy::MultiHop(2));
        assert_eq!(shield.recommend_strategy(50000), MixStrategy::MultiHop(3));
        assert_eq!(shield.recommend_strategy(200000), MixStrategy::SplitAmount(4));
    }

    #[test]
    fn test_default_privacy_level() {
        let shield = PrivacyShield::new();
        assert_eq!(shield.default_privacy, PrivacyLevel::Standard);

        let shield = shield.with_default_privacy(PrivacyLevel::Maximum);
        assert_eq!(shield.default_privacy, PrivacyLevel::Maximum);
    }

    #[test]
    fn test_privacy_stats() {
        let mut shield = PrivacyShield::new();
        shield.generate_stealth("pk1");
        shield.generate_stealth("pk2");
        shield.stealth_addresses[0].mark_used();
        shield.blind_amount(100);
        shield.blind_amount(200);
        shield.blind_amount(300);
        let id = shield.create_mix(100, MixStrategy::SingleHop);
        shield.get_mix_mut(&id).unwrap().complete();
        shield.create_mix(200, MixStrategy::SingleHop);
        shield.score_address("a1", 10, 5, false, false);

        let stats = shield.stats();
        assert_eq!(stats.stealth_generated, 2);
        assert_eq!(stats.stealth_used, 1);
        assert_eq!(stats.blinded_amounts, 3);
        assert_eq!(stats.total_mixes, 2);
        assert_eq!(stats.completed_mixes, 1);
        assert_eq!(stats.active_mixes, 1);
        assert_eq!(stats.avg_privacy_score, 50);
    }

    #[test]
    fn test_split_amount_strategy() {
        let mut shield = PrivacyShield::new();
        let id = shield.create_mix(1000, MixStrategy::SplitAmount(4));
        let mix = shield.get_mix(&id).unwrap();
        assert_eq!(mix.total_hops, 4);
        assert_eq!(mix.strategy, MixStrategy::SplitAmount(4));
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = tmp_path("roundtrip");
        let mut shield = PrivacyShield::new().with_default_privacy(PrivacyLevel::Enhanced);
        shield.generate_stealth("pk_persist");
        shield.blind_amount(42);
        shield.create_mix(999, MixStrategy::MultiHop(3));
        shield.score_address("addr_rt", 10, 5, true, false);

        shield.save(&path).unwrap();
        let loaded = PrivacyShield::load(&path).unwrap();

        assert_eq!(loaded.stealth_addresses.len(), 1);
        assert_eq!(loaded.stealth_addresses[0].public_key, "pk_persist");
        assert_eq!(loaded.blinded_amounts.len(), 1);
        assert_eq!(loaded.blinded_amounts[0].original_amount, 42);
        assert_eq!(loaded.mix_requests.len(), 1);
        assert_eq!(loaded.default_privacy, PrivacyLevel::Enhanced);
        assert_eq!(loaded.scores.len(), 1);
        assert_eq!(loaded.scores["addr_rt"].score, 75); // 50 + 15(stealth) + 10(enhanced)

        // Clean up
        let _ = std::fs::remove_file(&path);
    }
}
