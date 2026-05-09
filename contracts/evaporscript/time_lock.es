// TimeLock — seventh pilot. Stdlib contract #4: turns the time-lock
// primitive into a decay-native one.
//
// Decay-thesis hook: standard time-locks (Compound Timelock,
// OpenZeppelin TokenTimelock) define an unlock_epoch and then sit
// passively forever until someone claims. This means a forgotten time-
// lock occupies on-chain state in perpetuity. TimeLock keeps the
// unlock_epoch (some action *requires* a calendar deadline) but bounds
// the *claim window* by the contract's own energy lifetime: claim
// between unlock_epoch and contract evaporation, or your locked tokens
// revert to the grantor. The chain's natural decay enforces the claim
// deadline — no second timer, no off-chain reaper.
//
// Lifecycle:
//
//   1. Deploy → `set_terms(beneficiary, amount, unlock)` sets the
//      schedule. Sealed-once.
//   2. After `unlock_epoch`, beneficiary calls `claim()` to record the
//      pull. Coordinator handles actual transfer.
//   3. on_evaporate: if not yet claimed, the unclaimed amount is
//      forfeit (revert-to-grantor signal). The grantor reclaims via
//      a separate flow if the contract is resurrected; if not, the
//      ghost record preserves the audit trail.
//
// Auth model:
//   - `set_terms`:  caller == owner (grantor).
//   - `claim`:      caller == self.beneficiary, epoch >= unlock_epoch.
//   - `revoke`:     caller == owner, epoch < unlock_epoch (early
//                   cancellation only — once unlocked, the
//                   beneficiary's right is irrevocable).

contract TimeLock {
    state {
        beneficiary: address
        grantor: address

        locked_amount: u64 = 0
        unlock_epoch: u64 = 0

        claimed: bool = false
        revoked: bool = false
        sealed: bool = false

        // Forfeit signal — set if the contract evaporates with the
        // lock still pending. unclaimed_at_evaporate captures the
        // dollar-amount on the table at death.
        forfeit_signaled: bool = false
        unclaimed_at_evaporate: u64 = 0
    }

    // Phase 1: define the lock. unlock_epoch must be in the future at
    // setup (no instant-unlocks — those should use a direct transfer).
    fn set_terms(
        beneficiary_addr: address,
        amount: u64,
        unlock: u64
    ) {
        require(caller == owner, "only grantor can set terms")
        require(self.sealed == false, "terms already set")
        require(amount > 0, "amount must be positive")
        require(unlock > epoch, "unlock must be in the future")
        self.beneficiary = beneficiary_addr
        self.grantor = owner
        self.locked_amount = amount
        self.unlock_epoch = unlock
        self.sealed = true
        emit("time-lock terms set")
    }

    // Beneficiary claim. Gated on (a) unlock_epoch reached, (b) not
    // yet claimed, (c) not revoked. The decay window is enforced by
    // the chain runtime: if the contract has evaporated, the call
    // never lands.
    fn claim() -> u64 {
        require(self.sealed == true, "terms not set")
        require(self.revoked == false, "lock was revoked pre-unlock")
        require(self.claimed == false, "already claimed")
        require(caller == self.beneficiary, "only beneficiary can claim")
        require(epoch >= self.unlock_epoch, "still locked")
        self.claimed = true
        emit("time-lock claim recorded")
        return self.locked_amount
    }

    // Grantor can revoke pre-unlock — the lock has not yet matured,
    // so the beneficiary has no vested right. Post-unlock the lock
    // is immutable from the grantor's side.
    fn revoke() {
        require(caller == owner, "only grantor can revoke")
        require(self.sealed == true, "terms not set")
        require(self.claimed == false, "already claimed")
        require(self.revoked == false, "already revoked")
        require(epoch < self.unlock_epoch, "cannot revoke after unlock")
        self.revoked = true
        emit("time-lock revoked")
    }

    fn beneficiary_of() -> address {
        return self.beneficiary
    }

    fn locked() -> u64 {
        if self.claimed == true {
            return 0
        }
        if self.revoked == true {
            return 0
        }
        return self.locked_amount
    }

    fn unlock_at() -> u64 {
        return self.unlock_epoch
    }

    fn is_unlocked() -> bool {
        if self.sealed == false {
            return false
        }
        if self.revoked == true {
            return false
        }
        return epoch >= self.unlock_epoch
    }

    fn is_claimed() -> bool {
        return self.claimed
    }

    on_grace() {
        if self.claimed == false {
            if self.revoked == false {
                emit("time-lock energy low — beneficiary should claim before evaporation")
            }
        }
    }

    on_refresh() {
        emit("time-lock refreshed")
    }

    // Doctrine moment: an unclaimed lock at evaporation time is
    // forfeit — the chain doesn't carry orphaned lock state forever.
    on_evaporate() {
        if self.claimed == false {
            if self.revoked == false {
                self.forfeit_signaled = true
                self.unclaimed_at_evaporate = self.locked_amount
                emit("time-lock evaporated — unclaimed amount forfeit")
            }
        }
    }
}
