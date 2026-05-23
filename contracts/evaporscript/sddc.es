// SDDC — Singh Decay-Dutch Continuous Auction (EvaporScript contract).
//
// Per INVENTION_STACK.md §A5.2: the foundational two-axis clearing
// mechanism that underlies SFSV, SHLM, SAP, and SCL. One contract
// instance = one lot. Deploy a new instance per auction.
//
// Two-axis clearing:
//   axis 1 (price)      — Dutch descent from ceiling to floor over
//                         duration epochs; current_price falls linearly.
//   axis 2 (λ-tolerance)— Bidders declare max decay-rate they will
//                         tolerate (lambda_tolerance). The lot declares
//                         its own λ (lot_lambda). A bid clears only when
//                         BOTH conditions are met:
//                           current_price <= bid.max_price
//                           lot_lambda    <= bid.lambda_tolerance
//
// Clearing is first-valid-bid by submission order (the coordinator
// scans the bid registry in the order bids were registered and records
// the first that satisfies both axes at the clearing epoch). This
// contract records the coordinator's decision; it cannot compute ordering
// across all bids inside EvaporScript due to the totality constraint.
//
// Phases:
//   0 = OPEN    — accepting bids
//   1 = CLEARED — a winner was confirmed
//   2 = VOID    — auction closed without a winner (operator called void,
//                 or energy evaporated before clearing)
//
// Payment: off-chain. The coordinator verifies both axes, confirms
// payment transfer at the consensus layer, then calls try_clear here.
// This keeps the contract purely accountable for *who won* at *what
// price* — not for executing the energy transfer.

