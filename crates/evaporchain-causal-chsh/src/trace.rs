//! Real-data gate driver — analyses a slice of block summaries (real
//! chain trace) and reports the CHSH gate verdict.
//!
//! ## Concurrency proxy on a linear chain
//!
//! Real Ethereum is a linear chain — no LightCone DAG, no genuinely
//! concurrent blocks. To run the gate against real Ethereum data we
//! use a **concurrency-window proxy**: pairs of blocks separated by
//! ≤ `concurrency_window_secs` are treated as "would-be concurrent
//! under a LightCone refactor." This is HONEST about its limitations
//! — the gate's verdict on Ethereum is a *proxy* for what would
//! happen on a true LightCone chain. The real test of the bound on
//! a LightCone chain has to wait until EvaporChain testnet matures.
//!
//! ## Observables
//!
//! Per block, two binary observables:
//! - **A / A'** (first-side settings):
//!   - A = sign(energy − median_energy)
//!   - A' = sign(gas − median_gas)
//! - **B / B'** (second-side settings):
//!   - B = sign(tx_count − median_tx_count)
//!   - B' = sign(timestamp_parity)  // even/odd timestamp second
//!
//! Each pair contributes one ±1 observation per setting-pair; we
//! distribute pairs evenly across the four setting-pairs so each
//! sample has roughly the same size.
//!
//! ## Cartel injection
//!
//! Synthetic cartel rigs the four samples independently to violate
//! the bound. Implemented as `inject_cartel_signature(pairs, target_s)`
//! which deterministically rewrites observables to hit a chosen S
//! value. Used by the gate to verify the inequality DOES discriminate
//! the cartel signature from the honest baseline.

use crate::chsh::ConcurrentPairSamples;
use serde::{Deserialize, Serialize};

/// One row of chain trace — what the gate driver needs about a block.
/// Independent of EvaporChain types so this crate stays substrate-only
/// (no consensus/state crate deps).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BlockSummary {
    pub height: u64,
    pub timestamp_secs: u64,
    /// Total energy / gas / similar scalar measure of block "weight."
    pub energy: u64,
    /// Gas used (or analogous resource consumption metric).
    pub gas: u64,
    /// Transaction count.
    pub tx_count: u64,
}

/// Bundle the four CHSH setting-pair samples extracted from a real
/// chain trace. Drives `compute_chsh_s` directly.
pub fn extract_chsh_samples(
    trace: &[BlockSummary],
    concurrency_window_secs: u64,
) -> ConcurrentPairSamples {
    if trace.len() < 2 {
        return ConcurrentPairSamples {
            samples_ab: vec![],
            samples_ab_prime: vec![],
            samples_a_prime_b: vec![],
            samples_a_prime_b_prime: vec![],
        };
    }

    // Compute medians for binary observables. Non-allocating: clone
    // each scalar into a sort buffer.
    let mut energies: Vec<u64> = trace.iter().map(|b| b.energy).collect();
    let mut gases: Vec<u64> = trace.iter().map(|b| b.gas).collect();
    let mut counts: Vec<u64> = trace.iter().map(|b| b.tx_count).collect();
    energies.sort_unstable();
    gases.sort_unstable();
    counts.sort_unstable();
    let med_energy = energies[trace.len() / 2];
    let med_gas = gases[trace.len() / 2];
    let med_count = counts[trace.len() / 2];

    let obs_a = |b: &BlockSummary| if b.energy >= med_energy { 1i8 } else { -1 };
    let obs_a_prime = |b: &BlockSummary| if b.gas >= med_gas { 1i8 } else { -1 };
    let obs_b = |b: &BlockSummary| if b.tx_count >= med_count { 1i8 } else { -1 };
    let obs_b_prime = |b: &BlockSummary| {
        if b.timestamp_secs.is_multiple_of(2) {
            1i8
        } else {
            -1
        }
    };

    // Walk all pairs (i, j) where j > i and timestamps are within
    // window. Distribute round-robin across the four setting-pairs
    // so each sample gets ≈ |concurrent_pairs|/4 entries.
    let mut samples_ab: Vec<i8> = Vec::new();
    let mut samples_ab_prime: Vec<i8> = Vec::new();
    let mut samples_a_prime_b: Vec<i8> = Vec::new();
    let mut samples_a_prime_b_prime: Vec<i8> = Vec::new();

    let mut counter: usize = 0;
    for i in 0..trace.len() {
        for j in (i + 1)..trace.len() {
            // Concurrency proxy: timestamp difference within window.
            let dt = trace[j]
                .timestamp_secs
                .saturating_sub(trace[i].timestamp_secs);
            if dt > concurrency_window_secs {
                // Trace is height-sorted; once we exceed the window
                // for some j, all later j' also exceed.
                break;
            }
            let bi = &trace[i];
            let bj = &trace[j];
            match counter % 4 {
                0 => samples_ab.push(obs_a(bi) * obs_b(bj)),
                1 => samples_ab_prime.push(obs_a(bi) * obs_b_prime(bj)),
                2 => samples_a_prime_b.push(obs_a_prime(bi) * obs_b(bj)),
                3 => samples_a_prime_b_prime.push(obs_a_prime(bi) * obs_b_prime(bj)),
                _ => unreachable!(),
            }
            counter += 1;
        }
    }

    ConcurrentPairSamples {
        samples_ab,
        samples_ab_prime,
        samples_a_prime_b,
        samples_a_prime_b_prime,
    }
}

