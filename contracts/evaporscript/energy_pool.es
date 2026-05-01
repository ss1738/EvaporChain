// EnergyPool — second pilot EvaporScript contract for the energy-pool dApp.
//
// Each pool is its own contract instance. The contract aggregates energy
// stakes from multiple contributors and (off-chain) coordinates
// distribution to a configured set of protected objects. Guardian points
// reward contributors when distributions succeed; the contract's own
// energy is the pool's lifespan — when it evaporates, the pool stops
// distributing.
//
// Scoping decisions for this pilot (deliberate cuts to fit EvaporScript's
// structural-totality contract):
//
//   - `protected_objects` is held off-chain. The contract's job is the
//     stake ledger and the guardian-points tally; the actual distribution
//     loop runs in a coordinator that calls `record_save` when an object
//     is rescued. Putting an unbounded list of protected objects on-chain
//     would mean iteration with no static bound — outside EvaporScript's
//     totality contract.
//   - `strategy` is encoded as a u64 enum (0 = equal, 1 = priority-low-
//     energy). Strings would burn gas for no payoff; the dApp does the
//     mapping at the boundary.
//   - Distribution math: when a save happens, the coordinator passes
//     credit_to in caller. We award one guardian point per save to the
//     designated address. A pro-rata distribution across all current
//     stakers requires walking `stakes[]` which EvaporScript allows via
//     map iteration but at scale would cost real gas; deferred until the
//     pool sizes warrant it.

contract EnergyPool {
    state {
        // Pool identity and configuration. `sealed` flips true after
        // `set_metadata` runs once. Subsequent attempts revert.
        name: string = ""
        description: string = ""
        creator: address
        // 0 = equal, 1 = priority-low-energy. Other values reserved.
        strategy: u64 = 0
        sealed: bool = false

        // Cumulative stake-history accounting. `total_staked` is monotonic
        // (never decremented on unstake) so it doubles as a lifetime-stake
        // counter for stat queries; live aggregate is sum over `stakes[]`.
        total_staked: u64 = 0
        contributor_count: u64 = 0

        // Per-contributor ledger. stakes[addr] is the live staked balance;
        // guardian_points[addr] is the earned-rewards counter; joined_epoch
        // records when the address first staked.
        stakes: map[address -> u64]
        guardian_points: map[address -> u64]
        objects_saved: map[address -> u64]
        joined_epoch: map[address -> u64]

        last_distribution_epoch: u64 = 0
    }

    // Seal pool metadata exactly once. Only the creator (deployer / owner)
    // can call this. Subsequent attempts revert. Stake / unstake / record
    // operations all require `sealed == true`.
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

    // Add `amount` to the caller's staked balance. First-time stakers
    // bump `contributor_count` and record `joined_epoch`.
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

    // Withdraw `amount` from the caller's staked balance. `total_staked`
    // and `contributor_count` are NOT decremented — they're cumulative
    // metrics; live aggregate reads from `stakes[]`. A user who unstakes
    // to zero stays counted as a historical contributor.
    fn unstake(amount: u64) {
        require(self.sealed == true, "pool not yet sealed")
        let prev = self.stakes[caller]
        require(prev >= amount, "unstake exceeds staked balance")
        self.stakes[caller] = prev - amount
        emit("stake withdrawn")
    }

    // Coordinator hook: called when a pool distribution successfully
    // refreshes a protected object. Awards one guardian point + increments
    // objects_saved for the caller (the coordinator / relayer). The
    // creator authorisation gate ensures arbitrary callers can't mint
    // guardian points; in production a multisig or governance address
    // would replace `caller == owner`.
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
