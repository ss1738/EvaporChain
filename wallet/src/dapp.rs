//! dApp connector — session management and permission grants.
//!
//! Manages connections between the wallet and decentralized applications:
//! - Create/revoke sessions with specific permissions
//! - Approve/reject individual requests
//! - Track session history and request logs
//! - Persistent session storage
//!
//! Each session grants a dApp limited, revocable access to wallet operations.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum DappError {
    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("session expired: {0}")]
    SessionExpired(String),

    #[error("session revoked: {0}")]
    SessionRevoked(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("duplicate session for origin: {0}")]
    DuplicateSession(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// Permissions a dApp can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// View account address and balance.
    ViewAccount,
    /// View transaction history.
    ViewHistory,
    /// Request transaction signing (still requires user approval).
    RequestSign,
    /// View objects and NFTs.
    ViewAssets,
    /// View energy/decay state.
    ViewEnergy,
}

impl Permission {
    /// All available permissions.
    pub fn all() -> &'static [Permission] {
        &[
            Self::ViewAccount,
            Self::ViewHistory,
            Self::RequestSign,
            Self::ViewAssets,
            Self::ViewEnergy,
        ]
    }

    /// Human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::ViewAccount => "View your address and balance",
            Self::ViewHistory => "View your transaction history",
            Self::RequestSign => "Request transaction signatures (requires approval)",
            Self::ViewAssets => "View your objects and NFTs",
            Self::ViewEnergy => "View energy and decay state",
        }
    }

    /// Parse from string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "view_account" => Some(Self::ViewAccount),
            "view_history" => Some(Self::ViewHistory),
            "request_sign" => Some(Self::RequestSign),
            "view_assets" => Some(Self::ViewAssets),
            "view_energy" => Some(Self::ViewEnergy),
            _ => None,
        }
    }

    /// Label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ViewAccount => "view_account",
            Self::ViewHistory => "view_history",
            Self::RequestSign => "request_sign",
            Self::ViewAssets => "view_assets",
            Self::ViewEnergy => "view_energy",
        }
    }
}

/// Session status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Expired,
    Revoked,
}

/// A dApp session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DappSession {
    /// Unique session ID.
    pub id: String,
    /// dApp origin (e.g., "https://swap.evaporchain.io").
    pub origin: String,
    /// dApp name.
    pub name: String,
    /// Granted permissions.
    pub permissions: Vec<Permission>,
    /// Session status.
    pub status: SessionStatus,
    /// Account address connected to this session.
    pub account: String,
    /// When session was created.
    pub created_at: String,
    /// When session expires.
    pub expires_at: String,
    /// Number of requests made in this session.
    pub request_count: u64,
    /// Last request timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_request: Option<String>,
}

/// A request from a dApp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DappRequest {
    /// Request ID.
    pub id: String,
    /// Session ID.
    pub session_id: String,
    /// Type of request.
    pub request_type: String,
    /// Request details (JSON).
    pub params: serde_json::Value,
    /// Whether request was approved.
    pub approved: Option<bool>,
    /// When requested.
    pub timestamp: String,
}

// ──────────────────────────── DappConnector ───────────────────────────────

/// Manages dApp sessions and requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DappConnector {
    pub sessions: Vec<DappSession>,
    pub request_log: Vec<DappRequest>,
    #[serde(skip)]
    session_index: HashMap<String, usize>,
}

