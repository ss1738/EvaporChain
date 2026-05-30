// Single source of truth: `contracts/evaporscript/bell_oracle.es`.
// This is the inline copy the dApp ships with — keep it byte-identical
// to the .es file. The Rust pilot test
// `evaporchain-script` → `mod bell_oracle_pilot` (in lib.rs) is the
// regression barrier proving this exact source compiles + runs through
// the VM and is totality-clean. Don't edit the body here without
// running it.
//
// Lifecycle:
//   1. Deploy via /api/tx/deploy-script (caller becomes owner/operator).
//   2. arm(max_age) — set the freshness-window policy. One-shot.
//   3. submit_reading(s_milli, height) — operator posts a per-block
//      CHSH S reading from the chain's /api/bell/latest endpoint.
//      Sub-threshold (≤ 2000 milli) readings are STRUCTURALLY REJECTED
//      on-chain; only certifiably-quantum readings are stored.
//   4. is_certified_now() — downstream contracts gate on this.

export const BELL_ORACLE_SOURCE = `contract BellOracle {
    state {
        latest_s_milli: u64 = 0
        latest_height: u64 = 0
        latest_recorded_epoch: u64 = 0

        threshold_milli: u64 = 2000
        max_age_epochs: u64 = 10
        sealed: bool = false

        readings_accepted: u64 = 0
        readings_rejected_below_floor: u64 = 0
        readings_rejected_stale_height: u64 = 0
    }

    fn arm(max_age: u64) {
        require(caller == owner, "only operator arms")
        require(self.sealed == false, "already armed")
        require(max_age > 0, "max_age must be positive")
        self.max_age_epochs = max_age
        self.sealed = true
        emit("Bell oracle armed")
    }

    fn submit_reading(s_milli: u64, height: u64) {
        require(self.sealed == true, "oracle not armed")
        require(caller == owner, "only operator submits")
        if height <= self.latest_height {
            self.readings_rejected_stale_height += 1
            emit("reading rejected — height not strictly increasing")
            return
        }
        if s_milli <= self.threshold_milli {
            self.readings_rejected_below_floor += 1
            emit("reading rejected — at or below local-realism floor")
            return
        }
        self.latest_s_milli = s_milli
        self.latest_height = height
        self.latest_recorded_epoch = epoch
        self.readings_accepted += 1
        emit("Bell reading accepted")
    }

    fn is_certified_now() -> bool {
        if self.readings_accepted == 0 {
            return false
        }
        if epoch > self.latest_recorded_epoch + self.max_age_epochs {
            return false
        }
        if self.latest_s_milli <= self.threshold_milli {
            return false
        }
        return true
    }

    fn latest_s_milli_view() -> u64 {
        require(self.readings_accepted > 0, "no reading recorded yet")
        return self.latest_s_milli
    }

    fn last_height() -> u64 {
        return self.latest_height
    }

    fn last_epoch_recorded() -> u64 {
        return self.latest_recorded_epoch
    }

    fn is_fresh() -> bool {
        if self.readings_accepted == 0 {
            return false
        }
        return (epoch - self.latest_recorded_epoch) <= self.max_age_epochs
    }

    fn floor() -> u64 {
        return self.threshold_milli
    }

    fn max_age() -> u64 {
        return self.max_age_epochs
    }

    fn accepted_total() -> u64 {
        return self.readings_accepted
    }

    fn rejected_below_floor() -> u64 {
        return self.readings_rejected_below_floor
    }

    fn rejected_stale_height() -> u64 {
        return self.readings_rejected_stale_height
    }

    fn is_armed() -> bool {
        return self.sealed
    }

    on_grace() {
        emit("Bell oracle energy low — refresh to keep accepting readings")
    }

    on_refresh() {
        emit("Bell oracle refreshed")
    }

    on_evaporate() {
        emit("Bell oracle evaporated — downstream gates fail closed")
    }
}
`;
