use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CircuitBreakerError {
    #[error("circuit not found: {0}")]
    CircuitNotFound(String),
    #[error("duplicate circuit: {0}")]
    DuplicateCircuit(String),
    #[error("circuit open: {0}")]
    CircuitOpen(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum CircuitState {
    #[default]
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum BackoffStrategy {
    Fixed,
    Linear,
    #[default]
    Exponential,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitConfig {
    pub failure_threshold: u32,
    pub success_threshold: u32,
    pub timeout_seconds: u64,
    pub backoff: BackoffStrategy,
    pub max_retries: u32,
}

impl Default for CircuitConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            timeout_seconds: 30,
            backoff: BackoffStrategy::Exponential,
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Circuit {
    pub id: String,
    pub service_name: String,
    pub state: CircuitState,
    pub config: CircuitConfig,
    pub failure_count: u32,
    pub success_count: u32,
    pub total_requests: u64,
    pub total_failures: u64,
    pub total_successes: u64,
    pub last_failure: Option<String>,
    pub last_success: Option<String>,
    pub opened_at: Option<String>,
    pub half_open_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitEvent {
    pub circuit_id: String,
    pub event_type: String,
    pub from_state: CircuitState,
    pub to_state: CircuitState,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakerStats {
    pub total_circuits: usize,
    pub closed: usize,
    pub open: usize,
    pub half_open: usize,
    pub total_requests: u64,
    pub total_failures: u64,
    pub total_events: usize,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CircuitBreakerManager {
    pub circuits: HashMap<String, Circuit>,
    pub events: Vec<CircuitEvent>,
}

impl CircuitBreakerManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -- registration -------------------------------------------------------

    pub fn register(
        &mut self,
        id: &str,
        service_name: &str,
        config: CircuitConfig,
    ) -> Result<(), CircuitBreakerError> {
        if self.circuits.contains_key(id) {
            return Err(CircuitBreakerError::DuplicateCircuit(id.to_string()));
        }
        let circuit = Circuit {
            id: id.to_string(),
            service_name: service_name.to_string(),
            state: CircuitState::Closed,
            config,
            failure_count: 0,
            success_count: 0,
            total_requests: 0,
            total_failures: 0,
            total_successes: 0,
            last_failure: None,
            last_success: None,
            opened_at: None,
            half_open_at: None,
            created_at: Utc::now().to_rfc3339(),
        };
        self.circuits.insert(id.to_string(), circuit);
        Ok(())
    }

    pub fn unregister(&mut self, id: &str) -> Result<Circuit, CircuitBreakerError> {
        self.circuits
            .remove(id)
            .ok_or_else(|| CircuitBreakerError::CircuitNotFound(id.to_string()))
    }

    // -- recording ----------------------------------------------------------

    pub fn record_success(&mut self, id: &str) -> Result<(), CircuitBreakerError> {
        let circuit = self
            .circuits
            .get_mut(id)
            .ok_or_else(|| CircuitBreakerError::CircuitNotFound(id.to_string()))?;

        let now = Utc::now().to_rfc3339();
        circuit.total_requests += 1;
        circuit.total_successes += 1;
        circuit.last_success = Some(now.clone());

        match circuit.state {
            CircuitState::HalfOpen => {
                circuit.success_count += 1;
                if circuit.success_count >= circuit.config.success_threshold {
                    let from = circuit.state.clone();
                    circuit.state = CircuitState::Closed;
                    circuit.failure_count = 0;
                    circuit.success_count = 0;
                    circuit.opened_at = None;
                    circuit.half_open_at = None;
                    self.events.push(CircuitEvent {
                        circuit_id: id.to_string(),
                        event_type: "state_change".to_string(),
                        from_state: from,
                        to_state: CircuitState::Closed,
                        timestamp: now,
                    });
                }
            }
            CircuitState::Closed => {
                circuit.failure_count = 0;
            }
            CircuitState::Open => {}
        }
        Ok(())
    }

    pub fn record_failure(&mut self, id: &str) -> Result<(), CircuitBreakerError> {
        let circuit = self
            .circuits
            .get_mut(id)
            .ok_or_else(|| CircuitBreakerError::CircuitNotFound(id.to_string()))?;

        let now = Utc::now().to_rfc3339();
        circuit.total_requests += 1;
        circuit.total_failures += 1;
        circuit.last_failure = Some(now.clone());

        match circuit.state {
            CircuitState::Closed => {
                circuit.failure_count += 1;
                if circuit.failure_count >= circuit.config.failure_threshold {
                    let from = circuit.state.clone();
                    circuit.state = CircuitState::Open;
                    circuit.opened_at = Some(now.clone());
                    self.events.push(CircuitEvent {
                        circuit_id: id.to_string(),
                        event_type: "state_change".to_string(),
                        from_state: from,
                        to_state: CircuitState::Open,
                        timestamp: now,
                    });
                }
            }
            CircuitState::HalfOpen => {
                let from = circuit.state.clone();
                circuit.state = CircuitState::Open;
                circuit.opened_at = Some(now.clone());
                circuit.success_count = 0;
                self.events.push(CircuitEvent {
                    circuit_id: id.to_string(),
                    event_type: "state_change".to_string(),
                    from_state: from,
                    to_state: CircuitState::Open,
                    timestamp: now,
                });
            }
            CircuitState::Open => {}
        }
        Ok(())
    }

    // -- execution gate -----------------------------------------------------

    pub fn can_execute(&mut self, id: &str) -> Result<bool, CircuitBreakerError> {
        let circuit = self
            .circuits
            .get_mut(id)
            .ok_or_else(|| CircuitBreakerError::CircuitNotFound(id.to_string()))?;

        match circuit.state {
            CircuitState::Closed => Ok(true),
            CircuitState::HalfOpen => Ok(true),
            CircuitState::Open => {
                if let Some(ref opened_at) = circuit.opened_at {
                    if let Ok(opened) = chrono::DateTime::parse_from_rfc3339(opened_at) {
                        let elapsed = Utc::now().signed_duration_since(opened);
                        if elapsed.num_seconds() >= circuit.config.timeout_seconds as i64 {
                            let from = circuit.state.clone();
                            circuit.state = CircuitState::HalfOpen;
                            let now = Utc::now().to_rfc3339();
                            circuit.half_open_at = Some(now.clone());
                            circuit.success_count = 0;
                            // We need to push the event after releasing the mutable borrow via id copy
                            let cid = circuit.id.clone();
                            self.events.push(CircuitEvent {
                                circuit_id: cid,
                                event_type: "state_change".to_string(),
                                from_state: from,
                                to_state: CircuitState::HalfOpen,
                                timestamp: now,
                            });
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
        }
    }

    // -- force controls -----------------------------------------------------

    pub fn force_open(&mut self, id: &str) -> Result<(), CircuitBreakerError> {
        let circuit = self
            .circuits
            .get_mut(id)
            .ok_or_else(|| CircuitBreakerError::CircuitNotFound(id.to_string()))?;

        let from = circuit.state.clone();
        let now = Utc::now().to_rfc3339();
        circuit.state = CircuitState::Open;
        circuit.opened_at = Some(now.clone());
        let cid = circuit.id.clone();
        self.events.push(CircuitEvent {
            circuit_id: cid,
            event_type: "force_open".to_string(),
            from_state: from,
            to_state: CircuitState::Open,
            timestamp: now,
        });
        Ok(())
    }

    pub fn force_close(&mut self, id: &str) -> Result<(), CircuitBreakerError> {
        let circuit = self
            .circuits
            .get_mut(id)
            .ok_or_else(|| CircuitBreakerError::CircuitNotFound(id.to_string()))?;

        let from = circuit.state.clone();
        let now = Utc::now().to_rfc3339();
        circuit.state = CircuitState::Closed;
        circuit.failure_count = 0;
        circuit.success_count = 0;
        circuit.opened_at = None;
        circuit.half_open_at = None;
        let cid = circuit.id.clone();
        self.events.push(CircuitEvent {
            circuit_id: cid,
            event_type: "force_close".to_string(),
            from_state: from,
            to_state: CircuitState::Closed,
            timestamp: now,
        });
        Ok(())
    }

    // -- queries ------------------------------------------------------------

    pub fn get_circuit(&self, id: &str) -> Option<&Circuit> {
        self.circuits.get(id)
    }

    pub fn circuits_by_state(&self, state: &CircuitState) -> Vec<&Circuit> {
        self.circuits
            .values()
            .filter(|c| c.state == *state)
            .collect()
    }

    pub fn open_circuits(&self) -> Vec<&Circuit> {
        self.circuits_by_state(&CircuitState::Open)
    }

    pub fn recent_events(&self, n: usize) -> Vec<&CircuitEvent> {
        self.events.iter().rev().take(n).collect()
    }

    pub fn events_for_circuit(&self, id: &str) -> Vec<&CircuitEvent> {
        self.events
            .iter()
            .filter(|e| e.circuit_id == id)
            .collect()
    }

    pub fn stats(&self) -> BreakerStats {
        let mut closed = 0usize;
        let mut open = 0usize;
        let mut half_open = 0usize;
        let mut total_requests = 0u64;
        let mut total_failures = 0u64;

        for c in self.circuits.values() {
            match c.state {
                CircuitState::Closed => closed += 1,
                CircuitState::Open => open += 1,
                CircuitState::HalfOpen => half_open += 1,
            }
            total_requests += c.total_requests;
            total_failures += c.total_failures;
        }

        BreakerStats {
            total_circuits: self.circuits.len(),
            closed,
            open,
            half_open,
            total_requests,
            total_failures,
            total_events: self.events.len(),
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), CircuitBreakerError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, CircuitBreakerError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process;

    fn test_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("evaporchain_cb_{}_{}.json", process::id(), name))
    }

    fn make_manager_with(id: &str, threshold: u32) -> CircuitBreakerManager {
        let mut mgr = CircuitBreakerManager::new();
        let cfg = CircuitConfig {
            failure_threshold: threshold,
            ..Default::default()
        };
        mgr.register(id, "svc", cfg).unwrap();
        mgr
    }

    // 1
    #[test]
    fn test_register() {
        let mut mgr = CircuitBreakerManager::new();
        mgr.register("c1", "service-a", CircuitConfig::default()).unwrap();
        assert!(mgr.get_circuit("c1").is_some());
        assert_eq!(mgr.get_circuit("c1").unwrap().state, CircuitState::Closed);
    }

    // 2
    #[test]
    fn test_unregister() {
        let mut mgr = make_manager_with("c1", 5);
        let removed = mgr.unregister("c1").unwrap();
        assert_eq!(removed.id, "c1");
        assert!(mgr.get_circuit("c1").is_none());
    }

    // 3
    #[test]
    fn test_duplicate_register() {
        let mut mgr = make_manager_with("c1", 5);
        let err = mgr.register("c1", "svc2", CircuitConfig::default());
        assert!(matches!(err, Err(CircuitBreakerError::DuplicateCircuit(_))));
    }

    // 4
    #[test]
    fn test_record_success_closed() {
        let mut mgr = make_manager_with("c1", 3);
        // inject some failures first
        mgr.circuits.get_mut("c1").unwrap().failure_count = 2;
        mgr.record_success("c1").unwrap();
        let c = mgr.get_circuit("c1").unwrap();
        assert_eq!(c.failure_count, 0);
        assert_eq!(c.total_successes, 1);
    }

    // 5
    #[test]
    fn test_record_failures_until_open() {
        let mut mgr = make_manager_with("c1", 3);
        for _ in 0..3 {
            mgr.record_failure("c1").unwrap();
        }
        let c = mgr.get_circuit("c1").unwrap();
        assert_eq!(c.state, CircuitState::Open);
        assert_eq!(c.total_failures, 3);
    }

    // 6
    #[test]
    fn test_can_execute_closed() {
        let mut mgr = make_manager_with("c1", 5);
        assert!(mgr.can_execute("c1").unwrap());
    }

    // 7
    #[test]
    fn test_can_execute_open_no_timeout() {
        let mut mgr = make_manager_with("c1", 1);
        mgr.record_failure("c1").unwrap();
        // just opened, timeout not elapsed
        assert!(!mgr.can_execute("c1").unwrap());
    }

    // 8
    #[test]
    fn test_can_execute_open_timeout_elapsed() {
        let mut mgr = make_manager_with("c1", 1);
        let cfg = CircuitConfig {
            failure_threshold: 1,
            timeout_seconds: 0, // zero-second timeout => immediately eligible
            ..Default::default()
        };
        mgr.circuits.get_mut("c1").unwrap().config = cfg;
        mgr.record_failure("c1").unwrap();
        // timeout is 0 seconds, so should transition to HalfOpen
        assert!(mgr.can_execute("c1").unwrap());
        assert_eq!(mgr.get_circuit("c1").unwrap().state, CircuitState::HalfOpen);
    }

    // 9
    #[test]
    fn test_can_execute_half_open() {
        let mut mgr = make_manager_with("c1", 5);
        mgr.circuits.get_mut("c1").unwrap().state = CircuitState::HalfOpen;
        assert!(mgr.can_execute("c1").unwrap());
    }

    // 10
    #[test]
    fn test_half_open_to_closed_after_successes() {
        let mut mgr = make_manager_with("c1", 5);
        let c = mgr.circuits.get_mut("c1").unwrap();
        c.state = CircuitState::HalfOpen;
        c.config.success_threshold = 2;

        mgr.record_success("c1").unwrap();
        assert_eq!(mgr.get_circuit("c1").unwrap().state, CircuitState::HalfOpen);
        mgr.record_success("c1").unwrap();
        assert_eq!(mgr.get_circuit("c1").unwrap().state, CircuitState::Closed);
    }

    // 11
    #[test]
    fn test_half_open_to_open_on_failure() {
        let mut mgr = make_manager_with("c1", 5);
        mgr.circuits.get_mut("c1").unwrap().state = CircuitState::HalfOpen;
        mgr.record_failure("c1").unwrap();
        assert_eq!(mgr.get_circuit("c1").unwrap().state, CircuitState::Open);
    }

    // 12
    #[test]
    fn test_force_open() {
        let mut mgr = make_manager_with("c1", 5);
        mgr.force_open("c1").unwrap();
        assert_eq!(mgr.get_circuit("c1").unwrap().state, CircuitState::Open);
    }

    // 13
    #[test]
    fn test_force_close() {
        let mut mgr = make_manager_with("c1", 5);
        mgr.force_open("c1").unwrap();
        mgr.force_close("c1").unwrap();
        let c = mgr.get_circuit("c1").unwrap();
        assert_eq!(c.state, CircuitState::Closed);
        assert_eq!(c.failure_count, 0);
        assert_eq!(c.success_count, 0);
    }

    // 14
    #[test]
    fn test_circuits_by_state() {
        let mut mgr = CircuitBreakerManager::new();
        mgr.register("a", "s1", CircuitConfig::default()).unwrap();
        mgr.register("b", "s2", CircuitConfig::default()).unwrap();
        mgr.force_open("b").unwrap();
        assert_eq!(mgr.circuits_by_state(&CircuitState::Closed).len(), 1);
        assert_eq!(mgr.circuits_by_state(&CircuitState::Open).len(), 1);
    }

    // 15
    #[test]
    fn test_open_circuits() {
        let mut mgr = CircuitBreakerManager::new();
        mgr.register("a", "s1", CircuitConfig::default()).unwrap();
        mgr.register("b", "s2", CircuitConfig::default()).unwrap();
        mgr.force_open("a").unwrap();
        let open = mgr.open_circuits();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, "a");
    }

    // 16
    #[test]
    fn test_events_recorded() {
        let mut mgr = make_manager_with("c1", 2);
        mgr.record_failure("c1").unwrap();
        mgr.record_failure("c1").unwrap(); // triggers Open
        assert!(!mgr.events.is_empty());
        let ev = &mgr.events.last().unwrap();
        assert_eq!(ev.to_state, CircuitState::Open);
    }

    // 17
    #[test]
    fn test_recent_events() {
        let mut mgr = make_manager_with("c1", 1);
        mgr.record_failure("c1").unwrap(); // open
        mgr.force_close("c1").unwrap();
        let recent = mgr.recent_events(1);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].event_type, "force_close");
    }

    // 18
    #[test]
    fn test_events_for_circuit() {
        let mut mgr = CircuitBreakerManager::new();
        mgr.register("a", "s1", CircuitConfig { failure_threshold: 1, ..Default::default() }).unwrap();
        mgr.register("b", "s2", CircuitConfig::default()).unwrap();
        mgr.record_failure("a").unwrap();
        mgr.force_open("b").unwrap();
        let evs = mgr.events_for_circuit("a");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].circuit_id, "a");
    }

    // 19
    #[test]
    fn test_stats() {
        let mut mgr = CircuitBreakerManager::new();
        mgr.register("a", "s1", CircuitConfig { failure_threshold: 1, ..Default::default() }).unwrap();
        mgr.register("b", "s2", CircuitConfig::default()).unwrap();
        mgr.record_failure("a").unwrap(); // opens a
        mgr.record_success("b").unwrap();

        let s = mgr.stats();
        assert_eq!(s.total_circuits, 2);
        assert_eq!(s.open, 1);
        assert_eq!(s.closed, 1);
        assert_eq!(s.total_requests, 2);
        assert_eq!(s.total_failures, 1);
        assert!(s.total_events > 0);
    }

    // 20
    #[test]
    fn test_save_and_load() {
        let path = test_path("save_load");
        let mut mgr = make_manager_with("c1", 5);
        mgr.record_success("c1").unwrap();
        mgr.save(&path).unwrap();

        let loaded = CircuitBreakerManager::load(&path).unwrap();
        assert!(loaded.get_circuit("c1").is_some());
        assert_eq!(loaded.get_circuit("c1").unwrap().total_successes, 1);

        let _ = std::fs::remove_file(&path);
    }

    // 21
    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent");
        let _ = std::fs::remove_file(&path); // ensure gone
        let mgr = CircuitBreakerManager::load_or_default(&path);
        assert!(mgr.circuits.is_empty());
    }

    // 22
    #[test]
    fn test_not_found_errors() {
        let mut mgr = CircuitBreakerManager::new();
        assert!(matches!(mgr.record_success("x"), Err(CircuitBreakerError::CircuitNotFound(_))));
        assert!(matches!(mgr.record_failure("x"), Err(CircuitBreakerError::CircuitNotFound(_))));
        assert!(matches!(mgr.can_execute("x"), Err(CircuitBreakerError::CircuitNotFound(_))));
        assert!(matches!(mgr.unregister("x"), Err(CircuitBreakerError::CircuitNotFound(_))));
        assert!(matches!(mgr.force_open("x"), Err(CircuitBreakerError::CircuitNotFound(_))));
        assert!(matches!(mgr.force_close("x"), Err(CircuitBreakerError::CircuitNotFound(_))));
    }

    // 23 (bonus — default config values)
    #[test]
    fn test_default_config() {
        let cfg = CircuitConfig::default();
        assert_eq!(cfg.failure_threshold, 5);
        assert_eq!(cfg.success_threshold, 3);
        assert_eq!(cfg.timeout_seconds, 30);
        assert_eq!(cfg.backoff, BackoffStrategy::Exponential);
        assert_eq!(cfg.max_retries, 3);
    }
}
