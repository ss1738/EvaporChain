// Single source of truth: `contracts/evaporscript/witnessfit.es`.
// Byte-stable inline copy. Pilot at `mod witnessfit_pilot` is the
// regression barrier proving this exact source parses, compiles, and
// passes the V1 totality gate.

export const WITNESSFIT_SOURCE = `contract WitnessFit {
    state {
        streak_count: u64 = 0
        last_checkin_epoch: u64 = 0
        half_life: u64 = 7
        max_streak: u64 = 0
        total_checkins: u64 = 0
        boost_threshold_bp: u64 = 5000
    }

    fn check_in() {
        require(caller == owner, "only the wearer checks in")
        if self.last_checkin_epoch == 0 {
            self.streak_count = 1
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

    fn reset_peak() {
        require(caller == owner, "only the wearer resets peak")
        self.max_streak = self.streak_count
        emit("peak reset")
    }

    fn current_streak() -> u64 {
        if self.last_checkin_epoch == 0 {
            return 0
        }
        if epoch <= self.last_checkin_epoch + self.half_life {
            return self.streak_count
        }
        return 0
    }

    fn has_boost() -> bool {
        if self.max_streak == 0 {
            return false
        }
        if self.last_checkin_epoch == 0 {
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

    fn window_remaining() -> u64 {
        if self.last_checkin_epoch == 0 {
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
`;
