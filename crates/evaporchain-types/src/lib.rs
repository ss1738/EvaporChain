pub mod genesis;

use serde::{Deserialize, Serialize};

/// 32-byte object identifier.
pub type ObjectId = [u8; 32];

/// 32-byte account address.
pub type AccountAddress = [u8; 32];

/// Epoch number (monotonically increasing).
pub type Epoch = u64;

/// Energy units.
pub type Energy = u64;

/// Decay rate parameter.
pub type DecayRate = u64;

/// Half-life in epochs.
pub type HalfLife = u64;

/// A state object stored on-chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateObject {
    pub id: ObjectId,
    pub owner: AccountAddress,
    pub energy: Energy,
    pub half_life: HalfLife,
    pub created_at: Epoch,
    pub last_refreshed: Epoch,
    pub state: ObjectState,
    pub grace_epoch: Option<Epoch>,
    pub data: Vec<u8>,
}

impl StateObject {
    /// Compute remaining energy at the given epoch using exponential decay.
    pub fn energy_at(&self, current_epoch: Epoch) -> Energy {
        let epochs_since_refresh = current_epoch.saturating_sub(self.last_refreshed);
        energy_at_epoch(self.energy, self.half_life, epochs_since_refresh)
    }
}

/// Lifecycle state of an object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ObjectState {
    /// Object is live and accessible.
    Active,
    /// Energy reached zero; object is in grace period before evaporation.
    Grace,
    /// Object has been evaporated — only a nullifier proof remains.
    Ghost,
    /// Object was resurrected from Ghost state via a refresh transaction.
    Resurrected,
}

/// Record left behind when an object evaporates.
/// Stores a compact cryptographic commitment (data_hash) plus an optional
/// copy of the original data. By default, the node retains original data
/// for resurrection convenience. In production, data availability is handled
/// by the DA layer — compact ghosts (original_data = None) save storage
/// and resurrection requires the caller to supply the original data, which
/// is verified against data_hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GhostRecord {
    pub object_id: ObjectId,
    pub owner: AccountAddress,
    pub evaporated_at: Epoch,
    /// BLAKE3 hash of the original object data — always stored.
    pub data_hash: [u8; 32],
    /// Original data retained for resurrection. None = compact ghost
    /// (data must be supplied externally via DA layer for resurrection).
    #[serde(default)]
    pub original_data: Option<Vec<u8>>,
    /// Position in the MMR nullifier accumulator (None for legacy ghosts).
    #[serde(default)]
    pub mmr_position: Option<u64>,
}

/// A block in the chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub number: u64,
    pub epoch: Epoch,
    pub parent_hash: [u8; 32],
    pub state_root: [u8; 32],
    pub transactions: Vec<Transaction>,
    pub timestamp: u64,
    /// Validator ID that produced this block (None for single-node mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_id: Option<u64>,
}

/// An account with a balance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub address: AccountAddress,
    pub balance: u64,
    pub nonce: u64,
}

/// Transaction types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transaction {
    Transfer(TransferTx),
    Refresh(RefreshTx),
    CreateObject(CreateObjectTx),
    DeployContract(DeployContractTx),
    CallContract(CallContractTx),
    DeployScript(DeployScriptTx),
    CallScript(CallScriptTx),
    ValidatorStake(ValidatorStakeTx),
    ValidatorExit(ValidatorExitTx),
}

