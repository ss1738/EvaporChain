//! Authentication and wallet API endpoints.

use axum::{extract::State, http::HeaderMap, Json};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use evaporchain_crypto::signatures::MlDsaKeypair;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::user_db::UserDb;

/// Session store: token → (user_id, created_at).
pub type Sessions = Arc<Mutex<HashMap<String, (i64, Instant)>>>;

const SESSION_TTL_SECS: u64 = 86400; // 24 hours

/// Shared auth state.
pub struct AuthState {
    pub user_db: Arc<UserDb>,
    pub sessions: Sessions,
    /// Login rate limiter: email -> (attempt_count, first_attempt_time).
    pub login_rate_limit: Mutex<HashMap<String, (u32, Instant)>>,
    /// Registration rate limiter: IP not available, so limit by global counter per window.
    pub register_rate_limit: Mutex<(u32, Instant)>,
    /// R11 (audit 2026-05-15): verify-email rate limiter:
    /// email -> (attempt_count, first_attempt_time). Required to
    /// stop brute-force against the 6-digit verification code.
    /// `user_db.verify_email` is SQL-safe (prepared statement) but
    /// the underlying `UPDATE users SET email_verified = 1 WHERE
    /// email = ? AND verification_code = ? AND email_verified = 0`
    /// allows unlimited attempts against an unverified account.
    /// 10^6 / 2 ≈ 500K attempts to brute-force on average;
    /// without this gate that takes <1 minute at 10k req/s.
    pub verify_rate_limit: Mutex<HashMap<String, (u32, Instant)>>,
}

// ── Request / Response types ──

#[derive(Deserialize)]
pub struct RegisterReq {
    pub email: String,
    pub password: String,
    pub display_name: String,
}

#[derive(Serialize)]
pub struct RegisterResp {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_code: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginReq {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResp {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserInfo>,
}

#[derive(Serialize, Clone)]
pub struct UserInfo {
    pub id: i64,
    pub email: String,
    pub display_name: String,
    pub email_verified: bool,
}

#[derive(Deserialize)]
pub struct VerifyEmailReq {
    pub email: String,
    pub code: String,
}

#[derive(Deserialize)]
pub struct CreateWalletReq {
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct WalletResp {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

#[derive(Serialize)]
pub struct GenericResp {
    pub success: bool,
    pub message: String,
}

// ── Token helpers ──

fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..64).map(|_| rng.gen::<u8>()).collect();
    hex::encode(bytes)
}

fn generate_verification_code() -> String {
    let mut rng = rand::thread_rng();
    format!("{:06}", rng.gen_range(100000..999999))
}

/// Generate a wallet address derived from the ML-DSA public key.
/// Address = "0x" + hex(blake3(public_key_bytes)[..32]).
fn generate_address_from_pubkey(pk_bytes: &[u8]) -> String {
    let hash = blake3::hash(pk_bytes);
    format!("0x{}", hex::encode(hash.as_bytes()))
}

/// Derive the 32-byte master encryption key for wallet private keys.
/// Uses EVAPORCHAIN_KEY_MASTER env var if set, otherwise a dev-only fallback.
fn master_encryption_key() -> [u8; 32] {
    let seed = std::env::var("EVAPORCHAIN_KEY_MASTER")
        .unwrap_or_else(|_| "EVAPORCHAIN_DEV_KEY_DO_NOT_USE_IN_PRODUCTION".to_string());
    blake3::derive_key("evaporchain wallet key encryption", seed.as_bytes())
}

/// Encrypt a hex-encoded secret key with XChaCha20-Poly1305.
/// Returns hex(nonce || ciphertext).
fn encrypt_secret_key(sk_hex: &str) -> Result<String, String> {
    let key = master_encryption_key();
    let cipher =
        XChaCha20Poly1305::new_from_slice(&key).map_err(|e| format!("invalid key: {e}"))?;
    let mut nonce_bytes = [0u8; 24];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, sk_hex.as_bytes())
        .map_err(|e| format!("encryption failed: {e}"))?;
    let mut out = Vec::with_capacity(24 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(hex::encode(out))
}

/// Decrypt a hex-encoded encrypted secret key. Returns the plaintext hex SK.
#[allow(dead_code)]
fn decrypt_secret_key(encrypted_hex: &str) -> Result<String, String> {
    let data = hex::decode(encrypted_hex).map_err(|e| format!("hex decode: {e}"))?;
    if data.len() < 24 {
        return Err("encrypted key too short".into());
    }
    let (nonce_bytes, ciphertext) = data.split_at(24);
    let key = master_encryption_key();
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| "invalid key".to_string())?;
    let nonce = XNonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "decryption failed — wrong key or corrupted data".to_string())?;
    String::from_utf8(plaintext).map_err(|e| format!("invalid UTF-8: {e}"))
}

