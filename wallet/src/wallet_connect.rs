//! WalletConnect-style dApp session management.
//!
//! Manages WalletConnect sessions between the EvaporChain wallet and dApps:
//! - Create, activate, and disconnect sessions with fine-grained permissions
//! - Submit, approve, and reject signing requests
//! - Auto-approve policies for trusted permission types
//! - Persistent JSON storage for session and request data
//!
//! Sessions carry an expiry timestamp and are automatically marked as expired
//! during cleanup sweeps.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum WalletConnectError {
    #[error("session already exists: {0}")]
    SessionExists(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("session inactive: {0}")]
    SessionInactive(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("request not found: {0}")]
    RequestNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────────

/// Status of a WalletConnect session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Pending,
    Active,
    Expired,
    Disconnected,
}

/// Permissions a dApp can request for a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    SignTransaction,
    SignMessage,
    ReadBalance,
    ReadHistory,
    SendTransaction,
    AccessContacts,
    ManageTokens,
}

/// Status of a signing request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

/// Type of signing / transaction request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestType {
    SignTransaction,
    SignMessage,
    SendTransaction,
    PersonalSign,
    TypedData,
}

// ──────────────────────────── DappInfo ──────────────────────────────────

/// Metadata describing a connected dApp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DappInfo {
    pub name: String,
    pub url: String,
    pub icon_url: Option<String>,
    pub description: Option<String>,
    pub chain_id: String,
}

impl DappInfo {
    pub fn new(name: &str, url: &str, chain_id: &str) -> Self {
        Self {
            name: name.to_string(),
            url: url.to_string(),
            icon_url: None,
            description: None,
            chain_id: chain_id.to_string(),
        }
    }

    pub fn with_icon(mut self, url: &str) -> Self {
        self.icon_url = Some(url.to_string());
        self
    }

    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = Some(desc.to_string());
        self
    }
}

// ──────────────────────────── Session ──────────────────────────────────

/// A WalletConnect session between the wallet and a dApp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub dapp: DappInfo,
    pub account: String,
    pub permissions: Vec<Permission>,
    pub status: SessionStatus,
    pub created_at: String,
    pub expires_at: String,
    pub last_activity: Option<String>,
    pub request_count: u64,
    pub metadata: HashMap<String, String>,
}

impl Session {
    pub fn new(
        id: &str,
        dapp: DappInfo,
        account: &str,
        permissions: Vec<Permission>,
        expires_at: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            dapp,
            account: account.to_string(),
            permissions,
            status: SessionStatus::Pending,
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: expires_at.to_string(),
            last_activity: None,
            request_count: 0,
            metadata: HashMap::new(),
        }
    }

    /// Returns `true` if the session is active and has not expired.
    pub fn is_active(&self) -> bool {
        self.status == SessionStatus::Active && !self.is_expired()
    }

    /// Returns `true` if the current time is past `expires_at`.
    pub fn is_expired(&self) -> bool {
        if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&self.expires_at) {
            chrono::Utc::now() >= exp
        } else {
            false
        }
    }

    /// Check whether the session has the given permission.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(perm)
    }

    /// Activate the session.
    pub fn activate(&mut self) {
        self.status = SessionStatus::Active;
    }

    /// Disconnect the session.
    pub fn disconnect(&mut self) {
        self.status = SessionStatus::Disconnected;
    }

    /// Record activity — updates `last_activity` and increments `request_count`.
    pub fn record_activity(&mut self) {
        self.last_activity = Some(chrono::Utc::now().to_rfc3339());
        self.request_count += 1;
    }

    /// Extend the session expiry.
    pub fn extend(&mut self, new_expires_at: &str) {
        self.expires_at = new_expires_at.to_string();
    }

    /// Add a permission if not already present.
    pub fn add_permission(&mut self, perm: Permission) {
        if !self.permissions.contains(&perm) {
            self.permissions.push(perm);
        }
    }

    /// Remove a permission. Returns `true` if it was present.
    pub fn remove_permission(&mut self, perm: &Permission) -> bool {
        let before = self.permissions.len();
        self.permissions.retain(|p| p != perm);
        self.permissions.len() < before
    }

    /// Seconds remaining until expiry (negative if already expired).
    pub fn time_remaining_secs(&self) -> i64 {
        if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&self.expires_at) {
            let diff = exp.signed_duration_since(chrono::Utc::now());
            diff.num_seconds()
        } else {
            0
        }
    }
}

// ──────────────────────────── SigningRequest ─────────────────────────────

