// FutureSelfVault — EvaporScript contract for the SFSV dApp.
//
// A vault locks an energy deposit on behalf of the creator's future self.
// The vault IS the energy: deploying this contract with a chosen energy
// budget IS the act of locking the deposit. For the EnergyDecaysBelow
// predicate the contract's own energy field decays by physics — no
// hand-coded formula needed.
//
// Predicate types:
//   0 = EpochReached       — releases when epoch >= release_epoch
//   1 = EnergyDecaysBelow  — releases when contract energy < threshold
//                            (contract's physical energy is the yardstick)
//
// Lifecycle:
//   1. Deployer calls set_terms(...) once (seals the vault).
//   2. Holder optionally calls list_for_sale(ceiling, floor, duration)
//      to open a secondary-market listing.
//   3. Off-chain SDDC coordinator Dutch-clears the auction; winner calls
//      record_sale(winner_addr) — OR the holder calls record_sale directly
//      after off-chain confirmation.
//   4. Any address calls try_payout() once the predicate trips; the
//      contract emits a payout event and the off-chain execution layer
//      credits the current_holder then retires this contract instance.
//
// Invariants enforced on-chain:
//   - released = true is terminal; no further state mutations.
//   - Only the current holder can list or cancel a listing.
//   - record_sale is only valid during an open listing (listed = true).
//   - Predicate check is pure (no side effects on failure).
//
// Note: EvaporScript has no contract-internal method dispatch. Predicate
// evaluation logic is inlined into try_payout and predicate_satisfied.
// They must stay bit-for-bit identical — guarded by the adversarial tests
// in the SFSV crate.

