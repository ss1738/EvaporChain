// DeadManSwitch — fourteenth pilot. Stdlib contract #11: the canonical
// decay-native dApp. This is the contract EvaporChain was made for.
//
// Decay-thesis hook: every other chain implements dead-man's switches
// with off-chain reapers — a script that polls "has the principal
// checked in this week?" and triggers the release. The reaper is the
// single point of failure: who runs it, who pays its gas, what if it
// stops? On EvaporChain, the contract's *own energy* is the check-in
// window. The principal refreshes the contract (= "I'm still here").
// If they fail to refresh, the contract evaporates and on_evaporate
// fires the release. No reaper, no polling, no bystander gas. The
// chain's natural decay is the trigger.
//
// Use cases (real, not theoretical):
//   - Inheritance: assets release to next-of-kin if owner stops
//     checking in.
//   - Whistleblower escrow: a secret is released if the principal
//     doesn't check in (a journalist, an activist, a witness).
//   - Will execution: estate handler triggered by absence.
//   - Honeypot canary: trip-wire fires if a service operator goes
//     silent.
//
// Lifecycle:
//
//   1. Deploy → `set_switch(beneficiary, payload)` configures.
//      Sealed-once. Payload is opaque (encrypted blob, IPFS CID,
//      etc.); only the beneficiary's flow knows what to do with it.
//   2. Principal calls `check_in()` periodically. Each call refreshes
//      the contract's energy via the chain runtime — extending the
//      dead-man clock by however much the call boost gives.
//   3. If principal stops checking in, the contract eventually
//      evaporates. on_evaporate fires release.
//   4. Beneficiary calls `claim()` after release to confirm pickup.
//
// Auth model:
//   - `set_switch`:  caller == owner (principal).
//   - `check_in`:    caller == owner.
//   - `claim`:       caller == self.beneficiary, after release.
//   - `disarm`:      caller == owner — explicit cancel before death.

contract DeadManSwitch {
    state {
        principal: address
        beneficiary: address

        // Payload is the data the beneficiary receives at release.
        // Encrypted at the application layer if it's sensitive.
        payload: string = ""
        sealed: bool = false

        // Activity tracking. last_checkin_epoch is the principal's
        // last sign of life. checkin_count is the headline tally.
        last_checkin_epoch: u64 = 0
        checkin_count: u64 = 0

        // Release state. Set by on_evaporate when the principal stops
        // checking in. Once released, beneficiary can claim.
        released: bool = false
        released_at_epoch: u64 = 0
        claimed: bool = false
        claimed_at_epoch: u64 = 0
        disarmed: bool = false
    }

    // Phase 1: configure the switch. Beneficiary + payload are fixed
    // at this point. The principal's first check-in is implicit in
    // this call (it's their first sign of life on the contract).
    fn set_switch(beneficiary_addr: address, release_payload: string) {
        require(caller == owner, "only principal can set switch")
        require(self.sealed == false, "switch already armed")
        self.principal = owner
        self.beneficiary = beneficiary_addr
        self.payload = release_payload
        self.sealed = true
        self.last_checkin_epoch = epoch
        self.checkin_count = 1
        emit("switch armed")
    }

    // Phase 2: principal checks in. Refreshes the contract's energy
    // via the chain runtime (call_contract refreshes the target). The
    // contract's energy IS the dead-man clock.
    fn check_in() {
        require(self.sealed == true, "switch not armed")
        require(self.disarmed == false, "switch disarmed")
        require(self.released == false, "switch already released")
        require(caller == self.principal, "only principal can check in")
        self.last_checkin_epoch = epoch
        self.checkin_count += 1
        emit("check-in recorded")
    }

    // Principal can disarm explicitly while still alive — useful for
    // estate restructuring or simply changing the beneficiary (deploy
    // a new switch, disarm the old one). Only valid pre-release.
    fn disarm() {
        require(caller == owner, "only principal can disarm")
        require(self.sealed == true, "switch not armed")
        require(self.disarmed == false, "already disarmed")
        require(self.released == false, "already released")
        self.disarmed = true
        emit("switch disarmed")
    }

    // Beneficiary claims the released payload. The on_evaporate hook
    // sets released = true; until then this method reverts. Claim is
    // recorded so the coordinator (handling actual asset transfer or
    // payload delivery) can prove the beneficiary picked up.
    fn claim() -> string {
        require(self.released == true, "switch not yet released")
        require(self.claimed == false, "already claimed")
        require(caller == self.beneficiary, "only beneficiary can claim")
        self.claimed = true
        self.claimed_at_epoch = epoch
        emit("payload claimed")
        return self.payload
    }

    fn principal_of() -> address {
        return self.principal
    }

    fn beneficiary_of() -> address {
        return self.beneficiary
    }

    fn last_checkin() -> u64 {
        return self.last_checkin_epoch
    }

    fn checkins_total() -> u64 {
        return self.checkin_count
    }

    fn is_released() -> bool {
        return self.released
    }

    fn is_claimed() -> bool {
        return self.claimed
    }

    fn is_disarmed() -> bool {
        return self.disarmed
    }

    // Days since last check-in. Beneficiary watches this to know when
    // the contract is approaching its grace window. The chain emits
    // an on_grace event when energy crosses the grace threshold.
    fn silence_age() -> u64 {
        if self.sealed == false {
            return 0
        }
        return epoch - self.last_checkin_epoch
    }

    on_grace() {
        if self.disarmed == false {
            if self.released == false {
                emit("dead-man-switch energy low — principal must check in or switch fires")
            }
        }
    }

    on_refresh() {
        emit("dead-man-switch refreshed")
    }

    // The doctrine moment EvaporChain was built for. When the contract
    // evaporates, the principal has been silent past the dead-man
    // window. The switch fires: released = true, beneficiary can
    // claim. No off-chain reaper, no third-party gas, no
    // who-watches-the-watcher problem. Decay IS the trigger.
    on_evaporate() {
        if self.disarmed == false {
            self.released = true
            self.released_at_epoch = epoch
            emit("dead-man-switch fired — payload released to beneficiary")
        }
    }
}
