// Single source of truth: `contracts/evaporscript/time_lock.es`.
// Cargo pilot: `crates/evaporchain-script/tests/time_lock_pilot.rs`.

export const TIME_LOCK_SOURCE = `contract TimeLock {
    state {
        grantor: address
        beneficiary: address
        amount: u64 = 0
        unlock_epoch: u64 = 0
        sealed: bool = false
        claimed: bool = false
        revoked: bool = false
        forfeit_signaled: bool = false
        unclaimed_at_evaporate: u64 = 0
    }

    fn set_terms(target: address, lock_amount: u64, unlock: u64) {
        require(caller == owner, "only grantor can set terms")
        require(self.sealed == false, "terms already set")
        require(lock_amount > 0, "amount must be positive")
        require(unlock > epoch, "unlock must be in the future")
        self.grantor = owner
        self.beneficiary = target
        self.amount = lock_amount
        self.unlock_epoch = unlock
        self.sealed = true
        emit("time lock armed")
    }

    fn claim() -> u64 {
        require(self.sealed == true, "terms not yet set")
        require(self.revoked == false, "lock revoked")
        require(self.claimed == false, "already claimed")
        require(caller == self.beneficiary, "only beneficiary can claim")
        require(epoch >= self.unlock_epoch, "still locked")
        self.claimed = true
        emit("time lock claimed")
        return self.amount
    }

    fn revoke() {
        require(self.sealed == true, "terms not yet set")
        require(caller == owner, "only grantor can revoke")
        require(self.claimed == false, "cannot revoke after claim")
        require(epoch < self.unlock_epoch, "cannot revoke after unlock")
        self.revoked = true
        emit("time lock revoked")
    }

    fn beneficiary_of() -> address {
        return self.beneficiary
    }

    fn locked() -> u64 {
        if self.claimed == true {
            return 0
        }
        return self.amount
    }

    fn unlock_at() -> u64 {
        return self.unlock_epoch
    }

    fn is_unlocked() -> bool {
        return epoch >= self.unlock_epoch
    }

    fn is_claimed() -> bool {
        return self.claimed
    }

    on_grace() {
        emit("time lock energy low — claim window may close")
    }

    on_refresh() {
        emit("time lock refreshed")
    }

    on_evaporate() {
        if self.claimed == false {
            if self.revoked == false {
                self.forfeit_signaled = true
                self.unclaimed_at_evaporate = self.amount
                emit("time lock forfeited — return to grantor")
            }
        }
    }
}`;
