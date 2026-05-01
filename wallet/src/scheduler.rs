// wallet/src/scheduler.rs — Cron-like recurring task scheduler
//
// Schedule tasks: auto-refresh, auto-stake, balance alerts, backup reminders.
// Each job has a schedule (interval-based), last/next run tracking, and
// enable/disable lifecycle.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("job not found: {0}")]
    NotFound(String),
    #[error("job already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Schedule definition ───────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Schedule {
    /// Run every N seconds
    Seconds(u64),
    /// Run every N minutes
    Minutes(u64),
    /// Run every N hours
    Hours(u64),
    /// Run every N days
    Days(u64),
    /// Run every N weeks
    Weeks(u64),
}

impl Schedule {
    /// Get the interval in seconds
    pub fn interval_secs(&self) -> u64 {
        match self {
            Schedule::Seconds(n) => *n,
            Schedule::Minutes(n) => n * 60,
            Schedule::Hours(n) => n * 3600,
            Schedule::Days(n) => n * 86400,
            Schedule::Weeks(n) => n * 604800,
        }
    }

    /// Parse from string: "30s", "5m", "2h", "1d", "1w"
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self, SchedulerError> {
        let s = s.trim();
        if s.len() < 2 {
            return Err(SchedulerError::InvalidSchedule(format!(
                "too short: '{}'",
                s
            )));
        }
        let (num_str, unit) = s.split_at(s.len() - 1);
        let num: u64 = num_str.parse().map_err(|_| {
            SchedulerError::InvalidSchedule(format!("invalid number: '{}'", num_str))
        })?;
        if num == 0 {
            return Err(SchedulerError::InvalidSchedule(
                "interval must be > 0".into(),
            ));
        }
        match unit {
            "s" => Ok(Schedule::Seconds(num)),
            "m" => Ok(Schedule::Minutes(num)),
            "h" => Ok(Schedule::Hours(num)),
            "d" => Ok(Schedule::Days(num)),
            "w" => Ok(Schedule::Weeks(num)),
            _ => Err(SchedulerError::InvalidSchedule(format!(
                "unknown unit: '{}' (use s/m/h/d/w)",
                unit
            ))),
        }
    }

    pub fn to_human(&self) -> String {
        match self {
            Schedule::Seconds(n) => format!("every {} second(s)", n),
            Schedule::Minutes(n) => format!("every {} minute(s)", n),
            Schedule::Hours(n) => format!("every {} hour(s)", n),
            Schedule::Days(n) => format!("every {} day(s)", n),
            Schedule::Weeks(n) => format!("every {} week(s)", n),
        }
    }
}

// ── Job definition ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobAction {
    /// Refresh a specific object
    RefreshObject { object_id: String, energy: u64 },
    /// Refresh all objects below energy threshold
    RefreshBelow { threshold: u64, energy: u64 },
    /// Check balance and alert if below threshold
    BalanceAlert { min_balance: u64 },
    /// Run energy scan
    EnergyScan,
    /// Create backup
    Backup,
    /// Custom shell command
    Shell { command: String },
    /// Log a message
    Log { message: String },
    /// Stake rewards auto-compound
    AutoCompound { pool_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub description: String,
    pub action: JobAction,
    pub schedule: Schedule,
    pub enabled: bool,
    pub created_at: String,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub run_count: u64,
    pub fail_count: u64,
    pub last_error: Option<String>,
    pub max_failures: u64,
    pub tags: Vec<String>,
}

