// Single source of truth: `contracts/evaporscript/oracle_feed.es`.
// Cargo pilot: `crates/evaporchain-script/tests/oracle_feed_pilot.rs`.

export const ORACLE_FEED_SOURCE = `contract OracleFeed {
    state {
        label: string = ""
        max_age: u64 = 0
        sealed: bool = false

        value: u64 = 0
        value_set: bool = false
        updated_at_epoch: u64 = 0

        update_count: u64 = 0
        dispute_count: u64 = 0
    }

    // Configure the feed exactly once. Only the operator (deployer /
    // owner) can call. After sealing, label + max_age are immutable.
    fn set_feed(feed_label: string, freshness_window: u64) {
        require(caller == owner, "only operator can configure")
        require(self.sealed == false, "feed already configured")
        self.label = feed_label
        self.max_age = freshness_window
        self.sealed = true
        emit("feed configured")
    }

    // Publish a new value. Only the operator can update; updates stamp
    // \`updated_at_epoch\` and bump \`update_count\`. Disputes are unaffected.
    fn update(new_value: u64) {
        require(self.sealed == true, "feed not configured")
        require(caller == owner, "only operator can update")
        self.value = new_value
        self.value_set = true
        self.updated_at_epoch = epoch
        self.update_count += 1
        emit("feed updated")
    }

    // Latest value. Reverts if no value has ever been published — the
    // structural alternative to returning a sentinel that consumers
    // must remember to check.
    fn latest() -> u64 {
        require(self.sealed == true, "feed not configured")
        require(self.value_set == true, "no value published yet")
        return self.value
    }

    // Epochs since last update; 0 if no value has been set yet.
    fn age() -> u64 {
        if self.value_set == false {
            return 0
        }
        return epoch - self.updated_at_epoch
    }

    // Anyone can dispute — disputes are open by design. The counter is
    // a public signal; arbitration happens off-chain or in a paired
    // governance contract.
    fn dispute() {
        require(self.sealed == true, "feed not configured")
        self.dispute_count += 1
        emit("feed disputed")
    }

    fn feed_label() -> string {
        return self.label
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

    // Fresh iff value_set AND age <= max_age. Pre-update returns false
    // even when max_age is huge — "no value" is structurally !fresh.
    fn is_fresh() -> bool {
        if self.value_set == false {
            return false
        }
        return (epoch - self.updated_at_epoch) <= self.max_age
    }

    on_grace() {
        emit("oracle feed energy low — refresh to keep publishing")
    }

    on_refresh() {
        emit("oracle feed refreshed")
    }

    on_evaporate() {
        emit("oracle feed evaporated — stale data no longer on-chain")
    }
}`;