impl DappConnector {
    /// Create an empty connector.
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            request_log: Vec::new(),
            session_index: HashMap::new(),
        }
    }

    /// Rebuild internal indexes.
    fn rebuild_indexes(&mut self) {
        self.session_index.clear();
        for (i, s) in self.sessions.iter().enumerate() {
            self.session_index.insert(s.id.clone(), i);
        }
    }

    /// Load from file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, DappError> {
        let data = std::fs::read_to_string(path)?;
        let mut conn: DappConnector = serde_json::from_str(&data)?;
        conn.rebuild_indexes();
        Ok(conn)
    }

    /// Save to file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), DappError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    // ── Session Management ──────────────────────────────────────────────

    /// Create a new dApp session.
    pub fn create_session(
        &mut self,
        origin: &str,
        name: &str,
        permissions: Vec<Permission>,
        account: &str,
        duration_hours: u64,
    ) -> Result<&DappSession, DappError> {
        // Check for existing active session from same origin
        if self.sessions.iter().any(|s| s.origin == origin && s.status == SessionStatus::Active) {
            return Err(DappError::DuplicateSession(origin.to_string()));
        }

        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::hours(duration_hours as i64);
        let id = format!("sess_{}", now.timestamp_millis());

        let session = DappSession {
            id: id.clone(),
            origin: origin.to_string(),
            name: name.to_string(),
            permissions,
            status: SessionStatus::Active,
            account: account.to_string(),
            created_at: now.to_rfc3339(),
            expires_at: expires.to_rfc3339(),
            request_count: 0,
            last_request: None,
        };

        let idx = self.sessions.len();
        self.sessions.push(session);
        self.session_index.insert(id, idx);
        Ok(&self.sessions[idx])
    }

    /// Revoke a session.
    pub fn revoke_session(&mut self, session_id: &str) -> Result<(), DappError> {
        let idx = *self
            .session_index
            .get(session_id)
            .ok_or_else(|| DappError::SessionNotFound(session_id.to_string()))?;

        self.sessions[idx].status = SessionStatus::Revoked;
        Ok(())
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: &str) -> Option<&DappSession> {
        self.session_index
            .get(session_id)
            .map(|&i| &self.sessions[i])
    }

    /// List active sessions.
    pub fn active_sessions(&self) -> Vec<&DappSession> {
        self.sessions
            .iter()
            .filter(|s| s.status == SessionStatus::Active)
            .collect()
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> &[DappSession] {
        &self.sessions
    }

    /// Check if a session has a specific permission.
    pub fn has_permission(&self, session_id: &str, permission: Permission) -> Result<bool, DappError> {
        let session = self
            .get_session(session_id)
            .ok_or_else(|| DappError::SessionNotFound(session_id.to_string()))?;

        if session.status == SessionStatus::Revoked {
            return Err(DappError::SessionRevoked(session_id.to_string()));
        }

        // Check expiration
        if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&session.expires_at) {
            if chrono::Utc::now() > expires {
                return Err(DappError::SessionExpired(session_id.to_string()));
            }
        }

        Ok(session.permissions.contains(&permission))
    }

    // ── Request Management ──────────────────────────────────────────────

    /// Log a request from a dApp.
    pub fn log_request(
        &mut self,
        session_id: &str,
        request_type: &str,
        params: serde_json::Value,
        approved: bool,
    ) -> Result<(), DappError> {
        let idx = *self
            .session_index
            .get(session_id)
            .ok_or_else(|| DappError::SessionNotFound(session_id.to_string()))?;

        let now = chrono::Utc::now();
        let req_id = format!("req_{}", now.timestamp_millis());

        self.request_log.push(DappRequest {
            id: req_id,
            session_id: session_id.to_string(),
            request_type: request_type.to_string(),
            params,
            approved: Some(approved),
            timestamp: now.to_rfc3339(),
        });

        self.sessions[idx].request_count += 1;
        self.sessions[idx].last_request = Some(now.to_rfc3339());

        Ok(())
    }

    /// Get request history for a session.
    pub fn session_requests(&self, session_id: &str) -> Vec<&DappRequest> {
        self.request_log
            .iter()
            .filter(|r| r.session_id == session_id)
            .collect()
    }

    /// Get all request history.
    pub fn all_requests(&self) -> &[DappRequest] {
        &self.request_log
    }

    /// Expire old sessions (call periodically).
    pub fn expire_sessions(&mut self) -> usize {
        let now = chrono::Utc::now();
        let mut expired = 0;
        for session in &mut self.sessions {
            if session.status == SessionStatus::Active {
                if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&session.expires_at) {
                    if now > expires {
                        session.status = SessionStatus::Expired;
                        expired += 1;
                    }
                }
            }
        }
        expired
    }

    /// Revoke all sessions for an origin.
    pub fn revoke_origin(&mut self, origin: &str) -> usize {
        let mut revoked = 0;
        for session in &mut self.sessions {
            if session.origin == origin && session.status == SessionStatus::Active {
                session.status = SessionStatus::Revoked;
                revoked += 1;
            }
        }
        revoked
    }

    /// Number of sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Number of active sessions.
    pub fn active_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|s| s.status == SessionStatus::Active)
            .count()
    }
}