contract SDDC {
    state {
        // ── lot configuration ─────────────────────────────────────────
        seller: address
        item: string = ""
        ceiling: u64 = 0
        floor: u64 = 0
        lot_lambda: u64 = 0
        duration: u64 = 0        // auction window in epochs
        opened_at: u64 = 0

        // ── lifecycle ─────────────────────────────────────────────────
        sealed: bool = false     // lot configured
        phase: u64 = 0           // 0=OPEN 1=CLEARED 2=VOID

        // ── bid registry ──────────────────────────────────────────────
        // max_price and lambda_tolerance keyed by bidder address.
        // A zero entry means no bid from that address.
        bid_max_price: map[address -> u64]
        bid_lambda: map[address -> u64]
        bid_count: u64 = 0

        // ── settlement ────────────────────────────────────────────────
        winner: address
        price_paid: u64 = 0
        cleared_at: u64 = 0
    }

    // ── initialisation ───────────────────────────────────────────────────

    // Configure the lot one-shot. Only the deployer can call. The
    // `opened_at` epoch is captured here so current_price is anchored
    // to the seal moment, not the deploy moment.
    fn set_lot(
        item_label: string,
        price_ceiling: u64,
        price_floor: u64,
        lambda: u64,
        duration_epochs: u64
    ) {
        require(caller == owner, "only seller can configure lot")
        require(self.sealed == false, "lot already configured")
        require(price_ceiling > price_floor, "ceiling must exceed floor")
        require(lambda > 0, "lot_lambda must be positive")
        require(duration_epochs > 0, "duration must be positive")

        self.seller = owner
        self.item = item_label
        self.ceiling = price_ceiling
        self.floor = price_floor
        self.lot_lambda = lambda
        self.duration = duration_epochs
        self.opened_at = epoch
        self.sealed = true
        emit("lot configured")
    }

    // ── bidding ──────────────────────────────────────────────────────────

    // Submit or update a bid. Any address may bid. Overwrites any previous
    // bid from the same caller. Both axes must be positive.
    fn submit_bid(max_price: u64, lambda_tolerance: u64) {
        require(self.sealed == true, "lot not configured")
        require(self.phase == 0, "auction not open")
        require(max_price >= self.floor, "max_price below floor — bid cannot win")
        require(lambda_tolerance > 0, "lambda_tolerance must be positive")

        let is_new = 0
        if self.bid_max_price[caller] == 0 {
            is_new = 1
        }
        self.bid_max_price[caller] = max_price
        self.bid_lambda[caller] = lambda_tolerance
        if is_new == 1 {
            self.bid_count += 1
        }
        emit("bid registered")
    }

    // ── price query ──────────────────────────────────────────────────────

    // Linear-descent current price:
    //   current_price = ceiling - (ceiling - floor) * elapsed / duration
    // At elapsed = 0: price = ceiling. At elapsed >= duration: price = floor.
    // Using integer arithmetic throughout (no floats).
    fn current_price() -> u64 {
        if self.sealed == false {
            return 0
        }
        let elapsed = epoch - self.opened_at
        if elapsed >= self.duration {
            return self.floor
        }
        let spread = self.ceiling - self.floor
        let drop = spread * elapsed / self.duration
        return self.ceiling - drop
    }

    // ── settlement ───────────────────────────────────────────────────────

    // Record the winning bidder. Called by the off-chain coordinator after
    // it has verified both clearing conditions at `epoch`:
    //   1. current_price(epoch) <= winner_bid.max_price
    //   2. lot_lambda           <= winner_bid.lambda_tolerance
    //
    // The contract verifies the bid registry has an entry for `winner_addr`
    // and that `confirmed_price >= floor` (sanity check). Price must not
    // exceed ceiling either. The coordinator passes the agreed price in
    // `confirmed_price` (which is the current_price at the clearing epoch).
    fn try_clear(winner_addr: address, confirmed_price: u64) {
        // SDDC-1 (audit 2026-05-17): gate on owner so an adversary cannot
        // race the legitimate coordinator by submitting their own address
        // as winner_addr. Owner = deployer = coordinator service account.
        require(caller == owner, "only coordinator can clear")
        require(self.sealed == true, "lot not configured")
        require(self.phase == 0, "auction not open")
        require(self.bid_max_price[winner_addr] > 0, "winner has no bid on file")
        require(confirmed_price >= self.floor, "price below floor")
        require(confirmed_price <= self.ceiling, "price above ceiling")

        // Verify both axes for the recorded winner.
        let winner_max = self.bid_max_price[winner_addr]
        let winner_lam = self.bid_lambda[winner_addr]
        require(confirmed_price <= winner_max, "confirmed price exceeds winner max_price")
        require(self.lot_lambda <= winner_lam, "winner lambda_tolerance below lot_lambda")

        self.winner = winner_addr
        self.price_paid = confirmed_price
        self.cleared_at = epoch
        self.phase = 1
        emit("auction cleared")
    }

    // Operator voids the auction if no valid bid materialises. Only
    // available while the auction is open (phase 0). Coordinator calls
    // this after the window expires with no clearing winner.
    fn void_auction() {
        require(self.sealed == true, "lot not configured")
        require(caller == owner, "only seller can void")
        require(self.phase == 0, "auction not open")

        self.phase = 2
        emit("auction voided — no winner")
    }

    // ── read-only queries ────────────────────────────────────────────────

    fn lot_item() -> string {
        return self.item
    }

    fn auction_phase() -> u64 {
        return self.phase
    }

    fn bid_count_total() -> u64 {
        return self.bid_count
    }

    fn winner_address() -> address {
        return self.winner
    }

    fn winner_price() -> u64 {
        return self.price_paid
    }

    fn lot_ceiling() -> u64 {
        return self.ceiling
    }

    fn lot_floor() -> u64 {
        return self.floor
    }

    fn lot_lambda_rate() -> u64 {
        return self.lot_lambda
    }

    fn epochs_remaining() -> u64 {
        if self.sealed == false {
            return 0
        }
        let deadline = self.opened_at + self.duration
        if epoch >= deadline {
            return 0
        }
        return deadline - epoch
    }

    fn bid_of(who: address) -> u64 {
        return self.bid_max_price[who]
    }

    fn lambda_of(who: address) -> u64 {
        return self.bid_lambda[who]
    }

    // ── lifecycle hooks ──────────────────────────────────────────────────

    // Auction energy entering grace: the lot window may expire before
    // clearing. Coordinator should either clear or void before evaporation.
    on_grace() {
        emit("auction energy low — clear or void before evaporation")
    }

    on_refresh() {
        emit("auction energy refreshed")
    }

    // Evaporation without clearing = implicit void.
    // Off-chain coordinator reads phase == 0 at evaporation to confirm void.
    on_evaporate() {
        if self.phase == 0 {
            emit("auction evaporated — void by physics")
        }
    }
}
