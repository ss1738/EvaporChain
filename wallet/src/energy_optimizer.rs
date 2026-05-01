//! Energy decay optimization engine — THE killer feature.
//!
//! No other blockchain has thermodynamic state decay. This module tracks
//! on-chain objects, forecasts their energy decay, and generates optimized
//! batch-refresh plans that save users money.
//!
//! Key capabilities:
//! - Per-object decay forecasting with milestone alerts
//! - Urgency classification (Critical / High / Medium / Low / Safe)
//! - Batch refresh optimization with tiered discounts
//! - Automatic plan generation for at-risk objects
//! - Ghost resurrection cost calculation

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum EnergyOptimizerError {
    #[error("object already tracked: {0}")]
    AlreadyTracked(String),
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    #[error("plan not found: {0}")]
    PlanNotFound(String),
    #[error("plan already executed: {0}")]
    PlanAlreadyExecuted(String),
    #[error("no objects provided")]
    EmptyObjectList,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecayUrgency {
    Critical,
    High,
    Medium,
    Low,
    Safe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefreshStrategy {
    Immediate,
    Batched,
    Scheduled,
    CostOptimal,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectState {
    #[default]
    Healthy,
    Decaying,
    Grace,
    Ghost,
    Evaporated,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedObject {
    pub object_id: String,
    pub owner: String,
    pub current_energy: u64,
    pub max_energy: u64,
    pub decay_rate: u64,
    pub last_refresh: String,
    pub epochs_since_refresh: u32,
    pub estimated_grace_epoch: u32,
    pub estimated_evaporation_epoch: u32,
    pub state: ObjectState,
    pub refresh_cost: u64,
    pub priority: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshPlan {
    pub id: String,
    pub objects: Vec<String>,
    pub strategy: RefreshStrategy,
    pub total_cost: u64,
    pub savings_vs_individual: u64,
    pub scheduled_epoch: Option<u32>,
    pub created_at: String,
    pub executed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayForecast {
    pub object_id: String,
    pub current_energy_pct: f64,
    pub epochs_to_grace: u32,
    pub epochs_to_evaporation: u32,
    pub milestones: Vec<(u32, f64)>,
    pub urgency: DecayUrgency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOptimization {
    pub batch_size: usize,
    pub total_individual_cost: u64,
    pub total_batch_cost: u64,
    pub savings: u64,
    pub savings_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyStats {
    pub tracked_objects: usize,
    pub critical_count: usize,
    pub high_count: usize,
    pub healthy_count: usize,
    pub ghost_count: usize,
    pub total_refresh_cost: u64,
    pub avg_energy_pct: f64,
    pub plans_created: usize,
    pub plans_executed: usize,
}

// ---------------------------------------------------------------------------
// Main Store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnergyOptimizer {
    pub objects: HashMap<String, TrackedObject>,
    pub plans: HashMap<String, RefreshPlan>,
    pub refresh_history: Vec<(String, String, u64)>,
}

impl EnergyOptimizer {
    pub fn new() -> Self {
        Self::default()
    }

    // -- tracking -----------------------------------------------------------

    pub fn track_object(&mut self, obj: TrackedObject) -> Result<(), EnergyOptimizerError> {
        if self.objects.contains_key(&obj.object_id) {
            return Err(EnergyOptimizerError::AlreadyTracked(obj.object_id.clone()));
        }
        self.objects.insert(obj.object_id.clone(), obj);
        Ok(())
    }

    pub fn untrack_object(&mut self, id: &str) -> Result<TrackedObject, EnergyOptimizerError> {
        self.objects
            .remove(id)
            .ok_or_else(|| EnergyOptimizerError::ObjectNotFound(id.to_string()))
    }

    pub fn update_energy(&mut self, id: &str, new_energy: u64) -> Result<(), EnergyOptimizerError> {
        let obj = self
            .objects
            .get_mut(id)
            .ok_or_else(|| EnergyOptimizerError::ObjectNotFound(id.to_string()))?;
        obj.current_energy = new_energy;
        obj.state = compute_state(obj.current_energy, obj.max_energy);
        Ok(())
    }

    // -- forecasting --------------------------------------------------------

    pub fn forecast(&self, id: &str) -> Result<DecayForecast, EnergyOptimizerError> {
        let obj = self
            .objects
            .get(id)
            .ok_or_else(|| EnergyOptimizerError::ObjectNotFound(id.to_string()))?;
        Ok(build_forecast(obj))
    }

    pub fn forecast_all(&self) -> Vec<DecayForecast> {
        self.objects.values().map(build_forecast).collect()
    }

    pub fn objects_by_urgency(&self, urgency: &DecayUrgency) -> Vec<&TrackedObject> {
        self.objects
            .values()
            .filter(|obj| {
                let fc = build_forecast(obj);
                fc.urgency == *urgency
            })
            .collect()
    }

    pub fn critical_objects(&self) -> Vec<&TrackedObject> {
        self.objects
            .values()
            .filter(|obj| energy_pct(obj) < 10.0)
            .collect()
    }

    // -- batch optimization -------------------------------------------------

    pub fn batch_optimize(
        &self,
        object_ids: &[String],
    ) -> Result<BatchOptimization, EnergyOptimizerError> {
        if object_ids.is_empty() {
            return Err(EnergyOptimizerError::EmptyObjectList);
        }
        let mut total_individual_cost: u64 = 0;
        for oid in object_ids {
            let obj = self
                .objects
                .get(oid)
                .ok_or_else(|| EnergyOptimizerError::ObjectNotFound(oid.clone()))?;
            total_individual_cost += obj.refresh_cost;
        }

        let discount = batch_discount(object_ids.len());
        let total_batch_cost = (total_individual_cost as f64 * (1.0 - discount)).round() as u64;
        let savings = total_individual_cost.saturating_sub(total_batch_cost);
        let savings_pct = if total_individual_cost > 0 {
            (savings as f64 / total_individual_cost as f64) * 100.0
        } else {
            0.0
        };

        Ok(BatchOptimization {
            batch_size: object_ids.len(),
            total_individual_cost,
            total_batch_cost,
            savings,
            savings_pct,
        })
    }

    // -- plans --------------------------------------------------------------

    pub fn create_plan(
        &mut self,
        object_ids: Vec<String>,
        strategy: RefreshStrategy,
    ) -> Result<String, EnergyOptimizerError> {
        if object_ids.is_empty() {
            return Err(EnergyOptimizerError::EmptyObjectList);
        }
        for oid in &object_ids {
            if !self.objects.contains_key(oid) {
                return Err(EnergyOptimizerError::ObjectNotFound(oid.clone()));
            }
        }

        let opt = self.batch_optimize(&object_ids)?;
        let plan_id = uuid_v4();

        let scheduled_epoch = match strategy {
            RefreshStrategy::Scheduled | RefreshStrategy::CostOptimal => Some(
                self.objects
                    .values()
                    .filter(|o| object_ids.contains(&o.object_id))
                    .map(|o| o.estimated_grace_epoch.saturating_sub(1))
                    .min()
                    .unwrap_or(0),
            ),
            _ => None,
        };

        let plan = RefreshPlan {
            id: plan_id.clone(),
            objects: object_ids,
            strategy,
            total_cost: opt.total_batch_cost,
            savings_vs_individual: opt.savings,
            scheduled_epoch,
            created_at: Utc::now().to_rfc3339(),
            executed: false,
        };

        self.plans.insert(plan_id.clone(), plan);
        Ok(plan_id)
    }

    pub fn execute_plan(&mut self, plan_id: &str) -> Result<(), EnergyOptimizerError> {
        let plan = self
            .plans
            .get(plan_id)
            .ok_or_else(|| EnergyOptimizerError::PlanNotFound(plan_id.to_string()))?;

        if plan.executed {
            return Err(EnergyOptimizerError::PlanAlreadyExecuted(
                plan_id.to_string(),
            ));
        }

        let ts = Utc::now().to_rfc3339();
        let object_ids = plan.objects.clone();
        let cost_per = if object_ids.is_empty() {
            0
        } else {
            plan.total_cost / object_ids.len() as u64
        };

        for oid in &object_ids {
            self.refresh_history
                .push((oid.clone(), ts.clone(), cost_per));
            if let Some(obj) = self.objects.get_mut(oid) {
                obj.current_energy = obj.max_energy;
                obj.epochs_since_refresh = 0;
                obj.last_refresh = ts.clone();
                obj.state = ObjectState::Healthy;
            }
        }

        let plan = self.plans.get_mut(plan_id).unwrap();
        plan.executed = true;
        Ok(())
    }

    pub fn auto_plan(&mut self) -> Option<String> {
        let urgent_ids: Vec<String> = self
            .objects
            .values()
            .filter(|obj| {
                let fc = build_forecast(obj);
                fc.urgency == DecayUrgency::Critical || fc.urgency == DecayUrgency::High
            })
            .map(|obj| obj.object_id.clone())
            .collect();

        if urgent_ids.is_empty() {
            return None;
        }

        self.create_plan(urgent_ids, RefreshStrategy::CostOptimal)
            .ok()
    }

    // -- misc ---------------------------------------------------------------

    pub fn resurrection_cost(&self, id: &str) -> Result<u64, EnergyOptimizerError> {
        let obj = self
            .objects
            .get(id)
            .ok_or_else(|| EnergyOptimizerError::ObjectNotFound(id.to_string()))?;
        Ok(obj.refresh_cost * 2)
    }

    pub fn total_refresh_budget(&self) -> u64 {
        self.objects.values().map(|o| o.refresh_cost).sum()
    }

    pub fn stats(&self) -> EnergyStats {
        let mut critical = 0;
        let mut high = 0;
        let mut healthy = 0;
        let mut ghost = 0;
        let mut total_pct = 0.0;

        for obj in self.objects.values() {
            let pct = energy_pct(obj);
            total_pct += pct;
            let fc = build_forecast(obj);
            match fc.urgency {
                DecayUrgency::Critical => critical += 1,
                DecayUrgency::High => high += 1,
                DecayUrgency::Safe => healthy += 1,
                _ => {}
            }
            if obj.state == ObjectState::Ghost {
                ghost += 1;
            }
        }

        let avg_energy_pct = if self.objects.is_empty() {
            0.0
        } else {
            total_pct / self.objects.len() as f64
        };

        EnergyStats {
            tracked_objects: self.objects.len(),
            critical_count: critical,
            high_count: high,
            healthy_count: healthy,
            ghost_count: ghost,
            total_refresh_cost: self.total_refresh_budget(),
            avg_energy_pct,
            plans_created: self.plans.len(),
            plans_executed: self.plans.values().filter(|p| p.executed).count(),
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, EnergyOptimizerError> {
        let data = std::fs::read_to_string(path)?;
        let optimizer: Self = serde_json::from_str(&data)?;
        Ok(optimizer)
    }

    pub fn save(&self, path: &Path) -> Result<(), EnergyOptimizerError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn energy_pct(obj: &TrackedObject) -> f64 {
    if obj.max_energy == 0 {
        return 0.0;
    }
    (obj.current_energy as f64 / obj.max_energy as f64) * 100.0
}

fn compute_state(current: u64, max: u64) -> ObjectState {
    if max == 0 || current == 0 {
        return ObjectState::Evaporated;
    }
    let pct = (current as f64 / max as f64) * 100.0;
    if pct > 50.0 {
        ObjectState::Healthy
    } else if pct > 10.0 {
        ObjectState::Decaying
    } else if pct > 1.0 {
        ObjectState::Grace
    } else if pct > 0.0 {
        ObjectState::Ghost
    } else {
        ObjectState::Evaporated
    }
}

fn urgency_from_pct(pct: f64) -> DecayUrgency {
    if pct < 10.0 {
        DecayUrgency::Critical
    } else if pct < 25.0 {
        DecayUrgency::High
    } else if pct < 50.0 {
        DecayUrgency::Medium
    } else if pct < 75.0 {
        DecayUrgency::Low
    } else {
        DecayUrgency::Safe
    }
}

fn build_forecast(obj: &TrackedObject) -> DecayForecast {
    let pct = energy_pct(obj);
    let decay_rate = obj.decay_rate;

    let epochs_to_grace = if decay_rate > 0 && pct > 10.0 {
        let energy_to_lose = obj.current_energy.saturating_sub(obj.max_energy / 10);
        (energy_to_lose / decay_rate) as u32
    } else {
        0
    };

    let epochs_to_evaporation = if decay_rate > 0 && obj.current_energy > 0 {
        (obj.current_energy / decay_rate) as u32
    } else {
        0
    };

    // milestones at 75%, 50%, 25%, 10%, 1%
    let milestones: Vec<(u32, f64)> = [75.0, 50.0, 25.0, 10.0, 1.0]
        .iter()
        .filter_map(|&target| {
            if pct <= target || decay_rate == 0 {
                return None;
            }
            let target_energy = (obj.max_energy as f64 * target / 100.0) as u64;
            let energy_to_lose = obj.current_energy.saturating_sub(target_energy);
            let epochs = (energy_to_lose / decay_rate) as u32;
            Some((epochs, target))
        })
        .collect();

    DecayForecast {
        object_id: obj.object_id.clone(),
        current_energy_pct: pct,
        epochs_to_grace,
        epochs_to_evaporation,
        milestones,
        urgency: urgency_from_pct(pct),
    }
}

fn batch_discount(count: usize) -> f64 {
    match count {
        0..=1 => 0.0,
        2..=5 => 0.15,
        6..=10 => 0.25,
        _ => 0.35,
    }
}

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        d.as_nanos() as u32,
        (d.as_nanos() >> 32) as u16,
        ((d.as_nanos() >> 48) as u16 & 0x0fff) | 0x4000,
        ((d.as_nanos() >> 60) as u16 & 0x3fff) | 0x8000,
        (d.as_nanos() >> 64) as u64 ^ std::process::id() as u64,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process::id;

    fn temp_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("energy_opt_test_{}_{}", id(), name))
    }

    fn make_object(oid: &str, current: u64, max: u64, decay: u64, cost: u64) -> TrackedObject {
        TrackedObject {
            object_id: oid.to_string(),
            owner: "alice".to_string(),
            current_energy: current,
            max_energy: max,
            decay_rate: decay,
            last_refresh: Utc::now().to_rfc3339(),
            epochs_since_refresh: 0,
            estimated_grace_epoch: 100,
            estimated_evaporation_epoch: 200,
            state: compute_state(current, max),
            refresh_cost: cost,
            priority: 5,
        }
    }

    #[test]
    fn test_new_is_empty() {
        let opt = EnergyOptimizer::new();
        assert!(opt.objects.is_empty());
        assert!(opt.plans.is_empty());
        assert!(opt.refresh_history.is_empty());
    }

    #[test]
    fn test_track_object() {
        let mut opt = EnergyOptimizer::new();
        let obj = make_object("obj1", 1000, 1000, 10, 50);
        opt.track_object(obj).unwrap();
        assert_eq!(opt.objects.len(), 1);
    }

    #[test]
    fn test_track_duplicate_errors() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("obj1", 1000, 1000, 10, 50))
            .unwrap();
        let res = opt.track_object(make_object("obj1", 500, 1000, 10, 50));
        assert!(res.is_err());
    }

    #[test]
    fn test_untrack_object() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("obj1", 1000, 1000, 10, 50))
            .unwrap();
        let removed = opt.untrack_object("obj1").unwrap();
        assert_eq!(removed.object_id, "obj1");
        assert!(opt.objects.is_empty());
    }

    #[test]
    fn test_untrack_missing_errors() {
        let mut opt = EnergyOptimizer::new();
        assert!(opt.untrack_object("nope").is_err());
    }

    #[test]
    fn test_update_energy() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("obj1", 1000, 1000, 10, 50))
            .unwrap();
        opt.update_energy("obj1", 400).unwrap();
        let obj = opt.objects.get("obj1").unwrap();
        assert_eq!(obj.current_energy, 400);
        assert_eq!(obj.state, ObjectState::Decaying);
    }

    #[test]
    fn test_update_energy_missing_errors() {
        let mut opt = EnergyOptimizer::new();
        assert!(opt.update_energy("nope", 100).is_err());
    }

    #[test]
    fn test_forecast() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("obj1", 800, 1000, 10, 50))
            .unwrap();
        let fc = opt.forecast("obj1").unwrap();
        assert_eq!(fc.object_id, "obj1");
        assert!((fc.current_energy_pct - 80.0).abs() < 0.01);
        assert_eq!(fc.urgency, DecayUrgency::Safe);
        assert!(!fc.milestones.is_empty());
    }

    #[test]
    fn test_forecast_critical() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("obj1", 50, 1000, 10, 50))
            .unwrap();
        let fc = opt.forecast("obj1").unwrap();
        assert_eq!(fc.urgency, DecayUrgency::Critical);
    }

    #[test]
    fn test_forecast_all() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("a", 900, 1000, 10, 50))
            .unwrap();
        opt.track_object(make_object("b", 100, 1000, 10, 50))
            .unwrap();
        let forecasts = opt.forecast_all();
        assert_eq!(forecasts.len(), 2);
    }

    #[test]
    fn test_critical_objects() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("healthy", 900, 1000, 10, 50))
            .unwrap();
        opt.track_object(make_object("critical", 50, 1000, 10, 50))
            .unwrap();
        let crits = opt.critical_objects();
        assert_eq!(crits.len(), 1);
        assert_eq!(crits[0].object_id, "critical");
    }

    #[test]
    fn test_objects_by_urgency() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("safe", 900, 1000, 10, 50))
            .unwrap();
        opt.track_object(make_object("high", 200, 1000, 10, 50))
            .unwrap();
        let safe = opt.objects_by_urgency(&DecayUrgency::Safe);
        assert_eq!(safe.len(), 1);
    }

    #[test]
    fn test_batch_optimize_small() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("a", 500, 1000, 10, 100))
            .unwrap();
        opt.track_object(make_object("b", 500, 1000, 10, 100))
            .unwrap();
        let bo = opt.batch_optimize(&["a".into(), "b".into()]).unwrap();
        assert_eq!(bo.batch_size, 2);
        assert_eq!(bo.total_individual_cost, 200);
        assert_eq!(bo.total_batch_cost, 170); // 15% discount
        assert_eq!(bo.savings, 30);
    }

    #[test]
    fn test_batch_optimize_large() {
        let mut opt = EnergyOptimizer::new();
        for i in 0..12 {
            let oid = format!("obj{}", i);
            opt.track_object(make_object(&oid, 500, 1000, 10, 100))
                .unwrap();
        }
        let ids: Vec<String> = (0..12).map(|i| format!("obj{}", i)).collect();
        let bo = opt.batch_optimize(&ids).unwrap();
        assert_eq!(bo.total_batch_cost, 780); // 35% discount on 1200
    }

    #[test]
    fn test_batch_optimize_empty_errors() {
        let opt = EnergyOptimizer::new();
        assert!(opt.batch_optimize(&[]).is_err());
    }

    #[test]
    fn test_create_plan() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("a", 500, 1000, 10, 100))
            .unwrap();
        opt.track_object(make_object("b", 500, 1000, 10, 100))
            .unwrap();
        let plan_id = opt
            .create_plan(vec!["a".into(), "b".into()], RefreshStrategy::Immediate)
            .unwrap();
        assert!(opt.plans.contains_key(&plan_id));
        let plan = opt.plans.get(&plan_id).unwrap();
        assert!(!plan.executed);
        assert_eq!(plan.objects.len(), 2);
    }

    #[test]
    fn test_execute_plan() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("a", 500, 1000, 10, 100))
            .unwrap();
        let plan_id = opt
            .create_plan(vec!["a".into()], RefreshStrategy::Immediate)
            .unwrap();
        opt.execute_plan(&plan_id).unwrap();
        let plan = opt.plans.get(&plan_id).unwrap();
        assert!(plan.executed);
        let obj = opt.objects.get("a").unwrap();
        assert_eq!(obj.current_energy, obj.max_energy);
        assert!(!opt.refresh_history.is_empty());
    }

    #[test]
    fn test_execute_plan_twice_errors() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("a", 500, 1000, 10, 100))
            .unwrap();
        let plan_id = opt
            .create_plan(vec!["a".into()], RefreshStrategy::Immediate)
            .unwrap();
        opt.execute_plan(&plan_id).unwrap();
        assert!(opt.execute_plan(&plan_id).is_err());
    }

    #[test]
    fn test_auto_plan_creates_for_urgent() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("crit", 50, 1000, 10, 100))
            .unwrap();
        opt.track_object(make_object("safe", 900, 1000, 10, 100))
            .unwrap();
        let plan_id = opt.auto_plan();
        assert!(plan_id.is_some());
        let plan = opt.plans.get(&plan_id.unwrap()).unwrap();
        assert_eq!(plan.objects.len(), 1);
        assert_eq!(plan.strategy, RefreshStrategy::CostOptimal);
    }

    #[test]
    fn test_auto_plan_none_when_all_safe() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("safe", 900, 1000, 10, 100))
            .unwrap();
        assert!(opt.auto_plan().is_none());
    }

    #[test]
    fn test_resurrection_cost() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("ghost", 5, 1000, 10, 100))
            .unwrap();
        let cost = opt.resurrection_cost("ghost").unwrap();
        assert_eq!(cost, 200); // 2x normal refresh cost
    }

    #[test]
    fn test_total_refresh_budget() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("a", 500, 1000, 10, 100))
            .unwrap();
        opt.track_object(make_object("b", 500, 1000, 10, 200))
            .unwrap();
        assert_eq!(opt.total_refresh_budget(), 300);
    }

    #[test]
    fn test_stats() {
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("safe", 900, 1000, 10, 100))
            .unwrap();
        opt.track_object(make_object("crit", 50, 1000, 10, 100))
            .unwrap();
        let plan_id = opt
            .create_plan(vec!["safe".into()], RefreshStrategy::Immediate)
            .unwrap();
        opt.execute_plan(&plan_id).unwrap();
        let st = opt.stats();
        assert_eq!(st.tracked_objects, 2);
        assert_eq!(st.plans_created, 1);
        assert_eq!(st.plans_executed, 1);
        assert_eq!(st.total_refresh_cost, 200);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load.json");
        let mut opt = EnergyOptimizer::new();
        opt.track_object(make_object("obj1", 800, 1000, 10, 50))
            .unwrap();
        opt.save(&path).unwrap();

        let loaded = EnergyOptimizer::load(&path).unwrap();
        assert_eq!(loaded.objects.len(), 1);
        assert!(loaded.objects.contains_key("obj1"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("nonexistent.json");
        let opt = EnergyOptimizer::load_or_default(&path);
        assert!(opt.objects.is_empty());
    }
}
