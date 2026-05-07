//! Real-data pipeline driver.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_causal_chsh::chsh::ConcurrentPairSamples;
use evaporchain_causal_chsh::gate::{run_synthetic_gate, GateThresholds, GateVerdict};

/// Per-block stats extracted from the LightCone trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockStats {
    pub energy: u64,
    pub tx_count: u64,
}

/// One concurrent-block pair from the LightCone DAG, with both
/// endpoints' raw stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawConcurrentPair {
    /// Stats for the first concurrent block.
    pub first: BlockStats,
    /// Stats for the second concurrent block.
    pub second: BlockStats,
    /// 32-byte tag the chain assigns; used for deterministic bucketing.
    pub tag: [u8; 32],
}

/// Which CHSH setting-pair this concurrent pair will be measured under.
/// The runner deterministically buckets each `RawConcurrentPair` into
/// one of the four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingPair {
    Ab,
    AbPrime,
    APrimeB,
    APrimeBPrime,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RealDataGateError {
    #[error("trace empty — at least one concurrent pair required")]
    EmptyTrace,
    #[error("trace too small ({n}) — at least one pair per setting-pair (4) required")]
    InsufficientCoverage { n: usize },
    #[error("observable returned {0} on a real block — must be ±1")]
    NonBinaryObservable(i8),
}

/// Bucket a pair into one of four setting-pairs deterministically.
/// Uses two bits from the tag (last byte's low bits).
pub fn bucket_pair(tag: &[u8; 32]) -> SettingPair {
    match tag[31] & 0b11 {
        0 => SettingPair::Ab,
        1 => SettingPair::AbPrime,
        2 => SettingPair::APrimeB,
        _ => SettingPair::APrimeBPrime,
    }
}

/// An observable: maps a block's stats to ±1.
pub type Observable = fn(&BlockStats) -> i8;

/// Doctrine observable A / B: "energy above the trace's median energy"
/// → +1, otherwise -1. Note: the median is fixed per call to the
/// runner — passed in by the caller as part of the observable closure.
/// V1 ships a simple constructor that takes the median and returns the
/// observable function.
pub fn energy_above_median_observable(median_energy: u64) -> impl Fn(&BlockStats) -> i8 {
    move |b: &BlockStats| if b.energy >= median_energy { 1 } else { -1 }
}

/// Doctrine observable A' / B': "tx_count above the trace's median
/// tx-count" → +1, otherwise -1.
pub fn tx_count_above_median_observable(median_tx: u64) -> impl Fn(&BlockStats) -> i8 {
    move |b: &BlockStats| if b.tx_count >= median_tx { 1 } else { -1 }
}

