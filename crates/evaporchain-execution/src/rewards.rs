//! Block rewards and validator staking reward distribution.
//!
//! Implements two reward mechanisms:
//! 1. **Block rewards** — Minted tokens paid to the block producer, with
//!    exponential halving schedule.
//! 2. **Fee distribution** — Non-burned fee revenue split between producer
//!    and stakers according to tokenomics parameters.

use evaporchain_state::db::StateDB;
use evaporchain_types::genesis::{FeeDistribution, Tokenomics};
use evaporchain_types::{AccountAddress, Epoch};
use tracing::{debug, info};

/// Tracks cumulative staker reward pool for proportional distribution.
#[derive(Debug, Clone)]
pub struct RewardAccumulator {
    /// Tokenomics parameters for reward calculation.
    pub tokenomics: Tokenomics,
    /// Total tokens minted as block rewards so far.
    pub total_minted: u64,
    /// Total fees burned so far.
    pub total_burned: u64,
    /// Total fees paid to producers.
    pub total_to_producers: u64,
    /// Total fees distributed to stakers.
    pub total_to_stakers: u64,
    /// Undistributed staker rewards (accumulated until claimed or distributed).
    pub pending_staker_rewards: u64,
}

impl RewardAccumulator {
    /// Create a new reward accumulator with the given tokenomics.
    pub fn new(tokenomics: Tokenomics) -> Self {
        Self {
            tokenomics,
            total_minted: 0,
            total_burned: 0,
            total_to_producers: 0,
            total_to_stakers: 0,
            pending_staker_rewards: 0,
        }
    }

    /// Process rewards for a single block.
    ///
    /// 1. Mints block reward to the producer.
    /// 2. Distributes collected fees according to tokenomics.
    ///
    /// Returns the total tokens credited to the producer this block.
    pub fn process_block_rewards(
        &mut self,
        db: &mut dyn StateDB,
        producer: &AccountAddress,
        epoch: Epoch,
        total_fees_collected: u64,
    ) -> u64 {
        let mut producer_credit = 0u64;

        // 1. Block reward (minted)
        let block_reward = self.tokenomics.reward_at_epoch(epoch);
        if block_reward > 0 {
            let acct = db.get_or_create_account(producer);
            acct.balance = acct.balance.saturating_add(block_reward);
            self.total_minted = self.total_minted.saturating_add(block_reward);
            producer_credit += block_reward;
            debug!(
                producer = hex::encode(producer),
                reward = block_reward,
                epoch,
                "Block reward minted"
            );
        }

        // 2. Fee distribution
        if total_fees_collected > 0 {
            let dist = self.tokenomics.distribute_fees(total_fees_collected);
            self.total_burned = self.total_burned.saturating_add(dist.burned);

            // Producer share
            if dist.to_producer > 0 {
                let acct = db.get_or_create_account(producer);
                acct.balance = acct.balance.saturating_add(dist.to_producer);
                self.total_to_producers = self.total_to_producers.saturating_add(dist.to_producer);
                producer_credit += dist.to_producer;
            }

            // Staker share (accumulated for proportional distribution)
            if dist.to_stakers > 0 {
                self.pending_staker_rewards = self
                    .pending_staker_rewards
                    .saturating_add(dist.to_stakers);
                self.total_to_stakers = self.total_to_stakers.saturating_add(dist.to_stakers);
            }

            debug!(
                burned = dist.burned,
                to_producer = dist.to_producer,
                to_stakers = dist.to_stakers,
                "Fees distributed"
            );
        }

        if producer_credit > 0 {
            info!(
                producer = hex::encode(producer),
                block_reward,
                fee_share = producer_credit.saturating_sub(block_reward),
                total = producer_credit,
                "Producer rewarded"
            );
        }

        producer_credit
    }

    /// Distribute accumulated staker rewards proportionally to a set of stakers.
    ///
    /// Each staker receives `pending_staker_rewards * (staker_stake / total_stake)`.
    /// Call this periodically (e.g., every N blocks) or when a staker claims.
    pub fn distribute_staker_rewards(
        &mut self,
        db: &mut dyn StateDB,
        stakers: &[(AccountAddress, u64)], // (address, stake)
    ) -> u64 {
        if self.pending_staker_rewards == 0 || stakers.is_empty() {
            return 0;
        }

        let total_stake: u64 = stakers.iter().map(|(_, s)| *s).sum();
        if total_stake == 0 {
            return 0;
        }

        let mut distributed = 0u64;
        let pool = self.pending_staker_rewards;

        for (addr, stake) in stakers {
            // Proportional share using u128 to avoid overflow
            let share = (pool as u128 * *stake as u128 / total_stake as u128) as u64;
            if share > 0 {
                let acct = db.get_or_create_account(addr);
                acct.balance = acct.balance.saturating_add(share);
                distributed += share;
            }
        }

        // Any remainder stays in pending (rounding dust)
        self.pending_staker_rewards = self.pending_staker_rewards.saturating_sub(distributed);
        distributed
    }

