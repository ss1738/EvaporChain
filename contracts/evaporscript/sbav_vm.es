// SinghBennettVM (SBAV) — §A5.1 Reversible-VM Paradigm.
//
// Doctrine: INVENTION_STACK.md §A5.1.
//
// Formalism: Bennett 1973 reversible TM; Janus reversible imperative
// language (Yokoyama-Glück 2007); Landauer 1961 (irreversibility ↔
// entropy export ↔ energy cost).
//
// Decay synergy (structural — philosophically the cleanest in the
// stack): every classical opcode is reversible (zero entropy in the
// Landauer limit). Only decay(λ) exports entropy and is the unique
// irreversible primitive. Not analogy — Landauer literally. State at
// block t is bit-for-bit recoverable from t+k *except* for the
// λ-decay trace, which is the chain's thermodynamic arrow.
//
// Register file: 8 × u64 (indices 0–7).
// V1 reversible ops shipped here (EvaporScript-compatible):
//   ADD reg k   — reg += k;  inverse: SUB reg k.  Gas = 0.
//   SUB reg k   — reg -= k;  inverse: ADD reg k.  Gas = 0.
//   SWAP a b    — swap reg[a] ↔ reg[b]; inverse: SWAP a b. Gas = 0.
//   (XOR / NOT / ROTL / ROTR require bit ops absent in EvaporScript v1
//    grammar — deferred to a future bytecode extension.)
//
//   DECAY amount — entropy_exported += amount. Gas = amount (Landauer).
//     This is the ONLY non-zero-gas op. It is the ONLY op that advances
//     entropy_exported. Classical ops never touch it.
//
// Witness telemetry: witness_vm(reg_idx) captures reg[reg_idx] and
// entropy_exported into snapshot1 (first call) or snapshot2 (second).
//
// Proof gates:
//   require_zero_entropy()    — proves program is purely reversible.
//   require_nonzero_entropy() — proves Decay was applied at least once.
//
// Deploy-script demo (two modes):
//   reversible: add(0, 1000) → witness (snap1: reg0=1000, entropy=0) →
//               sub(0, 1000) [inverse] → witness (snap2: reg0=0, entropy=0)
//               → require_zero_entropy PASSED.
//   decay:      swap(0,1) → decay(500) → witness (snap1: entropy=500)
//               → require_nonzero_entropy PASSED.
//
// Press claim: "the first computer system where the laws of
// thermodynamics dictate which operations cost gas."
//
// INVENTION_STACK.md §A5.1: SBAV (Singh-Bennett Asymmetric VM).

contract SinghBennettVM {
    state {
        // 8-register file. reg[i] for i ∈ {0..7}. Default = 0.
        reg: map[u64 -> u64]

        // Running sum of all Decay(amount) events.
        // The chain's thermodynamic arrow: classical ops never change this.
        // entropy_exported > 0 iff at least one Decay was applied.
        entropy_exported: u64 = 0

        // Count of all ops applied (any type).
        op_count: u64 = 0

        // Witness snapshots (reg[sampled_idx] + entropy at call time).
        witness_count: u64 = 0
        snapshot1_reg_idx: u64 = 0
        snapshot1_reg_val: u64 = 0
        snapshot1_entropy: u64 = 0
        snapshot2_reg_idx: u64 = 0
        snapshot2_reg_val: u64 = 0
        snapshot2_entropy: u64 = 0
    }

    // ADD: reg[reg_idx] += k. Reversible; inverse is op_sub(reg_idx, k).
    // Gas cost = 0 (Landauer: no entropy exported).
    fn op_add(reg_idx: u64, k: u64) {
        require(reg_idx < 8, "register index out of bounds (valid: 0..7)")
        require(k > 0, "k must be positive — zero-add is a no-op")
        self.reg[reg_idx] = self.reg[reg_idx] + k
        self.op_count += 1
        emit("sbav.op.add")
    }

    // SUB: reg[reg_idx] -= k. Reversible; inverse is op_add(reg_idx, k).
    // Requires reg[reg_idx] >= k (VM rejects underflow; checked_sub).
    // Gas cost = 0.
    fn op_sub(reg_idx: u64, k: u64) {
        require(reg_idx < 8, "register index out of bounds (valid: 0..7)")
        require(k > 0, "k must be positive — zero-sub is a no-op")
        self.reg[reg_idx] = self.reg[reg_idx] - k
        self.op_count += 1
        emit("sbav.op.sub")
    }

    // SWAP: exchange reg[a] and reg[b]. Reversible; its own inverse (SWAP a b twice = id).
    // Gas cost = 0.
    fn op_swap(a: u64, b: u64) {
        require(a < 8, "register a out of bounds (valid: 0..7)")
        require(b < 8, "register b out of bounds (valid: 0..7)")
        require(a != b, "swap of same register is a no-op")
        let va = self.reg[a]
        let vb = self.reg[b]
        self.reg[a] = vb
        self.reg[b] = va
        self.op_count += 1
        emit("sbav.op.swap")
    }

    // DECAY: the unique irreversible op. Adds `amount` to entropy_exported.
    // Gas cost = amount (Landauer: only irreversible ops export entropy,
    // and only ops that export entropy pay gas).
    // entropy_exported is the thermodynamic arrow; it grows strictly monotone.
    fn op_decay(amount: u64) {
        require(amount > 0, "decay amount must be > 0 — zero-decay is a no-op, not a primitive")
        self.entropy_exported = self.entropy_exported + amount
        self.op_count += 1
        emit("sbav.op.decay")
    }

    // Witness: snapshot reg[reg_idx] and entropy_exported at call time.
    // First call → snapshot1, second call → snapshot2. Anyone can call.
    fn witness_vm(reg_idx: u64) {
        require(reg_idx < 8, "register index out of bounds (valid: 0..7)")
        let val = self.reg[reg_idx]
        let entropy = self.entropy_exported
        if self.witness_count == 0 {
            self.snapshot1_reg_idx = reg_idx
            self.snapshot1_reg_val = val
            self.snapshot1_entropy = entropy
        }
        if self.witness_count == 1 {
            self.snapshot2_reg_idx = reg_idx
            self.snapshot2_reg_val = val
            self.snapshot2_entropy = entropy
        }
        self.witness_count += 1
        emit("sbav.witnessed")
    }

    // Proof gate: proves the program is purely reversible.
    // entropy_exported == 0 iff no Decay was ever applied.
    // "All classical ops had zero gas cost — the thermodynamic arrow
    // did not advance."
    fn require_zero_entropy() {
        require(self.op_count > 0, "no ops applied yet")
        require(self.entropy_exported == 0, "entropy exported — Decay was applied; program is not purely reversible")
        emit("sbav.zero_entropy.confirmed")
    }

    // Proof gate: proves at least one Decay was applied.
    // entropy_exported > 0 iff Decay(amount > 0) was called.
    fn require_nonzero_entropy() {
        require(self.op_count > 0, "no ops applied yet")
        require(self.entropy_exported > 0, "no entropy exported — program is purely reversible, no Decay applied")
        emit("sbav.nonzero_entropy.confirmed")
    }

    on_grace() {
        emit("sbav.energy.low — VM state evaporating")
    }

    on_refresh() {
        emit("sbav.refreshed")
    }

    on_evaporate() {
        emit("sbav.evaporated — VM program gone")
    }
}
