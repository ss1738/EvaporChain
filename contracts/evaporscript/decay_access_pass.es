// DecayAccessPass — reference EvaporScript contract for the
// decay-credential pattern (the on-chain expression of the
// `evaporchain-decay-credential` substrate primitive).
//
// The pass's STRENGTH is the contract's own energy. The chain runtime
// decays it every epoch through `energy_at_epoch`, and the bare `energy`
// execution-context builtin exposes the live decayed value. The pass is
// valid only while that strength stays at or above `validity_floor` — so
// it evaporates unless the issuer refreshes the contract's energy (a
// chain-level RefreshScript tx). Issuer-only issue + revoke; revoke is
// terminal. Holder-gated exercise.
//
// Doctrine: "trust is a flow, not a stock" — an attestation that fades
// unless actively maintained, mirroring how the chain treats all state.
//
// Totality-clean (no `while`): passes the `script_vm_mode = "total"`
// deploy gate. All on-chain state is bounded (one address, three u64,
// one bool, one-shot seal flag).
//
// Deploy via DeployScript with `energy` = initial pass strength and
// `half_life` = decay rate; then call `issue(holder, floor)` exactly once.

contract DecayAccessPass {
    state {
        holder: address
        validity_floor: u64 = 0
        revoked: bool = false
        issued_epoch: u64 = 0
        sealed: bool = false
    }

    // Issue the pass exactly once: bind a holder + set the validity
    // floor. Issuer-only (`caller == owner`). The strength is the
    // contract's energy, chosen by the issuer at deploy time.
    fn issue(pass_holder: address, floor: u64) {
        require(caller == owner, "only issuer can issue")
        require(self.sealed == false, "pass already issued")
        self.holder = pass_holder
        self.validity_floor = floor
        self.issued_epoch = epoch
        self.sealed = true
        emit("pass issued")
    }

    // Revoke the pass — terminal, issuer-only. A revoked pass is invalid
    // regardless of remaining strength.
    fn revoke() {
        require(caller == owner, "only issuer can revoke")
        require(self.revoked == false, "pass already revoked")
        self.revoked = true
        emit("pass revoked")
    }

    // Valid iff issued, not revoked, and the contract's live (decayed)
    // energy is still at or above the floor.
    fn is_valid() -> bool {
        return (self.sealed == true) && (self.revoked == false) && (energy >= self.validity_floor)
    }

    // Exercise the pass — reverts unless the caller is the holder and the
    // pass is currently valid. Gate dApp actions behind this.
    fn require_valid() -> bool {
        require(caller == self.holder, "not the holder")
        require(self.sealed == true, "pass not issued")
        require(self.revoked == false, "pass revoked")
        require(energy >= self.validity_floor, "pass expired or below floor")
        return true
    }

    // Lifecycle hooks — the runtime fires these as the contract's energy
    // crosses active -> grace -> ghost, so a dApp can react to the pass
    // fading or being topped up.
    on_grace() { emit("pass weakening") }
    on_evaporate() { emit("pass expired") }
    on_refresh() { emit("pass refreshed") }
}
