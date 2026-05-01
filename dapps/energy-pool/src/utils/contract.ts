// Single source of truth: `contracts/evaporscript/energy_pool.es`.
// This file is the inline copy the dapp ships with — keep it byte-identical
// to the .es file. The Rust integration test
// `crates/evaporchain-script/tests/energy_pool_pilot.rs` is the regression
// barrier that proves this exact source compiles + runs cleanly through
// the VM (10/10 invariant tests). Don't edit the body here without running
// that test.
//
// Lifecycle:
//   1. POST /api/tx/deploy-script  with `source_code` = ENERGY_POOL_SOURCE
//      → returns {tx_hash}; chain assigns contract_id at execution time.
//   2. POST /api/tx/call-script    with method = "set_metadata",
//      args = [name, description, strategy_u64]
//      → seals the pool's identity. Re-seal reverts.
//   3. POST /api/tx/call-script    method = "stake" / "unstake" / "record_save"
//      → mutate the per-contributor ledger.
//   4. GET /api/script/:id         → read the live state map.

export const ENERGY_POOL_SOURCE = `// EnergyPool — second pilot EvaporScript contract for the energy-pool dApp.

contract EnergyPool {
    state {
        name: string = ""
        description: string = ""
        creator: address
        strategy: u64 = 0
        sealed: bool = false

        total_staked: u64 = 0
        contributor_count: u64 = 0

        stakes: map[address -> u64]
        guardian_points: map[address -> u64]
        objects_saved: map[address -> u64]
        joined_epoch: map[address -> u64]

        last_distribution_epoch: u64 = 0
    }

    fn set_metadata(pool_name: string, pool_description: string, pool_strategy: u64) {
        require(caller == owner, "only creator can seal")
        require(self.sealed == false, "pool already sealed")
        require(pool_strategy <= 1, "unknown strategy")
        self.name = pool_name
        self.description = pool_description
        self.strategy = pool_strategy
        self.creator = owner
        self.sealed = true
        emit("pool sealed")
    }

    fn stake(amount: u64) {
        require(self.sealed == true, "pool not yet sealed")
        require(amount > 0, "stake amount must be positive")
        let prev = self.stakes[caller]
        if prev == 0 {
            self.contributor_count += 1
            self.joined_epoch[caller] = epoch
        }
        self.stakes[caller] = prev + amount
        self.total_staked += amount
        emit("stake added")
    }

    fn unstake(amount: u64) {
        require(self.sealed == true, "pool not yet sealed")
        let prev = self.stakes[caller]
        require(prev >= amount, "unstake exceeds staked balance")
        self.stakes[caller] = prev - amount
        emit("stake withdrawn")
    }

    fn record_save() {
        require(self.sealed == true, "pool not yet sealed")
        require(caller == owner, "only coordinator can record saves")
        self.guardian_points[caller] += 1
        self.objects_saved[caller] += 1
        self.last_distribution_epoch = epoch
        emit("object saved by pool")
    }

    fn balance_of(addr: address) -> u64 {
        return self.stakes[addr]
    }

    fn guardian_points_of(addr: address) -> u64 {
        return self.guardian_points[addr]
    }

    fn pool_total() -> u64 {
        return self.total_staked
    }

    fn pool_contributors() -> u64 {
        return self.contributor_count
    }

    on_grace() {
        emit("pool energy low — boost to keep distributing")
    }

    on_refresh() {
        emit("pool boosted")
    }

    on_evaporate() {
        emit("pool evaporated — protected objects no longer covered")
    }
}
`;

export const STRATEGY_EQUAL = 0;
export const STRATEGY_PRIORITY_LOW_ENERGY = 1;

export function strategyToU64(strategy: "equal" | "priority-low-energy"): number {
  return strategy === "equal" ? STRATEGY_EQUAL : STRATEGY_PRIORITY_LOW_ENERGY;
}

export function u64ToStrategy(n: number): "equal" | "priority-low-energy" {
  return n === STRATEGY_PRIORITY_LOW_ENERGY ? "priority-low-energy" : "equal";
}
