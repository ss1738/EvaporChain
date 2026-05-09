// VestingSchedule — sixth pilot. Stdlib contract #3: turns the
// linear-vesting primitive into a decay-native one.
//
// Decay-thesis hook: standard vesting contracts (OpenZeppelin
// VestingWallet, Sablier) hold the entire grant indefinitely — even
// after the vesting period ends, unclaimed tokens sit in the contract
// forever. VestingSchedule gives the *post-vest claim window* a
// lifespan equal to the contract's own energy: the cliff and linear
// vest run normally, but once the contract evaporates, any vested-but-
// unclaimed amount lapses back to the deployer. Beneficiaries who
// don't claim within the contract's energy budget pay an opportunity
// cost — a strong nudge against forgotten grants.
//
// Lifecycle:
//
//   1. Deploy with cliff, duration, total grant via `set_terms`.
//      Sealed-once. Beneficiary is set in the same call.
//   2. Beneficiary calls `claim()` whenever they want to pull vested
//      tokens. Vested amount = `total * elapsed / duration` (post-
//      cliff), where elapsed = `epoch - start_epoch`, capped at total.
//   3. on_evaporate: forfeit the unclaimed remainder.
//
// Auth model:
//   - `set_terms`:  caller == owner (deployer = grantor).
//   - `claim`:      caller == self.beneficiary.
//   - `cancel`:     caller == owner (revoke before any vest claimed).
//
// Bounded state: 6 scalar fields + the beneficiary address. No maps,
// no loops. O(1) everywhere.

contract VestingSchedule {
    state {
        beneficiary: address
        grantor: address

        // Vesting parameters. start_epoch is the epoch in which
        // set_terms was called; cliff_epochs is the delay before any
        // vest accrues; duration_epochs is the linear-vest window
        // measured from start_epoch (not from cliff). After
        // start_epoch + duration_epochs the full grant is vested.
        total_grant: u64 = 0
        start_epoch: u64 = 0
        cliff_epochs: u64 = 0
        duration_epochs: u64 = 0

        // Cumulative claimed (monotonic). vested_at_evaporate captures
        // what was on the table when the contract died, for audit.
        claimed_amount: u64 = 0
        sealed: bool = false
        cancelled: bool = false
        forfeit_signaled: bool = false
        vested_at_evaporate: u64 = 0
    }

    // Phase 1: define the schedule. duration > 0 prevents division-by-
    // zero in vested_amount. cliff <= duration is enforced — a cliff
    // longer than the duration would mean nothing ever vests.
    fn set_terms(
        beneficiary_addr: address,
        grant: u64,
        cliff: u64,
        duration: u64
    ) {
        require(caller == owner, "only grantor can set terms")
        require(self.sealed == false, "terms already set")
        require(grant > 0, "grant must be positive")
        require(duration > 0, "duration must be positive")
        require(cliff <= duration, "cliff cannot exceed duration")
        self.beneficiary = beneficiary_addr
        self.grantor = owner
        self.total_grant = grant
        self.start_epoch = epoch
        self.cliff_epochs = cliff
        self.duration_epochs = duration
        self.sealed = true
        emit("vesting terms set")
    }

    // Beneficiary pulls whatever has vested but not yet been claimed.
    // Returns the delta (the amount the chain runtime / coordinator
    // should actually transfer). The vest math is monotonic — vested
    // never decreases — so claimed_amount stays consistent across
    // calls.
    fn claim() -> u64 {
        require(self.sealed == true, "terms not set")
        require(self.cancelled == false, "vesting cancelled")
        require(caller == self.beneficiary, "only beneficiary can claim")
        let v = self.vested_now()
        require(v > self.claimed_amount, "nothing to claim")
        let delta = v - self.claimed_amount
        self.claimed_amount = v
        emit("vest claim recorded")
        return delta
    }

    // Internal: how much is vested as of this epoch. Pre-cliff = 0.
    // Post-duration = total_grant. In between = linear interpolation.
    fn vested_now() -> u64 {
        if self.sealed == false {
            return 0
        }
        if self.cancelled == true {
            return self.claimed_amount
        }
        let elapsed = epoch - self.start_epoch
        if elapsed < self.cliff_epochs {
            return 0
        }
        if elapsed >= self.duration_epochs {
            return self.total_grant
        }
        return (self.total_grant * elapsed) / self.duration_epochs
    }

    // Grantor can cancel iff nothing has been claimed yet. After any
    // claim, the schedule becomes irrevocable — beneficiary's earned
    // amount is protected.
    fn cancel() {
        require(caller == owner, "only grantor can cancel")
        require(self.sealed == true, "terms not set")
        require(self.claimed_amount == 0, "schedule has vested claims; immutable")
        require(self.cancelled == false, "already cancelled")
        self.cancelled = true
        emit("vesting cancelled")
    }

    fn vested_amount() -> u64 {
        return self.vested_now()
    }

    fn pending_amount() -> u64 {
        let v = self.vested_now()
        if v <= self.claimed_amount {
            return 0
        }
        return v - self.claimed_amount
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
        emit("vesting energy low — beneficiary should claim before evaporation")
    }

    on_refresh() {
        emit("vesting refreshed")
    }

    // Doctrine moment: any vested-but-unclaimed amount is forfeit when
    // the contract evaporates. This forces grantees to actively claim
    // — a forgotten grant doesn't sit on-chain forever.
    on_evaporate() {
        let v = self.vested_now()
        self.vested_at_evaporate = v
        self.forfeit_signaled = true
        emit("vesting evaporated — unclaimed vest forfeit")
    }
}
