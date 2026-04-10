// wallet/src/network_health.rs — Chain health monitoring dashboard
//
// Tracks block times, reorgs, epoch progress, finality, TPS, and peer
// counts to give operators an at-a-glance view of chain liveness.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum NetworkHealthError {
    #[error("invalid height: {0}")]
    InvalidHeight(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthGrade {
    Excellent,
    Good,
    Fair,
    Poor,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkEvent {
    BlockProduced,
    Reorg,
    EpochTransition,
    Slowdown,
    Recovery,
    PeerDropped,
    PeerJoined,
}

// ──────────────────────────── Structs ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockTimeSample {
    pub height: u64,
    pub block_time_ms: u64,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorgEvent {
    pub detected_at: String,
    pub from_height: u64,
    pub to_height: u64,
    pub depth: u64,
    pub recovered: bool,
}

impl ReorgEvent {
    pub fn new(from_height: u64, to_height: u64) -> Self {
        Self {
            detected_at: chrono::Utc::now().to_rfc3339(),
            from_height,
            to_height,
            depth: from_height.saturating_sub(to_height),
            recovered: false,
        }
    }

    pub fn mark_recovered(&mut self) {
        self.recovered = true;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochInfo {
    pub epoch: u64,
    pub start_height: u64,
    pub end_height: Option<u64>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub block_count: u64,
    pub avg_block_time_ms: u64,
}

impl EpochInfo {
    pub fn new(epoch: u64, start_height: u64) -> Self {
        Self {
            epoch,
            start_height,
            end_height: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            ended_at: None,
            block_count: 0,
            avg_block_time_ms: 0,
        }
    }

    pub fn duration_secs(&self) -> Option<u64> {
        let ended = self.ended_at.as_ref()?;
        let start = chrono::DateTime::parse_from_rfc3339(&self.started_at).ok()?;
        let end = chrono::DateTime::parse_from_rfc3339(ended).ok()?;
        let delta = end.signed_duration_since(start);
        Some(delta.num_seconds().max(0) as u64)
    }

    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
    }

    pub fn progress(&self, current_height: u64, expected_blocks: u64) -> f64 {
        if expected_blocks == 0 {
            return 0.0;
        }
        let elapsed = current_height.saturating_sub(self.start_height) as f64;
        (elapsed / expected_blocks as f64).clamp(0.0, 1.0)
    }
}

// ──────────────────────────── NetworkStats ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub current_height: u64,
    pub current_epoch: u64,
    pub avg_block_time_ms: u64,
    pub median_block_time_ms: u64,
    pub avg_tps: f64,
    pub peer_count: u32,
    pub reorg_count: usize,
    pub max_reorg_depth: u64,
    pub health_grade: HealthGrade,
    pub finality_depth: u64,
    pub total_events: usize,
}

// ──────────────────────────── Monitor ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkHealthMonitor {
    pub block_times: Vec<BlockTimeSample>,
    pub reorgs: Vec<ReorgEvent>,
    pub epochs: Vec<EpochInfo>,
    pub events: Vec<(String, NetworkEvent)>,
    pub current_height: u64,
    pub current_epoch: u64,
    pub peer_count: u32,
    pub tps_samples: Vec<(String, f64)>,
    pub finality_depth: u64,
}

impl Default for NetworkHealthMonitor {
    fn default() -> Self {
        Self {
            block_times: Vec::new(),
            reorgs: Vec::new(),
            epochs: Vec::new(),
            events: Vec::new(),
            current_height: 0,
            current_epoch: 0,
            peer_count: 0,
            tps_samples: Vec::new(),
            finality_depth: 6,
        }
    }
}

impl NetworkHealthMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_finality_depth(mut self, depth: u64) -> Self {
        self.finality_depth = depth;
        self
    }

    // ── Recording ────────────────────────────────────────────────

    pub fn record_block(&mut self, height: u64, block_time_ms: u64) {
        let now = chrono::Utc::now().to_rfc3339();
        self.block_times.push(BlockTimeSample {
            height,
            block_time_ms,
            recorded_at: now.clone(),
        });
        self.current_height = height;
        if self.block_times.len() > 500 {
            let excess = self.block_times.len() - 500;
            self.block_times.drain(..excess);
        }
        self.push_event(NetworkEvent::BlockProduced);
    }

    pub fn record_reorg(&mut self, from_height: u64, to_height: u64) {
        let reorg = ReorgEvent::new(from_height, to_height);
        self.reorgs.push(reorg);
        self.push_event(NetworkEvent::Reorg);
    }

    pub fn record_epoch(&mut self, epoch: u64, start_height: u64) {
        // Close previous epoch
        if let Some(prev) = self.epochs.last_mut() {
            if prev.is_active() {
                prev.end_height = Some(start_height.saturating_sub(1));
                prev.ended_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }
        let info = EpochInfo::new(epoch, start_height);
        self.epochs.push(info);
        self.current_epoch = epoch;
        self.push_event(NetworkEvent::EpochTransition);
    }

    pub fn record_tps(&mut self, tps: f64) {
        let now = chrono::Utc::now().to_rfc3339();
        self.tps_samples.push((now, tps));
        if self.tps_samples.len() > 500 {
            let excess = self.tps_samples.len() - 500;
            self.tps_samples.drain(..excess);
        }
    }

    pub fn set_peer_count(&mut self, count: u32) {
        let old = self.peer_count;
        self.peer_count = count;
        if count > old {
            self.push_event(NetworkEvent::PeerJoined);
        } else if count < old {
            self.push_event(NetworkEvent::PeerDropped);
        }
    }

    // ── Queries ──────────────────────────────────────────────────

    pub fn avg_block_time(&self) -> u64 {
        let samples: Vec<u64> = self
            .block_times
            .iter()
            .rev()
            .take(100)
            .map(|s| s.block_time_ms)
            .collect();
        if samples.is_empty() {
            return 0;
        }
        let sum: u64 = samples.iter().sum();
        sum / samples.len() as u64
    }

    pub fn median_block_time(&self) -> u64 {
        let mut samples: Vec<u64> = self
            .block_times
            .iter()
            .rev()
            .take(100)
            .map(|s| s.block_time_ms)
            .collect();
        if samples.is_empty() {
            return 0;
        }
        samples.sort_unstable();
        samples[samples.len() / 2]
    }

    pub fn avg_tps(&self) -> f64 {
        let samples: Vec<f64> = self
            .tps_samples
            .iter()
            .rev()
            .take(50)
            .map(|(_, t)| *t)
            .collect();
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().sum();
        sum / samples.len() as f64
    }

    pub fn health_grade(&self) -> HealthGrade {
        let avg = self.avg_block_time();
        let mut grade = if avg < 3000 {
            HealthGrade::Excellent
        } else if avg < 5000 {
            HealthGrade::Good
        } else if avg < 10000 {
            HealthGrade::Fair
        } else if avg < 20000 {
            HealthGrade::Poor
        } else {
            HealthGrade::Critical
        };

        // Degrade if reorgs occurred in last 10 blocks
        let recent_reorg = self.reorgs.iter().any(|r| {
            r.from_height + 10 >= self.current_height
        });
        if recent_reorg {
            grade = Self::degrade(grade);
        }

        grade
    }

    pub fn is_finalized(&self, height: u64) -> bool {
        self.current_height >= height && self.current_height - height >= self.finality_depth
    }

    pub fn reorg_count(&self) -> usize {
        self.reorgs.len()
    }

    pub fn max_reorg_depth(&self) -> u64 {
        self.reorgs.iter().map(|r| r.depth).max().unwrap_or(0)
    }

    pub fn current_epoch_info(&self) -> Option<&EpochInfo> {
        self.epochs.last()
    }

    pub fn epoch_progress(&self, expected_blocks: u64) -> f64 {
        match self.current_epoch_info() {
            Some(epoch) => epoch.progress(self.current_height, expected_blocks),
            None => 0.0,
        }
    }

    pub fn recent_events(&self, n: usize) -> Vec<&(String, NetworkEvent)> {
        self.events.iter().rev().take(n).collect()
    }

    pub fn stats(&self) -> NetworkStats {
        NetworkStats {
            current_height: self.current_height,
            current_epoch: self.current_epoch,
            avg_block_time_ms: self.avg_block_time(),
            median_block_time_ms: self.median_block_time(),
            avg_tps: self.avg_tps(),
            peer_count: self.peer_count,
            reorg_count: self.reorg_count(),
            max_reorg_depth: self.max_reorg_depth(),
            health_grade: self.health_grade(),
            finality_depth: self.finality_depth,
            total_events: self.events.len(),
        }
    }

    pub fn grade_description(grade: &HealthGrade) -> &'static str {
        match grade {
            HealthGrade::Excellent => "Chain running optimally",
            HealthGrade::Good => "Normal operation",
            HealthGrade::Fair => "Some delays detected",
            HealthGrade::Poor => "Significant delays",
            HealthGrade::Critical => "Chain may be stalled",
        }
    }

    // ── Persistence ──────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), NetworkHealthError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, NetworkHealthError> {
        let data = std::fs::read_to_string(path)?;
        let monitor: Self = serde_json::from_str(&data)?;
        Ok(monitor)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    // ── Internal ─────────────────────────────────────────────────

    fn push_event(&mut self, event: NetworkEvent) {
        let now = chrono::Utc::now().to_rfc3339();
        self.events.push((now, event));
        if self.events.len() > 1000 {
            let excess = self.events.len() - 1000;
            self.events.drain(..excess);
        }
    }

    fn degrade(grade: HealthGrade) -> HealthGrade {
        match grade {
            HealthGrade::Excellent => HealthGrade::Good,
            HealthGrade::Good => HealthGrade::Fair,
            HealthGrade::Fair => HealthGrade::Poor,
            HealthGrade::Poor => HealthGrade::Critical,
            HealthGrade::Critical => HealthGrade::Critical,
        }
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("net_health_test_{}", std::process::id()))
            .join(format!("{}.json", name))
    }

    #[test]
    fn test_record_block() {
        let mut m = NetworkHealthMonitor::new();
        m.record_block(1, 2000);
        m.record_block(2, 2500);
        assert_eq!(m.current_height, 2);
        assert_eq!(m.block_times.len(), 2);
        assert_eq!(m.block_times[0].height, 1);
        assert_eq!(m.block_times[1].block_time_ms, 2500);
    }

    #[test]
    fn test_avg_block_time() {
        let mut m = NetworkHealthMonitor::new();
        m.record_block(1, 1000);
        m.record_block(2, 3000);
        // avg = (1000 + 3000) / 2 = 2000
        assert_eq!(m.avg_block_time(), 2000);
    }

    #[test]
    fn test_avg_block_time_empty() {
        let m = NetworkHealthMonitor::new();
        assert_eq!(m.avg_block_time(), 0);
    }

    #[test]
    fn test_median_block_time() {
        let mut m = NetworkHealthMonitor::new();
        m.record_block(1, 1000);
        m.record_block(2, 2000);
        m.record_block(3, 9000);
        // sorted: [1000, 2000, 9000], median at index 1 = 2000
        assert_eq!(m.median_block_time(), 2000);
    }

    #[test]
    fn test_record_reorg() {
        let mut m = NetworkHealthMonitor::new();
        m.record_reorg(100, 97);
        assert_eq!(m.reorgs.len(), 1);
        assert_eq!(m.reorgs[0].depth, 3);
        assert!(!m.reorgs[0].recovered);
    }

    #[test]
    fn test_max_reorg_depth() {
        let mut m = NetworkHealthMonitor::new();
        m.record_reorg(100, 97); // depth 3
        m.record_reorg(200, 195); // depth 5
        m.record_reorg(300, 299); // depth 1
        assert_eq!(m.max_reorg_depth(), 5);
    }

    #[test]
    fn test_max_reorg_depth_empty() {
        let m = NetworkHealthMonitor::new();
        assert_eq!(m.max_reorg_depth(), 0);
    }

    #[test]
    fn test_health_grade_excellent() {
        let mut m = NetworkHealthMonitor::new();
        for i in 1..=10 {
            m.record_block(i, 2000);
        }
        assert_eq!(m.health_grade(), HealthGrade::Excellent);
    }

    #[test]
    fn test_health_grade_poor() {
        let mut m = NetworkHealthMonitor::new();
        for i in 1..=10 {
            m.record_block(i, 15000);
        }
        assert_eq!(m.health_grade(), HealthGrade::Poor);
    }

    #[test]
    fn test_health_grade_degrades_on_reorg() {
        let mut m = NetworkHealthMonitor::new();
        for i in 1..=10 {
            m.record_block(i, 2000);
        }
        // Without reorg = Excellent
        assert_eq!(m.health_grade(), HealthGrade::Excellent);
        // Reorg in recent 10 blocks degrades by one
        m.record_reorg(8, 5);
        assert_eq!(m.health_grade(), HealthGrade::Good);
    }

    #[test]
    fn test_is_finalized() {
        let mut m = NetworkHealthMonitor::new();
        m.current_height = 100;
        m.finality_depth = 6;
        assert!(m.is_finalized(94));
        assert!(m.is_finalized(90));
    }

    #[test]
    fn test_not_finalized() {
        let mut m = NetworkHealthMonitor::new();
        m.current_height = 100;
        m.finality_depth = 6;
        assert!(!m.is_finalized(95));
        assert!(!m.is_finalized(100));
    }

    #[test]
    fn test_record_epoch() {
        let mut m = NetworkHealthMonitor::new();
        m.record_epoch(1, 0);
        m.record_epoch(2, 100);
        assert_eq!(m.epochs.len(), 2);
        assert_eq!(m.current_epoch, 2);
        // Previous epoch should be closed
        assert!(!m.epochs[0].is_active());
        assert!(m.epochs[0].end_height.is_some());
        assert!(m.epochs[1].is_active());
    }

    #[test]
    fn test_epoch_progress() {
        let mut m = NetworkHealthMonitor::new();
        m.record_epoch(1, 0);
        m.current_height = 50;
        // 50 blocks out of 100 expected = 0.5
        let progress = m.epoch_progress(100);
        assert!((progress - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_epoch_progress_clamped() {
        let mut m = NetworkHealthMonitor::new();
        m.record_epoch(1, 0);
        m.current_height = 200;
        // 200/100 would be 2.0 but clamped to 1.0
        let progress = m.epoch_progress(100);
        assert!((progress - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_current_epoch_info() {
        let mut m = NetworkHealthMonitor::new();
        assert!(m.current_epoch_info().is_none());
        m.record_epoch(1, 0);
        let info = m.current_epoch_info().unwrap();
        assert_eq!(info.epoch, 1);
        assert!(info.is_active());
    }

    #[test]
    fn test_record_tps() {
        let mut m = NetworkHealthMonitor::new();
        m.record_tps(100.0);
        m.record_tps(200.0);
        assert_eq!(m.tps_samples.len(), 2);
    }

    #[test]
    fn test_avg_tps() {
        let mut m = NetworkHealthMonitor::new();
        m.record_tps(100.0);
        m.record_tps(200.0);
        m.record_tps(300.0);
        assert!((m.avg_tps() - 200.0).abs() < 0.001);
    }

    #[test]
    fn test_avg_tps_empty() {
        let m = NetworkHealthMonitor::new();
        assert!((m.avg_tps() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_peer_count_events() {
        let mut m = NetworkHealthMonitor::new();
        m.set_peer_count(5);
        // 0 -> 5 = PeerJoined
        assert_eq!(m.events.len(), 1);
        assert_eq!(m.events[0].1, NetworkEvent::PeerJoined);

        m.set_peer_count(3);
        // 5 -> 3 = PeerDropped
        assert_eq!(m.events.len(), 2);
        assert_eq!(m.events[1].1, NetworkEvent::PeerDropped);

        // Same count = no event
        m.set_peer_count(3);
        assert_eq!(m.events.len(), 2);
    }

    #[test]
    fn test_recent_events() {
        let mut m = NetworkHealthMonitor::new();
        m.record_block(1, 1000);
        m.record_block(2, 2000);
        m.record_block(3, 3000);
        let recent = m.recent_events(2);
        assert_eq!(recent.len(), 2);
        // Most recent first
        assert_eq!(recent[0].1, NetworkEvent::BlockProduced);
    }

    #[test]
    fn test_grade_description() {
        assert_eq!(
            NetworkHealthMonitor::grade_description(&HealthGrade::Excellent),
            "Chain running optimally"
        );
        assert_eq!(
            NetworkHealthMonitor::grade_description(&HealthGrade::Critical),
            "Chain may be stalled"
        );
    }

    #[test]
    fn test_stats() {
        let mut m = NetworkHealthMonitor::new();
        for i in 1..=5 {
            m.record_block(i, 2000);
        }
        m.record_tps(150.0);
        m.set_peer_count(10);

        let s = m.stats();
        assert_eq!(s.current_height, 5);
        assert_eq!(s.avg_block_time_ms, 2000);
        assert_eq!(s.peer_count, 10);
        assert_eq!(s.health_grade, HealthGrade::Excellent);
        assert_eq!(s.finality_depth, 6);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut m = NetworkHealthMonitor::new().with_finality_depth(10);
        m.record_block(1, 2000);
        m.record_block(2, 3000);
        m.record_reorg(50, 47);
        m.record_tps(120.0);
        m.set_peer_count(8);

        m.save(&path).expect("save failed");
        let loaded = NetworkHealthMonitor::load(&path).expect("load failed");

        assert_eq!(loaded.current_height, 2);
        assert_eq!(loaded.finality_depth, 10);
        assert_eq!(loaded.block_times.len(), 2);
        assert_eq!(loaded.reorgs.len(), 1);
        assert_eq!(loaded.peer_count, 8);

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing() {
        let path = test_path("does_not_exist");
        let m = NetworkHealthMonitor::load_or_default(&path);
        assert_eq!(m.current_height, 0);
        assert_eq!(m.finality_depth, 6);
    }

    #[test]
    fn test_reorg_mark_recovered() {
        let mut r = ReorgEvent::new(100, 95);
        assert!(!r.recovered);
        r.mark_recovered();
        assert!(r.recovered);
    }

    #[test]
    fn test_block_time_pruning() {
        let mut m = NetworkHealthMonitor::new();
        for i in 1..=550 {
            m.record_block(i, 1000);
        }
        assert_eq!(m.block_times.len(), 500);
        assert_eq!(m.block_times[0].height, 51);
    }

    #[test]
    fn test_with_finality_depth() {
        let m = NetworkHealthMonitor::new().with_finality_depth(12);
        assert_eq!(m.finality_depth, 12);
    }

    #[test]
    fn test_epoch_duration() {
        let mut e = EpochInfo::new(1, 0);
        assert!(e.duration_secs().is_none());
        // Set ended_at to a time slightly after started_at
        e.ended_at = Some(e.started_at.clone());
        assert_eq!(e.duration_secs(), Some(0));
    }
}