impl Job {
    pub fn new(id: &str, name: &str, action: JobAction, schedule: Schedule) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            action,
            schedule,
            enabled: true,
            created_at: now.clone(),
            last_run: None,
            next_run: Some(now),
            run_count: 0,
            fail_count: 0,
            last_error: None,
            max_failures: 5,
            tags: Vec::new(),
        }
    }

    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = desc.to_string();
        self
    }

    pub fn with_max_failures(mut self, max: u64) -> Self {
        self.max_failures = max;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Check if this job is due to run based on current time
    pub fn is_due(&self, now_epoch_secs: i64) -> bool {
        if !self.enabled {
            return false;
        }
        if self.fail_count >= self.max_failures {
            return false;
        }
        match &self.next_run {
            Some(next) => {
                if let Ok(next_dt) = chrono::DateTime::parse_from_rfc3339(next) {
                    next_dt.timestamp() <= now_epoch_secs
                } else {
                    true // Can't parse, assume due
                }
            }
            None => true,
        }
    }

    /// Record a successful execution
    pub fn record_success(&mut self) {
        let now = chrono::Utc::now();
        self.last_run = Some(now.to_rfc3339());
        self.run_count += 1;
        self.last_error = None;
        // Compute next run
        let next = now + chrono::Duration::seconds(self.schedule.interval_secs() as i64);
        self.next_run = Some(next.to_rfc3339());
    }

    /// Record a failed execution
    pub fn record_failure(&mut self, error: &str) {
        let now = chrono::Utc::now();
        self.last_run = Some(now.to_rfc3339());
        self.fail_count += 1;
        self.last_error = Some(error.to_string());
        // Still schedule next run
        let next = now + chrono::Duration::seconds(self.schedule.interval_secs() as i64);
        self.next_run = Some(next.to_rfc3339());
    }

    /// Auto-disabled due to too many failures?
    pub fn is_auto_disabled(&self) -> bool {
        self.fail_count >= self.max_failures
    }

    /// Reset failure counter
    pub fn reset_failures(&mut self) {
        self.fail_count = 0;
        self.last_error = None;
    }
}

