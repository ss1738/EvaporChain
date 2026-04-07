//! Hardware wallet bridge — abstract interface for external signing devices.
//!
//! Provides a trait-based abstraction for hardware wallets (Ledger, Trezor, etc.)
//! so the wallet can delegate signing to a secure device without ever
//! touching private keys.
//!
//! Includes a simulated device for testing and development.
//!
//! # Architecture
//!
//! ```text
//! HardwareWallet (trait)
//! ├── SimulatedDevice  (for testing)
//! └── [LedgerDevice]   (future: real Ledger integration)
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum HardwareError {
    #[error("device not connected: {0}")]
    NotConnected(String),

    #[error("user rejected on device")]
    UserRejected,

    #[error("device locked — unlock required")]
    DeviceLocked,

    #[error("signing failed: {0}")]
    SigningFailed(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),

    #[error("device timeout")]
    Timeout,

    #[error("device not found: {0}")]
    DeviceNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// Device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Ledger,
    Trezor,
    Simulated,
}

impl DeviceType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ledger => "ledger",
            Self::Trezor => "trezor",
            Self::Simulated => "simulated",
        }
    }
}

/// Information about a connected device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Device ID.
    pub id: String,
    /// Device type.
    pub device_type: DeviceType,
    /// Device model/name.
    pub model: String,
    /// Firmware version.
    pub firmware: String,
    /// Whether device is unlocked.
    pub unlocked: bool,
    /// Public key (if available).
    pub public_key: Option<String>,
    /// Address derived from public key.
    pub address: Option<String>,
}

/// A signing request to the hardware device.
#[derive(Debug, Clone, Serialize)]
pub struct SignRequest {
    /// What type of transaction.
    pub tx_type: String,
    /// Human-readable description for device display.
    pub display_message: String,
    /// Raw bytes to sign.
    pub payload: Vec<u8>,
}

/// Result of a signing operation.
#[derive(Debug, Clone, Serialize)]
pub struct SignResult {
    /// The signature bytes (hex-encoded).
    pub signature: String,
    /// The public key used (hex-encoded).
    pub public_key: String,
    /// Device that signed.
    pub device_id: String,
}

// ──────────────────────────── HardwareWallet Trait ────────────────────────

/// Trait for hardware wallet implementations.
pub trait HardwareWallet {
    /// Get device info.
    fn info(&self) -> &DeviceInfo;

    /// Check if device is connected and ready.
    fn is_connected(&self) -> bool;

    /// Get the device's public key.
    fn get_public_key(&self) -> Result<String, HardwareError>;

    /// Get the device's address.
    fn get_address(&self) -> Result<String, HardwareError>;

    /// Sign a message on the device.
    fn sign(&self, request: &SignRequest) -> Result<SignResult, HardwareError>;

    /// Verify a signature on the device.
    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool, HardwareError>;
}

// ──────────────────────────── SimulatedDevice ────────────────────────────

/// A simulated hardware device for testing.
/// Uses a local keypair to mimic device signing.
pub struct SimulatedDevice {
    info: DeviceInfo,
    /// Whether to simulate user rejection.
    reject_next: bool,
    /// Whether to simulate being locked.
    locked: bool,
    /// Keypair for signing (hex-encoded).
    secret_key: Vec<u8>,
    public_key: Vec<u8>,
}

impl SimulatedDevice {
    /// Create a new simulated device.
    pub fn new(name: &str) -> Self {
        // Generate a deterministic keypair from the name
        let seed = blake3::hash(name.as_bytes());
        let seed_bytes = seed.as_bytes();

        // Use first 32 bytes as "secret key" and hash again for "public key"
        let secret_key = seed_bytes.to_vec();
        let public_key = blake3::hash(&secret_key).as_bytes().to_vec();
        let address = format!("0x{}", hex::encode(&public_key[..32]));

        Self {
            info: DeviceInfo {
                id: format!("sim_{}", &hex::encode(&seed_bytes[..4])),
                device_type: DeviceType::Simulated,
                model: format!("Simulated ({})", name),
                firmware: "1.0.0-sim".to_string(),
                unlocked: true,
                public_key: Some(hex::encode(&public_key)),
                address: Some(address),
            },
            reject_next: false,
            locked: false,
            secret_key,
            public_key,
        }
    }

    /// Set whether the next signing request should be rejected.
    pub fn set_reject_next(&mut self, reject: bool) {
        self.reject_next = reject;
    }

    /// Set whether the device is locked.
    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.info.unlocked = !locked;
    }
}

