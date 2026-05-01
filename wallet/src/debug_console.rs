use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum DebugConsoleError {
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("breakpoint not found: {0}")]
    BreakpointNotFound(String),
    #[error("command not found: {0}")]
    CommandNotFound(String),
    #[error("session already exists: {0}")]
    SessionAlreadyExists(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
}

impl From<std::io::Error> for DebugConsoleError {
    fn from(e: std::io::Error) -> Self {
        DebugConsoleError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for DebugConsoleError {
    fn from(e: serde_json::Error) -> Self {
        DebugConsoleError::Parse(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SessionStatus {
    #[default]
    Active,
    Paused,
    Ended,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BreakpointType {
    EventMatch,
    BalanceThreshold,
    BlockNumber,
    TxHash,
    GasAbove,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum BreakpointStatus {
    #[default]
    Enabled,
    Disabled,
    Triggered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugSession {
    pub id: String,
    pub name: String,
    pub status: SessionStatus,
    pub created_at: String,
    pub ended_at: Option<String>,
    pub commands_run: u64,
    pub breakpoints_hit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breakpoint {
    pub id: String,
    pub bp_type: BreakpointType,
    pub condition: String,
    pub status: BreakpointStatus,
    pub hit_count: u64,
    pub created_at: String,
    pub last_hit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugLog {
    pub session_id: String,
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
    pub context: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayEntry {
    pub tx_hash: String,
    pub original_timestamp: String,
    pub replayed_at: String,
    pub success: bool,
    pub gas_used: u64,
    pub result_diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConsoleStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub total_breakpoints: usize,
    pub enabled_breakpoints: usize,
    pub total_logs: usize,
    pub total_replays: usize,
    pub commands_executed: u64,
}

// ---------------------------------------------------------------------------
// Main struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugConsole {
    pub sessions: HashMap<String, DebugSession>,
    pub breakpoints: HashMap<String, Breakpoint>,
    pub logs: Vec<DebugLog>,
    pub replays: Vec<ReplayEntry>,
    pub max_logs: usize,
}

impl Default for DebugConsole {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            breakpoints: HashMap::new(),
            logs: Vec::new(),
            replays: Vec::new(),
            max_logs: 10_000,
        }
    }
}

impl DebugConsole {
    pub fn new() -> Self {
        Self::default()
    }

    // -- sessions -----------------------------------------------------------

    pub fn create_session(&mut self, name: &str) -> Result<String, DebugConsoleError> {
        // Check for duplicate name.
        if self.sessions.values().any(|s| s.name == name) {
            return Err(DebugConsoleError::SessionAlreadyExists(name.to_string()));
        }
        let now = Utc::now();
        let id = format!("dbg_{}_{}", now.timestamp_millis(), self.sessions.len());
        let session = DebugSession {
            id: id.clone(),
            name: name.to_string(),
            status: SessionStatus::Active,
            created_at: now.to_rfc3339(),
            ended_at: None,
            commands_run: 0,
            breakpoints_hit: 0,
        };
        self.sessions.insert(id.clone(), session);
        Ok(id)
    }

    pub fn end_session(&mut self, id: &str) -> Result<(), DebugConsoleError> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| DebugConsoleError::SessionNotFound(id.to_string()))?;
        session.status = SessionStatus::Ended;
        session.ended_at = Some(Utc::now().to_rfc3339());
        Ok(())
    }

    pub fn pause_session(&mut self, id: &str) -> Result<(), DebugConsoleError> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| DebugConsoleError::SessionNotFound(id.to_string()))?;
        session.status = SessionStatus::Paused;
        Ok(())
    }

    pub fn resume_session(&mut self, id: &str) -> Result<(), DebugConsoleError> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| DebugConsoleError::SessionNotFound(id.to_string()))?;
        session.status = SessionStatus::Active;
        Ok(())
    }

    // -- breakpoints --------------------------------------------------------

    pub fn add_breakpoint(&mut self, bp: Breakpoint) -> Result<(), DebugConsoleError> {
        if self.breakpoints.contains_key(&bp.id) {
            return Err(DebugConsoleError::SessionAlreadyExists(bp.id.clone()));
        }
        self.breakpoints.insert(bp.id.clone(), bp);
        Ok(())
    }

    pub fn remove_breakpoint(&mut self, id: &str) -> Result<Breakpoint, DebugConsoleError> {
        self.breakpoints
            .remove(id)
            .ok_or_else(|| DebugConsoleError::BreakpointNotFound(id.to_string()))
    }

    pub fn enable_breakpoint(&mut self, id: &str) -> Result<(), DebugConsoleError> {
        let bp = self
            .breakpoints
            .get_mut(id)
            .ok_or_else(|| DebugConsoleError::BreakpointNotFound(id.to_string()))?;
        bp.status = BreakpointStatus::Enabled;
        Ok(())
    }

    pub fn disable_breakpoint(&mut self, id: &str) -> Result<(), DebugConsoleError> {
        let bp = self
            .breakpoints
            .get_mut(id)
            .ok_or_else(|| DebugConsoleError::BreakpointNotFound(id.to_string()))?;
        bp.status = BreakpointStatus::Disabled;
        Ok(())
    }

    pub fn trigger_breakpoint(&mut self, id: &str) -> Result<(), DebugConsoleError> {
        let bp = self
            .breakpoints
            .get_mut(id)
            .ok_or_else(|| DebugConsoleError::BreakpointNotFound(id.to_string()))?;
        bp.hit_count += 1;
        bp.last_hit = Some(Utc::now().to_rfc3339());
        bp.status = BreakpointStatus::Triggered;
        Ok(())
    }

    // -- logs ---------------------------------------------------------------

    pub fn add_log(&mut self, log: DebugLog) {
        self.logs.push(log);
        if self.logs.len() > self.max_logs {
            let excess = self.logs.len() - self.max_logs;
            self.logs.drain(..excess);
        }
    }

    pub fn logs_for_session(&self, session_id: &str) -> Vec<&DebugLog> {
        self.logs
            .iter()
            .filter(|l| l.session_id == session_id)
            .collect()
    }

    pub fn logs_by_level(&self, level: &LogLevel) -> Vec<&DebugLog> {
        self.logs.iter().filter(|l| l.level == *level).collect()
    }

    pub fn recent_logs(&self, n: usize) -> Vec<&DebugLog> {
        let start = self.logs.len().saturating_sub(n);
        self.logs[start..].iter().collect()
    }

    // -- replays ------------------------------------------------------------

    pub fn replay_tx(&mut self, entry: ReplayEntry) {
        self.replays.push(entry);
    }

    pub fn recent_replays(&self, n: usize) -> Vec<&ReplayEntry> {
        let start = self.replays.len().saturating_sub(n);
        self.replays[start..].iter().collect()
    }

    // -- queries ------------------------------------------------------------

    pub fn active_sessions(&self) -> Vec<&DebugSession> {
        self.sessions
            .values()
            .filter(|s| s.status == SessionStatus::Active)
            .collect()
    }

    pub fn enabled_breakpoints(&self) -> Vec<&Breakpoint> {
        self.breakpoints
            .values()
            .filter(|b| b.status == BreakpointStatus::Enabled)
            .collect()
    }

    pub fn triggered_breakpoints(&self) -> Vec<&Breakpoint> {
        self.breakpoints
            .values()
            .filter(|b| b.status == BreakpointStatus::Triggered)
            .collect()
    }

    pub fn stats(&self) -> ConsoleStats {
        let commands_executed: u64 = self.sessions.values().map(|s| s.commands_run).sum();
        ConsoleStats {
            total_sessions: self.sessions.len(),
            active_sessions: self
                .sessions
                .values()
                .filter(|s| s.status == SessionStatus::Active)
                .count(),
            total_breakpoints: self.breakpoints.len(),
            enabled_breakpoints: self
                .breakpoints
                .values()
                .filter(|b| b.status == BreakpointStatus::Enabled)
                .count(),
            total_logs: self.logs.len(),
            total_replays: self.replays.len(),
            commands_executed,
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), DebugConsoleError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, DebugConsoleError> {
        let data = std::fs::read_to_string(path)?;
        let console: Self = serde_json::from_str(&data)?;
        Ok(console)
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

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_debug_console_{}_{}.json",
            std::process::id(),
            name
        ))
    }

    fn make_breakpoint(id: &str) -> Breakpoint {
        Breakpoint {
            id: id.to_string(),
            bp_type: BreakpointType::EventMatch,
            condition: "event == Transfer".to_string(),
            status: BreakpointStatus::Enabled,
            hit_count: 0,
            created_at: Utc::now().to_rfc3339(),
            last_hit: None,
        }
    }

    fn make_log(session_id: &str, level: LogLevel, msg: &str) -> DebugLog {
        DebugLog {
            session_id: session_id.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            level,
            message: msg.to_string(),
            context: HashMap::new(),
        }
    }

    fn make_replay(hash: &str, success: bool) -> ReplayEntry {
        ReplayEntry {
            tx_hash: hash.to_string(),
            original_timestamp: Utc::now().to_rfc3339(),
            replayed_at: Utc::now().to_rfc3339(),
            success,
            gas_used: 21000,
            result_diff: None,
        }
    }

    #[test]
    fn test_create_session() {
        let mut console = DebugConsole::new();
        let id = console.create_session("sess1").unwrap();
        assert!(id.starts_with("dbg_"));
        assert_eq!(console.sessions.len(), 1);
        let sess = &console.sessions[&id];
        assert_eq!(sess.name, "sess1");
        assert_eq!(sess.status, SessionStatus::Active);
    }

    #[test]
    fn test_duplicate_session_name() {
        let mut console = DebugConsole::new();
        console.create_session("dup").unwrap();
        let err = console.create_session("dup").unwrap_err();
        assert!(matches!(err, DebugConsoleError::SessionAlreadyExists(_)));
    }

    #[test]
    fn test_end_session() {
        let mut console = DebugConsole::new();
        let id = console.create_session("end_me").unwrap();
        console.end_session(&id).unwrap();
        assert_eq!(console.sessions[&id].status, SessionStatus::Ended);
        assert!(console.sessions[&id].ended_at.is_some());
    }

    #[test]
    fn test_pause_session() {
        let mut console = DebugConsole::new();
        let id = console.create_session("pause_me").unwrap();
        console.pause_session(&id).unwrap();
        assert_eq!(console.sessions[&id].status, SessionStatus::Paused);
    }

    #[test]
    fn test_resume_session() {
        let mut console = DebugConsole::new();
        let id = console.create_session("resume_me").unwrap();
        console.pause_session(&id).unwrap();
        console.resume_session(&id).unwrap();
        assert_eq!(console.sessions[&id].status, SessionStatus::Active);
    }

    #[test]
    fn test_session_not_found() {
        let mut console = DebugConsole::new();
        let err = console.end_session("nonexistent").unwrap_err();
        assert!(matches!(err, DebugConsoleError::SessionNotFound(_)));
    }

    #[test]
    fn test_add_breakpoint() {
        let mut console = DebugConsole::new();
        let bp = make_breakpoint("bp1");
        console.add_breakpoint(bp).unwrap();
        assert_eq!(console.breakpoints.len(), 1);
    }

    #[test]
    fn test_duplicate_breakpoint() {
        let mut console = DebugConsole::new();
        console.add_breakpoint(make_breakpoint("bp_dup")).unwrap();
        let err = console
            .add_breakpoint(make_breakpoint("bp_dup"))
            .unwrap_err();
        assert!(matches!(err, DebugConsoleError::SessionAlreadyExists(_)));
    }

    #[test]
    fn test_remove_breakpoint() {
        let mut console = DebugConsole::new();
        console.add_breakpoint(make_breakpoint("bp_rm")).unwrap();
        let removed = console.remove_breakpoint("bp_rm").unwrap();
        assert_eq!(removed.id, "bp_rm");
        assert!(console.breakpoints.is_empty());
    }

    #[test]
    fn test_remove_breakpoint_not_found() {
        let mut console = DebugConsole::new();
        let err = console.remove_breakpoint("nope").unwrap_err();
        assert!(matches!(err, DebugConsoleError::BreakpointNotFound(_)));
    }

    #[test]
    fn test_enable_breakpoint() {
        let mut console = DebugConsole::new();
        let mut bp = make_breakpoint("bp_en");
        bp.status = BreakpointStatus::Disabled;
        console.add_breakpoint(bp).unwrap();
        console.enable_breakpoint("bp_en").unwrap();
        assert_eq!(
            console.breakpoints["bp_en"].status,
            BreakpointStatus::Enabled
        );
    }

    #[test]
    fn test_disable_breakpoint() {
        let mut console = DebugConsole::new();
        console.add_breakpoint(make_breakpoint("bp_dis")).unwrap();
        console.disable_breakpoint("bp_dis").unwrap();
        assert_eq!(
            console.breakpoints["bp_dis"].status,
            BreakpointStatus::Disabled
        );
    }

    #[test]
    fn test_trigger_breakpoint() {
        let mut console = DebugConsole::new();
        console.add_breakpoint(make_breakpoint("bp_trig")).unwrap();
        console.trigger_breakpoint("bp_trig").unwrap();
        let bp = &console.breakpoints["bp_trig"];
        assert_eq!(bp.hit_count, 1);
        assert!(bp.last_hit.is_some());
        assert_eq!(bp.status, BreakpointStatus::Triggered);
    }

    #[test]
    fn test_add_log() {
        let mut console = DebugConsole::new();
        console.add_log(make_log("s1", LogLevel::Info, "hello"));
        assert_eq!(console.logs.len(), 1);
    }

    #[test]
    fn test_log_trim() {
        let mut console = DebugConsole::new();
        console.max_logs = 5;
        for i in 0..8 {
            console.add_log(make_log("s1", LogLevel::Debug, &format!("msg{}", i)));
        }
        assert_eq!(console.logs.len(), 5);
        assert_eq!(console.logs[0].message, "msg3");
    }

    #[test]
    fn test_logs_for_session() {
        let mut console = DebugConsole::new();
        console.add_log(make_log("s1", LogLevel::Info, "a"));
        console.add_log(make_log("s2", LogLevel::Info, "b"));
        console.add_log(make_log("s1", LogLevel::Warn, "c"));
        let filtered = console.logs_for_session("s1");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_logs_by_level() {
        let mut console = DebugConsole::new();
        console.add_log(make_log("s1", LogLevel::Error, "e1"));
        console.add_log(make_log("s1", LogLevel::Info, "i1"));
        console.add_log(make_log("s1", LogLevel::Error, "e2"));
        let errors = console.logs_by_level(&LogLevel::Error);
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_recent_logs() {
        let mut console = DebugConsole::new();
        for i in 0..10 {
            console.add_log(make_log("s1", LogLevel::Info, &format!("m{}", i)));
        }
        let recent = console.recent_logs(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].message, "m7");
    }

    #[test]
    fn test_replay_tx() {
        let mut console = DebugConsole::new();
        console.replay_tx(make_replay("0xabc", true));
        console.replay_tx(make_replay("0xdef", false));
        assert_eq!(console.replays.len(), 2);
    }

    #[test]
    fn test_recent_replays() {
        let mut console = DebugConsole::new();
        for i in 0..5 {
            console.replay_tx(make_replay(&format!("0x{}", i), true));
        }
        let recent = console.recent_replays(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].tx_hash, "0x3");
    }

    #[test]
    fn test_active_sessions() {
        let mut console = DebugConsole::new();
        let id1 = console.create_session("a1").unwrap();
        let _id2 = console.create_session("a2").unwrap();
        console.end_session(&id1).unwrap();
        assert_eq!(console.active_sessions().len(), 1);
    }

    #[test]
    fn test_enabled_and_triggered_breakpoints() {
        let mut console = DebugConsole::new();
        console.add_breakpoint(make_breakpoint("b1")).unwrap();
        console.add_breakpoint(make_breakpoint("b2")).unwrap();
        console.disable_breakpoint("b1").unwrap();
        console.trigger_breakpoint("b2").unwrap();
        assert_eq!(console.enabled_breakpoints().len(), 0);
        assert_eq!(console.triggered_breakpoints().len(), 1);
    }

    #[test]
    fn test_stats() {
        let mut console = DebugConsole::new();
        console.create_session("st1").unwrap();
        console.create_session("st2").unwrap();
        console.add_breakpoint(make_breakpoint("sb1")).unwrap();
        console.add_log(make_log("s1", LogLevel::Info, "x"));
        console.replay_tx(make_replay("0x1", true));
        let stats = console.stats();
        assert_eq!(stats.total_sessions, 2);
        assert_eq!(stats.active_sessions, 2);
        assert_eq!(stats.total_breakpoints, 1);
        assert_eq!(stats.enabled_breakpoints, 1);
        assert_eq!(stats.total_logs, 1);
        assert_eq!(stats.total_replays, 1);
    }

    #[test]
    fn test_save_and_load() {
        let path = test_path("roundtrip");
        let mut console = DebugConsole::new();
        console.create_session("persist").unwrap();
        console.add_breakpoint(make_breakpoint("pbp")).unwrap();
        console.add_log(make_log("s1", LogLevel::Info, "saved"));
        console.replay_tx(make_replay("0xsave", true));
        console.save(&path).unwrap();

        let loaded = DebugConsole::load(&path).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.breakpoints.len(), 1);
        assert_eq!(loaded.logs.len(), 1);
        assert_eq!(loaded.replays.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent_load");
        let _ = std::fs::remove_file(&path);
        let console = DebugConsole::load_or_default(&path);
        assert_eq!(console.max_logs, 10_000);
        assert!(console.sessions.is_empty());
    }
}
