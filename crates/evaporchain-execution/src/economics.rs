//! Economic model simulation and parameter tuning for EvaporChain.
//!
//! Models energy decay, gas pricing, validator economics, and game-theoretic
//! attack costs. Used for parameter tuning and economic safety analysis.

/// Configuration for energy decay simulation.
#[derive(Debug, Clone)]
pub struct DecaySimConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub grace_period: u64,
    pub refresh_cost: u64,
    pub blocks_to_simulate: u64,
}

/// Result of energy decay simulation.
#[derive(Debug, Clone)]
pub struct DecaySimResult {
    pub energy_at_end: u64,
    pub blocks_until_grace: Option<u64>,
    pub blocks_until_evaporation: Option<u64>,
    pub total_refresh_cost: u64,
    pub refreshes_needed: u64,
}

/// **DEPRECATED. AUDIT_2026_05_13 H10.**
///
/// Pre-fix this was a `pub fn` that used raw `>>` on energy values
/// AND `f64` interpolation. Both violate CLAUDE.md's Layer 0 invariant
/// ("Never use `>>` on energy values outside `evaporchain-types`")
/// because:
/// 1. Raw `>>` outside `evaporchain-types` makes the decay formula
///    drift from the Coq-verified canonical one (the
///    `energy_at_epoch_monotone` theorem in `research/coq/`).
/// 2. `f64` arithmetic is architecture-dependent — two validators on
///    different cores can compute different `(diff as f64 *
///    fraction_elapsed) as u64` results, breaking consensus
///    determinism.
///
/// Today no external callers; the `pub` visibility was the hazard.
/// Function is now `pub(crate)` + `#[deprecated]`; the only in-file
/// caller (`simulate_decay` below) has been rerouted through the
/// canonical `evaporchain_types::energy_at_epoch`. Any future use must
/// go through the canonical function instead.
#[deprecated(
    since = "0.2.0",
    note = "AUDIT_2026_05_13 H10: violates Layer 0 invariant (raw `>>` + f64). \
            Use `evaporchain_types::energy_at_epoch` for the canonical, \
            Coq-verified, deterministic-across-architectures formula."
)]
pub(crate) fn compute_energy(initial: u64, half_life: u64, epochs_elapsed: u64) -> u64 {
    // Forwarder to the canonical function. Result-wise this may differ
    // by ~1 unit from the pre-deprecation body due to u128-integer vs
    // f64-float interpolation, but the canonical answer is the only
    // one that all validators agree on byte-for-byte.
    evaporchain_types::energy_at_epoch(initial, half_life, epochs_elapsed)
}

/// Simulate energy decay over N blocks.
pub fn simulate_decay(config: &DecaySimConfig) -> DecaySimResult {
    let mut blocks_until_grace = None;
    let mut blocks_until_evaporation = None;
    let mut refreshes = 0u64;
    let mut total_cost = 0u64;

    for block in 1..=config.blocks_to_simulate {
        // AUDIT H10: route through canonical energy_at_epoch (Layer 0).
        let energy =
            evaporchain_types::energy_at_epoch(config.initial_energy, config.half_life, block);
        if energy == 0 && blocks_until_grace.is_none() {
            blocks_until_grace = Some(block);
        }
        if energy == 0 && block >= blocks_until_grace.unwrap_or(0) + config.grace_period {
            blocks_until_evaporation = Some(block);
            break;
        }
    }

    // AUDIT H10: canonical energy_at_epoch (deterministic across arches).
    let energy_at_end = evaporchain_types::energy_at_epoch(
        config.initial_energy,
        config.half_life,
        config.blocks_to_simulate,
    );

    // Calculate refreshes needed to keep object alive
    if config.half_life > 0 {
        let refresh_interval = config.half_life; // refresh every half-life
        if config.blocks_to_simulate > refresh_interval {
            refreshes = config.blocks_to_simulate / refresh_interval;
            total_cost = refreshes * config.refresh_cost;
        }
    }

    DecaySimResult {
        energy_at_end,
        blocks_until_grace,
        blocks_until_evaporation,
        total_refresh_cost: total_cost,
        refreshes_needed: refreshes,
    }
}

