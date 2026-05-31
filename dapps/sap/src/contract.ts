// Single source of truth: `contracts/evaporscript/sap.es`. Byte-stable
// inline copy. Pilot at `mod sap_pilot` is the regression barrier.

export const SAP_SOURCE = `contract SAP {
    state {
        initial_value: u64 = 1000
        half_life: u64 = 45
        max_aq_per_window: u64 = 10
        window_epochs: u64 = 60
        sealed: bool = false

        aq_born: map[address -> u64]
        aq_redeemed: map[address -> u64]

        window_start: u64 = 0
        issued_this_window: u64 = 0

        total_issued: u64 = 0
        total_redeemed: u64 = 0
    }

    fn arm(initial: u64, hl: u64, max_aq: u64, window: u64) {
        require(caller == owner, "only issuer arms")
        require(self.sealed == false, "already armed")
        require(initial > 0, "initial value must be positive")
        require(hl > 0, "half_life must be positive")
        require(max_aq > 0, "max_aq_per_window must be positive")
        require(window > 0, "window_epochs must be positive")
        self.initial_value = initial
        self.half_life = hl
        self.max_aq_per_window = max_aq
        self.window_epochs = window
        self.sealed = true
        emit("issuer armed")
    }

    fn issue(recipient: address) {
        require(self.sealed == true, "not armed")
        require(caller == owner, "only issuer issues")
        require(
            self.aq_born[recipient] == 0 || self.aq_redeemed[recipient] == 1,
            "recipient has an outstanding AQ — redeem first"
        )
        if epoch >= self.window_start + self.window_epochs {
            self.window_start = epoch
            self.issued_this_window = 0
        }
        require(
            self.issued_this_window < self.max_aq_per_window,
            "rate cap reached for this window"
        )
        self.aq_born[recipient] = epoch + 1
        self.aq_redeemed[recipient] = 0
        self.issued_this_window += 1
        self.total_issued += 1
        emit("AQ issued")
    }

    fn redeem() {
        require(self.sealed == true, "not armed")
        require(self.aq_born[caller] != 0, "no AQ to redeem")
        require(self.aq_redeemed[caller] == 0, "AQ already redeemed")
        self.aq_redeemed[caller] = 1
        self.total_redeemed += 1
        emit("AQ redeemed")
    }

    fn current_value(who: address) -> u64 {
        if self.aq_born[who] == 0 {
            return 0
        }
        if self.aq_redeemed[who] == 1 {
            return 0
        }
        if epoch + 1 < self.aq_born[who] {
            return self.initial_value
        }
        if (epoch + 1 - self.aq_born[who]) / self.half_life >= 64 {
            return 0
        }
        return self.initial_value >> ((epoch + 1 - self.aq_born[who]) / self.half_life)
    }

    fn has_active_aq(who: address) -> bool {
        if self.aq_born[who] == 0 {
            return false
        }
        if self.aq_redeemed[who] == 1 {
            return false
        }
        if epoch + 1 < self.aq_born[who] {
            return true
        }
        if (epoch + 1 - self.aq_born[who]) / self.half_life >= 64 {
            return false
        }
        return self.initial_value >> ((epoch + 1 - self.aq_born[who]) / self.half_life) > 0
    }

    fn epochs_until_expiry(who: address) -> u64 {
        if self.aq_born[who] == 0 {
            return 0
        }
        if self.aq_redeemed[who] == 1 {
            return 0
        }
        if epoch + 1 < self.aq_born[who] {
            return 64 * self.half_life
        }
        if (epoch + 1 - self.aq_born[who]) / self.half_life >= 64 {
            return 0
        }
        return self.aq_born[who] + 64 * self.half_life - epoch - 1
    }

    fn aq_born_view(who: address) -> u64 {
        return self.aq_born[who]
    }

    fn aq_is_redeemed(who: address) -> bool {
        return self.aq_redeemed[who] == 1
    }

    fn issued_in_current_window() -> u64 {
        return self.issued_this_window
    }

    fn current_window_start() -> u64 {
        return self.window_start
    }

    fn slots_left_in_window() -> u64 {
        if self.issued_this_window >= self.max_aq_per_window {
            return 0
        }
        return self.max_aq_per_window - self.issued_this_window
    }

    fn is_armed() -> bool {
        return self.sealed
    }

    fn initial_value_view() -> u64 {
        return self.initial_value
    }

    fn half_life_view() -> u64 {
        return self.half_life
    }

    fn max_aq_per_window_view() -> u64 {
        return self.max_aq_per_window
    }

    fn window_epochs_view() -> u64 {
        return self.window_epochs
    }

    fn total_issued_view() -> u64 {
        return self.total_issued
    }

    fn total_redeemed_view() -> u64 {
        return self.total_redeemed
    }

    on_grace() {
        emit("issuer energy low — refresh to keep minting attention")
    }

    on_refresh() {
        emit("issuer refreshed")
    }

    on_evaporate() {
        emit("issuer evaporated — no new attention quanta")
    }
}
`;
