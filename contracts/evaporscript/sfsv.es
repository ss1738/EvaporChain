// SFSV — Singh-Future-Self-Vault. Reference contract behind
// SFSV_VAULT (0x0001_0102, Marketplace lane).
//
// Doctrine claim (from the catalogue): "Sell your future-self's claim —
// third parties bid for delayed-payout vaults via SDDC."
//
// The vault locks a deposit until a release_epoch. While locked, the
// CURRENT beneficiary may `sell(buyer)` — transferring the right-to-
// withdraw to a third party. The buyer pays off-chain (or via a
// paired SDDC auction contract); this contract just records the
// on-chain transfer of claim. After release_epoch, the current
// beneficiary calls `withdraw()` to mark the vault terminal.
//
// Doctrine point: the vault's OWN energy is the deposit's
// structural lifespan. If the contract evaporates BEFORE
// release_epoch, the deposit is forfeit — `on_evaporate` emits
// "deposit forfeit" if the vault never reached release. THIS is why
// the future-self's claim trades at a discount: the future payout
// is structurally uncertain, not just time-discounted. The off-chain
// auction price reflects (a) the time-value-of-money standard
// discount, AND (b) the survival probability of the vault contract
// itself.
//
// One contract = one vault. The deposit_amount is recorded but
// the actual token transfer is handled by a paired tx flow (the
// contract is an attestation primitive, like SCL).

contract SFSV {
    state {
        // ── one-shot vault config ──────────────────────────────────
        future_self: address          // the original beneficiary
        deposit_amount: u64 = 0       // recorded; tokens move off-script
        release_epoch: u64 = 0        // earliest withdraw epoch
        sealed: bool = false

        // ── claim ownership ────────────────────────────────────────
        // beneficiary starts as future_self; sell() transfers it.
        beneficiary: address
        sold: bool = false

        // ── terminal flag ──────────────────────────────────────────
        withdrawn: bool = false
    }

    // Owner-only, one-shot: arm the vault with (future_self,
    // deposit_amount, release_epoch). The deployer becomes the
    // owner; future_self is recorded separately (so a depositor
    // can lock for someone else — e.g., a parent for a child).
    fn arm(self_addr: address, amount: u64, release_at: u64) {
        require(caller == owner, "only depositor arms")
        require(self.sealed == false, "already armed")
        require(amount > 0, "deposit must be positive")
        require(release_at > epoch, "release_epoch must be in the future")
        self.future_self = self_addr
        self.beneficiary = self_addr
        self.deposit_amount = amount
        self.release_epoch = release_at
        self.sealed = true
        emit("vault armed")
    }

    // Current beneficiary sells the claim to `buyer`. One-shot —
    // a chain of sales would require multiple contracts (intentional:
    // each hop is a separate auction event, audit-traceable).
    fn sell(buyer: address) {
        require(self.sealed == true, "not armed")
        require(self.withdrawn == false, "already withdrawn")
        require(self.sold == false, "claim already sold once")
        require(caller == self.beneficiary, "only current beneficiary sells")
        self.beneficiary = buyer
        self.sold = true
        emit("claim sold")
    }

    // Beneficiary withdraws after release_epoch. Marks the vault
    // terminal; subsequent calls fail.
    fn withdraw() {
        require(self.sealed == true, "not armed")
        require(self.withdrawn == false, "already withdrawn")
        require(caller == self.beneficiary, "only beneficiary withdraws")
        require(epoch >= self.release_epoch, "still locked")
        self.withdrawn = true
        emit("vault withdrawn")
    }

    // ── Views ──────────────────────────────────────────────────────

    // Composite view — is the vault armed AND not yet withdrawn AND
    // past the release epoch? The function downstream contracts /
    // auction layers consult.
    fn is_releasable() -> bool {
        if self.sealed == false {
            return false
        }
        if self.withdrawn == true {
            return false
        }
        if epoch < self.release_epoch {
            return false
        }
        return true
    }

    fn epochs_until_release() -> u64 {
        if self.sealed == false {
            return 0
        }
        if epoch >= self.release_epoch {
            return 0
        }
        return self.release_epoch - epoch
    }

    fn is_beneficiary(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.beneficiary
    }

    fn is_original_future_self(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.future_self
    }

    fn deposit_amount_view() -> u64 {
        return self.deposit_amount
    }

    fn release_at() -> u64 {
        return self.release_epoch
    }

    fn is_armed() -> bool {
        return self.sealed
    }

    fn is_sold() -> bool {
        return self.sold
    }

    fn is_withdrawn() -> bool {
        return self.withdrawn
    }

    on_grace() {
        emit("vault fading — refresh to extend the lock OR the claim risks forfeit")
    }

    on_refresh() {
        emit("vault refreshed")
    }

    // The doctrine point. If the vault evaporates BEFORE
    // withdrawal, the deposit is structurally forfeit — the chain
    // can't honour what no longer exists. This is what makes
    // sell()-side discounting non-zero: future payouts can fail
    // to arrive.
    on_evaporate() {
        if self.withdrawn == false {
            emit("vault evaporated unwithdrawn — deposit forfeit")
        } else {
            emit("vault evaporated post-withdrawal")
        }
    }
}