/// Compute the median of a slice (returns the lower-median for even
/// counts; pure integer, validator-deterministic).
fn median(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted: Vec<u64> = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

/// Build the doctrine observables (A=A'=energy>median; B=B'=tx>median —
/// well, A=energy, A'=tx_count for the FIRST block; B=energy, B'=tx_count
/// for the SECOND block).
///
/// Returns `(obs_a, obs_a_prime, obs_b, obs_b_prime)` as boxed closures
/// since each closure captures a different median.
pub fn doctrine_observables(
    pairs: &[RawConcurrentPair],
) -> (
    Box<dyn Fn(&BlockStats) -> i8>,
    Box<dyn Fn(&BlockStats) -> i8>,
    Box<dyn Fn(&BlockStats) -> i8>,
    Box<dyn Fn(&BlockStats) -> i8>,
) {
    // Compute medians over the FULL trace so all four observables
    // share a stable threshold.
    let energies_first: Vec<u64> = pairs.iter().map(|p| p.first.energy).collect();
    let txs_first: Vec<u64> = pairs.iter().map(|p| p.first.tx_count).collect();
    let energies_second: Vec<u64> = pairs.iter().map(|p| p.second.energy).collect();
    let txs_second: Vec<u64> = pairs.iter().map(|p| p.second.tx_count).collect();

    let med_energy_first = median(&energies_first);
    let med_tx_first = median(&txs_first);
    let med_energy_second = median(&energies_second);
    let med_tx_second = median(&txs_second);

    let obs_a = energy_above_median_observable(med_energy_first);
    let obs_a_prime = tx_count_above_median_observable(med_tx_first);
    let obs_b = energy_above_median_observable(med_energy_second);
    let obs_b_prime = tx_count_above_median_observable(med_tx_second);

    (
        Box::new(obs_a),
        Box::new(obs_a_prime),
        Box::new(obs_b),
        Box::new(obs_b_prime),
    )
}

/// Build `ConcurrentPairSamples` from a trace + four observables.
///
/// Each pair is bucketed into its setting-pair via `bucket_pair`; the
/// product of the relevant left + right observable readings becomes
/// the ±1 sample value for that bucket.
pub fn build_samples(
    pairs: &[RawConcurrentPair],
    obs_a: &dyn Fn(&BlockStats) -> i8,
    obs_a_prime: &dyn Fn(&BlockStats) -> i8,
    obs_b: &dyn Fn(&BlockStats) -> i8,
    obs_b_prime: &dyn Fn(&BlockStats) -> i8,
) -> Result<ConcurrentPairSamples, RealDataGateError> {
    if pairs.is_empty() {
        return Err(RealDataGateError::EmptyTrace);
    }
    let mut samples_ab: Vec<i8> = Vec::new();
    let mut samples_ab_prime: Vec<i8> = Vec::new();
    let mut samples_a_prime_b: Vec<i8> = Vec::new();
    let mut samples_a_prime_b_prime: Vec<i8> = Vec::new();

    for p in pairs {
        let bucket = bucket_pair(&p.tag);
        let (left_obs, right_obs): (i8, i8) = match bucket {
            SettingPair::Ab => (obs_a(&p.first), obs_b(&p.second)),
            SettingPair::AbPrime => (obs_a(&p.first), obs_b_prime(&p.second)),
            SettingPair::APrimeB => (obs_a_prime(&p.first), obs_b(&p.second)),
            SettingPair::APrimeBPrime => (obs_a_prime(&p.first), obs_b_prime(&p.second)),
        };
        if !is_binary(left_obs) {
            return Err(RealDataGateError::NonBinaryObservable(left_obs));
        }
        if !is_binary(right_obs) {
            return Err(RealDataGateError::NonBinaryObservable(right_obs));
        }
        let product = left_obs * right_obs;
        match bucket {
            SettingPair::Ab => samples_ab.push(product),
            SettingPair::AbPrime => samples_ab_prime.push(product),
            SettingPair::APrimeB => samples_a_prime_b.push(product),
            SettingPair::APrimeBPrime => samples_a_prime_b_prime.push(product),
        }
    }

    if samples_ab.is_empty()
        || samples_ab_prime.is_empty()
        || samples_a_prime_b.is_empty()
        || samples_a_prime_b_prime.is_empty()
    {
        return Err(RealDataGateError::InsufficientCoverage { n: pairs.len() });
    }

    Ok(ConcurrentPairSamples {
        samples_ab,
        samples_ab_prime,
        samples_a_prime_b,
        samples_a_prime_b_prime,
    })
}

#[inline]
fn is_binary(v: i8) -> bool {
    v == 1 || v == -1
}

/// Default static cartel injection: rigs three setting-pair samples to
/// +1 products and the fourth to -1 products. Yields S = 4 (algebraic
/// max for ±1 observables). Same shape as the unit-test cartel in
/// `evaporchain-causal-chsh::gate::tests`.
pub fn default_cartel_injection(honest: &ConcurrentPairSamples) -> ConcurrentPairSamples {
    ConcurrentPairSamples {
        samples_ab: vec![1; honest.samples_ab.len().max(1)],
        samples_ab_prime: vec![1; honest.samples_ab_prime.len().max(1)],
        samples_a_prime_b: vec![1; honest.samples_a_prime_b.len().max(1)],
        samples_a_prime_b_prime: vec![-1; honest.samples_a_prime_b_prime.len().max(1)],
    }
}

/// Final report shape — caller serialises to `GATE_RESULT.md` or JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateReport {
    pub n_pairs: usize,
    pub bucket_counts: [usize; 4],
    pub s_honest: f64,
    pub s_cartel: f64,
    pub gap: f64,
    pub thresholds: GateThresholdsSerde,
    pub verdict: GateVerdictKind,
    pub fail_reasons: Vec<String>,
}

