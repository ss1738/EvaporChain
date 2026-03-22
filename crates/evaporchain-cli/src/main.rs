use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "evaporchain", about = "EvaporChain node CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new EvaporChain node
    Init,
    /// Run the EvaporChain node
    Run,
    /// Show node status
    Status,
    /// Run benchmarks
    Benchmark,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init => {
            println!("Initializing new EvaporChain node...");
        }
        Commands::Run => {
            println!("Starting EvaporChain node...");
        }
        Commands::Status => {
            println!("Node status: scaffold mode");
        }
        Commands::Benchmark => {
            println!("Run: cd prototypes/fold-a-block && cargo run --release");
        }
    }

    Ok(())
}
