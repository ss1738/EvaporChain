// Single source of truth: `contracts/evaporscript/scl.es`.
// Byte-stable inline copy. Pilot at `mod scl_pilot` is the regression
// barrier proving this exact source parses, compiles, and is
// totality-clean.

export const SCL_SOURCE = `contract SCL {
    state {
        lessee: address
        verb: string = ""
        object_hex: string = ""
        duration_epochs: u64 = 0
        granted_at_epoch: u64 = 0

        sealed: bool = false
        revoked: bool = false

        exercises: u64 = 0
    }

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

    fn revoke() {
        require(caller == owner, "only lessor revokes")
        require(self.sealed == true, "not granted")
        require(self.revoked == false, "already revoked")
        self.revoked = true
        emit("capability revoked")
    }

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

    on_evaporate() {
        emit("capability evaporated — structurally revoked")
    }
}
`;
