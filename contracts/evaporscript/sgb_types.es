// SinghGirardTypes (SGB) — §A5.1 Linear-Logic Type Discipline.
//
// Doctrine: INVENTION_STACK.md §A5.1.
//
// Formalism: Girard 1987 linear logic. `!T` admits duplication
// (decay-immune); `?T` admits silent loss (decay-eligible); de Morgan
// duals. Light Linear Logic (Girard 1998) and Soft Linear Logic
// (Lafont 2004) give polytime fragments.
//
// Decay synergy (structural): λ-decay *is* the categorical weakening
// rule applied automatically. Existing affine/linear chains (Move, Sui)
// only use `!`'s shadow; `?` has never been exposed as a contract type
// because no chain had a principled "decay" primitive to pair with it.
//
// Three type codes (stored as u64):
//   1 = Lin      — pure linear: MUST be used exactly once.
//                  Over-use or drop = discipline violation.
//   2 = Bang     — `!T`: decay-immune, may be duplicated ANY number
//                  of times. No upper bound on uses.
//   3 = Whimper  — `?T`: decay-ELIGIBLE, may be silently dropped.
//                  Use count: 0 or 1 (cannot be duplicated = no contraction).
//
// Variable lifecycle:
//   declare_var(type_id)        — register a new variable, auto-assigned slot.
//   use_var(slot)               — record one use of the variable.
//   check_discipline()          — verify linear invariants across all vars.
//                                 Writes: check_violations (0 = all OK).
//   witness_var(slot)           — snapshot type + uses for slot.
//   require_sound_discipline()  — gate: check_violations == 0.
//   require_violation_present() — gate: check_violations > 0 (adversarial demo).
//
// Discipline rules:
//   Lin (1):      uses must equal 1 → violated if uses < 1 OR uses > 1.
//   Bang (2):     any uses ≥ 0 → never violated.
//   Whimper (3):  uses must be 0 or 1 → violated if uses > 1.
//
// Deploy-script demo:
//   sound mode:    declare Lin, Bang, Whimper → use Lin once, Bang 3×,
//                  Whimper once → check → violations=0 → gate PASSED.
//   violated mode: declare Lin, Whimper → use Whimper twice (violation)
//                  → check → violations=2 (Lin dropped + Whimper dup)
//                  → require_violation_present PASSED.
//
// Press claim: "first system where ephemerality is a type, not a
// runtime convention. Type theorists have been waiting 30 years for
// industrial use of `?`."
//
// INVENTION_STACK.md §A5.1: SGB (Singh-Girard Bang-Whimper Types).

contract SinghGirardTypes {
    state {
        // Per-variable storage (slot = auto-assigned u64 on declare_var).
        var_type: map[u64 -> u64]   // slot → type code (1=Lin, 2=Bang, 3=Whimper)
        var_uses: map[u64 -> u64]   // slot → use count
        var_present: map[u64 -> u64] // slot → 0=absent, 1=registered
        var_count: u64 = 0

        // Aggregate type counters (convenience; updated at declare_var time).
        total_lin: u64 = 0
        total_bang: u64 = 0
        total_whimper: u64 = 0

        // Discipline check result (written by check_discipline()).
        check_ran: u64 = 0
        check_violations: u64 = 0

        // Witness snapshots (written by witness_var()).
        witness_count: u64 = 0
        snapshot1_slot: u64 = 0
        snapshot1_type: u64 = 0
        snapshot1_uses: u64 = 0
        snapshot2_slot: u64 = 0
        snapshot2_type: u64 = 0
        snapshot2_uses: u64 = 0
    }

    // Declare a new variable. Auto-assigns slot = var_count.
    // type_id: 1=Lin, 2=Bang, 3=Whimper.
    fn declare_var(type_id: u64) {
        require(type_id >= 1, "type_id must be 1 (Lin), 2 (Bang), or 3 (Whimper)")
        require(type_id <= 3, "type_id must be 1 (Lin), 2 (Bang), or 3 (Whimper)")
        let slot = self.var_count
        self.var_type[slot] = type_id
        self.var_uses[slot] = 0
        self.var_present[slot] = 1
        if type_id == 1 {
            self.total_lin += 1
        }
        if type_id == 2 {
            self.total_bang += 1
        }
        if type_id == 3 {
            self.total_whimper += 1
        }
        self.var_count += 1
        emit("sgb.var.declared")
    }

    // Record one use of variable at `slot`. Anyone can call (simulates
    // a computation step that consumes/reads the variable once).
    fn use_var(slot: u64) {
        require(slot < self.var_count, "slot out of range — variable not declared")
        require(self.var_present[slot] == 1, "variable not registered at this slot")
        self.var_uses[slot] = self.var_uses[slot] + 1
        emit("sgb.var.used")
    }

    // Verify linear discipline across all declared variables.
    // Iterates all slots; counts violations:
    //   Lin (1):     violated if uses != 1 (must be exactly once).
    //   Whimper (3): violated if uses > 1 (must be 0 or 1).
    //   Bang (2):    never violated (any number of uses allowed).
    // Writes check_violations and sets check_ran = 1.
    fn check_discipline() {
        let i = 0
        let violations = 0
        while i < self.var_count {
            if self.var_present[i] == 1 {
                let t = self.var_type[i]
                let u = self.var_uses[i]
                if t == 1 {
                    if u < 1 {
                        violations = violations + 1
                    }
                    if u > 1 {
                        violations = violations + 1
                    }
                }
                if t == 3 {
                    if u > 1 {
                        violations = violations + 1
                    }
                }
            }
            i = i + 1
        }
        self.check_violations = violations
        self.check_ran = 1
        emit("sgb.discipline.checked")
    }

    // Witness: snapshot type + uses for `slot` at call time.
    // First call → snapshot1, second call → snapshot2.
    fn witness_var(slot: u64) {
        require(slot < self.var_count, "slot out of range")
        require(self.var_present[slot] == 1, "variable not registered")
        let t = self.var_type[slot]
        let u = self.var_uses[slot]
        if self.witness_count == 0 {
            self.snapshot1_slot = slot
            self.snapshot1_type = t
            self.snapshot1_uses = u
        }
        if self.witness_count == 1 {
            self.snapshot2_slot = slot
            self.snapshot2_type = t
            self.snapshot2_uses = u
        }
        self.witness_count += 1
        emit("sgb.var.witnessed")
    }

    // Proof gate: linear discipline is satisfied across all variables.
    // Passes iff check_discipline() has been run and found zero violations.
    fn require_sound_discipline() {
        require(self.check_ran == 1, "check_discipline not yet called")
        require(self.check_violations == 0, "discipline violated — Lin variable not used exactly once, or Whimper used more than once")
        emit("sgb.sound_discipline.confirmed")
    }

    // Adversarial proof gate: at least one discipline violation found.
    // Proves the type checker correctly detects violations.
    fn require_violation_present() {
        require(self.check_ran == 1, "check_discipline not yet called")
        require(self.check_violations > 0, "no violations found — all types are satisfied")
        emit("sgb.violation_present.confirmed")
    }

    on_grace() {
        emit("sgb.energy.low — type context evaporating")
    }

    on_refresh() {
        emit("sgb.refreshed")
    }

    on_evaporate() {
        emit("sgb.evaporated — type context gone")
    }
}
