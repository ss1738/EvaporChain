//! Bell-Certified Beacon V2 — chain-level certificate.
//!
//! V1 (`evaporchain-bell-beacon`) ships the abstract CHSH gate at
//! integer milli-units. V2 hardens that primitive onto real chain
//! data:
//!
//! 1. Operator collects a window of concurrent-block pairs from the
//!    LightCone DAG (height range `[window_start, window_end)`).
//! 2. Doctrine observables (energy-above-median, tx-count-above-median)
//!    bin each pair into one of four CHSH setting-pairs deterministically
//!    via the pair's tag.
//! 3. The gate runs against the honest sample plus a synthetic
//!    coordinated-subset cartel injection over the *same* pair set
//!    (so the gap measures discrimination on the actual traffic).
//! 4. On Pass, a `BellCertificate` is issued: window bounds, sample
//!    stats, and a beacon seed = BLAKE3(domain || window || prev_block ||
//!    pair_tags). The certificate attaches to the proposer's next
//!    block header.
//! 5. Verifiers re-run the gate against the same window and re-derive
//!    the seed. Any divergence rejects the block.
//!
//! Anti-grinding follows from the seed derivation: the seed depends
//! on the chain-supplied `prev_block_hash` plus the canonical pair
//! tags, so a proposer cannot pick the seed by reordering pairs.

pub mod certificate;
pub mod issuance;
pub mod observables;
pub mod verification;

pub use certificate::{BellCertificate, CertificateBytes, GateThresholdsMilli};
pub use issuance::{issue_certificate, BeaconError};
pub use observables::{ConcurrentPair, PairStats};
pub use verification::{verify_certificate, VerifyError};
