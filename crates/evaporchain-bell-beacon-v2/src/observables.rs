//! Doctrine observables, bucketing, and `ConcurrentPair`.

use evaporchain_causal_chsh::chsh::ConcurrentPairSamples;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairStats {
    pub energy: u64,
    pub tx_count: u64,
}

/// One concurrent-block pair from the LightCone DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrentPair {
    /// Stats for the first concurrent block.
    pub first: PairStats,
    /// Stats for the second concurrent block.
    pub second: PairStats,
    /// 32-byte tag the chain assigns; bucketed via `tag[31] & 0b11`.
    pub tag: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingPair {
    Ab,
    AbPrime,
    APrimeB,
    APrimeBPrime,
}

fn bucket(tag: &[u8; 32]) -> SettingPair {
    match tag[31] & 0b11 {
        0 => SettingPair::Ab,
        1 => SettingPair::AbPrime,
        2 => SettingPair::APrimeB,
        _ => SettingPair::APrimeBPrime,
    }
}

/// Lower-median over a slice (validator-deterministic).
fn median(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted: Vec<u64> = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

/// Build the four `ConcurrentPairSamples` buckets from a window of
/// concurrent pairs using the doctrine observables:
///
/// - A  = first.energy   ≥ median_energy_first   ? +1 : -1
/// - A' = first.tx_count ≥ median_tx_first       ? +1 : -1
/// - B  = second.energy  ≥ median_energy_second  ? +1 : -1
/// - B' = second.tx_count ≥ median_tx_second     ? +1 : -1
///
/// Returns `None` if any of the four buckets ends up empty.
pub fn build_samples(pairs: &[ConcurrentPair]) -> Option<ConcurrentPairSamples> {
    if pairs.is_empty() {
        return None;
    }
    let energies_first: Vec<u64> = pairs.iter().map(|p| p.first.energy).collect();
    let txs_first: Vec<u64> = pairs.iter().map(|p| p.first.tx_count).collect();
    let energies_second: Vec<u64> = pairs.iter().map(|p| p.second.energy).collect();
    let txs_second: Vec<u64> = pairs.iter().map(|p| p.second.tx_count).collect();

    let med_e1 = median(&energies_first);
    let med_t1 = median(&txs_first);
    let med_e2 = median(&energies_second);
    let med_t2 = median(&txs_second);

    let obs_a = |s: &PairStats| if s.energy >= med_e1 { 1i8 } else { -1 };
    let obs_a_prime = |s: &PairStats| if s.tx_count >= med_t1 { 1i8 } else { -1 };
    let obs_b = |s: &PairStats| if s.energy >= med_e2 { 1i8 } else { -1 };
    let obs_b_prime = |s: &PairStats| if s.tx_count >= med_t2 { 1i8 } else { -1 };

    let mut s_ab: Vec<i8> = Vec::new();
    let mut s_ab_p: Vec<i8> = Vec::new();
    let mut s_ap_b: Vec<i8> = Vec::new();
    let mut s_ap_bp: Vec<i8> = Vec::new();

    for p in pairs {
        match bucket(&p.tag) {
            SettingPair::Ab => s_ab.push(obs_a(&p.first) * obs_b(&p.second)),
            SettingPair::AbPrime => s_ab_p.push(obs_a(&p.first) * obs_b_prime(&p.second)),
            SettingPair::APrimeB => s_ap_b.push(obs_a_prime(&p.first) * obs_b(&p.second)),
            SettingPair::APrimeBPrime => {
                s_ap_bp.push(obs_a_prime(&p.first) * obs_b_prime(&p.second))
            }
        }
    }

    if s_ab.is_empty() || s_ab_p.is_empty() || s_ap_b.is_empty() || s_ap_bp.is_empty() {
        return None;
    }

    Some(ConcurrentPairSamples {
        samples_ab: s_ab,
        samples_ab_prime: s_ab_p,
        samples_a_prime_b: s_ap_b,
        samples_a_prime_b_prime: s_ap_bp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(e1: u64, t1: u64, e2: u64, t2: u64, tag_byte: u8) -> ConcurrentPair {
        let mut tag = [0u8; 32];
        tag[31] = tag_byte;
        ConcurrentPair {
            first: PairStats {
                energy: e1,
                tx_count: t1,
            },
            second: PairStats {
                energy: e2,
                tx_count: t2,
            },
            tag,
        }
    }

    #[test]
    fn empty_returns_none() {
        assert!(build_samples(&[]).is_none());
    }

    #[test]
    fn single_bucket_returns_none() {
        let trace = vec![pair(10, 10, 10, 10, 0); 4];
        assert!(build_samples(&trace).is_none());
    }

    #[test]
    fn balanced_trace_fills_all_four_buckets() {
        let mut trace = Vec::new();
        for i in 0..16u8 {
            trace.push(pair(
                if i & 1 == 1 { 100 } else { 10 },
                if (i >> 1) & 1 == 1 { 100 } else { 10 },
                if (i >> 2) & 1 == 1 { 100 } else { 10 },
                if (i >> 3) & 1 == 1 { 100 } else { 10 },
                i & 0b11,
            ));
        }
        let s = build_samples(&trace).unwrap();
        assert!(!s.samples_ab.is_empty());
        assert!(!s.samples_ab_prime.is_empty());
        assert!(!s.samples_a_prime_b.is_empty());
        assert!(!s.samples_a_prime_b_prime.is_empty());
    }
}
