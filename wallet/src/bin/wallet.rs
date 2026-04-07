//! EvaporChain Wallet CLI binary entry point.

use clap::Parser;
use evaporchain_wallet::cli::{Cli, run};
use evaporchain_wallet::hints::format_error_with_hint;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("{}", format_error_with_hint(&e.to_string()));
        std::process::exit(1);
    }
}
