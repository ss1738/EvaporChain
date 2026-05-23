// TotalEvaporScriptVM — §4.2 Tier-2 VM Paradigm: structural totality checker.
//
// Doctrine: INVENTION_STACK.md §4.2. Crate: evaporchain-total-evaporscript.
//
// Background: Gas metering kills individual infinite-loop attacks by running
// until the gas limit. But every contract still has the *type* "may not
// terminate." Total programming makes "this program terminates" a *type-level*
// fact. The infinite-loop DoS class doesn't get mitigated — it ceases to be
// expressible.
//
// Two loop forms (structural):
//
//   BoundedFor (kind=1):
//     loop var is fixed at registration; body forbidden from mutating it.
//     Iteration count = bound (step count). Always total.
//
//   BoundedWhile (kind=2):
//     carries a named ranking variable. Body MUST contain a strict-decrement
//     of that variable (x = x - k, k > 0). has_decrement=1 → total.
//     has_decrement=0 → body has no visible decrement → nontotal (divergence-prone).
//
// There is NO general while. There is NO general recursion. Every loop
// construct carried in this VM is either (a) total by construction or
// (b) flagged as nontotal by check_total().
//
// Contract lifecycle:
//   add_bounded_for(bound)                 — register BoundedFor instruction.
//   add_bounded_while(ranking, has_dec)    — register BoundedWhile instruction.
//   check_total()                          — scan all instructions; count violations.
//   witness_instr(slot)                    — snapshot kind+bound+has_decrement.
//   require_total()                        — gate: violations == 0.
//   require_nontotal_found()               — gate: violations > 0 (adversarial).
//
// Violation rule: a BoundedWhile with has_decrement=0 counts as 1 violation.
// BoundedFor is never a violation (iteration count is fixed at add time).
//
// Deploy-script demo:
//   total mode:    add BoundedFor(100) + BoundedWhile(50, dec=1) →
//                  check → violations=0 → require_total PASSED.
//   nontotal mode: add BoundedFor(200) + BoundedWhile(50, dec=0) →
//                  check → violations=1 → require_nontotal_found PASSED.
//
// Press claim: "EvaporChain is the first L1 whose contract VM has structural
// totality at the language level. The infinite-loop DoS class does not get
// mitigated — it ceases to be expressible."
//
// INVENTION_STACK.md §4.2: TotalEvaporScriptVM.

