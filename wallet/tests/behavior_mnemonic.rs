//! Behavior tests for mnemonic backup and recovery.
//!
//! Validates BIP-39 phrase generation, keypair backup/recovery round-trips,
//! multi-account backups, and recovery failure modes.

use evaporchain_crypto::signatures::{MlDsaKeypair, MlDsaVerifier, Signer, Verifier};
use evaporchain_wallet::mnemonic::{Mnemonic, MnemonicBackup, MnemonicError, MNEMONIC_WORD_COUNT};
use evaporchain_wallet::address::derive_address;

// ═══════════════════════════════════════════════════════════════════════
// Scenario 1: Full backup and recovery lifecycle
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn full_backup_recovery_lifecycle() {
    // Step 1: Generate mnemonic
    let mnemonic = Mnemonic::generate();
    let phrase = mnemonic.phrase();
    assert_eq!(mnemonic.words().len(), MNEMONIC_WORD_COUNT);

    // Step 2: Generate keypair and sign something
    let kp = MlDsaKeypair::generate();
    let original_pk = kp.public_key_bytes();
    let original_addr = derive_address(&original_pk);
    let msg = b"test message before backup";
    let sig_before = kp.sign(msg);

    // Step 3: Create backup
    let backup = mnemonic.backup_keypair(&kp).unwrap();
    assert_eq!(backup.version, 1);
    assert_eq!(backup.account_index, 0);
    assert!(!backup.encrypted_keypair.is_empty());

    // Step 4: Serialize backup to JSON (simulates writing to file)
    let backup_json = backup.to_json().unwrap();

    // Step 5: "Lose" the original — reconstruct mnemonic from phrase
    drop(kp);
    let recovered_mnemonic = Mnemonic::from_phrase(&phrase).unwrap();

    // Step 6: Load backup from JSON
    let loaded_backup = MnemonicBackup::from_json(&backup_json).unwrap();

    // Step 7: Recover keypair
    let recovered = recovered_mnemonic.recover_keypair(&loaded_backup).unwrap();
    assert_eq!(recovered.public_key_bytes(), original_pk);
    assert_eq!(derive_address(&recovered.public_key_bytes()), original_addr);

    // Step 8: Verify old signature with recovered key
    assert!(MlDsaVerifier::verify(msg, &sig_before, &recovered.public_key_bytes()));

    // Step 9: Sign new message with recovered key
    let new_msg = b"test message after recovery";
    let new_sig = recovered.sign(new_msg);
    assert!(MlDsaVerifier::verify(new_msg, &new_sig, &recovered.public_key_bytes()));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 2: Multi-account backup with different indices
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn multi_account_backup_different_indices() {
    let mnemonic = Mnemonic::generate();

    // Create 5 keypairs, backup each at a different index
    let mut backups = Vec::new();
    let mut original_pks = Vec::new();

    for i in 0..5u32 {
        let kp = MlDsaKeypair::generate();
        original_pks.push(kp.public_key_bytes());
        let backup = mnemonic.backup_keypair_at(&kp, i).unwrap();
        assert_eq!(backup.account_index, i);
        backups.push(backup);
    }

    // All encrypted keypairs are different (different data + different keys)
    for i in 0..5 {
        for j in (i + 1)..5 {
            assert_ne!(
                backups[i].encrypted_keypair,
                backups[j].encrypted_keypair,
                "backups {} and {} should differ",
                i,
                j
            );
        }
    }

    // Recover all — each matches its original
    for (i, backup) in backups.iter().enumerate() {
        let recovered = mnemonic.recover_keypair(backup).unwrap();
        assert_eq!(
            recovered.public_key_bytes(),
            original_pks[i],
            "account {} recovery mismatch",
            i
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 3: Wrong mnemonic cannot recover
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn wrong_mnemonic_fails_recovery() {
    let m1 = Mnemonic::generate();
    let m2 = Mnemonic::generate();
    let kp = MlDsaKeypair::generate();

    let backup = m1.backup_keypair(&kp).unwrap();

    // Wrong mnemonic fails
    let result = m2.recover_keypair(&backup);
    assert!(result.is_err());

    // Right mnemonic succeeds
    let recovered = m1.recover_keypair(&backup).unwrap();
    assert_eq!(recovered.public_key_bytes(), kp.public_key_bytes());
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 4: Mnemonic phrase round-trip — deterministic
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn phrase_roundtrip_deterministic() {
    let m = Mnemonic::generate();
    let phrase = m.phrase();

    // Parse and re-generate phrase multiple times
    for _ in 0..3 {
        let parsed = Mnemonic::from_phrase(&phrase).unwrap();
        assert_eq!(parsed.phrase(), phrase);
        assert_eq!(parsed.derive_seed(), m.derive_seed());
    }
}

#[test]
fn phrase_case_insensitive() {
    let m = Mnemonic::generate();
    let upper = m.phrase().to_uppercase();
    let mixed = m.words().iter().enumerate().map(|(i, w)| {
        if i % 2 == 0 { w.to_uppercase() } else { w.clone() }
    }).collect::<Vec<_>>().join(" ");

    let from_upper = Mnemonic::from_phrase(&upper).unwrap();
    let from_mixed = Mnemonic::from_phrase(&mixed).unwrap();

    assert_eq!(from_upper.phrase(), m.phrase());
    assert_eq!(from_mixed.phrase(), m.phrase());
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 5: Seed derivation properties
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn seed_derivation_properties() {
    let m1 = Mnemonic::from_entropy(&[1u8; 32]);
    let m2 = Mnemonic::from_entropy(&[2u8; 32]);

    // Different entropy → different seeds
    assert_ne!(m1.derive_seed(), m2.derive_seed());

    // Same entropy → same seeds
    let m1_clone = Mnemonic::from_entropy(&[1u8; 32]);
    assert_eq!(m1.derive_seed(), m1_clone.derive_seed());

    // derive_key_at different indices → different keys
    assert_ne!(m1.derive_key_at(0), m1.derive_key_at(1));
    assert_ne!(m1.derive_key_at(0), m1.derive_key_at(u32::MAX));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 6: Invalid phrase error handling
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn invalid_word_count() {
    assert!(matches!(
        Mnemonic::from_phrase("one two three"),
        Err(MnemonicError::InvalidWordCount { .. })
    ));

    assert!(matches!(
        Mnemonic::from_phrase(""),
        Err(MnemonicError::InvalidWordCount { .. })
    ));
}

#[test]
fn unknown_word_in_phrase() {
    let m = Mnemonic::generate();
    let mut words = m.words().to_vec();
    words[12] = "xyzzyplugh".to_string(); // not in BIP-39
    let phrase = words.join(" ");

    assert!(matches!(
        Mnemonic::from_phrase(&phrase),
        Err(MnemonicError::UnknownWord(_))
    ));
}

#[test]
fn corrupted_checksum() {
    let m = Mnemonic::generate();
    let mut words = m.words().to_vec();
    // Swap first and last word — breaks checksum
    words.swap(0, 23);
    let phrase = words.join(" ");

    // This should fail (either checksum or unknown word)
    let result = Mnemonic::from_phrase(&phrase);
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 7: Backup JSON serialization
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn backup_json_roundtrip() {
    let m = Mnemonic::generate();
    let kp = MlDsaKeypair::generate();
    let backup = m.backup_keypair(&kp).unwrap();

    let json = backup.to_json().unwrap();

    // JSON is well-formed
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["account_index"], 0);
    assert!(parsed["encrypted_keypair"].is_string());
    assert!(parsed["nonce"].is_string());
    assert!(parsed["address"].as_str().unwrap().starts_with("0x"));

    // Deserialize back and recover
    let loaded = MnemonicBackup::from_json(&json).unwrap();
    let recovered = m.recover_keypair(&loaded).unwrap();
    assert_eq!(recovered.public_key_bytes(), kp.public_key_bytes());
}