impl Default for DappConnector {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path for dApp connector data.
pub fn default_dapp_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("dapp_sessions.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_connector() -> DappConnector {
        let mut conn = DappConnector::new();
        conn.create_session(
            "https://swap.evaporchain.io",
            "EvapSwap",
            vec![Permission::ViewAccount, Permission::RequestSign],
            "0xmyaddr",
            24,
        )
        .unwrap();
        conn
    }

    fn get_session_id(conn: &DappConnector) -> String {
        conn.sessions[0].id.clone()
    }

    #[test]
    fn test_create_session() {
        let conn = make_connector();
        assert_eq!(conn.session_count(), 1);
        assert_eq!(conn.active_count(), 1);
        let s = &conn.sessions[0];
        assert_eq!(s.origin, "https://swap.evaporchain.io");
        assert_eq!(s.name, "EvapSwap");
        assert_eq!(s.permissions.len(), 2);
        assert_eq!(s.status, SessionStatus::Active);
    }

    #[test]
    fn test_create_duplicate_session() {
        let mut conn = make_connector();
        let err = conn
            .create_session(
                "https://swap.evaporchain.io",
                "Dup",
                vec![Permission::ViewAccount],
                "0xaddr",
                24,
            )
            .unwrap_err();
        assert!(matches!(err, DappError::DuplicateSession(_)));
    }

    #[test]
    fn test_revoke_session() {
        let mut conn = make_connector();
        let id = get_session_id(&conn);
        conn.revoke_session(&id).unwrap();
        assert_eq!(conn.active_count(), 0);
        assert_eq!(conn.get_session(&id).unwrap().status, SessionStatus::Revoked);
    }

    #[test]
    fn test_revoke_not_found() {
        let mut conn = DappConnector::new();
        let err = conn.revoke_session("nope").unwrap_err();
        assert!(matches!(err, DappError::SessionNotFound(_)));
    }

    #[test]
    fn test_has_permission() {
        let conn = make_connector();
        let id = get_session_id(&conn);
        assert!(conn.has_permission(&id, Permission::ViewAccount).unwrap());
        assert!(conn.has_permission(&id, Permission::RequestSign).unwrap());
        assert!(!conn.has_permission(&id, Permission::ViewHistory).unwrap());
    }

    #[test]
    fn test_has_permission_revoked() {
        let mut conn = make_connector();
        let id = get_session_id(&conn);
        conn.revoke_session(&id).unwrap();
        let err = conn.has_permission(&id, Permission::ViewAccount).unwrap_err();
        assert!(matches!(err, DappError::SessionRevoked(_)));
    }

    #[test]
    fn test_log_request() {
        let mut conn = make_connector();
        let id = get_session_id(&conn);
        conn.log_request(&id, "get_balance", serde_json::json!({}), true)
            .unwrap();
        assert_eq!(conn.all_requests().len(), 1);
        assert_eq!(conn.session_requests(&id).len(), 1);
        assert_eq!(conn.get_session(&id).unwrap().request_count, 1);
    }

    #[test]
    fn test_log_request_session_not_found() {
        let mut conn = DappConnector::new();
        let err = conn
            .log_request("nope", "test", serde_json::json!({}), true)
            .unwrap_err();
        assert!(matches!(err, DappError::SessionNotFound(_)));
    }

    #[test]
    fn test_active_sessions() {
        let mut conn = make_connector();
        // Add another session
        conn.create_session(
            "https://lend.evaporchain.io",
            "EvapLend",
            vec![Permission::ViewAccount],
            "0xmyaddr",
            24,
        )
        .unwrap();
        assert_eq!(conn.active_sessions().len(), 2);

        // Revoke one
        let id = get_session_id(&conn);
        conn.revoke_session(&id).unwrap();
        assert_eq!(conn.active_sessions().len(), 1);
    }

