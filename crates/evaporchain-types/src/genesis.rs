//! Genesis configuration and chain parameters.
//!
//! Defines the full configuration needed to bootstrap an EvaporChain network:
//! chain identity, validator set, initial account allocations, economic parameters,
//! and consensus settings. The genesis config is serializable to/from JSON for
//! easy distribution.

use crate::{AccountAddress, Energy, Epoch, HalfLife, ObjectId};
use serde::{Deserialize, Serialize};

// ─────────────────────── Chain Parameters ────────────────────────────────────

/// Core chain parameters that govern network behavior.
/// These are fixed at genesis and can only change via hard fork.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainParams {
    /// Unique chain identifier (e.g., "evaporchain-mainnet-1").
    pub chain_id: String,

    /// Block production interval in milliseconds.
    pub block_interval_ms: u64,

    /// Number of epochs an object stays in Grace before evaporation.
    pub grace_period: u64,

    /// Maximum gas consumption per block.
    pub block_gas_limit: u64,

    /// Maximum transaction size in bytes.
    pub max_tx_size: u64,

    /// Maximum number of transactions per block.
    pub max_txs_per_block: usize,

    /// Minimum stake required to become a validator (in base units).
    pub min_validator_stake: u64,

    /// Number of epochs a validator must wait after requesting exit before
    /// their stake is unlocked (unbonding period).
    pub unbonding_period: u64,
}

impl Default for ChainParams {
    fn default() -> Self {
        Self {
            chain_id: "evaporchain-mainnet-1".to_string(),
            block_interval_ms: 2000,
            grace_period: 5,
            block_gas_limit: 500_000,
            max_tx_size: 1_048_576,   // 1 MB
            max_txs_per_block: 10_000,
            min_validator_stake: 100_000,
            unbonding_period: 100,
        }
    }
}

impl ChainParams {
    /// Testnet configuration with lower requirements.
    pub fn testnet() -> Self {
        Self {
            chain_id: "evaporchain-testnet-1".to_string(),
            block_interval_ms: 1000,
            grace_period: 5,
            block_gas_limit: 500_000,
            max_tx_size: 1_048_576,
            max_txs_per_block: 10_000,
            min_validator_stake: 100,
            unbonding_period: 10,
        }
    }
}

// ─────────────────────── Tokenomics ─────────────────────────────────────────

/// Economic parameters governing token supply and rewards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokenomics {
    /// Total token supply at genesis.
    pub total_supply: u64,

    /// Block reward paid to the block producer (minted per block).
    /// Set to 0 for a fully deflationary model.
    pub block_reward: u64,

    /// Half-life for block reward decay (in epochs).
    /// Reward halves every `reward_half_life` epochs.
    /// Set to 0 to disable reward decay (constant reward).
    pub reward_half_life: u64,

    /// Fraction of fees burned (0.0 = none burned, 1.0 = all burned).
    /// Remainder goes to the block producer.
    pub fee_burn_rate: f64,

    /// Fraction of fees redistributed to stakers (from the non-burned portion).
    /// E.g., if fee_burn_rate=0.5 and staker_fee_share=0.5, then:
    ///   50% burned, 25% to producer, 25% to stakers.
    pub staker_fee_share: f64,

    /// Annual percentage yield target for staking rewards (informational).
    pub target_staking_apy: f64,
}

impl Default for Tokenomics {
    fn default() -> Self {
        Self {
            total_supply: 1_000_000_000, // 1B tokens
            block_reward: 100,
            reward_half_life: 1_000_000, // ~2 years at 2s blocks
            fee_burn_rate: 0.50,
            staker_fee_share: 0.50,
            target_staking_apy: 0.05,
        }
    }
}

impl Tokenomics {
    /// Compute the block reward at a given epoch, accounting for halving.
    pub fn reward_at_epoch(&self, epoch: Epoch) -> u64 {
        if self.block_reward == 0 || self.reward_half_life == 0 {
            return self.block_reward;
        }
        crate::energy_at_epoch(self.block_reward, self.reward_half_life, epoch)
    }

    /// Compute fee distribution for a block.
    pub fn distribute_fees(&self, total_fees: u64) -> FeeDistribution {
        let burned = (total_fees as f64 * self.fee_burn_rate).round() as u64;
        let remaining = total_fees.saturating_sub(burned);
        let to_stakers = (remaining as f64 * self.staker_fee_share).round() as u64;
        let to_producer = remaining.saturating_sub(to_stakers);
        FeeDistribution {
            burned,
            to_producer,
            to_stakers,
        }
    }
}