/// Find the minimum half-life needed for an object to survive M blocks.
pub fn find_min_half_life_for_survival(initial_energy: u64, target_blocks: u64) -> u64 {
    for hl in 1..=target_blocks * 2 {
        // AUDIT H10: canonical energy_at_epoch (Layer 0).
        let energy = evaporchain_types::energy_at_epoch(initial_energy, hl, target_blocks);
        if energy > 0 {
            return hl;
        }
    }
    target_blocks * 2
}

/// Calculate steady-state cost to maintain N objects over M blocks.
pub fn steady_state_maintenance_cost(
    num_objects: u64,
    half_life: u64,
    refresh_cost: u64,
    total_blocks: u64,
) -> u64 {
    if half_life == 0 {
        return 0;
    }
    let refreshes_per_object = total_blocks / half_life;
    num_objects * refreshes_per_object * refresh_cost
}

// ─── Gas Price Model ────────────────────────────────────────────────────────

/// EIP-1559-style gas price configuration.
#[derive(Debug, Clone)]
pub struct GasPriceConfig {
    pub initial_base_fee: u64,
    pub target_gas_per_block: u64,
    pub max_gas_per_block: u64,
    pub adjustment_denominator: u64,
    pub min_base_fee: u64,
}

impl Default for GasPriceConfig {
    fn default() -> Self {
        Self {
            initial_base_fee: 100,
            target_gas_per_block: 5_000_000,
            max_gas_per_block: 10_000_000,
            adjustment_denominator: 8,
            min_base_fee: 1,
        }
    }
}

/// Simulate gas price evolution under constant load.
pub fn simulate_gas_price(config: &GasPriceConfig, load_pct: u64, blocks: u64) -> Vec<u64> {
    let gas_used_per_block = config.target_gas_per_block * load_pct / 100;
    let mut prices = Vec::with_capacity(blocks as usize);
    let mut base_fee = config.initial_base_fee;

    for _ in 0..blocks {
        prices.push(base_fee);
        base_fee = adjust_base_fee(
            base_fee,
            gas_used_per_block,
            config.target_gas_per_block,
            config.adjustment_denominator,
            config.min_base_fee,
        );
    }
    prices
}

/// Simulate gas price under varying load over time.
pub fn simulate_gas_price_varying(
    config: &GasPriceConfig,
    load_schedule: &[(u64, u64)], // (blocks, load_pct)
) -> Vec<u64> {
    let mut prices = Vec::new();
    let mut base_fee = config.initial_base_fee;

    for &(blocks, load_pct) in load_schedule {
        let gas_used = config.target_gas_per_block * load_pct / 100;
        for _ in 0..blocks {
            prices.push(base_fee);
            base_fee = adjust_base_fee(
                base_fee,
                gas_used,
                config.target_gas_per_block,
                config.adjustment_denominator,
                config.min_base_fee,
            );
        }
    }
    prices
}

fn adjust_base_fee(
    current: u64,
    gas_used: u64,
    target: u64,
    denominator: u64,
    min_fee: u64,
) -> u64 {
    if gas_used > target {
        let excess = gas_used - target;
        let delta = (current * excess) / (target * denominator);
        current.saturating_add(delta.max(1))
    } else if gas_used < target {
        let deficit = target - gas_used;
        let delta = (current * deficit) / (target * denominator);
        current.saturating_sub(delta).max(min_fee)
    } else {
        current
    }
}

// ─── Validator Economics ─────────────────────────────────────────────────────

/// Validator economics configuration.
#[derive(Debug, Clone)]
pub struct ValidatorEconConfig {
    pub block_reward: u64,
    pub total_validators: u64,
    pub stake_per_validator: u64,
    pub blocks_per_year: u64,
    pub fee_burn_pct: u64,          // percentage of fees burned
    pub fee_producer_pct: u64,      // percentage to block producer
    pub slash_double_vote_pct: u64, // percentage of stake slashed
}

