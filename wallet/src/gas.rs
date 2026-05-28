//! Client-side gas and fee estimation.
//!
//! Mirrors the node's gas constants and PID-based fee controller to give
//! users accurate cost predictions before submitting transactions.
//!
//! # Fee Components
//!
//! ```text
//! total_fee = gas_fee + extra_fee
//! gas_fee   = base_fee × gas_used
//! extra_fee = creation_deposit | refresh_fee | resurrection_fee | 0
//! ```

use evaporchain_types::Transaction;

// ──────────────────────────── Gas Constants ───────────────────────────
// Must stay in sync with evaporchain-execution/src/lib.rs

/// Base gas cost for a transfer.
pub const GAS_TRANSFER: u64 = 21_000;
/// Base gas cost for creating an object.
pub const GAS_CREATE_OBJECT_BASE: u64 = 50_000;
/// Additional gas per byte of object data.
pub const GAS_CREATE_OBJECT_PER_BYTE: u64 = 200;
/// Gas cost for refreshing an object.
pub const GAS_REFRESH: u64 = 30_000;
/// Gas cost for deploying a contract.
pub const GAS_DEPLOY_CONTRACT: u64 = 100_000;
/// Gas cost for calling a contract.
pub const GAS_CALL_CONTRACT: u64 = 40_000;
/// Gas cost for deploying a script.
pub const GAS_DEPLOY_SCRIPT: u64 = 150_000;
/// Gas cost for calling a script.
pub const GAS_CALL_SCRIPT: u64 = 50_000;
/// Gas cost for validator staking.
pub const GAS_VALIDATOR_STAKE: u64 = 50_000;
/// Gas cost for validator exit.
pub const GAS_VALIDATOR_EXIT: u64 = 30_000;

// ──────────────────────────── Deposit Constants ───────────────────────

/// Units of deposit per byte of data for object creation.
const DEPOSIT_PER_BYTE: u64 = 100;
/// Minimum creation deposit.
const MIN_CREATION_DEPOSIT: u64 = 1_000;
/// Refresh fee as fraction of equivalent creation deposit (20%).
const REFRESH_FEE_RATIO: f64 = 0.20;
/// Resurrection fee as fraction of equivalent creation deposit (60%).
const RESURRECTION_FEE_RATIO: f64 = 0.60;

// ──────────────────────────── Fee Estimate ────────────────────────────

/// Breakdown of estimated fees for a transaction.
#[derive(Debug, Clone)]
pub struct FeeEstimate {
    /// Gas units this transaction will consume.
    pub gas_used: u64,
    /// Current base fee per gas unit (from latest block).
    pub base_fee: u64,
    /// Gas fee = base_fee × gas_used.
    pub gas_fee: u64,
    /// Extra fee (creation deposit, refresh fee, etc.).
    pub extra_fee: u64,
    /// Total fee = gas_fee + extra_fee.
    pub total_fee: u64,
    /// Human-readable description of fee components.
    pub breakdown: String,
}

// ──────────────────────────── Gas Estimator ───────────────────────────

/// Estimates gas and fees for transactions.
pub struct GasEstimator {
    base_fee: u64,
}

impl GasEstimator {
    /// Create an estimator with the current base fee from the latest block.
    pub fn new(base_fee: u64) -> Self {
        Self { base_fee }
    }

    /// Create an estimator by fetching the latest block's base fee.
    pub async fn from_rpc(rpc: &crate::rpc::RpcClient) -> Result<Self, crate::rpc::RpcError> {
        let block = rpc.get_latest_block().await?;
        Ok(Self {
            base_fee: block.base_fee,
        })
    }