/// How collected fees are split each block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeDistribution {
    /// Amount permanently destroyed.
    pub burned: u64,
    /// Amount paid to the block producer.
    pub to_producer: u64,
    /// Amount distributed to stakers (proportional to stake).
    pub to_stakers: u64,
}

// ─────────────────────── Genesis Accounts ────────────────────────────────────

/// An account allocation at genesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisAccount {
    /// Account address (32 bytes, hex-encoded in JSON).
    pub address: AccountAddress,
    /// Initial balance.
    pub balance: u64,
    /// Human-readable label (not stored on-chain).
    #[serde(default)]
    pub label: String,
}

// ─────────────────────── Genesis Validators ──────────────────────────────────

/// A validator defined at genesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisValidator {
    /// Unique validator ID.
    pub id: u64,
    /// Human-readable name.
    pub name: String,
    /// Initial stake amount.
    pub stake: u64,
    /// Validator address (32 bytes).
    pub address: AccountAddress,
    /// BLS12-381 public key (hex-encoded, 48 bytes compressed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bls_public_key: Option<String>,
    /// P2P multiaddress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p2p_address: Option<String>,
}

// ─────────────────────── Genesis Objects ─────────────────────────────────────

/// A state object pre-created at genesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisObject {
    /// Object ID.
    pub id: ObjectId,
    /// Owner address.
    pub owner: AccountAddress,
    /// Initial energy.
    pub energy: Energy,
    /// Half-life in epochs.
    pub half_life: HalfLife,
    /// Initial data payload.
    pub data: Vec<u8>,
}

// ─────────────────────── Genesis Config ──────────────────────────────────────

/// Complete genesis configuration for bootstrapping an EvaporChain network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisConfig {
    /// Chain parameters.
    pub chain_params: ChainParams,

    /// Economic parameters.
    pub tokenomics: Tokenomics,

    /// Genesis timestamp (ISO 8601).
    pub genesis_time: String,

    /// Initial validator set.
    pub validators: Vec<GenesisValidator>,

    /// Initial account allocations.
    pub accounts: Vec<GenesisAccount>,

    /// Pre-created state objects (optional).
    #[serde(default)]
    pub objects: Vec<GenesisObject>,

    /// Bootstrap peer addresses for P2P discovery.
    #[serde(default)]
    pub bootstrap_peers: Vec<String>,
}

impl GenesisConfig {
    /// Validate the genesis configuration for internal consistency.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // Chain ID must not be empty
        if self.chain_params.chain_id.is_empty() {
            errors.push("chain_id must not be empty".into());
        }

        // Must have at least one validator
        if self.validators.is_empty() {
            errors.push("must have at least one validator".into());
        }

        // Validators must meet minimum stake
        for v in &self.validators {
            if v.stake < self.chain_params.min_validator_stake {
                errors.push(format!(
                    "validator {} ({}) has stake {} below minimum {}",
                    v.id, v.name, v.stake, self.chain_params.min_validator_stake
                ));
            }
        }

        // No duplicate validator IDs
        let mut seen_ids = std::collections::HashSet::new();
        for v in &self.validators {
            if !seen_ids.insert(v.id) {
                errors.push(format!("duplicate validator id: {}", v.id));
            }
        }

        // No duplicate account addresses
        let mut seen_addrs = std::collections::HashSet::new();
        for a in &self.accounts {
            if !seen_addrs.insert(a.address) {
                errors.push(format!("duplicate account address: {}", hex::encode(a.address)));
            }
        }

        // Total account balances must not exceed total supply
        let total_allocated: u64 = self.accounts.iter().map(|a| a.balance).sum();
        if total_allocated > self.tokenomics.total_supply {
            errors.push(format!(
                "total allocated ({}) exceeds total supply ({})",
                total_allocated, self.tokenomics.total_supply
            ));
        }

        // Fee burn rate must be in [0, 1]
        if !(0.0..=1.0).contains(&self.tokenomics.fee_burn_rate) {
            errors.push(format!(
                "fee_burn_rate must be 0.0–1.0, got {}",
                self.tokenomics.fee_burn_rate
            ));
        }