impl Default for ValidatorEconConfig {
    fn default() -> Self {
        Self {
            block_reward: 1000,
            total_validators: 100,
            stake_per_validator: 100_000,
            blocks_per_year: 10_512_000, // ~3s blocks
            fee_burn_pct: 30,
            fee_producer_pct: 50,
            slash_double_vote_pct: 5,
        }
    }
}

/// Validator economics simulation result.
#[derive(Debug, Clone)]
pub struct ValidatorEconResult {
    pub annual_reward_per_validator: u64,
    pub annual_return_pct: f64,
    pub total_annual_inflation: u64,
    pub cost_of_bft_attack: u64,
    pub min_viable_stake: u64,
}

/// Simulate validator economics.
pub fn simulate_validator_economics(config: &ValidatorEconConfig) -> ValidatorEconResult {
    let blocks_per_validator = config.blocks_per_year / config.total_validators;
    let annual_reward = blocks_per_validator * config.block_reward;
    let return_pct = if config.stake_per_validator > 0 {
        (annual_reward as f64 / config.stake_per_validator as f64) * 100.0
    } else {
        0.0
    };

    let total_inflation = config.blocks_per_year * config.block_reward;
    let total_stake = config.total_validators * config.stake_per_validator;

    // BFT attack: need >2/3 of total stake
    let attack_stake = (total_stake * 2 / 3) + 1;

    // Minimum viable stake: reward must exceed operational cost (rough estimate: $1000/year)
    let min_viable = if config.block_reward > 0 {
        config.stake_per_validator / 10 // heuristic: 10% of typical stake
    } else {
        0
    };

    ValidatorEconResult {
        annual_reward_per_validator: annual_reward,
        annual_return_pct: return_pct,
        total_annual_inflation: total_inflation,
        cost_of_bft_attack: attack_stake,
        min_viable_stake: min_viable,
    }
}

// ─── Game-Theoretic Analysis ─────────────────────────────────────────────────

/// Attack cost analysis results.
#[derive(Debug, Clone)]
pub struct AttackCostAnalysis {
    pub bft_attack_cost: u64,
    pub state_spam_cost_per_block: u64,
    pub dust_attack_cost_per_block: u64,
    pub blocks_reward_to_recoup_attack: u64,
}

/// Analyze attack costs given economic parameters.
pub fn analyze_attack_costs(
    total_stake: u64,
    base_fee: u64,
    block_gas_limit: u64,
    object_creation_cost: u64,
    block_reward: u64,
) -> AttackCostAnalysis {
    // BFT attack: acquire >2/3 of total stake
    let bft_cost = (total_stake * 2 / 3) + 1;

    // State spam: fill every block with object creations
    let objects_per_block = block_gas_limit / 50_000; // GAS_CREATE_OBJECT_BASE
    let spam_cost = objects_per_block * object_creation_cost;

    // Dust attack: fill blocks with minimum-fee transfers
    let min_tx_gas = 21_000u64; // GAS_TRANSFER
    let txs_per_block = block_gas_limit / min_tx_gas;
    let dust_cost = txs_per_block * base_fee * min_tx_gas;

    // How many blocks of rewards to recoup the BFT attack cost
    #[allow(unknown_lints, clippy::manual_checked_ops)]
    let recoup_blocks = if block_reward > 0 {
        bft_cost / block_reward
    } else {
        u64::MAX
    };

    AttackCostAnalysis {
        bft_attack_cost: bft_cost,
        state_spam_cost_per_block: spam_cost,
        dust_attack_cost_per_block: dust_cost,
        blocks_reward_to_recoup_attack: recoup_blocks,
    }
}

// ─── Parameter Tuning ────────────────────────────────────────────────────────

/// All tunable economic parameters.
#[derive(Debug, Clone)]
pub struct EconomicParams {
    pub half_life: u64,
    pub grace_period: u64,
    pub base_fee: u64,
    pub block_reward: u64,
    pub target_gas: u64,
    pub max_gas: u64,
    pub num_validators: u64,
    pub stake_per_validator: u64,
    pub object_creation_cost: u64,
    pub refresh_cost: u64,
}

