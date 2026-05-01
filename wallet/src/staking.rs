//! Staking and governance management.
//!
//! Provides a high-level view of staking pools, user positions,
//! reward forecasting with decay, and DAO proposal participation.

use evaporchain_types::energy_at_epoch;
use thiserror::Error;

use crate::rpc::{
    ClaimRequest, ProposalResponse, ProposeRequest, RpcClient, RpcError, StakeRequest,
    StakingPoolResponse, TxResultResponse, UnstakeRequest, VoteRequest,
};

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum StakingError {
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError),
    #[error("not staked in pool {0}")]
    NotStaked(u64),
    #[error("pool not found: {0}")]
    PoolNotFound(u64),
}

// ──────────────────────────── Stake Position ───────────────────────────

/// A user's position in a staking pool.
#[derive(Debug, Clone)]
pub struct StakePosition {
    pub pool_id: u64,
    pub pool_name: String,
    pub staked_amount: u64,
    pub pending_rewards: u64,
    pub reward_decay_pct: f64,
    pub staked_epoch: u64,
    pub total_claimed: u64,
    pub total_decayed: u64,
}

/// Reward forecast showing gross and net rewards after decay.
#[derive(Debug, Clone)]
pub struct RewardForecast {
    pub epochs_forward: u64,
    pub gross_rewards: u64,
    pub decay_loss: u64,
    pub net_rewards: u64,
}

// ──────────────────────────── Staking Manager ──────────────────────────

/// Manages staking pool queries and operations.
pub struct StakingManager {
    rpc: RpcClient,
}

impl StakingManager {
    pub fn new(rpc: RpcClient) -> Self {
        Self { rpc }
    }

    /// List all staking pools.
    pub async fn list_pools(&self) -> Result<Vec<StakingPoolResponse>, StakingError> {
        Ok(self.rpc.get_staking_pools().await?)
    }

    /// Get a specific pool.
    pub async fn get_pool(&self, id: u64) -> Result<StakingPoolResponse, StakingError> {
        self.rpc
            .get_staking_pool(id)
            .await
            .map_err(StakingError::from)
    }

    /// Get all staking positions for an address across all pools.
    pub async fn my_positions(&self, address: &str) -> Result<Vec<StakePosition>, StakingError> {
        let pools = self.rpc.get_staking_pools().await?;
        let mut positions = Vec::new();

        for pool in &pools {
            for staker in &pool.stakers {
                if staker.address == address || staker.address.contains(address) {
                    positions.push(StakePosition {
                        pool_id: pool.id,
                        pool_name: pool.name.clone(),
                        staked_amount: staker.amount,
                        pending_rewards: staker.pending_rewards,
                        reward_decay_pct: staker.reward_decay_pct,
                        staked_epoch: staker.staked_epoch,
                        total_claimed: staker.total_claimed,
                        total_decayed: staker.total_decayed,
                    });
                }
            }
        }

        Ok(positions)
    }

    /// Stake tokens in a pool.
    pub async fn stake(
        &self,
        pool_id: u64,
        address: &str,
        amount: u64,
    ) -> Result<TxResultResponse, StakingError> {
        let req = StakeRequest {
            pool_id,
            address: address.to_string(),
            amount,
        };
        Ok(self.rpc.stake(&req).await?)
    }

    /// Unstake tokens from a pool.
    pub async fn unstake(
        &self,
        pool_id: u64,
        address: &str,
        amount: u64,
    ) -> Result<TxResultResponse, StakingError> {
        let req = UnstakeRequest {
            pool_id,
            address: address.to_string(),
            amount,
        };
        Ok(self.rpc.unstake(&req).await?)
    }

    /// Claim pending rewards from a pool.
    pub async fn claim(
        &self,
        pool_id: u64,
        address: &str,
    ) -> Result<TxResultResponse, StakingError> {
        let req = ClaimRequest {
            pool_id,
            address: address.to_string(),
        };
        Ok(self.rpc.claim_rewards(&req).await?)
    }

    /// Forecast rewards for a given stake over time, accounting for reward decay.
    ///
    /// `reward_rate` and `reward_decay_hl` come from the pool.
    /// `total_staked` is the pool's total staked amount (for share calculation).
    pub fn reward_forecast(
        staked_amount: u64,
        reward_rate: u64,
        reward_decay_hl: u64,
        total_staked: u64,
        epochs_forward: u64,
    ) -> RewardForecast {
        if total_staked == 0 || staked_amount == 0 || reward_rate == 0 {
            return RewardForecast {
                epochs_forward,
                gross_rewards: 0,
                decay_loss: 0,
                net_rewards: 0,
            };
        }

        let share = staked_amount as f64 / total_staked as f64;
        let gross_per_epoch = (share * reward_rate as f64) as u64;
        let gross = gross_per_epoch.saturating_mul(epochs_forward);

        // Net rewards after decay: energy_at_epoch models how rewards decay
        let net = if reward_decay_hl > 0 {
            energy_at_epoch(gross, reward_decay_hl, epochs_forward)
        } else {
            gross
        };

        RewardForecast {
            epochs_forward,
            gross_rewards: gross,
            decay_loss: gross.saturating_sub(net),
            net_rewards: net,
        }
    }
}

