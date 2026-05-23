// DPNativeVM — §4.2 Tier-2 VM Paradigm: differential-privacy-native.
//
// Doctrine: INVENTION_STACK.md §4.2. Formal basis: Dwork–McSherry–Nissim–Smith
// 2006 (ε-differential privacy); Dwork–Roth 2014 (algorithmic foundations).
//
// Why DP-native, why monotone — structural argument:
//
//   Differential privacy is a mathematical budget: each query consumes ε
//   from a finite (ε_total, δ_total) envelope. Classical systems track
//   this budget off-chain (trust the analyst) or not at all. DP-Native
//   makes the budget a *first-class on-chain object* — unforgeable,
//   monotone (spend-only), and structurally irrevocable.
//
//   Monotone invariant: ds_consumed_eps and ds_consumed_delta are
//   write-once-in-one-direction. The contract has no decrement path.
//   The VM itself becomes the accountability layer: no query can proceed
//   without burning budget on-chain, and no budget can be reloaded
//   once the envelope is set. Re-registration is structurally forbidden —
//   the ds_present guard closes the "reset to refill" attack.
//
//   EvaporChain's energy-decay pairs naturally: the contract itself
//   evaporates when the node's energy expires, but the privacy budget
//   it guarded is monotone within that lifetime — two independent
//   irreversibility dimensions.
//
// Precision encoding:
//   epsilon in micros: 1 ε = 1,000,000 micros.
//   delta in parts-per-billion (ppb): 1×10⁻⁹ δ = 1 ppb.
//   Both stored as u64; never floating-point.
//
// Dataset lifecycle:
//   register_dataset(ds_id, eps_micros, delta_ppb) — create envelope.
//     Re-registration of same ds_id is a hard revert.
//   consume_budget(ds_id, eps_q, delta_q) — spend from envelope.
//     Reverts if either axis would overflow the envelope.
//   witness_budget(ds_id) — snapshot (consumed_eps, total_eps) into
//     snapshot1 (first call) or snapshot2 (second call).
//   require_exhausted(ds_id) — gate: consumed_eps ≥ total_eps.
//   require_budget_remaining(ds_id) — gate: consumed_eps < total_eps
//     (adversarial-demo companion to require_exhausted).
//
// Deploy-script demo:
//   exhaust mode:  register(eps=1000) → consume 300+400+300 →
//     witness → require_exhausted PASSED.
//   monotone mode: register(eps=1000) → consume 400 → witness snap1
//     (consumed=400) → consume 300 → witness snap2 (consumed=700) →
//     require_budget_remaining PASSED. Shows monotone increase.
//
// Press claim: "privacy budget as an on-chain monotone type — no
// analyst can exceed their ε envelope; chain enforces it structurally."
//
// INVENTION_STACK.md §4.2: DPNativeVM.

