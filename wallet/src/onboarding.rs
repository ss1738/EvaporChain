use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum OnboardingError {
    #[error("step not found: {0}")]
    StepNotFound(String),
    #[error("already completed: {0}")]
    AlreadyCompleted(String),
    #[error("prerequisite not met: {0}")]
    PrerequisiteNotMet(String),
    #[error("flow not found: {0}")]
    FlowNotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepStatus2 {
    NotStarted,
    InProgress,
    Completed,
    Skipped,
}

impl Default for StepStatus2 {
    fn default() -> Self {
        Self::NotStarted
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepType {
    CreateWallet,
    BackupMnemonic,
    SetPassword,
    FundWallet,
    SendFirstTx,
    ExploreEnergy,
    SetupSecurity,
    ConnectDapp,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FlowType {
    NewUser,
    ImportUser,
    DeveloperSetup,
    QuickStart,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingStep {
    pub id: String,
    pub step_type: StepType,
    pub title: String,
    pub description: String,
    pub status: StepStatus2,
    pub order: u32,
    pub prerequisite: Option<String>,
    pub completed_at: Option<String>,
    pub tips: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingFlow {
    pub id: String,
    pub flow_type: FlowType,
    pub name: String,
    pub steps: Vec<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingTip {
    pub id: String,
    pub title: String,
    pub content: String,
    pub shown: bool,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingStats {
    pub total_flows: usize,
    pub completed_flows: usize,
    pub total_steps: usize,
    pub completed_steps: usize,
    pub skipped_steps: usize,
    pub progress_pct: f64,
    pub tips_shown: usize,
    pub tips_total: usize,
}

// ---------------------------------------------------------------------------
// OnboardingManager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OnboardingManager {
    pub flows: HashMap<String, OnboardingFlow>,
    pub steps: HashMap<String, OnboardingStep>,
    pub tips: Vec<OnboardingTip>,
    pub active_flow: Option<String>,
}

impl OnboardingManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -- flow creation ------------------------------------------------------

    pub fn create_flow(&mut self, flow_type: FlowType) -> String {
        let flow_id = format!("flow-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));

        let step_defs: Vec<(StepType, &str, &str, Option<String>)> = match &flow_type {
            FlowType::NewUser => vec![
                (StepType::CreateWallet, "Create Wallet", "Create your new EvaporChain wallet", None),
                (StepType::BackupMnemonic, "Backup Mnemonic", "Securely back up your recovery phrase", Some("create_wallet".into())),
                (StepType::SetPassword, "Set Password", "Set a strong password for your wallet", Some("backup_mnemonic".into())),
                (StepType::FundWallet, "Fund Wallet", "Add funds to your wallet", Some("set_password".into())),
                (StepType::SendFirstTx, "Send First Transaction", "Send your first transaction on EvaporChain", Some("fund_wallet".into())),
                (StepType::ExploreEnergy, "Explore Energy", "Learn about EvaporChain energy mechanics", None),
                (StepType::SetupSecurity, "Setup Security", "Configure advanced security settings", None),
                (StepType::ConnectDapp, "Connect Dapp", "Connect to a decentralized application", None),
            ],
            FlowType::ImportUser => vec![
                (StepType::CreateWallet, "Import Wallet", "Import your existing wallet", None),
                (StepType::SetPassword, "Set Password", "Set a password for your imported wallet", Some("import_wallet".into())),
                (StepType::FundWallet, "Fund Wallet", "Add funds to your wallet", Some("set_password".into())),
                (StepType::SetupSecurity, "Setup Security", "Configure security settings", None),
                (StepType::ConnectDapp, "Connect Dapp", "Connect to a dapp", None),
            ],
            FlowType::DeveloperSetup => vec![
                (StepType::CreateWallet, "Create Dev Wallet", "Create a development wallet", None),
                (StepType::SetPassword, "Set Password", "Set a dev password", Some("create_dev_wallet".into())),
                (StepType::FundWallet, "Get Testnet Funds", "Get testnet tokens", Some("set_password".into())),
                (StepType::SendFirstTx, "Deploy Contract", "Deploy your first smart contract", Some("get_testnet_funds".into())),
                (StepType::ExploreEnergy, "Explore Energy", "Understand energy costs", None),
                (StepType::ConnectDapp, "Connect Dapp", "Connect your dapp", None),
            ],
            FlowType::QuickStart => vec![
                (StepType::CreateWallet, "Create Wallet", "Quick wallet creation", None),
                (StepType::SetPassword, "Set Password", "Set a password", Some("create_wallet".into())),
                (StepType::FundWallet, "Fund Wallet", "Fund your wallet", Some("set_password".into())),
            ],
        };

        let flow_name = match &flow_type {
            FlowType::NewUser => "New User Onboarding",
            FlowType::ImportUser => "Import User Onboarding",
            FlowType::DeveloperSetup => "Developer Setup",
            FlowType::QuickStart => "Quick Start",
        };

        let mut step_ids = Vec::new();
        for (order, (stype, title, desc, prereq)) in step_defs.into_iter().enumerate() {
            let step_id = format!(
                "{}-step-{}",
                flow_id,
                title.to_lowercase().replace(' ', "_")
            );
            let step = OnboardingStep {
                id: step_id.clone(),
                step_type: stype,
                title: title.to_string(),
                description: desc.to_string(),
                status: StepStatus2::NotStarted,
                order: order as u32,
                prerequisite: prereq.map(|p| format!("{}-step-{}", flow_id, p)),
                completed_at: None,
                tips: Vec::new(),
            };
            self.steps.insert(step_id.clone(), step);
            step_ids.push(step_id);
        }

        let flow = OnboardingFlow {
            id: flow_id.clone(),
            flow_type,
            name: flow_name.to_string(),
            steps: step_ids,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
        };
        self.flows.insert(flow_id.clone(), flow);
        flow_id
    }

    // -- flow control -------------------------------------------------------

    pub fn start_flow(&mut self, flow_id: &str) -> Result<(), OnboardingError> {
        if !self.flows.contains_key(flow_id) {
            return Err(OnboardingError::FlowNotFound(flow_id.to_string()));
        }
        self.active_flow = Some(flow_id.to_string());
        // Mark the first step as InProgress
        if let Some(flow) = self.flows.get(flow_id) {
            if let Some(first_id) = flow.steps.first() {
                if let Some(step) = self.steps.get_mut(first_id) {
                    if step.status == StepStatus2::NotStarted {
                        step.status = StepStatus2::InProgress;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn complete_step(&mut self, step_id: &str) -> Result<(), OnboardingError> {
        // Check step exists
        let step = self
            .steps
            .get(step_id)
            .ok_or_else(|| OnboardingError::StepNotFound(step_id.to_string()))?;

        if step.status == StepStatus2::Completed {
            return Err(OnboardingError::AlreadyCompleted(step_id.to_string()));
        }

        // Check prerequisite
        if let Some(ref prereq_id) = step.prerequisite.clone() {
            let prereq = self
                .steps
                .get(prereq_id)
                .ok_or_else(|| OnboardingError::StepNotFound(prereq_id.clone()))?;
            if prereq.status != StepStatus2::Completed && prereq.status != StepStatus2::Skipped {
                return Err(OnboardingError::PrerequisiteNotMet(prereq_id.clone()));
            }
        }

        let step = self.steps.get_mut(step_id).unwrap();
        step.status = StepStatus2::Completed;
        step.completed_at = Some(Utc::now().to_rfc3339());

        // Check if flow is now complete
        self.check_flow_completion();
        Ok(())
    }

    pub fn skip_step(&mut self, step_id: &str) -> Result<(), OnboardingError> {
        let step = self
            .steps
            .get_mut(step_id)
            .ok_or_else(|| OnboardingError::StepNotFound(step_id.to_string()))?;

        if step.status == StepStatus2::Completed {
            return Err(OnboardingError::AlreadyCompleted(step_id.to_string()));
        }

        step.status = StepStatus2::Skipped;
        self.check_flow_completion();
        Ok(())
    }

    pub fn get_current_step(&self) -> Option<&OnboardingStep> {
        let flow_id = self.active_flow.as_ref()?;
        let flow = self.flows.get(flow_id)?;
        for sid in &flow.steps {
            if let Some(step) = self.steps.get(sid) {
                if step.status != StepStatus2::Completed && step.status != StepStatus2::Skipped {
                    return Some(step);
                }
            }
        }
        None
    }

    pub fn flow_progress(&self, flow_id: &str) -> Result<f64, OnboardingError> {
        let flow = self
            .flows
            .get(flow_id)
            .ok_or_else(|| OnboardingError::FlowNotFound(flow_id.to_string()))?;

        if flow.steps.is_empty() {
            return Ok(100.0);
        }

        let done = flow
            .steps
            .iter()
            .filter(|sid| {
                self.steps
                    .get(*sid)
                    .map(|s| s.status == StepStatus2::Completed || s.status == StepStatus2::Skipped)
                    .unwrap_or(false)
            })
            .count();

        Ok((done as f64 / flow.steps.len() as f64) * 100.0)
    }

    pub fn is_flow_complete(&self, flow_id: &str) -> Result<bool, OnboardingError> {
        let progress = self.flow_progress(flow_id)?;
        Ok((progress - 100.0).abs() < f64::EPSILON)
    }

    pub fn reset_flow(&mut self, flow_id: &str) -> Result<(), OnboardingError> {
        let flow = self
            .flows
            .get_mut(flow_id)
            .ok_or_else(|| OnboardingError::FlowNotFound(flow_id.to_string()))?;

        flow.completed_at = None;
        let step_ids: Vec<String> = flow.steps.clone();

        for sid in &step_ids {
            if let Some(step) = self.steps.get_mut(sid) {
                step.status = StepStatus2::NotStarted;
                step.completed_at = None;
            }
        }
        Ok(())
    }

    // -- tips ---------------------------------------------------------------

    pub fn add_tip(&mut self, tip: OnboardingTip) {
        self.tips.push(tip);
    }

    pub fn show_tip(&mut self, id: &str) -> Result<&OnboardingTip, OnboardingError> {
        let idx = self
            .tips
            .iter()
            .position(|t| t.id.contains(id))
            .ok_or_else(|| OnboardingError::StepNotFound(format!("tip not found: {}", id)))?;
        self.tips[idx].shown = true;
        Ok(&self.tips[idx])
    }

    pub fn unshown_tips(&self) -> Vec<&OnboardingTip> {
        self.tips.iter().filter(|t| !t.shown).collect()
    }

    pub fn tips_by_category(&self, cat: &str) -> Vec<&OnboardingTip> {
        self.tips.iter().filter(|t| t.category == cat).collect()
    }

    // -- query --------------------------------------------------------------

    pub fn active_flow(&self) -> Option<&OnboardingFlow> {
        self.active_flow
            .as_ref()
            .and_then(|id| self.flows.get(id))
    }

    pub fn stats(&self) -> OnboardingStats {
        let completed_flows = self
            .flows
            .values()
            .filter(|f| f.completed_at.is_some())
            .count();

        let completed_steps = self
            .steps
            .values()
            .filter(|s| s.status == StepStatus2::Completed)
            .count();

        let skipped_steps = self
            .steps
            .values()
            .filter(|s| s.status == StepStatus2::Skipped)
            .count();

        let total_steps = self.steps.len();
        let progress_pct = if total_steps == 0 {
            0.0
        } else {
            ((completed_steps + skipped_steps) as f64 / total_steps as f64) * 100.0
        };

        let tips_shown = self.tips.iter().filter(|t| t.shown).count();

        OnboardingStats {
            total_flows: self.flows.len(),
            completed_flows,
            total_steps,
            completed_steps,
            skipped_steps,
            progress_pct,
            tips_shown,
            tips_total: self.tips.len(),
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), OnboardingError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, OnboardingError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    // -- internal -----------------------------------------------------------

    fn check_flow_completion(&mut self) {
        let flow_ids: Vec<String> = self.flows.keys().cloned().collect();
        for fid in flow_ids {
            let all_done = {
                let flow = &self.flows[&fid];
                flow.steps.iter().all(|sid| {
                    self.steps
                        .get(sid)
                        .map(|s| {
                            s.status == StepStatus2::Completed || s.status == StepStatus2::Skipped
                        })
                        .unwrap_or(true)
                })
            };
            if all_done {
                let flow = self.flows.get_mut(&fid).unwrap();
                if flow.completed_at.is_none() {
                    flow.completed_at = Some(Utc::now().to_rfc3339());
                }
            }
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tip(id: &str, cat: &str) -> OnboardingTip {
        OnboardingTip {
            id: id.to_string(),
            title: format!("Tip {}", id),
            content: "Some helpful content".to_string(),
            shown: false,
            category: cat.to_string(),
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_onboarding_test_{}_{}.json",
            std::process::id(),
            name
        ))
    }

    // 1. create_flow new user has 8 steps
    #[test]
    fn test_create_flow_new_user_has_8_steps() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::NewUser);
        let flow = mgr.flows.get(&fid).unwrap();
        assert_eq!(flow.steps.len(), 8);
        assert_eq!(flow.flow_type, FlowType::NewUser);
    }

    // 2. create_flow import user has 5 steps
    #[test]
    fn test_create_flow_import_user_has_5_steps() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::ImportUser);
        assert_eq!(mgr.flows.get(&fid).unwrap().steps.len(), 5);
    }

    // 3. create_flow developer setup has 6 steps
    #[test]
    fn test_create_flow_developer_setup_has_6_steps() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::DeveloperSetup);
        assert_eq!(mgr.flows.get(&fid).unwrap().steps.len(), 6);
    }

    // 4. create_flow quick start has 3 steps
    #[test]
    fn test_create_flow_quick_start_has_3_steps() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::QuickStart);
        assert_eq!(mgr.flows.get(&fid).unwrap().steps.len(), 3);
    }

    // 5. start flow
    #[test]
    fn test_start_flow() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::QuickStart);
        mgr.start_flow(&fid).unwrap();
        assert_eq!(mgr.active_flow, Some(fid.clone()));
        // First step should be InProgress
        let flow = mgr.flows.get(&fid).unwrap();
        let first = mgr.steps.get(&flow.steps[0]).unwrap();
        assert_eq!(first.status, StepStatus2::InProgress);
    }

    // 6. start flow not found
    #[test]
    fn test_start_flow_not_found() {
        let mut mgr = OnboardingManager::new();
        let result = mgr.start_flow("nonexistent");
        assert!(matches!(result, Err(OnboardingError::FlowNotFound(_))));
    }

    // 7. complete step
    #[test]
    fn test_complete_step() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::NewUser);
        mgr.start_flow(&fid).unwrap();
        let flow = mgr.flows.get(&fid).unwrap();
        let step_id = flow.steps[0].clone();
        mgr.complete_step(&step_id).unwrap();
        assert_eq!(mgr.steps.get(&step_id).unwrap().status, StepStatus2::Completed);
        assert!(mgr.steps.get(&step_id).unwrap().completed_at.is_some());
    }

    // 8. complete step with prerequisite met
    #[test]
    fn test_complete_step_prerequisite_met() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::NewUser);
        mgr.start_flow(&fid).unwrap();
        let flow = mgr.flows.get(&fid).unwrap();
        let first = flow.steps[0].clone();
        let second = flow.steps[1].clone();
        mgr.complete_step(&first).unwrap();
        // Second step has prerequisite on first
        mgr.complete_step(&second).unwrap();
        assert_eq!(mgr.steps.get(&second).unwrap().status, StepStatus2::Completed);
    }

    // 9. complete step prerequisite not met
    #[test]
    fn test_complete_step_prerequisite_not_met() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::NewUser);
        mgr.start_flow(&fid).unwrap();
        let flow = mgr.flows.get(&fid).unwrap();
        let second = flow.steps[1].clone();
        // Try to complete second without completing first
        let result = mgr.complete_step(&second);
        assert!(matches!(result, Err(OnboardingError::PrerequisiteNotMet(_))));
    }

    // 10. step not found
    #[test]
    fn test_step_not_found() {
        let mut mgr = OnboardingManager::new();
        let result = mgr.complete_step("nonexistent");
        assert!(matches!(result, Err(OnboardingError::StepNotFound(_))));
    }

    // 11. already completed
    #[test]
    fn test_already_completed() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::NewUser);
        mgr.start_flow(&fid).unwrap();
        let flow = mgr.flows.get(&fid).unwrap();
        let step_id = flow.steps[0].clone();
        mgr.complete_step(&step_id).unwrap();
        let result = mgr.complete_step(&step_id);
        assert!(matches!(result, Err(OnboardingError::AlreadyCompleted(_))));
    }

    // 12. skip step
    #[test]
    fn test_skip_step() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::NewUser);
        let flow = mgr.flows.get(&fid).unwrap();
        let step_id = flow.steps[0].clone();
        mgr.skip_step(&step_id).unwrap();
        assert_eq!(mgr.steps.get(&step_id).unwrap().status, StepStatus2::Skipped);
    }

    // 13. get current step
    #[test]
    fn test_get_current_step() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::QuickStart);
        mgr.start_flow(&fid).unwrap();
        let current = mgr.get_current_step().unwrap();
        assert_eq!(current.order, 0);

        let flow = mgr.flows.get(&fid).unwrap();
        let first = flow.steps[0].clone();
        mgr.complete_step(&first).unwrap();
        let current = mgr.get_current_step().unwrap();
        assert_eq!(current.order, 1);
    }

    // 14. flow progress
    #[test]
    fn test_flow_progress() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::QuickStart);
        mgr.start_flow(&fid).unwrap();
        let progress = mgr.flow_progress(&fid).unwrap();
        assert!((progress - 0.0).abs() < f64::EPSILON);

        let flow = mgr.flows.get(&fid).unwrap();
        let first = flow.steps[0].clone();
        mgr.complete_step(&first).unwrap();
        let progress = mgr.flow_progress(&fid).unwrap();
        // 1/3 = 33.33...
        assert!((progress - 100.0 / 3.0).abs() < 0.01);
    }

    // 15. is_flow_complete
    #[test]
    fn test_is_flow_complete() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::QuickStart);
        mgr.start_flow(&fid).unwrap();
        assert!(!mgr.is_flow_complete(&fid).unwrap());

        let flow = mgr.flows.get(&fid).unwrap();
        let step_ids: Vec<String> = flow.steps.clone();
        for sid in &step_ids {
            mgr.complete_step(sid).unwrap();
        }
        assert!(mgr.is_flow_complete(&fid).unwrap());
    }

    // 16. reset flow
    #[test]
    fn test_reset_flow() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::QuickStart);
        mgr.start_flow(&fid).unwrap();
        let flow = mgr.flows.get(&fid).unwrap();
        let step_ids: Vec<String> = flow.steps.clone();
        for sid in &step_ids {
            mgr.complete_step(sid).unwrap();
        }
        assert!(mgr.is_flow_complete(&fid).unwrap());

        mgr.reset_flow(&fid).unwrap();
        assert!(!mgr.is_flow_complete(&fid).unwrap());
        for sid in &step_ids {
            assert_eq!(mgr.steps.get(sid).unwrap().status, StepStatus2::NotStarted);
        }
    }

    // 17. add and show tip
    #[test]
    fn test_add_and_show_tip() {
        let mut mgr = OnboardingManager::new();
        mgr.add_tip(make_tip("tip1", "security"));
        assert_eq!(mgr.tips.len(), 1);
        assert!(!mgr.tips[0].shown);

        let tip = mgr.show_tip("tip1").unwrap();
        assert!(tip.shown);
        assert_eq!(tip.id, "tip1");
    }

    // 18. unshown tips
    #[test]
    fn test_unshown_tips() {
        let mut mgr = OnboardingManager::new();
        mgr.add_tip(make_tip("t1", "a"));
        mgr.add_tip(make_tip("t2", "a"));
        mgr.add_tip(make_tip("t3", "b"));
        assert_eq!(mgr.unshown_tips().len(), 3);

        mgr.show_tip("t1").unwrap();
        assert_eq!(mgr.unshown_tips().len(), 2);
    }

    // 19. tips by category
    #[test]
    fn test_tips_by_category() {
        let mut mgr = OnboardingManager::new();
        mgr.add_tip(make_tip("t1", "security"));
        mgr.add_tip(make_tip("t2", "energy"));
        mgr.add_tip(make_tip("t3", "security"));
        assert_eq!(mgr.tips_by_category("security").len(), 2);
        assert_eq!(mgr.tips_by_category("energy").len(), 1);
        assert_eq!(mgr.tips_by_category("other").len(), 0);
    }

    // 20. stats
    #[test]
    fn test_stats() {
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::QuickStart);
        mgr.start_flow(&fid).unwrap();
        mgr.add_tip(make_tip("t1", "a"));
        mgr.add_tip(make_tip("t2", "b"));
        mgr.show_tip("t1").unwrap();

        let flow = mgr.flows.get(&fid).unwrap();
        let first = flow.steps[0].clone();
        let second = flow.steps[1].clone();
        mgr.complete_step(&first).unwrap();
        mgr.skip_step(&second).unwrap();

        let stats = mgr.stats();
        assert_eq!(stats.total_flows, 1);
        assert_eq!(stats.total_steps, 3);
        assert_eq!(stats.completed_steps, 1);
        assert_eq!(stats.skipped_steps, 1);
        assert_eq!(stats.tips_shown, 1);
        assert_eq!(stats.tips_total, 2);
        assert!(stats.progress_pct > 60.0);
    }

    // 21. persistence roundtrip
    #[test]
    fn test_persistence_roundtrip() {
        let path = temp_path("roundtrip");
        let mut mgr = OnboardingManager::new();
        let fid = mgr.create_flow(FlowType::NewUser);
        mgr.start_flow(&fid).unwrap();
        mgr.add_tip(make_tip("tip_persist", "security"));

        mgr.save(&path).unwrap();
        let loaded = OnboardingManager::load(&path).unwrap();
        assert_eq!(loaded.flows.len(), 1);
        assert_eq!(loaded.steps.len(), 8);
        assert_eq!(loaded.tips.len(), 1);
        assert_eq!(loaded.active_flow, Some(fid));

        // cleanup
        let _ = std::fs::remove_file(&path);
    }

    // Bonus: load_or_default with missing file
    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("nonexistent_load_or_default");
        let _ = std::fs::remove_file(&path); // ensure doesn't exist
        let mgr = OnboardingManager::load_or_default(&path);
        assert!(mgr.flows.is_empty());
        assert!(mgr.steps.is_empty());
    }
}
