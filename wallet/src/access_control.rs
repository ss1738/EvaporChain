// wallet/src/access_control.rs — Role-based access control for EvaporChain wallet
//
// Manages users, roles, and permissions with:
//   - Predefined roles (Owner, Admin, Operator, Viewer) and custom roles
//   - Per-role spending limits and 2FA requirements
//   - Decision audit log with bounded history
//   - JSON persistence

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ── Error ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum AccessControlError {
    #[error("user already exists: {0}")]
    UserExists(String),
    #[error("user not found: {0}")]
    UserNotFound(String),
    #[error("user inactive: {0}")]
    UserInactive(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ── Enums ────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Role {
    Owner,
    Admin,
    Operator,
    Viewer,
    Custom(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    Transfer,
    Sign,
    ViewBalance,
    ViewHistory,
    ManageKeys,
    ManageContacts,
    ConfigureWallet,
    Stake,
    Govern,
    Export,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessDecision {
    Allow,
    Deny,
    NeedsApproval,
}

// ── RolePermissions ──────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RolePermissions {
    pub role: Role,
    pub allowed_actions: Vec<Action>,
    pub denied_actions: Vec<Action>,
    pub spending_limit: Option<u64>,
    pub requires_2fa: bool,
}

impl RolePermissions {
    pub fn new(role: Role) -> Self {
        Self {
            role,
            allowed_actions: Vec::new(),
            denied_actions: Vec::new(),
            spending_limit: None,
            requires_2fa: false,
        }
    }

    pub fn allow(mut self, action: Action) -> Self {
        if !self.allowed_actions.contains(&action) {
            self.allowed_actions.push(action);
        }
        self
    }

    pub fn deny(mut self, action: Action) -> Self {
        if !self.denied_actions.contains(&action) {
            self.denied_actions.push(action);
        }
        self
    }

    pub fn with_spending_limit(mut self, limit: u64) -> Self {
        self.spending_limit = Some(limit);
        self
    }

    pub fn with_2fa(mut self) -> Self {
        self.requires_2fa = true;
        self
    }

    pub fn can_perform(&self, action: &Action) -> AccessDecision {
        if self.denied_actions.contains(action) {
            return AccessDecision::Deny;
        }
        if self.allowed_actions.contains(action) {
            if self.requires_2fa {
                return AccessDecision::NeedsApproval;
            }
            return AccessDecision::Allow;
        }
        AccessDecision::Deny
    }

    pub fn default_owner() -> Self {
        Self::new(Role::Owner)
            .allow(Action::Transfer)
            .allow(Action::Sign)
            .allow(Action::ViewBalance)
            .allow(Action::ViewHistory)
            .allow(Action::ManageKeys)
            .allow(Action::ManageContacts)
            .allow(Action::ConfigureWallet)
            .allow(Action::Stake)
            .allow(Action::Govern)
            .allow(Action::Export)
    }

    pub fn default_admin() -> Self {
        Self::new(Role::Admin)
            .allow(Action::Transfer)
            .allow(Action::Sign)
            .allow(Action::ViewBalance)
            .allow(Action::ViewHistory)
            .allow(Action::ManageKeys)
            .allow(Action::ManageContacts)
            .allow(Action::ConfigureWallet)
            .allow(Action::Stake)
            .allow(Action::Govern)
    }

    pub fn default_operator() -> Self {
        Self::new(Role::Operator)
            .allow(Action::Transfer)
            .allow(Action::Sign)
            .allow(Action::ViewBalance)
            .allow(Action::ViewHistory)
            .allow(Action::Stake)
    }

    pub fn default_viewer() -> Self {
        Self::new(Role::Viewer)
            .allow(Action::ViewBalance)
            .allow(Action::ViewHistory)
    }
}

// ── WalletUser ───────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WalletUser {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub added_at: String,
    pub last_active: Option<String>,
    pub active: bool,
    pub metadata: HashMap<String, String>,
}

impl WalletUser {
    pub fn new(id: impl Into<String>, name: impl Into<String>, role: Role) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            role,
            added_at: chrono::Utc::now().to_rfc3339(),
            last_active: None,
            active: true,
            metadata: HashMap::new(),
        }
    }

    pub fn record_activity(&mut self) {
        self.last_active = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn deactivate(&mut self) {
        self.active = false;
    }

    pub fn activate(&mut self) {
        self.active = true;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

// ── AccessLog ────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccessLog {
    pub user_id: String,
    pub action: Action,
    pub decision: AccessDecision,
    pub timestamp: String,
    pub details: String,
}

// ── AccessStats ──────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
pub struct AccessStats {
    pub total_users: usize,
    pub active_users: usize,
    pub roles_configured: usize,
    pub total_log_entries: usize,
    pub recent_denials: usize,
}

// ── AccessController ─────────────────────────────────────────

const MAX_LOG_ENTRIES: usize = 1000;

#[derive(Serialize, Deserialize, Debug)]
pub struct AccessController {
    pub users: HashMap<String, WalletUser>,
    pub role_permissions: HashMap<Role, RolePermissions>,
    pub access_log: Vec<AccessLog>,
    pub require_owner_for_config: bool,
}

impl Default for AccessController {
    fn default() -> Self {
        Self::new()
    }
}

impl AccessController {
    pub fn new() -> Self {
        let mut role_permissions = HashMap::new();
        let owner = RolePermissions::default_owner();
        let admin = RolePermissions::default_admin();
        let operator = RolePermissions::default_operator();
        let viewer = RolePermissions::default_viewer();
        role_permissions.insert(Role::Owner, owner);
        role_permissions.insert(Role::Admin, admin);
        role_permissions.insert(Role::Operator, operator);
        role_permissions.insert(Role::Viewer, viewer);

        Self {
            users: HashMap::new(),
            role_permissions,
            access_log: Vec::new(),
            require_owner_for_config: true,
        }
    }

    pub fn add_user(&mut self, user: WalletUser) -> Result<(), AccessControlError> {
        if self.users.contains_key(&user.id) {
            return Err(AccessControlError::UserExists(user.id));
        }
        self.users.insert(user.id.clone(), user);
        Ok(())
    }

    pub fn remove_user(&mut self, id: &str) -> Result<WalletUser, AccessControlError> {
        self.users
            .remove(id)
            .ok_or_else(|| AccessControlError::UserNotFound(id.to_string()))
    }

    pub fn get_user(&self, id: &str) -> Option<&WalletUser> {
        self.users.get(id)
    }

    pub fn get_user_mut(&mut self, id: &str) -> Option<&mut WalletUser> {
        self.users.get_mut(id)
    }

    pub fn list_users(&self) -> Vec<&WalletUser> {
        self.users.values().collect()
    }

    pub fn users_by_role(&self, role: &Role) -> Vec<&WalletUser> {
        self.users
            .values()
            .filter(|u| &u.role == role)
            .collect()
    }

    pub fn set_role_permissions(&mut self, perms: RolePermissions) {
        self.role_permissions.insert(perms.role.clone(), perms);
    }

    pub fn check_access(
        &mut self,
        user_id: &str,
        action: &Action,
    ) -> Result<AccessDecision, AccessControlError> {
        let user = self
            .users
            .get(user_id)
            .ok_or_else(|| AccessControlError::UserNotFound(user_id.to_string()))?;

        if !user.is_active() {
            return Err(AccessControlError::UserInactive(user_id.to_string()));
        }

        let role = user.role.clone();
        let decision = match self.role_permissions.get(&role) {
            Some(perms) => perms.can_perform(action),
            None => AccessDecision::Deny,
        };

        let details = format!(
            "User '{}' ({:?}) attempted {:?} -> {:?}",
            user_id, role, action, decision
        );

        let log_entry = AccessLog {
            user_id: user_id.to_string(),
            action: action.clone(),
            decision,
            timestamp: chrono::Utc::now().to_rfc3339(),
            details,
        };

        self.access_log.push(log_entry);
        if self.access_log.len() > MAX_LOG_ENTRIES {
            self.access_log.remove(0);
        }

        // Record activity on the user
        if let Some(u) = self.users.get_mut(user_id) {
            u.record_activity();
        }

        Ok(decision)
    }

    pub fn check_spending(
        &self,
        user_id: &str,
        amount: u64,
    ) -> Result<AccessDecision, AccessControlError> {
        let user = self
            .users
            .get(user_id)
            .ok_or_else(|| AccessControlError::UserNotFound(user_id.to_string()))?;

        if !user.is_active() {
            return Err(AccessControlError::UserInactive(user_id.to_string()));
        }

        let role = &user.role;
        match self.role_permissions.get(role) {
            Some(perms) => match perms.spending_limit {
                Some(limit) => {
                    if amount <= limit {
                        Ok(AccessDecision::Allow)
                    } else {
                        Ok(AccessDecision::Deny)
                    }
                }
                None => Ok(AccessDecision::Allow),
            },
            None => Ok(AccessDecision::Deny),
        }
    }

    pub fn grant_action(&mut self, role: &Role, action: Action) {
        let perms = self
            .role_permissions
            .entry(role.clone())
            .or_insert_with(|| RolePermissions::new(role.clone()));
        if !perms.allowed_actions.contains(&action) {
            perms.allowed_actions.push(action.clone());
        }
        perms.denied_actions.retain(|a| a != &action);
    }

    pub fn revoke_action(&mut self, role: &Role, action: &Action) {
        if let Some(perms) = self.role_permissions.get_mut(role) {
            perms.allowed_actions.retain(|a| a != action);
            if !perms.denied_actions.contains(action) {
                perms.denied_actions.push(action.clone());
            }
        }
    }

    pub fn access_log_for_user(&self, user_id: &str) -> Vec<&AccessLog> {
        self.access_log
            .iter()
            .filter(|l| l.user_id == user_id)
            .collect()
    }

    pub fn recent_denials(&self) -> Vec<&AccessLog> {
        self.access_log
            .iter()
            .filter(|l| l.decision == AccessDecision::Deny)
            .collect()
    }

    pub fn clear_log(&mut self) -> usize {
        let count = self.access_log.len();
        self.access_log.clear();
        count
    }

    pub fn stats(&self) -> AccessStats {
        let active_users = self.users.values().filter(|u| u.is_active()).count();
        let recent_denials = self
            .access_log
            .iter()
            .filter(|l| l.decision == AccessDecision::Deny)
            .count();
        AccessStats {
            total_users: self.users.len(),
            active_users,
            roles_configured: self.role_permissions.len(),
            total_log_entries: self.access_log.len(),
            recent_denials,
        }
    }

    /// JSON persistence
    pub fn save(&self, path: &Path) -> Result<(), AccessControlError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, AccessControlError> {
        let data = std::fs::read_to_string(path)?;
        let ctrl: Self = serde_json::from_str(&data)?;
        Ok(ctrl)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "access_ctrl_test_{}_{name}",
            std::process::id()
        ))
    }

    fn make_controller_with_user() -> (AccessController, String) {
        let mut ctrl = AccessController::new();
        let user = WalletUser::new("u1", "Alice", Role::Owner);
        ctrl.add_user(user).unwrap();
        (ctrl, "u1".to_string())
    }

    #[test]
    fn test_add_and_get_user() {
        let mut ctrl = AccessController::new();
        let user = WalletUser::new("u1", "Alice", Role::Owner);
        ctrl.add_user(user).unwrap();
        let u = ctrl.get_user("u1").unwrap();
        assert_eq!(u.name, "Alice");
        assert_eq!(u.role, Role::Owner);
        assert!(u.is_active());
    }

    #[test]
    fn test_add_duplicate_rejected() {
        let mut ctrl = AccessController::new();
        let u1 = WalletUser::new("u1", "Alice", Role::Owner);
        let u2 = WalletUser::new("u1", "Bob", Role::Admin);
        ctrl.add_user(u1).unwrap();
        let err = ctrl.add_user(u2).unwrap_err();
        assert!(matches!(err, AccessControlError::UserExists(_)));
    }

    #[test]
    fn test_remove_user() {
        let (mut ctrl, uid) = make_controller_with_user();
        let removed = ctrl.remove_user(&uid).unwrap();
        assert_eq!(removed.id, uid);
        assert!(ctrl.get_user(&uid).is_none());

        let err = ctrl.remove_user("nonexistent").unwrap_err();
        assert!(matches!(err, AccessControlError::UserNotFound(_)));
    }

    #[test]
    fn test_default_owner_permissions() {
        let perms = RolePermissions::default_owner();
        assert_eq!(perms.can_perform(&Action::Transfer), AccessDecision::Allow);
        assert_eq!(perms.can_perform(&Action::Export), AccessDecision::Allow);
        assert_eq!(perms.can_perform(&Action::ConfigureWallet), AccessDecision::Allow);
    }

    #[test]
    fn test_default_admin_permissions() {
        let perms = RolePermissions::default_admin();
        assert_eq!(perms.can_perform(&Action::Transfer), AccessDecision::Allow);
        assert_eq!(perms.can_perform(&Action::Export), AccessDecision::Deny);
    }

    #[test]
    fn test_default_operator_permissions() {
        let perms = RolePermissions::default_operator();
        assert_eq!(perms.can_perform(&Action::Transfer), AccessDecision::Allow);
        assert_eq!(perms.can_perform(&Action::Stake), AccessDecision::Allow);
        assert_eq!(perms.can_perform(&Action::ManageKeys), AccessDecision::Deny);
        assert_eq!(perms.can_perform(&Action::Export), AccessDecision::Deny);
    }

    #[test]
    fn test_default_viewer_permissions() {
        let perms = RolePermissions::default_viewer();
        assert_eq!(perms.can_perform(&Action::ViewBalance), AccessDecision::Allow);
        assert_eq!(perms.can_perform(&Action::ViewHistory), AccessDecision::Allow);
        assert_eq!(perms.can_perform(&Action::Transfer), AccessDecision::Deny);
    }

    #[test]
    fn test_check_access_allowed() {
        let (mut ctrl, uid) = make_controller_with_user();
        let decision = ctrl.check_access(&uid, &Action::Transfer).unwrap();
        assert_eq!(decision, AccessDecision::Allow);
    }

    #[test]
    fn test_check_access_denied() {
        let mut ctrl = AccessController::new();
        let user = WalletUser::new("v1", "Viewer", Role::Viewer);
        ctrl.add_user(user).unwrap();
        let decision = ctrl.check_access("v1", &Action::Transfer).unwrap();
        assert_eq!(decision, AccessDecision::Deny);
    }

    #[test]
    fn test_check_access_inactive_user() {
        let mut ctrl = AccessController::new();
        let mut user = WalletUser::new("u1", "Alice", Role::Owner);
        user.deactivate();
        ctrl.add_user(user).unwrap();
        let err = ctrl.check_access("u1", &Action::Transfer).unwrap_err();
        assert!(matches!(err, AccessControlError::UserInactive(_)));
    }

    #[test]
    fn test_check_access_needs_approval() {
        let mut ctrl = AccessController::new();
        let perms = RolePermissions::default_operator().with_2fa();
        ctrl.set_role_permissions(perms);
        let user = WalletUser::new("op1", "Operator", Role::Operator);
        ctrl.add_user(user).unwrap();
        let decision = ctrl.check_access("op1", &Action::Transfer).unwrap();
        assert_eq!(decision, AccessDecision::NeedsApproval);
    }

    #[test]
    fn test_check_spending_within_limit() {
        let mut ctrl = AccessController::new();
        let perms = RolePermissions::default_operator().with_spending_limit(1000);
        ctrl.set_role_permissions(perms);
        let user = WalletUser::new("op1", "Operator", Role::Operator);
        ctrl.add_user(user).unwrap();
        let decision = ctrl.check_spending("op1", 500).unwrap();
        assert_eq!(decision, AccessDecision::Allow);
    }

    #[test]
    fn test_check_spending_over_limit() {
        let mut ctrl = AccessController::new();
        let perms = RolePermissions::default_operator().with_spending_limit(1000);
        ctrl.set_role_permissions(perms);
        let user = WalletUser::new("op1", "Operator", Role::Operator);
        ctrl.add_user(user).unwrap();
        let decision = ctrl.check_spending("op1", 1500).unwrap();
        assert_eq!(decision, AccessDecision::Deny);
    }

    #[test]
    fn test_grant_action() {
        let mut ctrl = AccessController::new();
        ctrl.grant_action(&Role::Viewer, Action::Transfer);
        let perms = ctrl.role_permissions.get(&Role::Viewer).unwrap();
        assert_eq!(perms.can_perform(&Action::Transfer), AccessDecision::Allow);
    }

    #[test]
    fn test_revoke_action() {
        let mut ctrl = AccessController::new();
        ctrl.revoke_action(&Role::Owner, &Action::Export);
        let perms = ctrl.role_permissions.get(&Role::Owner).unwrap();
        assert_eq!(perms.can_perform(&Action::Export), AccessDecision::Deny);
    }

    #[test]
    fn test_users_by_role() {
        let mut ctrl = AccessController::new();
        ctrl.add_user(WalletUser::new("a1", "Alice", Role::Admin)).unwrap();
        ctrl.add_user(WalletUser::new("a2", "Bob", Role::Admin)).unwrap();
        ctrl.add_user(WalletUser::new("v1", "Carol", Role::Viewer)).unwrap();
        let admins = ctrl.users_by_role(&Role::Admin);
        assert_eq!(admins.len(), 2);
    }

    #[test]
    fn test_access_log() {
        let (mut ctrl, uid) = make_controller_with_user();
        ctrl.check_access(&uid, &Action::Transfer).unwrap();
        ctrl.check_access(&uid, &Action::ViewBalance).unwrap();
        assert_eq!(ctrl.access_log.len(), 2);
    }

    #[test]
    fn test_access_log_for_user() {
        let mut ctrl = AccessController::new();
        ctrl.add_user(WalletUser::new("u1", "Alice", Role::Owner)).unwrap();
        ctrl.add_user(WalletUser::new("u2", "Bob", Role::Viewer)).unwrap();
        ctrl.check_access("u1", &Action::Transfer).unwrap();
        ctrl.check_access("u2", &Action::ViewBalance).unwrap();
        ctrl.check_access("u1", &Action::Export).unwrap();
        let u1_log = ctrl.access_log_for_user("u1");
        assert_eq!(u1_log.len(), 2);
        let u2_log = ctrl.access_log_for_user("u2");
        assert_eq!(u2_log.len(), 1);
    }

    #[test]
    fn test_recent_denials() {
        let mut ctrl = AccessController::new();
        ctrl.add_user(WalletUser::new("v1", "Viewer", Role::Viewer)).unwrap();
        ctrl.check_access("v1", &Action::ViewBalance).unwrap();
        ctrl.check_access("v1", &Action::Transfer).unwrap();
        ctrl.check_access("v1", &Action::Export).unwrap();
        let denials = ctrl.recent_denials();
        assert_eq!(denials.len(), 2);
    }

    #[test]
    fn test_custom_role() {
        let mut ctrl = AccessController::new();
        let custom_perms = RolePermissions::new(Role::Custom("Auditor".into()))
            .allow(Action::ViewBalance)
            .allow(Action::ViewHistory)
            .allow(Action::Export);
        ctrl.set_role_permissions(custom_perms);
        let user = WalletUser::new("aud1", "Dan", Role::Custom("Auditor".into()));
        ctrl.add_user(user).unwrap();
        let d = ctrl.check_access("aud1", &Action::Export).unwrap();
        assert_eq!(d, AccessDecision::Allow);
        let d = ctrl.check_access("aud1", &Action::Transfer).unwrap();
        assert_eq!(d, AccessDecision::Deny);
    }

    #[test]
    fn test_deactivate_activate() {
        let mut ctrl = AccessController::new();
        ctrl.add_user(WalletUser::new("u1", "Alice", Role::Owner)).unwrap();
        ctrl.get_user_mut("u1").unwrap().deactivate();
        assert!(!ctrl.get_user("u1").unwrap().is_active());
        ctrl.get_user_mut("u1").unwrap().activate();
        assert!(ctrl.get_user("u1").unwrap().is_active());
    }

    #[test]
    fn test_stats() {
        let mut ctrl = AccessController::new();
        ctrl.add_user(WalletUser::new("u1", "Alice", Role::Owner)).unwrap();
        ctrl.add_user(WalletUser::new("u2", "Bob", Role::Viewer)).unwrap();
        let mut inactive = WalletUser::new("u3", "Carol", Role::Admin);
        inactive.deactivate();
        ctrl.add_user(inactive).unwrap();

        ctrl.check_access("u2", &Action::Transfer).unwrap(); // deny
        ctrl.check_access("u1", &Action::Transfer).unwrap(); // allow

        let stats = ctrl.stats();
        assert_eq!(stats.total_users, 3);
        assert_eq!(stats.active_users, 2);
        assert_eq!(stats.roles_configured, 4);
        assert_eq!(stats.total_log_entries, 2);
        assert_eq!(stats.recent_denials, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip.json");
        let mut ctrl = AccessController::new();
        ctrl.add_user(WalletUser::new("u1", "Alice", Role::Owner)).unwrap();
        ctrl.add_user(WalletUser::new("u2", "Bob", Role::Viewer)).unwrap();
        ctrl.check_access("u1", &Action::Transfer).unwrap();

        ctrl.save(&path).unwrap();
        let loaded = AccessController::load(&path).unwrap();
        assert_eq!(loaded.users.len(), 2);
        assert!(loaded.get_user("u1").is_some());
        assert_eq!(loaded.access_log.len(), 1);
        assert_eq!(loaded.role_permissions.len(), 4);

        // Clean up
        let _ = std::fs::remove_file(&path);

        // Test load_or_default on missing file
        let missing = test_path("nonexistent.json");
        let default = AccessController::load_or_default(&missing);
        assert!(default.users.is_empty());
        assert_eq!(default.role_permissions.len(), 4);
    }
}