        // Staker fee share must be in [0, 1]
        if !(0.0..=1.0).contains(&self.tokenomics.staker_fee_share) {
            errors.push(format!(
                "staker_fee_share must be 0.0–1.0, got {}",
                self.tokenomics.staker_fee_share
            ));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Create a minimal testnet genesis config.
    pub fn testnet_default() -> Self {
        let mut addr1 = [0u8; 32];
        addr1[0] = 0x01;
        let mut addr2 = [0u8; 32];
        addr2[0] = 0x02;
        let mut faucet_addr = [0u8; 32];
        faucet_addr[0] = 0xFF;

        Self {
            chain_params: ChainParams::testnet(),
            tokenomics: Tokenomics {
                total_supply: 10_000_000,
                block_reward: 10,
                reward_half_life: 100_000,
                fee_burn_rate: 0.50,
                staker_fee_share: 0.50,
                target_staking_apy: 0.05,
            },
            genesis_time: "2026-04-06T00:00:00Z".to_string(),
            validators: vec![
                GenesisValidator {
                    id: 1,
                    name: "validator-alpha".into(),
                    stake: 1000,
                    address: addr1,
                    bls_public_key: None,
                    p2p_address: Some("/ip4/127.0.0.1/tcp/9000".into()),
                },
                GenesisValidator {
                    id: 2,
                    name: "validator-beta".into(),
                    stake: 1000,
                    address: addr2,
                    bls_public_key: None,
                    p2p_address: Some("/ip4/127.0.0.1/tcp/9001".into()),
                },
            ],
            accounts: vec![
                GenesisAccount {
                    address: faucet_addr,
                    balance: 5_000_000,
                    label: "Faucet".into(),
                },
                GenesisAccount {
                    address: addr1,
                    balance: 2_000_000,
                    label: "Validator-1".into(),
                },
                GenesisAccount {
                    address: addr2,
                    balance: 2_000_000,
                    label: "Validator-2".into(),
                },
            ],
            objects: vec![],
            bootstrap_peers: vec![
                "/ip4/127.0.0.1/tcp/9000".into(),
                "/ip4/127.0.0.1/tcp/9001".into(),
            ],
        }
    }
}

// ─────────────────────── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_chain_params() {
        let p = ChainParams::default();
        assert_eq!(p.chain_id, "evaporchain-mainnet-1");
        assert_eq!(p.block_gas_limit, 500_000);
        assert_eq!(p.min_validator_stake, 100_000);
    }

    #[test]
    fn test_testnet_chain_params() {
        let p = ChainParams::testnet();
        assert_eq!(p.min_validator_stake, 100);
        assert_eq!(p.unbonding_period, 10);
    }

    #[test]
    fn test_tokenomics_reward_at_epoch() {
        let t = Tokenomics::default();
        // At epoch 0, full reward
        assert_eq!(t.reward_at_epoch(0), 100);
        // After one reward_half_life, half reward
        assert_eq!(t.reward_at_epoch(t.reward_half_life), 50);
        // Zero reward config
        let t0 = Tokenomics { block_reward: 0, ..Default::default() };
        assert_eq!(t0.reward_at_epoch(100), 0);
    }

    #[test]
    fn test_fee_distribution() {
        let t = Tokenomics {
            fee_burn_rate: 0.50,
            staker_fee_share: 0.50,
            ..Default::default()
        };
        let dist = t.distribute_fees(1000);
        assert_eq!(dist.burned, 500);
        assert_eq!(dist.to_stakers, 250);
        assert_eq!(dist.to_producer, 250);
    }

    #[test]
    fn test_fee_distribution_all_burned() {
        let t = Tokenomics {
            fee_burn_rate: 1.0,
            staker_fee_share: 0.50,
            ..Default::default()
        };
        let dist = t.distribute_fees(1000);
        assert_eq!(dist.burned, 1000);
        assert_eq!(dist.to_producer, 0);
        assert_eq!(dist.to_stakers, 0);
    }

    #[test]
    fn test_genesis_config_validation_valid() {
        let config = GenesisConfig::testnet_default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_genesis_config_validation_no_validators() {
        let mut config = GenesisConfig::testnet_default();
        config.validators.clear();
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("at least one validator")));
    }

    #[test]
    fn test_genesis_config_validation_stake_too_low() {
        let mut config = GenesisConfig::testnet_default();
        config.validators[0].stake = 1; // below min_validator_stake=100
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("below minimum")));
    }

    #[test]
    fn test_genesis_config_validation_duplicate_ids() {
        let mut config = GenesisConfig::testnet_default();
        config.validators[1].id = config.validators[0].id;
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("duplicate validator id")));
    }

    #[test]
    fn test_genesis_config_validation_exceeds_supply() {
        let mut config = GenesisConfig::testnet_default();
        config.accounts[0].balance = config.tokenomics.total_supply + 1;
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("exceeds total supply")));
    }

    #[test]
    fn test_genesis_config_serialization_roundtrip() {
        let config = GenesisConfig::testnet_default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: GenesisConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chain_params.chain_id, config.chain_params.chain_id);
        assert_eq!(parsed.validators.len(), config.validators.len());
        assert_eq!(parsed.accounts.len(), config.accounts.len());
    }
}