impl HardwareWallet for SimulatedDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn is_connected(&self) -> bool {
        true // simulated device is always connected
    }

    fn get_public_key(&self) -> Result<String, HardwareError> {
        if self.locked {
            return Err(HardwareError::DeviceLocked);
        }
        Ok(hex::encode(&self.public_key))
    }

    fn get_address(&self) -> Result<String, HardwareError> {
        if self.locked {
            return Err(HardwareError::DeviceLocked);
        }
        Ok(self.info.address.clone().unwrap_or_default())
    }

    fn sign(&self, request: &SignRequest) -> Result<SignResult, HardwareError> {
        if self.locked {
            return Err(HardwareError::DeviceLocked);
        }
        if self.reject_next {
            return Err(HardwareError::UserRejected);
        }

        // Simulate signing: BLAKE3(secret_key || payload)
        let mut data = self.secret_key.clone();
        data.extend_from_slice(&request.payload);
        let signature = blake3::hash(&data);

        Ok(SignResult {
            signature: hex::encode(signature.as_bytes()),
            public_key: hex::encode(&self.public_key),
            device_id: self.info.id.clone(),
        })
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool, HardwareError> {
        // Simulate verification
        let mut data = self.secret_key.clone();
        data.extend_from_slice(message);
        let expected = blake3::hash(&data);
        Ok(expected.as_bytes() == signature)
    }
}

// ──────────────────────────── DeviceRegistry ─────────────────────────────

/// Registry of known/connected hardware devices.
#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceRegistry {
    /// Known devices by ID.
    pub devices: Vec<DeviceEntry>,
}

/// A registered device entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceEntry {
    /// Device ID.
    pub id: String,
    /// Device type.
    pub device_type: DeviceType,
    /// Human-readable name.
    pub name: String,
    /// Associated address.
    pub address: Option<String>,
    /// When device was registered.
    pub registered_at: String,
    /// Last used timestamp.
    pub last_used: Option<String>,
}

