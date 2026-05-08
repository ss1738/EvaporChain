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

    /// Annual percentage yield target for staking rewards.
    /// Used as a ceiling: if projected annual rewards exceed
    /// `total_staked * target_staking_apy`, block rewards are scaled down.
    /// TOKENOMICS §2.5 / Q21 ceremony decision 2026-05-08.
    pub target_staking_apy: f64,

    /// Default validator commission rate (0.0 = no commission, 1.0 = 100%).
    /// Applied to the staker pool before proportional distribution:
    /// validator receives `commission × staker_pool`, delegators share the rest.
    /// TOKENOMICS §2.2 / Q7. Default 0.10 (10%) — governance-adjustable per validator.
    /// Backwards-compat: old genesis files without this field default to 0.10.
    #[serde(default = "Tokenomics::default_commission")]
    pub validator_commission_default: f64,

    /// Blocks produced per year at 2-second block time.
    /// Used by the APY cap controller. Default: 15_768_000 (2s blocks).
    #[serde(default = "Tokenomics::default_blocks_per_year")]
    pub blocks_per_year: u64,

    /// Optional terminal cap on cumulative emissions (in token base units).
    /// Once `total_minted >= max_supply_cap`, `reward_at_epoch_capped`
    /// returns 0 regardless of schedule. The final pre-cap block is
    /// clipped to whatever headroom remains, so cumulative emissions
    /// never exceed the cap by even one base unit. `None` = uncapped
    /// (chain emits forever per the schedule). Audit 2026-05-06 MEDIUM.
    #[serde(default)]
    pub max_supply_cap: Option<u64>,

    /// Optional richer emission schedule (TOKENOMICS §2.4 / Q4 closure).
    /// `None` ⇒ legacy path: `reward_at_epoch_capped` (block_reward +
    /// reward_half_life simple halving). `Some(_)` ⇒ takes precedence:
    /// callers MUST use [`crate::emission::block_reward_at`] with
    /// these params, ignoring `block_reward` / `reward_half_life` /
    /// `max_supply_cap` (the cap travels inside `EmissionParams`).
    ///
    /// Backwards-compatible: existing genesis files without an
    /// `emission` block deserialise to `None` and behave identically
    /// to pre-feature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission: Option<crate::emission::EmissionParams>,
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
            validator_commission_default: Self::default_commission(),
            max_supply_cap: None,
            emission: None,
            blocks_per_year: Self::default_blocks_per_year(),
        }
    }
}

impl Tokenomics {
    pub const fn default_blocks_per_year() -> u64 {
        15_768_000 // 365 * 24 * 1800 at 2-second blocks
    }

    /// Default validator commission rate. Used by `#[serde(default)]`.
    pub fn default_commission() -> f64 {
        0.10
    }

    /// Split a staker pool into `(net_to_delegators, validator_commission)`.
    ///
    /// Applies TOKENOMICS §2.2: the validator retains `commission_rate` of the
    /// staker pool; delegators share the remainder proportionally.
    ///
    /// `commission_rate` is clamped to `[0.0, 1.0]` defensively.
    pub fn split_staker_pool(&self, staker_pool: u64) -> (u64, u64) {
        if staker_pool == 0 {
            return (0, 0);
        }
        let rate = self.validator_commission_default.clamp(0.0, 1.0);
        let commission = (staker_pool as f64 * rate).round() as u64;
        let commission = commission.min(staker_pool);
        (staker_pool.saturating_sub(commission), commission)
    }

    /// Compute the APY-capped block reward.
    /// If `total_staked > 0` and the projected annual reward exceeds
    /// `total_staked * target_staking_apy`, scales the reward down to
    /// the per-block APY budget. TOKENOMICS §2.5 / Q21.
    pub fn apy_capped_reward(&self, raw_reward: u64, total_staked: u64) -> u64 {
        if self.target_staking_apy <= 0.0 || total_staked == 0 {
            return raw_reward;
        }
        let bpy = self.blocks_per_year.max(1);
        let annual_budget = (total_staked as f64 * self.target_staking_apy) as u64;
        let budget_per_block = annual_budget / bpy;
        raw_reward.min(budget_per_block.max(1))
    }
    /// Compute the block reward at a given epoch, accounting for halving.
    pub fn reward_at_epoch(&self, epoch: Epoch) -> u64 {
        if self.block_reward == 0 || self.reward_half_life == 0 {
            return self.block_reward;
        }
        crate::energy_at_epoch(self.block_reward, self.reward_half_life, epoch)
    }

    /// Compute the block reward at `epoch`, then clip against
    /// [`max_supply_cap`] given the cumulative emitted-so-far.
    /// Returns the actual reward to mint this block (may be 0 if
    /// the cap is already exhausted, or smaller than the schedule
    /// value if this is the last pre-cap block).
    ///
    /// Audit 2026-05-06 MEDIUM (block reward / emission schedule):
    /// the pre-fix `reward_at_epoch` had no terminal cap, so the
    /// chain would emit forever per the halving schedule. Operators
    /// who want a Bitcoin-style 21M cap had nowhere to encode it.
    /// This method plugs that gap without changing the existing
    /// `reward_at_epoch` shape — callers that don't need cap
    /// enforcement still use the simple form.
    pub fn reward_at_epoch_capped(&self, epoch: Epoch, total_minted: u64) -> u64 {
        let raw = self.reward_at_epoch(epoch);
        match self.max_supply_cap {
            None => raw,
            Some(cap) => {
                if total_minted >= cap {
                    0
                } else {
                    let headroom = cap - total_minted;
                    raw.min(headroom)
                }
            }
        }
    }

