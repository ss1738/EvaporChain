//! Lane O.6 — CLI driver for the Causal-CHSH stack.
//!
//! The library half holds the testable pipeline: argv parsing,
//! input reading, sweep + render orchestration. The binary half
//! (`main.rs`) is a thin wrapper that calls `run_cli` against real
//! `std::env::args` and `std::io::{stdin, stdout}`.
//!
//! Usage (after `cargo build --release`):
//!
//! ```text
//! evaporchain-causal-chsh --input honest.json [--seed 0xDEADBEEF…] [--output report.md]
//! evaporchain-causal-chsh --input -        # read JSON from stdin
//! evaporchain-causal-chsh --input - --output -   # read stdin, write stdout
//! ```
//!
//! Doctrine thresholds are hard-coded to {1.8 / 2.2 / 0.4} in V1.
//! Override flags are a V2 concern.

pub mod cli;

pub use cli::{run_cli, CliArgs, CliError, CliIo};
