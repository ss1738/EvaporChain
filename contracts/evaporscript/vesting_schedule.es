// VestingSchedule — seventh pilot EvaporScript contract.
//
// Classic linear vest with a cliff: `cliff` epochs after `set_terms`,
// nothing has vested; between `cliff` and `duration` the vested amount
// rises linearly from 0 to `total_grant`; at and after `duration`
// the full grant is vested.
//
// Doctrine moment: vested-but-unclaimed amount forfeits at evaporation.
// The cliff + linear-vest math is conventional; what's novel is that
// the *post-vest claim window* is bounded by the contract's own
// energy. If the contract evaporates while there's vested-and-unclaimed
// amount, `on_evaporate` stamps `vested_at_evaporate` and flips
// `forfeit_signaled` so the dApp coordinator can return the unclaimed
// remainder to the grantor.
//
// EvaporScript has no contract-internal method dispatch, so the vest
// math is duplicated across `vested_now`, `claim`, `pending_amount`,
// and `on_evaporate`. Three branches each -- kept identical by hand.
//
// VEST-1 (audit 2026-05-17): all five vesting-math sites use
// division-first arithmetic to avoid u64 overflow at large grants.
// Pattern: vest_whole * elapsed + vest_rem * elapsed / duration_epochs.
// Rounding error <= 1 unit versus the naive multiply-first form.

contract VestingSchedule {
    state {
        grantor: address
        beneficiary: address
        total_grant: u64 = 0
        cliff_epochs: u64 = 0
        duration_epochs: u64 = 0
        start_epoch: u64 = 0
        sealed: bool = false

        claimed_amount: u64 = 0
        cancelled: bool = false
        cancelled_at_epoch: u64 = 0

        vested_at_evaporate: u64 = 0
        forfeit_signaled: bool = false
    }

    // Arm the vest exactly once. Only the deployer (grantor / owner)
    // can call. Validates inputs: grant + duration must be positive,
    // cliff cannot exceed duration. `start_epoch` is captured at seal.
    fn set_terms(target: address, grant: u64, cliff: u64, duration: u64) {
        require(caller == owner, "only grantor can set terms")
        require(self.sealed == false, "terms already set")
        require(grant > 0, "grant must be positive")
        require(duration > 0, "duration must be positive")
        require(cliff <= duration, "cliff cannot exceed duration")
        self.grantor = owner
        self.beneficiary = target
        self.total_grant = grant
        self.cliff_epochs = cliff
        self.duration_epochs = duration
        self.start_epoch = epoch
        self.sealed = true
        emit("vesting armed")
    }

    // Linear vest with cliff. Pre-seal: 0. Pre-cliff: 0. Mid-vest:
    // total_grant * elapsed / duration. Post-duration: total_grant.
    fn vested_now() -> u64 {
        if self.sealed == false {
            return 0
        }
        let elapsed = epoch - self.start_epoch
        if elapsed < self.cliff_epochs {
            return 0
        }
        if elapsed >= self.duration_epochs {
            return self.total_grant
        }
        let vest_whole = self.total_grant / self.duration_epochs
        let vest_rem = self.total_grant % self.duration_epochs
        return vest_whole * elapsed + vest_rem * elapsed / self.duration_epochs
    }

    // Beneficiary withdraws the delta between vested-now and
    // claimed-so-far. Returns the delta. `claimed_amount` is monotonic.
    fn claim() -> u64 {
        require(self.sealed == true, "terms not yet set")
        require(self.cancelled == false, "vest cancelled")
        require(caller == self.beneficiary, "only beneficiary can claim")
        let elapsed = epoch - self.start_epoch
        let vested = 0
        if elapsed >= self.cliff_epochs {
            if elapsed >= self.duration_epochs {
                vested = self.total_grant
            } else {
                let vest_whole = self.total_grant / self.duration_epochs
                let vest_rem = self.total_grant % self.duration_epochs
                vested = vest_whole * elapsed + vest_rem * elapsed / self.duration_epochs
            }
        }
        require(vested > self.claimed_amount, "nothing to claim")
        let delta = vested - self.claimed_amount
        self.claimed_amount = vested
        emit("vest partial claim")
        return delta
    }

    // Grantor cancels the schedule. Allowed only while nothing has
    // been claimed -- once the beneficiary has touched the grant the
    // schedule is immutable (the chain became the source of truth).
    fn cancel() {
        require(self.sealed == true, "terms not yet set")
        require(caller == owner, "only grantor can cancel")
        require(self.claimed_amount == 0, "vest immutable after first claim")
        require(self.cancelled == false, "already cancelled")
        self.cancelled = true
        self.cancelled_at_epoch = epoch
        emit("vest cancelled")
    }

    fn vested_amount() -> u64 {
        if self.sealed == false {
            return 0
        }
        let elapsed = epoch - self.start_epoch
        if elapsed < self.cliff_epochs {
            return 0
        }
        if elapsed >= self.duration_epochs {
            return self.total_grant
        }
        let vest_whole = self.total_grant / self.duration_epochs
        let vest_rem = self.total_grant % self.duration_epochs
        return vest_whole * elapsed + vest_rem * elapsed / self.duration_epochs
    }

    // Vested-but-not-yet-claimed.
    fn pending_amount() -> u64 {
        if self.sealed == false {
            return 0
        }
        let elapsed = epoch - self.start_epoch
        let vested = 0
        if elapsed >= self.cliff_epochs {
            if elapsed >= self.duration_epochs {
                vested = self.total_grant
            } else {
                let vest_whole = self.total_grant / self.duration_epochs
                let vest_rem = self.total_grant % self.duration_epochs
                vested = vest_whole * elapsed + vest_rem * elapsed / self.duration_epochs
            }
        }
        if vested <= self.claimed_amount {
            return 0
        }
        return vested - self.claimed_amount
    }

    fn beneficiary_of() -> address {
        return self.beneficiary
    }

    fn grant_total() -> u64 {
        return self.total_grant
    }

    fn cliff_at() -> u64 {
        return self.start_epoch + self.cliff_epochs
    }

    fn fully_vested_at() -> u64 {
        return self.start_epoch + self.duration_epochs
    }

    on_grace() {
        emit("vesting energy low -- claim window may close")
    }

    on_refresh() {
        emit("vesting refreshed")
    }

    // Capture vested-at-death for forensics + signal the coordinator
    // to return any unclaimed remainder to the grantor.
    on_evaporate() {
        let elapsed = epoch - self.start_epoch
        let vested = 0
        if elapsed >= self.cliff_epochs {
            if elapsed >= self.duration_epochs {
                vested = self.total_grant
            } else {
                let vest_whole = self.total_grant / self.duration_epochs
                let vest_rem = self.total_grant % self.duration_epochs
                vested = vest_whole * elapsed + vest_rem * elapsed / self.duration_epochs
            }
        }
        self.vested_at_evaporate = vested
        self.forfeit_signaled = true
        emit("vesting evaporated -- unclaimed remainder forfeits")
    }
}
