//! WASM bridge for ML-DSA (Dilithium3) post-quantum cryptography.
//!
//! Exposes key generation, signing, and verification to the browser extension
//! via wasm-bindgen. Uses `pqc_dilithium` in mode3 (NIST security level 3)
//! which is the same `pqc_dilithium` crate used by the EvaporChain node,
//! ensuring byte-level compatibility between browser and node signatures.
//!
//! Key sizes (Dilithium3 / ML-DSA-65):
//! - Public key:  1952 bytes
//! - Secret key:  4000 bytes
//! - Signature:   3293 bytes

use pqc_dilithium::Keypair;
use sha2::{Digest, Sha256};
use wasm_bindgen::prelude::*;

/// Generate a new ML-DSA keypair.
///
/// Returns a JS object `{ publicKey: Uint8Array, secretKey: Uint8Array }`.
#[wasm_bindgen(js_name = "mlDsaKeygen")]
pub fn ml_dsa_keygen() -> Result<JsValue, JsValue> {
    let kp = Keypair::generate();

    let obj = js_sys::Object::new();
    js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("publicKey"),
        &js_sys::Uint8Array::from(&kp.public[..]).into(),
    )?;
    js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("secretKey"),
        &js_sys::Uint8Array::from(kp.expose_secret()).into(),
    )?;

    Ok(obj.into())
}

/// Sign a message with an ML-DSA secret key.
///
/// Accepts the full secret key bytes and message.
/// Returns the raw signature bytes (3293 bytes for Dilithium3).
#[wasm_bindgen(js_name = "mlDsaSign")]
pub fn ml_dsa_sign(secret_key: &[u8], message: &[u8]) -> Result<Vec<u8>, JsValue> {
    // Reconstruct keypair from secret key — we need a dummy public key
    // since pqc_dilithium's sign() only uses the secret key internally
    let kp = reconstruct_keypair(secret_key)
        .map_err(|e| JsValue::from_str(&e))?;
    let sig = kp.sign(message);
    Ok(sig.to_vec())
}

/// Verify an ML-DSA signature.
///
/// Returns `true` if the signature is valid for the given message and public key.
#[wasm_bindgen(js_name = "mlDsaVerify")]
pub fn ml_dsa_verify(message: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
    pqc_dilithium::verify(signature, message, public_key).is_ok()
}

/// Derive an EvaporChain address from a public key.
///
/// address = "0x" + hex(SHA-256(publicKey))
#[wasm_bindgen(js_name = "deriveAddress")]
pub fn derive_address(public_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key);
    let hash = hasher.finalize();
    let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
    format!("0x{}", hex)
}

/// Reconstruct a Keypair from secret key bytes.
/// The Keypair struct has private `secret` field, so we use unsafe transmutation
/// via raw bytes. The public key is embedded in the secret key for Dilithium.
fn reconstruct_keypair(sk_bytes: &[u8]) -> Result<Keypair, String> {
    use pqc_dilithium::SECRETKEYBYTES;
    if sk_bytes.len() != SECRETKEYBYTES {
        return Err(format!(
            "Invalid secret key length: expected {}, got {}",
            SECRETKEYBYTES,
            sk_bytes.len()
        ));
    }

    // Generate a throwaway keypair and overwrite with our secret key bytes.
    // This is safe because Dilithium signing only reads from the secret key.
    let mut kp = Keypair::generate();

    // SAFETY: Keypair is Copy + repr layout is { public: [u8; PK], secret: [u8; SK] }
    // We need to set the secret key. Since the field is private, we use raw pointer math.
    unsafe {
        let kp_ptr = &mut kp as *mut Keypair as *mut u8;
        let sk_offset = pqc_dilithium::PUBLICKEYBYTES;
        std::ptr::copy_nonoverlapping(sk_bytes.as_ptr(), kp_ptr.add(sk_offset), SECRETKEYBYTES);
    }

    Ok(kp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keygen_sign_verify() {
        let kp = Keypair::generate();
        let msg = b"hello evaporchain";
        let sig = kp.sign(msg);
        assert!(pqc_dilithium::verify(&sig, msg, &kp.public).is_ok());
    }

    #[test]
    fn test_wrong_message_rejects() {
        let kp = Keypair::generate();
        let sig = kp.sign(b"message A");
        assert!(pqc_dilithium::verify(&sig, b"message B", &kp.public).is_err());
    }

    #[test]
    fn test_reconstruct_and_sign() {
        let kp = Keypair::generate();
        let sk = kp.expose_secret().to_vec();
        let msg = b"reconstruct test";

        let kp2 = reconstruct_keypair(&sk).unwrap();
        let sig = kp2.sign(msg);
        assert!(pqc_dilithium::verify(&sig, msg, &kp.public).is_ok());
    }

    #[test]
    fn test_address_derivation() {
        let kp = Keypair::generate();
        let addr = derive_address(&kp.public);
        assert!(addr.starts_with("0x"));
        assert_eq!(addr.len(), 66); // "0x" + 64 hex chars
    }

    #[test]
    fn test_invalid_sk_length() {
        assert!(reconstruct_keypair(&[0u8; 10]).is_err());
    }
}
