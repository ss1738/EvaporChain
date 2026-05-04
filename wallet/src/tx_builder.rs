//! Transaction builder for constructing EvaporChain transactions.
//!
//! Provides a fluent API for building transactions without manually
//! constructing structs. The builder auto-fills the sender address
//! from the signer.

use evaporchain_types::{
    AccountAddress, CallContractTx, CallScriptTx, CreateObjectTx, DeployContractTx, DeployScriptTx,
    Energy, Epoch, HalfLife, ObjectId, RefreshTx, Transaction, TransferTx, ValidatorExitTx,
    ValidatorStakeTx,
};

use crate::rpc::{
    AttractorSpec, BellBeaconRequest, DemurrageOwedRequest, DsnFoldRequest,
    FeeControllerStepRequest, ForkChoiceAmendRequest, HlwaEffectiveSupplyRequest, LadAction,
    LadMode, LadSimulateRequest, PatronageActionRequest, PatronagePledgeRequest,
    SettleDemurrageRequest,
};

/// Transaction builder for constructing and optionally signing transactions.
pub struct TxBuilder {
    sender: AccountAddress,
}

impl TxBuilder {
    /// Create a new builder for the given sender address.
    pub fn new(sender: AccountAddress) -> Self {
        Self { sender }
    }

    /// Build a transfer transaction.
    pub fn transfer(&self, to: AccountAddress, amount: u64, nonce: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: self.sender,
            to,
            amount,
            nonce,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })
    }

    /// Build a create-object transaction.
    pub fn create_object(
        &self,
        object_id: ObjectId,
        energy: Energy,
        half_life: HalfLife,
        data: Vec<u8>,
    ) -> Transaction {
        Transaction::CreateObject(CreateObjectTx {
            creator: self.sender,
            object_id,
            energy,
            half_life,
            data,
            decay_curve: None,
            lad_mode: None,
            signature: None,
            public_key: None,
        })
    }

    /// Build a refresh transaction.
    pub fn refresh(&self, object_id: ObjectId, energy_deposit: Energy) -> Transaction {
        Transaction::Refresh(RefreshTx {
            object_id,
            energy_deposit,
            signature: None,
            public_key: None,
        })
    }

    /// Build a deploy-contract transaction.
    pub fn deploy_contract(
        &self,
        template: &str,
        init_args: &str,
        energy: Energy,
        half_life: HalfLife,
    ) -> Transaction {
        Transaction::DeployContract(DeployContractTx {
            deployer: self.sender,
            template: template.to_string(),
            init_args: init_args.to_string(),
            energy,
            half_life,
            rules: None,
            signature: None,
            public_key: None,
        })
    }

    /// Build a call-contract transaction.
    pub fn call_contract(
        &self,
        contract_id: u64,
        method: &str,
        args: &str,
        epoch: Epoch,
    ) -> Transaction {
        Transaction::CallContract(CallContractTx {
            caller: self.sender,
            contract_id,
            method: method.to_string(),
            args: args.to_string(),
            epoch,
            signature: None,
            public_key: None,
        })
    }

    /// Build a deploy-script transaction.
    pub fn deploy_script(
        &self,
        source_code: &str,
        energy: Energy,
        half_life: HalfLife,
    ) -> Transaction {
        Transaction::DeployScript(DeployScriptTx {
            deployer: self.sender,
            source_code: source_code.to_string(),
            energy,
            half_life,
            signature: None,
            public_key: None,
        })
    }

    /// Build a call-script transaction.
    pub fn call_script(
        &self,
        contract_id: u64,
        method: &str,
        args: &str,
        epoch: Epoch,
    ) -> Transaction {
        Transaction::CallScript(CallScriptTx {
            caller: self.sender,
            contract_id,
            method: method.to_string(),
            args: args.to_string(),
            epoch,
            signature: None,
            public_key: None,
        })
    }

    /// Build a validator stake transaction.
    pub fn validator_stake(
        &self,
        validator_id: u64,
        stake_amount: u64,
        nonce: u64,
        bls_public_key: Option<Vec<u8>>,
    ) -> Transaction {
        Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: self.sender,
            stake_amount,
            validator_id,
            nonce,
            bls_public_key,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        })
    }

    /// Build a validator exit transaction.
    pub fn validator_exit(&self, validator_id: u64, nonce: u64) -> Transaction {
        Transaction::ValidatorExit(ValidatorExitTx {
            validator_address: self.sender,
            validator_id,
            nonce,
            signature: None,
            public_key: None,
        })
    }

    // ── Substrate primitives ──────────────────────────────────────────
    //
    // These endpoints are not enum variants of [`Transaction`] — they are
    // JSON-bodied REST POSTs against the substrate API. The helpers
    // below construct the typed request bodies; for endpoints that
    // require a chain signature (only `settle_demurrage` today), see the
    // dedicated `*_signed` variant.

    // ─── Patronage covenants ────────────────────────────────────────

    /// Build a `/api/patronage/pledge` request body. The body itself
    /// carries no signature — authority is derived from the namespace
    /// pool credit at the node.
    pub fn build_patronage_pledge_request(
        object_id_hex: impl Into<String>,
        namespace_id_hex: impl Into<String>,
        donation_per_epoch: u64,
        epochs: u64,
        current_epoch: u64,
    ) -> PatronagePledgeRequest {
        PatronagePledgeRequest {
            object_id_hex: object_id_hex.into(),
            namespace_id_hex: namespace_id_hex.into(),
            donation_per_epoch,
            epochs,
            current_epoch,
        }
    }

    /// Build a shared `/api/patronage/honour` or `/api/patronage/revoke`
    /// request body (`{object_id_hex, epoch}`).
    pub fn build_patronage_action_request(
        object_id_hex: impl Into<String>,
        epoch: u64,
    ) -> PatronageActionRequest {
        PatronageActionRequest {
            object_id_hex: object_id_hex.into(),
            epoch,
        }
    }

    // ─── Governance fork-choice ─────────────────────────────────────

    /// Build a `/api/governance/fork_choice_mode` amendment body.
    /// Authorisation is by stake quorum (`endorser_stakes` summing to at
    /// least `required_stake`); no per-tx signature on the body.
    pub fn build_fork_choice_amend_request(
        mode: impl Into<String>,
        attractors: Option<Vec<AttractorSpec>>,
        endorser_stakes: Vec<u64>,
        required_stake: u64,
    ) -> ForkChoiceAmendRequest {
        ForkChoiceAmendRequest {
            mode: mode.into(),
            attractors,
            endorser_stakes,
            required_stake,
        }
    }

    // ─── Refresh pool / fee controller / demurrage ─────────────────

    /// Build a `/api/fee_controller/step` request body.
    pub fn build_fee_controller_step_request(
        gas_used: u64,
        epochs_elapsed: u64,
    ) -> FeeControllerStepRequest {
        FeeControllerStepRequest {
            gas_used,
            epochs_elapsed,
        }
    }

    /// Build a `/api/demurrage/owed` request body.
    pub fn build_demurrage_owed_request(
        balance: u64,
        last_touched_epoch: u64,
        current_epoch: u64,
        lambda_base_ppm: u64,
        threshold: u64,
    ) -> DemurrageOwedRequest {
        DemurrageOwedRequest {
            balance,
            last_touched_epoch,
            current_epoch,
            lambda_base_ppm,
            threshold,
        }
    }

    /// Compute the canonical signing payload bytes for
    /// `/api/tx/settle_demurrage`. Layout:
    /// `JSON({type:"settle_demurrage",from,current_epoch})` — exactly
    /// the byte sequence the node verifies in `post_settle_demurrage`
    /// (see `crates/evaporchain-node/src/api.rs::post_settle_demurrage`).
    pub fn settle_demurrage_signable_bytes(from: &str, current_epoch: u64) -> Vec<u8> {
        format!(
            "{{\"type\":\"settle_demurrage\",\"from\":\"{}\",\"current_epoch\":{}}}",
            from, current_epoch
        )
        .into_bytes()
    }

    /// Build an unsigned `/api/tx/settle_demurrage` request body —
    /// caller is responsible for filling in `signature` + `public_key`
    /// over [`Self::settle_demurrage_signable_bytes`].
    pub fn build_settle_demurrage_request(
        from: impl Into<String>,
        signature_hex: impl Into<String>,
        public_key_hex: impl Into<String>,
    ) -> SettleDemurrageRequest {
        SettleDemurrageRequest {
            from: from.into(),
            signature: signature_hex.into(),
            public_key: public_key_hex.into(),
        }
    }

    /// Build a fully-signed `/api/tx/settle_demurrage` body using a
    /// [`crate::signer::WalletSigner`]. Signs the canonical payload
    /// `JSON({type:"settle_demurrage",from,current_epoch})` and emits
    /// the hex `signature` / `public_key` fields the node expects.
    ///
    /// Note: `current_epoch` MUST be the chain's current epoch (read
    /// from `/api/status` or `/api/chain`) — the node re-computes the
    /// canonical payload using its own latest epoch, so a stale value
    /// here will be rejected at signature verification.
    pub fn build_settle_demurrage_signed(
        from: impl Into<String>,
        current_epoch: u64,
        signer: &crate::signer::WalletSigner,
    ) -> SettleDemurrageRequest {
        let from_str = from.into();
        let msg = Self::settle_demurrage_signable_bytes(&from_str, current_epoch);
        let sig = signer.sign_bytes(&msg);
        let pk = signer.public_key_bytes();
        SettleDemurrageRequest {
            from: from_str,
            signature: hex::encode(&sig),
            public_key: hex::encode(&pk),
        }
    }

    // ─── HLWA — Half-Life Wrapped Asset ────────────────────────────

    /// Build a shared `/api/hlwa/{effective_supply,re_attest}` body.
    pub fn build_hlwa_effective_supply_request(
        current_supply: u64,
        origin_attested_supply: u64,
        last_attested_epoch: u64,
        attestation_lambda_epochs: u64,
        current_epoch: u64,
    ) -> HlwaEffectiveSupplyRequest {
        HlwaEffectiveSupplyRequest {
            current_supply,
            origin_attested_supply,
            last_attested_epoch,
            attestation_lambda_epochs,
            current_epoch,
        }
    }

    // ─── DSN — Decay-Stamped Nullifiers ───────────────────────────

    /// Build a `/api/dsn/fold_nullifier` request body. `nullifier_hex`
    /// must be 64 hex chars (32 bytes) — the node rejects anything else.
    pub fn build_dsn_fold_request(nullifier_hex: impl Into<String>) -> DsnFoldRequest {
        DsnFoldRequest {
            nullifier_hex: nullifier_hex.into(),
        }
    }

    // ─── LAD-VM ───────────────────────────────────────────────────

    /// Build a `/api/lad_vm/simulate` request body. `decay_window` is
    /// only meaningful when `mode == LadMode::Decaying` — the node
    /// rejects Decaying mode without one.
    pub fn build_lad_simulate_request(
        mode: LadMode,
        value: u64,
        created_at_epoch: u64,
        decay_window: Option<u64>,
        current_epoch: u64,
        action: LadAction,
    ) -> LadSimulateRequest {
        LadSimulateRequest {
            mode,
            value,
            created_at_epoch,
            decay_window,
            current_epoch,
            action,
        }
    }

    // ─── Bell-Beacon ──────────────────────────────────────────────

    /// Build a `/api/bell_beacon` worked-example request body. Pass
    /// `threshold_milli = None` to use the default LOCAL_REALISM_S_MILLI
    /// (2000).
    pub fn build_bell_beacon_request(
        e_ab: i64,
        e_ab_prime: i64,
        e_a_prime_b: i64,
        e_a_prime_b_prime: i64,
        threshold_milli: Option<u64>,
    ) -> BellBeaconRequest {
        BellBeaconRequest {
            e_ab,
            e_ab_prime,
            e_a_prime_b,
            e_a_prime_b_prime,
            threshold_milli,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_build_transfer() {
        let builder = TxBuilder::new(sender());
        let tx = builder.transfer(recipient(), 1000, 5);

        match &tx {
            Transaction::Transfer(t) => {
                assert_eq!(t.from, sender());
                assert_eq!(t.to, recipient());
                assert_eq!(t.amount, 1000);
                assert_eq!(t.nonce, 5);
                assert!(t.signature.is_none());
            }
            _ => panic!("expected Transfer"),
        }
    }

    #[test]
    fn test_build_create_object() {
        let builder = TxBuilder::new(sender());
        let tx = builder.create_object(obj_id(), 5000, 100, vec![0xAB; 16]);

        match &tx {
            Transaction::CreateObject(t) => {
                assert_eq!(t.creator, sender());
                assert_eq!(t.object_id, obj_id());
                assert_eq!(t.energy, 5000);
                assert_eq!(t.half_life, 100);
                assert_eq!(t.data.len(), 16);
            }
            _ => panic!("expected CreateObject"),
        }
    }

    #[test]
    fn test_build_refresh() {
        let builder = TxBuilder::new(sender());
        let tx = builder.refresh(obj_id(), 500);

        match &tx {
            Transaction::Refresh(t) => {
                assert_eq!(t.object_id, obj_id());
                assert_eq!(t.energy_deposit, 500);
            }
            _ => panic!("expected Refresh"),
        }
    }

    #[test]
    fn test_build_deploy_contract() {
        let builder = TxBuilder::new(sender());
        let tx = builder.deploy_contract("DecayingToken", "{}", 10000, 200);

        match &tx {
            Transaction::DeployContract(t) => {
                assert_eq!(t.deployer, sender());
                assert_eq!(t.template, "DecayingToken");
                assert_eq!(t.energy, 10000);
            }
            _ => panic!("expected DeployContract"),
        }
    }

    #[test]
    fn test_build_call_contract() {
        let builder = TxBuilder::new(sender());
        let tx = builder.call_contract(1, "transfer", "{}", 42);

        match &tx {
            Transaction::CallContract(t) => {
                assert_eq!(t.caller, sender());
                assert_eq!(t.contract_id, 1);
                assert_eq!(t.method, "transfer");
                assert_eq!(t.epoch, 42);
            }
            _ => panic!("expected CallContract"),
        }
    }

    #[test]
    fn test_build_deploy_script() {
        let builder = TxBuilder::new(sender());
        let tx = builder.deploy_script("fn main() { return 42; }", 8000, 150);

        match &tx {
            Transaction::DeployScript(t) => {
                assert_eq!(t.deployer, sender());
                assert!(t.source_code.contains("main"));
            }
            _ => panic!("expected DeployScript"),
        }
    }

    #[test]
    fn test_build_validator_stake() {
        let builder = TxBuilder::new(sender());
        let tx = builder.validator_stake(1, 100_000, 0, None);

        match &tx {
            Transaction::ValidatorStake(t) => {
                assert_eq!(t.validator_address, sender());
                assert_eq!(t.stake_amount, 100_000);
                assert_eq!(t.validator_id, 1);
            }
            _ => panic!("expected ValidatorStake"),
        }
    }

    #[test]
    fn test_build_validator_exit() {
        let builder = TxBuilder::new(sender());
        let tx = builder.validator_exit(1, 3);

        match &tx {
            Transaction::ValidatorExit(t) => {
                assert_eq!(t.validator_address, sender());
                assert_eq!(t.validator_id, 1);
                assert_eq!(t.nonce, 3);
            }
            _ => panic!("expected ValidatorExit"),
        }
    }

    // ── Substrate primitive builders ──────────────────────────────

    #[test]
    fn test_build_patronage_pledge_request() {
        let req = TxBuilder::build_patronage_pledge_request("0101", "01", 100, 10, 0);
        assert_eq!(req.object_id_hex, "0101");
        assert_eq!(req.namespace_id_hex, "01");
        assert_eq!(req.donation_per_epoch, 100);
        assert_eq!(req.epochs, 10);
        assert_eq!(req.current_epoch, 0);

        // Round-trips through the same JSON shape api.rs's
        // PatronagePledgeReq deserialises.
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["object_id_hex"], "0101");
        assert_eq!(v["donation_per_epoch"], 100);
    }

    #[test]
    fn test_build_patronage_action_request() {
        let req = TxBuilder::build_patronage_action_request("0101", 5);
        assert_eq!(req.object_id_hex, "0101");
        assert_eq!(req.epoch, 5);
    }

    #[test]
    fn test_build_fork_choice_amend_request() {
        let req = TxBuilder::build_fork_choice_amend_request(
            "singh_attractor",
            Some(vec![AttractorSpec {
                center: 1000,
                basin_radius: 200,
            }]),
            vec![1000, 800],
            1500,
        );
        assert_eq!(req.mode, "singh_attractor");
        assert_eq!(req.endorser_stakes, vec![1000, 800]);
        assert_eq!(req.required_stake, 1500);
        assert_eq!(req.attractors.as_ref().unwrap()[0].center, 1000);
    }

    #[test]
    fn test_build_fee_controller_step_request() {
        let req = TxBuilder::build_fee_controller_step_request(25_000_000, 1);
        assert_eq!(req.gas_used, 25_000_000);
        assert_eq!(req.epochs_elapsed, 1);
    }

    #[test]
    fn test_build_demurrage_owed_request() {
        let req = TxBuilder::build_demurrage_owed_request(2_000_000, 0, 1000, 1, 1024);
        assert_eq!(req.balance, 2_000_000);
        assert_eq!(req.lambda_base_ppm, 1);
        assert_eq!(req.threshold, 1024);
    }

    #[test]
    fn test_settle_demurrage_signable_bytes_canonical_form() {
        // Must match api.rs::post_settle_demurrage's `format!` byte-for-
        // byte. Any change here desyncs from the node verifier.
        let bytes = TxBuilder::settle_demurrage_signable_bytes("0xabcd", 42);
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"type":"settle_demurrage","from":"0xabcd","current_epoch":42}"#
        );
    }

    #[test]
    fn test_build_settle_demurrage_request_unsigned() {
        let req = TxBuilder::build_settle_demurrage_request("0xabcd", "deadbeef", "cafe");
        assert_eq!(req.from, "0xabcd");
        assert_eq!(req.signature, "deadbeef");
        assert_eq!(req.public_key, "cafe");
    }

    #[test]
    fn test_build_settle_demurrage_signed_round_trips() {
        use crate::signer::WalletSigner;
        use evaporchain_crypto::signatures::{HybridVerifier, MlDsaKeypair, Verifier};

        let signer = WalletSigner::from_keypair(MlDsaKeypair::generate());
        let from_hex = format!("0x{}", hex::encode(signer.address()));
        let req = TxBuilder::build_settle_demurrage_signed(&from_hex, 42, &signer);

        // Same canonical bytes the node will hash/verify.
        let canonical = TxBuilder::settle_demurrage_signable_bytes(&from_hex, 42);
        let sig = hex::decode(&req.signature).unwrap();
        let pk = hex::decode(&req.public_key).unwrap();
        assert!(HybridVerifier::verify(&canonical, &sig, &pk));
    }

    #[test]
    fn test_build_hlwa_effective_supply_request() {
        let req =
            TxBuilder::build_hlwa_effective_supply_request(1_000_000, 1_000_000, 0, 500, 100);
        assert_eq!(req.attestation_lambda_epochs, 500);
        assert_eq!(req.current_epoch, 100);
    }

    #[test]
    fn test_build_dsn_fold_request() {
        let req = TxBuilder::build_dsn_fold_request("00".repeat(31) + "01");
        assert_eq!(req.nullifier_hex.len(), 64);
    }

    #[test]
    fn test_build_lad_simulate_request_decaying_carries_window() {
        let req = TxBuilder::build_lad_simulate_request(
            LadMode::Decaying,
            42,
            0,
            Some(10),
            15,
            LadAction::Use,
        );
        assert_eq!(req.mode, LadMode::Decaying);
        assert_eq!(req.action, LadAction::Use);
        assert_eq!(req.decay_window, Some(10));

        // Lowercase serde rename — must round-trip through api.rs's
        // serde(rename_all = "lowercase") on LadModeReq / LadActionReq.
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["mode"], "decaying");
        assert_eq!(v["action"], "use");
    }

    #[test]
    fn test_build_bell_beacon_request_default_threshold() {
        let req = TxBuilder::build_bell_beacon_request(500, -500, 500, 500, None);
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["e_ab"], 500);
        assert!(v.get("threshold_milli").is_none(), "None must be skipped");
    }

    #[test]
    fn test_all_unsigned_transactions() {
        let builder = TxBuilder::new(sender());

        // All txs should start with no signature
        let txs = vec![
            builder.transfer(recipient(), 100, 0),
            builder.create_object(obj_id(), 1000, 50, vec![]),
            builder.refresh(obj_id(), 200),
            builder.deploy_contract("MortalNFT", "{}", 5000, 100),
            builder.call_contract(1, "bid", "{}", 10),
            builder.deploy_script("fn f() {}", 3000, 80),
            builder.call_script(1, "f", "[]", 10),
            builder.validator_stake(1, 50000, 0, None),
            builder.validator_exit(1, 0),
        ];

        for tx in &txs {
            assert!(tx.signature().is_none(), "tx should be unsigned");
            assert!(tx.public_key().is_none(), "tx should have no pubkey");
            // signable_bytes should produce non-empty output
            assert!(!tx.signable_bytes().is_empty());
        }
    }
}
