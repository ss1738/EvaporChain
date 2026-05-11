//! Block rewards and validator staking reward distribution.
//!
//! Implements two reward mechanisms:
//! 1. **Block rewards** — Minted tokens paid to the block producer, with
//!    exponential halving schedule.
//! 2. **Fee distribution** — Non-burned fee revenue split between producer
//!    and stakers according to tokenomics parameters.

use evaporchain_energy_kernel::RefreshPool;
use evaporchain_state::db::StateDB;
use evaporchain_types::genesis::Tokenomics;
use evaporchain_types::{AccountAddress, Epoch};
use tracing::{debug, info};

/// Refresh-pool namespace for energy redirected away from tombstoned
/// producers. Distinct from the rent-exhaustion bucket
/// (`SYSTEM_REFRESH_NAMESPACE` = `b"evaporchain-system-refresh"`) so
/// `/api/four_act` can surface the dead-producer accrual independently
/// — that makes the doctrine "the chain's death is final" empirically
/// auditable instead of commingled with rent-driven accruals.
const DEAD_PRODUCER_REFRESH_NAMESPACE: &[u8] = b"evaporchain-dead-producer-refresh";

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

    /// Mint a proposer-priority bonus equal to
    /// `priority_sum.saturating_div(scale_per_unit)` tokens.
    ///
    /// This is Phase-1.5 of the energy-stamped MEV defense
    /// (`research/proposals/energy-stamped-mev-resistance.md`): without
    /// this, the proposer has no economic reason to honour the priority
    /// ordering. The bonus is computed AFTER `take_with_priority` selects
    /// the block's txs, using the priority sum returned by
    /// `Mempool::take_with_priority_and_sum`.
    ///
    /// `scale_per_unit` is a divisor that translates raw priority units
    /// (~`BASE_INCLUSION_ENERGY` per fresh-arrival tx, decayed by the
    /// inclusion half-life) into native tokens. A larger divisor means a
    /// smaller bonus per priority unit. Defaults are operator-tunable;
    /// passing 0 disables the bonus entirely (no-op).
    ///
    /// Returns the bonus actually credited (zero on no-op or empty pool).
    pub fn apply_priority_bonus(
        &mut self,
        db: &mut dyn StateDB,
        producer: &AccountAddress,
        epoch: Epoch,
        priority_sum: u64,
        scale_per_unit: u64,
        producer_alive: bool,
        refresh_pool_for_dead_producer: Option<&mut RefreshPool>,
    ) -> u64 {
        if scale_per_unit == 0 || priority_sum == 0 {
            return 0;
        }
        let bonus = priority_sum / scale_per_unit;
        if bonus == 0 {
            return 0;
        }
        self.total_minted = self.total_minted.saturating_add(bonus);
        if producer_alive {
            let acct = db.get_or_create_account(producer);
            acct.balance = acct.balance.saturating_add(bonus);
            acct.last_touched_epoch = epoch;
            debug!(
                producer = hex::encode(producer),
                priority_sum, scale_per_unit, bonus, epoch, "Proposer priority bonus minted"
            );
            bonus
        } else if let Some(pool) = refresh_pool_for_dead_producer {
            pool.accrue(DEAD_PRODUCER_REFRESH_NAMESPACE.to_vec(), bonus, epoch);
            info!(
                producer = hex::encode(producer),
                bonus,
                epoch,
                "Priority bonus redirected to refresh_pool: producer is tombstoned"
            );
            // Caller's "credited to producer" semantics: 0 since the
            // producer received nothing.
            0
        } else {
            0
        }
    }

    /// Process rewards for a single block.
    ///
    /// 1. Mints block reward to the producer.
    /// 2. Distributes collected fees according to tokenomics.
    ///
    /// `producer_alive` is `false` iff the producer's address is
    /// memorialised in the eulogy trie (i.e. the account has been
    /// tombstoned). Per `evaporchain-tombstone::EulogyTrie` doctrine
    /// "the chain's death is final" — a tombstoned account must not
    /// receive credits. When `producer_alive == false`, the producer-
    /// bound block reward and fee share are redirected into
    /// `refresh_pool_for_dead_producer` instead, preserving §1.2
    /// energy conservation. Staker share is unaffected (it is not
    /// account-bound).
    ///
    /// Returns the total tokens credited to the producer this block
    /// (always 0 if the producer is tombstoned).
    pub fn process_block_rewards(
        &mut self,
        db: &mut dyn StateDB,
        producer: &AccountAddress,
        epoch: Epoch,
        total_fees_collected: u64,
        producer_alive: bool,
        refresh_pool_for_dead_producer: Option<&mut RefreshPool>,
    ) -> u64 {
        let mut producer_credit = 0u64;
        // Reusable redirect closure: take the producer-bound amount and
        // accrue into refresh_pool under the dead-producer namespace if
        // both (a) producer is dead and (b) the pool was supplied.
        let mut redirect_pool = refresh_pool_for_dead_producer;

        // 1. Block reward (minted). Dispatches via
        //    `Tokenomics::block_reward`:
        //      * `emission: Some(_)` ⇒ rich schedule (Constant /
        //        Halving / LinearDecay) via
        //        `evaporchain_types::emission::block_reward_at` —
        //        TOKENOMICS §2.4 / Q4 closure.
        //      * `emission: None` (default for existing genesis files)
        //        ⇒ legacy `reward_at_epoch_capped` path. Both
        //        respect their own cap mechanism.
        let block_reward = self.tokenomics.block_reward(epoch, self.total_minted);
        if block_reward > 0 {
            self.total_minted = self.total_minted.saturating_add(block_reward);
            if producer_alive {
                let acct = db.get_or_create_account(producer);
                acct.balance = acct.balance.saturating_add(block_reward);
                // Reward credit refreshes the demurrage anchor — block-producing
                // validators are visibly active.
                acct.last_touched_epoch = epoch;
                producer_credit += block_reward;
                debug!(
                    producer = hex::encode(producer),
                    reward = block_reward,
                    epoch,
                    "Block reward minted"
                );
            } else if let Some(pool) = redirect_pool.as_deref_mut() {
                pool.accrue(
                    DEAD_PRODUCER_REFRESH_NAMESPACE.to_vec(),
                    block_reward,
                    epoch,
                );
                info!(
                    producer = hex::encode(producer),
                    reward = block_reward,
                    epoch,
                    "Block reward redirected to refresh_pool: producer is tombstoned"
                );
            }
            // If !producer_alive and no pool was supplied, the
            // block_reward is still counted in total_minted (it was
            // notionally minted) but lands nowhere. Production call
            // sites always supply the pool so this branch is unreachable;
            // it exists only to keep the function total over its inputs.
        }

        // 2. Fee distribution
        if total_fees_collected > 0 {
            let dist = self.tokenomics.distribute_fees(total_fees_collected);
            self.total_burned = self.total_burned.saturating_add(dist.burned);

            // Producer share
            if dist.to_producer > 0 {
                self.total_to_producers = self.total_to_producers.saturating_add(dist.to_producer);
                if producer_alive {
                    let acct = db.get_or_create_account(producer);
                    acct.balance = acct.balance.saturating_add(dist.to_producer);
                    acct.last_touched_epoch = epoch;
                    producer_credit += dist.to_producer;
                } else if let Some(pool) = redirect_pool.as_deref_mut() {
                    pool.accrue(
                        DEAD_PRODUCER_REFRESH_NAMESPACE.to_vec(),
                        dist.to_producer,
                        epoch,
                    );
                }
            }

            // Staker share (accumulated for proportional distribution).
            // Not account-bound; unaffected by producer tombstone state.
            if dist.to_stakers > 0 {
                self.pending_staker_rewards =
                    self.pending_staker_rewards.saturating_add(dist.to_stakers);
                self.total_to_stakers = self.total_to_stakers.saturating_add(dist.to_stakers);
            }

            debug!(
                burned = dist.burned,
                to_producer = dist.to_producer,
                to_stakers = dist.to_stakers,
                producer_alive,
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

    /// Process block rewards with TOKENOMICS §2.1 / §2.5 ceremony decisions:
    /// - 60% of block reward to proposer, 40% split equally among `attesters`
    /// - `total_staked` drives the APY-cap controller (§2.5)
    ///
    /// When `attesters` is empty or `total_staked` is zero, falls back to
    /// the original `process_block_rewards` behaviour (100% to proposer,
    /// no APY cap). Existing call sites can migrate incrementally.
    #[allow(clippy::too_many_arguments)]
    pub fn process_block_rewards_v2(
        &mut self,
        db: &mut dyn StateDB,
        producer: &AccountAddress,
        epoch: Epoch,
        total_fees_collected: u64,
        producer_alive: bool,
        refresh_pool_for_dead_producer: Option<&mut RefreshPool>,
        attesters: &[AccountAddress],
        total_staked: u64,
    ) -> u64 {
        // APY cap: scale the raw block reward down if staking yield would
        // exceed target_staking_apy. TOKENOMICS §2.5 / Q21.
        let raw_reward = self.tokenomics.block_reward(epoch, self.total_minted);
        let capped_reward = self.tokenomics.apy_capped_reward(raw_reward, total_staked);

        // Temporarily patch tokenomics so process_block_rewards uses the
        // capped value. We restore afterward.
        let orig_block_reward = self.tokenomics.block_reward;
        // Scale the base field proportionally if not using the emission path.
        if self.tokenomics.emission.is_none() && raw_reward > 0 {
            self.tokenomics.block_reward = capped_reward
                .saturating_mul(orig_block_reward)
                .checked_div(raw_reward)
                .unwrap_or(capped_reward);
        }

        let mut redirect_pool = refresh_pool_for_dead_producer;
        let mut producer_credit = 0u64;
        let block_reward = self.tokenomics.block_reward(epoch, self.total_minted);

        if block_reward > 0 {
            // 60/40 split only when attesters are present.
            let (proposer_share, attester_total) = if attesters.is_empty() {
                (block_reward, 0u64)
            } else {
                let p = block_reward * 60 / 100;
                (p, block_reward.saturating_sub(p))
            };

            self.total_minted = self.total_minted.saturating_add(block_reward);

            // Credit proposer share
            if proposer_share > 0 {
                if producer_alive {
                    let acct = db.get_or_create_account(producer);
                    acct.balance = acct.balance.saturating_add(proposer_share);
                    acct.last_touched_epoch = epoch;
                    producer_credit += proposer_share;
                } else if let Some(pool) = redirect_pool.as_deref_mut() {
                    pool.accrue(DEAD_PRODUCER_REFRESH_NAMESPACE.to_vec(), proposer_share, epoch);
                }
            }

            // Credit attester shares (40% split equally; dust to first attester)
            if attester_total > 0 && !attesters.is_empty() {
                let per_attester = attester_total / attesters.len() as u64;
                let mut dust = attester_total.saturating_sub(per_attester * attesters.len() as u64);
                for (i, att) in attesters.iter().enumerate() {
                    let share = if i == 0 { per_attester + dust } else { per_attester };
                    if i == 0 { dust = 0; }
                    if share > 0 {
                        let acct = db.get_or_create_account(att);
                        acct.balance = acct.balance.saturating_add(share);
                        acct.last_touched_epoch = epoch;
                    }
                }
            }
        }

        // Fee distribution — TOKENOMICS §2.2: validator commission applied to
        // the staker pool before delegators receive their pro-rata share.
        if total_fees_collected > 0 {
            let dist = self.tokenomics.distribute_fees(total_fees_collected);
            self.total_burned = self.total_burned.saturating_add(dist.burned);
            if dist.to_producer > 0 {
                self.total_to_producers = self.total_to_producers.saturating_add(dist.to_producer);
                if producer_alive {
                    let acct = db.get_or_create_account(producer);
                    acct.balance = acct.balance.saturating_add(dist.to_producer);
                    acct.last_touched_epoch = epoch;
                    producer_credit += dist.to_producer;
                } else if let Some(pool) = redirect_pool.as_deref_mut() {
                    pool.accrue(DEAD_PRODUCER_REFRESH_NAMESPACE.to_vec(), dist.to_producer, epoch);
                }
            }
            if dist.to_stakers > 0 {
                // §2.2: split staker pool into delegator share + validator commission.
                let (net_staker, commission) = self.tokenomics.split_staker_pool(dist.to_stakers);
                // Validator commission goes to the producer (same tombstone logic).
                if commission > 0 {
                    if producer_alive {
                        let acct = db.get_or_create_account(producer);
                        acct.balance = acct.balance.saturating_add(commission);
                        acct.last_touched_epoch = epoch;
                        producer_credit += commission;
                    } else if let Some(pool) = redirect_pool.as_deref_mut() {
                        pool.accrue(
                            DEAD_PRODUCER_REFRESH_NAMESPACE.to_vec(),
                            commission,
                            epoch,
                        );
                    }
                    self.total_to_producers =
                        self.total_to_producers.saturating_add(commission);
                }
                // Net delegator share goes to the pending staker rewards pool.
                if net_staker > 0 {
                    self.pending_staker_rewards =
                        self.pending_staker_rewards.saturating_add(net_staker);
                    self.total_to_stakers =
                        self.total_to_stakers.saturating_add(net_staker);
                }
            }
        }

        // Restore patched field
        self.tokenomics.block_reward = orig_block_reward;
        producer_credit
    }

    /// Distribute accumulated staker rewards proportionally to a set of stakers.
    ///
    /// Each staker receives `pending_staker_rewards * (staker_stake / total_stake)`.
    /// Call this periodically (e.g., every N blocks) or when a staker claims.
    /// `epoch` is the current block's epoch and is stamped onto every
    /// credited account's `last_touched_epoch` to refresh its demurrage
    /// anchor — staker-reward credits are explicit balance changes.
    pub fn distribute_staker_rewards(
        &mut self,
        db: &mut dyn StateDB,
        stakers: &[(AccountAddress, u64)], // (address, stake)
        epoch: u64,
    ) -> u64 {
        if self.pending_staker_rewards == 0 || stakers.is_empty() {
            return 0;
        }

        let total_stake: u64 = stakers
            .iter()
            .map(|(_, s)| *s)
            .fold(0u64, |a, s| a.saturating_add(s));
        if total_stake == 0 {
            return 0;
        }

        let mut distributed = 0u64;
        let pool = self.pending_staker_rewards;

        for (addr, stake) in stakers {
            let share = (pool as u128 * *stake as u128 / total_stake as u128) as u64;
            if share > 0 {
                let acct = db.get_or_create_account(addr);
                acct.balance = acct.balance.saturating_add(share);
                acct.last_touched_epoch = epoch;
                distributed = distributed.saturating_add(share);
            }
        }

        // Distribute rounding dust to highest-stake stakers (1 unit each)
        let mut remainder = pool.saturating_sub(distributed);
        if remainder > 0 {
            let mut sorted: Vec<&(AccountAddress, u64)> = stakers.iter().collect();
            sorted.sort_by_key(|a| std::cmp::Reverse(a.1));
            for (addr, _) in sorted {
                if remainder == 0 {
                    break;
                }
                let acct = db.get_or_create_account(addr);
                acct.balance = acct.balance.saturating_add(1);
                acct.last_touched_epoch = epoch;
                distributed = distributed.saturating_add(1);
                remainder -= 1;
            }
        }

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
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
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
            // Zero commission keeps existing tests unaffected; §2.2 tests
            // construct their own Tokenomics with the desired rate.
            validator_commission_default: 0.0,
            max_supply_cap: None,
            emission: None,
            blocks_per_year: Tokenomics::default_blocks_per_year(),
        }
    }

    /// TOKENOMICS §2.4 / Q4 — when `emission: Some(LinearDecay)`,
    /// rewards must follow the linear curve, not the legacy halving
    /// path. Producer at epoch 0 gets `initial_reward`, at midpoint
    /// gets ~half, post-window gets 0.
    #[test]
    fn test_block_reward_uses_linear_decay_when_emission_set() {
        use evaporchain_types::emission::{EmissionParams, EmissionSchedule};

        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut tk = test_tokenomics();
        tk.emission = Some(
            EmissionParams::new(
                1000,
                EmissionSchedule::LinearDecay { decay_epochs: 100 },
                None,
            )
            .unwrap(),
        );
        let mut acc = RewardAccumulator::new(tk);

        // Epoch 0: full reward.
        let r0 = acc.process_block_rewards(&mut db, &addr(1), 0, 0, true, None);
        assert_eq!(r0, 1000);

        // Epoch 50 (midpoint): ~half. linear: 1000 * (100-50) / 100 = 500.
        // Use a fresh accumulator so total_minted doesn't compound.
        let mut tk2 = test_tokenomics();
        tk2.emission = Some(
            EmissionParams::new(
                1000,
                EmissionSchedule::LinearDecay { decay_epochs: 100 },
                None,
            )
            .unwrap(),
        );
        let mut acc2 = RewardAccumulator::new(tk2);
        let r50 = acc2.process_block_rewards(&mut db, &addr(1), 50, 0, true, None);
        assert_eq!(r50, 500);

        // Epoch 100 (post-window): 0.
        let mut acc3 = RewardAccumulator::new(test_tokenomics());
        acc3.tokenomics.emission = Some(
            EmissionParams::new(
                1000,
                EmissionSchedule::LinearDecay { decay_epochs: 100 },
                None,
            )
            .unwrap(),
        );
        let r100 = acc3.process_block_rewards(&mut db, &addr(1), 100, 0, true, None);
        assert_eq!(r100, 0);
    }

    /// TOKENOMICS §2.4 / Q5 — emission's max_supply cap fires
    /// independently of the legacy max_supply_cap field.
    #[test]
    fn test_block_reward_respects_emission_max_supply() {
        use evaporchain_types::emission::{EmissionParams, EmissionSchedule};

        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut tk = test_tokenomics();
        tk.emission = Some(
            EmissionParams::new(
                100,
                EmissionSchedule::Constant,
                Some(150), // cap at 150 — second block partial, third block 0
            )
            .unwrap(),
        );
        let mut acc = RewardAccumulator::new(tk);

        // Block 0: full 100.
        let r0 = acc.process_block_rewards(&mut db, &addr(1), 0, 0, true, None);
        assert_eq!(r0, 100);
        assert_eq!(acc.total_minted, 100);

        // Block 1: clipped to 50 (only 50 headroom left).
        let r1 = acc.process_block_rewards(&mut db, &addr(1), 1, 0, true, None);
        assert_eq!(r1, 50);
        assert_eq!(acc.total_minted, 150);

        // Block 2: 0 (cap reached).
        let r2 = acc.process_block_rewards(&mut db, &addr(1), 2, 0, true, None);
        assert_eq!(r2, 0);
        assert_eq!(acc.total_minted, 150);
    }

    /// Regression — emission: None must preserve the legacy
    /// reward_at_epoch_capped behavior verbatim. Critical for
    /// running cluster non-disruption when the new binary deploys.
    #[test]
    fn test_block_reward_none_emission_matches_legacy() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let tk = test_tokenomics(); // emission: None by helper default
        let mut acc = RewardAccumulator::new(tk.clone());

        // Standard legacy path: block_reward=100, half_life=1000, no cap
        // → at epoch 0 reward=100, at epoch 1000 reward=50.
        let r0 = acc.process_block_rewards(&mut db, &addr(1), 0, 0, true, None);
        assert_eq!(r0, 100);

        // Verify Tokenomics::block_reward dispatcher equals
        // reward_at_epoch_capped when emission=None.
        assert_eq!(
            tk.block_reward(0, 0),
            tk.reward_at_epoch_capped(0, 0)
        );
        assert_eq!(
            tk.block_reward(500, 0),
            tk.reward_at_epoch_capped(500, 0)
        );
        assert_eq!(
            tk.block_reward(1000, 0),
            tk.reward_at_epoch_capped(1000, 0)
        );
    }

    #[test]
    fn test_block_reward_minted_to_producer() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 0, true, None);
        assert_eq!(credit, 100);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 100);
        assert_eq!(acc.total_minted, 100);
    }

    /// Doctrine: `EulogyTrie` says "the chain's death is final." When
    /// a producer is tombstoned, `process_block_rewards` must NOT
    /// credit the dead account — the energy redirects into the
    /// supplied refresh pool, preserving §1.2 conservation.
    #[test]
    fn test_block_reward_redirects_to_refresh_pool_when_producer_dead() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());
        let mut pool = RefreshPool::new();

        // Producer is tombstoned (alive=false). Block reward should
        // redirect; producer balance must stay zero.
        let credit = acc.process_block_rewards(
            &mut db,
            &addr(1),
            0,
            0,
            /* producer_alive = */ false,
            Some(&mut pool),
        );
        assert_eq!(credit, 0, "dead producer must receive nothing");
        assert_eq!(
            db.get_account(&addr(1)).unwrap().balance,
            0,
            "tombstoned account balance must not change"
        );
        // total_minted still increments — the reward was notionally
        // minted, just routed to refresh pool. Conservation invariant
        // is the system-level total, not the producer total.
        assert_eq!(acc.total_minted, 100);
        // The pool should have accrued the 100 EVP.
        assert_eq!(
            pool.total_accrued(),
            100,
            "refresh pool must absorb the redirected block reward"
        );
    }

    /// Same doctrine, fee-distribution path. A tombstoned producer's
    /// fee share must redirect; staker share must still flow to
    /// pending_staker_rewards (it's not account-bound).
    #[test]
    fn test_fee_share_redirects_to_refresh_pool_when_producer_dead() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());
        let mut pool = RefreshPool::new();

        // 1000 fees: 500 burned, 250 to producer (redirected), 250 to stakers.
        let credit = acc.process_block_rewards(
            &mut db,
            &addr(1),
            0,
            1000,
            /* producer_alive = */ false,
            Some(&mut pool),
        );
        assert_eq!(credit, 0, "dead producer credit must be zero");
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 0);
        // block reward (100) + producer fee share (250) = 350 EVP redirected.
        assert_eq!(pool.total_accrued(), 350);
        // Staker share is independent of producer life-state.
        assert_eq!(acc.pending_staker_rewards, 250);
        assert_eq!(acc.total_burned, 500);
    }

    /// Priority bonus must follow the same doctrine: redirect when
    /// the producer is tombstoned.
    #[test]
    fn test_priority_bonus_redirects_when_producer_dead() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());
        let mut pool = RefreshPool::new();

        let bonus = acc.apply_priority_bonus(
            &mut db,
            &addr(1),
            5,
            10_000,
            1_000,
            /* producer_alive = */ false,
            Some(&mut pool),
        );
        assert_eq!(bonus, 0, "dead producer's priority bonus must be zero");
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 0);
        assert_eq!(pool.total_accrued(), 10);
        assert_eq!(acc.total_minted, 10);
    }

    #[test]
    fn test_priority_bonus_minted_to_producer() {
        // Phase-1.5 of energy-stamped MEV defense. apply_priority_bonus
        // mints `priority_sum / scale_per_unit` to the producer.
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        // priority_sum=10_000, scale=1000 → bonus=10
        let bonus = acc.apply_priority_bonus(&mut db, &addr(1), 5, 10_000, 1_000, true, None);
        assert_eq!(bonus, 10);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 10);
        assert_eq!(acc.total_minted, 10);
        // last_touched_epoch refreshed by the mint.
        assert_eq!(db.get_account(&addr(1)).unwrap().last_touched_epoch, 5);
    }

    #[test]
    fn test_priority_bonus_zero_scale_disables() {
        // scale_per_unit=0 → no-op (bonus disabled).
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        let bonus = acc.apply_priority_bonus(&mut db, &addr(1), 0, 1_000_000, 0, true, None);
        assert_eq!(bonus, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 0);
        assert_eq!(acc.total_minted, 0);
    }

    #[test]
    fn test_priority_bonus_sub_unit_yields_zero() {
        // priority_sum=100, scale=1000 → 100/1000=0 (integer div).
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        let bonus = acc.apply_priority_bonus(&mut db, &addr(1), 0, 100, 1_000, true, None);
        assert_eq!(bonus, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 0);
    }

    #[test]
    fn test_block_reward_halves() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        // At epoch 1000 (one half-life), reward should be 50
        let credit = acc.process_block_rewards(&mut db, &addr(1), 1000, 0, true, None);
        assert_eq!(credit, 50);
    }

    #[test]
    fn test_fee_distribution_to_producer_and_stakers() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        // 1000 fees: 500 burned, 250 to producer, 250 to stakers
        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 1000, true, None);
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
        acc.process_block_rewards(&mut db, &addr(1), 0, 2000, true, None);
        // 2000 * 0.5 = 1000 burned, 500 to producer, 500 to stakers
        assert_eq!(acc.pending_staker_rewards, 500);

        // Distribute to two stakers with 75/25 split
        let stakers = vec![(addr(10), 750), (addr(20), 250)];
        let distributed = acc.distribute_staker_rewards(&mut db, &stakers, 0);
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

        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 0, true, None);
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

        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 1000, true, None);
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

        let credit = acc.process_block_rewards(&mut db, &addr(1), 0, 1000, true, None);
        // Only fee share: 1000 * 0.5 = 500 burned, 250 to producer
        assert_eq!(credit, 250);
    }

    #[test]
    fn test_reward_summary() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());

        acc.process_block_rewards(&mut db, &addr(1), 0, 2000, true, None);
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
            acc.process_block_rewards(&mut db, &addr(1), epoch, 100, true, None);
        }
        // 10 blocks × 100 reward + fee shares
        assert!(acc.total_minted >= 1000);
        assert!(db.get_account(&addr(1)).unwrap().balance > 1000);
    }

    // ── TOKENOMICS §2.2 validator commission tests ─────────────────────────

    /// §2.2 happy path: 10% commission on the staker pool goes to the
    /// producer; delegators receive the remaining 90%.
    #[test]
    fn test_commission_splits_staker_pool_v2() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0); // producer
        fund(&mut db, 2, 0); // attester

        let mut tk = test_tokenomics();
        tk.validator_commission_default = 0.10; // 10% commission
        let mut acc = RewardAccumulator::new(tk);

        // 1000 fees: 500 burned, remaining 500 split:
        //   50% staker_fee_share → 250 to staker pool
        //   50% to producer directly (dist.to_producer) = 125
        //   Wait: fee_burn_rate=0.5 → 500 burned.
        //   Remaining 500: staker_fee_share=0.5 → 250 to stakers, 250 to producer.
        //   Of 250 staker pool: 10% = 25 commission to producer, 225 to pending.
        let credit = acc.process_block_rewards_v2(
            &mut db,
            &addr(1),
            0,
            1000, // total_fees
            true,
            None,
            &[addr(2)],
            // total_staked=0 → APY cap bypassed (matches the
            // test_zero_commission_v2_matches_no_commission convention
            // below). 100_000 was too small: 100_000 * 0.05 / 15_768_000
            // blocks/year ≈ 0 budget/block, capping block_reward to 1
            // and zeroing the 60% proposer share — which made the
            // expected 335 producer credit drop to 275.
            0,
        );
        // Block reward (100) with 60/40 split (attester present): 60 to producer.
        // + producer fee share: 250.
        // + commission on staker pool: 25.
        // Total producer credit = 60 + 250 + 25 = 335.
        assert_eq!(credit, 335, "producer credit with 10% commission");
        // Net staker pool: 250 - 25 = 225.
        assert_eq!(acc.pending_staker_rewards, 225, "delegators get 90% of staker pool");
        // Burned is independent of commission.
        assert_eq!(acc.total_burned, 500);
    }

    /// §2.2 zero commission: backward-compat — staker pool flows unchanged.
    #[test]
    fn test_zero_commission_v2_matches_no_commission() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);

        let mut tk = test_tokenomics();
        tk.validator_commission_default = 0.0;
        let mut acc = RewardAccumulator::new(tk);

        acc.process_block_rewards_v2(
            &mut db,
            &addr(1),
            0,
            1000,
            true,
            None,
            &[], // no attesters → 100% to proposer
            0,   // total_staked=0 → APY cap bypassed
        );
        // 1000 fees: 500 burned, 250 to producer, 250 to stakers.
        // Commission = 0 → pending staker = 250.
        assert_eq!(acc.pending_staker_rewards, 250);
        assert_eq!(acc.total_burned, 500);
    }

    /// §2.2 adversarial: commission > 100% is clamped — should never
    /// credit more than the staker pool total.
    #[test]
    fn test_commission_clamp_prevents_overflow() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);

        let mut tk = test_tokenomics();
        // Deliberately out-of-bounds (caught by genesis validation, but
        // split_staker_pool must still be safe).
        tk.validator_commission_default = 2.0; // 200% — clamped to 100%
        let (net, commission) = tk.split_staker_pool(500);
        assert_eq!(commission, 500, "clamped to 100% → full pool is commission");
        assert_eq!(net, 0, "delegators receive nothing at 100% commission");
    }

    /// §2.2 commission on dead producer: commission redirects to refresh
    /// pool exactly like the producer fee share does.
    #[test]
    fn test_commission_redirects_when_producer_dead() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 0);

        let mut tk = test_tokenomics();
        tk.validator_commission_default = 0.10;
        let mut acc = RewardAccumulator::new(tk);
        let mut pool = RefreshPool::new();

        // 1000 fees, producer dead.
        // Burned=500; dist.to_producer=250 (dead→pool); staker pool=250.
        // Commission 10% = 25 (dead→pool); net staker = 225.
        let credit = acc.process_block_rewards_v2(
            &mut db,
            &addr(1),
            0,
            1000,
            false,
            Some(&mut pool),
            &[],
            0,
        );
        assert_eq!(credit, 0, "dead producer receives nothing");
        // block_reward(100) + to_producer(250) + commission(25) = 375 redirected.
        assert_eq!(pool.total_accrued(), 375, "all producer-bound amounts redirected");
        assert_eq!(acc.pending_staker_rewards, 225, "net delegator share preserved");
    }

    // ─── T1.20 rewards.rs coverage push ──────────────────────────────

    /// distribute_staker_rewards with rounding remainder — exercises
    /// the dust-distribution loop (lines 410-422 previously uncovered).
    /// pool=100 split across 3 equal-stake stakers: each gets 33,
    /// distributed=99, remainder=1. The dust loop must hand the 1
    /// unit to the first sorted staker.
    #[test]
    fn t1_20_distribute_staker_rewards_dust_remainder() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 10, 0);
        fund(&mut db, 20, 0);
        fund(&mut db, 30, 0);
        let mut acc = RewardAccumulator::new(test_tokenomics());
        acc.pending_staker_rewards = 100;

        let stakers = vec![(addr(10), 1u64), (addr(20), 1u64), (addr(30), 1u64)];
        let distributed = acc.distribute_staker_rewards(&mut db, &stakers, 0);
        // 100 split 1:1:1 → 33 each = 99, remainder = 1, dust → 100.
        assert_eq!(distributed, 100, "dust-loop must reclaim the rounding remainder");
        // pending_staker_rewards is reset.
        assert_eq!(acc.pending_staker_rewards, 0);
        // Sum of balances equals the pool.
        let sum = db.get_account(&addr(10)).unwrap().balance
            + db.get_account(&addr(20)).unwrap().balance
            + db.get_account(&addr(30)).unwrap().balance;
        assert_eq!(sum, 100);
    }

    /// apply_priority_bonus with dead producer + refresh_pool —
    /// previously uncovered redirect path (lines 97-109).
    #[test]
    fn t1_20_apply_priority_bonus_redirects_when_producer_dead() {
        let mut db = InMemoryStateDB::new();
        let mut acc = RewardAccumulator::new(test_tokenomics());
        let mut pool = RefreshPool::new();
        let producer = addr(99);

        let credit = acc.apply_priority_bonus(
            &mut db,
            &producer,
            100,
            1_000_000,   // priority_sum
            1_000,       // scale_per_unit → bonus = 1000
            false,       // producer_alive
            Some(&mut pool),
        );
        // Dead producer: caller-visible credit = 0.
        assert_eq!(credit, 0);
        // Producer account was NOT created.
        assert!(db.get_account(&producer).is_none());
        // total_minted bumped (notional emission).
        assert_eq!(acc.total_minted, 1000);
    }

    /// apply_priority_bonus with dead producer + no pool — bonus is
    /// minted to total_minted but lands nowhere. Distinct from the
    /// pool-redirect path (line 108 → 110).
    #[test]
    fn t1_20_apply_priority_bonus_dead_no_pool_returns_zero() {
        let mut db = InMemoryStateDB::new();
        let mut acc = RewardAccumulator::new(test_tokenomics());
        let producer = addr(99);
        let credit = acc.apply_priority_bonus(
            &mut db,
            &producer,
            100,
            1_000_000,
            1_000,
            false,
            None,
        );
        assert_eq!(credit, 0);
        assert!(db.get_account(&producer).is_none());
        assert_eq!(acc.total_minted, 1000);
    }

    /// process_block_rewards with dead producer + refresh_pool —
    /// previously uncovered (lines 165-182).
    #[test]
    fn t1_20_process_block_rewards_redirects_when_producer_dead() {
        let mut db = InMemoryStateDB::new();
        let mut acc = RewardAccumulator::new(test_tokenomics());
        let mut pool = RefreshPool::new();
        let producer = addr(99);

        let credit = acc.process_block_rewards(
            &mut db,
            &producer,
            0,        // epoch
            0,        // total_fees
            false,    // producer_alive
            Some(&mut pool),
        );
        // Dead producer: caller-visible credit = 0.
        assert_eq!(credit, 0);
        // Producer account not created.
        assert!(db.get_account(&producer).is_none());
        // total_minted bumped by the block reward.
        assert_eq!(acc.total_minted, test_tokenomics().block_reward);
    }
}
