//! Behavior tests for the EvaporChain wallet keystore.
//!
//! These tests verify end-to-end keystore workflows: create → unlock → sign → persist,
//! multi-account management, import/export, and security edge cases.

use evaporchain_crypto::signatures::{MlDsaKeypair, MlDsaVerifier, Signer, Verifier};
use evaporchain_wallet::keystore::{KeyStore, KeyStoreError};
use evaporchain_wallet::address::{derive_address, format_address, parse_address};
use std::path::PathBuf;

fn temp_keystore_path(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("evaporchain_behavior_{}_{}", std::process::id(), name))
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 1: Full wallet lifecycle — create, unlock, sign, verify, persist
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn full_wallet_lifecycle() {
    let mut store = KeyStore::new();

    // Step 1: Generate a new key
    let addr = store.generate_key("primary", "strong-password-123").unwrap();
    assert_ne!(addr, [0u8; 32]);
    assert_eq!(store.len(), 1);

    // Step 2: Unlock and get keypair
    let kp = store.unlock_key("primary", "strong-password-123").unwrap();
    assert_eq!(derive_address(&kp.public_key_bytes()), addr);

    // Step 3: Sign a message and verify
    let msg = b"EvaporChain transfer: 1000 EVR to 0xdead...";
    let sig = kp.sign(msg);
    assert!(MlDsaVerifier::verify(msg, &sig, &kp.public_key_bytes()));

    // Step 4: Persist to disk and reload
    let path = temp_keystore_path("lifecycle.json");
    store.save(&path).unwrap();

    let loaded = KeyStore::load(&path).unwrap();
    assert_eq!(loaded.len(), 1);

    // Step 5: Unlock from loaded store — same address
    let kp2 = loaded.unlock_key("primary", "strong-password-123").unwrap();
    assert_eq!(derive_address(&kp2.public_key_bytes()), addr);

    // Step 6: Sign with reloaded key — signature should verify
    let sig2 = kp2.sign(msg);
    assert!(MlDsaVerifier::verify(msg, &sig2, &kp2.public_key_bytes()));

    // Cleanup
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 2: Multi-account wallet management
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn multi_account_management() {
    let mut store = KeyStore::new();

    // Create 5 accounts
    let mut addresses = Vec::new();
    for i in 0..5 {
        let name = format!("account-{}", i);
        let pass = format!("pass-{}", i);
        let addr = store.generate_key(&name, &pass).unwrap();
        addresses.push(addr);
    }
    assert_eq!(store.len(), 5);

    // All addresses are unique
    for (i, a) in addresses.iter().enumerate() {
        for (j, b) in addresses.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "accounts {} and {} have same address", i, j);
            }
        }
    }

    // List returns all in insertion order
    let list = store.list();
    assert_eq!(list.len(), 5);
    for (i, (name, _addr_hex)) in list.iter().enumerate() {
        assert_eq!(*name, format!("account-{}", i));
    }

    // Each account unlocks with its own password only
    for i in 0..5 {
        let name = format!("account-{}", i);
        let pass = format!("pass-{}", i);
        let kp = store.unlock_key(&name, &pass).unwrap();
        assert_eq!(derive_address(&kp.public_key_bytes()), addresses[i]);

        // Wrong password fails
        let wrong = store.unlock_key(&name, "wrong");
        assert!(wrong.is_err());
    }

    // Remove middle account
    assert!(store.remove("account-2"));
    assert_eq!(store.len(), 4);
    assert!(store.unlock_key("account-2", "pass-2").is_err());

    // Remaining accounts still work
    let kp = store.unlock_key("account-3", "pass-3").unwrap();
    assert_eq!(derive_address(&kp.public_key_bytes()), addresses[3]);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 3: Import/export keypair round-trip
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn import_export_roundtrip() {
    // Generate a keypair externally
    let original_kp = MlDsaKeypair::generate();
    let pk = original_kp.public_key_bytes();
    let sk = original_kp.secret_key();
    let expected_addr = derive_address(&pk);

    // Import into keystore
    let mut store = KeyStore::new();
    let addr = store.import_key("imported", "import-pass", &pk, sk).unwrap();
    assert_eq!(addr, expected_addr);

    // Unlock and verify public key matches
    let unlocked = store.unlock_key("imported", "import-pass").unwrap();
    assert_eq!(unlocked.public_key_bytes(), pk);

    // Sign with imported key — verify with original public key
    let msg = b"cross-verification test";
    let sig = unlocked.sign(msg);
    assert!(MlDsaVerifier::verify(msg, &sig, &pk));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 4: Address derivation consistency
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn address_derivation_consistency() {
    let mut store = KeyStore::new();
    let addr = store.generate_key("addr-test", "pass").unwrap();

    // get_address matches generate_key return
    assert_eq!(store.get_address("addr-test"), Some(addr));

    // format → parse roundtrip
    let hex_addr = format_address(&addr);
    assert!(hex_addr.starts_with("0x"));
    assert_eq!(hex_addr.len(), 66); // 0x + 64 hex chars
    let parsed = parse_address(&hex_addr).unwrap();
    assert_eq!(parsed, addr);

    // unlock_by_address works
    let kp = store.unlock_by_address(&addr, "pass").unwrap();
    assert_eq!(derive_address(&kp.public_key_bytes()), addr);

    // public key → address is deterministic
    let pk = store.get_public_key("addr-test").unwrap();
    assert_eq!(derive_address(&pk), addr);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 5: Security — wrong passwords and corrupted data
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn wrong_password_always_fails() {
    let mut store = KeyStore::new();
    store.generate_key("secure", "correct-horse-battery-staple").unwrap();

    // Various wrong passwords
    let wrong_passwords = [
        "",
        " ",
        "correct-horse-battery-stapl",   // off by one
        "correct-horse-battery-staple ", // trailing space
        "Correct-Horse-Battery-Staple",  // case change
        "wrong",
        "correct-horse-battery-staple\0", // null byte
    ];

    for wrong in &wrong_passwords {
        let result = store.unlock_key("secure", wrong);
        assert!(result.is_err(), "password '{}' should have failed", wrong);
    }

    // Correct password still works
    assert!(store.unlock_key("secure", "correct-horse-battery-staple").is_ok());
}

#[test]
fn nonexistent_key_errors() {
    let store = KeyStore::new();

    assert!(matches!(
        store.unlock_key("ghost", "pass"),
        Err(KeyStoreError::NotFound(_))
    ));

    assert_eq!(store.get_address("ghost"), None);
    assert_eq!(store.get_public_key("ghost"), None);
}

#[test]
fn duplicate_name_rejected() {
    let mut store = KeyStore::new();
    store.generate_key("alice", "pass1").unwrap();

    let result = store.generate_key("alice", "pass2");
    assert!(matches!(result, Err(KeyStoreError::DuplicateName(_))));

    // Original still works
    assert!(store.unlock_key("alice", "pass1").is_ok());
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 6: Persistence across save/load cycles
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn multiple_save_load_cycles() {
    let path = temp_keystore_path("multi_cycle.json");
    let password = "cycle-pass";

    // Cycle 1: create
    {
        let mut store = KeyStore::new();
        store.generate_key("first", password).unwrap();
        store.save(&path).unwrap();
    }

    // Cycle 2: load, add, save
    {
        let mut store = KeyStore::load(&path).unwrap();
        assert_eq!(store.len(), 1);
        store.generate_key("second", password).unwrap();
        store.save(&path).unwrap();
    }

    // Cycle 3: load, remove, add, save
    {
        let mut store = KeyStore::load(&path).unwrap();
        assert_eq!(store.len(), 2);
        store.remove("first");
        store.generate_key("third", password).unwrap();
        store.save(&path).unwrap();
    }

    // Final verification
    {
        let store = KeyStore::load(&path).unwrap();
        assert_eq!(store.len(), 2);
        let names: Vec<&str> = store.list().iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"second"));
        assert!(names.contains(&"third"));
        assert!(!names.contains(&"first"));

        // Both keys unlock
        store.unlock_key("second", password).unwrap();
        store.unlock_key("third", password).unwrap();
    }

    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 7: Each key has independent encryption
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn independent_encryption_per_key() {
    let mut store = KeyStore::new();

    // Same password, different keys → different ciphertexts
    store.generate_key("a", "same-pass").unwrap();
    store.generate_key("b", "same-pass").unwrap();

    let kp_a = store.unlock_key("a", "same-pass").unwrap();
    let kp_b = store.unlock_key("b", "same-pass").unwrap();

    // Different keypairs
    assert_ne!(kp_a.public_key_bytes(), kp_b.public_key_bytes());

    // Different passwords for different keys
    let mut store2 = KeyStore::new();
    store2.generate_key("x", "pass-x").unwrap();
    store2.generate_key("y", "pass-y").unwrap();

    // Cross-password access fails
    assert!(store2.unlock_key("x", "pass-y").is_err());
    assert!(store2.unlock_key("y", "pass-x").is_err());

    // Correct passwords work
    assert!(store2.unlock_key("x", "pass-x").is_ok());
    assert!(store2.unlock_key("y", "pass-y").is_ok());
}