    /// Estimate the gas for a given transaction.
    pub fn estimate_gas(&self, tx: &Transaction) -> u64 {
        match tx {
            Transaction::Transfer(_) => GAS_TRANSFER,
            Transaction::CreateObject(tx) => {
                GAS_CREATE_OBJECT_BASE + (tx.data.len() as u64) * GAS_CREATE_OBJECT_PER_BYTE
            }
            Transaction::Refresh(_) => GAS_REFRESH,
            Transaction::DeployContract(_) => GAS_DEPLOY_CONTRACT,
            Transaction::CallContract(_) => GAS_CALL_CONTRACT,
            Transaction::DeployScript(_) => GAS_DEPLOY_SCRIPT,
            Transaction::CallScript(_) => GAS_CALL_CONTRACT, // same cost as CallContract
            Transaction::ValidatorStake(_) => GAS_VALIDATOR_STAKE,
            Transaction::ValidatorExit(_) => GAS_VALIDATOR_EXIT,
            Transaction::ValidatorClaimStake(_) => GAS_VALIDATOR_EXIT,
            Transaction::Shield(_) => 60_000,
            Transaction::Unshield(_) => 80_000,
            Transaction::PrivateTransfer(ptx) => {
                100_000
                    + 20_000 * ptx.input_nullifiers.len() as u64
                    + 15_000 * ptx.output_commitments.len() as u64
            }
            Transaction::Deferred(dtx) => 75_000 + 5_000 * dtx.guards.len() as u64,
            Transaction::Blob(tx) => 50_000 + 10 * tx.data.len() as u64,
            Transaction::Governance(_) => 25_000,
            Transaction::MultiSig(tx) => 50_000 + 10_000 * tx.signatures.len() as u64,
            Transaction::UserOp(tx) => 30_000 + 16 * tx.call_data.len() as u64,
            Transaction::UpgradeContract(tx) => 100_000 + 200 * tx.new_bytecode.len() as u64,
            Transaction::Delegate(_) => 40_000,
            Transaction::Undelegate(_) => 40_000,
            Transaction::RotateValidatorKey(_) => 60_000,
            Transaction::ClaimDelegation(_) => 30_000,
            // Lane R.7: Refund variant exists in evaporchain-types
            // since 27bfab9 (Crooks-MEV Phase 3.1) but the wallet
            // gas estimator's match was never updated. Refund txs
            // are protocol-issued and cost the same as Transfer.
            Transaction::Refund(_) => GAS_TRANSFER,
            // DeployTemplate landed in evaporchain-types but the
            // wallet gas estimator's match was never updated —
            // unblocks `cargo build --workspace`. Cost mirrors
            // `evaporchain-consensus/src/mempool.rs::estimate_tx_size`:
            // base 50_000 + 50 per param byte.
            Transaction::DeployTemplate(tx) => 50_000 + 50 * tx.params.len() as u64,
        }
    }

    /// Compute the creation deposit for an object of `data_size` bytes.
    pub fn creation_deposit(&self, data_size: usize) -> u64 {
        let deposit = (data_size as u64) * DEPOSIT_PER_BYTE;
        deposit.max(MIN_CREATION_DEPOSIT)
    }

    /// Compute the refresh fee for a given energy deposit.
    pub fn refresh_fee(&self, energy_deposited: u64) -> u64 {
        (energy_deposited as f64 * REFRESH_FEE_RATIO) as u64
    }

    /// Compute the resurrection fee for an object of `data_size` bytes.
    pub fn resurrection_fee(&self, data_size: usize) -> u64 {
        let deposit = self.creation_deposit(data_size);
        (deposit as f64 * RESURRECTION_FEE_RATIO) as u64
    }

    /// Full fee estimate for a transaction.
    pub fn estimate(&self, tx: &Transaction) -> FeeEstimate {
        let gas_used = self.estimate_gas(tx);
        let gas_fee = self.base_fee * gas_used;

        let (extra_fee, extra_desc) = match tx {
            Transaction::CreateObject(tx) => {
                let deposit = self.creation_deposit(tx.data.len());
                (
                    deposit,
                    format!("creation deposit ({} bytes)", tx.data.len()),
                )
            }
            Transaction::Refresh(tx) => {
                let fee = self.refresh_fee(tx.energy_deposit);
                (
                    fee,
                    format!("refresh fee ({}E deposited)", tx.energy_deposit),
                )
            }
            _ => (0, String::new()),
        };

        let total_fee = gas_fee + extra_fee;
        let breakdown = if extra_fee > 0 {
            format!(
                "gas: {} × {} = {} + {} ({}) = {} total",
                self.base_fee, gas_used, gas_fee, extra_fee, extra_desc, total_fee
            )
        } else {
            format!(
                "gas: {} × {} = {} total",
                self.base_fee, gas_used, total_fee
            )
        };

        FeeEstimate {
            gas_used,
            base_fee: self.base_fee,
            gas_fee,
            extra_fee,
            total_fee,
            breakdown,
        }
    }