/// Plain-old-data version of the gate thresholds for serialisation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GateThresholdsSerde {
    pub honest_ceiling: f64,
    pub cartel_floor: f64,
    pub min_gap: f64,
}

impl From<GateThresholds> for GateThresholdsSerde {
    fn from(t: GateThresholds) -> Self {
        Self {
            honest_ceiling: t.honest_ceiling,
            cartel_floor: t.cartel_floor,
            min_gap: t.min_gap,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateVerdictKind {
    Pass,
    Fail,
    InputError,
}

/// Run the full real-data gate pipeline.
pub fn run_realdata_gate(
    pairs: &[RawConcurrentPair],
    thresholds: GateThresholds,
) -> Result<GateReport, RealDataGateError> {
    if pairs.is_empty() {
        return Err(RealDataGateError::EmptyTrace);
    }
    let (obs_a, obs_a_prime, obs_b, obs_b_prime) = doctrine_observables(pairs);
    let honest = build_samples(pairs, &*obs_a, &*obs_a_prime, &*obs_b, &*obs_b_prime)?;
    let cartel = default_cartel_injection(&honest);

    // Bucket counts for the report.
    let mut bucket_counts = [0usize; 4];
    for p in pairs {
        match bucket_pair(&p.tag) {
            SettingPair::Ab => bucket_counts[0] += 1,
            SettingPair::AbPrime => bucket_counts[1] += 1,
            SettingPair::APrimeB => bucket_counts[2] += 1,
            SettingPair::APrimeBPrime => bucket_counts[3] += 1,
        }
    }

    let verdict = run_synthetic_gate(&honest, &cartel, thresholds);
    let (kind, s_honest, s_cartel, gap, fail_reasons) = match verdict {
        GateVerdict::Pass {
            s_honest,
            s_cartel,
            gap,
        } => (GateVerdictKind::Pass, s_honest, s_cartel, gap, vec![]),
        GateVerdict::Fail {
            s_honest,
            s_cartel,
            gap,
            reasons,
        } => (GateVerdictKind::Fail, s_honest, s_cartel, gap, reasons),
        GateVerdict::InputError(msg) => (GateVerdictKind::InputError, 0.0, 0.0, 0.0, vec![msg]),
    };

    Ok(GateReport {
        n_pairs: pairs.len(),
        bucket_counts,
        s_honest,
        s_cartel,
        gap,
        thresholds: thresholds.into(),
        verdict: kind,
        fail_reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_causal_chsh::gate::GateThresholds;

    fn pair(e1: u64, t1: u64, e2: u64, t2: u64, tag_byte: u8) -> RawConcurrentPair {
        let mut tag = [0u8; 32];
        tag[31] = tag_byte;
        RawConcurrentPair {
            first: BlockStats {
                energy: e1,
                tx_count: t1,
            },
            second: BlockStats {
                energy: e2,
                tx_count: t2,
            },
            tag,
        }
    }

    fn balanced_trace() -> Vec<RawConcurrentPair> {
        // 16 pairs covering all 4 buckets evenly. Each block's stats
        // are split across high/low halves.
        let mut out = Vec::new();
        for i in 0..16u8 {
            let high_energy = i & 1 == 1;
            let high_tx = (i >> 1) & 1 == 1;
            let high_energy2 = (i >> 2) & 1 == 1;
            let high_tx2 = (i >> 3) & 1 == 1;
            let bucket_byte = i & 0b11;
            out.push(pair(
                if high_energy { 100 } else { 10 },
                if high_tx { 100 } else { 10 },
                if high_energy2 { 100 } else { 10 },
                if high_tx2 { 100 } else { 10 },
                bucket_byte,
            ));
        }
        out
    }

    // ── bucketing ────────────────────────────────────────────────

    #[test]
    fn bucket_uses_low_bits_of_last_byte() {
        let mut tag = [0u8; 32];
        tag[31] = 0;
        assert_eq!(bucket_pair(&tag), SettingPair::Ab);
        tag[31] = 1;
        assert_eq!(bucket_pair(&tag), SettingPair::AbPrime);
        tag[31] = 2;
        assert_eq!(bucket_pair(&tag), SettingPair::APrimeB);
        tag[31] = 3;
        assert_eq!(bucket_pair(&tag), SettingPair::APrimeBPrime);
        // Higher bits ignored.
        tag[31] = 0b1111_1100;
        assert_eq!(bucket_pair(&tag), SettingPair::Ab);
    }

    #[test]
    fn bucketing_is_deterministic() {
        let trace = balanced_trace();
        let mut counts1 = [0usize; 4];
        let mut counts2 = [0usize; 4];
        for p in &trace {
            match bucket_pair(&p.tag) {
                SettingPair::Ab => counts1[0] += 1,
                SettingPair::AbPrime => counts1[1] += 1,
                SettingPair::APrimeB => counts1[2] += 1,
                SettingPair::APrimeBPrime => counts1[3] += 1,
            }
        }
        for p in &trace {
            match bucket_pair(&p.tag) {
                SettingPair::Ab => counts2[0] += 1,
                SettingPair::AbPrime => counts2[1] += 1,
                SettingPair::APrimeB => counts2[2] += 1,
                SettingPair::APrimeBPrime => counts2[3] += 1,
            }
        }
        assert_eq!(counts1, counts2);
        assert_eq!(counts1, [4, 4, 4, 4]);
    }

    // ── observables ──────────────────────────────────────────────

    #[test]
    fn energy_above_median_returns_pm1() {
        let obs = energy_above_median_observable(50);
        assert_eq!(
            obs(&BlockStats {
                energy: 100,
                tx_count: 0
            }),
            1
        );
        assert_eq!(
            obs(&BlockStats {
                energy: 10,
                tx_count: 0
            }),
            -1
        );
        assert_eq!(
            obs(&BlockStats {
                energy: 50,
                tx_count: 0
            }),
            1
        );
    }

    #[test]
    fn tx_above_median_returns_pm1() {
        let obs = tx_count_above_median_observable(50);
        assert_eq!(
            obs(&BlockStats {
                energy: 0,
                tx_count: 100
            }),
            1
        );
        assert_eq!(
            obs(&BlockStats {
                energy: 0,
                tx_count: 10
            }),
            -1
        );
    }

    #[test]
    fn doctrine_observables_compute_per_side_medians() {
        let trace = balanced_trace();
        let (obs_a, obs_a_prime, obs_b, obs_b_prime) = doctrine_observables(&trace);
        // Sanity: each observable returns ±1 on every block.
        for p in &trace {
            for v in [
                obs_a(&p.first),
                obs_a_prime(&p.first),
                obs_b(&p.second),
                obs_b_prime(&p.second),
            ] {
                assert!(v == 1 || v == -1);
            }
        }
    }

    // ── build_samples ────────────────────────────────────────────

    #[test]
    fn empty_trace_rejected() {
        let pairs: Vec<RawConcurrentPair> = vec![];
        let obs = energy_above_median_observable(50);
        let r = build_samples(&pairs, &obs, &obs, &obs, &obs);
        assert_eq!(r.unwrap_err(), RealDataGateError::EmptyTrace);
    }

    #[test]
    fn trace_missing_a_bucket_rejected() {
        // Three pairs, all bucketed into AB (tag_byte = 0).
        let trace = vec![
            pair(100, 100, 100, 100, 0),
            pair(10, 10, 10, 10, 0),
            pair(100, 10, 10, 100, 0),
        ];
        let obs = energy_above_median_observable(50);
        let err = build_samples(&trace, &obs, &obs, &obs, &obs).unwrap_err();
        assert!(matches!(
            err,
            RealDataGateError::InsufficientCoverage { n: 3 }
        ));
    }

    #[test]
    fn balanced_trace_produces_four_non_empty_samples() {
        let trace = balanced_trace();
        let (obs_a, obs_a_prime, obs_b, obs_b_prime) = doctrine_observables(&trace);
        let s = build_samples(&trace, &*obs_a, &*obs_a_prime, &*obs_b, &*obs_b_prime).unwrap();
        assert!(!s.samples_ab.is_empty());
        assert!(!s.samples_ab_prime.is_empty());
        assert!(!s.samples_a_prime_b.is_empty());
        assert!(!s.samples_a_prime_b_prime.is_empty());
        // Total samples = number of pairs.
        let total = s.samples_ab.len()
            + s.samples_ab_prime.len()
            + s.samples_a_prime_b.len()
            + s.samples_a_prime_b_prime.len();
        assert_eq!(total, trace.len());
    }

    // ── cartel injection ─────────────────────────────────────────

    #[test]
    fn cartel_injection_preserves_sample_counts() {
        let trace = balanced_trace();
        let (obs_a, obs_a_prime, obs_b, obs_b_prime) = doctrine_observables(&trace);
        let honest = build_samples(&trace, &*obs_a, &*obs_a_prime, &*obs_b, &*obs_b_prime).unwrap();
        let cartel = default_cartel_injection(&honest);
        assert_eq!(honest.samples_ab.len(), cartel.samples_ab.len());
        assert_eq!(honest.samples_ab_prime.len(), cartel.samples_ab_prime.len());
        assert_eq!(
            honest.samples_a_prime_b.len(),
            cartel.samples_a_prime_b.len()
        );
        assert_eq!(
            honest.samples_a_prime_b_prime.len(),
            cartel.samples_a_prime_b_prime.len()
        );
    }

    #[test]
    fn cartel_injection_yields_max_violation() {
        let trace = balanced_trace();
        let (obs_a, obs_a_prime, obs_b, obs_b_prime) = doctrine_observables(&trace);
        let honest = build_samples(&trace, &*obs_a, &*obs_a_prime, &*obs_b, &*obs_b_prime).unwrap();
        let cartel = default_cartel_injection(&honest);
        let s = evaporchain_causal_chsh::compute_chsh_s(&cartel).unwrap();
        assert!((s - 4.0).abs() < 1e-9);
    }

    // ── run_realdata_gate ────────────────────────────────────────

    #[test]
    fn run_gate_on_balanced_trace_passes_doctrine() {
        let trace = balanced_trace();
        let report = run_realdata_gate(&trace, GateThresholds::doctrine()).unwrap();
        assert_eq!(report.n_pairs, 16);
        assert_eq!(report.bucket_counts, [4, 4, 4, 4]);
        assert!(matches!(report.verdict, GateVerdictKind::Pass));
        assert!(report.s_cartel > 2.2);
        assert!(report.s_honest < 1.8);
        assert!(report.gap > 0.4);
    }

    #[test]
    fn run_gate_serialises_to_json() {
        let trace = balanced_trace();
        let report = run_realdata_gate(&trace, GateThresholds::doctrine()).unwrap();
        let s = serde_json::to_string(&report).unwrap();
        assert!(s.contains("verdict"));
        assert!(s.contains("Pass"));
    }

    // ── median ───────────────────────────────────────────────────

    #[test]
    fn median_of_sorted_input() {
        assert_eq!(median(&[1, 2, 3, 4, 5]), 3);
    }

    #[test]
    fn median_of_unsorted_input() {
        assert_eq!(median(&[5, 1, 4, 2, 3]), 3);
    }

    #[test]
    fn median_of_empty_returns_zero() {
        assert_eq!(median(&[]), 0);
    }

    #[test]
    fn median_of_even_count_returns_upper_lower_split() {
        // [1, 2, 3, 4]: median index = 4/2 = 2 → 3.
        assert_eq!(median(&[1, 2, 3, 4]), 3);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Lane O.2 ships the real-data gate runner. A
        // chain operator extracts concurrent-block pairs from a
        // LightCone trace, hands them to run_realdata_gate, gets
        // a serialisable GateReport. The synthetic cartel
        // injection runs over the SAME pair set so the gap
        // measures discrimination on the same underlying
        // traffic. Doctrine threshold {1.8/2.2/0.4} → Pass under
        // a balanced honest trace + max-violation cartel."

        let trace = balanced_trace();
        let report = run_realdata_gate(&trace, GateThresholds::doctrine()).unwrap();
        assert!(matches!(report.verdict, GateVerdictKind::Pass));

        // Per-bucket coverage is balanced.
        for &c in &report.bucket_counts {
            assert!(c > 0);
        }

        // Report serialises round-trip.
        let s = serde_json::to_string(&report).unwrap();
        let _: GateReport = serde_json::from_str(&s).unwrap();
    }
}
