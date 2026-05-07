//! Thin binary wrapper.

use std::process::ExitCode;

use evaporchain_causal_chsh_cli::{run_cli, CliError, CliIo};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut io = StdIo;
    match run_cli(&argv, &mut io) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("evaporchain-causal-chsh: {e}");
            ExitCode::from(1)
        }
    }
}

struct StdIo;

impl CliIo for StdIo {
    fn read_input(&mut self, path: &str) -> Result<String, CliError> {
        use std::io::Read;
        if path == "-" {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| CliError::Io(format!("stdin: {e}")))?;
            Ok(buf)
        } else {
            std::fs::read_to_string(path).map_err(|e| CliError::Io(format!("{path}: {e}")))
        }
    }
    fn write_output(&mut self, path: &str, content: &str) -> Result<(), CliError> {
        use std::io::Write;
        if path == "-" {
            std::io::stdout()
                .write_all(content.as_bytes())
                .map_err(|e| CliError::Io(format!("stdout: {e}")))?;
            Ok(())
        } else {
            std::fs::write(path, content).map_err(|e| CliError::Io(format!("{path}: {e}")))
        }
    }
}