// ──────────────────────────── Governance Manager ───────────────────────

/// Manages DAO proposals and voting.
pub struct GovernanceManager {
    rpc: RpcClient,
}

impl GovernanceManager {
    pub fn new(rpc: RpcClient) -> Self {
        Self { rpc }
    }

    /// List all proposals.
    pub async fn list_proposals(&self) -> Result<Vec<ProposalResponse>, StakingError> {
        Ok(self.rpc.get_proposals().await?)
    }

    /// Get a specific proposal.
    pub async fn get_proposal(&self, id: u64) -> Result<ProposalResponse, StakingError> {
        Ok(self.rpc.get_proposal(id).await?)
    }

    /// Create a new proposal.
    pub async fn create_proposal(
        &self,
        title: &str,
        description: &str,
        options: Vec<String>,
        voting_period: u64,
        creator: Option<&str>,
    ) -> Result<TxResultResponse, StakingError> {
        let req = ProposeRequest {
            title: title.to_string(),
            description: description.to_string(),
            options,
            voting_period,
            creator: creator.map(|s| s.to_string()),
        };
        Ok(self.rpc.create_proposal(&req).await?)
    }

    /// Vote on a proposal.
    pub async fn vote(
        &self,
        proposal_id: u64,
        option: &str,
        weight: u64,
        voter: Option<&str>,
    ) -> Result<TxResultResponse, StakingError> {
        let req = VoteRequest {
            proposal_id,
            option: option.to_string(),
            weight,
            voter: voter.map(|s| s.to_string()),
        };
        Ok(self.rpc.vote(&req).await?)
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reward_forecast_basic() {
        // 10000 staked in pool with 100k total, reward rate 100/epoch, decay hl 1000
        let forecast = StakingManager::reward_forecast(10000, 100, 1000, 100000, 100);

        assert_eq!(forecast.epochs_forward, 100);
        // share = 10%, gross per epoch = 10, gross over 100 epochs = 1000
        assert_eq!(forecast.gross_rewards, 1000);
        // With decay hl 1000, 100 epochs is 1/10 of half-life, so ~93% remains
        assert!(forecast.net_rewards > 0);
        assert!(forecast.net_rewards <= forecast.gross_rewards);
        assert_eq!(
            forecast.decay_loss,
            forecast.gross_rewards - forecast.net_rewards
        );
    }

    #[test]
    fn test_reward_forecast_zero_stake() {
        let forecast = StakingManager::reward_forecast(0, 100, 1000, 100000, 100);
        assert_eq!(forecast.gross_rewards, 0);
        assert_eq!(forecast.net_rewards, 0);
    }

    #[test]
    fn test_reward_forecast_zero_pool() {
        let forecast = StakingManager::reward_forecast(10000, 100, 1000, 0, 100);
        assert_eq!(forecast.gross_rewards, 0);
    }

    #[test]
    fn test_reward_forecast_no_decay() {
        // reward_decay_hl = 0 means no decay
        let forecast = StakingManager::reward_forecast(10000, 100, 0, 100000, 100);
        assert_eq!(forecast.gross_rewards, forecast.net_rewards);
        assert_eq!(forecast.decay_loss, 0);
    }

    #[test]
    fn test_reward_forecast_long_duration() {
        // Over many half-lives, decay should eat most rewards
        let forecast = StakingManager::reward_forecast(50000, 200, 100, 100000, 1000);

        assert!(forecast.decay_loss > forecast.net_rewards);
    }

    #[test]
    fn test_stake_position_from_pool_data() {
        let pos = StakePosition {
            pool_id: 1,
            pool_name: "Main Pool".to_string(),
            staked_amount: 50000,
            pending_rewards: 250,
            reward_decay_pct: 5.0,
            staked_epoch: 10,
            total_claimed: 100,
            total_decayed: 20,
        };

        assert_eq!(pos.pool_id, 1);
        assert_eq!(pos.staked_amount, 50000);
        assert_eq!(pos.pending_rewards, 250);
    }
}
