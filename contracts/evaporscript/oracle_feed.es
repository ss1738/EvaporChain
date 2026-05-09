// OracleFeed — ninth pilot. Stdlib contract #6: turns the price-feed /
// data-oracle primitive into a decay-native one.
//
// Decay-thesis hook: classic oracles (Chainlink, Pyth, UMA) publish
// data with a timestamp and let consumers decide whether it's fresh.
// This means stale data accumulates on-chain forever — Chainlink's
// price registry holds quotes that haven't been touched in years.
// OracleFeed inverts the relationship: the feed *is* a decaying
// contract. Each update refreshes its energy. Skip enough updates and
// the contract evaporates; consumers can't read stale data because
// stale data physically doesn't exist on-chain anymore. Freshness is
// enforced by chain physics, not by a check-the-timestamp convention.
//
// Lifecycle:
//
//   1. Deploy → `set_feed(name, max_age)` configures the feed. Sealed-
//      once. max_age is a soft bound the dApp uses to drive its
//      refresh cadence; the hard bound is the contract's own energy.
//   2. Operator (= deployer) calls `update(value)` on each new quote.
//      The chain runtime is expected to refresh the contract's energy
//      on each successful update (call_contract operations refresh
//      the target contract by default in the pilot grammar).
//   3. Consumers call `latest()` to get the current quote + age. If
//      the contract has evaporated, the call fails — there is no
//      stale-data path.
//
// Auth model:
//   - `set_feed`:  caller == owner (operator).
//   - `update`:    caller == owner.
//   - `latest`:    open (any address reads).
//   - `dispute`:   open — anyone can flag a value; resolution is
//                  off-chain governance.

contract OracleFeed {
    state {
        operator: address
        feed_name: string = ""
        max_age_epochs: u64 = 0
        sealed: bool = false

        // Latest quote + telemetry. value_set flips true on the first
        // update so consumers can distinguish "no data yet" from
        // "data is zero".
        value: u64 = 0
        value_set: bool = false
        updated_at_epoch: u64 = 0
        update_count: u64 = 0

        // Dispute tally. Off-chain governance resolves; the on-chain
        // counter is just a signal for consumers to pause if it spikes.
        dispute_count: u64 = 0
        last_dispute_epoch: u64 = 0
    }

    // Phase 1: configure the feed. max_age is informational — the
    // dApp uses it to schedule updates. The actual freshness ceiling
    // is the contract's energy half-life (set at deploy via the
    // standard DeployScript args).
    fn set_feed(name: string, max_age: u64) {
        require(caller == owner, "only operator can set feed")
        require(self.sealed == false, "feed already configured")
        require(max_age > 0, "max_age must be positive")
        self.operator = owner
        self.feed_name = name
        self.max_age_epochs = max_age
        self.sealed = true
        emit("feed configured")
    }

    // Operator publishes a new quote. The pilot grammar models a
    // single u64 value — dApps wanting tuples (price + volume,
    // bid + ask) deploy multiple feeds or pack into the u64.
    fn update(new_value: u64) {
        require(caller == owner, "only operator can update")
        require(self.sealed == true, "feed not configured")
        self.value = new_value
        self.value_set = true
        self.updated_at_epoch = epoch
        self.update_count += 1
        emit("feed updated")
    }

    // Reader. Reverts if no value has been published yet — consumers
    // never see a default-zero read masquerading as real data.
    fn latest() -> u64 {
        require(self.sealed == true, "feed not configured")
        require(self.value_set == true, "no value published yet")
        return self.value
    }

    // Age (in epochs) of the current quote. Useful for consumers that
    // want to apply their own freshness policy on top of the contract's
    // energy decay.
    fn age() -> u64 {
        if self.value_set == false {
            return 0
        }
        return epoch - self.updated_at_epoch
    }

    // Anyone can flag a value as suspect. The on-chain counter only
    // tallies; resolution is off-chain. Consumers can pause if
    // dispute_count > threshold.
    fn dispute() {
        require(self.sealed == true, "feed not configured")
        require(self.value_set == true, "nothing to dispute")
        self.dispute_count += 1
        self.last_dispute_epoch = epoch
        emit("feed disputed")
    }

    fn feed_label() -> string {
        return self.feed_name
    }

    fn updates_total() -> u64 {
        return self.update_count
    }

    fn disputes_total() -> u64 {
        return self.dispute_count
    }

    fn last_updated() -> u64 {
        return self.updated_at_epoch
    }

    fn is_fresh() -> bool {
        if self.value_set == false {
            return false
        }
        let a = epoch - self.updated_at_epoch
        return a <= self.max_age_epochs
    }

    on_grace() {
        emit("feed energy low — operator should publish update before evaporation")
    }

    on_refresh() {
        emit("feed refreshed")
    }

    // Doctrine moment: an evaporated feed leaves *no* readable value.
    // Consumers attempting to query a ghosted feed get an explicit
    // not-found rather than a stale stale read. Freshness is enforced
    // physically, not by convention.
    on_evaporate() {
        emit("feed evaporated — no readable value, consumers must switch source")
    }
}
