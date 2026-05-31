// DeadMan Switch — secret-release contract where the chain's own
// epoch advancement IS the trigger. No external keeper service.
// No expensive ongoing storage burn. The doctrine ("data fades
// unless refreshed") IS the product.
//
// Doctrine claim: a publishable secret commitment (e.g., a hash of
// some off-chain payload — a leaked dataset, an inheritance key, a
// kill-switch credential) is held in escrow by an EvaporChain
// contract. The holder must `refresh()` within `refresh_window`
// epochs. If they miss the window, ANYONE can call `release_dead()`
// to publish the payload on-chain. The holder may also voluntarily
// `trigger_early()` to release at will.
//
// Why this is doctrine-native (and not just "any chain can do
// this"): the chain has structural death. Once released, the
// payload event lives in the eulogy/tombstone even after the
// contract itself evaporates — it doesn't sit forever in active
// state burning storage. The four-act lifecycle does the cleanup
// for free: Active (refreshing) → Grace (deadline passed,
// releasable) → Ghost (released, energy decaying) → Tomb (eulogy
// preserves the release fact). Other chains release secrets fine;
// EvaporChain releases them AND cleans up after.
//
// One contract = one switch. Multi-switch portfolios deploy N
// instances. Cheap; the per-switch state is tiny.

contract DeadManSwitch {
    state {
        // ── Configuration ──────────────────────────────────────────
        holder: address                       // who can refresh / trigger
        secret_hash: string = ""              // committed payload (released as-is)
        refresh_window: u64 = 0               // epochs holder may go silent

        // ── Refresh ledger ─────────────────────────────────────────
        last_refresh_epoch: u64 = 0
        // sentinel: epoch=0 vs never-refreshed is the same hazard
        // we've already hit in mortal_dao + witnessfit + mnemochain.
        has_refreshed: bool = false
        refresh_count: u64 = 0

        // ── Release state ──────────────────────────────────────────
        released: bool = false
        released_at_epoch: u64 = 0
        released_by: address                  // who fired the release
        // The plaintext the releaser provided (if any). Lets the
        // releaser optionally publish not just the commitment hash
        // but the actual data alongside. Most uses leave this empty
        // and reveal off-chain; some use cases (kill-switch flags,
        // small messages) put the plaintext on-chain directly.
        revealed_secret: string = ""

        // ── Setup gate ─────────────────────────────────────────────
        sealed: bool = false                  // arm() called
    }

    // Owner-only, one-shot: arm with (holder, secret_hash,
    // refresh_window). The deployer locks the configuration; after
    // arm() the only mutator is the holder (refresh/trigger) or
    // anyone-after-deadline (release_dead).
    fn arm(switch_holder: address, payload_hash: string, window_epochs: u64) {
        require(caller == owner, "only deployer arms")
        require(self.sealed == false, "already armed")
        require(window_epochs > 0, "window must be positive")
        self.holder = switch_holder
        self.secret_hash = payload_hash
        self.refresh_window = window_epochs
        self.last_refresh_epoch = epoch
        self.has_refreshed = true
        self.refresh_count = 1
        self.sealed = true
        emit("dead-man switch armed")
    }

    // Holder pushes the deadline forward. Resets the silent-counter
    // to zero. Cannot refresh after release.
    fn refresh() {
        require(self.sealed == true, "not armed")
        require(self.released == false, "already released")
        require(caller == self.holder, "only holder refreshes")
        self.last_refresh_epoch = epoch
        self.has_refreshed = true
        self.refresh_count += 1
        emit("switch refreshed")
    }

    // Holder voluntarily fires the switch. Common case: the holder
    // realises they want to release on a specific event (death,
    // arrest, deal-collapse) rather than waiting for the deadline.
    // The plaintext reveal is optional — pass "" to release only
    // the commitment hash and reveal off-chain.
    fn trigger_early(plaintext: string) {
        require(self.sealed == true, "not armed")
        require(self.released == false, "already released")
        require(caller == self.holder, "only holder triggers early")
        self.released = true
        self.released_at_epoch = epoch
        self.released_by = self.holder
        self.revealed_secret = plaintext
        emit("switch triggered early by holder")
    }

    // Anyone may fire the switch after the deadline lapses. The
    // doctrine moment: the chain's own epoch advancement is the
    // trigger; nobody needs to run a keeper service to make this
    // work. Anyone observing the chain can release.
    //
    // Releaser can OPTIONALLY supply the plaintext if they already
    // know it (e.g., they're the holder's chosen executor who held
    // the key in escrow). Otherwise the contract just publishes the
    // commitment hash + the "dead" fact.
    fn release_dead(plaintext: string) {
        require(self.sealed == true, "not armed")
        require(self.released == false, "already released")
        require(epoch >= self.last_refresh_epoch + self.refresh_window,
                "deadline not yet passed")
        self.released = true
        self.released_at_epoch = epoch
        self.released_by = caller
        self.revealed_secret = plaintext
        emit("switch released — deadline lapsed")
    }

    // Holder rotates control. Useful when the original holder
    // hands off responsibility (e.g., named successor takes over
    // the refresh duty). Holder rotation does NOT touch the
    // deadline — last_refresh_epoch stays where it is.
    fn transfer_holder(new_holder: address) {
        require(self.sealed == true, "not armed")
        require(self.released == false, "released — holder role retired")
        require(caller == self.holder, "only current holder transfers")
        self.holder = new_holder
        emit("holder transferred")
    }

    // ── Views ──────────────────────────────────────────────────────

    fn is_armed() -> bool {
        return self.sealed
    }

    fn is_released() -> bool {
        return self.released
    }

    // "Alive" = armed, not released, AND the holder is still within
    // the refresh window. This is the green-light state where the
    // switch is doing its passive job.
    fn is_alive() -> bool {
        if self.sealed == false {
            return false
        }
        if self.released == true {
            return false
        }
        if epoch >= self.last_refresh_epoch + self.refresh_window {
            return false
        }
        return true
    }

    // "Dead but not yet released" = armed, deadline passed, but
    // nobody has called release_dead() yet. This is the window in
    // which any observer can fire the switch.
    fn is_releasable() -> bool {
        if self.sealed == false {
            return false
        }
        if self.released == true {
            return false
        }
        if epoch < self.last_refresh_epoch + self.refresh_window {
            return false
        }
        return true
    }

    // How many epochs the holder has left before the switch
    // becomes releasable. Returns 0 if already past the deadline.
    fn epochs_until_deadline() -> u64 {
        if self.sealed == false {
            return 0
        }
        if epoch >= self.last_refresh_epoch + self.refresh_window {
            return 0
        }
        return self.last_refresh_epoch + self.refresh_window - epoch
    }

    fn secret_hash_view() -> string {
        return self.secret_hash
    }

    fn revealed_secret_view() -> string {
        require(self.released == true, "not yet released")
        return self.revealed_secret
    }

    fn released_at_view() -> u64 {
        require(self.released == true, "not yet released")
        return self.released_at_epoch
    }

    fn refresh_count_view() -> u64 {
        return self.refresh_count
    }

    fn last_refresh_view() -> u64 {
        return self.last_refresh_epoch
    }

    fn holder_view() -> address {
        require(self.sealed == true, "not armed")
        return self.holder
    }

    fn is_holder(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.holder
    }

    on_grace() {
        emit("switch contract energy low — refresh or release before evaporation")
    }

    on_refresh() {
        emit("switch contract refreshed")
    }

    on_evaporate() {
        // If the switch contract itself evaporates while still alive
        // and unreleased, the secret commitment dies with it. This
        // is doctrinally consistent: data fades unless refreshed,
        // and that applies to the dead-man's switch too.
        emit("switch evaporated — secret commitment archived to tomb")
    }
}
