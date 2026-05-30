// SCL — Singh-Capability-Lease. Reference contract behind SCL_LEASE
// (0x0001_0104, Marketplace lane).
//
// Doctrine claim (from the catalogue): "Permission market with
// structural revocation — capabilities can't outlive their purpose."
// This is the THESIS of EvaporChain applied to access control: a
// permission system where rights DECAY rather than persist by
// convention. The classical alternative is a revoke method on a
// global ACL that the principal might forget to call; here, the
// capability is a separate CONTRACT whose ENERGY is its budget —
// once it evaporates, the right is gone, no manual revocation
// needed.
//
// One contract = ONE leased capability between two parties:
//
//   lessor (owner)  —[verb, object_hex, duration_epochs]→  lessee
//
// Methods:
//   arm(lessee, verb, object_hex, duration)  — owner-only, one-shot.
//                                              Seals the lease + records
//                                              granted_at_epoch.
//   exercise()                                — lessee-only; reverts if
//                                              the contract is revoked
//                                              or past `granted_at +
//                                              duration` (duration is
//                                              the SOFT cap; the chain's
//                                              own decay is the HARD
//                                              cap — once the contract
//                                              evaporates, no method
//                                              can be called).
//   revoke()                                  — owner-only; flips the
//                                              revoked flag (early
//                                              terminate).
//   is_active() -> bool                       — composite gate that
//                                              other contracts can
//                                              reference for permission
//                                              checks.
//
// Note: this contract does NOT itself enforce the verb/object_hex
// semantics. It is the ATTESTATION primitive — other contracts /
// off-chain enforcers consult it (via is_active + is_lessee) before
// honouring the action. That keeps the lease portable; the
// enforcement layer above can be swapped without changing the
// capability contract.

contract SCL {
    state {
        // ── one-shot lease config ──────────────────────────────────
        lessee: address
        verb: string = ""
        object_hex: string = ""
        duration_epochs: u64 = 0
        granted_at_epoch: u64 = 0

        // ── flags ──────────────────────────────────────────────────
        // sealed bool doubles as the "has been armed" sentinel,
        // so granted_at_epoch can be the raw epoch (including 0)
        // without colliding with "never armed." Same lesson as
        // mayfly + witnessfit.
        sealed: bool = false
        revoked: bool = false

        // ── counters ───────────────────────────────────────────────
        exercises: u64 = 0
    }

    // Owner-only, one-shot: arm the lease.
    fn arm(to: address, v: string, obj: string, dur: u64) {
        require(caller == owner, "only lessor arms")
        require(self.sealed == false, "already armed")
        require(dur > 0, "duration must be positive")
        self.lessee = to
        self.verb = v
        self.object_hex = obj
        self.duration_epochs = dur
        self.granted_at_epoch = epoch
        self.sealed = true
        emit("capability granted")
    }

    // Lessee exercises the capability. Reverts if:
    //   - lease not yet armed
    //   - lease revoked
    //   - caller is not the lessee
    //   - past the soft expiry (epoch >= granted_at + duration)
    fn exercise() {
        require(self.sealed == true, "not granted")
        require(self.revoked == false, "capability revoked")
        require(caller == self.lessee, "not the lessee")
        require(
            epoch < self.granted_at_epoch + self.duration_epochs,
            "capability expired"
        )
        self.exercises += 1
        emit("capability exercised")
    }

    // Owner-only: terminate the lease early. Idempotent-ish — a
    // second revoke reverts, so callers can tell the difference
    // between "I just revoked it" and "it was already revoked."
    fn revoke() {
        require(caller == owner, "only lessor revokes")
        require(self.sealed == true, "not granted")
        require(self.revoked == false, "already revoked")
        self.revoked = true
        emit("capability revoked")
    }

    // ── Doctrine view: the gate other contracts consult ────────────
    fn is_active() -> bool {
        if self.sealed == false {
            return false
        }
        if self.revoked == true {
            return false
        }
        if epoch >= self.granted_at_epoch + self.duration_epochs {
            return false
        }
        return true
    }

    fn epochs_remaining() -> u64 {
        if self.sealed == false {
            return 0
        }
        if self.revoked == true {
            return 0
        }
        if epoch >= self.granted_at_epoch + self.duration_epochs {
            return 0
        }
        return self.granted_at_epoch + self.duration_epochs - epoch
    }

    fn is_lessee(who: address) -> bool {
        if self.sealed == false {
            return false
        }
        return who == self.lessee
    }

    // Verb + object are immutable post-arm. Both reverts pre-arm
    // because returning empty strings would mask the "not yet
    // armed" state from downstream contracts.
    fn verb_view() -> string {
        require(self.sealed == true, "not granted")
        return self.verb
    }

    fn object_view() -> string {
        require(self.sealed == true, "not granted")
        return self.object_hex
    }

    fn exercises_total() -> u64 {
        return self.exercises
    }

    fn is_sealed() -> bool {
        return self.sealed
    }

    fn is_revoked() -> bool {
        return self.revoked
    }

    fn duration_view() -> u64 {
        return self.duration_epochs
    }

    fn granted_at() -> u64 {
        return self.granted_at_epoch
    }

    on_grace() {
        emit("capability fading — refresh to extend or let it expire")
    }

    on_refresh() {
        emit("capability refreshed")
    }

    // Structural revocation — capability dies with the contract. This
    // is the doctrine point. No bookkeeping needed; the chain's
    // decay IS the revocation primitive.
    on_evaporate() {
        emit("capability evaporated — structurally revoked")
    }
}
