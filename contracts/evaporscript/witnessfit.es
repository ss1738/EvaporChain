// WitnessFit — Singh-Streak. The reference contract behind the
// catalogue's WITNESSFIT_STREAK (0x0001_0303), the first .es backing
// in the Consumer lane.
//
// Doctrine claim: "Wearable streaks where the streak ITSELF decays.
// Graceful fade, not cliff." Apps like Snapchat/Duolingo today treat
// the streak as an integer that resets to 0 the moment you miss a
// day — cliff failure mode. This contract makes the streak a
// first-class on-chain quantity bound to the contract's own energy:
// check in within the half-life window to grow it; miss the window
// and your CURRENT streak resets to 1 next check-in, but your
// `max_streak` (your historical peak) is preserved. The boost gate
// fires while your current streak is at least `boost_threshold_bp`
// basis points (default 5000 = 50 %) of your peak — so a graceful
// fade in the boost, not a cliff.
//
// One contract = one user (the deployer is the wearer; `caller ==
// owner` gates check_in). Multi-user wearable apps deploy one
// contract per user — the cost is small + each user's streak is
// independent of every other user's.

contract WitnessFit {
    state {
        // ── streak state ───────────────────────────────────────────
        streak_count: u64 = 0
        // The actual last check-in epoch. `has_checked_in` is the
        // sentinel that distinguishes "never checked in" from
        // "checked in at epoch 0" — using `last_checkin_epoch == 0`
        // as the sentinel collides with check-ins at epoch 0, which
        // breaks every gate on the first hour of a new chain (the
        // same lesson learned for mortal_dao.es's membership map,
        // applied here with a boolean since this contract is
        // single-user and a sentinel bool is simpler than a +1 shift
        // through arithmetic comparisons).
        last_checkin_epoch: u64 = 0
        has_checked_in: bool = false
        // Decay window in epochs (≈ days). Default 7 = a week.
        half_life: u64 = 7
        // Historical peak. Never decreases on its own; `reset_peak()`
        // is a separate op so users can intentionally start a new
        // chapter.
        max_streak: u64 = 0
        total_checkins: u64 = 0

        // Boost gate: streak must be ≥ this fraction (basis points,
        // out of 10000) of the peak to grant the boost. 5000 = 50%.
        boost_threshold_bp: u64 = 5000
    }

    // Owner-only check-in. The first check-in seeds streak = 1.
    // Subsequent check-ins inside the half-life grow the streak;
    // outside it, the streak resets to 1 (today is the new day 1).
    // Two check-ins in the same epoch are rejected.
    fn check_in() {
        require(caller == owner, "only the wearer checks in")
        if self.has_checked_in == false {
            self.streak_count = 1
            self.has_checked_in = true
        } else {
            require(epoch > self.last_checkin_epoch, "already checked in this epoch")
            if epoch <= self.last_checkin_epoch + self.half_life {
                self.streak_count += 1
            } else {
                self.streak_count = 1
            }
        }
        self.last_checkin_epoch = epoch
        self.total_checkins += 1
        if self.streak_count > self.max_streak {
            self.max_streak = self.streak_count
        }
        emit("check-in recorded")
    }

    // Voluntary peak reset — the wearer chooses to start a new
    // chapter (e.g., switching fitness goals, returning after a
    // long absence and not wanting the old peak to dominate the
    // boost gate). Doesn't touch the current streak.
    fn reset_peak() {
        require(caller == owner, "only the wearer resets peak")
        self.max_streak = self.streak_count
        emit("peak reset")
    }

    // Current streak, decay-aware: returns 0 if the half-life window
    // has elapsed without a check-in (i.e. what `check_in()` would
    // reset to 1 right now). Returns the live counter otherwise.
    fn current_streak() -> u64 {
        if self.has_checked_in == false {
            return 0
        }
        if epoch <= self.last_checkin_epoch + self.half_life {
            return self.streak_count
        }
        return 0
    }

    // Boost gate. True iff:
    //   (a) current streak > 0 (decay window not elapsed),
    //   (b) streak_count * 10000 ≥ boost_threshold_bp * max_streak.
    // Returns false before any check-ins (has_checked_in still false).
    fn has_boost() -> bool {
        if self.max_streak == 0 {
            return false
        }
        if self.has_checked_in == false {
            return false
        }
        if epoch > self.last_checkin_epoch + self.half_life {
            return false
        }
        if self.streak_count * 10000 >= self.boost_threshold_bp * self.max_streak {
            return true
        }
        return false
    }

    // ── Views ──────────────────────────────────────────────────────
    fn peak() -> u64 {
        return self.max_streak
    }

    fn checkins_total() -> u64 {
        return self.total_checkins
    }

    fn last_checkin_now() -> u64 {
        return self.last_checkin_epoch
    }

    fn half_life_now() -> u64 {
        return self.half_life
    }

    fn boost_threshold() -> u64 {
        return self.boost_threshold_bp
    }

    // Epochs of decay-window remaining before the streak would reset
    // on the next check-in. Returns 0 if the window has already
    // elapsed, or if there has never been a check-in.
    fn window_remaining() -> u64 {
        if self.has_checked_in == false {
            return 0
        }
        if epoch > self.last_checkin_epoch + self.half_life {
            return 0
        }
        return self.last_checkin_epoch + self.half_life - epoch
    }

    on_grace() {
        emit("witnessfit energy low — refresh to keep streak alive")
    }

    on_refresh() {
        emit("witnessfit refreshed")
    }

    on_evaporate() {
        emit("witnessfit evaporated — streak archived")
    }
}
