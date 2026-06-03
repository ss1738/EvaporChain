// Single source of truth: `contracts/evaporscript/vesting_schedule.es`.
// Cargo pilot: `crates/evaporchain-script/tests/vesting_schedule_pilot.rs`.

export const VESTING_SCHEDULE_SOURCE = `contract VestingSchedule {
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
}`;
