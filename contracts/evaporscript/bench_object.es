// bench_object.es — minimal decaying state unit for the state-decay
// benchmark (research/BENCH_STATE_DECAY.md).
//
// One deploy = one decaying on-chain object. There is deliberately NO
// business logic: its only job is to occupy state and then either
// evaporate (small energy/half_life) or persist (astronomical
// half_life). The benchmark deploys many of these under two regimes
// and measures whether the aggregate active-object set / data-dir
// size stays bounded by construction (decay) or grows monotonically
// (no-decay control). Grammar mirrors the verified gdpr_vault.es.
contract BenchObject {
    state {
        marker: u64 = 1
        born_at: u64 = 0
    }

    // Optional no-op touch (not required by the benchmark — a bare
    // deploy already creates the decaying object). Present only so the
    // contract has a callable surface consistent with the other .es.
    fn touch() {
        self.born_at = epoch
        emit("bench object touched")
    }
}
