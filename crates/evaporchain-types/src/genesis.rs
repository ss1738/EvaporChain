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
            max_tx_size: 1_048_576, // 1 MB
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

/// Parts-per-million denominator for `*_ppm` ratio fields.
/// 1_000_000 ppm = 100% = 1.0 in legacy float form.
pub const PPM_DENOMINATOR: u64 = 1_000_000;

/// Basis-points denominator for `*_bps` ratio fields.
/// 10_000 bps = 100% = 1.0 in legacy float form.
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Economic parameters governing token supply and rewards.
///
/// **Validator-determinism note:** all ratio fields are integer
/// parts-per-million (`*_ppm`) or basis-points (`*_bps`). Float
/// arithmetic on these values is forbidden on the consensus path —
/// `distribute_fees` is pure u128 integer math.
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

    /// Fraction of fees burned, in parts-per-million (0..=1_000_000).
    /// 500_000 ppm = 50% burned. Remainder goes to producer + stakers.
    pub fee_burn_rate_ppm: u32,

    /// Fraction of (post-burn) fees redistributed to stakers, in
    /// parts-per-million (0..=1_000_000). E.g., if
    /// `fee_burn_rate_ppm = 500_000` and `staker_fee_share_ppm =
    /// 500_000`, then: 50% burned, 25% to producer, 25% to stakers.
    pub staker_fee_share_ppm: u32,

    /// Annual percentage yield target for staking rewards, in basis
    /// points (informational only — not used in fee distribution
    /// math). 500 bps = 5%.
    pub target_staking_apy_bps: u32,
}

