//! Streaming payment system for the EvaporChain wallet.
//!
//! Enables continuous, time-based token transfers between parties. Supports
//! salary disbursements, subscription billing, token vesting, and custom
//! streaming schedules with pause/resume/cancel controls.
//!
//! # Features
//!
//! - Create streams with configurable rate, duration, and cancellability
//! - Withdraw accrued funds based on elapsed time
//! - Pause, resume, and cancel active streams
//! - Query streams by sender, recipient, or status
//! - Detect streams expiring within a time window
//! - Aggregate outflow and withdrawal statistics

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum PaymentStreamError {
    #[error("Stream already exists: {0}")]
    StreamAlreadyExists(String),
    #[error("Stream not found: {0}")]
    StreamNotFound(String),
    #[error("Stream not active: {0}")]
    StreamNotActive(String),
    #[error("Stream not paused: {0}")]
    StreamNotPaused(String),
    #[error("Stream is not cancellable: {0}")]
    StreamNotCancellable(String),
    #[error("Withdraw amount {requested} exceeds available {available}")]
    InsufficientBalance { requested: u64, available: u64 },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StreamStatus {
    Active,
    Paused,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StreamType {
    Salary,
    Subscription,
    Vesting,
    Custom(String),
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentStream {
    pub id: String,
    pub name: String,
    pub sender: String,
    pub recipient: String,
    pub token: String,
    pub total_amount: u64,
    pub withdrawn: u64,
    pub rate_per_second: u64,
    pub stream_type: StreamType,
    pub status: StreamStatus,
    pub created_at: String,
    pub start_time: String,
    pub end_time: String,
    pub last_withdrawal: Option<String>,
    pub cancellable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawalRecord {
    pub stream_id: String,
    pub amount: u64,
    pub timestamp: String,
    pub recipient: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamStats {
    pub total_streams: usize,
    pub active_streams: usize,
    pub paused_streams: usize,
    pub completed_streams: usize,
    pub total_streamed: u64,
    pub total_withdrawn: u64,
    pub total_pending: u64,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PaymentStreamManager {
    pub streams: HashMap<String, PaymentStream>,
    pub withdrawals: Vec<WithdrawalRecord>,
}

impl PaymentStreamManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Stream CRUD --------------------------------------------------------

    pub fn create_stream(&mut self, stream: PaymentStream) -> Result<(), PaymentStreamError> {
        if self.streams.contains_key(&stream.id) {
            return Err(PaymentStreamError::StreamAlreadyExists(stream.id));
        }
        self.streams.insert(stream.id.clone(), stream);
        Ok(())
    }

    pub fn cancel_stream(&mut self, id: &str) -> Result<(), PaymentStreamError> {
        let stream = self
            .streams
            .get_mut(id)
            .ok_or_else(|| PaymentStreamError::StreamNotFound(id.to_string()))?;
        if stream.status != StreamStatus::Active {
            return Err(PaymentStreamError::StreamNotActive(id.to_string()));
        }
        if !stream.cancellable {
            return Err(PaymentStreamError::StreamNotCancellable(id.to_string()));
        }
        stream.status = StreamStatus::Cancelled;
        Ok(())
    }

    pub fn pause_stream(&mut self, id: &str) -> Result<(), PaymentStreamError> {
        let stream = self
            .streams
            .get_mut(id)
            .ok_or_else(|| PaymentStreamError::StreamNotFound(id.to_string()))?;
        if stream.status != StreamStatus::Active {
            return Err(PaymentStreamError::StreamNotActive(id.to_string()));
        }
        stream.status = StreamStatus::Paused;
        Ok(())
    }

    pub fn resume_stream(&mut self, id: &str) -> Result<(), PaymentStreamError> {
        let stream = self
            .streams
            .get_mut(id)
            .ok_or_else(|| PaymentStreamError::StreamNotFound(id.to_string()))?;
        if stream.status != StreamStatus::Paused {
            return Err(PaymentStreamError::StreamNotPaused(id.to_string()));
        }
        stream.status = StreamStatus::Active;
        Ok(())
    }

    pub fn get_stream(&self, id: &str) -> Option<&PaymentStream> {
        self.streams.get(id)
    }

    // -- Withdrawals --------------------------------------------------------

    pub fn withdrawable_amount(&self, id: &str) -> Result<u64, PaymentStreamError> {
        let stream = self
            .streams
            .get(id)
            .ok_or_else(|| PaymentStreamError::StreamNotFound(id.to_string()))?;

        let start = chrono::DateTime::parse_from_rfc3339(&stream.start_time)
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let end = chrono::DateTime::parse_from_rfc3339(&stream.end_time)
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let now = chrono::Utc::now().timestamp();

        let effective_now = std::cmp::min(now, end);
        let elapsed = std::cmp::max(effective_now - start, 0) as u64;

        let streamed = elapsed * stream.rate_per_second;
        let capped = std::cmp::min(streamed, stream.total_amount);
        let available = capped.saturating_sub(stream.withdrawn);

        Ok(available)
    }

    pub fn withdraw(
        &mut self,
        id: &str,
        amount: u64,
    ) -> Result<&WithdrawalRecord, PaymentStreamError> {
        // Calculate available first (borrows self immutably)
        let available = self.withdrawable_amount(id)?;
        if amount > available {
            return Err(PaymentStreamError::InsufficientBalance {
                requested: amount,
                available,
            });
        }

        let now = chrono::Utc::now().to_rfc3339();

        // Update the stream
        let stream = self.streams.get_mut(id).unwrap();
        let recipient = stream.recipient.clone();
        stream.withdrawn += amount;
        stream.last_withdrawal = Some(now.clone());

        // Auto-complete if fully withdrawn
        if stream.withdrawn >= stream.total_amount {
            stream.status = StreamStatus::Completed;
        }

        // Record the withdrawal
        let record = WithdrawalRecord {
            stream_id: id.to_string(),
            amount,
            timestamp: now,
            recipient,
        };
        self.withdrawals.push(record);

        Ok(self.withdrawals.last().unwrap())
    }

    // -- Queries ------------------------------------------------------------

    pub fn streams_by_sender(&self, sender: &str) -> Vec<&PaymentStream> {
        self.streams
            .values()
            .filter(|s| s.sender == sender)
            .collect()
    }

    pub fn streams_by_recipient(&self, recipient: &str) -> Vec<&PaymentStream> {
        self.streams
            .values()
            .filter(|s| s.recipient == recipient)
            .collect()
    }

    pub fn active_streams(&self) -> Vec<&PaymentStream> {
        self.streams
            .values()
            .filter(|s| s.status == StreamStatus::Active)
            .collect()
    }

    pub fn expiring_soon(&self, hours: u64) -> Vec<&PaymentStream> {
        let now = chrono::Utc::now();
        let threshold = now + chrono::Duration::hours(hours as i64);
        let threshold_ts = threshold.timestamp();

        self.streams
            .values()
            .filter(|s| {
                s.status == StreamStatus::Active
                    && chrono::DateTime::parse_from_rfc3339(&s.end_time)
                        .map(|dt| dt.timestamp() <= threshold_ts)
                        .unwrap_or(false)
            })
            .collect()
    }

    pub fn total_outflow(&self, sender: &str) -> u64 {
        self.streams
            .values()
            .filter(|s| s.sender == sender && s.status == StreamStatus::Active)
            .map(|s| s.rate_per_second)
            .sum()
    }

    pub fn withdrawal_history(&self, stream_id: &str) -> Vec<&WithdrawalRecord> {
        self.withdrawals
            .iter()
            .filter(|w| w.stream_id == stream_id)
            .collect()
    }

    // -- Analytics ----------------------------------------------------------

    pub fn stats(&self) -> StreamStats {
        let active_streams = self
            .streams
            .values()
            .filter(|s| s.status == StreamStatus::Active)
            .count();
        let paused_streams = self
            .streams
            .values()
            .filter(|s| s.status == StreamStatus::Paused)
            .count();
        let completed_streams = self
            .streams
            .values()
            .filter(|s| s.status == StreamStatus::Completed)
            .count();

        let total_streamed: u64 = self.streams.values().map(|s| s.total_amount).sum();
        let total_withdrawn: u64 = self.streams.values().map(|s| s.withdrawn).sum();
        let total_pending: u64 = total_streamed.saturating_sub(total_withdrawn);

        StreamStats {
            total_streams: self.streams.len(),
            active_streams,
            paused_streams,
            completed_streams,
            total_streamed,
            total_withdrawn,
            total_pending,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, PaymentStreamError> {
        let data = std::fs::read_to_string(path)?;
        let manager: Self = serde_json::from_str(&data)?;
        Ok(manager)
    }

    pub fn save(&self, path: &Path) -> Result<(), PaymentStreamError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_payment_stream_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    fn sample_stream(id: &str) -> PaymentStream {
        let now = chrono::Utc::now();
        let start = (now - chrono::Duration::hours(1)).to_rfc3339();
        let end = (now + chrono::Duration::hours(1)).to_rfc3339();
        PaymentStream {
            id: id.to_string(),
            name: format!("Stream {}", id),
            sender: "alice".to_string(),
            recipient: "bob".to_string(),
            token: "EVAP".to_string(),
            total_amount: 7200,
            withdrawn: 0,
            rate_per_second: 1,
            stream_type: StreamType::Salary,
            status: StreamStatus::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            start_time: start,
            end_time: end,
            last_withdrawal: None,
            cancellable: true,
        }
    }

    fn past_stream(id: &str) -> PaymentStream {
        let now = chrono::Utc::now();
        let start = (now - chrono::Duration::hours(2)).to_rfc3339();
        let end = (now - chrono::Duration::hours(1)).to_rfc3339();
        PaymentStream {
            id: id.to_string(),
            name: format!("Past Stream {}", id),
            sender: "alice".to_string(),
            recipient: "bob".to_string(),
            token: "EVAP".to_string(),
            total_amount: 3600,
            withdrawn: 0,
            rate_per_second: 1,
            stream_type: StreamType::Vesting,
            status: StreamStatus::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            start_time: start,
            end_time: end,
            last_withdrawal: None,
            cancellable: true,
        }
    }

    #[test]
    fn test_create_stream() {
        let mut mgr = PaymentStreamManager::new();
        assert!(mgr.create_stream(sample_stream("s1")).is_ok());
        assert!(mgr.get_stream("s1").is_some());
    }

    #[test]
    fn test_create_duplicate_stream() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        let err = mgr.create_stream(sample_stream("s1"));
        assert!(err.is_err());
    }

    #[test]
    fn test_cancel_stream() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        assert!(mgr.cancel_stream("s1").is_ok());
        assert_eq!(
            mgr.get_stream("s1").unwrap().status,
            StreamStatus::Cancelled
        );
    }

    #[test]
    fn test_cancel_not_active() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        mgr.pause_stream("s1").unwrap();
        assert!(mgr.cancel_stream("s1").is_err());
    }

    #[test]
    fn test_cancel_not_cancellable() {
        let mut mgr = PaymentStreamManager::new();
        let mut s = sample_stream("s1");
        s.cancellable = false;
        mgr.create_stream(s).unwrap();
        assert!(mgr.cancel_stream("s1").is_err());
    }

    #[test]
    fn test_pause_stream() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        assert!(mgr.pause_stream("s1").is_ok());
        assert_eq!(mgr.get_stream("s1").unwrap().status, StreamStatus::Paused);
    }

    #[test]
    fn test_pause_not_active() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        mgr.cancel_stream("s1").unwrap();
        assert!(mgr.pause_stream("s1").is_err());
    }

    #[test]
    fn test_resume_stream() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        mgr.pause_stream("s1").unwrap();
        assert!(mgr.resume_stream("s1").is_ok());
        assert_eq!(mgr.get_stream("s1").unwrap().status, StreamStatus::Active);
    }

    #[test]
    fn test_resume_not_paused() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        assert!(mgr.resume_stream("s1").is_err());
    }

    #[test]
    fn test_get_stream_not_found() {
        let mgr = PaymentStreamManager::new();
        assert!(mgr.get_stream("nonexistent").is_none());
    }

    #[test]
    fn test_withdrawable_amount() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        // Stream started 1 hour ago with rate 1/sec => ~3600 available
        let available = mgr.withdrawable_amount("s1").unwrap();
        assert!(available >= 3500 && available <= 3700);
    }

    #[test]
    fn test_withdrawable_amount_past_end() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(past_stream("s1")).unwrap();
        // Stream fully elapsed: total_amount = 3600, rate = 1/sec, duration = 3600s
        let available = mgr.withdrawable_amount("s1").unwrap();
        assert_eq!(available, 3600);
    }

    #[test]
    fn test_withdrawable_not_found() {
        let mgr = PaymentStreamManager::new();
        assert!(mgr.withdrawable_amount("nope").is_err());
    }

    #[test]
    fn test_withdraw() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        let result = mgr.withdraw("s1", 100);
        assert!(result.is_ok());
        let rec = result.unwrap();
        assert_eq!(rec.stream_id, "s1");
        assert_eq!(rec.amount, 100);
        assert_eq!(mgr.get_stream("s1").unwrap().withdrawn, 100);
    }

    #[test]
    fn test_withdraw_exceeds_available() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        // Try to withdraw more than total_amount
        let result = mgr.withdraw("s1", 999_999);
        assert!(result.is_err());
    }

    #[test]
    fn test_withdraw_auto_complete() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(past_stream("s1")).unwrap();
        // Past stream has 3600 available, withdraw all
        mgr.withdraw("s1", 3600).unwrap();
        assert_eq!(
            mgr.get_stream("s1").unwrap().status,
            StreamStatus::Completed
        );
    }

    #[test]
    fn test_streams_by_sender() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        let mut s2 = sample_stream("s2");
        s2.sender = "charlie".to_string();
        mgr.create_stream(s2).unwrap();
        assert_eq!(mgr.streams_by_sender("alice").len(), 1);
        assert_eq!(mgr.streams_by_sender("charlie").len(), 1);
    }

    #[test]
    fn test_streams_by_recipient() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        let mut s2 = sample_stream("s2");
        s2.recipient = "dave".to_string();
        mgr.create_stream(s2).unwrap();
        assert_eq!(mgr.streams_by_recipient("bob").len(), 1);
        assert_eq!(mgr.streams_by_recipient("dave").len(), 1);
    }

    #[test]
    fn test_active_streams() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        mgr.create_stream(sample_stream("s2")).unwrap();
        mgr.pause_stream("s2").unwrap();
        assert_eq!(mgr.active_streams().len(), 1);
    }

    #[test]
    fn test_expiring_soon() {
        let mut mgr = PaymentStreamManager::new();
        // sample_stream ends in 1 hour
        mgr.create_stream(sample_stream("s1")).unwrap();
        // Should appear when checking 2 hours ahead
        assert_eq!(mgr.expiring_soon(2).len(), 1);
        // Should not appear when checking 30 minutes ahead (it ends in ~60 min)
        assert_eq!(mgr.expiring_soon(0).len(), 0);
    }

    #[test]
    fn test_total_outflow() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        let mut s2 = sample_stream("s2");
        s2.rate_per_second = 5;
        mgr.create_stream(s2).unwrap();
        assert_eq!(mgr.total_outflow("alice"), 6);
    }

    #[test]
    fn test_withdrawal_history() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        mgr.withdraw("s1", 10).unwrap();
        mgr.withdraw("s1", 20).unwrap();
        let history = mgr.withdrawal_history("s1");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].amount, 10);
        assert_eq!(history[1].amount, 20);
    }

    #[test]
    fn test_stats() {
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        mgr.create_stream(sample_stream("s2")).unwrap();
        mgr.pause_stream("s2").unwrap();
        let stats = mgr.stats();
        assert_eq!(stats.total_streams, 2);
        assert_eq!(stats.active_streams, 1);
        assert_eq!(stats.paused_streams, 1);
        assert_eq!(stats.total_streamed, 14400);
    }

    #[test]
    fn test_save_and_load() {
        let path = tmp_path("save_load");
        let mut mgr = PaymentStreamManager::new();
        mgr.create_stream(sample_stream("s1")).unwrap();
        mgr.save(&path).unwrap();

        let loaded = PaymentStreamManager::load(&path).unwrap();
        assert!(loaded.get_stream("s1").is_some());
        assert_eq!(loaded.get_stream("s1").unwrap().name, "Stream s1");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = tmp_path("nonexistent_file");
        let mgr = PaymentStreamManager::load_or_default(&path);
        assert_eq!(mgr.streams.len(), 0);
    }

    #[test]
    fn test_stream_type_custom() {
        let mut mgr = PaymentStreamManager::new();
        let mut s = sample_stream("s1");
        s.stream_type = StreamType::Custom("Royalties".to_string());
        mgr.create_stream(s).unwrap();
        assert_eq!(
            mgr.get_stream("s1").unwrap().stream_type,
            StreamType::Custom("Royalties".to_string())
        );
    }
}