/// A request from a dApp to sign data or submit a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningRequest {
    pub id: String,
    pub session_id: String,
    pub request_type: RequestType,
    pub status: RequestStatus,
    pub payload: String,
    pub from_address: String,
    pub created_at: String,
    pub responded_at: Option<String>,
    pub result: Option<String>,
    pub reason: Option<String>,
}

impl SigningRequest {
    pub fn new(
        id: &str,
        session_id: &str,
        request_type: RequestType,
        payload: &str,
        from_address: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            session_id: session_id.to_string(),
            request_type,
            status: RequestStatus::Pending,
            payload: payload.to_string(),
            from_address: from_address.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            responded_at: None,
            result: None,
            reason: None,
        }
    }

    /// Approve the request with a result (signature or tx hash).
    pub fn approve(&mut self, result: &str) {
        self.status = RequestStatus::Approved;
        self.responded_at = Some(chrono::Utc::now().to_rfc3339());
        self.result = Some(result.to_string());
    }

    /// Reject the request with a reason.
    pub fn reject(&mut self, reason: &str) {
        self.status = RequestStatus::Rejected;
        self.responded_at = Some(chrono::Utc::now().to_rfc3339());
        self.reason = Some(reason.to_string());
    }

    /// Returns `true` if the request is still pending.
    pub fn is_pending(&self) -> bool {
        self.status == RequestStatus::Pending
    }
}

// ──────────────────────────── ConnectStats ───────────────────────────────

/// Summary statistics for the WalletConnect manager.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectStats {
    pub total_sessions: usize,
    pub active: usize,
    pub expired: usize,
    pub disconnected: usize,
    pub pending_requests: usize,
    pub total_requests: usize,
}

// ──────────────────────────── WalletConnectManager ──────────────────────

/// Manages WalletConnect sessions and signing requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WalletConnectManager {
    pub sessions: HashMap<String, Session>,
    pub requests: Vec<SigningRequest>,
    pub auto_approve_permissions: Vec<Permission>,
}

impl WalletConnectManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new session. Fails if the session ID already exists.
    /// Automatically activates the session if its status is `Pending`.
    pub fn create_session(&mut self, mut session: Session) -> Result<(), WalletConnectError> {
        if self.sessions.contains_key(&session.id) {
            return Err(WalletConnectError::SessionExists(session.id.clone()));
        }
        if session.status == SessionStatus::Pending {
            session.activate();
        }
        self.sessions.insert(session.id.clone(), session);
        Ok(())
    }

    /// Disconnect a session by ID.
    pub fn disconnect_session(&mut self, id: &str) -> Result<(), WalletConnectError> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| WalletConnectError::SessionNotFound(id.to_string()))?;
        session.disconnect();
        Ok(())
    }

    /// Get a session by ID.
    pub fn get_session(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    /// Get a mutable session by ID.
    pub fn get_session_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// Return all currently active sessions.
    pub fn active_sessions(&self) -> Vec<&Session> {
        self.sessions.values().filter(|s| s.is_active()).collect()
    }

    /// Return all expired sessions.
    pub fn expired_sessions(&self) -> Vec<&Session> {
        self.sessions.values().filter(|s| s.is_expired()).collect()
    }

    /// Return sessions connected to a particular dApp URL.
    pub fn sessions_for_dapp(&self, dapp_url: &str) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| s.dapp.url == dapp_url)
            .collect()
    }

    /// Submit a signing request.
    ///
    /// Validates:
    /// - The session exists and is active.
    /// - The session has the required permission for the request type.
    ///
    /// Records activity on the session and prunes requests if > 500.
    pub fn submit_request(
        &mut self,
        request: SigningRequest,
    ) -> Result<(), WalletConnectError> {
        let session = self
            .sessions
            .get_mut(&request.session_id)
            .ok_or_else(|| WalletConnectError::SessionNotFound(request.session_id.clone()))?;

        if !session.is_active() {
            return Err(WalletConnectError::SessionInactive(
                request.session_id.clone(),
            ));
        }

        let required_perm = required_permission(&request.request_type);
        if !session.has_permission(&required_perm) {
            return Err(WalletConnectError::PermissionDenied(format!(
                "session {} lacks {:?}",
                request.session_id, required_perm
            )));
        }

        session.record_activity();
        self.requests.push(request);

        // Prune oldest requests if over 500.
        if self.requests.len() > 500 {
            let excess = self.requests.len() - 500;
            self.requests.drain(..excess);
        }

        Ok(())
    }

    /// Approve a pending request by ID.
    pub fn approve_request(
        &mut self,
        id: &str,
        result: &str,
    ) -> Result<(), WalletConnectError> {
        let req = self
            .requests
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| WalletConnectError::RequestNotFound(id.to_string()))?;
        req.approve(result);
        Ok(())
    }

    /// Reject a pending request by ID.
    pub fn reject_request(
        &mut self,
        id: &str,
        reason: &str,
    ) -> Result<(), WalletConnectError> {
        let req = self
            .requests
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| WalletConnectError::RequestNotFound(id.to_string()))?;
        req.reject(reason);
        Ok(())
    }

    /// Return all pending requests.
    pub fn pending_requests(&self) -> Vec<&SigningRequest> {
        self.requests.iter().filter(|r| r.is_pending()).collect()
    }

    /// Return all requests for a given session.
    pub fn requests_for_session(&self, session_id: &str) -> Vec<&SigningRequest> {
        self.requests
            .iter()
            .filter(|r| r.session_id == session_id)
            .collect()
    }

    /// Add a permission to the auto-approve list.
    pub fn set_auto_approve(&mut self, perm: Permission) {
        if !self.auto_approve_permissions.contains(&perm) {
            self.auto_approve_permissions.push(perm);
        }
    }

    /// Remove a permission from the auto-approve list. Returns `true` if present.
    pub fn remove_auto_approve(&mut self, perm: &Permission) -> bool {
        let before = self.auto_approve_permissions.len();
        self.auto_approve_permissions.retain(|p| p != perm);
        self.auto_approve_permissions.len() < before
    }

    /// Disconnect all expired sessions. Returns the number of sessions disconnected.
    pub fn cleanup_expired(&mut self) -> usize {
        let mut count = 0;
        for session in self.sessions.values_mut() {
            if session.is_expired() && session.status != SessionStatus::Disconnected {
                session.disconnect();
                count += 1;
            }
        }
        count
    }

    /// Compute summary statistics.
    pub fn stats(&self) -> ConnectStats {
        let mut active = 0;
        let mut expired = 0;
        let mut disconnected = 0;
        for s in self.sessions.values() {
            match s.status {
                SessionStatus::Active if !s.is_expired() => active += 1,
                SessionStatus::Disconnected => disconnected += 1,
                _ if s.is_expired() => expired += 1,
                _ => {}
            }
        }
        ConnectStats {
            total_sessions: self.sessions.len(),
            active,
            expired,
            disconnected,
            pending_requests: self.requests.iter().filter(|r| r.is_pending()).count(),
            total_requests: self.requests.len(),
        }
    }

    // ── Persistence ────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), WalletConnectError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, WalletConnectError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Helpers ────────────────────────────────────

