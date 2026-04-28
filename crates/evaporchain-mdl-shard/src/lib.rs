//! MDL-Shard.
//!
//! Per `research/INVENTION_STACK.md` §A1.3 (Tier-0 supporting):
//!
//! > **MDL-Shard** | Rissanen 1978 (Minimum Description Length) |
//! > sharding partition `Π* = argmin L(Π) + L(D | Π)`; provably
//! > optimal not heuristic.
//!
//! ## Why MDL is the right principle
//!
//! Existing chains pick shard counts and assignments by intuition
//! (Ethereum 64, Near "dynamic resharding", Cosmos zone-per-app).
//! MDL chooses the partition that minimises the *total* cost of
//! describing both the partition itself and the data conditional on
//! it — i.e., the partition that wastes the least bits.
//!
//! For a chain `D = {state objects}` and a partition `Π = {S_1, …,
//! S_k}`:
//!
//! - `L(Π)` = bits to describe how the items are grouped.
//! - `L(D | Π)` = bits to describe the items given that grouping
//!   (fewer when items in each shard are "similar" by some encoding).
//!
//! Minimising the sum picks the shard count and assignment that best
//! exploits the regularity in the chain's workload. No tuning
//! parameter.
//!
//! ## What this crate ships
//!
//! - [`partition`] — `Partition` type + invariant check.
//! - [`length`] — `description_length(Π)` and `data_length(Π, items)`.
//! - [`search`] — `mdl_optimal(items, max_shards)` exhaustive
//!   small-case search. Production replaces with a beam-search
//!   approximation; substrate exposes the score function so the
//!   approximation can be benchmarked against the exact optimum.

pub mod length;
pub mod partition;
pub mod search;

pub use length::{data_length, description_length, mdl_score};
pub use partition::{Partition, PartitionError, ShardId};
pub use search::mdl_optimal;
