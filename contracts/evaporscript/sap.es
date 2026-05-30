// SAP — Singh-Attention-Pulse / Attention Quantum. Reference contract
// behind SAP_AQ (0x0001_0105, Marketplace lane).
//
// Doctrine claim (from the catalogue): "Attention with a half-life.
// 5-min-old AQ worth more than 25-min one. Anti-Sybil rate cap."
//
// One contract = one ISSUER (typically a content creator or a node
// providing a service). The issuer mints Attention Quanta (AQs) to
// recipients via `issue(recipient)`. Each AQ has a per-recipient
// born_epoch + redeemed flag stored in the contract. Value at age
// is computed by `current_value(who)` / `value_at_epoch(who, e)` —
// linear decay from `initial_value` to 0 over `2 * half_life`
// epochs. This is the V1 EvaporScript approximation to the
// exponential half-life curve; the value at exactly `half_life`
// epochs is `initial_value / 2`, matching the doctrine intent.
// V2 with bit-shift support can swap in exact halvings without
// touching the rest of the interface.
//
// Anti-Sybil: the issuer's mint rate is capped at
// `max_aq_per_window` per `window_epochs`-epoch window. The window
// rolls automatically on the next issue() that crosses the boundary.
//
// One AQ per recipient at a time. Re-issuing to the same recipient
// requires they redeem their outstanding AQ first (redeem() is
// always available — even on expired AQs — and marks the slot
// terminal so a new AQ can be issued).

contract SAP {
    state {
        // ── issuer config (one-shot via arm) ───────────────────────
        initial_value: u64 = 1000
        half_life: u64 = 45
        max_aq_per_window: u64 = 10
        window_epochs: u64 = 60
        sealed: bool = false

        // ── per-recipient AQ state ─────────────────────────────────
        // aq_born stores `real_born_epoch + 1` so a born=0 sentinel
        // means "never issued an AQ to this recipient" — the same
        // +1 trick used in mortal_dao / refresh_market.
        aq_born: map[address -> u64]
        aq_redeemed: map[address -> u64]

        // ── rate-cap state ─────────────────────────────────────────
        window_start: u64 = 0
        issued_this_window: u64 = 0

        // ── counters ───────────────────────────────────────────────
        total_issued: u64 = 0
        total_redeemed: u64 = 0
    }

    // Owner-only, one-shot: arm the issuer with the curve + rate-cap
    // policy.
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

    // Owner-only: mint a new AQ for `recipient`. The window rolls
    // automatically when crossed; the rate-cap check fires against
    // the rolled count. Recipient must NOT already hold an
    // outstanding (un-redeemed) AQ from this issuer; if they do,
    // they must redeem first (redeem() works on any AQ, expired
    // or not).
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

    // Recipient redeems their own AQ — marks the slot terminal.
    // Works on any non-redeemed AQ, expired or not (an expired AQ
    // has value 0; the marketplace would price it at zero either
    // way).
    fn redeem() {
        require(self.sealed == true, "not armed")
        require(self.aq_born[caller] != 0, "no AQ to redeem")
        require(self.aq_redeemed[caller] == 0, "AQ already redeemed")
        self.aq_redeemed[caller] = 1
        self.total_redeemed += 1
        emit("AQ redeemed")
    }

    // ── Doctrine view: value at the current epoch ──────────────────
    // Linear decay from initial_value to 0 over 2 * half_life epochs.
    // value(age) = initial * (2*hl - age) / (2*hl)
    // where age = epoch + 1 - aq_born[who].
    // Returns 0 if never issued, already redeemed, or past 2*hl
    // epochs since mint.
    fn current_value(who: address) -> u64 {
        if self.aq_born[who] == 0 {
            return 0
        }
        if self.aq_redeemed[who] == 1 {
            return 0
        }
        if epoch + 1 >= self.aq_born[who] + 2 * self.half_life {
            return 0
        }
        return self.initial_value * (self.aq_born[who] + 2 * self.half_life - epoch - 1) / (2 * self.half_life)
    }

    // ── Composite gate: is the AQ live ─────────────────────────────
    fn has_active_aq(who: address) -> bool {
        if self.aq_born[who] == 0 {
            return false
        }
        if self.aq_redeemed[who] == 1 {
            return false
        }
        if epoch + 1 >= self.aq_born[who] + 2 * self.half_life {
            return false
        }
        return true
    }

    // ── Epochs until the AQ's value hits 0 ─────────────────────────
    fn epochs_until_expiry(who: address) -> u64 {
        if self.aq_born[who] == 0 {
            return 0
        }
        if self.aq_redeemed[who] == 1 {
            return 0
        }
        if epoch + 1 >= self.aq_born[who] + 2 * self.half_life {
            return 0
        }
        return self.aq_born[who] + 2 * self.half_life - epoch - 1
    }

    fn aq_born_view(who: address) -> u64 {
        return self.aq_born[who]
    }

    fn aq_is_redeemed(who: address) -> bool {
        return self.aq_redeemed[who] == 1
    }

    // ── Rate-cap inspection ────────────────────────────────────────
    fn issued_in_current_window() -> u64 {
        // Note: this returns the RECORDED value; a query right after
        // the window boundary but before the next issue() shows the
        // pre-roll counter. Off-chain code should treat this as a
        // soft hint and confirm via issue().
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

    // ── Pre-arm-safe views ─────────────────────────────────────────
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
