// CapabilityLease — EvaporScript on-chain record for the SCL primitive.
//
// Per INVENTION_STACK.md §A5.2 (Singh Capability Lease):
//   "Capability tuples (subject, verb, object, λ_cap) minted as
//    fungible-non-transferable leases. At expiry the underlying right
//    snaps back atomically — no revocation tx, no race."
//
// Each deployed instance IS one lease. The grantor deploys the contract,
// calls grant() once to seal it, and the lease is live until expires_at.
//
// Key structural properties:
//   1. No revoke() function exists — expiry is structural, not transactional.
//      assert_authorized() reverts past expires_at by chain physics alone.
//   2. The subject can only change via record_resale() after a SDDC-cleared
//      Dutch auction. Direct transfer is banned — prevents the "leak-and-
//      flood" subletting attack where N siblings each acquire authority.
//   3. verb and object are opaque 32-byte hashes (passed as address-type
//      in EvaporScript; AccountAddress = [u8; 32] on-chain). Higher layers
//      map those hashes onto domain semantics (treasury method selectors,
//      builder slots, wallet APIs).
//
// Lifecycle:
//   1. Grantor deploys and calls grant(subject, verb, object, duration).
//   2. Subject may call list_for_sale() → off-chain SDDC coordinator
//      Dutch-clears → coordinator calls record_resale(winner_addr).
//   3. Any contract (gated dApp) calls assert_authorized(verb, object)
//      to prove the caller holds the active lease.
//   4. At expires_at the lease is structurally dead. No cleanup needed.
//
// Invariants enforced on-chain:
//   - grant() is one-shot; sealed = true is terminal for configuration.
//   - assert_authorized() reverts if epoch > expires_at (structural expiry).
//   - subject transfer only via record_resale() gated by owner (coordinator).
//   - No revoke path exists — the contract has no revoke() function.

contract CapabilityLease {
    state {
        // ── capability tuple ──────────────────────────────────────────
        // verb and object are opaque 32-byte hashes encoded as address.
        // Examples: verb = blake3("treasury.withdraw.v1")
        //           object = blake3("vault.multisig.0xDEAD")
        verb: address
        object_hash: address     // "object" is a reserved word; use object_hash

        // ── lease identity ────────────────────────────────────────────
        subject: address         // current holder of the capability
        expires_at: u64 = 0     // epoch at which the lease structurally dies

        // ── lifecycle ─────────────────────────────────────────────────
        sealed: bool = false     // set once by grant(); locks all parameters

        // ── secondary-market listing (SDDC resale) ────────────────────
        listed: bool = false
        list_ceiling: u64 = 0
        list_floor: u64 = 0
        list_opened_at: u64 = 0
        list_duration: u64 = 0
    }

    // ── initialisation ───────────────────────────────────────────────────

    // Issue the lease. Only the deployer (owner = grantor) can call.
    // duration is in epochs from the current epoch.
    // The verb_hash and object_hash are arbitrary 32-byte identifiers —
    // the contract stores them and asserts against them in assert_authorized.
    fn grant(
        subject_addr: address,
        verb_hash: address,
        obj_hash: address,
        duration: u64
    ) {
        require(caller == owner, "only grantor can issue lease")
        require(self.sealed == false, "lease already issued")
        require(duration > 0, "duration must be positive")
        require(duration <= 1000000, "duration exceeds maximum (1M epochs)")

        self.verb = verb_hash
        self.object_hash = obj_hash
        self.subject = subject_addr
        self.expires_at = epoch + duration
        self.sealed = true
        emit("capability lease granted")
    }

    // ── authorisation gate ───────────────────────────────────────────────

    // Assert that the caller is currently authorised to invoke verb_hash
    // on obj_hash. Reverts — and therefore the calling transaction fails —
    // if any condition is not met. This is the structural revocation:
    // epoch > expires_at makes every authorisation call impossible with no
    // revocation transaction having been needed.
    //
    // Gated dApps call this at the top of any privileged function.
    fn assert_authorized(verb_hash: address, obj_hash: address) {
        require(self.sealed == true, "lease not yet issued")
        require(epoch <= self.expires_at, "capability lease expired — structural revocation")
        require(caller == self.subject, "caller is not the lease subject")
        require(verb_hash == self.verb, "verb hash mismatch")
        require(obj_hash == self.object_hash, "object hash mismatch")
        emit("capability authorized")
    }

    // ── secondary market (SDDC resale) ──────────────────────────────────

    // Open a secondary-market SDDC listing. Only the current subject can
    // list their remaining lease term. Listing while expired is rejected.
    fn list_for_sale(ceiling: u64, floor: u64, duration: u64) {
        require(self.sealed == true, "lease not issued")
        require(epoch <= self.expires_at, "lease expired — cannot list dead lease")
        require(caller == self.subject, "only current subject can list")
        require(self.listed == false, "listing already active")
        require(ceiling > floor, "ceiling must exceed floor")
        require(duration > 0, "duration must be positive")
        // Listing duration must fit within the remaining lease term.
        let remaining = self.expires_at - epoch
        require(duration <= remaining, "listing duration exceeds remaining lease term")

        self.list_ceiling = ceiling
        self.list_floor = floor
        self.list_opened_at = epoch
        self.list_duration = duration
        self.listed = true
        emit("lease listed for sale")
    }

    // Cancel an active listing. Only the current subject can cancel.
    fn cancel_listing() {
        require(self.listed == true, "no active listing")
        require(caller == self.subject, "only current subject can cancel")

        self.listed = false
        self.list_ceiling = 0
        self.list_floor = 0
        self.list_opened_at = 0
        self.list_duration = 0
        emit("listing cancelled")
    }

    // Record a SDDC-cleared resale. Only the grantor/coordinator (owner)
    // can call this — prevents a third party racing the coordinator to
    // claim the winner slot (SDDC-1 / SFSV-1 class fix applied here).
    // On success, subject changes to winner_addr and the listing clears.
    fn record_resale(winner_addr: address) {
        require(caller == owner, "only coordinator can record resale")
        require(self.sealed == true, "lease not issued")
        require(self.listed == true, "no active listing")
        require(epoch <= self.expires_at, "lease expired")
        let listing_expired = 0
        if epoch > self.list_opened_at + self.list_duration {
            listing_expired = 1
        }
        require(listing_expired == 0, "listing period has elapsed")

        self.subject = winner_addr
        self.listed = false
        self.list_ceiling = 0
        self.list_floor = 0
        self.list_opened_at = 0
        self.list_duration = 0
        emit("lease subject transferred via resale")
    }

    // ── read-only queries ────────────────────────────────────────────────

    fn current_subject() -> address {
        return self.subject
    }

    // Returns 1 if the lease is currently active for the given subject,
    // 0 otherwise. Pure — no state change.
    fn is_active_for(subject_addr: address) -> u64 {
        if self.sealed == false {
            return 0
        }
        if epoch > self.expires_at {
            return 0
        }
        if subject_addr == self.subject {
            return 1
        }
        return 0
    }

    fn epochs_remaining() -> u64 {
        if self.sealed == false {
            return 0
        }
        if epoch >= self.expires_at {
            return 0
        }
        return self.expires_at - epoch
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

    // Energy evaporation = the lease is permanently dead. Any authority
    // delegated under this lease vanishes with the contract. Gated dApps
    // watching for "capability lease evaporated" events should revoke
    // any cached authorisation they stored.
    on_evaporate() {
        emit("capability lease evaporated — permission permanently extinct")
    }
}
