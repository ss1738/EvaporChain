//! Singh Attractor Consensus V2 — Bell-anchored fallback +
//! bounded Lyapunov drift.
//!
//! V1 (`evaporchain-singh-attractor`) ships basin-membership and a
//! deterministic *nearest-centre* fallback. The fallback is
//! predictable: any validator can compute it from public state, so
//! a malicious proposer can target the fallback attractor by
//! pushing state into the no-basin region.
//!
//! V2 closes that gap:
//!
//! 1. **Bell-anchored fallback** — when no basin contains the
//!    state, V2 uses a 32-byte certificate seed (typically from
//!    `evaporchain-bell-beacon-v2::BellCertificate.seed`) to draw
//!    the fallback attractor by *energy-distance-weighted sampling*.
//!    Closer attractors are likelier, but not deterministic; the
//!    seed makes the choice unpredictable until the certificate
//!    is published. Anti-grinding follows from the Bell-Beacon's
//!    own anti-grinding properties (sorted-tag seed derivation).
//!
//! 2. **Bounded Lyapunov drift** — V2 also returns the next-step
//!    nudge `drift = clamp(center − state, ±drift_rate)`. The
//!    chain applies this drift to the energy state each epoch.
//!    Strict-decrement of `|state − center|` ⇒ Lyapunov stability
//!    on the basin's interior.
//!
//! 3. **In-basin selection unchanged** — when a basin contains
//!    the state, V2 picks that attractor identically to V1. Only
//!    the fallback path consumes the certificate seed.

pub mod draw;

pub use draw::{draw_attractor, AttractorV2, DrawError, DrawResult};