/// Generate a real ML-DSA (Dilithium3) keypair.
/// Returns (address, public_key_hex, encrypted_secret_key_hex).
fn generate_keypair() -> (String, String, String) {
    let kp = MlDsaKeypair::generate();
    let pk_hex = hex::encode(kp.public_key());
    let sk_hex = hex::encode(kp.secret_key());
    let address = generate_address_from_pubkey(kp.public_key());
    let encrypted_sk = encrypt_secret_key(&sk_hex).expect("wallet key encryption must not fail");
    (address, pk_hex, encrypted_sk)
}

/// Extract user_id from Authorization header token.
pub fn authenticate(headers: &HeaderMap, sessions: &Sessions) -> Result<i64, String> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.strip_prefix("Bearer ").unwrap_or(s))
        .ok_or("Missing authorization token")?;

    let sessions = sessions.lock().unwrap();
    let (user_id, created) = sessions.get(token).ok_or("Invalid or expired token")?;

    if created.elapsed().as_secs() > SESSION_TTL_SECS {
        return Err("Token expired".into());
    }

    Ok(*user_id)
}

// ── Handlers ──

pub async fn register(
    State(state): State<Arc<AuthState>>,
    Json(req): Json<RegisterReq>,
) -> Json<RegisterResp> {
    // Rate limit: max 20 registrations per 10 minutes globally
    {
        let mut limit = state.register_rate_limit.lock().unwrap();
        if limit.1.elapsed().as_secs() > 600 {
            *limit = (0, Instant::now());
        }
        limit.0 += 1;
        if limit.0 > 20 {
            return Json(RegisterResp {
                success: false,
                message: "Registration rate limit reached. Try again later.".into(),
                user_id: None,
                verification_code: None,
            });
        }
    }
    // Validate
    let email = req.email.trim().to_lowercase();
    if email.len() > 254 {
        return Json(RegisterResp {
            success: false,
            message: "Email too long".into(),
            user_id: None,
            verification_code: None,
        });
    }
    if email.is_empty() || !crate::api::is_valid_email(&email) {
        return Json(RegisterResp {
            success: false,
            message: "Invalid email format".into(),
            user_id: None,
            verification_code: None,
        });
    }
    if req.password.len() < 8 {
        return Json(RegisterResp {
            success: false,
            message: "Password must be at least 8 characters".into(),
            user_id: None,
            verification_code: None,
        });
    }
    if req.password.len() > 128 {
        return Json(RegisterResp {
            success: false,
            message: "Password too long (max 128 characters)".into(),
            user_id: None,
            verification_code: None,
        });
    }
    let display_name = req.display_name.trim().to_string();
    if display_name.is_empty() {
        return Json(RegisterResp {
            success: false,
            message: "Display name required".into(),
            user_id: None,
            verification_code: None,
        });
    }
    if display_name.len() > 100 {
        return Json(RegisterResp {
            success: false,
            message: "Display name too long (max 100 characters)".into(),
            user_id: None,
            verification_code: None,
        });
    }
    // Strip HTML from display name to prevent stored XSS
    let display_name: String = {
        let mut result = String::new();
        let mut in_tag = false;
        for c in display_name.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => result.push(c),
                _ => {}
            }
        }
        result
    };

    // Hash password (cost=10 for production, using 4 would be faster for testnet)
    let hash = match bcrypt::hash(&req.password, 10) {
        Ok(h) => h,
        Err(_e) => {
            return Json(RegisterResp {
                success: false,
                message: "Registration error. Please try again.".into(),
                user_id: None,
                verification_code: None,
            })
        }
    };

    let code = generate_verification_code();
    match state
        .user_db
        .create_user(&email, &hash, &display_name, &code)
    {
        Ok(user_id) => {
            // Auto-verify on testnet (no email server)
            let _ = state.user_db.verify_email(&email, &code);
            Json(RegisterResp {
                success: true,
                message: "Account created and verified!".into(),
                user_id: Some(user_id),
                verification_code: None,
            })
        }
        Err(e) => Json(RegisterResp {
            success: false,
            message: e,
            user_id: None,
            verification_code: None,
        }),
    }
}