/// Map a request type to the permission required for it.
fn required_permission(rt: &RequestType) -> Permission {
    match rt {
        RequestType::SignTransaction => Permission::SignTransaction,
        RequestType::SignMessage => Permission::SignMessage,
        RequestType::SendTransaction => Permission::SendTransaction,
        RequestType::PersonalSign => Permission::SignMessage,
        RequestType::TypedData => Permission::SignMessage,
    }
}

// ──────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const FUTURE_EXPIRY: &str = "2099-01-01T00:00:00+00:00";
    const PAST_EXPIRY: &str = "2020-01-01T00:00:00+00:00";

    fn test_dapp() -> DappInfo {
        DappInfo::new("TestDapp", "https://testdapp.io", "evapor-1")
    }

    fn test_session(id: &str, expires: &str) -> Session {
        Session::new(
            id,
            test_dapp(),
            "evap1abc123",
            vec![Permission::SignTransaction, Permission::ReadBalance],
            expires,
        )
    }

    fn active_session(id: &str) -> Session {
        test_session(id, FUTURE_EXPIRY)
    }

    fn test_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("wc_test_{}", std::process::id()))
    }

    #[test]
    fn test_create_session() {
        let mut mgr = WalletConnectManager::new();
        let s = active_session("s1");
        mgr.create_session(s).unwrap();
        let stored = mgr.get_session("s1").unwrap();
        assert_eq!(stored.status, SessionStatus::Active);
        assert_eq!(stored.account, "evap1abc123");
    }

    #[test]
    fn test_create_duplicate_rejected() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        let result = mgr.create_session(active_session("s1"));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WalletConnectError::SessionExists(_)
        ));
    }

    #[test]
    fn test_disconnect_session() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        mgr.disconnect_session("s1").unwrap();
        let s = mgr.get_session("s1").unwrap();
        assert_eq!(s.status, SessionStatus::Disconnected);
    }

    #[test]
    fn test_session_is_active() {
        let s = active_session("s1");
        // Session starts as Pending, but after create_session it becomes Active.
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(s).unwrap();
        let s = mgr.get_session("s1").unwrap();
        assert!(s.is_active());
    }

    #[test]
    fn test_session_expired() {
        let s = test_session("s1", PAST_EXPIRY);
        assert!(s.is_expired());
        // Even if status is Active, is_active returns false when expired.
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(s).unwrap();
        let s = mgr.get_session("s1").unwrap();
        assert_eq!(s.status, SessionStatus::Active); // was auto-activated
        assert!(!s.is_active()); // but expired
        assert!(s.is_expired());
    }

    #[test]
    fn test_session_permissions() {
        let s = active_session("s1");
        assert!(s.has_permission(&Permission::SignTransaction));
        assert!(s.has_permission(&Permission::ReadBalance));
        assert!(!s.has_permission(&Permission::SendTransaction));
    }

    #[test]
    fn test_add_remove_permission() {
        let mut s = active_session("s1");
        assert!(!s.has_permission(&Permission::ManageTokens));
        s.add_permission(Permission::ManageTokens);
        assert!(s.has_permission(&Permission::ManageTokens));
        // Adding again is a no-op.
        s.add_permission(Permission::ManageTokens);
        assert_eq!(
            s.permissions.iter().filter(|p| **p == Permission::ManageTokens).count(),
            1
        );
        assert!(s.remove_permission(&Permission::ManageTokens));
        assert!(!s.has_permission(&Permission::ManageTokens));
        // Removing again returns false.
        assert!(!s.remove_permission(&Permission::ManageTokens));
    }

    #[test]
    fn test_submit_request() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        let req = SigningRequest::new(
            "r1",
            "s1",
            RequestType::SignTransaction,
            r#"{"to":"evap1xyz","amount":100}"#,
            "evap1abc123",
        );
        mgr.submit_request(req).unwrap();
        assert_eq!(mgr.requests.len(), 1);
        assert_eq!(mgr.get_session("s1").unwrap().request_count, 1);
    }

    #[test]
    fn test_submit_request_no_permission() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        let req = SigningRequest::new(
            "r1",
            "s1",
            RequestType::SendTransaction,
            "{}",
            "evap1abc123",
        );
        let result = mgr.submit_request(req);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WalletConnectError::PermissionDenied(_)
        ));
    }

    #[test]
    fn test_submit_request_inactive_session() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        mgr.disconnect_session("s1").unwrap();
        let req = SigningRequest::new(
            "r1",
            "s1",
            RequestType::SignTransaction,
            "{}",
            "evap1abc123",
        );
        let result = mgr.submit_request(req);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WalletConnectError::SessionInactive(_)
        ));
    }

    #[test]
    fn test_approve_request() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        let req = SigningRequest::new(
            "r1",
            "s1",
            RequestType::SignTransaction,
            "{}",
            "evap1abc123",
        );
        mgr.submit_request(req).unwrap();
        mgr.approve_request("r1", "0xdeadbeef").unwrap();
        let r = mgr.requests.iter().find(|r| r.id == "r1").unwrap();
        assert_eq!(r.status, RequestStatus::Approved);
        assert_eq!(r.result.as_deref(), Some("0xdeadbeef"));
        assert!(r.responded_at.is_some());
    }

    #[test]
    fn test_reject_request() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        let req = SigningRequest::new(
            "r1",
            "s1",
            RequestType::SignTransaction,
            "{}",
            "evap1abc123",
        );
        mgr.submit_request(req).unwrap();
        mgr.reject_request("r1", "user declined").unwrap();
        let r = mgr.requests.iter().find(|r| r.id == "r1").unwrap();
        assert_eq!(r.status, RequestStatus::Rejected);
        assert_eq!(r.reason.as_deref(), Some("user declined"));
    }

    #[test]
    fn test_pending_requests() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        for i in 0..3 {
            let req = SigningRequest::new(
                &format!("r{}", i),
                "s1",
                RequestType::SignTransaction,
                "{}",
                "evap1abc123",
            );
            mgr.submit_request(req).unwrap();
        }
        mgr.approve_request("r0", "sig").unwrap();
        assert_eq!(mgr.pending_requests().len(), 2);
    }

    #[test]
    fn test_requests_for_session() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        mgr.create_session(active_session("s2")).unwrap();
        mgr.submit_request(SigningRequest::new(
            "r1", "s1", RequestType::SignTransaction, "{}", "a",
        ))
        .unwrap();
        mgr.submit_request(SigningRequest::new(
            "r2", "s2", RequestType::SignTransaction, "{}", "a",
        ))
        .unwrap();
        mgr.submit_request(SigningRequest::new(
            "r3", "s1", RequestType::SignTransaction, "{}", "a",
        ))
        .unwrap();
        assert_eq!(mgr.requests_for_session("s1").len(), 2);
        assert_eq!(mgr.requests_for_session("s2").len(), 1);
    }

    #[test]
    fn test_active_sessions() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        mgr.create_session(active_session("s2")).unwrap();
        mgr.create_session(test_session("s3", PAST_EXPIRY)).unwrap();
        mgr.disconnect_session("s2").unwrap();
        let active = mgr.active_sessions();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "s1");
    }

    #[test]
    fn test_sessions_for_dapp() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();

        let dapp2 = DappInfo::new("Other", "https://other.io", "evapor-1");
        let s2 = Session::new("s2", dapp2, "evap1abc", vec![], FUTURE_EXPIRY);
        mgr.create_session(s2).unwrap();

        let results = mgr.sessions_for_dapp("https://testdapp.io");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "s1");
    }

    #[test]
    fn test_cleanup_expired() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        mgr.create_session(test_session("s2", PAST_EXPIRY)).unwrap();
        mgr.create_session(test_session("s3", PAST_EXPIRY)).unwrap();
        let cleaned = mgr.cleanup_expired();
        assert_eq!(cleaned, 2);
        assert_eq!(
            mgr.get_session("s2").unwrap().status,
            SessionStatus::Disconnected
        );
        assert_eq!(
            mgr.get_session("s3").unwrap().status,
            SessionStatus::Disconnected
        );
        // Running again should return 0 (already disconnected).
        assert_eq!(mgr.cleanup_expired(), 0);
    }

    #[test]
    fn test_auto_approve() {
        let mut mgr = WalletConnectManager::new();
        mgr.set_auto_approve(Permission::ReadBalance);
        mgr.set_auto_approve(Permission::ReadBalance); // duplicate is no-op
        assert_eq!(mgr.auto_approve_permissions.len(), 1);
        assert!(mgr.auto_approve_permissions.contains(&Permission::ReadBalance));
        assert!(mgr.remove_auto_approve(&Permission::ReadBalance));
        assert!(!mgr.remove_auto_approve(&Permission::ReadBalance));
        assert!(mgr.auto_approve_permissions.is_empty());
    }

    #[test]
    fn test_session_record_activity() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        let s = mgr.get_session_mut("s1").unwrap();
        assert!(s.last_activity.is_none());
        assert_eq!(s.request_count, 0);
        s.record_activity();
        assert!(s.last_activity.is_some());
        assert_eq!(s.request_count, 1);
        s.record_activity();
        assert_eq!(s.request_count, 2);
    }

    #[test]
    fn test_stats() {
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        mgr.create_session(active_session("s2")).unwrap();
        mgr.create_session(test_session("s3", PAST_EXPIRY)).unwrap();
        mgr.disconnect_session("s2").unwrap();

        mgr.submit_request(SigningRequest::new(
            "r1", "s1", RequestType::SignTransaction, "{}", "a",
        ))
        .unwrap();
        mgr.submit_request(SigningRequest::new(
            "r2", "s1", RequestType::SignTransaction, "{}", "a",
        ))
        .unwrap();
        mgr.approve_request("r1", "sig").unwrap();

        let s = mgr.stats();
        assert_eq!(s.total_sessions, 3);
        assert_eq!(s.active, 1);
        assert_eq!(s.expired, 1);
        assert_eq!(s.disconnected, 1);
        assert_eq!(s.pending_requests, 1);
        assert_eq!(s.total_requests, 2);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path();
        let mut mgr = WalletConnectManager::new();
        mgr.create_session(active_session("s1")).unwrap();
        mgr.submit_request(SigningRequest::new(
            "r1", "s1", RequestType::SignTransaction, "{}", "evap1abc",
        ))
        .unwrap();
        mgr.set_auto_approve(Permission::ReadBalance);
        mgr.save(&path).unwrap();

        let loaded = WalletConnectManager::load(&path).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.requests.len(), 1);
        assert_eq!(loaded.auto_approve_permissions.len(), 1);
        assert_eq!(loaded.get_session("s1").unwrap().account, "evap1abc123");

        // Also test load_or_default with valid file.
        let loaded2 = WalletConnectManager::load_or_default(&path);
        assert_eq!(loaded2.sessions.len(), 1);

        // Cleanup.
        let _ = std::fs::remove_file(&path);

        // load_or_default on missing file returns default.
        let default = WalletConnectManager::load_or_default(&path);
        assert!(default.sessions.is_empty());
    }
}