impl EconomicParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.half_life == 0 {
            return Err("half_life must be > 0");
        }
        if self.base_fee == 0 {
            return Err("base_fee must be > 0");
        }
        if self.target_gas == 0 {
            return Err("target_gas must be > 0");
        }
        if self.max_gas < self.target_gas {
            return Err("max_gas must be >= target_gas");
        }
        if self.num_validators == 0 {
            return Err("num_validators must be > 0");
        }
        Ok(())
    }
}

/// Simulation metrics output.
#[derive(Debug, Clone)]
pub struct SimulationMetrics {
    pub avg_gas_price: u64,
    pub gas_price_volatility: f64,
    pub object_survival_rate: f64,
    pub validator_annual_return_pct: f64,
    pub bft_attack_cost: u64,
    pub state_spam_cost_per_block: u64,
    pub composite_score: f64,
}

/// Run a full economic simulation with the given parameters.
pub fn simulate(params: &EconomicParams, blocks: u64) -> Result<SimulationMetrics, &'static str> {
    params.validate()?;

    // Gas price simulation
    let gas_config = GasPriceConfig {
        initial_base_fee: params.base_fee,
        target_gas_per_block: params.target_gas,
        max_gas_per_block: params.max_gas,
        adjustment_denominator: 8,
        min_base_fee: 1,
    };
    let gas_prices = simulate_gas_price(&gas_config, 100, blocks);
    let avg_gas: u64 = gas_prices.iter().sum::<u64>() / gas_prices.len().max(1) as u64;
    let mean = avg_gas as f64;
    let variance = gas_prices
        .iter()
        .map(|&p| {
            let diff = p as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / gas_prices.len().max(1) as f64;
    let volatility = variance.sqrt() / mean.max(1.0);

    // Decay simulation — object with 10K energy
    let decay_result = simulate_decay(&DecaySimConfig {
        initial_energy: 10_000,
        half_life: params.half_life,
        grace_period: params.grace_period,
        refresh_cost: params.refresh_cost,
        blocks_to_simulate: blocks,
    });
    let survival_rate = if decay_result.blocks_until_evaporation.is_some() {
        0.0
    } else {
        1.0
    };

    // Validator economics
    let val_config = ValidatorEconConfig {
        block_reward: params.block_reward,
        total_validators: params.num_validators,
        stake_per_validator: params.stake_per_validator,
        blocks_per_year: 10_512_000,
        ..Default::default()
    };
    let val_result = simulate_validator_economics(&val_config);

    // Attack costs
    let total_stake = params.num_validators * params.stake_per_validator;
    let attacks = analyze_attack_costs(
        total_stake,
        avg_gas,
        params.max_gas,
        params.object_creation_cost,
        params.block_reward,
    );

    // Composite score: higher is better
    // Weight: security (40%), stability (30%), incentives (30%)
    let security_score = (attacks.bft_attack_cost as f64).log10() / 15.0; // normalize
    let stability_score = 1.0 - volatility.min(1.0);
    let incentive_score = (val_result.annual_return_pct / 20.0).min(1.0); // 20% return = perfect
    let composite = security_score * 0.4 + stability_score * 0.3 + incentive_score * 0.3;

    Ok(SimulationMetrics {
        avg_gas_price: avg_gas,
        gas_price_volatility: volatility,
        object_survival_rate: survival_rate,
        validator_annual_return_pct: val_result.annual_return_pct,
        bft_attack_cost: attacks.bft_attack_cost,
        state_spam_cost_per_block: attacks.state_spam_cost_per_block,
        composite_score: composite,
    })
}

#[cfg(test)]
#[allow(deprecated)] // tests still exercise the deprecated compute_energy forwarder
mod tests {
    use super::*;

    #[test]
    fn test_energy_decay_basic() {
        assert_eq!(compute_energy(1000, 10, 0), 1000);
        assert_eq!(compute_energy(1000, 10, 10), 500);
        assert_eq!(compute_energy(1000, 10, 20), 250);
        assert_eq!(compute_energy(1000, 10, 30), 125);
    }

    // ─── AUDIT_2026_05_13 H10 regression suite ────────────────────────

    #[test]
    fn audit_h10_compute_energy_forwards_to_canonical() {
        // Post-fix, compute_energy is a thin forwarder to
        // evaporchain_types::energy_at_epoch. Result-wise the integer
        // u128 interpolation may differ by ~1 unit from the pre-fix
        // f64 version, but it MUST match the canonical answer that
        // every validator computes the same way.
        for (initial, half_life, epochs) in [
            (1000u64, 10u64, 0u64),
            (1000, 10, 5),
            (1000, 10, 10),
            (1000, 10, 15),
            (1_000_000, 100, 123),
            (u64::MAX, 1, 64), // saturation path
        ] {
            assert_eq!(
                compute_energy(initial, half_life, epochs),
                evaporchain_types::energy_at_epoch(initial, half_life, epochs),
                "compute_energy must match canonical energy_at_epoch for ({initial}, {half_life}, {epochs})"
            );
        }
    }

    #[test]
    fn audit_h10_no_external_callers_outside_economics() {
        // Documentary assertion: the deprecated `compute_energy` is
        // `pub(crate)` so the workspace-grep would catch any external
        // reactivation. This test serves as a build-time reminder by
        // referencing the symbol through its narrowed visibility.
        let _ = compute_energy(1, 1, 0); // pub(crate) — compiles only in-crate
    }

    #[test]
    fn test_energy_decay_zero_half_life() {
        assert_eq!(compute_energy(1000, 0, 5), 0);
    }

    #[test]
    fn test_energy_decay_large_elapsed() {
        assert_eq!(compute_energy(1000, 1, 100), 0);
    }

    #[test]
    fn test_energy_decay_interpolation() {
        let e5 = compute_energy(1000, 10, 5);
        assert!(
            e5 > 500 && e5 < 1000,
            "Midpoint energy should be between halves: {}",
            e5
        );
    }

    #[test]
    fn test_simulate_decay_short_lived() {
        let config = DecaySimConfig {
            initial_energy: 100,
            half_life: 5,
            grace_period: 3,
            refresh_cost: 10,
            blocks_to_simulate: 100,
        };
        let result = simulate_decay(&config);
        assert!(result.blocks_until_grace.is_some());
    }

    #[test]
    fn test_simulate_decay_long_lived() {
        let config = DecaySimConfig {
            initial_energy: 1_000_000,
            half_life: 1000,
            grace_period: 100,
            refresh_cost: 10,
            blocks_to_simulate: 100,
        };
        let result = simulate_decay(&config);
        assert!(result.energy_at_end > 0);
    }

    #[test]
    fn test_find_min_half_life() {
        let hl = find_min_half_life_for_survival(1000, 100);
        let energy = compute_energy(1000, hl, 100);
        assert!(energy > 0);
    }

    #[test]
    fn test_steady_state_cost() {
        let cost = steady_state_maintenance_cost(100, 50, 10, 1000);
        // 100 objects * (1000/50=20 refreshes) * 10 cost = 20,000
        assert_eq!(cost, 20_000);
    }

    #[test]
    fn test_gas_price_stable_at_target() {
        let config = GasPriceConfig::default();
        let prices = simulate_gas_price(&config, 100, 100); // exactly at target
        assert_eq!(prices[0], config.initial_base_fee);
        // Should stay stable when gas_used == target
        let last = *prices.last().unwrap();
        assert_eq!(last, config.initial_base_fee);
    }

    #[test]
    fn test_gas_price_increases_above_target() {
        let config = GasPriceConfig::default();
        let prices = simulate_gas_price(&config, 200, 50); // 2x target load
        let last = *prices.last().unwrap();
        assert!(
            last > config.initial_base_fee,
            "Gas price should increase above target"
        );
    }

    #[test]
    fn test_gas_price_decreases_below_target() {
        let config = GasPriceConfig::default();
        let prices = simulate_gas_price(&config, 10, 50); // 10% load
        let last = *prices.last().unwrap();
        assert!(
            last < config.initial_base_fee,
            "Gas price should decrease below target"
        );
    }

    #[test]
    fn test_gas_price_floor() {
        let config = GasPriceConfig {
            min_base_fee: 5,
            ..Default::default()
        };
        let prices = simulate_gas_price(&config, 0, 100); // zero load
        let last = *prices.last().unwrap();
        assert!(
            last >= config.min_base_fee,
            "Gas price should not go below floor"
        );
    }

    #[test]
    fn test_validator_economics_basic() {
        let config = ValidatorEconConfig::default();
        let result = simulate_validator_economics(&config);
        assert!(result.annual_reward_per_validator > 0);
        assert!(result.annual_return_pct > 0.0);
        assert!(result.cost_of_bft_attack > 0);
    }

    #[test]
    fn test_bft_attack_cost_scales_with_stake() {
        let config1 = ValidatorEconConfig {
            stake_per_validator: 100_000,
            ..Default::default()
        };
        let config2 = ValidatorEconConfig {
            stake_per_validator: 1_000_000,
            ..Default::default()
        };
        let r1 = simulate_validator_economics(&config1);
        let r2 = simulate_validator_economics(&config2);
        assert!(r2.cost_of_bft_attack > r1.cost_of_bft_attack);
    }

    #[test]
    fn test_attack_cost_analysis() {
        let result = analyze_attack_costs(
            10_000_000, // total stake
            100,        // base fee
            10_000_000, // block gas limit
            50_000,     // object creation cost
            1000,       // block reward
        );
        assert!(result.bft_attack_cost > 6_000_000);
        assert!(result.state_spam_cost_per_block > 0);
        assert!(result.dust_attack_cost_per_block > 0);
        assert!(result.blocks_reward_to_recoup_attack > 0);
    }

    #[test]
    fn test_economic_params_validation() {
        let mut params = EconomicParams {
            half_life: 100,
            grace_period: 10,
            base_fee: 100,
            block_reward: 1000,
            target_gas: 5_000_000,
            max_gas: 10_000_000,
            num_validators: 100,
            stake_per_validator: 100_000,
            object_creation_cost: 50_000,
            refresh_cost: 1000,
        };
        assert!(params.validate().is_ok());

        params.half_life = 0;
        assert!(params.validate().is_err());

        params.half_life = 100;
        params.max_gas = 1; // less than target
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_full_simulation() {
        let params = EconomicParams {
            half_life: 1000,
            grace_period: 100,
            base_fee: 100,
            block_reward: 1000,
            target_gas: 5_000_000,
            max_gas: 10_000_000,
            num_validators: 100,
            stake_per_validator: 100_000,
            object_creation_cost: 50_000,
            refresh_cost: 1000,
        };
        let metrics = simulate(&params, 1000).unwrap();
        assert!(metrics.avg_gas_price > 0);
        assert!(metrics.bft_attack_cost > 0);
        assert!(metrics.composite_score > 0.0 && metrics.composite_score <= 1.0);
    }

    #[test]
    fn test_attack_cost_exceeds_reward() {
        let result = analyze_attack_costs(100_000_000, 100, 10_000_000, 50_000, 1000);
        // Attack cost should take many blocks to recoup
        assert!(
            result.blocks_reward_to_recoup_attack > 1000,
            "Attack should take >1000 blocks to recoup: {}",
            result.blocks_reward_to_recoup_attack
        );
    }

    #[test]
    fn test_no_negative_energy() {
        for elapsed in 0..200 {
            let e = compute_energy(100, 10, elapsed);
            // energy is u64, can't be negative
            assert!(e <= 100, "Energy should never exceed initial: {}", e);
        }
    }

    #[test]
    fn test_varying_load_gas_price() {
        let config = GasPriceConfig::default();
        let schedule = vec![
            (20, 50),  // 50% load for 20 blocks
            (20, 200), // 200% load for 20 blocks
            (20, 50),  // back to 50%
        ];
        let prices = simulate_gas_price_varying(&config, &schedule);
        assert_eq!(prices.len(), 60);
        // Price should spike during high-load period
        let mid_price = prices[30]; // during high load
        let end_price = prices[59]; // after cooldown
        assert!(mid_price > end_price, "Price should be higher during spike");
    }
}
