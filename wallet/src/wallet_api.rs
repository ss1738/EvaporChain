use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum WalletApiError {
    #[error("endpoint not found: {0}")]
    EndpointNotFound(String),
    #[error("duplicate endpoint: {0}")]
    DuplicateEndpoint(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Post => write!(f, "POST"),
            HttpMethod::Put => write!(f, "PUT"),
            HttpMethod::Delete => write!(f, "DELETE"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApiVersion {
    V1,
    V2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthType {
    None,
    ApiKey,
    Bearer,
    Hmac,
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpoint {
    pub id: String,
    pub path: String,
    pub method: HttpMethod,
    pub version: ApiVersion,
    pub description: String,
    pub auth_required: AuthType,
    pub rate_limit_per_min: Option<u32>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub id: String,
    pub endpoint_id: String,
    pub method: HttpMethod,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub timestamp: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub request_id: String,
    pub status_code: u16,
    pub body: String,
    pub headers: HashMap<String, String>,
    pub duration_ms: u64,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey2 {
    pub key: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub active: bool,
    pub request_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiStats2 {
    pub total_endpoints: usize,
    pub total_requests: u64,
    pub total_api_keys: usize,
    pub active_keys: usize,
    pub avg_response_ms: f64,
    pub by_method: HashMap<String, u64>,
    pub by_status: HashMap<String, u64>,
}

// ---------------------------------------------------------------------------
// WalletApi
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WalletApi {
    pub endpoints: HashMap<String, ApiEndpoint>,
    pub api_keys: HashMap<String, ApiKey2>,
    pub request_log: Vec<ApiResponse>,
    pub total_requests: u64,
}

impl WalletApi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_endpoint(&mut self, endpoint: ApiEndpoint) -> Result<(), WalletApiError> {
        if self.endpoints.contains_key(&endpoint.id) {
            return Err(WalletApiError::DuplicateEndpoint(endpoint.id));
        }
        self.endpoints.insert(endpoint.id.clone(), endpoint);
        Ok(())
    }

    pub fn remove_endpoint(&mut self, id: &str) -> Result<ApiEndpoint, WalletApiError> {
        self.endpoints
            .remove(id)
            .ok_or_else(|| WalletApiError::EndpointNotFound(id.to_string()))
    }

    pub fn get_endpoint(&self, id: &str) -> Option<&ApiEndpoint> {
        self.endpoints.get(id)
    }

    pub fn create_api_key(
        &mut self,
        key: &str,
        name: &str,
        permissions: Vec<String>,
        expires_at: Option<String>,
    ) -> Result<(), WalletApiError> {
        if self.api_keys.contains_key(key) {
            return Err(WalletApiError::InvalidRequest(format!(
                "API key already exists: {}",
                key
            )));
        }
        let api_key = ApiKey2 {
            key: key.to_string(),
            name: name.to_string(),
            permissions,
            created_at: Utc::now().to_rfc3339(),
            expires_at,
            active: true,
            request_count: 0,
        };
        self.api_keys.insert(key.to_string(), api_key);
        Ok(())
    }

    pub fn revoke_api_key(&mut self, key: &str) -> Result<(), WalletApiError> {
        match self.api_keys.get_mut(key) {
            Some(k) => {
                k.active = false;
                Ok(())
            }
            None => Err(WalletApiError::EndpointNotFound(format!(
                "API key not found: {}",
                key
            ))),
        }
    }

    pub fn validate_api_key(&self, key: &str) -> Result<bool, WalletApiError> {
        match self.api_keys.get(key) {
            Some(k) => {
                if !k.active {
                    return Ok(false);
                }
                if let Some(ref exp) = k.expires_at {
                    if let Ok(exp_time) = exp.parse::<chrono::DateTime<Utc>>() {
                        if Utc::now() > exp_time {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn handle_request(&mut self, request: ApiRequest) -> Result<ApiResponse, WalletApiError> {
        let endpoint = self
            .endpoints
            .get(&request.endpoint_id)
            .ok_or_else(|| WalletApiError::EndpointNotFound(request.endpoint_id.clone()))?;

        // Check auth if required
        if endpoint.auth_required != AuthType::None {
            match &request.api_key {
                Some(key) => {
                    let valid = self.validate_api_key(key)?;
                    if !valid {
                        return Err(WalletApiError::Unauthorized(
                            "invalid or inactive API key".to_string(),
                        ));
                    }
                    // Increment request count on the key
                    if let Some(k) = self.api_keys.get_mut(key) {
                        k.request_count += 1;
                    }
                }
                None => {
                    return Err(WalletApiError::Unauthorized("API key required".to_string()));
                }
            }
        }

        self.total_requests += 1;

        let response = ApiResponse {
            request_id: request.id,
            status_code: 200,
            body: "ok".to_string(),
            headers: HashMap::new(),
            duration_ms: 1,
            timestamp: Utc::now().to_rfc3339(),
        };

        self.request_log.push(response.clone());
        Ok(response)
    }

    pub fn endpoints_by_version(&self, version: &ApiVersion) -> Vec<&ApiEndpoint> {
        self.endpoints
            .values()
            .filter(|e| &e.version == version)
            .collect()
    }

    pub fn endpoints_by_method(&self, method: &HttpMethod) -> Vec<&ApiEndpoint> {
        self.endpoints
            .values()
            .filter(|e| &e.method == method)
            .collect()
    }

    pub fn active_api_keys(&self) -> Vec<&ApiKey2> {
        self.api_keys.values().filter(|k| k.active).collect()
    }

    pub fn recent_responses(&self, n: usize) -> Vec<&ApiResponse> {
        self.request_log.iter().rev().take(n).collect()
    }

    pub fn search_endpoints(&self, query: &str) -> Vec<&ApiEndpoint> {
        let q = query.to_lowercase();
        self.endpoints
            .values()
            .filter(|e| {
                e.path.to_lowercase().contains(&q) || e.description.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn stats(&self) -> ApiStats2 {
        let mut by_method: HashMap<String, u64> = HashMap::new();
        let mut by_status: HashMap<String, u64> = HashMap::new();
        let mut total_ms: u64 = 0;

        for resp in &self.request_log {
            *by_status.entry(resp.status_code.to_string()).or_insert(0) += 1;
            total_ms += resp.duration_ms;
        }

        for ep in self.endpoints.values() {
            *by_method.entry(ep.method.to_string()).or_insert(0) += 1;
        }

        let avg_response_ms = if self.request_log.is_empty() {
            0.0
        } else {
            total_ms as f64 / self.request_log.len() as f64
        };

        let active_keys = self.api_keys.values().filter(|k| k.active).count();

        ApiStats2 {
            total_endpoints: self.endpoints.len(),
            total_requests: self.total_requests,
            total_api_keys: self.api_keys.len(),
            active_keys,
            avg_response_ms,
            by_method,
            by_status,
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), WalletApiError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, WalletApiError> {
        let data = std::fs::read_to_string(path)?;
        let api: Self = serde_json::from_str(&data)?;
        Ok(api)
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

    fn test_path() -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!("wallet_api_test_{}.json", std::process::id()))
    }

    fn make_endpoint(
        id: &str,
        method: HttpMethod,
        version: ApiVersion,
        auth: AuthType,
    ) -> ApiEndpoint {
        ApiEndpoint {
            id: id.to_string(),
            path: format!("/api/{}", id),
            method,
            version,
            description: format!("Endpoint {}", id),
            auth_required: auth,
            rate_limit_per_min: Some(100),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn make_request(id: &str, endpoint_id: &str, key: Option<&str>) -> ApiRequest {
        ApiRequest {
            id: id.to_string(),
            endpoint_id: endpoint_id.to_string(),
            method: HttpMethod::Get,
            headers: HashMap::new(),
            body: None,
            timestamp: Utc::now().to_rfc3339(),
            api_key: key.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_register_endpoint() {
        let mut api = WalletApi::new();
        let ep = make_endpoint("balance", HttpMethod::Get, ApiVersion::V1, AuthType::None);
        assert!(api.register_endpoint(ep).is_ok());
        assert_eq!(api.endpoints.len(), 1);
    }

    #[test]
    fn test_register_duplicate_endpoint() {
        let mut api = WalletApi::new();
        let ep1 = make_endpoint("balance", HttpMethod::Get, ApiVersion::V1, AuthType::None);
        let ep2 = make_endpoint("balance", HttpMethod::Post, ApiVersion::V2, AuthType::None);
        api.register_endpoint(ep1).unwrap();
        assert!(api.register_endpoint(ep2).is_err());
    }

    #[test]
    fn test_remove_endpoint() {
        let mut api = WalletApi::new();
        let ep = make_endpoint("balance", HttpMethod::Get, ApiVersion::V1, AuthType::None);
        api.register_endpoint(ep).unwrap();
        let removed = api.remove_endpoint("balance").unwrap();
        assert_eq!(removed.id, "balance");
        assert!(api.endpoints.is_empty());
    }

    #[test]
    fn test_remove_endpoint_not_found() {
        let mut api = WalletApi::new();
        assert!(api.remove_endpoint("nope").is_err());
    }

    #[test]
    fn test_get_endpoint() {
        let mut api = WalletApi::new();
        let ep = make_endpoint("tx", HttpMethod::Post, ApiVersion::V1, AuthType::None);
        api.register_endpoint(ep).unwrap();
        assert!(api.get_endpoint("tx").is_some());
        assert!(api.get_endpoint("missing").is_none());
    }

    #[test]
    fn test_create_api_key() {
        let mut api = WalletApi::new();
        assert!(api
            .create_api_key("key1", "Test Key", vec!["read".into()], None)
            .is_ok());
        assert_eq!(api.api_keys.len(), 1);
        assert!(api.api_keys.get("key1").unwrap().active);
    }

    #[test]
    fn test_create_duplicate_api_key() {
        let mut api = WalletApi::new();
        api.create_api_key("key1", "A", vec![], None).unwrap();
        assert!(api.create_api_key("key1", "B", vec![], None).is_err());
    }

    #[test]
    fn test_revoke_api_key() {
        let mut api = WalletApi::new();
        api.create_api_key("key1", "A", vec![], None).unwrap();
        api.revoke_api_key("key1").unwrap();
        assert!(!api.api_keys.get("key1").unwrap().active);
    }

    #[test]
    fn test_revoke_missing_key() {
        let mut api = WalletApi::new();
        assert!(api.revoke_api_key("nope").is_err());
    }

    #[test]
    fn test_validate_active_key() {
        let mut api = WalletApi::new();
        api.create_api_key("key1", "A", vec![], None).unwrap();
        assert!(api.validate_api_key("key1").unwrap());
    }

    #[test]
    fn test_validate_revoked_key() {
        let mut api = WalletApi::new();
        api.create_api_key("key1", "A", vec![], None).unwrap();
        api.revoke_api_key("key1").unwrap();
        assert!(!api.validate_api_key("key1").unwrap());
    }

    #[test]
    fn test_validate_expired_key() {
        let mut api = WalletApi::new();
        let past = "2020-01-01T00:00:00Z".to_string();
        api.create_api_key("key1", "A", vec![], Some(past)).unwrap();
        assert!(!api.validate_api_key("key1").unwrap());
    }

    #[test]
    fn test_validate_missing_key() {
        let api = WalletApi::new();
        assert!(!api.validate_api_key("nope").unwrap());
    }

    #[test]
    fn test_handle_request_success_no_auth() {
        let mut api = WalletApi::new();
        let ep = make_endpoint("balance", HttpMethod::Get, ApiVersion::V1, AuthType::None);
        api.register_endpoint(ep).unwrap();

        let req = make_request("r1", "balance", None);
        let resp = api.handle_request(req).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body, "ok");
        assert_eq!(api.total_requests, 1);
    }

    #[test]
    fn test_handle_request_success_with_auth() {
        let mut api = WalletApi::new();
        let ep = make_endpoint(
            "transfer",
            HttpMethod::Post,
            ApiVersion::V1,
            AuthType::ApiKey,
        );
        api.register_endpoint(ep).unwrap();
        api.create_api_key("key1", "A", vec![], None).unwrap();

        let req = make_request("r1", "transfer", Some("key1"));
        let resp = api.handle_request(req).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(api.api_keys.get("key1").unwrap().request_count, 1);
    }

    #[test]
    fn test_handle_request_unauthorized_no_key() {
        let mut api = WalletApi::new();
        let ep = make_endpoint(
            "transfer",
            HttpMethod::Post,
            ApiVersion::V1,
            AuthType::ApiKey,
        );
        api.register_endpoint(ep).unwrap();

        let req = make_request("r1", "transfer", None);
        assert!(api.handle_request(req).is_err());
    }

    #[test]
    fn test_handle_request_unauthorized_invalid_key() {
        let mut api = WalletApi::new();
        let ep = make_endpoint(
            "transfer",
            HttpMethod::Post,
            ApiVersion::V1,
            AuthType::Bearer,
        );
        api.register_endpoint(ep).unwrap();

        let req = make_request("r1", "transfer", Some("bad_key"));
        assert!(api.handle_request(req).is_err());
    }

    #[test]
    fn test_handle_request_endpoint_not_found() {
        let mut api = WalletApi::new();
        let req = make_request("r1", "missing", None);
        assert!(api.handle_request(req).is_err());
    }

    #[test]
    fn test_endpoints_by_version() {
        let mut api = WalletApi::new();
        api.register_endpoint(make_endpoint(
            "a",
            HttpMethod::Get,
            ApiVersion::V1,
            AuthType::None,
        ))
        .unwrap();
        api.register_endpoint(make_endpoint(
            "b",
            HttpMethod::Post,
            ApiVersion::V2,
            AuthType::None,
        ))
        .unwrap();
        api.register_endpoint(make_endpoint(
            "c",
            HttpMethod::Get,
            ApiVersion::V1,
            AuthType::None,
        ))
        .unwrap();

        assert_eq!(api.endpoints_by_version(&ApiVersion::V1).len(), 2);
        assert_eq!(api.endpoints_by_version(&ApiVersion::V2).len(), 1);
    }

    #[test]
    fn test_endpoints_by_method() {
        let mut api = WalletApi::new();
        api.register_endpoint(make_endpoint(
            "a",
            HttpMethod::Get,
            ApiVersion::V1,
            AuthType::None,
        ))
        .unwrap();
        api.register_endpoint(make_endpoint(
            "b",
            HttpMethod::Post,
            ApiVersion::V1,
            AuthType::None,
        ))
        .unwrap();

        assert_eq!(api.endpoints_by_method(&HttpMethod::Get).len(), 1);
        assert_eq!(api.endpoints_by_method(&HttpMethod::Delete).len(), 0);
    }

    #[test]
    fn test_active_api_keys() {
        let mut api = WalletApi::new();
        api.create_api_key("k1", "A", vec![], None).unwrap();
        api.create_api_key("k2", "B", vec![], None).unwrap();
        api.revoke_api_key("k1").unwrap();

        assert_eq!(api.active_api_keys().len(), 1);
    }

    #[test]
    fn test_recent_responses() {
        let mut api = WalletApi::new();
        api.register_endpoint(make_endpoint(
            "ep",
            HttpMethod::Get,
            ApiVersion::V1,
            AuthType::None,
        ))
        .unwrap();

        for i in 0..5 {
            let req = make_request(&format!("r{}", i), "ep", None);
            api.handle_request(req).unwrap();
        }

        let recent = api.recent_responses(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].request_id, "r4");
    }

    #[test]
    fn test_search_endpoints() {
        let mut api = WalletApi::new();
        let mut ep = make_endpoint("bal", HttpMethod::Get, ApiVersion::V1, AuthType::None);
        ep.path = "/api/balance".to_string();
        ep.description = "Get wallet balance".to_string();
        api.register_endpoint(ep).unwrap();

        let mut ep2 = make_endpoint("tx", HttpMethod::Post, ApiVersion::V1, AuthType::None);
        ep2.path = "/api/transfer".to_string();
        ep2.description = "Send tokens".to_string();
        api.register_endpoint(ep2).unwrap();

        assert_eq!(api.search_endpoints("balance").len(), 1);
        assert_eq!(api.search_endpoints("token").len(), 1);
        assert_eq!(api.search_endpoints("xyz").len(), 0);
    }

    #[test]
    fn test_stats() {
        let mut api = WalletApi::new();
        api.register_endpoint(make_endpoint(
            "a",
            HttpMethod::Get,
            ApiVersion::V1,
            AuthType::None,
        ))
        .unwrap();
        api.register_endpoint(make_endpoint(
            "b",
            HttpMethod::Post,
            ApiVersion::V1,
            AuthType::None,
        ))
        .unwrap();
        api.create_api_key("k1", "A", vec![], None).unwrap();

        let req = make_request("r1", "a", None);
        api.handle_request(req).unwrap();

        let s = api.stats();
        assert_eq!(s.total_endpoints, 2);
        assert_eq!(s.total_requests, 1);
        assert_eq!(s.total_api_keys, 1);
        assert_eq!(s.active_keys, 1);
        assert!(s.avg_response_ms > 0.0);
    }

    #[test]
    fn test_save_and_load() {
        let path = test_path();
        let mut api = WalletApi::new();
        api.register_endpoint(make_endpoint(
            "ep1",
            HttpMethod::Get,
            ApiVersion::V1,
            AuthType::None,
        ))
        .unwrap();
        api.create_api_key("k1", "Key1", vec!["read".into()], None)
            .unwrap();
        api.save(&path).unwrap();

        let loaded = WalletApi::load(&path).unwrap();
        assert_eq!(loaded.endpoints.len(), 1);
        assert_eq!(loaded.api_keys.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = std::env::temp_dir().join(format!("nonexistent_{}.json", std::process::id()));
        let api = WalletApi::load_or_default(&path);
        assert!(api.endpoints.is_empty());
    }
}
