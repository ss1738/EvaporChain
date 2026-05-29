// Single source of truth: `contracts/evaporscript/decay_access_pass.es`.
// This is the inline copy the dApp ships with — keep it byte-identical to
// the .es file. The Rust pilot test
// `evaporchain-script` → `mod decay_access_pass_pilot` (in lib.rs) is the
// regression barrier proving this exact source compiles + runs through the
// VM and is totality-clean. Don't edit the body here without running it.
//
// Deploy flow:
//   1. POST /api/tx/deploy-script  with source_code = DECAY_ACCESS_PASS_SOURCE
//      + energy (initial strength) + half_life (decay rate).
//   2. POST /api/tx/call-script    method = "issue", args = [holder, floor].
//   Then "is_valid" / "require_valid" / "revoke" as needed.

export const DECAY_ACCESS_PASS_SOURCE = `contract DecayAccessPass {
    state {
        holder: address
        validity_floor: u64 = 0
        revoked: bool = false
        issued_epoch: u64 = 0
        sealed: bool = false
    }

    fn issue(pass_holder: address, floor: u64) {
        require(caller == owner, "only issuer can issue")
        require(self.sealed == false, "pass already issued")
        self.holder = pass_holder
        self.validity_floor = floor
        self.issued_epoch = epoch
        self.sealed = true
        emit("pass issued")
    }

    fn revoke() {
        require(caller == owner, "only issuer can revoke")
        require(self.revoked == false, "pass already revoked")
        self.revoked = true
        emit("pass revoked")
    }

    fn is_valid() -> bool {
        return (self.sealed == true) && (self.revoked == false) && (energy >= self.validity_floor)
    }

    fn require_valid() -> bool {
        require(caller == self.holder, "not the holder")
        require(self.sealed == true, "pass not issued")
        require(self.revoked == false, "pass revoked")
        require(energy >= self.validity_floor, "pass expired or below floor")
        return true
    }

    on_grace() { emit("pass weakening") }
    on_evaporate() { emit("pass expired") }
    on_refresh() { emit("pass refreshed") }
}
`;