impl Transaction {
    /// Compute the canonical byte representation for signing.
    /// Excludes signature/public_key fields — only the transaction body is signed.
    pub fn signable_bytes(&self) -> Vec<u8> {
        match self {
            Transaction::Transfer(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 32 + 8 + 8);
                buf.push(0x01); // type tag
                buf.extend_from_slice(&tx.from);
                buf.extend_from_slice(&tx.to);
                buf.extend_from_slice(&tx.amount.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            Transaction::Refresh(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 8);
                buf.push(0x02);
                buf.extend_from_slice(&tx.object_id);
                buf.extend_from_slice(&tx.energy_deposit.to_le_bytes());
                buf
            }
            Transaction::CreateObject(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 32 + 8 + 8 + tx.data.len());
                buf.push(0x03);
                buf.extend_from_slice(&tx.creator);
                buf.extend_from_slice(&tx.object_id);
                buf.extend_from_slice(&tx.energy.to_le_bytes());
                buf.extend_from_slice(&tx.half_life.to_le_bytes());
                buf.extend_from_slice(&tx.data);
                buf
            }
            Transaction::DeployContract(tx) => {
                let mut buf = Vec::new();
                buf.push(0x04);
                buf.extend_from_slice(&tx.deployer);
                buf.extend_from_slice(tx.template.as_bytes());
                buf.extend_from_slice(tx.init_args.as_bytes());
                buf.extend_from_slice(&tx.energy.to_le_bytes());
                buf.extend_from_slice(&tx.half_life.to_le_bytes());
                buf
            }
            Transaction::CallContract(tx) => {
                let mut buf = Vec::new();
                buf.push(0x05);
                buf.extend_from_slice(&tx.caller);
                buf.extend_from_slice(&tx.contract_id.to_le_bytes());
                buf.extend_from_slice(tx.method.as_bytes());
                buf.extend_from_slice(tx.args.as_bytes());
                buf
            }
            Transaction::DeployScript(tx) => {
                let mut buf = Vec::new();
                buf.push(0x06);
                buf.extend_from_slice(&tx.deployer);
                buf.extend_from_slice(tx.source_code.as_bytes());
                buf.extend_from_slice(&tx.energy.to_le_bytes());
                buf.extend_from_slice(&tx.half_life.to_le_bytes());
                buf
            }
            Transaction::CallScript(tx) => {
                let mut buf = Vec::new();
                buf.push(0x07);
                buf.extend_from_slice(&tx.caller);
                buf.extend_from_slice(&tx.contract_id.to_le_bytes());
                buf.extend_from_slice(tx.method.as_bytes());
                buf.extend_from_slice(tx.args.as_bytes());
                buf
            }
            Transaction::ValidatorStake(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 8 + 8 + 8);
                buf.push(0x08);
                buf.extend_from_slice(&tx.validator_address);
                buf.extend_from_slice(&tx.stake_amount.to_le_bytes());
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            Transaction::ValidatorExit(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 8 + 8);
                buf.push(0x09);
                buf.extend_from_slice(&tx.validator_address);
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
        }
    }

    /// Get the signature bytes (if present on the inner tx).
    pub fn signature(&self) -> Option<&[u8]> {
        match self {
            Transaction::Transfer(tx) => tx.signature.as_deref(),
            Transaction::Refresh(tx) => tx.signature.as_deref(),
            Transaction::CreateObject(tx) => tx.signature.as_deref(),
            Transaction::DeployContract(tx) => tx.signature.as_deref(),
            Transaction::CallContract(tx) => tx.signature.as_deref(),
            Transaction::DeployScript(tx) => tx.signature.as_deref(),
            Transaction::CallScript(tx) => tx.signature.as_deref(),
            Transaction::ValidatorStake(tx) => tx.signature.as_deref(),
            Transaction::ValidatorExit(tx) => tx.signature.as_deref(),
        }
    }

    /// Get the public key bytes (if present on the inner tx).
    pub fn public_key(&self) -> Option<&[u8]> {
        match self {
            Transaction::Transfer(tx) => tx.public_key.as_deref(),
            Transaction::Refresh(tx) => tx.public_key.as_deref(),
            Transaction::CreateObject(tx) => tx.public_key.as_deref(),
            Transaction::DeployContract(tx) => tx.public_key.as_deref(),
            Transaction::CallContract(tx) => tx.public_key.as_deref(),
            Transaction::DeployScript(tx) => tx.public_key.as_deref(),
            Transaction::CallScript(tx) => tx.public_key.as_deref(),
            Transaction::ValidatorStake(tx) => tx.public_key.as_deref(),
            Transaction::ValidatorExit(tx) => tx.public_key.as_deref(),
        }
    }

    /// Get the sender/payer address for fee deduction.
    /// Returns the address of the account responsible for paying gas fees.
    pub fn sender(&self) -> Option<&AccountAddress> {
        match self {
            Transaction::Transfer(tx) => Some(&tx.from),
            Transaction::CreateObject(tx) => Some(&tx.creator),
            Transaction::DeployContract(tx) => Some(&tx.deployer),
            Transaction::CallContract(tx) => Some(&tx.caller),
            Transaction::DeployScript(tx) => Some(&tx.deployer),
            Transaction::CallScript(tx) => Some(&tx.caller),
            Transaction::Refresh(_) => None, // Refresh has no sender address field
            Transaction::ValidatorStake(tx) => Some(&tx.validator_address),
            Transaction::ValidatorExit(tx) => Some(&tx.validator_address),
        }
    }
}