    #[test]
    fn test_revoke_origin() {
        let mut conn = make_connector();
        let count = conn.revoke_origin("https://swap.evaporchain.io");
        assert_eq!(count, 1);
        assert_eq!(conn.active_count(), 0);
    }

    #[test]
    fn test_revoke_origin_none() {
        let mut conn = make_connector();
        let count = conn.revoke_origin("https://nonexistent.com");
        assert_eq!(count, 0);
        assert_eq!(conn.active_count(), 1);
    }

    #[test]
    fn test_permission_all() {
        assert_eq!(Permission::all().len(), 5);
    }

    #[test]
    fn test_permission_from_str() {
        assert_eq!(Permission::from_str("view_account"), Some(Permission::ViewAccount));
        assert_eq!(Permission::from_str("request_sign"), Some(Permission::RequestSign));
        assert_eq!(Permission::from_str("invalid"), None);
    }

    #[test]
    fn test_permission_description() {
        assert!(!Permission::ViewAccount.description().is_empty());
        assert!(!Permission::RequestSign.description().is_empty());
    }

    #[test]
    fn test_permission_label() {
        assert_eq!(Permission::ViewAccount.label(), "view_account");
        assert_eq!(Permission::RequestSign.label(), "request_sign");
    }

    #[test]
    fn test_json_roundtrip() {
        let mut conn = make_connector();
        let id = get_session_id(&conn);
        conn.log_request(&id, "test", serde_json::json!({"key": "val"}), true)
            .unwrap();

        let json = serde_json::to_string_pretty(&conn).unwrap();
        let mut loaded: DappConnector = serde_json::from_str(&json).unwrap();
        loaded.rebuild_indexes();

        assert_eq!(loaded.session_count(), 1);
        assert_eq!(loaded.all_requests().len(), 1);
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_dapp_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dapp.json");

        let conn = make_connector();
        conn.save(&path).unwrap();

        let loaded = DappConnector::load(&path).unwrap();
        assert_eq!(loaded.session_count(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_session_serializable() {
        let conn = make_connector();
        let json = serde_json::to_string(&conn.sessions[0]).unwrap();
        assert!(json.contains("\"status\":\"active\""));
        assert!(json.contains("swap.evaporchain.io"));
    }

    #[test]
    fn test_request_serializable() {
        let req = DappRequest {
            id: "req_1".into(),
            session_id: "sess_1".into(),
            request_type: "sign".into(),
            params: serde_json::json!({"to": "0xabc"}),
            approved: Some(true),
            timestamp: "2026-01-01".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"approved\":true"));
    }

    #[test]
    fn test_expire_sessions() {
        let mut conn = DappConnector::new();
        // Create a session with 0 duration (already expired)
        let now = chrono::Utc::now();
        let past = now - chrono::Duration::hours(1);
        let session = DappSession {
            id: "sess_old".into(),
            origin: "https://old.com".into(),
            name: "Old".into(),
            permissions: vec![Permission::ViewAccount],
            status: SessionStatus::Active,
            account: "0x1".into(),
            created_at: past.to_rfc3339(),
            expires_at: past.to_rfc3339(), // already expired
            request_count: 0,
            last_request: None,
        };
        conn.sessions.push(session);
        conn.rebuild_indexes();

        let expired = conn.expire_sessions();
        assert_eq!(expired, 1);
        assert_eq!(conn.sessions[0].status, SessionStatus::Expired);
    }

    #[test]
    fn test_multiple_requests_tracked() {
        let mut conn = make_connector();
        let id = get_session_id(&conn);
        for i in 0..5 {
            conn.log_request(&id, "test", serde_json::json!({"n": i}), true)
                .unwrap();
        }
        assert_eq!(conn.get_session(&id).unwrap().request_count, 5);
        assert_eq!(conn.session_requests(&id).len(), 5);
    }
}