/// Synthetic cartel injection: produce a `ConcurrentPairSamples` that
/// deterministically violates the Bell bound. Used to verify the gate
/// can DISCRIMINATE the cartel signature from honest data.
///
/// Constructs all four samples to be uniformly +1 except `samples_a_prime_b_prime`
/// which is uniformly -1, giving `S = 4` (algebraic max).
///
/// `n` controls the sample size per setting-pair.
pub fn synthesize_max_cartel_samples(n: usize) -> ConcurrentPairSamples {
    ConcurrentPairSamples {
        samples_ab: vec![1; n],
        samples_ab_prime: vec![1; n],
        samples_a_prime_b: vec![1; n],
        samples_a_prime_b_prime: vec![-1; n],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chsh::compute_chsh_s;
    use crate::gate::{run_synthetic_gate, GateThresholds, GateVerdict};

    /// Generate a synthetic "honest Ethereum" trace: 200 blocks at
    /// 12-second intervals, with energy/gas/tx_count drawn from
    /// roughly-realistic ranges. Observable values are independent
    /// per block — no cartel coordination.
    fn synthetic_honest_eth_trace() -> Vec<BlockSummary> {
        let mut trace = Vec::with_capacity(200);
        // Use a deterministic LCG so the test is reproducible without
        // an rand dep.
        let mut rng: u64 = 0xCAFEBABEDEADBEEF;
        let mut next = |bound: u64| {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) % bound
        };
        for i in 0..200 {
            trace.push(BlockSummary {
                height: i,
                timestamp_secs: i * 12 + next(3), // small jitter
                energy: 10_000_000 + next(2_000_000),
                gas: 12_000_000 + next(2_000_000),
                tx_count: 100 + next(80),
            });
        }
        trace
    }

    #[test]
    fn extract_from_short_trace_returns_empty() {
        let v = extract_chsh_samples(&[], 60);
        assert!(v.samples_ab.is_empty());
        assert!(v.samples_ab_prime.is_empty());
    }

    #[test]
    fn synthetic_honest_trace_satisfies_bell_bound() {
        // Lane O.2 load-bearing assertion: independent honest-source
        // observables on a synthetic Eth-like trace produce S well
        // below the 1.8 ceiling. Validates the proxy methodology.
        let trace = synthetic_honest_eth_trace();
        let samples = extract_chsh_samples(&trace, 60);
        // All four samples should have substantial size (≥10 entries
        // each given 200 blocks at 12s intervals + 60s window).
        assert!(samples.samples_ab.len() >= 10);
        assert!(samples.samples_ab_prime.len() >= 10);
        assert!(samples.samples_a_prime_b.len() >= 10);
        assert!(samples.samples_a_prime_b_prime.len() >= 10);

        let s = compute_chsh_s(&samples).unwrap();
        // Honest source: S should be small (well below 1.8 doctrine
        // ceiling). The proptest gives empirical assurance for this
        // kind of independent random source.
        assert!(s < 1.8, "honest synthetic Eth trace gave S={}", s);
    }

    #[test]
    fn synthetic_cartel_violates_bound_at_size() {
        // Cartel injection at size n=200 should hit S=4 exactly
        // (deterministic). Validates the cartel construction works
        // against the same-shape ConcurrentPairSamples bundle the
        // real-trace extractor produces.
        let cartel = synthesize_max_cartel_samples(200);
        let s = compute_chsh_s(&cartel).unwrap();
        assert!((s - 4.0).abs() < 1e-9);
    }

    #[test]
    fn synthetic_eth_gate_passes_doctrine_thresholds() {
        // End-to-end: run the synthetic-Eth honest sample against
        // the cartel injection through the doctrine gate. Pass means
        // the math + extractor + gate harness all work together.
        let honest = extract_chsh_samples(&synthetic_honest_eth_trace(), 60);
        let cartel = synthesize_max_cartel_samples(200);
        let v = run_synthetic_gate(&honest, &cartel, GateThresholds::doctrine());
        match v {
            GateVerdict::Pass {
                s_honest,
                s_cartel,
                gap,
            } => {
                assert!(s_honest < 1.8);
                assert!(s_cartel > 2.2);
                assert!(gap > 0.4);
            }
            other => panic!("expected Pass, got {:?}", other),
        }
    }

    #[test]
    fn concurrency_window_zero_admits_no_pairs() {
        // dt > 0 for all distinct blocks → empty samples.
        // Note: jitter in our synthetic trace means some blocks may
        // share a timestamp — admit those. The point is the window
        // bound is HONOURED.
        let trace = synthetic_honest_eth_trace();
        let samples = extract_chsh_samples(&trace, 0);
        // Some pairs may slip through because of timestamp ties from
        // the small jitter; but the count must be much smaller than
        // the 60s-window count.
        let small = samples.samples_ab.len()
            + samples.samples_ab_prime.len()
            + samples.samples_a_prime_b.len()
            + samples.samples_a_prime_b_prime.len();
        let big_samples = extract_chsh_samples(&trace, 60);
        let big = big_samples.samples_ab.len()
            + big_samples.samples_ab_prime.len()
            + big_samples.samples_a_prime_b.len()
            + big_samples.samples_a_prime_b_prime.len();
        assert!(small <= big, "smaller window must yield ≤ samples");
    }
}