/// Value transfer transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTx {
    pub from: AccountAddress,
    pub to: AccountAddress,
    pub amount: u64,
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Energy refresh transaction (prevents evaporation or resurrects a ghost).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTx {
    pub object_id: ObjectId,
    pub energy_deposit: Energy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Create a new state object with initial energy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateObjectTx {
    pub creator: AccountAddress,
    pub object_id: ObjectId,
    pub energy: Energy,
    pub half_life: HalfLife,
    pub data: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Deploy a smart contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployContractTx {
    pub deployer: AccountAddress,
    /// Template name: "DecayingToken", "MortalNFT", "ThermodynamicEscrow",
    /// "DecayingAuction", "StakingPool", "DAOVote"
    pub template: String,
    /// JSON-encoded initialization arguments.
    pub init_args: String,
    /// Initial energy for the contract instance.
    pub energy: Energy,
    /// Half-life for contract energy decay.
    pub half_life: HalfLife,
    /// Custom rules (JSON-encoded array), optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Call a method on a deployed contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallContractTx {
    pub caller: AccountAddress,
    pub contract_id: u64,
    pub method: String,
    /// JSON-encoded method arguments.
    pub args: String,
    /// Current epoch (for energy checks).
    pub epoch: Epoch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Deploy an EvaporScript contract from source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployScriptTx {
    pub deployer: AccountAddress,
    /// EvaporScript source code.
    pub source_code: String,
    /// Initial energy for the script contract.
    pub energy: Energy,
    /// Half-life for script contract energy decay.
    pub half_life: HalfLife,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Call a method on a deployed EvaporScript contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallScriptTx {
    pub caller: AccountAddress,
    pub contract_id: u64,
    pub method: String,
    /// JSON-encoded method arguments.
    pub args: String,
    /// Current epoch (for energy checks).
    pub epoch: Epoch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Stake tokens as a validator (or increase existing stake).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorStakeTx {
    /// Validator address (same as staker address).
    pub validator_address: AccountAddress,
    /// Amount to stake (transferred from balance to stake).
    pub stake_amount: u64,
    /// Validator ID to register or update.
    pub validator_id: u64,
    /// Sender nonce.
    pub nonce: u64,
    /// BLS12-381 public key for consensus (hex-encoded, 48 bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bls_public_key: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Request to exit the validator set and begin unbonding.
/// Stake is locked for `unbonding_period` epochs after this tx is processed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorExitTx {
    /// Validator address.
    pub validator_address: AccountAddress,
    /// Validator ID to exit.
    pub validator_id: u64,
    /// Sender nonce.
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Commitment to the global state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateCommitment {
    pub verkle_root: [u8; 32],
    pub accumulator_value: [u8; 32],
    pub epoch: Epoch,
}

/// Dual commitment: Verkle state trie + MMR nullifier accumulator.
/// This is the canonical commitment to EvaporChain's full state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DualCommitment {
    /// Verkle trie root over active objects and accounts.
    pub verkle_root: [u8; 32],
    /// MMR root over all energy-stamped nullifiers (evaporated objects).
    pub mmr_root: [u8; 32],
    /// Current epoch.
    pub epoch: Epoch,
    /// Number of active (non-ghost) objects.
    pub active_count: usize,
    /// Number of ghost records.
    pub ghost_count: usize,
}

/// Compute remaining energy after exponential decay using integer math.
///
/// Uses the approximation: energy * 2^(-epochs_elapsed / half_life)
/// Implemented via bit-shifting for complete halvings and linear
/// interpolation for the fractional part.
pub fn energy_at_epoch(initial: Energy, half_life: HalfLife, epochs_elapsed: u64) -> Energy {
    if half_life == 0 {
        return 0;
    }
    let full_halvings = epochs_elapsed / half_life;
    let remainder = epochs_elapsed % half_life;

    if full_halvings >= 64 {
        return 0;
    }

    let after_halvings = initial >> full_halvings;

    // Linear interpolation for the fractional part between halvings.
    // Use u128 to avoid overflow on large values.
    let fractional_decay =
        (after_halvings as u128 * remainder as u128 / (2u128 * half_life as u128)) as u64;
    after_halvings.saturating_sub(fractional_decay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_energy_no_decay() {
        assert_eq!(energy_at_epoch(1000, 10, 0), 1000);
    }

    #[test]
    fn test_energy_one_half_life() {
        assert_eq!(energy_at_epoch(1000, 10, 10), 500);
    }

    #[test]
    fn test_energy_two_half_lives() {
        assert_eq!(energy_at_epoch(1000, 10, 20), 250);
    }

    #[test]
    fn test_energy_zero_half_life() {
        assert_eq!(energy_at_epoch(1000, 0, 5), 0);
    }

    #[test]
    fn test_energy_large_elapsed() {
        assert_eq!(energy_at_epoch(1000, 1, 100), 0);
    }

    #[test]
    fn test_energy_partial_decay() {
        let result = energy_at_epoch(1000, 10, 5);
        assert!(result > 500 && result < 1000, "got {result}");
    }

    #[test]
    fn test_state_object_energy_at() {
        let obj = StateObject {
            id: [1u8; 32],
            owner: [2u8; 32],
            energy: 1000,
            half_life: 10,
            created_at: 0,
            last_refreshed: 5,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
        };
        // At epoch 15, 10 epochs since refresh -> one half-life -> 500
        assert_eq!(obj.energy_at(15), 500);
        // At epoch 5 (same as refresh), no decay
        assert_eq!(obj.energy_at(5), 1000);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Property-Based Tests (Audit Hardening)
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Energy never increases with time (monotonically non-increasing).
        #[test]
        fn energy_monotonically_decreasing(
            initial in 1u64..1_000_000,
            half_life in 1u64..1000,
            t1 in 0u64..500,
            dt in 0u64..500,
        ) {
            let t2 = t1 + dt;
            let e1 = energy_at_epoch(initial, half_life, t1);
            let e2 = energy_at_epoch(initial, half_life, t2);
            prop_assert!(e2 <= e1, "energy increased: e({})={} > e({})={}", t1, e1, t2, e2);
        }

        /// Energy at t=0 equals the initial value.
        #[test]
        fn energy_at_zero_is_initial(initial in 0u64..u64::MAX, half_life in 1u64..1000) {
            prop_assert_eq!(energy_at_epoch(initial, half_life, 0), initial);
        }

        /// Energy after exactly one half-life is exactly half (for exact halvings).
        #[test]
        fn energy_halves_at_half_life(initial in 0u64..1_000_000, half_life in 1u64..1000) {
            let result = energy_at_epoch(initial, half_life, half_life);
            prop_assert_eq!(result, initial / 2);
        }

        /// Energy after exactly two half-lives is exactly one quarter.
        #[test]
        fn energy_quarters_at_two_half_lives(initial in 0u64..1_000_000, half_life in 1u64..500) {
            let result = energy_at_epoch(initial, half_life, 2 * half_life);
            prop_assert_eq!(result, initial / 4);
        }

        /// Energy never overflows or panics for any valid inputs.
        #[test]
        fn energy_never_panics(
            initial in any::<u64>(),
            half_life in any::<u64>(),
            elapsed in any::<u64>(),
        ) {
            let _ = energy_at_epoch(initial, half_life, elapsed);
        }

        /// Energy eventually reaches zero for any positive initial and half_life.
        #[test]
        fn energy_eventually_reaches_zero(
            initial in 1u64..1_000_000,
            half_life in 1u64..100,
        ) {
            // After 64 full halvings, any u64 initial should be 0
            let elapsed = 64 * half_life;
            let result = energy_at_epoch(initial, half_life, elapsed);
            prop_assert_eq!(result, 0, "energy should be 0 after 64 half-lives");
        }

        /// Energy with zero half_life is always zero.
        #[test]
        fn zero_half_life_gives_zero(initial in any::<u64>(), elapsed in any::<u64>()) {
            prop_assert_eq!(energy_at_epoch(initial, 0, elapsed), 0);
        }
    }
}
