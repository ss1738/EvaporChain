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
    pub data: Vec<u8>,
}

/// Lifecycle state of an object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ObjectState {
    Active,
    Grace,
    Ghost,
    Resurrected,
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
}

/// Transaction types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transaction {
    Transfer(TransferTx),
    Refresh(RefreshTx),
}

/// Value transfer transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTx {
    pub from: AccountAddress,
    pub to: AccountAddress,
    pub amount: u64,
    pub nonce: u64,
}

/// Energy refresh transaction (prevents evaporation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTx {
    pub object_id: ObjectId,
    pub energy_deposit: Energy,
}

/// Commitment to the global state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateCommitment {
    pub verkle_root: [u8; 32],
    pub accumulator_value: [u8; 32],
    pub epoch: Epoch,
}

/// Compute remaining energy after exponential decay using integer math.
///
/// Uses the approximation: energy * 2^(-epochs_elapsed / half_life)
/// Implemented via bit-shifting for integer arithmetic.
pub fn energy_at_epoch(initial: Energy, half_life: HalfLife, epochs_elapsed: u64) -> Energy {
    if half_life == 0 {
        return 0;
    }
    // Number of complete half-lives elapsed
    let full_halvings = epochs_elapsed / half_life;
    let remainder = epochs_elapsed % half_life;

    if full_halvings >= 64 {
        return 0;
    }

    // Apply complete halvings via right-shift
    let after_halvings = initial >> full_halvings;

    // For the fractional part, linearly interpolate between current and next halving
    // energy * (1 - remainder / (2 * half_life))
    // This is a first-order approximation of the exponential between halvings
    let fractional_decay = after_halvings * remainder / (2 * half_life);
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
        // After exactly one half-life, energy should be ~500
        assert_eq!(energy_at_epoch(1000, 10, 10), 500);
    }

    #[test]
    fn test_energy_two_half_lives() {
        // After two half-lives, energy should be ~250
        assert_eq!(energy_at_epoch(1000, 10, 20), 250);
    }

    #[test]
    fn test_energy_zero_half_life() {
        assert_eq!(energy_at_epoch(1000, 0, 5), 0);
    }

    #[test]
    fn test_energy_large_elapsed() {
        // After 64+ half-lives, should be 0
        assert_eq!(energy_at_epoch(1000, 1, 100), 0);
    }

    #[test]
    fn test_energy_partial_decay() {
        // After 5 epochs with half-life 10, energy decays partially
        let result = energy_at_epoch(1000, 10, 5);
        assert!(result > 500 && result < 1000, "got {result}");
    }
}