impl DeviceRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }

    /// Register a device.
    pub fn register(&mut self, info: &DeviceInfo, name: &str) {
        // Remove existing with same ID
        self.devices.retain(|d| d.id != info.id);
        self.devices.push(DeviceEntry {
            id: info.id.clone(),
            device_type: info.device_type,
            name: name.to_string(),
            address: info.address.clone(),
            registered_at: chrono::Utc::now().to_rfc3339(),
            last_used: None,
        });
    }

    /// Remove a device.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != id);
        self.devices.len() < before
    }

    /// Get a device entry.
    pub fn get(&self, id: &str) -> Option<&DeviceEntry> {
        self.devices.iter().find(|d| d.id == id)
    }

    /// List all devices.
    pub fn list(&self) -> &[DeviceEntry] {
        &self.devices
    }

    /// Mark a device as recently used.
    pub fn mark_used(&mut self, id: &str) {
        if let Some(dev) = self.devices.iter_mut().find(|d| d.id == id) {
            dev.last_used = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    /// Load from file.
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> Result<Self, HardwareError> {
        let data = std::fs::read_to_string(path)?;
        let reg: DeviceRegistry = serde_json::from_str(&data)?;
        Ok(reg)
    }

    /// Save to file.
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), HardwareError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Number of registered devices.
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Whether registry is empty.
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path for device registry.
pub fn default_device_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("devices.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_device() -> SimulatedDevice {
        SimulatedDevice::new("test-device")
    }

    fn make_sign_request() -> SignRequest {
        SignRequest {
            tx_type: "transfer".to_string(),
            display_message: "Send 1000 EVAP to 0xabc".to_string(),
            payload: vec![0xDE, 0xAD, 0xBE, 0xEF],
        }
    }

    #[test]
    fn test_simulated_device_creation() {
        let device = make_device();
        assert!(device.is_connected());
        assert!(device.info().unlocked);
        assert_eq!(device.info().device_type, DeviceType::Simulated);
    }

    #[test]
    fn test_get_public_key() {
        let device = make_device();
        let pk = device.get_public_key().unwrap();
        assert!(!pk.is_empty());
    }

    #[test]
    fn test_get_address() {
        let device = make_device();
        let addr = device.get_address().unwrap();
        assert!(addr.starts_with("0x"));
    }

    #[test]
    fn test_sign_success() {
        let device = make_device();
        let req = make_sign_request();
        let result = device.sign(&req).unwrap();
        assert!(!result.signature.is_empty());
        assert!(!result.public_key.is_empty());
        assert_eq!(result.device_id, device.info().id);
    }

    #[test]
    fn test_sign_deterministic() {
        let device = make_device();
        let req = make_sign_request();
        let r1 = device.sign(&req).unwrap();
        let r2 = device.sign(&req).unwrap();
        assert_eq!(r1.signature, r2.signature);
    }

    #[test]
    fn test_sign_different_payloads() {
        let device = make_device();
        let req1 = SignRequest {
            tx_type: "transfer".into(),
            display_message: "test".into(),
            payload: vec![1, 2, 3],
        };
        let req2 = SignRequest {
            tx_type: "transfer".into(),
            display_message: "test".into(),
            payload: vec![4, 5, 6],
        };
        let r1 = device.sign(&req1).unwrap();
        let r2 = device.sign(&req2).unwrap();
        assert_ne!(r1.signature, r2.signature);
    }

    #[test]
    fn test_sign_rejected() {
        let mut device = make_device();
        device.set_reject_next(true);
        let req = make_sign_request();
        let err = device.sign(&req).unwrap_err();
        assert!(matches!(err, HardwareError::UserRejected));
    }

    #[test]
    fn test_sign_locked() {
        let mut device = make_device();
        device.set_locked(true);
        let req = make_sign_request();
        let err = device.sign(&req).unwrap_err();
        assert!(matches!(err, HardwareError::DeviceLocked));
    }

    #[test]
    fn test_get_key_locked() {
        let mut device = make_device();
        device.set_locked(true);
        assert!(device.get_public_key().is_err());
        assert!(device.get_address().is_err());
    }

    #[test]
    fn test_verify_valid() {
        let device = make_device();
        let req = make_sign_request();
        let result = device.sign(&req).unwrap();
        let sig_bytes = hex::decode(&result.signature).unwrap();
        assert!(device.verify(&req.payload, &sig_bytes).unwrap());
    }

    #[test]
    fn test_verify_invalid() {
        let device = make_device();
        assert!(!device.verify(&[1, 2, 3], &[0; 32]).unwrap());
    }

    #[test]
    fn test_device_registry_register() {
        let mut reg = DeviceRegistry::new();
        let device = make_device();
        reg.register(device.info(), "My Test Device");
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get(&device.info().id).unwrap().name, "My Test Device");
    }

    #[test]
    fn test_device_registry_register_replaces() {
        let mut reg = DeviceRegistry::new();
        let device = make_device();
        reg.register(device.info(), "First");
        reg.register(device.info(), "Second");
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get(&device.info().id).unwrap().name, "Second");
    }

    #[test]
    fn test_device_registry_remove() {
        let mut reg = DeviceRegistry::new();
        let device = make_device();
        reg.register(device.info(), "Test");
        assert!(reg.remove(&device.info().id));
        assert!(reg.is_empty());
    }

    #[test]
    fn test_device_registry_remove_not_found() {
        let mut reg = DeviceRegistry::new();
        assert!(!reg.remove("nope"));
    }

    #[test]
    fn test_device_registry_mark_used() {
        let mut reg = DeviceRegistry::new();
        let device = make_device();
        reg.register(device.info(), "Test");
        assert!(reg.get(&device.info().id).unwrap().last_used.is_none());
        reg.mark_used(&device.info().id);
        assert!(reg.get(&device.info().id).unwrap().last_used.is_some());
    }

    #[test]
    fn test_device_type_label() {
        assert_eq!(DeviceType::Ledger.label(), "ledger");
        assert_eq!(DeviceType::Trezor.label(), "trezor");
        assert_eq!(DeviceType::Simulated.label(), "simulated");
    }

    #[test]
    fn test_device_info_serializable() {
        let device = make_device();
        let json = serde_json::to_string(device.info()).unwrap();
        assert!(json.contains("\"device_type\":\"simulated\""));
    }

    #[test]
    fn test_sign_result_serializable() {
        let result = SignResult {
            signature: "abc123".into(),
            public_key: "def456".into(),
            device_id: "sim_1".into(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"signature\":\"abc123\""));
    }

    #[test]
    fn test_sign_request_serializable() {
        let req = make_sign_request();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"tx_type\":\"transfer\""));
    }

    #[test]
    fn test_registry_json_roundtrip() {
        let mut reg = DeviceRegistry::new();
        let device = make_device();
        reg.register(device.info(), "Test");

        let json = serde_json::to_string_pretty(&reg).unwrap();
        let loaded: DeviceRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn test_registry_file_save_load() {
        let dir = std::env::temp_dir().join("evaporchain_hw_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("devices.json");

        let mut reg = DeviceRegistry::new();
        let device = make_device();
        reg.register(device.info(), "Test");
        reg.save(&path).unwrap();

        let loaded = DeviceRegistry::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_different_devices_different_keys() {
        let d1 = SimulatedDevice::new("device-a");
        let d2 = SimulatedDevice::new("device-b");
        assert_ne!(d1.get_public_key().unwrap(), d2.get_public_key().unwrap());
        assert_ne!(d1.get_address().unwrap(), d2.get_address().unwrap());
    }
}