    /// Convenience: estimate fee for a transfer.
    pub fn estimate_transfer(&self) -> FeeEstimate {
        let gas_fee = self.base_fee * GAS_TRANSFER;
        FeeEstimate {
            gas_used: GAS_TRANSFER,
            base_fee: self.base_fee,
            gas_fee,
            extra_fee: 0,
            total_fee: gas_fee,
            breakdown: format!("gas: {} × {} = {}", self.base_fee, GAS_TRANSFER, gas_fee),
        }
    }

    /// Convenience: estimate fee for object creation.
    pub fn estimate_create_object(&self, data_size: usize) -> FeeEstimate {
        let gas_used = GAS_CREATE_OBJECT_BASE + (data_size as u64) * GAS_CREATE_OBJECT_PER_BYTE;
        let gas_fee = self.base_fee * gas_used;
        let deposit = self.creation_deposit(data_size);
        let total = gas_fee + deposit;
        FeeEstimate {
            gas_used,
            base_fee: self.base_fee,
            gas_fee,
            extra_fee: deposit,
            total_fee: total,
            breakdown: format!(
                "gas: {} × {} = {} + deposit {} = {}",
                self.base_fee, gas_used, gas_fee, deposit, total
            ),
        }
    }

    /// Get the current base fee.
    pub fn base_fee(&self) -> u64 {
        self.base_fee
    }

    /// Update the base fee (e.g., after fetching a new block).
    pub fn set_base_fee(&mut self, base_fee: u64) {
        self.base_fee = base_fee;
    }
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn estimator() -> GasEstimator {
        GasEstimator::new(100) // base_fee = 100
    }

    #[test]
    fn test_transfer_gas() {
        let e = estimator();
        let est = e.estimate_transfer();
        assert_eq!(est.gas_used, 21_000);
        assert_eq!(est.gas_fee, 100 * 21_000);
        assert_eq!(est.extra_fee, 0);
        assert_eq!(est.total_fee, 2_100_000);
    }

    #[test]
    fn test_create_object_gas_scales_with_data() {
        let e = estimator();
        let small = e.estimate_create_object(10);
        let big = e.estimate_create_object(1000);
        assert!(big.gas_used > small.gas_used);
        assert!(big.total_fee > small.total_fee);
    }

    #[test]
    fn test_create_object_has_deposit() {
        let e = estimator();
        let est = e.estimate_create_object(100);
        assert_eq!(est.extra_fee, 100 * 100); // 100 bytes × 100 per byte
        assert!(est.total_fee > est.gas_fee);
    }

    #[test]
    fn test_min_creation_deposit() {
        let e = estimator();
        // 5 bytes × 100 = 500 < min 1000
        assert_eq!(e.creation_deposit(5), 1_000);
        // 20 bytes × 100 = 2000 > min
        assert_eq!(e.creation_deposit(20), 2_000);
    }

    #[test]
    fn test_refresh_fee() {
        let e = estimator();
        assert_eq!(e.refresh_fee(10_000), 2_000); // 20% of 10,000
    }

    #[test]
    fn test_resurrection_fee() {
        let e = estimator();
        // 100 bytes → deposit = 10,000 → resurrection = 60% = 6,000
        assert_eq!(e.resurrection_fee(100), 6_000);
    }