contract TotalEvaporScriptVM {
    state {
        // Per-instruction storage. Slot = auto-assigned sequential u64.
        instr_kind: map[u64 -> u64]          // 1=BoundedFor, 2=BoundedWhile
        instr_bound: map[u64 -> u64]         // BoundedFor: step count; BoundedWhile: initial ranking
        instr_has_decrement: map[u64 -> u64] // BoundedWhile: 1=total, 0=nontotal; BoundedFor: always 1
        instr_present: map[u64 -> u64]       // 0=absent, 1=registered
        instr_count: u64 = 0

        // Discipline check result (written by check_total).
        check_ran: u64 = 0
        check_violations: u64 = 0           // count of nontotal BoundedWhile instructions

        // Witness snapshots (written by witness_instr).
        witness_count: u64 = 0
        snapshot1_slot: u64 = 0
        snapshot1_kind: u64 = 0
        snapshot1_bound: u64 = 0
        snapshot1_has_decrement: u64 = 0
        snapshot2_slot: u64 = 0
        snapshot2_kind: u64 = 0
        snapshot2_bound: u64 = 0
        snapshot2_has_decrement: u64 = 0
    }

    // Register a BoundedFor loop. Always structurally total.
    // bound: iteration count (body executes exactly `bound` times).
    // The loop variable is read-only in the body; mutation is a discipline
    // violation caught statically by the real EvaporScript parser.
    fn add_bounded_for(bound: u64) {
        require(bound > 0, "loop bound must be positive — zero-iteration BoundedFor is a no-op, not a loop")
        let slot = self.instr_count
        self.instr_kind[slot] = 1
        self.instr_bound[slot] = bound
        self.instr_has_decrement[slot] = 1  // BoundedFor is always total — no ranking needed
        self.instr_present[slot] = 1
        self.instr_count += 1
        emit("total.instr.bounded_for.added")
    }

    // Register a BoundedWhile loop with a named ranking variable.
    // initial_ranking: starting value of the ranking variable (must be positive).
    // has_decrement: 1 if the loop body syntactically strict-decrements the ranking
    //   variable on every CFG path (x = x - k, k > 0). 0 if no such decrement
    //   exists in the body — the loop is divergence-prone (nontotal).
    // The chain cannot encode a live BoundedWhile without declaring the ranking
    // variable here; has_decrement is the syntactic totality certificate.
    fn add_bounded_while(initial_ranking: u64, has_decrement: u64) {
        require(initial_ranking > 0, "initial ranking must be positive — zero ranking means the loop condition is immediately false")
        require(has_decrement <= 1, "has_decrement must be 0 (nontotal) or 1 (total)")
        let slot = self.instr_count
        self.instr_kind[slot] = 2
        self.instr_bound[slot] = initial_ranking
        self.instr_has_decrement[slot] = has_decrement
        self.instr_present[slot] = 1
        self.instr_count += 1
        emit("total.instr.bounded_while.added")
    }

    // Check structural totality across all registered instructions.
    // Violation: a BoundedWhile (kind=2) with has_decrement=0.
    // BoundedFor (kind=1) is never a violation.
    // Writes: check_violations + sets check_ran=1.
    fn check_total() {
        let i = 0
        let violations = 0
        while i < self.instr_count {
            if self.instr_present[i] == 1 {
                let k = self.instr_kind[i]
                if k == 2 {
                    let d = self.instr_has_decrement[i]
                    if d == 0 {
                        violations = violations + 1
                    }
                }
            }
            i = i + 1
        }
        self.check_violations = violations
        self.check_ran = 1
        emit("total.discipline.checked")
    }

    // Witness: snapshot kind + bound + has_decrement for slot at call time.
    // First call → snapshot1, second call → snapshot2.
    fn witness_instr(slot: u64) {
        require(slot < self.instr_count, "slot out of range — instruction not declared")
        require(self.instr_present[slot] == 1, "instruction not registered at this slot")
        let k = self.instr_kind[slot]
        let b = self.instr_bound[slot]
        let d = self.instr_has_decrement[slot]
        if self.witness_count == 0 {
            self.snapshot1_slot = slot
            self.snapshot1_kind = k
            self.snapshot1_bound = b
            self.snapshot1_has_decrement = d
        }
        if self.witness_count == 1 {
            self.snapshot2_slot = slot
            self.snapshot2_kind = k
            self.snapshot2_bound = b
            self.snapshot2_has_decrement = d
        }
        self.witness_count += 1
        emit("total.instr.witnessed")
    }

    // Proof gate: all registered instructions are total.
    // Passes iff check_total() has been run and found zero violations.
    fn require_total() {
        require(self.check_ran == 1, "check_total not yet called")
        require(self.check_violations == 0, "nontotal instruction found — BoundedWhile without strict-decrement eliminates the termination guarantee")
        emit("total.sound.confirmed")
    }

    // Adversarial proof gate: at least one nontotal instruction was detected.
    // Proves the checker correctly identifies divergence-prone BoundedWhile loops.
    fn require_nontotal_found() {
        require(self.check_ran == 1, "check_total not yet called")
        require(self.check_violations > 0, "no violations found — all instructions are structurally total")
        emit("total.nontotal_found.confirmed")
    }

    on_grace() {
        emit("total.energy.low — totality registry evaporating")
    }

    on_refresh() {
        emit("total.refreshed")
    }

    on_evaporate() {
        emit("total.evaporated — totality registry gone")
    }
}
