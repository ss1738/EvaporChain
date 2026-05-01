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