impl Default for Tokenomics {
    fn default() -> Self {
        Self {
            total_supply: 1_000_000_000, // 1B tokens
            block_reward: 100,
            reward_half_life: 1_000_000, // ~2 years at 2s blocks
            fee_burn_rate_ppm: 500_000,  // 50%
            staker_fee_share_ppm: 500_000, // 50%
            target_staking_apy_bps: 500, // 5%
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
    ///
    /// Pure-integer u128 math; no f64 anywhere. Validator-deterministic.
    /// `burned + to_producer + to_stakers = total_fees` always (no
    /// dust loss; truncating division gives any remainder to the
    /// producer).
    pub fn distribute_fees(&self, total_fees: u64) -> FeeDistribution {
        let total_u128 = total_fees as u128;
        // burned = floor(total * fee_burn_rate_ppm / 1_000_000)
        let burned_u128 = total_u128
            .saturating_mul(self.fee_burn_rate_ppm as u128)
            / PPM_DENOMINATOR as u128;
        let burned = burned_u128.min(total_u128) as u64; // clamp ≤ total
        let remaining = total_fees.saturating_sub(burned);
        let to_stakers_u128 = (remaining as u128)
            .saturating_mul(self.staker_fee_share_ppm as u128)
            / PPM_DENOMINATOR as u128;
        let to_stakers = to_stakers_u128.min(remaining as u128) as u64;
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
    /// BLS12-381 proof-of-possession (hex-encoded signature over
    /// `bls_public_key` under `BLS_POP_DST`). Required to defeat
    /// rogue-key attacks on aggregate signatures: the node verifies
    /// this PoP against `bls_public_key` and only marks the validator
    /// `pop_verified` if it succeeds. Genesis configs without this
    /// field load with `pop_verified=false`, which prevents the
    /// validator's BLS key from being used in aggregate certificates
    /// until they broadcast a verified KeyAnnounce. Audit-flagged
    /// 2026-04-27 §2 ("BLS PoP is implemented but not enforced at
    /// validator registration"); closed 2026-05-02.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bls_pop: Option<String>,
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

    /// Trusted weak subjectivity checkpoint for safe node bootstrap.
    /// New nodes joining after genesis MUST include this to defend against long-range attacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_checkpoint: Option<GenesisCheckpoint>,

    /// Coordinator's ML-DSA-65 public key (hex). Set by the genesis ceremony
    /// coordinator and signed over the canonical bytes of every other field.
    /// Closes K-07/K-08 by giving every operator a way to detect tampering of
    /// the genesis JSON between the ceremony and node startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator_pk: Option<String>,

    /// Coordinator's ML-DSA-65 signature (hex) over the canonical bytes of
    /// every other field of this struct (i.e. with `coordinator_signature`
    /// stripped). Verified by node startup and `evaporchain onboarding verify`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator_signature: Option<String>,
}

/// A weak subjectivity checkpoint embedded in genesis config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisCheckpoint {
    pub height: u64,
    pub state_root: String,
    #[serde(default)]
    pub block_hash: String,
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
                errors.push(format!(
                    "duplicate account address: {}",
                    hex::encode(a.address)
                ));
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

        // Fee burn rate must be in [0, 1_000_000] ppm
        if self.tokenomics.fee_burn_rate_ppm > PPM_DENOMINATOR as u32 {
            errors.push(format!(
                "fee_burn_rate_ppm must be 0..=1_000_000, got {}",
                self.tokenomics.fee_burn_rate_ppm
            ));
        }

        // Staker fee share must be in [0, 1_000_000] ppm
        if self.tokenomics.staker_fee_share_ppm > PPM_DENOMINATOR as u32 {
            errors.push(format!(
                "staker_fee_share_ppm must be 0..=1_000_000, got {}",
                self.tokenomics.staker_fee_share_ppm
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
                fee_burn_rate_ppm: 500_000,
                staker_fee_share_ppm: 500_000,
                target_staking_apy_bps: 500,
            },
            genesis_time: "2026-04-06T00:00:00Z".to_string(),
            validators: vec![
                GenesisValidator {
                    id: 1,
                    name: "validator-alpha".into(),
                    stake: 1000,
                    address: addr1,
                    bls_public_key: None,
                    bls_pop: None,
                    p2p_address: Some("/ip4/127.0.0.1/tcp/9000".into()),
                },
                GenesisValidator {
                    id: 2,
                    name: "validator-beta".into(),
                    stake: 1000,
                    address: addr2,
                    bls_public_key: None,
                    bls_pop: None,
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
            trusted_checkpoint: None,
            coordinator_pk: None,
            coordinator_signature: None,
        }
    }

    /// Serialize the config to deterministic bytes for coordinator signing,
    /// with `coordinator_signature` always set to `None` so the signature
    /// covers every other field exactly. Field order is fixed by the struct
    /// declaration; `serde_json` preserves it. The `coordinator_pk` is
    /// included so its value is committed alongside the rest of the config.
    pub fn canonical_signing_bytes(&self) -> Vec<u8> {
        let mut clone = self.clone();
        clone.coordinator_signature = None;
        // Pretty-printing is deliberate: the resulting bytes are stable across
        // round-trips through any JSON parser that re-emits the same struct
        // ordering, and humans can diff candidate genesis files against the
        // exact bytes that were signed.
        serde_json::to_vec(&clone).expect("GenesisConfig is always JSON-serializable")
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
        let t0 = Tokenomics {
            block_reward: 0,
            ..Default::default()
        };
        assert_eq!(t0.reward_at_epoch(100), 0);
    }

    #[test]
    fn test_fee_distribution() {
        let t = Tokenomics {
            fee_burn_rate_ppm: 500_000,
            staker_fee_share_ppm: 500_000,
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
            fee_burn_rate_ppm: 1_000_000,
            staker_fee_share_ppm: 500_000,
            ..Default::default()
        };
        let dist = t.distribute_fees(1000);
        assert_eq!(dist.burned, 1000);
        assert_eq!(dist.to_producer, 0);
        assert_eq!(dist.to_stakers, 0);
    }

    #[test]
    fn test_fee_distribution_deterministic_30_70() {
        // Validator-determinism witness: 30% burn, 70% to stakers.
        // Pure integer math; same byte-exact output on every run /
        // every architecture.
        let t = Tokenomics {
            fee_burn_rate_ppm: 300_000,
            staker_fee_share_ppm: 1_000_000, // all of remainder to stakers
            ..Default::default()
        };
        let dist = t.distribute_fees(1_000_000);
        assert_eq!(dist.burned, 300_000);
        assert_eq!(dist.to_stakers, 700_000);
        assert_eq!(dist.to_producer, 0);
        // Conservation: burned + producer + stakers == total
        assert_eq!(dist.burned + dist.to_producer + dist.to_stakers, 1_000_000);
    }

    #[test]
    fn test_fee_distribution_no_dust_loss() {
        // For any total_fees and any ratio combo, the three buckets
        // must sum exactly to total_fees (no dust escapes).
        let t = Tokenomics {
            fee_burn_rate_ppm: 333_333,
            staker_fee_share_ppm: 666_666,
            ..Default::default()
        };
        for total in [0, 1, 7, 100, 999, 1_000_000, u64::MAX / 2] {
            let dist = t.distribute_fees(total);
            assert_eq!(
                dist.burned.saturating_add(dist.to_producer).saturating_add(dist.to_stakers),
                total,
                "conservation violated at total={total}"
            );
        }
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
