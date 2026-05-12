// TimeLock — eighth pilot EvaporScript contract.
//
// Locks an `amount` of value (off-chain bookkeeping; on-chain we record
// only the promise) for `beneficiary` until `unlock_epoch`. Before the
// unlock the grantor may `revoke` to cancel. At or after unlock the
// beneficiary may `claim` exactly once.
//
// The contract's own energy doubles as the *claim window*: if the
// beneficiary never claims and the contract evaporates, `on_evaporate`
// flips `forfeit_signaled` so the dApp coordinator can return the
// locked amount to the grantor. No off-chain reaper is needed — the
// runtime is the deadline enforcer.
//
// Deploy via DeployScript with energy sized for the full window; the
// grantor then calls `set_terms` exactly once to arm.

contract TimeLock {
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

    // Arm the lock exactly once. Only the deployer (grantor / owner)
    // can call. `unlock` must be strictly in the future; `amount` must
    // be positive. After this call `sealed` flips true and the terms
    // are immutable for the contract's lifetime.
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

    // Beneficiary withdraws once the unlock epoch is reached. Returns
    // the locked amount so the coordinator knows how much to release.
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

    // Grantor cancels the lock before the unlock epoch. Post-unlock the
    // promise is irrevocable: the beneficiary's claim window is now
    // open and shortening it unilaterally would be unsafe.
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

    // Pre-claim returns the full locked amount; post-claim returns 0.
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

    // If the contract evaporates with the lock still active (never
    // claimed, never revoked) signal forfeit so the coordinator can
    // return the locked amount to the grantor.
    on_evaporate() {
        if self.claimed == false {
            if self.revoked == false {
                self.forfeit_signaled = true
                self.unclaimed_at_evaporate = self.amount
                emit("time lock forfeited — return to grantor")
            }
        }
    }
}
