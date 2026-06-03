// Single source of truth: `contracts/evaporscript/payment_split.es`.
// Cargo pilot: `crates/evaporchain-script/tests/payment_split_pilot.rs`.

export const PAYMENT_SPLIT_SOURCE = `contract PaymentSplit {
    state {
        shares: map[address -> u64]
        claimed: map[address -> u64]
        total_bps: u64 = 0
        recipient_count: u64 = 0
        total_deposited: u64 = 0
        sealed: bool = false

        forfeit_signaled: bool = false
        unclaimed_at_evaporate: u64 = 0
    }

    fn add_recipient(target: address, bps: u64) {
        require(caller == owner, "only owner can add recipients")
        require(self.sealed == false, "payment split already sealed")
        require(bps > 0, "share must be positive")
        require(self.shares[target] == 0, "recipient already added")
        let new_total = self.total_bps + bps
        require(new_total <= 10000, "total bps would exceed 10000")
        self.shares[target] = bps
        self.total_bps = new_total
        self.recipient_count += 1
        emit("recipient added")
    }

    fn seal() {
        require(caller == owner, "only owner can seal")
        require(self.sealed == false, "payment split already sealed")
        require(self.total_bps == 10000, "total bps must equal 10000")
        self.sealed = true
        emit("payment split sealed")
    }

    fn deposit(amount: u64) {
        require(self.sealed == true, "payment split not yet sealed")
        require(amount > 0, "deposit must be positive")
        self.total_deposited += amount
        emit("deposit received")
    }

    fn claim() -> u64 {
        require(self.sealed == true, "payment split not yet sealed")
        let bps = self.shares[caller]
        require(bps > 0, "not a recipient")
        let whole = self.total_deposited / 10000
        let rem   = self.total_deposited % 10000
        let entitlement = whole * bps + rem * bps / 10000
        let already = self.claimed[caller]
        require(entitlement > already, "nothing to claim")
        let delta = entitlement - already
        self.claimed[caller] = entitlement
        emit("share claimed")
        return delta
    }

    fn entitlement_of(who: address) -> u64 {
        let bps = self.shares[who]
        let whole = self.total_deposited / 10000
        let rem   = self.total_deposited % 10000
        return whole * bps + rem * bps / 10000
    }

    fn pending_of(who: address) -> u64 {
        let bps = self.shares[who]
        let whole = self.total_deposited / 10000
        let rem   = self.total_deposited % 10000
        let entitlement = whole * bps + rem * bps / 10000
        let already = self.claimed[who]
        if entitlement <= already {
            return 0
        }
        return entitlement - already
    }

    fn share_of(who: address) -> u64 {
        return self.shares[who]
    }

    fn total_pool() -> u64 {
        return self.total_deposited
    }

    fn recipients() -> u64 {
        return self.recipient_count
    }

    on_grace() {
        emit("payment split energy low — claim window may close")
    }

    on_refresh() {
        emit("payment split refreshed")
    }

    on_evaporate() {
        self.unclaimed_at_evaporate = self.total_deposited
        self.forfeit_signaled = true
        emit("payment split evaporated — unclaimed forfeits")
    }
}`;