    #[test]
    fn test_estimate_transfer_tx() {
        use evaporchain_types::TransferTx;
        let e = estimator();
        let tx = Transaction::Transfer(TransferTx {
            from: [0u8; 32],
            to: [1u8; 32],
            amount: 1000,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let est = e.estimate(&tx);
        assert_eq!(est.gas_used, GAS_TRANSFER);
        assert_eq!(est.extra_fee, 0);
    }

    #[test]
    fn test_estimate_create_object_tx() {
        use evaporchain_types::CreateObjectTx;
        let e = estimator();
        let tx = Transaction::CreateObject(CreateObjectTx {
            creator: [0u8; 32],
            object_id: [0u8; 32],
            energy: 100,
            half_life: 10,
            data: vec![0u8; 50],
            decay_curve: None,
            lad_mode: None,
            signature: None,
            public_key: None,
        });
        let est = e.estimate(&tx);
        assert_eq!(
            est.gas_used,
            GAS_CREATE_OBJECT_BASE + 50 * GAS_CREATE_OBJECT_PER_BYTE
        );
        assert_eq!(est.extra_fee, 50 * 100); // creation deposit
    }

    #[test]
    fn test_estimate_refresh_tx() {
        use evaporchain_types::RefreshTx;
        let e = estimator();
        let tx = Transaction::Refresh(RefreshTx {
            object_id: [0u8; 32],
            energy_deposit: 5000,
            signature: None,
            public_key: None,
        });
        let est = e.estimate(&tx);
        assert_eq!(est.gas_used, GAS_REFRESH);
        assert_eq!(est.extra_fee, 1000); // 20% of 5000
    }

    #[test]
    fn test_zero_base_fee() {
        let e = GasEstimator::new(0);
        let est = e.estimate_transfer();
        assert_eq!(est.gas_fee, 0);
        assert_eq!(est.total_fee, 0);
    }

    #[test]
    fn test_set_base_fee() {
        let mut e = GasEstimator::new(100);
        assert_eq!(e.base_fee(), 100);
        e.set_base_fee(200);
        assert_eq!(e.base_fee(), 200);
        let est = e.estimate_transfer();
        assert_eq!(est.gas_fee, 200 * 21_000);
    }

    #[test]
    fn test_breakdown_string_non_empty() {
        let e = estimator();
        let est = e.estimate_transfer();
        assert!(!est.breakdown.is_empty());
        assert!(est.breakdown.contains("gas"));
    }

    // ─── Additional coverage tests (session 63): all estimate_gas() arms ────

    #[test]
    fn test_estimate_gas_constant_variants() {
        use evaporchain_types::{
            CallContractTx, CallScriptTx, ClaimDelegationTx, DelegateTx, DeployContractTx,
            DeployScriptTx, GovernanceAction, GovernanceTx, RefundTx, RotateValidatorKeyTx,
            UndelegateTx, ValidatorClaimStakeTx, ValidatorExitTx, ValidatorStakeTx,
        };
        let e = estimator();

        assert_eq!(
            e.estimate_gas(&Transaction::DeployContract(DeployContractTx {
                deployer: [0u8; 32],
                template: "DecayingToken".into(),
                init_args: "{}".into(),
                energy: 1000,
                half_life: 100,
                rules: None,
                signature: None,
                public_key: None,
            })),
            GAS_DEPLOY_CONTRACT
        );

        assert_eq!(
            e.estimate_gas(&Transaction::CallContract(CallContractTx {
                caller: [0u8; 32],
                contract_id: 1,
                method: "transfer".into(),
                args: "{}".into(),
                epoch: 0,
                signature: None,
                public_key: None,
            })),
            GAS_CALL_CONTRACT
        );

        assert_eq!(
            e.estimate_gas(&Transaction::DeployScript(DeployScriptTx {
                deployer: [0u8; 32],
                source_code: "let x = 1;".into(),
                energy: 500,
                half_life: 50,
                signature: None,
                public_key: None,
            })),
            GAS_DEPLOY_SCRIPT
        );

        assert_eq!(
            e.estimate_gas(&Transaction::CallScript(CallScriptTx {
                caller: [0u8; 32],
                contract_id: 2,
                method: "run".into(),
                args: "{}".into(),
                epoch: 0,
                signature: None,
                public_key: None,
            })),
            GAS_CALL_CONTRACT
        );

        assert_eq!(
            e.estimate_gas(&Transaction::ValidatorStake(ValidatorStakeTx {
                validator_address: [0u8; 32],
                stake_amount: 1_000_000,
                validator_id: 1,
                nonce: 0,
                bls_public_key: None,
                vrf_public_key: None,
                signature: None,
                public_key: None,
            })),
            GAS_VALIDATOR_STAKE
        );

        assert_eq!(
            e.estimate_gas(&Transaction::ValidatorExit(ValidatorExitTx {
                validator_address: [0u8; 32],
                validator_id: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })),
            GAS_VALIDATOR_EXIT
        );

        assert_eq!(
            e.estimate_gas(&Transaction::ValidatorClaimStake(ValidatorClaimStakeTx {
                validator_address: [0u8; 32],
                validator_id: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })),
            GAS_VALIDATOR_EXIT
        );

        assert_eq!(
            e.estimate_gas(&Transaction::Governance(GovernanceTx {
                action: GovernanceAction::CastVote {
                    proposal_id: 1,
                    vote: true,
                },
                sender: [0u8; 32],
                nonce: 0,
                signature: None,
                public_key: None,
            })),
            25_000
        );

        assert_eq!(
            e.estimate_gas(&Transaction::Delegate(DelegateTx {
                delegator: [0u8; 32],
                validator_id: 1,
                amount: 500,
                nonce: 0,
                signature: None,
                public_key: None,
            })),
            40_000
        );

        assert_eq!(
            e.estimate_gas(&Transaction::Undelegate(UndelegateTx {
                delegator: [0u8; 32],
                validator_id: 1,
                amount: 500,
                nonce: 0,
                signature: None,
                public_key: None,
            })),
            40_000
        );

        assert_eq!(
            e.estimate_gas(&Transaction::RotateValidatorKey(RotateValidatorKeyTx {
                validator_address: [0u8; 32],
                validator_id: 1,
                new_bls_public_key: vec![0u8; 48],
                bls_pop_old: vec![0u8; 96],
                bls_pop_new: vec![0u8; 96],
                effective_epoch: 10,
                nonce: 0,
                signature: None,
                public_key: None,
            })),
            60_000
        );

        assert_eq!(
            e.estimate_gas(&Transaction::ClaimDelegation(ClaimDelegationTx {
                delegator: [0u8; 32],
                validator_id: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })),
            30_000
        );

        assert_eq!(
            e.estimate_gas(&Transaction::Refund(RefundTx {
                source_block_height: 100,
                source_observation_idx: 0,
                attacker: [0u8; 32],
                victim: [1u8; 32],
                amount: 1000,
                settle_block_height: 200,
            })),
            GAS_TRANSFER
        );
    }

    #[test]
    fn test_estimate_gas_size_dependent_variants() {
        use evaporchain_types::{
            BlobTx, DeferredTx, DeployTemplateTx, MultiSigTx, PrivateTransferTx, ShieldTx,
            UnshieldTx, UpgradeContractTx, UserOpTx,
        };
        let e = estimator();

        // Shield → 60_000
        assert_eq!(
            e.estimate_gas(&Transaction::Shield(ShieldTx {
                from: [0u8; 32],
                amount: 1000,
                nonce: 0,
                note_owner_hash: [0u8; 32],
                value_blinding: [0u8; 32],
                energy: None,
                energy_blinding: None,
                half_life: 0,
                signature: None,
                public_key: None,
            })),
            60_000
        );

        // Unshield → 80_000
        assert_eq!(
            e.estimate_gas(&Transaction::Unshield(UnshieldTx {
                to: [0u8; 32],
                amount: 500,
                input_nullifiers: vec![],
                anchor: [0u8; 32],
                balance_binding: [0u8; 32],
                input_amounts: vec![],
                input_blindings: vec![],
                input_value_commitments: vec![],
                input_note_commitments: vec![],
                input_merkle_proofs: vec![],
                output_blindings: vec![],
                change_commitments: vec![],
                energy_proofs: vec![],
            })),
            80_000
        );

        // PrivateTransfer with 2 nullifiers + 1 commitment
        assert_eq!(
            e.estimate_gas(&Transaction::PrivateTransfer(PrivateTransferTx {
                input_nullifiers: vec![[0u8; 32], [1u8; 32]],
                output_commitments: vec![[2u8; 32]],
                anchor: [0u8; 32],
                balance_binding: [0u8; 32],
                fee: 0,
                input_amounts: vec![],
                input_blindings: vec![],
                input_value_commitments: vec![],
                input_note_commitments: vec![],
                input_merkle_proofs: vec![],
                output_amounts: vec![],
                output_blindings: vec![],
                energy_proofs: vec![],
            })),
            100_000 + 20_000 * 2 + 15_000 * 1
        );

        // Deferred with 3 guards
        assert_eq!(
            e.estimate_gas(&Transaction::Deferred(DeferredTx {
                submitter: [0u8; 32],
                nonce: 0,
                deposit: 0,
                guards: vec![
                    evaporchain_types::TemporalGuard::AfterEpoch(10),
                    evaporchain_types::TemporalGuard::AfterEpoch(20),
                    evaporchain_types::TemporalGuard::AfterEpoch(30),
                ],
                inner_tx_bytes: vec![],
                gas_limit: 0,
                signature: None,
                public_key: None,
            })),
            75_000 + 5_000 * 3
        );

        // Blob: 20 bytes
        assert_eq!(
            e.estimate_gas(&Transaction::Blob(BlobTx {
                submitter: [0u8; 32],
                data: vec![0u8; 20],
                nonce: 0,
                namespace_id: 0,
                signature: None,
                public_key: None,
            })),
            50_000 + 10 * 20
        );

        // MultiSig with 2 signatures
        assert_eq!(
            e.estimate_gas(&Transaction::MultiSig(MultiSigTx {
                multisig_address: [0u8; 32],
                threshold: 2,
                signers: vec![],
                inner_tx_bytes: vec![],
                signatures: vec![([0u8; 32], vec![]), ([1u8; 32], vec![]),],
                public_keys: vec![],
                nonce: 0,
            })),
            50_000 + 10_000 * 2
        );

        // UserOp: 10 bytes call_data
        assert_eq!(
            e.estimate_gas(&Transaction::UserOp(UserOpTx {
                sender: [0u8; 32],
                nonce: 0,
                call_data: vec![0u8; 10],
                call_gas_limit: 100_000,
                paymaster: None,
                paymaster_nonce: None,
                paymaster_data: None,
                paymaster_signature: None,
                paymaster_public_key: None,
                signature: None,
                public_key: None,
            })),
            30_000 + 16 * 10
        );

        // UpgradeContract: 5 bytes bytecode
        assert_eq!(
            e.estimate_gas(&Transaction::UpgradeContract(UpgradeContractTx {
                owner: [0u8; 32],
                contract_id: 1,
                new_bytecode: vec![0u8; 5],
                new_bytecode_hash: [0u8; 32],
                nonce: 0,
                admin_signature: None,
                admin_public_key: None,
                endorser_stakes: vec![],
                required_stake: 0,
                governance_approved: false,
                signature: None,
                public_key: None,
            })),
            100_000 + 200 * 5
        );

        // DeployTemplate: 8 bytes params
        assert_eq!(
            e.estimate_gas(&Transaction::DeployTemplate(DeployTemplateTx {
                deployer: [0u8; 32],
                template_class: 0x0001_0001,
                params: vec![0u8; 8],
                nonce: 0,
                submitted_at_epoch: 0,
                signature: None,
                public_key: None,
            })),
            50_000 + 50 * 8
        );
    }
}
