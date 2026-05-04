//! Thin binary wrapper.

use std::process::ExitCode;

use evaporchain_finality_attestation_cli::{run_cli, cli::StdIo};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut io = StdIo;
    match run_cli(&argv, &mut io) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("evaporchain-finality-attestation: {e}");
            ExitCode::from(1)
        }
    }
}