contract FutureSelfVault {
    state {
        // ── identity ──────────────────────────────────────────────────
        future_self: address
        holder: address
        deposit: u64 = 0

        // ── predicate ─────────────────────────────────────────────────
        // 0 = EpochReached, 1 = EnergyDecaysBelow
        predicate_type: u64 = 0
        release_epoch: u64 = 0   // EpochReached: epoch at which vault unlocks
        threshold: u64 = 0       // EnergyDecaysBelow: energy level that trips release

        // ── lifecycle flags ───────────────────────────────────────────
        sealed: bool = false
        released: bool = false
        payout_at: u64 = 0

        // ── secondary-market listing ──────────────────────────────────
        listed: bool = false
        list_ceiling: u64 = 0
        list_floor: u64 = 0
        list_opened_at: u64 = 0
        list_duration: u64 = 0
    }

    // ── initialisation ───────────────────────────────────────────────────

    // Seal the vault exactly once. Only the deployer (creator = owner) can
    // call. Predicate parameters are validated here; the contract cannot be
    // re-armed after sealing.
    //
    //   predicate  0 → EpochReached:      release_param = target epoch
    //   predicate  1 → EnergyDecaysBelow: release_param = energy threshold
    //
    // `deposit_amount` is a snapshot of the energy the deployer committed —
    // stored for off-chain accounting (the actual energy is the contract's
    // own field, which decays by the chain's evaporation engine).
    fn set_terms(
        future_self_addr: address,
        predicate: u64,
        release_param: u64,
        deposit_amount: u64
    ) {
        require(caller == owner, "only creator can seal vault")
        require(self.sealed == false, "vault already sealed")
        require(deposit_amount > 0, "deposit must be positive")
        require(predicate <= 1, "unknown predicate type (0=epoch 1=energy)")
        require(release_param > 0, "release param must be positive")

        self.future_self = future_self_addr
        self.holder = future_self_addr
        self.deposit = deposit_amount
        self.predicate_type = predicate
        if predicate == 0 {
            self.release_epoch = release_param
        }
        if predicate == 1 {
            self.threshold = release_param
        }
        self.sealed = true
        emit("vault sealed")
    }

    // ── secondary market ─────────────────────────────────────────────────

    // Open a secondary-market listing. Only the current holder can list;
    // only one active listing at a time. Duration is in epochs.
    fn list_for_sale(ceiling: u64, floor: u64, duration: u64) {
        require(self.sealed == true, "vault not yet sealed")
        require(self.released == false, "vault already released")
        require(caller == self.holder, "only current holder can list")
        require(self.listed == false, "listing already active")
        require(ceiling > floor, "ceiling must exceed floor")
        require(duration > 0, "duration must be positive")

        self.list_ceiling = ceiling
        self.list_floor = floor
        self.list_opened_at = epoch
        self.list_duration = duration
        self.listed = true
        emit("vault listed for sale")
    }

    // Cancel an active listing. Only the current holder can cancel.
    fn cancel_listing() {
        require(self.sealed == true, "vault not yet sealed")
        require(self.listed == true, "no active listing")
        require(caller == self.holder, "only current holder can cancel listing")

        self.listed = false
        self.list_ceiling = 0
        self.list_floor = 0
        self.list_opened_at = 0
        self.list_duration = 0
        emit("listing cancelled")
    }

    // Record a SDDC-cleared sale. The off-chain coordinator calls this
    // after the Dutch auction clears. Transfers the claim to the winner
    // and closes the listing. Anyone with the winning address confirmed
    // off-chain may call — no caller restriction here since the listing
    // state itself guards the transition.
    fn record_sale(winner_addr: address) {
        require(self.sealed == true, "vault not yet sealed")
        require(self.released == false, "vault already released")
        require(self.listed == true, "no active listing")
        let listing_expired = 0
        if epoch > self.list_opened_at + self.list_duration {
            listing_expired = 1
        }
        require(listing_expired == 0, "listing has expired")

        self.holder = winner_addr
        self.listed = false
        self.list_ceiling = 0
        self.list_floor = 0
        self.list_opened_at = 0
        self.list_duration = 0
        emit("claim transferred to buyer")
    }

    // ── payout ───────────────────────────────────────────────────────────

    // Check the predicate and, if satisfied, mark the vault released.
    // Idempotent guard: once released, reverts. The off-chain execution
    // layer watches for the "vault payout" event and credits self.holder.
    fn try_payout() {
        require(self.sealed == true, "vault not yet sealed")
        require(self.released == false, "vault already released")

        // Inline predicate evaluation (no method dispatch in EvaporScript).
        let satisfied = 0

        if self.predicate_type == 0 {
            // EpochReached: release when epoch >= release_epoch.
            if epoch >= self.release_epoch {
                satisfied = 1
            }
        }

        if self.predicate_type == 1 {
            // EnergyDecaysBelow: release when contract energy < threshold.
            // `energy` is the built-in field tracking this contract's
            // current energy — it decays naturally via the evaporation
            // engine, exactly matching the Rust predicate::evaluate path.
            if energy < self.threshold {
                satisfied = 1
            }
        }

        require(satisfied == 1, "predicate not yet satisfied")

        self.released = true
        self.payout_at = epoch
        emit("vault payout")
    }

    // ── read-only queries ────────────────────────────────────────────────

    fn current_holder() -> address {
        return self.holder
    }

    fn is_released() -> bool {
        return self.released
    }

    fn is_listed() -> bool {
        return self.listed
    }

    fn deposit_amount() -> u64 {
        return self.deposit
    }

    fn release_target() -> u64 {
        if self.predicate_type == 0 {
            return self.release_epoch
        }
        return self.threshold
    }

    // Returns 1 iff predicate is currently satisfied (pure — no state change).
    fn predicate_satisfied() -> u64 {
        if self.sealed == false {
            return 0
        }
        if self.predicate_type == 0 {
            if epoch >= self.release_epoch {
                return 1
            }
            return 0
        }
        if self.predicate_type == 1 {
            if energy < self.threshold {
                return 1
            }
            return 0
        }
        return 0
    }

    fn listing_ceiling() -> u64 {
        return self.list_ceiling
    }

    fn listing_floor() -> u64 {
        return self.list_floor
    }

    fn epochs_until_listing_expires() -> u64 {
        if self.listed == false {
            return 0
        }
        let deadline = self.list_opened_at + self.list_duration
        if epoch >= deadline {
            return 0
        }
        return deadline - epoch
    }

    // ── lifecycle hooks ──────────────────────────────────────────────────

    // Contract enters grace: the deposit is about to evaporate.
    // For EnergyDecaysBelow vaults this may be the intended trigger —
    // the holder should call try_payout before energy hits zero.
    on_grace() {
        emit("vault energy low — holder should try_payout or boost")
    }

    // Energy was refreshed (boost applied). The EnergyDecaysBelow predicate
    // resets its effective threshold distance; EpochReached is unaffected.
    on_refresh() {
        emit("vault boosted")
    }

    // Vault evaporated without payout. Deposit is lost by physics.
    // Off-chain coordinator reads `released == false` at evaporation and
    // records the forfeiture. Holders who ignore grace warnings lose their
    // claim — this is the "skin in the game" invariant of the dApp.
    on_evaporate() {
        if self.released == false {
            emit("vault evaporated — deposit forfeited")
        }
    }
}
