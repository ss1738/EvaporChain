//! CLI for the Finality Attestation capstone.
//!
//! Two subcommands:
//!
//! ```text
//! evaporchain-finality-attestation build  --input att.json [--output root.hex]
//! evaporchain-finality-attestation verify --input att.json --root <hex64>
//! ```
//!
//! ## I/O conventions
//!
//! - `--input -` reads JSON from stdin.
//! - `--output -` (build only) writes hex to stdout.
//! - `--root` accepts `0x` prefix or bare 64 hex chars.
//!
//! Pluggable `CliIo` trait so the library half is fully unit-testable
//! in-memory.

pub mod cli;

pub use cli::{run_cli, CliArgs, CliError, CliIo, Subcommand};