    /// Get a summary of the current reward state.
    pub fn summary(&self) -> RewardSummary {
        RewardSummary {
            total_minted: self.total_minted,
            total_burned: self.total_burned,
            total_to_producers: self.total_to_producers,
            total_to_stakers: self.total_to_stakers,
            pending_staker_rewards: self.pending_staker_rewards,
            net_supply_change: self.total_minted as i64 - self.total_burned as i64,
        }
    }
}

/// Summary of reward distribution state.
#[derive(Debug, Clone)]
pub struct RewardSummary {
    pub total_minted: u64,
    pub total_burned: u64,
    pub total_to_producers: u64,
    pub total_to_stakers: u64,
    pub pending_staker_rewards: u64,
    /// Net token supply change (minted - burned). Negative = deflationary.
    pub net_supply_change: i64,
}

// ─────────────────────── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::genesis::Tokenomics;

    fn addr(byte: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = byte;
        a
    }

    fn fund(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
        use evaporchain_types::Account;
        db.put_account(Account {
            address: addr(byte),
            balance,
            nonce: 0,
        });
    }

    fn test_tokenomics() -> Tokenomics {
        Tokenomics {
            total_supply: 10_000_000,
            block_reward: 100,
            reward_half_life: 1000,
            fee_burn_rate: 0.50,
            staker_fee_share: 0.50,
            target_staking_apy: 0.05,
        }
    }

    #[test]
    fn test_block_reward_minted_to_producer() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 0);
        assert_eq!(credit, 100);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 100);
        assert_eq!(acc.total_minted, 100);
    }

    #[test]
    fn test_block_reward_halves() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        // At epoch 1000 (one half-life), reward should be 50
        let credit = acc.process_block_rewards(&mut db, &addr(1), 1000, 0);
        assert_eq!(credit, 50);
    }

    #[test]
    fn test_fee_distribution_to_producer_and_stakers() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        // 1000 fees: 500 burned, 250 to producer, 250 to stakers
        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 1000);
        // Producer gets block_reward(100) + fee_share(250) = 350
        assert_eq!(credit, 350);
        assert_eq!(acc.total_burned, 500);
        assert_eq!(acc.pending_staker_rewards, 250);
    }

    #[test]
    fn test_staker_reward_distribution() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        fund(&mut db, 10, 0);
        fund(&mut db, 20, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        // Accumulate some staker rewards
        acc.process_block_rewards(&mut db, &addr(1), 0, 2000);
        // 2000 * 0.5 = 1000 burned, 500 to producer, 500 to stakers
        assert_eq!(acc.pending_staker_rewards, 500);

        // Distribute to two stakers with 75/25 split
        let stakers = vec![(addr(10), 750), (addr(20), 250)];
        let distributed = acc.distribute_staker_rewards(&mut db, &stakers);
        assert_eq!(distributed, 500);
        assert_eq!(db.get_account(&addr(10)).unwrap().balance, 375); // 500 * 750/1000
        assert_eq!(db.get_account(&addr(20)).unwrap().balance, 125); // 500 * 250/1000
        assert_eq!(acc.pending_staker_rewards, 0);
    }

    #[test]
    fn test_zero_fees_no_distribution() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 0);
        assert_eq!(credit, 100); // just block reward
        assert_eq!(acc.total_burned, 0);
        assert_eq!(acc.pending_staker_rewards, 0);
    }

    #[test]
    fn test_all_fees_burned() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(Tokenomics {
            fee_burn_rate: 1.0,
            ..test_tokenomics()
        });

        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 1000);
        assert_eq!(credit, 100); // just block reward, all fees burned
        assert_eq!(acc.total_burned, 1000);
        assert_eq!(acc.pending_staker_rewards, 0);
    }

    #[test]
    fn test_no_block_reward() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(Tokenomics {
            block_reward: 0,
            ..test_tokenomics()
        });

        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 1000);
        // Only fee share: 1000 * 0.5 = 500 burned, 250 to producer
        assert_eq!(credit, 250);
    }

    #[test]
    fn test_reward_summary() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        acc.process_block_rewards(&mut db, &addr(1), 0, 2000);
        let summary = acc.summary();
        assert_eq!(summary.total_minted, 100);
        assert_eq!(summary.total_burned, 1000);
        assert_eq!(summary.net_supply_change, -900); // deflationary!
    }

    #[test]
    fn test_multi_block_accumulation() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        for epoch in 0..10u64 {
            acc.process_block_rewards(&mut db, &addr(1), epoch, 100);
        }
        // 10 blocks × 100 reward + fee shares
        assert!(acc.total_minted >= 1000);
        assert!(db.get_account(&addr(1)).unwrap().balance > 1000);
    }
}
