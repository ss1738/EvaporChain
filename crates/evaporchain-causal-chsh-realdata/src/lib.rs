//! Lane O.2 — real-data gate runner for Causal-CHSH.
//!
//! ## Pipeline
//!
//! 1. Caller passes a `Vec<RawConcurrentPair>` — pre-extracted
//!    concurrent-block pairs from a LightCone-DAG trace, each
//!    carrying both blocks' raw stats (energy, tx_count).
//! 2. Caller picks the four observable definitions for settings
//!    A, A' (on the first block) and B, B' (on the second).
//!    Each observable is a `Fn(&BlockStats) -> i8` returning ±1.
//!    The doctrine ships two canonical observables: energy-
//!    above-median and tx-count-above-median.
//! 3. The runner partitions the pairs across the four setting-
//!    pairs (deterministic hash-based bucketing) and computes
//!    each sample of ±1 products.
//! 4. The runner builds `ConcurrentPairSamples`, calls
//!    `compute_chsh_s` for the honest-trace S, runs the cartel
//!    injection over the same pairs, and feeds both into
//!    `run_synthetic_gate`.
//! 5. Output: a [`GateReport`] suitable for serialising to
//!    `GATE_RESULT.md` (the doctrine target shape).
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Bucketing is deterministic.** Two runners on the same
//!    input pair the same indices into the same setting-pair
//!    buckets, validator-byte-equality.
//!
//! 2. **Cartel injection mutates only observables, not the
//!    pair set.** The honest sample and cartel sample have
//!    identical pair counts and pair identities, so the gate's
//!    "gap" measures discrimination on the same underlying
//!    traffic.
//!
//! 3. **Empty / single-pair traces are rejected with a clear
//!    error.** No silent fallback to noise.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT pull real chain data. Caller hands it in; the
//!   chain's higher layer (or an off-chain ETL) does the
//!   extraction.
//! - Does NOT model adaptive cartels. V1 ships a static
//!   "max-violation" cartel (rig (A,B), (A,B'), (A',B) to +1
//!   and (A',B') to -1). Adaptive / quantum-statistic cartels
//!   are V2.
//!
//! ## Module map
//!
//! - [`runner`] — the pipeline driver + observable types +
//!   gate-report shape.

pub mod runner;

pub use runner::{
    bucket_pair, build_samples, default_cartel_injection, doctrine_observables,
    energy_above_median_observable, run_realdata_gate, tx_count_above_median_observable,
    BlockStats, GateReport, RawConcurrentPair, RealDataGateError, SettingPair,
};
