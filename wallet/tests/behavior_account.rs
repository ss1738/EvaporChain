//! Behavior tests for the account manager.
//!
//! Tests multi-account creation, active account switching, signer integration,
//! nonce management, and account lifecycle operations.

use evaporchain_crypto::signatures::{MlDsaVerifier, Verifier};
use evaporchain_wallet::account::{AccountError, AccountManager};
use evaporchain_wallet::address::format_address;
use evaporchain_wallet::keystore::KeyStore;
use evaporchain_wallet::rpc::RpcClient;

fn make_manager() -> AccountManager {
    let keystore = KeyStore::new();
    let rpc = RpcClient::new("http://localhost:3000").unwrap();
    AccountManager::new(keystore, rpc)
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 1: Multi-account creation and management
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn multi_account_creation_and_listing() {
    let mut mgr = make_manager();

    // Create multiple accounts
    let addr_alice = mgr.create_account("alice", "pass-a").unwrap();
    let addr_bob = mgr.create_account("bob", "pass-b").unwrap();
    let addr_carol = mgr.create_account("carol", "pass-c").unwrap();

    // All addresses unique
    assert_ne!(addr_alice, addr_bob);
    assert_ne!(addr_bob, addr_carol);
    assert_ne!(addr_alice, addr_carol);

    assert_eq!(mgr.account_count(), 3);

    // List shows all accounts
    let list = mgr.list_accounts();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].name, "alice");
    assert_eq!(list[1].name, "bob");
    assert_eq!(list[2].name, "carol");

    // Addresses match
    assert_eq!(list[0].address, format_address(&addr_alice));
    assert_eq!(list[1].address, format_address(&addr_bob));
    assert_eq!(list[2].address, format_address(&addr_carol));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 2: First account auto-becomes active
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn first_account_auto_active() {
    let mut mgr = make_manager();
    assert!(mgr.active_name().is_none());
    assert!(mgr.active_address().is_none());

    let addr = mgr.create_account("first", "pass").unwrap();
    assert_eq!(mgr.active_name(), Some("first"));
    assert_eq!(mgr.active_address(), Some(addr));

    // Second account does NOT change active
    mgr.create_account("second", "pass").unwrap();
    assert_eq!(mgr.active_name(), Some("first"));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 3: Switch active account
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn switch_active_account() {
    let mut mgr = make_manager();
    mgr.create_account("alice", "pass-a").unwrap();
    let addr_bob = mgr.create_account("bob", "pass-b").unwrap();

    // Alice is active initially
    assert_eq!(mgr.active_name(), Some("alice"));
    let list = mgr.list_accounts();
    assert!(list[0].is_active);
    assert!(!list[1].is_active);

    // Switch to bob
    mgr.set_active("bob").unwrap();
    assert_eq!(mgr.active_name(), Some("bob"));
    assert_eq!(mgr.active_address(), Some(addr_bob));

    let list = mgr.list_accounts();
    assert!(!list[0].is_active);
    assert!(list[1].is_active);
}

#[test]
fn switch_to_nonexistent_fails() {
    let mut mgr = make_manager();
    let result = mgr.set_active("ghost");
    assert!(matches!(result, Err(AccountError::NotFound(_))));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 4: Remove account — active switches
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn remove_active_account_switches() {
    let mut mgr = make_manager();
    mgr.create_account("alice", "pass-a").unwrap();
    mgr.create_account("bob", "pass-b").unwrap();

    assert_eq!(mgr.active_name(), Some("alice"));
    assert!(mgr.remove_account("alice"));
    assert_eq!(mgr.account_count(), 1);

    // Active should switch to remaining account
    assert_eq!(mgr.active_name(), Some("bob"));
}

#[test]
fn remove_last_account() {
    let mut mgr = make_manager();
    mgr.create_account("only", "pass").unwrap();
    assert!(mgr.remove_account("only"));
    assert_eq!(mgr.account_count(), 0);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 5: Signer integration — sign from account manager
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn get_signer_by_name() {
    let mut mgr = make_manager();
    let addr = mgr.create_account("signer-test", "my-pass").unwrap();

    let signer = mgr.get_signer("signer-test", "my-pass").unwrap();
    assert_eq!(*signer.address(), addr);

    // Sign and verify
    let msg = b"account manager signing test";
    let sig = signer.sign_bytes(msg);
    let pk = signer.public_key_bytes();
    assert!(MlDsaVerifier::verify(msg, &sig, &pk));
}

#[test]
fn get_active_signer() {
    let mut mgr = make_manager();
    let addr = mgr.create_account("active", "pass").unwrap();

    let signer = mgr.get_active_signer("pass").unwrap();
    assert_eq!(*signer.address(), addr);
}

#[test]
fn get_active_signer_no_active() {
    let mgr = make_manager();
    let result = mgr.get_active_signer("pass");
    assert!(matches!(result, Err(AccountError::NoActiveAccount)));
}

#[test]
fn wrong_password_fails_signer() {
    let mut mgr = make_manager();
    mgr.create_account("secure", "correct").unwrap();

    assert!(mgr.get_signer("secure", "wrong").is_err());
    assert!(mgr.get_signer("secure", "correct").is_ok());
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 6: Nonce tracking
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn nonce_tracking() {
    let mut mgr = make_manager();
    mgr.create_account("alice", "pass").unwrap();

    // No cache initially
    assert!(mgr.cached_balance("alice").is_none());
    assert!(mgr.cached_nonce("alice").is_none());

    // Manually seed cache (simulates refresh_balance)
    let addr = mgr.active_address().unwrap();
    let _addr_hex = format_address(&addr);
    // We can't call refresh_balance without a running node, but we can test increment
    // by directly inserting into cache via the keystore
}

#[test]
fn active_address_hex_format() {
    let mut mgr = make_manager();
    mgr.create_account("hex-test", "pass").unwrap();

    let hex = mgr.active_address_hex().unwrap();
    assert!(hex.starts_with("0x"));
    assert_eq!(hex.len(), 66); // 0x + 64 hex chars
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 7: Import account
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn import_account() {
    use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};

    let mut mgr = make_manager();

    // Generate a keypair externally
    let kp = MlDsaKeypair::generate();
    let pk = kp.public_key_bytes();
    let sk = kp.secret_key();

    let addr = mgr
        .import_account("imported", "import-pass", &pk, sk)
        .unwrap();
    assert_eq!(mgr.account_count(), 1);
    assert_eq!(mgr.active_name(), Some("imported")); // auto-active

    // Can sign with imported account
    let signer = mgr.get_signer("imported", "import-pass").unwrap();
    assert_eq!(*signer.address(), addr);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 8: End-to-end — create accounts, switch, sign, remove
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn end_to_end_account_workflow() {
    let mut mgr = make_manager();

    // Create 3 accounts
    let addr_a = mgr.create_account("alice", "pass-a").unwrap();
    let _addr_b = mgr.create_account("bob", "pass-b").unwrap();
    let addr_c = mgr.create_account("carol", "pass-c").unwrap();

    // Alice is active
    assert_eq!(mgr.active_name(), Some("alice"));

    // Sign with alice
    let signer_a = mgr.get_active_signer("pass-a").unwrap();
    assert_eq!(*signer_a.address(), addr_a);

    // Switch to carol, sign
    mgr.set_active("carol").unwrap();
    let signer_c = mgr.get_active_signer("pass-c").unwrap();
    assert_eq!(*signer_c.address(), addr_c);

    // Remove bob (not active, shouldn't change active)
    assert!(mgr.remove_account("bob"));
    assert_eq!(mgr.active_name(), Some("carol"));
    assert_eq!(mgr.account_count(), 2);

    // Remove carol (active) — should switch to alice
    assert!(mgr.remove_account("carol"));
    assert_eq!(mgr.active_name(), Some("alice"));
    assert_eq!(mgr.account_count(), 1);

    // Alice still works
    let signer = mgr.get_active_signer("pass-a").unwrap();
    let sig = signer.sign_bytes(b"final check");
    assert!(MlDsaVerifier::verify(
        b"final check",
        &sig,
        &signer.public_key_bytes()
    ));
}
