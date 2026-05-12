//! Behavior tests for the EvaporChain transaction builder.
//!
//! Validates the builder pattern, all 9 transaction types, field correctness,
//! and integration with the signer for build-then-sign workflows.

use evaporchain_crypto::signatures::{MlDsaKeypair, MlDsaVerifier, Verifier};
use evaporchain_types::*;
use evaporchain_wallet::signer::WalletSigner;
use evaporchain_wallet::tx_builder::TxBuilder;

fn sender() -> AccountAddress {
    let mut a = [0u8; 32];
    a[0] = 0xAA;
    a
}

fn recipient() -> AccountAddress {
    let mut a = [0u8; 32];
    a[0] = 0xBB;
    a
}

fn obj_id() -> ObjectId {
    let mut id = [0u8; 32];
    id[0] = 0xCC;
    id
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 1: Build-then-sign workflow for all transaction types
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn build_then_sign_all_types() {
    let kp = MlDsaKeypair::generate();
    let signer = WalletSigner::from_keypair(kp);
    let addr = *signer.address();
    let builder = TxBuilder::new(addr);

    let transactions = vec![
        builder.transfer(recipient(), 1000, 0),
        builder.create_object(obj_id(), 5000, 100, vec![0xAB; 16]),
        builder.refresh(obj_id(), 500),
        builder.deploy_contract("Token", "{}", 10000, 200),
        builder.call_contract(1, "transfer", "{}", 42),
        builder.deploy_script("fn main() {}", 8000, 150),
        builder.call_script(1, "run", "[]", 10),
        builder.validator_stake(1, 100_000, 0, None),
        builder.validator_exit(1, 3),
    ];

    for tx in &transactions {
        // Unsigned
        assert!(tx.signature().is_none());
        assert!(!tx.signable_bytes().is_empty());

        // Sign
        let signed = signer.sign_for_chain(tx, "");
        let msg = signed.signing_message("");
        let sig = signed.signature().unwrap();
        let pk = signed.public_key().unwrap();
        assert!(MlDsaVerifier::verify(&msg, sig, pk));
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 2: Transfer field correctness
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn transfer_fields_correct() {
    let builder = TxBuilder::new(sender());

    // Various amounts and nonces
    let test_cases = vec![
        (0u64, 0u64),         // zero transfer
        (1, 1),               // minimum
        (u64::MAX, u64::MAX), // maximum
        (1_000_000_000, 42),  // realistic
    ];

    for (amount, nonce) in test_cases {
        let tx = builder.transfer(recipient(), amount, nonce);
        match &tx {
            Transaction::Transfer(t) => {
                assert_eq!(t.from, sender());
                assert_eq!(t.to, recipient());
                assert_eq!(t.amount, amount);
                assert_eq!(t.nonce, nonce);
                assert!(t.signature.is_none());
                assert!(t.public_key.is_none());
            }
            _ => panic!("expected Transfer"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 3: CreateObject with various data sizes
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn create_object_various_data_sizes() {
    let builder = TxBuilder::new(sender());

    let data_sizes = vec![0, 1, 256, 1024, 64 * 1024]; // empty to 64KB

    for size in data_sizes {
        let data = vec![0xAB; size];
        let tx = builder.create_object(obj_id(), 5000, 100, data.clone());
        match &tx {
            Transaction::CreateObject(t) => {
                assert_eq!(t.creator, sender());
                assert_eq!(t.data.len(), size);
                assert_eq!(t.energy, 5000);
                assert_eq!(t.half_life, 100);
            }
            _ => panic!("expected CreateObject"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 4: Contract deployment and calls
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn deploy_and_call_contract_workflow() {
    let builder = TxBuilder::new(sender());

    // Deploy
    let deploy = builder.deploy_contract("DecayingToken", r#"{"supply":1000000}"#, 50000, 200);
    match &deploy {
        Transaction::DeployContract(t) => {
            assert_eq!(t.deployer, sender());
            assert_eq!(t.template, "DecayingToken");
            assert!(t.init_args.contains("supply"));
            assert_eq!(t.energy, 50000);
            assert_eq!(t.half_life, 200);
        }
        _ => panic!("expected DeployContract"),
    }

    // Call methods on deployed contract
    let methods = [
        ("transfer", r#"{"to":"0xbb...","amount":100}"#),
        ("approve", r#"{"spender":"0xcc...","amount":500}"#),
        ("burn", r#"{"amount":50}"#),
    ];

    for (epoch, (method, args)) in methods.iter().enumerate() {
        let call = builder.call_contract(1, method, args, epoch as u64);
        match &call {
            Transaction::CallContract(t) => {
                assert_eq!(t.caller, sender());
                assert_eq!(t.contract_id, 1);
                assert_eq!(t.method, *method);
                assert_eq!(t.epoch, epoch as u64);
            }
            _ => panic!("expected CallContract"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 5: Script deployment and execution
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn deploy_and_call_script_workflow() {
    let builder = TxBuilder::new(sender());

    let source = r#"
        fn calculate_decay(energy: u64, half_life: u64, elapsed: u64) -> u64 {
            energy >> (elapsed / half_life)
        }
    "#;

    let deploy = builder.deploy_script(source, 8000, 150);
    match &deploy {
        Transaction::DeployScript(t) => {
            assert_eq!(t.deployer, sender());
            assert!(t.source_code.contains("calculate_decay"));
        }
        _ => panic!("expected DeployScript"),
    }

    let call = builder.call_script(1, "calculate_decay", r#"[1000, 100, 200]"#, 5);
    match &call {
        Transaction::CallScript(t) => {
            assert_eq!(t.caller, sender());
            assert_eq!(t.method, "calculate_decay");
        }
        _ => panic!("expected CallScript"),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 6: Validator staking with BLS key
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn validator_stake_with_and_without_bls() {
    let builder = TxBuilder::new(sender());

    // Without BLS key
    let tx_no_bls = builder.validator_stake(1, 100_000, 0, None);
    match &tx_no_bls {
        Transaction::ValidatorStake(t) => {
            assert_eq!(t.validator_address, sender());
            assert_eq!(t.stake_amount, 100_000);
            assert!(t.bls_public_key.is_none());
        }
        _ => panic!("expected ValidatorStake"),
    }

    // With BLS key
    let bls_key = vec![0xDE; 48]; // 48-byte BLS public key
    let tx_bls = builder.validator_stake(1, 200_000, 1, Some(bls_key.clone()));
    match &tx_bls {
        Transaction::ValidatorStake(t) => {
            assert_eq!(t.stake_amount, 200_000);
            assert_eq!(t.bls_public_key.as_ref().unwrap(), &bls_key);
        }
        _ => panic!("expected ValidatorStake"),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 7: Validator exit
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn validator_exit_fields() {
    let builder = TxBuilder::new(sender());
    let tx = builder.validator_exit(42, 7);

    match &tx {
        Transaction::ValidatorExit(t) => {
            assert_eq!(t.validator_address, sender());
            assert_eq!(t.validator_id, 42);
            assert_eq!(t.nonce, 7);
            assert!(t.signature.is_none());
        }
        _ => panic!("expected ValidatorExit"),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 8: signable_bytes uniqueness — different txs produce different bytes
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn signable_bytes_uniqueness() {
    let builder = TxBuilder::new(sender());

    let tx1 = builder.transfer(recipient(), 1000, 0);
    let tx2 = builder.transfer(recipient(), 1001, 0); // different amount
    let tx3 = builder.transfer(recipient(), 1000, 1); // different nonce
    let tx4 = builder.transfer([0xCC; 32], 1000, 0); // different recipient

    let bytes1 = tx1.signable_bytes();
    let bytes2 = tx2.signable_bytes();
    let bytes3 = tx3.signable_bytes();
    let bytes4 = tx4.signable_bytes();

    // All different
    assert_ne!(
        bytes1, bytes2,
        "different amount should produce different bytes"
    );
    assert_ne!(
        bytes1, bytes3,
        "different nonce should produce different bytes"
    );
    assert_ne!(
        bytes1, bytes4,
        "different recipient should produce different bytes"
    );

    // Different tx types
    let tx_refresh = builder.refresh(obj_id(), 500);
    let tx_create = builder.create_object(obj_id(), 500, 100, vec![]);
    assert_ne!(tx_refresh.signable_bytes(), tx_create.signable_bytes());
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 9: Builder sender is correctly set on all types
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn builder_sender_propagates_to_all_types() {
    let custom_sender = {
        let mut a = [0u8; 32];
        a[0] = 0xFF;
        a[31] = 0x01;
        a
    };
    let builder = TxBuilder::new(custom_sender);

    // Check sender/deployer/caller/validator_address is correct
    match builder.transfer(recipient(), 100, 0) {
        Transaction::Transfer(t) => assert_eq!(t.from, custom_sender),
        _ => panic!(),
    }
    match builder.create_object(obj_id(), 100, 10, vec![]) {
        Transaction::CreateObject(t) => assert_eq!(t.creator, custom_sender),
        _ => panic!(),
    }
    match builder.deploy_contract("T", "{}", 100, 10) {
        Transaction::DeployContract(t) => assert_eq!(t.deployer, custom_sender),
        _ => panic!(),
    }
    match builder.call_contract(1, "m", "{}", 0) {
        Transaction::CallContract(t) => assert_eq!(t.caller, custom_sender),
        _ => panic!(),
    }
    match builder.deploy_script("s", 100, 10) {
        Transaction::DeployScript(t) => assert_eq!(t.deployer, custom_sender),
        _ => panic!(),
    }
    match builder.call_script(1, "m", "{}", 0) {
        Transaction::CallScript(t) => assert_eq!(t.caller, custom_sender),
        _ => panic!(),
    }
    match builder.validator_stake(1, 1000, 0, None) {
        Transaction::ValidatorStake(t) => assert_eq!(t.validator_address, custom_sender),
        _ => panic!(),
    }
    match builder.validator_exit(1, 0) {
        Transaction::ValidatorExit(t) => assert_eq!(t.validator_address, custom_sender),
        _ => panic!(),
    }
}
