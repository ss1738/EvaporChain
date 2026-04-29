//! Autopoietic Chain — A4.3.8.
//!
//! # Doctrine
//!
//! Maturana-Varela 1972 *De máquinas y seres vivos* / 1980 *Autopoiesis and
//! Cognition*.  An autopoietic system:
//!   1. Produces the components of which it is composed.
//!   2. Maintains the boundary that defines it as a system.
//!   3. Does so continuously without external direction.
//!
//! EvaporChain is already partly autopoietic.  Three shipped primitives
//! map directly to the three autopoietic requirements:
//!
//! | Autopoietic requirement | EvaporChain primitive |
//! |---|---|
//! | Self-production (funds itself) | `evaporchain-refresh-patronage` — RefreshPool feeds covenant holders |
//! | Self-maintenance (adjusts parameters) | `evaporchain-sentinel` — autonomic controller adjusts λ-derived bounds |
//! | Self-boundary (gates own upgrades) | `evaporchain-llsa` — Coq-verified proof gates every amendment |
//!
//! This crate provides:
//! - [`AutopoieticSystem`] trait — formal interface for any autopoietic chain subsystem.
//! - [`AutopoieticHealth`] — aggregate health report across all three components.
//! - [`ChainAutopoiesis`] — concrete coordinator; produces the health report from
//!   live subsystem state.
//!
//! # Four-act narrative spine
//!
//! Per §A2.9.18: Tombstone → Mortis → Sentinel → Genesis.  In autopoietic
//! terms: the chain acknowledges death (Tombstone), executes death (Mortis),
//! adjusts its own parameters to survive (Sentinel), and maintains the legal
//! boundary for future life (LLSA + Patronage).  This is not metaphor — it is
//! the literal operational flow.

pub mod autopoiesis;

pub use autopoiesis::{AutopoieticHealth, AutopoieticStatus, ChainAutopoiesis};
