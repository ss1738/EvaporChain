// PaymentSplit — second pilot EvaporScript contract.
//
// Pull-payment revenue splitter with basis-point shares. The deployer
// configures N recipients with bps shares that must sum to exactly
// 10_000 (100.00%), then seals. Any address can deposit; each recipient
// pulls their cumulative share on demand. Per-recipient `claimed[]`
// tracking makes the math idempotent: `owed = total_deposited * bps /
// 10_000 - claimed[recipient]`. Repeated claims with no new deposits
// revert (no rounding refunds).
//
// Doctrine moment: at evaporation, unclaimed amounts forfeit (the
// coordinator can return the residue to the deployer). No off-chain
// recovery sweep — the runtime is the closer.

contract PaymentSplit {
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

    // Add a recipient with a basis-point share. Owner-only, pre-seal.
    // Duplicates are rejected; total share is capped at 10_000.
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

    // Seal the recipient set. Requires total_bps == 10_000 exactly —
    // no under-allocation, no over-allocation, no dust.
    fn seal() {
        require(caller == owner, "only owner can seal")
        require(self.sealed == false, "payment split already sealed")
        require(self.total_bps == 10000, "total bps must equal 10000")
        self.sealed = true
        emit("payment split sealed")
    }

    // Deposit is open — any address may add to the pool. Post-seal only.
    fn deposit(amount: u64) {
        require(self.sealed == true, "payment split not yet sealed")
        require(amount > 0, "deposit must be positive")
        self.total_deposited += amount
        emit("deposit received")
    }

    // Pull-payment: recipient claims their cumulative share minus what
    // they've already pulled. Re-claim with no new deposit reverts.
    fn claim() -> u64 {
        require(self.sealed == true, "payment split not yet sealed")
        let bps = self.shares[caller]
        require(bps > 0, "not a recipient")
        // SPLIT-1 (audit 2026-05-17): division-first to avoid u64 overflow at
        // total_deposited > u64::MAX/bps (~1.8e15 for bps=10000). Pre-fix
        // `total * bps / 10000` overflowed u64 silently, bricking all claims.
        // Rounding error: at most 1 unit per recipient (lost to integer division).
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

    // Gross cumulative entitlement for `who`. Non-recipients yield 0.
    fn entitlement_of(who: address) -> u64 {
        let bps = self.shares[who]
        let whole = self.total_deposited / 10000
        let rem   = self.total_deposited % 10000
        return whole * bps + rem * bps / 10000
    }

    // Vested-but-not-yet-claimed for `who`. Non-recipients yield 0.
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

    // Stamp the unclaimed pool size + signal forfeit so the
    // coordinator can return any residue to the deployer.
    on_evaporate() {
        self.unclaimed_at_evaporate = self.total_deposited
        self.forfeit_signaled = true
        emit("payment split evaporated — unclaimed forfeits")
    }
}
