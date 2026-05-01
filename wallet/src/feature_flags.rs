use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum FeatureFlagError {
    #[error("Flag not found: {0}")]
    FlagNotFound(String),
    #[error("Duplicate flag: {0}")]
    DuplicateFlag(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum FlagStatus2 {
    Enabled,
    #[default]
    Disabled,
    Rollout(u8),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum FlagCategory {
    #[default]
    Core,
    Experimental,
    Beta,
    Deprecated,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: FlagStatus2,
    pub category: FlagCategory,
    pub created_at: String,
    pub updated_at: String,
    pub kill_switch: bool,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagOverride {
    pub flag_id: String,
    pub user_id: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagEvaluation {
    pub flag_id: String,
    pub user_id: String,
    pub enabled: bool,
    pub reason: String,
    pub evaluated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagStats {
    pub total_flags: usize,
    pub enabled: usize,
    pub disabled: usize,
    pub rollout: usize,
    pub overrides: usize,
    pub evaluations: u64,
    pub kill_switches: usize,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeatureFlagManager {
    pub flags: HashMap<String, FeatureFlag>,
    pub overrides: Vec<FlagOverride>,
    pub evaluations: Vec<FlagEvaluation>,
    pub eval_count: u64,
}

impl FeatureFlagManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_flag(&mut self, flag: FeatureFlag) -> Result<(), FeatureFlagError> {
        if self.flags.contains_key(&flag.id) {
            return Err(FeatureFlagError::DuplicateFlag(flag.id.clone()));
        }
        self.flags.insert(flag.id.clone(), flag);
        Ok(())
    }

    pub fn remove_flag(&mut self, id: &str) -> Result<FeatureFlag, FeatureFlagError> {
        self.flags
            .remove(id)
            .ok_or_else(|| FeatureFlagError::FlagNotFound(id.to_string()))
    }

    pub fn enable_flag(&mut self, id: &str) -> Result<(), FeatureFlagError> {
        let flag = self
            .flags
            .get_mut(id)
            .ok_or_else(|| FeatureFlagError::FlagNotFound(id.to_string()))?;
        flag.status = FlagStatus2::Enabled;
        flag.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    pub fn disable_flag(&mut self, id: &str) -> Result<(), FeatureFlagError> {
        let flag = self
            .flags
            .get_mut(id)
            .ok_or_else(|| FeatureFlagError::FlagNotFound(id.to_string()))?;
        flag.status = FlagStatus2::Disabled;
        flag.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    pub fn set_rollout(&mut self, id: &str, pct: u8) -> Result<(), FeatureFlagError> {
        let flag = self
            .flags
            .get_mut(id)
            .ok_or_else(|| FeatureFlagError::FlagNotFound(id.to_string()))?;
        flag.status = FlagStatus2::Rollout(pct);
        flag.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    pub fn kill_switch(&mut self, id: &str) -> Result<(), FeatureFlagError> {
        let flag = self
            .flags
            .get_mut(id)
            .ok_or_else(|| FeatureFlagError::FlagNotFound(id.to_string()))?;
        flag.status = FlagStatus2::Disabled;
        flag.kill_switch = true;
        flag.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    pub fn is_enabled(&mut self, flag_id: &str, user_id: &str) -> Result<bool, FeatureFlagError> {
        let flag = self
            .flags
            .get(flag_id)
            .ok_or_else(|| FeatureFlagError::FlagNotFound(flag_id.to_string()))?;

        // Check overrides first
        if let Some(ovr) = self
            .overrides
            .iter()
            .find(|o| o.flag_id == flag_id && o.user_id == user_id)
        {
            let enabled = ovr.enabled;
            self.log_evaluation(flag_id, user_id, enabled, "override");
            return Ok(enabled);
        }

        // Evaluate based on flag status
        let (enabled, reason) = match &flag.status {
            FlagStatus2::Enabled => (true, "enabled"),
            FlagStatus2::Disabled => (false, "disabled"),
            FlagStatus2::Rollout(pct) => {
                let hash_val = rollout_hash(user_id);
                let in_rollout = hash_val < *pct as u32;
                (in_rollout, "rollout")
            }
        };

        self.log_evaluation(flag_id, user_id, enabled, reason);
        Ok(enabled)
    }

    fn log_evaluation(&mut self, flag_id: &str, user_id: &str, enabled: bool, reason: &str) {
        self.eval_count += 1;
        self.evaluations.push(FlagEvaluation {
            flag_id: flag_id.to_string(),
            user_id: user_id.to_string(),
            enabled,
            reason: reason.to_string(),
            evaluated_at: Utc::now().to_rfc3339(),
        });
    }

    pub fn add_override(
        &mut self,
        flag_id: &str,
        user_id: &str,
        enabled: bool,
    ) -> Result<(), FeatureFlagError> {
        if !self.flags.contains_key(flag_id) {
            return Err(FeatureFlagError::FlagNotFound(flag_id.to_string()));
        }
        self.overrides.push(FlagOverride {
            flag_id: flag_id.to_string(),
            user_id: user_id.to_string(),
            enabled,
            created_at: Utc::now().to_rfc3339(),
        });
        Ok(())
    }

    pub fn remove_override(
        &mut self,
        flag_id: &str,
        user_id: &str,
    ) -> Result<(), FeatureFlagError> {
        let before = self.overrides.len();
        self.overrides
            .retain(|o| !(o.flag_id == flag_id && o.user_id == user_id));
        if self.overrides.len() == before {
            return Err(FeatureFlagError::FlagNotFound(format!(
                "override {flag_id}/{user_id}"
            )));
        }
        Ok(())
    }

    pub fn flags_by_category(&self, cat: &FlagCategory) -> Vec<&FeatureFlag> {
        self.flags.values().filter(|f| f.category == *cat).collect()
    }

    pub fn enabled_flags(&self) -> Vec<&FeatureFlag> {
        self.flags
            .values()
            .filter(|f| f.status == FlagStatus2::Enabled)
            .collect()
    }

    pub fn killed_flags(&self) -> Vec<&FeatureFlag> {
        self.flags.values().filter(|f| f.kill_switch).collect()
    }

    pub fn overrides_for_flag(&self, flag_id: &str) -> Vec<&FlagOverride> {
        self.overrides
            .iter()
            .filter(|o| o.flag_id == flag_id)
            .collect()
    }

    pub fn recent_evaluations(&self, n: usize) -> Vec<&FlagEvaluation> {
        let len = self.evaluations.len();
        let start = len.saturating_sub(n);
        self.evaluations[start..].iter().collect()
    }

    pub fn stats(&self) -> FlagStats {
        let mut enabled = 0usize;
        let mut disabled = 0usize;
        let mut rollout = 0usize;
        let mut kill_switches = 0usize;

        for f in self.flags.values() {
            match f.status {
                FlagStatus2::Enabled => enabled += 1,
                FlagStatus2::Disabled => disabled += 1,
                FlagStatus2::Rollout(_) => rollout += 1,
            }
            if f.kill_switch {
                kill_switches += 1;
            }
        }

        FlagStats {
            total_flags: self.flags.len(),
            enabled,
            disabled,
            rollout,
            overrides: self.overrides.len(),
            evaluations: self.eval_count,
            kill_switches,
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), FeatureFlagError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, FeatureFlagError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

/// Simple deterministic hash: sum the bytes of user_id, mod 100.
fn rollout_hash(user_id: &str) -> u32 {
    let sum: u32 = user_id.bytes().map(|b| b as u32).sum();
    sum % 100
}

// ---------------------------------------------------------------------------
// Helper to build a test flag
// ---------------------------------------------------------------------------

#[cfg(test)]
fn make_flag(id: &str, status: FlagStatus2, category: FlagCategory) -> FeatureFlag {
    let now = Utc::now().to_rfc3339();
    FeatureFlag {
        id: id.to_string(),
        name: format!("{id}_name"),
        description: format!("{id} description"),
        status,
        category,
        created_at: now.clone(),
        updated_at: now,
        kill_switch: false,
        metadata: HashMap::new(),
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
        temp_dir().join(format!("evaporchain_ff_{}_{}.json", process::id(), name))
    }

    #[test]
    fn test_register_flag() {
        let mut mgr = FeatureFlagManager::new();
        let flag = make_flag("f1", FlagStatus2::Disabled, FlagCategory::Core);
        assert!(mgr.register_flag(flag).is_ok());
        assert_eq!(mgr.flags.len(), 1);
    }

    #[test]
    fn test_register_duplicate() {
        let mut mgr = FeatureFlagManager::new();
        let f1 = make_flag("dup", FlagStatus2::Disabled, FlagCategory::Core);
        let f2 = make_flag("dup", FlagStatus2::Enabled, FlagCategory::Beta);
        mgr.register_flag(f1).unwrap();
        let err = mgr.register_flag(f2).unwrap_err();
        assert!(matches!(err, FeatureFlagError::DuplicateFlag(_)));
    }

    #[test]
    fn test_remove_flag() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("r1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        let removed = mgr.remove_flag("r1").unwrap();
        assert_eq!(removed.id, "r1");
        assert!(mgr.flags.is_empty());
    }

    #[test]
    fn test_remove_flag_not_found() {
        let mut mgr = FeatureFlagManager::new();
        assert!(matches!(
            mgr.remove_flag("nope"),
            Err(FeatureFlagError::FlagNotFound(_))
        ));
    }

    #[test]
    fn test_enable_flag() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("e1", FlagStatus2::Disabled, FlagCategory::Core))
            .unwrap();
        mgr.enable_flag("e1").unwrap();
        assert_eq!(mgr.flags["e1"].status, FlagStatus2::Enabled);
    }

    #[test]
    fn test_disable_flag() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("d1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        mgr.disable_flag("d1").unwrap();
        assert_eq!(mgr.flags["d1"].status, FlagStatus2::Disabled);
    }

    #[test]
    fn test_set_rollout() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("ro1", FlagStatus2::Disabled, FlagCategory::Beta))
            .unwrap();
        mgr.set_rollout("ro1", 50).unwrap();
        assert_eq!(mgr.flags["ro1"].status, FlagStatus2::Rollout(50));
    }

    #[test]
    fn test_kill_switch() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("k1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        mgr.kill_switch("k1").unwrap();
        assert_eq!(mgr.flags["k1"].status, FlagStatus2::Disabled);
        assert!(mgr.flags["k1"].kill_switch);
    }

    #[test]
    fn test_is_enabled_enabled_flag() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("ie1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        assert!(mgr.is_enabled("ie1", "user1").unwrap());
    }

    #[test]
    fn test_is_enabled_disabled_flag() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("id1", FlagStatus2::Disabled, FlagCategory::Core))
            .unwrap();
        assert!(!mgr.is_enabled("id1", "user1").unwrap());
    }

    #[test]
    fn test_is_enabled_rollout() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag(
            "iro",
            FlagStatus2::Rollout(50),
            FlagCategory::Beta,
        ))
        .unwrap();
        // Deterministic: we know the hash for "user_a"
        let hash_val = rollout_hash("user_a");
        let expected = hash_val < 50;
        assert_eq!(mgr.is_enabled("iro", "user_a").unwrap(), expected);
    }

    #[test]
    fn test_is_enabled_with_override() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("iov", FlagStatus2::Disabled, FlagCategory::Core))
            .unwrap();
        mgr.add_override("iov", "user_x", true).unwrap();
        // Override should win over disabled status
        assert!(mgr.is_enabled("iov", "user_x").unwrap());
    }

    #[test]
    fn test_is_enabled_not_found() {
        let mut mgr = FeatureFlagManager::new();
        assert!(matches!(
            mgr.is_enabled("ghost", "u"),
            Err(FeatureFlagError::FlagNotFound(_))
        ));
    }

    #[test]
    fn test_add_override() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("ao1", FlagStatus2::Disabled, FlagCategory::Core))
            .unwrap();
        mgr.add_override("ao1", "u1", true).unwrap();
        assert_eq!(mgr.overrides.len(), 1);
        assert!(mgr.overrides[0].enabled);
    }

    #[test]
    fn test_add_override_flag_not_found() {
        let mut mgr = FeatureFlagManager::new();
        assert!(matches!(
            mgr.add_override("nope", "u1", true),
            Err(FeatureFlagError::FlagNotFound(_))
        ));
    }

    #[test]
    fn test_remove_override() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("rmo", FlagStatus2::Disabled, FlagCategory::Core))
            .unwrap();
        mgr.add_override("rmo", "u1", true).unwrap();
        mgr.remove_override("rmo", "u1").unwrap();
        assert!(mgr.overrides.is_empty());
    }

    #[test]
    fn test_flags_by_category() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("c1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        mgr.register_flag(make_flag("c2", FlagStatus2::Enabled, FlagCategory::Beta))
            .unwrap();
        mgr.register_flag(make_flag("c3", FlagStatus2::Disabled, FlagCategory::Core))
            .unwrap();
        let core = mgr.flags_by_category(&FlagCategory::Core);
        assert_eq!(core.len(), 2);
    }

    #[test]
    fn test_enabled_flags() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("ef1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        mgr.register_flag(make_flag("ef2", FlagStatus2::Disabled, FlagCategory::Core))
            .unwrap();
        mgr.register_flag(make_flag("ef3", FlagStatus2::Enabled, FlagCategory::Beta))
            .unwrap();
        assert_eq!(mgr.enabled_flags().len(), 2);
    }

    #[test]
    fn test_killed_flags() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("kf1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        mgr.register_flag(make_flag("kf2", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        mgr.kill_switch("kf1").unwrap();
        let killed = mgr.killed_flags();
        assert_eq!(killed.len(), 1);
        assert_eq!(killed[0].id, "kf1");
    }

    #[test]
    fn test_overrides_for_flag() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("of1", FlagStatus2::Disabled, FlagCategory::Core))
            .unwrap();
        mgr.add_override("of1", "u1", true).unwrap();
        mgr.add_override("of1", "u2", false).unwrap();
        let ovrs = mgr.overrides_for_flag("of1");
        assert_eq!(ovrs.len(), 2);
    }

    #[test]
    fn test_recent_evaluations() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("re1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        for i in 0..5 {
            mgr.is_enabled("re1", &format!("u{i}")).unwrap();
        }
        let recent = mgr.recent_evaluations(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_stats() {
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("s1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        mgr.register_flag(make_flag("s2", FlagStatus2::Disabled, FlagCategory::Beta))
            .unwrap();
        mgr.register_flag(make_flag(
            "s3",
            FlagStatus2::Rollout(30),
            FlagCategory::Experimental,
        ))
        .unwrap();
        mgr.add_override("s1", "u1", false).unwrap();
        mgr.kill_switch("s2").unwrap();
        mgr.is_enabled("s1", "u1").unwrap();

        let st = mgr.stats();
        assert_eq!(st.total_flags, 3);
        assert_eq!(st.enabled, 1);
        assert_eq!(st.disabled, 1); // s2 killed -> disabled
        assert_eq!(st.rollout, 1);
        assert_eq!(st.overrides, 1);
        assert_eq!(st.evaluations, 1);
        assert_eq!(st.kill_switches, 1);
    }

    #[test]
    fn test_save_and_load() {
        let path = test_path("save_load");
        let mut mgr = FeatureFlagManager::new();
        mgr.register_flag(make_flag("sl1", FlagStatus2::Enabled, FlagCategory::Core))
            .unwrap();
        mgr.add_override("sl1", "u1", false).unwrap();
        mgr.save(&path).unwrap();

        let loaded = FeatureFlagManager::load(&path).unwrap();
        assert_eq!(loaded.flags.len(), 1);
        assert_eq!(loaded.overrides.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent_load");
        let _ = std::fs::remove_file(&path); // ensure absent
        let mgr = FeatureFlagManager::load_or_default(&path);
        assert!(mgr.flags.is_empty());
    }

    #[test]
    fn test_enable_flag_not_found() {
        let mut mgr = FeatureFlagManager::new();
        assert!(matches!(
            mgr.enable_flag("nope"),
            Err(FeatureFlagError::FlagNotFound(_))
        ));
    }
}
