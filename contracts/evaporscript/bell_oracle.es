// BellOracle — on-chain consumer of the per-block CHSH S-value.
//
// EvaporChain uniquely measures a Bell-CHSH S-value per block from
// VRF outputs (see /api/bell/latest, threshold_milli = 2000 corresponds
// to S = 2.0, the local-realism floor). A reading strictly above 2000
// milli-units means the validator-set physics is violating Bell
// inequality — i.e., quantum-grade entropy. This is the only chain
// whose state has this property natively.
//
// BellOracle is the on-chain consumer pattern. An operator (off-chain
// relayer with read access to /api/bell/latest) submits each new
// measurement via `submit_reading(s_milli, height)`. The contract
// STRUCTURALLY REJECTS any reading below the local-realism floor —
// classical readings are not just labelled, they cannot be stored.
//
// Downstream callers gate their actions on `is_certified_now()`,
// which is true iff:
//   (a) at least one reading has been accepted
//   (b) the last accepted reading is younger than `max_age_epochs`
//   (c) that reading was strictly above the local-realism floor
//       (always true because submit_reading rejects otherwise)
//
// The contract's own energy is its lifespan — once it evaporates,
// no more readings are accepted; downstream gates fail closed.

contract BellOracle {
    state {
        // ── latest accepted reading ────────────────────────────────
        latest_s_milli: u64 = 0
        latest_height: u64 = 0
        latest_recorded_epoch: u64 = 0

        // ── policy ─────────────────────────────────────────────────
        // 2000 milli = S = 2.0, the Bell-CHSH local-realism floor.
        // Readings strictly above this clear the gate; equality is
        // not enough (classical maxima saturate exactly at 2).
        threshold_milli: u64 = 2000
        // freshness window in EPOCHS (≈ blocks); 10 ≈ 20s at 2s blocks.
        max_age_epochs: u64 = 10
        sealed: bool = false

        // ── counters ───────────────────────────────────────────────
        readings_accepted: u64 = 0
        readings_rejected_below_floor: u64 = 0
        readings_rejected_stale_height: u64 = 0
    }

    // Owner-only: arm the oracle with the freshness-window policy.
    // After arming, max_age + threshold are immutable. (V1: threshold
    // is hardcoded at 2000 milli — the Bell floor is a physical
    // constant, not a policy knob.)
    fn arm(max_age: u64) {
        require(caller == owner, "only operator arms")
        require(self.sealed == false, "already armed")
        require(max_age > 0, "max_age must be positive")
        self.max_age_epochs = max_age
        self.sealed = true
        emit("Bell oracle armed")
    }

    // Owner-only: submit a per-block Bell-CHSH reading. The contract
    // STRUCTURALLY rejects below-floor readings (counter bumped; no
    // emit beyond the warning). Heights must monotone-strictly increase.
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

    // True iff we hold a fresh, Bell-certified reading right now.
    // Downstream contracts gate quantum-grade-randomness-requiring
    // operations on this.
    fn is_certified_now() -> bool {
        if self.readings_accepted == 0 {
            return false
        }
        if (epoch - self.latest_recorded_epoch) > self.max_age_epochs {
            return false
        }
        // submit_reading ensures stored values strictly exceed the
        // floor, but check defensively in case the threshold was
        // bumped (V2 doctrine work).
        if self.latest_s_milli <= self.threshold_milli {
            return false
        }
        return true
    }

    // View: latest accepted Bell S in milli-units. Reverts if no
    // reading has been recorded — "no data" is structurally distinct
    // from "zero."
    fn latest_s_milli_view() -> u64 {
        require(self.readings_accepted > 0, "no reading recorded yet")
        return self.latest_s_milli
    }

    // View: epoch height of the latest accepted measurement (the
    // chain block height that produced the S-value, NOT this epoch).
    fn last_height() -> u64 {
        return self.latest_height
    }

    fn last_epoch_recorded() -> u64 {
        return self.latest_recorded_epoch
    }

    // View: is the latest reading younger than max_age? Returns false
    // before the first accepted reading.
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