    /// Compute the per-block reward at `epoch`. Dispatches based on
    /// the `emission` field:
    ///
    ///   * `Some(params)` — use the rich schedule via
    ///     [`crate::emission::block_reward_at`]. Supports Constant /
    ///     Halving / LinearDecay shapes plus a `max_supply` cap.
    ///   * `None` — fall through to the legacy
    ///     [`Self::reward_at_epoch_capped`] (block_reward +
    ///     reward_half_life + max_supply_cap).
    ///
    /// `total_minted` is the running cumulative emission. Only the
    /// `Some` path uses it for cap enforcement; the `None` path
    /// already respects `max_supply_cap` via
    /// `reward_at_epoch_capped`.
    pub fn block_reward(&self, epoch: Epoch, total_minted: u64) -> u64 {
        match self.emission.as_ref() {
            Some(params) => {
                crate::emission::block_reward_at(params, epoch, total_minted as u128)
            }
            None => self.reward_at_epoch_capped(epoch, total_minted),
        }
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
    /// Optional time-locked vesting schedule on this allocation. Absent in
    /// JSON ⇒ `None` ⇒ fully liquid at genesis. Present ⇒ the account's
    /// `balance` includes the locked portion; outflows are gated until the
    /// cliff + linear-release schedule releases.
    ///
    /// JSON shape: `{"cliff_epoch": u64, "linear_release_epochs": u64,
    ///                "total_locked": u64}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vesting: Option<crate::VestingLock>,
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

        // Validator commission must be in [0, 1]
        if !(0.0..=1.0).contains(&self.tokenomics.validator_commission_default) {
            errors.push(format!(
                "validator_commission_default must be 0.0–1.0, got {}",
                self.tokenomics.validator_commission_default
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
                validator_commission_default: Tokenomics::default_commission(),
                max_supply_cap: None,
                emission: None,
                blocks_per_year: Tokenomics::default_blocks_per_year(),
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
                    vesting: None,
                },
                GenesisAccount {
                    address: addr1,
                    balance: 2_000_000,
                    label: "Validator-1".into(),
                    vesting: None,
                },
                GenesisAccount {
                    address: addr2,
                    balance: 2_000_000,
                    label: "Validator-2".into(),
                    vesting: None,
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

    // ── max_supply_cap (audit 2026-05-06 MEDIUM) ──────────────────

    #[test]
    fn reward_at_epoch_capped_with_no_cap_returns_raw_reward() {
        let tok = Tokenomics::default(); // max_supply_cap = None
        let total = 0;
        // No cap → matches uncapped reward exactly.
        assert_eq!(tok.reward_at_epoch_capped(0, total), tok.reward_at_epoch(0));
        assert_eq!(
            tok.reward_at_epoch_capped(1_000_000, u64::MAX / 2),
            tok.reward_at_epoch(1_000_000)
        );
    }

    #[test]
    fn reward_at_epoch_capped_zero_when_total_at_cap() {
        let mut tok = Tokenomics::default();
        tok.max_supply_cap = Some(1_000_000);
        // cumulative already AT cap → no more emissions.
        assert_eq!(tok.reward_at_epoch_capped(0, 1_000_000), 0);
        assert_eq!(tok.reward_at_epoch_capped(0, 1_000_001), 0);
        assert_eq!(tok.reward_at_epoch_capped(1_000_000, u64::MAX), 0);
    }

    #[test]
    fn reward_at_epoch_capped_clips_final_block_to_headroom() {
        let mut tok = Tokenomics::default();
        tok.block_reward = 100;
        tok.reward_half_life = 0; // constant 100
        tok.max_supply_cap = Some(1050);
        // cumulative = 1000 → headroom = 50, so reward should be
        // clipped from 100 to 50. The next block (cumulative = 1050)
        // gets 0.
        assert_eq!(tok.reward_at_epoch_capped(0, 1000), 50);
        assert_eq!(tok.reward_at_epoch_capped(0, 1050), 0);
    }

    #[test]
    fn reward_at_epoch_capped_compose_with_halving() {
        // Halving every 100 epochs, initial 1024, cap 5000.
        let mut tok = Tokenomics::default();
        tok.block_reward = 1024;
        tok.reward_half_life = 100;
        tok.max_supply_cap = Some(5000);
        // At epoch 0: schedule says 1024, headroom > 1024 → 1024.
        assert_eq!(tok.reward_at_epoch_capped(0, 0), 1024);
        // At epoch 100 (one halving): schedule = 512, cumulative
        // = 4500, headroom = 500, so reward clipped to 500.
        assert_eq!(tok.reward_at_epoch_capped(100, 4500), 500);
        // At epoch 100, cumulative at cap = 0.
        assert_eq!(tok.reward_at_epoch_capped(100, 5000), 0);
    }

    #[test]
    fn tokenomics_serde_round_trip_with_cap() {
        let mut tok = Tokenomics::default();
        tok.max_supply_cap = Some(21_000_000);
        let json = serde_json::to_string(&tok).unwrap();
        let parsed: Tokenomics = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.max_supply_cap, Some(21_000_000));
    }

    #[test]
    fn tokenomics_serde_round_trip_without_cap_field_defaults_to_none() {
        // Existing genesis configs without `max_supply_cap` field
        // must still parse — the #[serde(default)] makes it
        // backward-compatible. Locks the migration contract.
        let json_no_cap = r#"{
            "total_supply": 1000000,
            "block_reward": 100,
            "reward_half_life": 1000000,
            "fee_burn_rate": 0.5,
            "staker_fee_share": 0.5,
            "target_staking_apy": 0.05
        }"#;
        let parsed: Tokenomics = serde_json::from_str(json_no_cap).unwrap();
        assert_eq!(parsed.max_supply_cap, None);
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