// ── Scheduler ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Scheduler {
    pub jobs: HashMap<String, Job>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
        }
    }

    /// Add a new job
    pub fn add(&mut self, job: Job) -> Result<(), SchedulerError> {
        if self.jobs.contains_key(&job.id) {
            return Err(SchedulerError::AlreadyExists(job.id.clone()));
        }
        self.jobs.insert(job.id.clone(), job);
        Ok(())
    }

    /// Remove a job
    pub fn remove(&mut self, id: &str) -> Result<Job, SchedulerError> {
        self.jobs
            .remove(id)
            .ok_or_else(|| SchedulerError::NotFound(id.into()))
    }

    /// Get a job
    pub fn get(&self, id: &str) -> Option<&Job> {
        self.jobs.get(id)
    }

    /// Get a mutable job
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.get_mut(id)
    }

    /// Enable a job
    pub fn enable(&mut self, id: &str) -> Result<(), SchedulerError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| SchedulerError::NotFound(id.into()))?;
        job.enabled = true;
        Ok(())
    }

    /// Disable a job
    pub fn disable(&mut self, id: &str) -> Result<(), SchedulerError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| SchedulerError::NotFound(id.into()))?;
        job.enabled = false;
        Ok(())
    }

    /// List all jobs
    pub fn list(&self) -> Vec<&Job> {
        self.jobs.values().collect()
    }

    /// List enabled jobs
    pub fn enabled_jobs(&self) -> Vec<&Job> {
        self.jobs.values().filter(|j| j.enabled).collect()
    }

    /// Get all due jobs at the given epoch timestamp
    pub fn due_jobs(&self, now_epoch_secs: i64) -> Vec<&Job> {
        self.jobs
            .values()
            .filter(|j| j.is_due(now_epoch_secs))
            .collect()
    }

    /// Find jobs by tag
    pub fn by_tag(&self, tag: &str) -> Vec<&Job> {
        self.jobs
            .values()
            .filter(|j| j.tags.iter().any(|t| t == tag))
            .collect()
    }

    /// Find jobs by action type
    pub fn by_action_type(&self, action_type: &str) -> Vec<&Job> {
        self.jobs
            .values()
            .filter(|j| {
                let type_name = match &j.action {
                    JobAction::RefreshObject { .. } => "refresh_object",
                    JobAction::RefreshBelow { .. } => "refresh_below",
                    JobAction::BalanceAlert { .. } => "balance_alert",
                    JobAction::EnergyScan => "energy_scan",
                    JobAction::Backup => "backup",
                    JobAction::Shell { .. } => "shell",
                    JobAction::Log { .. } => "log",
                    JobAction::AutoCompound { .. } => "auto_compound",
                };
                type_name == action_type
            })
            .collect()
    }

    /// Summary statistics
    pub fn stats(&self) -> SchedulerStats {
        let total = self.jobs.len();
        let enabled = self.jobs.values().filter(|j| j.enabled).count();
        let disabled = total - enabled;
        let auto_disabled = self.jobs.values().filter(|j| j.is_auto_disabled()).count();
        let total_runs: u64 = self.jobs.values().map(|j| j.run_count).sum();
        let total_failures: u64 = self.jobs.values().map(|j| j.fail_count).sum();

        SchedulerStats {
            total_jobs: total,
            enabled,
            disabled,
            auto_disabled,
            total_runs,
            total_failures,
        }
    }

    /// JSON persistence
    pub fn save(&self, path: &Path) -> Result<(), SchedulerError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| SchedulerError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| SchedulerError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, SchedulerError> {
        let data = std::fs::read_to_string(path).map_err(|e| SchedulerError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| SchedulerError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerStats {
    pub total_jobs: usize,
    pub enabled: usize,
    pub disabled: usize,
    pub auto_disabled: usize,
    pub total_runs: u64,
    pub total_failures: u64,
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_job(id: &str) -> Job {
        Job::new(
            id,
            &format!("Job {}", id),
            JobAction::Log {
                message: "test".into(),
            },
            Schedule::Minutes(5),
        )
    }

    #[test]
    fn test_schedule_from_str() {
        assert_eq!(Schedule::from_str("30s").unwrap(), Schedule::Seconds(30));
        assert_eq!(Schedule::from_str("5m").unwrap(), Schedule::Minutes(5));
        assert_eq!(Schedule::from_str("2h").unwrap(), Schedule::Hours(2));
        assert_eq!(Schedule::from_str("1d").unwrap(), Schedule::Days(1));
        assert_eq!(Schedule::from_str("1w").unwrap(), Schedule::Weeks(1));
    }

    #[test]
    fn test_schedule_from_str_invalid() {
        assert!(Schedule::from_str("abc").is_err());
        assert!(Schedule::from_str("0s").is_err());
        assert!(Schedule::from_str("5x").is_err());
        assert!(Schedule::from_str("s").is_err());
    }

    #[test]
    fn test_schedule_interval_secs() {
        assert_eq!(Schedule::Seconds(30).interval_secs(), 30);
        assert_eq!(Schedule::Minutes(5).interval_secs(), 300);
        assert_eq!(Schedule::Hours(1).interval_secs(), 3600);
        assert_eq!(Schedule::Days(1).interval_secs(), 86400);
        assert_eq!(Schedule::Weeks(1).interval_secs(), 604800);
    }

    #[test]
    fn test_schedule_to_human() {
        assert_eq!(Schedule::Minutes(5).to_human(), "every 5 minute(s)");
        assert_eq!(Schedule::Hours(1).to_human(), "every 1 hour(s)");
    }

    #[test]
    fn test_job_new() {
        let job = make_job("j1");
        assert_eq!(job.id, "j1");
        assert!(job.enabled);
        assert_eq!(job.run_count, 0);
        assert!(job.last_run.is_none());
    }

    #[test]
    fn test_job_with_description() {
        let job = make_job("j1").with_description("test job");
        assert_eq!(job.description, "test job");
    }

    #[test]
    fn test_job_with_tags() {
        let job = make_job("j1").with_tags(vec!["energy".into(), "auto".into()]);
        assert_eq!(job.tags.len(), 2);
    }

    #[test]
    fn test_job_is_due() {
        let job = make_job("j1");
        let far_future = chrono::Utc::now().timestamp() + 999999;
        assert!(job.is_due(far_future));
    }

    #[test]
    fn test_job_not_due_when_disabled() {
        let mut job = make_job("j1");
        job.enabled = false;
        let far_future = chrono::Utc::now().timestamp() + 999999;
        assert!(!job.is_due(far_future));
    }

    #[test]
    fn test_job_record_success() {
        let mut job = make_job("j1");
        job.record_success();
        assert_eq!(job.run_count, 1);
        assert!(job.last_run.is_some());
        assert!(job.next_run.is_some());
        assert!(job.last_error.is_none());
    }

    #[test]
    fn test_job_record_failure() {
        let mut job = make_job("j1");
        job.record_failure("something broke");
        assert_eq!(job.fail_count, 1);
        assert_eq!(job.last_error, Some("something broke".to_string()));
    }

    #[test]
    fn test_job_auto_disabled() {
        let mut job = make_job("j1").with_max_failures(2);
        assert!(!job.is_auto_disabled());
        job.record_failure("err1");
        job.record_failure("err2");
        assert!(job.is_auto_disabled());
        let far_future = chrono::Utc::now().timestamp() + 999999;
        assert!(!job.is_due(far_future));
    }

    #[test]
    fn test_job_reset_failures() {
        let mut job = make_job("j1");
        job.record_failure("err");
        job.reset_failures();
        assert_eq!(job.fail_count, 0);
        assert!(job.last_error.is_none());
    }

    #[test]
    fn test_scheduler_add_remove() {
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap();
        assert_eq!(sched.list().len(), 1);
        sched.remove("j1").unwrap();
        assert_eq!(sched.list().len(), 0);
    }

    #[test]
    fn test_scheduler_add_duplicate() {
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap();
        assert!(sched.add(make_job("j1")).is_err());
    }

    #[test]
    fn test_scheduler_remove_not_found() {
        let mut sched = Scheduler::new();
        assert!(sched.remove("nope").is_err());
    }

    #[test]
    fn test_scheduler_enable_disable() {
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap();
        sched.disable("j1").unwrap();
        assert!(!sched.get("j1").unwrap().enabled);
        sched.enable("j1").unwrap();
        assert!(sched.get("j1").unwrap().enabled);
    }

    #[test]
    fn test_scheduler_enabled_jobs() {
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap();
        sched.add(make_job("j2")).unwrap();
        sched.disable("j2").unwrap();
        assert_eq!(sched.enabled_jobs().len(), 1);
    }

    #[test]
    fn test_scheduler_due_jobs() {
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap();
        let far_future = chrono::Utc::now().timestamp() + 999999;
        assert_eq!(sched.due_jobs(far_future).len(), 1);
    }

    #[test]
    fn test_scheduler_by_tag() {
        let mut sched = Scheduler::new();
        sched
            .add(make_job("j1").with_tags(vec!["energy".into()]))
            .unwrap();
        sched.add(make_job("j2")).unwrap();
        assert_eq!(sched.by_tag("energy").len(), 1);
        assert_eq!(sched.by_tag("none").len(), 0);
    }

    #[test]
    fn test_scheduler_by_action_type() {
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap(); // Log action
        sched
            .add(Job::new(
                "j2",
                "backup",
                JobAction::Backup,
                Schedule::Days(1),
            ))
            .unwrap();
        assert_eq!(sched.by_action_type("log").len(), 1);
        assert_eq!(sched.by_action_type("backup").len(), 1);
        assert_eq!(sched.by_action_type("shell").len(), 0);
    }

    #[test]
    fn test_scheduler_stats() {
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap();
        sched.add(make_job("j2")).unwrap();
        sched.disable("j2").unwrap();
        let stats = sched.stats();
        assert_eq!(stats.total_jobs, 2);
        assert_eq!(stats.enabled, 1);
        assert_eq!(stats.disabled, 1);
    }

    #[test]
    fn test_scheduler_save_load() {
        let path = std::env::temp_dir().join(format!("evap_scheduler_{}.json", std::process::id()));
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap();
        sched.save(&path).unwrap();
        let loaded = Scheduler::load(&path).unwrap();
        assert_eq!(loaded.list().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_scheduler_load_or_default() {
        let path = std::env::temp_dir().join("evap_sched_noexist.json");
        let sched = Scheduler::load_or_default(&path);
        assert_eq!(sched.list().len(), 0);
    }

    #[test]
    fn test_get_mut() {
        let mut sched = Scheduler::new();
        sched.add(make_job("j1")).unwrap();
        let job = sched.get_mut("j1").unwrap();
        job.description = "updated".to_string();
        assert_eq!(sched.get("j1").unwrap().description, "updated");
    }
}