pub async fn login(
    State(state): State<Arc<AuthState>>,
    Json(req): Json<LoginReq>,
) -> Json<LoginResp> {
    let email = req.email.trim().to_lowercase();

    // Rate limit: max 10 attempts per email per 15 minutes
    {
        let mut limits = state.login_rate_limit.lock().unwrap();
        let entry = limits.entry(email.clone()).or_insert((0, Instant::now()));
        if entry.1.elapsed().as_secs() > 900 {
            *entry = (0, Instant::now()); // Reset window
        }
        entry.0 += 1;
        if entry.0 > 10 {
            return Json(LoginResp {
                success: false,
                message: "Too many login attempts. Try again in 15 minutes.".into(),
                token: None,
                user: None,
            });
        }
        // Evict old entries
        if limits.len() > 10_000 {
            limits.retain(|_, (_, t)| t.elapsed().as_secs() < 900);
        }
    }

    let user = match state.user_db.get_user_by_email(&email) {
        Ok(Some(u)) => u,
        Ok(None) => {
            return Json(LoginResp {
                success: false,
                message: "Invalid email or password".into(),
                token: None,
                user: None,
            })
        }
        Err(e) => {
            return Json(LoginResp {
                success: false,
                message: format!("Error: {e}"),
                token: None,
                user: None,
            })
        }
    };

    let (user_id, password_hash, display_name, verified, _code) = user;

    match bcrypt::verify(&req.password, &password_hash) {
        Ok(true) => {}
        _ => {
            return Json(LoginResp {
                success: false,
                message: "Invalid email or password".into(),
                token: None,
                user: None,
            })
        }
    }

    let token = generate_token();
    {
        let mut sessions = state.sessions.lock().unwrap();
        // Clean expired sessions
        sessions.retain(|_, (_, created)| created.elapsed().as_secs() < SESSION_TTL_SECS);
        // Revoke all existing sessions for this user (prevents token theft persistence)
        sessions.retain(|_, (uid, _)| *uid != user_id);
        sessions.insert(token.clone(), (user_id, Instant::now()));
    }

    let _ = state.user_db.update_last_login(user_id);

    Json(LoginResp {
        success: true,
        message: "Login successful".into(),
        token: Some(token),
        user: Some(UserInfo {
            id: user_id,
            email,
            display_name,
            email_verified: verified,
        }),
    })
}

pub async fn verify_email(
    State(state): State<Arc<AuthState>>,
    Json(req): Json<VerifyEmailReq>,
) -> Json<GenericResp> {
    let email = req.email.trim().to_lowercase();

    // R11 (audit 2026-05-15): rate-limit per email — pattern
    // mirrors `login` at auth.rs:325. Max 10 attempts per email
    // per 15 minutes; window resets on first attempt after expiry.
    // Without this gate, the 6-digit verification code is
    // brute-forceable in seconds.
    {
        let mut limits = state.verify_rate_limit.lock().unwrap();
        let entry = limits.entry(email.clone()).or_insert((0, Instant::now()));
        if entry.1.elapsed().as_secs() > 900 {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
        if entry.0 > 10 {
            return Json(GenericResp {
                success: false,
                message: "Too many verification attempts. Try again in 15 minutes.".into(),
            });
        }
        // Evict old entries (mirror login's bound).
        if limits.len() > 10_000 {
            limits.retain(|_, (_, t)| t.elapsed().as_secs() < 900);
        }
    }

    match state.user_db.verify_email(&email, &req.code) {
        Ok(true) => Json(GenericResp {
            success: true,
            message: "Email verified!".into(),
        }),
        Ok(false) => Json(GenericResp {
            success: false,
            message: "Invalid verification code".into(),
        }),
        Err(_e) => Json(GenericResp {
            success: false,
            // R11 (audit 2026-05-15): do not reflect raw DB error
            // text to the client — the underlying `rusqlite::Error`
            // can include filesystem paths, schema names, and
            // internal state useful for fingerprinting / pivoting.
            // Same sanitisation pattern as `login`'s
            // "Invalid email or password" canonicalisation.
            message: "Verification failed. Try again.".into(),
        }),
    }
}

pub async fn get_me(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let user_id = match authenticate(&headers, &state.sessions) {
        Ok(id) => id,
        Err(e) => return Json(serde_json::json!({"success": false, "message": e})),
    };

    match state.user_db.get_user_by_id(user_id) {
        Ok(Some(user)) => Json(serde_json::json!({"success": true, "user": user})),
        Ok(None) => Json(serde_json::json!({"success": false, "message": "User not found"})),
        Err(e) => Json(serde_json::json!({"success": false, "message": e})),
    }
}

pub async fn logout(State(state): State<Arc<AuthState>>, headers: HeaderMap) -> Json<GenericResp> {
    if let Some(token) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        let token = token.strip_prefix("Bearer ").unwrap_or(token);
        let mut sessions = state.sessions.lock().unwrap();
        sessions.remove(token);
    }
    Json(GenericResp {
        success: true,
        message: "Logged out".into(),
    })
}

