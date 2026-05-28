use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum GasProfilerError {
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error("duplicate profile: {0}")]
    DuplicateProfile(String),
    #[error("no samples for profile: {0}")]
    NoSamples(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum OpType {
    Transfer,
    ContractCall,
    ContractDeploy,
    Refresh,
    Stake,
    Unstake,
    NFTMint,
    TokenTransfer,
    Bridge,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SuggestionPriority {
    Low,
    Medium,
    High,
    Critical,
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasSample {
    pub tx_hash: String,
    pub op_type: OpType,
    pub gas_used: u64,
    pub gas_limit: u64,
    pub gas_price: u64,
    pub timestamp: String,
    pub block_number: u64,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasProfile {
    pub id: String,
    pub op_type: OpType,
    pub samples: Vec<GasSample>,
    pub created_at: String,
}

impl GasProfile {
    pub fn avg_gas(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.samples.iter().map(|s| s.gas_used).sum();
        sum as f64 / self.samples.len() as f64
    }

    pub fn min_gas(&self) -> u64 {
        self.samples.iter().map(|s| s.gas_used).min().unwrap_or(0)
    }

    pub fn max_gas(&self) -> u64 {
        self.samples.iter().map(|s| s.gas_used).max().unwrap_or(0)
    }

    pub fn median_gas(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let mut vals: Vec<u64> = self.samples.iter().map(|s| s.gas_used).collect();
        vals.sort_unstable();
        let mid = vals.len() / 2;
        if vals.len().is_multiple_of(2) {
            (vals[mid - 1] + vals[mid]) / 2
        } else {
            vals[mid]
        }
    }

    pub fn p95_gas(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let mut vals: Vec<u64> = self.samples.iter().map(|s| s.gas_used).collect();
        vals.sort_unstable();
        let idx = ((vals.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        let idx = idx.min(vals.len() - 1);
        vals[idx]
    }

    pub fn efficiency(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let ratios: f64 = self
            .samples
            .iter()
            .filter(|s| s.gas_limit > 0)
            .map(|s| s.gas_used as f64 / s.gas_limit as f64)
            .sum();
        let count = self.samples.iter().filter(|s| s.gas_limit > 0).count();
        if count == 0 {
            return 0.0;
        }
        (ratios / count as f64) * 100.0
    }

    pub fn total_cost(&self) -> u64 {
        self.samples.iter().map(|s| s.gas_used * s.gas_price).sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotspot {
    pub op_type: OpType,
    pub avg_gas: f64,
    pub sample_count: usize,
    pub total_cost: u64,
    pub percentage_of_total: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationSuggestion {
    pub id: String,
    pub op_type: OpType,
    pub suggestion: String,
    pub estimated_savings: u64,
    pub priority: SuggestionPriority,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilerStats {
    pub total_profiles: usize,
    pub total_samples: usize,
    pub total_gas_spent: u64,
    pub avg_gas_per_tx: f64,
    pub most_expensive_op: Option<String>,
    pub hotspot_count: usize,
    pub suggestions_count: usize,
}

// ---------------------------------------------------------------------------
// Main struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GasProfiler {
    pub profiles: HashMap<String, GasProfile>,
    pub suggestions: Vec<OptimizationSuggestion>,
}

impl GasProfiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_profile(&mut self, id: &str, op_type: OpType) -> Result<(), GasProfilerError> {
        if self.profiles.contains_key(id) {
            return Err(GasProfilerError::DuplicateProfile(id.to_string()));
        }
        self.profiles.insert(
            id.to_string(),
            GasProfile {
                id: id.to_string(),
                op_type,
                samples: Vec::new(),
                created_at: Utc::now().to_rfc3339(),
            },
        );
        Ok(())
    }

    pub fn remove_profile(&mut self, id: &str) -> Result<GasProfile, GasProfilerError> {
        self.profiles
            .remove(id)
            .ok_or_else(|| GasProfilerError::ProfileNotFound(id.to_string()))
    }

    pub fn add_sample(
        &mut self,
        profile_id: &str,
        sample: GasSample,
    ) -> Result<(), GasProfilerError> {
        let profile = self
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| GasProfilerError::ProfileNotFound(profile_id.to_string()))?;
        profile.samples.push(sample);
        Ok(())
    }

    pub fn get_profile(&self, id: &str) -> Option<&GasProfile> {
        self.profiles.get(id)
    }

    pub fn profiles_by_type(&self, op_type: &OpType) -> Vec<&GasProfile> {
        self.profiles
            .values()
            .filter(|p| &p.op_type == op_type)
            .collect()
    }

    pub fn detect_hotspots(&self) -> Vec<Hotspot> {
        // Group all samples across all profiles by op_type.
        let mut by_type: HashMap<OpType, Vec<&GasSample>> = HashMap::new();
        for profile in self.profiles.values() {
            for sample in &profile.samples {
                by_type
                    .entry(sample.op_type.clone())
                    .or_default()
                    .push(sample);
            }
        }

        let grand_total_cost: u64 = self
            .profiles
            .values()
            .flat_map(|p| &p.samples)
            .map(|s| s.gas_used * s.gas_price)
            .sum();

        let mut hotspots: Vec<Hotspot> = by_type
            .into_iter()
            .map(|(op_type, samples)| {
                let avg_gas =
                    samples.iter().map(|s| s.gas_used).sum::<u64>() as f64 / samples.len() as f64;
                let total_cost: u64 = samples.iter().map(|s| s.gas_used * s.gas_price).sum();
                let percentage_of_total = if grand_total_cost > 0 {
                    total_cost as f64 / grand_total_cost as f64 * 100.0
                } else {
                    0.0
                };
                Hotspot {
                    op_type,
                    avg_gas,
                    sample_count: samples.len(),
                    total_cost,
                    percentage_of_total,
                }
            })
            .collect();

        hotspots.sort_by(|a, b| b.avg_gas.partial_cmp(&a.avg_gas).unwrap());
        hotspots
    }

    pub fn generate_suggestions(&self) -> Vec<OptimizationSuggestion> {
        let mut suggestions = Vec::new();
        let mut counter = 0u64;

        for profile in self.profiles.values() {
            if profile.samples.is_empty() {
                continue;
            }

            // Low efficiency → suggest reducing gas limit.
            let eff = profile.efficiency();
            if eff < 50.0 {
                counter += 1;
                suggestions.push(OptimizationSuggestion {
                    id: format!("sug-{counter}"),
                    op_type: profile.op_type.clone(),
                    suggestion: "Reduce gas limit".to_string(),
                    estimated_savings: (profile.avg_gas() * 0.2) as u64,
                    priority: SuggestionPriority::Medium,
                    created_at: Utc::now().to_rfc3339(),
                });
            }

            // High average gas → suggest batching.
            if profile.avg_gas() > 100_000.0 {
                counter += 1;
                suggestions.push(OptimizationSuggestion {
                    id: format!("sug-{counter}"),
                    op_type: profile.op_type.clone(),
                    suggestion: "Consider batching".to_string(),
                    estimated_savings: (profile.avg_gas() * 0.3) as u64,
                    priority: SuggestionPriority::High,
                    created_at: Utc::now().to_rfc3339(),
                });
            }

            // High variance → inconsistent gas usage.
            if profile.samples.len() >= 2 {
                let avg = profile.avg_gas();
                let variance: f64 = profile
                    .samples
                    .iter()
                    .map(|s| {
                        let diff = s.gas_used as f64 - avg;
                        diff * diff
                    })
                    .sum::<f64>()
                    / profile.samples.len() as f64;
                let stddev = variance.sqrt();
                // If coefficient of variation > 50%, flag it.
                if avg > 0.0 && (stddev / avg) > 0.5 {
                    counter += 1;
                    suggestions.push(OptimizationSuggestion {
                        id: format!("sug-{counter}"),
                        op_type: profile.op_type.clone(),
                        suggestion: "Inconsistent gas usage".to_string(),
                        estimated_savings: stddev as u64,
                        priority: SuggestionPriority::Low,
                        created_at: Utc::now().to_rfc3339(),
                    });
                }
            }
        }

        suggestions
    }

    pub fn add_suggestion(&mut self, suggestion: OptimizationSuggestion) {
        self.suggestions.push(suggestion);
    }

    pub fn all_samples(&self) -> Vec<&GasSample> {
        self.profiles.values().flat_map(|p| &p.samples).collect()
    }

    pub fn recent_samples(&self, n: usize) -> Vec<&GasSample> {
        let mut samples: Vec<&GasSample> = self.all_samples();
        samples.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        samples.truncate(n);
        samples
    }

    pub fn stats(&self) -> ProfilerStats {
        let total_profiles = self.profiles.len();
        let all = self.all_samples();
        let total_samples = all.len();
        let total_gas_spent: u64 = all.iter().map(|s| s.gas_used * s.gas_price).sum();
        let avg_gas_per_tx = if total_samples > 0 {
            all.iter().map(|s| s.gas_used).sum::<u64>() as f64 / total_samples as f64
        } else {
            0.0
        };

        let hotspots = self.detect_hotspots();
        let most_expensive_op = hotspots.first().map(|h| format!("{:?}", h.op_type));
        let generated = self.generate_suggestions();

        ProfilerStats {
            total_profiles,
            total_samples,
            total_gas_spent,
            avg_gas_per_tx,
            most_expensive_op,
            hotspot_count: hotspots.len(),
            suggestions_count: self.suggestions.len() + generated.len(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), GasProfilerError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, GasProfilerError> {
        let data = std::fs::read_to_string(path)?;
        let profiler: Self = serde_json::from_str(&data)?;
        Ok(profiler)
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

    fn sample(gas_used: u64, gas_limit: u64, gas_price: u64) -> GasSample {
        GasSample {
            tx_hash: format!("0x{gas_used:x}"),
            op_type: OpType::Transfer,
            gas_used,
            gas_limit,
            gas_price,
            timestamp: Utc::now().to_rfc3339(),
            block_number: 1,
            success: true,
        }
    }

    fn sample_with_type(op_type: OpType, gas_used: u64) -> GasSample {
        GasSample {
            tx_hash: format!("0x{gas_used:x}"),
            op_type,
            gas_used,
            gas_limit: gas_used * 2,
            gas_price: 10,
            timestamp: Utc::now().to_rfc3339(),
            block_number: 1,
            success: true,
        }
    }

    fn test_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("gas_profiler_test_{}.json", std::process::id()))
    }

    // 1
    #[test]
    fn test_create_profile() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        assert!(gp.get_profile("p1").is_some());
    }

    // 2
    #[test]
    fn test_duplicate_profile() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        let err = gp.create_profile("p1", OpType::Transfer).unwrap_err();
        assert!(matches!(err, GasProfilerError::DuplicateProfile(_)));
    }

    // 3
    #[test]
    fn test_remove_profile() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Stake).unwrap();
        let removed = gp.remove_profile("p1").unwrap();
        assert_eq!(removed.id, "p1");
        assert!(gp.get_profile("p1").is_none());
    }

    // 4
    #[test]
    fn test_remove_profile_not_found() {
        let mut gp = GasProfiler::new();
        let err = gp.remove_profile("nope").unwrap_err();
        assert!(matches!(err, GasProfilerError::ProfileNotFound(_)));
    }

    // 5
    #[test]
    fn test_add_sample() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(21000, 42000, 10)).unwrap();
        assert_eq!(gp.get_profile("p1").unwrap().samples.len(), 1);
    }

    // 6
    #[test]
    fn test_add_sample_profile_not_found() {
        let mut gp = GasProfiler::new();
        let err = gp.add_sample("nope", sample(21000, 42000, 10)).unwrap_err();
        assert!(matches!(err, GasProfilerError::ProfileNotFound(_)));
    }

    // 7
    #[test]
    fn test_avg_gas() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(10000, 50000, 10)).unwrap();
        gp.add_sample("p1", sample(20000, 50000, 10)).unwrap();
        gp.add_sample("p1", sample(30000, 50000, 10)).unwrap();
        let profile = gp.get_profile("p1").unwrap();
        assert!((profile.avg_gas() - 20000.0).abs() < 0.01);
    }

    // 8
    #[test]
    fn test_min_gas() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(5000, 50000, 10)).unwrap();
        gp.add_sample("p1", sample(15000, 50000, 10)).unwrap();
        assert_eq!(gp.get_profile("p1").unwrap().min_gas(), 5000);
    }

    // 9
    #[test]
    fn test_max_gas() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(5000, 50000, 10)).unwrap();
        gp.add_sample("p1", sample(15000, 50000, 10)).unwrap();
        assert_eq!(gp.get_profile("p1").unwrap().max_gas(), 15000);
    }

    // 10
    #[test]
    fn test_median_gas_odd() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(100, 500, 10)).unwrap();
        gp.add_sample("p1", sample(200, 500, 10)).unwrap();
        gp.add_sample("p1", sample(300, 500, 10)).unwrap();
        assert_eq!(gp.get_profile("p1").unwrap().median_gas(), 200);
    }

    // 11
    #[test]
    fn test_median_gas_even() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(100, 500, 10)).unwrap();
        gp.add_sample("p1", sample(200, 500, 10)).unwrap();
        gp.add_sample("p1", sample(300, 500, 10)).unwrap();
        gp.add_sample("p1", sample(400, 500, 10)).unwrap();
        assert_eq!(gp.get_profile("p1").unwrap().median_gas(), 250);
    }

    // 12
    #[test]
    fn test_p95_gas() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        for i in 1..=20 {
            gp.add_sample("p1", sample(i * 1000, 50000, 10)).unwrap();
        }
        let p95 = gp.get_profile("p1").unwrap().p95_gas();
        assert_eq!(p95, 19000);
    }

    // 13
    #[test]
    fn test_efficiency() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        // gas_used=25000, gas_limit=50000 → ratio=0.5 → 50%
        gp.add_sample("p1", sample(25000, 50000, 10)).unwrap();
        let eff = gp.get_profile("p1").unwrap().efficiency();
        assert!((eff - 50.0).abs() < 0.01);
    }

    // 14
    #[test]
    fn test_total_cost() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(1000, 5000, 20)).unwrap();
        gp.add_sample("p1", sample(2000, 5000, 10)).unwrap();
        // 1000*20 + 2000*10 = 40000
        assert_eq!(gp.get_profile("p1").unwrap().total_cost(), 40000);
    }

    // 15
    #[test]
    fn test_profiles_by_type() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.create_profile("p2", OpType::Stake).unwrap();
        gp.create_profile("p3", OpType::Transfer).unwrap();
        let transfers = gp.profiles_by_type(&OpType::Transfer);
        assert_eq!(transfers.len(), 2);
    }

    // 16
    #[test]
    fn test_detect_hotspots() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.create_profile("p2", OpType::ContractCall).unwrap();
        gp.add_sample("p1", sample_with_type(OpType::Transfer, 10000))
            .unwrap();
        gp.add_sample("p2", sample_with_type(OpType::ContractCall, 50000))
            .unwrap();
        let hotspots = gp.detect_hotspots();
        assert_eq!(hotspots.len(), 2);
        // Sorted descending by avg_gas, so ContractCall first.
        assert_eq!(hotspots[0].op_type, OpType::ContractCall);
        assert!(hotspots[0].percentage_of_total > hotspots[1].percentage_of_total);
    }

    // 17
    #[test]
    fn test_generate_suggestions_low_efficiency() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        // gas_used=1000, gas_limit=50000 → efficiency ~2% → "Reduce gas limit"
        gp.add_sample("p1", sample(1000, 50000, 10)).unwrap();
        let suggestions = gp.generate_suggestions();
        assert!(suggestions
            .iter()
            .any(|s| s.suggestion == "Reduce gas limit"));
    }

    // 18
    #[test]
    fn test_generate_suggestions_high_gas() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::ContractDeploy).unwrap();
        gp.add_sample("p1", sample(200_000, 200_000, 10)).unwrap();
        let suggestions = gp.generate_suggestions();
        assert!(suggestions
            .iter()
            .any(|s| s.suggestion == "Consider batching"));
    }

    // 19
    #[test]
    fn test_stats() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(21000, 42000, 10)).unwrap();
        let stats = gp.stats();
        assert_eq!(stats.total_profiles, 1);
        assert_eq!(stats.total_samples, 1);
        assert_eq!(stats.total_gas_spent, 210_000);
    }

    // 20
    #[test]
    fn test_save_and_load() {
        let path = test_path();
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        gp.add_sample("p1", sample(21000, 42000, 10)).unwrap();
        gp.save(&path).unwrap();

        let loaded = GasProfiler::load(&path).unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.get_profile("p1").unwrap().samples.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    // 21
    #[test]
    fn test_load_or_default_missing_file() {
        let path =
            std::env::temp_dir().join(format!("gas_profiler_missing_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let gp = GasProfiler::load_or_default(&path);
        assert!(gp.profiles.is_empty());
    }

    // ─── Additional coverage tests ─────────────────────────────────────────

    #[test]
    fn test_avg_gas_empty_profile_covers_line_78() {
        let mut gp = GasProfiler::new();
        gp.create_profile("empty", OpType::Transfer).unwrap();
        assert_eq!(gp.get_profile("empty").unwrap().avg_gas(), 0.0);
    }

    #[test]
    fn test_median_gas_empty_profile_covers_line_94() {
        let mut gp = GasProfiler::new();
        gp.create_profile("empty", OpType::Transfer).unwrap();
        assert_eq!(gp.get_profile("empty").unwrap().median_gas(), 0);
    }

    #[test]
    fn test_p95_gas_empty_profile_covers_line_108() {
        let mut gp = GasProfiler::new();
        gp.create_profile("empty", OpType::Transfer).unwrap();
        assert_eq!(gp.get_profile("empty").unwrap().p95_gas(), 0);
    }

    #[test]
    fn test_efficiency_empty_profile_covers_line_119() {
        let mut gp = GasProfiler::new();
        gp.create_profile("empty", OpType::Transfer).unwrap();
        assert_eq!(gp.get_profile("empty").unwrap().efficiency(), 0.0);
    }

    #[test]
    fn test_efficiency_zero_gas_limit_covers_line_129() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        // gas_limit == 0 → filtered out → count == 0 → return 0.0
        gp.add_sample("p1", sample(1000, 0, 10)).unwrap();
        assert_eq!(gp.get_profile("p1").unwrap().efficiency(), 0.0);
    }

    #[test]
    fn test_detect_hotspots_zero_grand_total_covers_line_258() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        // gas_price == 0 → total_cost == 0 → grand_total_cost == 0
        gp.add_sample(
            "p1",
            GasSample {
                tx_hash: "0x1".into(),
                op_type: OpType::Transfer,
                gas_used: 1000,
                gas_limit: 2000,
                gas_price: 0,
                timestamp: Utc::now().to_rfc3339(),
                block_number: 1,
                success: true,
            },
        )
        .unwrap();
        let hotspots = gp.detect_hotspots();
        assert_eq!(hotspots.len(), 1);
        assert_eq!(hotspots[0].percentage_of_total, 0.0);
    }

    #[test]
    fn test_generate_suggestions_skips_empty_profile_covers_line_280() {
        let mut gp = GasProfiler::new();
        gp.create_profile("empty", OpType::Transfer).unwrap();
        // No samples → should not generate suggestions
        let suggestions = gp.generate_suggestions();
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_generate_suggestions_high_variance_covers_lines_312_334() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        // Very high variance: 1000 and 100_000 → stddev/avg >> 0.5
        gp.add_sample("p1", sample(1000, 200000, 10)).unwrap();
        gp.add_sample("p1", sample(100000, 200000, 10)).unwrap();
        let suggestions = gp.generate_suggestions();
        assert!(suggestions
            .iter()
            .any(|s| s.suggestion == "Inconsistent gas usage"));
    }

    #[test]
    fn test_add_suggestion_covers_lines_341_343() {
        let mut gp = GasProfiler::new();
        gp.add_suggestion(OptimizationSuggestion {
            id: "manual-1".into(),
            op_type: OpType::Transfer,
            suggestion: "Manual suggestion".into(),
            estimated_savings: 5000,
            priority: SuggestionPriority::Low,
            created_at: Utc::now().to_rfc3339(),
        });
        assert_eq!(gp.suggestions.len(), 1);
    }

    #[test]
    fn test_recent_samples_covers_lines_349_354() {
        let mut gp = GasProfiler::new();
        gp.create_profile("p1", OpType::Transfer).unwrap();
        for i in 1..=5 {
            gp.add_sample("p1", sample(i * 1000, 50000, 10)).unwrap();
        }
        let recent = gp.recent_samples(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_stats_no_samples_covers_line_364() {
        let gp = GasProfiler::new();
        let stats = gp.stats();
        assert_eq!(stats.total_samples, 0);
        assert_eq!(stats.avg_gas_per_tx, 0.0);
    }
}
