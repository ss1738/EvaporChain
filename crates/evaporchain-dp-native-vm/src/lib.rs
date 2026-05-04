//! Differential-Privacy-Native VM.
//!
//! ## What this crate is
//!
//! A budget tracker + noise mechanism layer for VM-level
//! `(ε, δ)`-differential privacy. Every query against a sensitive
//! dataset draws from a **finite ε-budget** assigned to the dataset
//! at registration. When the budget is exhausted, the dataset's
//! query gate fails closed; no future query — including identical
//! ones — can be answered.
//!
//! Three structural decisions:
//!
//! 1. **Budget is decay-native.** The budget reservoir maps onto
//!    EvaporChain's single-λ: `ε_remaining(t) = ε_initial - Σ
//!    ε_consumed_by_queries(s)` for `s ≤ t`, where each query's
//!    consumption is bounded by the composition theorem. Once
//!    spent, budget cannot be replenished — same as any decay
//!    quantity on the chain.
//!
//! 2. **Noise is validator-deterministic.** Each query carries a
//!    32-byte seed; noise is sampled via seeded ChaCha-derived
//!    Laplace/Gaussian, NOT system entropy. Two validators
//!    answering the same query against the same dataset under the
//!    same seed produce byte-identical noised outputs — required
//!    for consensus.
//!
//! 3. **Composition is provable, not heuristic.** Two composition
//!    theorems are exposed:
//!    - **Basic (Dwork-McSherry-Nissim-Smith 2006):** k-fold
//!      composition of `(ε, 0)`-DP mechanisms is `(kε, 0)`-DP.
//!    - **Advanced (Dwork-Rothblum-Vadhan 2010):** for `δ' > 0`,
//!      the same k-fold composition is also `(ε', δ')`-DP with
//!      `ε' = ε√(2k ln(1/δ')) + kε(eᵉ-1)`. Tighter for large k.
//!    The query gate accepts whichever bound the caller provides,
//!    and refuses the query if either is over budget.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT execute the underlying queries. The VM submits a
//!   `(query_sensitivity, ε_consumed, δ_consumed, seed)` tuple +
//!   the raw answer; this layer adds noise and gates the budget.
//! - It does NOT define the dataset. The chain identifies datasets
//!   by 32-byte handles; this layer tracks budgets per handle.
//! - It does NOT provide local-DP / shuffle-DP / Renyi-DP. V1 is
//!   central-DP only. RDP (more compositionally tight) is V2.
//!
//! ## Module map
//!
//! - [`budget`] — [`PrivacyBudget`] + [`BudgetRegistry`] + the
//!   query gate.
//! - [`noise`] — Laplace and Gaussian mechanisms with seeded
//!   ChaCha for validator-determinism.
//! - [`composition`] — basic + advanced composition theorems +
//!   the witness types `BasicComposition` / `AdvancedComposition`.

pub mod budget;
pub mod composition;
pub mod noise;

pub use budget::{BudgetError, BudgetRegistry, DatasetId, PrivacyBudget};
pub use composition::{
    advanced_composition_epsilon, basic_composition_epsilon, AdvancedComposition,
    BasicComposition, CompositionError,
};
pub use noise::{NoiseError, NoiseMechanism, NoiseSeed};