contract DPNativeVM {
    state {
        // Per-dataset privacy budget storage. ds_id = caller-provided u64 handle.
        ds_eps_budget: map[u64 -> u64]       // ds_id → total epsilon budget (micros; 1ε = 1,000,000)
        ds_delta_budget: map[u64 -> u64]     // ds_id → total delta budget (ppb)
        ds_consumed_eps: map[u64 -> u64]     // ds_id → epsilon consumed so far (micros; monotone-only)
        ds_consumed_delta: map[u64 -> u64]   // ds_id → delta consumed so far (ppb; monotone-only)
        ds_present: map[u64 -> u64]          // ds_id → 0=absent, 1=registered
        ds_count: u64 = 0
        query_count: u64 = 0

        // Witness snapshots (consumed_eps + total_eps at call time).
        witness_count: u64 = 0
        snapshot1_ds_id: u64 = 0
        snapshot1_consumed_eps: u64 = 0
        snapshot1_total_eps: u64 = 0
        snapshot2_ds_id: u64 = 0
        snapshot2_consumed_eps: u64 = 0
        snapshot2_total_eps: u64 = 0
    }

    // Register a dataset with a fixed privacy envelope. Owner-only.
    // ds_id: caller-chosen u64 handle (arbitrary; must be unique).
    // eps_micros: total epsilon budget in micros (1ε = 1,000,000 micros).
    // delta_ppb: total delta budget in ppb (1e-9 = 1 ppb).
    // Re-registration of an existing ds_id is structurally forbidden.
    fn register_dataset(ds_id: u64, eps_micros: u64, delta_ppb: u64) {
        require(caller == owner, "only owner can register datasets")
        require(self.ds_present[ds_id] == 0, "dataset already registered — re-registration forbidden: budget is spend-only monotone")
        require(eps_micros > 0, "epsilon budget must be positive")
        require(delta_ppb > 0, "delta budget must be positive")
        self.ds_eps_budget[ds_id] = eps_micros
        self.ds_delta_budget[ds_id] = delta_ppb
        self.ds_consumed_eps[ds_id] = 0
        self.ds_consumed_delta[ds_id] = 0
        self.ds_present[ds_id] = 1
        self.ds_count += 1
        emit("dpnative.dataset.registered")
    }

    // Consume privacy budget for one query against ds_id.
    // eps_q, delta_q: epsilon and delta cost of this query (must be positive).
    // Reverts if either axis would exceed the registered envelope.
    // Monotone guarantee: ds_consumed_eps and ds_consumed_delta only increase.
    fn consume_budget(ds_id: u64, eps_q: u64, delta_q: u64) {
        require(self.ds_present[ds_id] == 1, "dataset not registered")
        require(eps_q > 0, "epsilon consumption must be positive")
        require(delta_q > 0, "delta consumption must be positive")
        let new_eps = self.ds_consumed_eps[ds_id] + eps_q
        let new_delta = self.ds_consumed_delta[ds_id] + delta_q
        require(new_eps <= self.ds_eps_budget[ds_id], "epsilon budget exhausted — query would violate privacy bound")
        require(new_delta <= self.ds_delta_budget[ds_id], "delta budget exhausted — query would violate privacy bound")
        self.ds_consumed_eps[ds_id] = new_eps
        self.ds_consumed_delta[ds_id] = new_delta
        self.query_count += 1
        emit("dpnative.budget.consumed")
    }

    // Witness: snapshot (consumed_eps, total_eps) for ds_id at call time.
    // First call → snapshot1, second call → snapshot2.
    // Pair with two calls to prove monotone increase over time.
    fn witness_budget(ds_id: u64) {
        require(self.ds_present[ds_id] == 1, "dataset not registered")
        let c = self.ds_consumed_eps[ds_id]
        let t = self.ds_eps_budget[ds_id]
        if self.witness_count == 0 {
            self.snapshot1_ds_id = ds_id
            self.snapshot1_consumed_eps = c
            self.snapshot1_total_eps = t
        }
        if self.witness_count == 1 {
            self.snapshot2_ds_id = ds_id
            self.snapshot2_consumed_eps = c
            self.snapshot2_total_eps = t
        }
        self.witness_count += 1
        emit("dpnative.budget.witnessed")
    }

    // Proof gate: the full epsilon envelope has been consumed.
    // Passes iff consumed_eps ≥ total_eps (budget at or past limit).
    fn require_exhausted(ds_id: u64) {
        require(self.ds_present[ds_id] == 1, "dataset not registered")
        require(self.ds_consumed_eps[ds_id] >= self.ds_eps_budget[ds_id], "budget not exhausted — epsilon still remaining")
        emit("dpnative.exhausted.confirmed")
    }

    // Proof gate: epsilon budget is still partially available.
    // Passes iff consumed_eps < total_eps. Adversarial companion to
    // require_exhausted — proves mid-sequence monotone state is readable.
    fn require_budget_remaining(ds_id: u64) {
        require(self.ds_present[ds_id] == 1, "dataset not registered")
        require(self.ds_consumed_eps[ds_id] < self.ds_eps_budget[ds_id], "budget fully exhausted — no epsilon remaining")
        emit("dpnative.budget_remaining.confirmed")
    }

    on_grace() {
        emit("dpnative.energy.low — privacy budget registry evaporating")
    }

    on_refresh() {
        emit("dpnative.refreshed")
    }

    on_evaporate() {
        emit("dpnative.evaporated — privacy budget registry gone")
    }
}