pub async fn create_wallet(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
    Json(req): Json<CreateWalletReq>,
) -> Json<WalletResp> {
    let user_id = match authenticate(&headers, &state.sessions) {
        Ok(id) => id,
        Err(e) => {
            return Json(WalletResp {
                success: false,
                message: e,
                address: None,
                public_key: None,
            })
        }
    };

    let name = req.name.unwrap_or_else(|| "Main Wallet".into());
    let (address, public_key, encrypted_private_key) = generate_keypair();

    match state.user_db.create_wallet(
        user_id,
        &name,
        &address,
        &public_key,
        &encrypted_private_key,
    ) {
        Ok(_) => {
            let _ = state.user_db.log_activity(
                user_id,
                &address,
                "wallet_created",
                &format!("Created wallet: {name}"),
            );
            Json(WalletResp {
                success: true,
                message: "Wallet created".into(),
                address: Some(address),
                public_key: Some(public_key),
            })
        }
        Err(e) => Json(WalletResp {
            success: false,
            message: e,
            address: None,
            public_key: None,
        }),
    }
}

pub async fn list_wallets(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let user_id = match authenticate(&headers, &state.sessions) {
        Ok(id) => id,
        Err(e) => return Json(serde_json::json!({"success": false, "message": e})),
    };

    match state.user_db.list_wallets(user_id) {
        Ok(wallets) => Json(serde_json::json!({"success": true, "wallets": wallets})),
        Err(e) => Json(serde_json::json!({"success": false, "message": e})),
    }
}

pub async fn get_activity(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let user_id = match authenticate(&headers, &state.sessions) {
        Ok(id) => id,
        Err(e) => return Json(serde_json::json!({"success": false, "message": e})),
    };

    match state.user_db.get_activity(user_id, 50) {
        Ok(entries) => Json(serde_json::json!({"success": true, "activity": entries})),
        Err(e) => Json(serde_json::json!({"success": false, "message": e})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token_length() {
        let token = generate_token();
        assert_eq!(token.len(), 128); // 64 bytes = 128 hex chars
    }

    #[test]
    fn test_generate_verification_code() {
        let code = generate_verification_code();
        assert_eq!(code.len(), 6);
        assert!(code.parse::<u32>().is_ok());
    }

    #[test]
    fn test_generate_keypair_produces_encrypted_mldsa() {
        let (address, pk_hex, encrypted_sk) = generate_keypair();
        assert!(address.starts_with("0x"));
        assert_eq!(address.len(), 66);
        assert_eq!(pk_hex.len(), 1952 * 2);
        // Encrypted key is NOT the raw hex SK — it's longer due to nonce + tag
        assert!(encrypted_sk.len() > 4000 * 2);
        // Verify roundtrip: decrypt should recover a valid SK hex
        let sk_hex = decrypt_secret_key(&encrypted_sk).unwrap();
        assert_eq!(sk_hex.len(), 4000 * 2); // 4000 bytes hex-encoded
                                            // Verify address derivation
        let pk_bytes = hex::decode(&pk_hex).unwrap();
        assert_eq!(address, generate_address_from_pubkey(&pk_bytes));
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = "deadbeef".repeat(100);
        let encrypted = encrypt_secret_key(&plaintext).unwrap();
        assert_ne!(encrypted, plaintext);
        let decrypted = decrypt_secret_key(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_tampered_fails() {
        let encrypted = encrypt_secret_key("secret_data_here").unwrap();
        let mut bytes = hex::decode(&encrypted).unwrap();
        // Tamper with the ciphertext
        if let Some(last) = bytes.last_mut() {
            *last ^= 0xFF;
        }
        let tampered = hex::encode(&bytes);
        assert!(decrypt_secret_key(&tampered).is_err());
    }

    #[test]
    fn test_session_authenticate() {
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let token = "test_token_123";
        {
            let mut s = sessions.lock().unwrap();
            s.insert(token.to_string(), (42, Instant::now()));
        }

        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        assert_eq!(authenticate(&headers, &sessions).unwrap(), 42);

        // Missing header
        let empty_headers = HeaderMap::new();
        assert!(authenticate(&empty_headers, &sessions).is_err());

        // Wrong token
        let mut bad_headers = HeaderMap::new();
        bad_headers.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(authenticate(&bad_headers, &sessions).is_err());
    }
}
