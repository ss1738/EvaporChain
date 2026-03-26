use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::*;
use serde::{Deserialize, Serialize};

// ──────────────────────────── CLI Arguments ──────────────────────────────

#[derive(Parser)]
#[command(
    name = "evaporchain",
    about = "EvaporChain CLI — interact with a running node",
    version,
    after_help = "Examples:\n  evaporchain status\n  evaporchain objects\n  evaporchain transfer --from 1 --to 2 --amount 500\n  evaporchain blocks --limit 5 --json"
)]
pub struct Cli {
    /// API server URL
    #[arg(long, default_value = "http://localhost:8080", global = true)]
    pub api_url: String,

    /// Output as JSON instead of formatted text
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Show node status (block height, epoch, objects, peers)
    Status,

    /// List all state objects with energy bars
    Objects,

    /// List all accounts with balances
    Accounts,

    /// Show recent blocks
    Blocks {
        /// Number of blocks to show
        #[arg(long, default_value = "10")]
        limit: usize,
    },

    /// Submit a transfer transaction
    Transfer {
        /// Sender address byte (1=Alice, 2=Bob, 3=Charlie)
        #[arg(long)]
        from: u8,

        /// Receiver address byte
        #[arg(long)]
        to: u8,

        /// Amount to transfer
        #[arg(long)]
        amount: u64,

        /// Transaction nonce
        #[arg(long, default_value = "0")]
        nonce: u64,
    },

    /// Create a new state object
    CreateObject {
        /// Creator address byte (1=Alice, 2=Bob, 3=Charlie)
        #[arg(long)]
        creator: u8,

        /// Object ID byte
        #[arg(long)]
        id: u8,

        /// Initial energy
        #[arg(long)]
        energy: u64,

        /// Half-life in epochs
        #[arg(long)]
        half_life: u64,
    },

    /// Refresh a decaying object (add energy)
    Refresh {
        /// Object ID byte
        #[arg(long)]
        object: u8,

        /// Energy to deposit
        #[arg(long)]
        energy: u64,
    },

    /// Resurrect a ghost object
    Resurrect {
        /// Ghost object ID byte
        #[arg(long)]
        object: u8,

        /// Energy to deposit
        #[arg(long)]
        energy: u64,
    },

    /// Request tokens from the faucet
    Faucet {
        /// Address to receive tokens (hex)
        #[arg(long)]
        address: String,
    },

    /// Show consensus info (validators, proposer, round)
    Consensus,

    /// Launch a local devnet with multiple validators
    Devnet {
        /// Number of validators (default 4)
        #[arg(long, default_value = "4")]
        validators: u32,

        /// Enable demo mode (auto-generate transactions)
        #[arg(long)]
        demo: bool,
    },
}

// ──────────────────────────── API Response Types ─────────────────────────

#[derive(Debug, Deserialize, Serialize)]
struct StatusResponse {
    chain_name: String,
    version: String,
    block_height: u64,
    epoch: u64,
    active_objects: usize,
    ghost_count: usize,
    total_evaporated: u64,
    peer_count: usize,
    state_root: String,
    proving_enabled: bool,
    uptime_seconds: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct ObjectResponse {
    id: String,
    name: String,
    owner: String,
    owner_name: String,
    energy: u64,
    max_energy: u64,
    half_life: u64,
    state: String,
    created_epoch: u64,
    last_refreshed: u64,
    grace_epoch: Option<u64>,
    current_energy: u64,
    decay_percentage: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct AccountResponse {
    address: String,
    name: String,
    balance: u64,
    nonce: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct BlockRecord {
    number: u64,
    epoch: u64,
    parent_hash: String,
    state_root: String,
    tx_count: usize,
    evaporations: usize,
    entered_grace: usize,
    timestamp: u64,
    active_objects: usize,
    ghost_count: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct TxResult {
    success: bool,
    message: String,
}

// ──────────────────────────── Display Helpers ─────────────────────────────

fn energy_bar(current: u64, max: u64, width: usize) -> String {
    if max == 0 {
        return " ".repeat(width);
    }
    let pct = (current as f64 / max as f64).clamp(0.0, 1.0);
    let filled = (pct * width as f64).round() as usize;
    let empty = width - filled;

    let bar_char = "\u{2588}"; // Full block
    let empty_char = "\u{2591}"; // Light shade

    let filled_str = bar_char.repeat(filled);
    let empty_str = empty_char.repeat(empty);

    if pct > 0.5 {
        format!(
            "{}{}",
            filled_str.green(),
            empty_str.truecolor(60, 60, 60)
        )
    } else if pct > 0.2 {
        format!(
            "{}{}",
            filled_str.yellow(),
            empty_str.truecolor(60, 60, 60)
        )
    } else {
        format!("{}{}", filled_str.red(), empty_str.truecolor(60, 60, 60))
    }
}

fn state_badge(state: &str) -> ColoredString {
    match state {
        "Active" => " Active ".on_truecolor(20, 60, 30).green().bold(),
        "Grace" => " Grace  ".on_truecolor(60, 50, 10).yellow().bold(),
        "Ghost" => " Ghost  ".on_truecolor(60, 20, 20).red().bold(),
        "Risen" => " Risen  ".on_truecolor(40, 20, 60).purple().bold(),
        _ => state.normal(),
    }
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn separator() -> String {
    "\u{2500}".repeat(72).truecolor(60, 70, 80).to_string()
}

fn print_header(title: &str) {
    println!();
    println!(
        "  {}  {}",
        "\u{25C6}".cyan(),
        title.bold()
    );
    println!("  {}", separator());
}

// ──────────────────────────── API Client ──────────────────────────────────

async fn api_get<T: serde::de::DeserializeOwned>(base: &str, path: &str) -> Result<T> {
    let url = format!("{}{}", base, path);
    let resp = reqwest::get(&url)
        .await
        .with_context(|| format!("Failed to connect to {}", url))?;

    if !resp.status().is_success() {
        anyhow::bail!("API returned status {}", resp.status());
    }

    resp.json::<T>()
        .await
        .with_context(|| format!("Failed to parse response from {}", path))
}

async fn api_post<T: serde::de::DeserializeOwned>(
    base: &str,
    path: &str,
    body: &serde_json::Value,
) -> Result<T> {
    let url = format!("{}{}", base, path);
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(body)
        .send()
        .await
        .with_context(|| format!("Failed to connect to {}", url))?;

    if !resp.status().is_success() {
        anyhow::bail!("API returned status {}", resp.status());
    }

    resp.json::<T>()
        .await
        .with_context(|| format!("Failed to parse response from {}", path))
}

// ──────────────────────────── Command Handlers ───────────────────────────

async fn cmd_status(base: &str, json_mode: bool) -> Result<()> {
    let status: StatusResponse = api_get(base, "/api/status").await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    print_header("EvaporChain Node Status");

    println!(
        "  {}  {}",
        "Chain:".truecolor(140, 150, 170),
        format!("{} v{}", status.chain_name, status.version).white().bold()
    );
    println!(
        "  {}  {}",
        "Block:".truecolor(140, 150, 170),
        status.block_height.to_string().cyan().bold()
    );
    println!(
        "  {}  {}",
        "Epoch:".truecolor(140, 150, 170),
        status.epoch.to_string().purple().bold()
    );
    println!(
        "  {}  {} active  {} ghosts  {} evaporated",
        "State:".truecolor(140, 150, 170),
        status.active_objects.to_string().green().bold(),
        status.ghost_count.to_string().red(),
        status.total_evaporated.to_string().truecolor(200, 100, 100),
    );
    println!(
        "  {}  {}",
        "Peers:".truecolor(140, 150, 170),
        if status.peer_count > 0 {
            status.peer_count.to_string().green()
        } else {
            status.peer_count.to_string().truecolor(140, 150, 170)
        }
    );
    println!(
        "  {}  {}",
        "Root: ".truecolor(140, 150, 170),
        format!("{}...", &status.state_root[..24])
            .truecolor(100, 110, 130)
    );
    println!(
        "  {}  {}",
        "Prove:".truecolor(140, 150, 170),
        if status.proving_enabled {
            "Nova IVC".green()
        } else {
            "Mock".truecolor(140, 150, 170)
        }
    );
    println!(
        "  {}  {}",
        "Up:   ".truecolor(140, 150, 170),
        format_uptime(status.uptime_seconds).truecolor(180, 190, 200)
    );
    println!();

    Ok(())
}

async fn cmd_objects(base: &str, json_mode: bool) -> Result<()> {
    let objects: Vec<ObjectResponse> = api_get(base, "/api/objects").await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&objects)?);
        return Ok(());
    }

    print_header(&format!("State Objects ({})", objects.len()));

    if objects.is_empty() {
        println!("  {}", "No objects in state.".truecolor(140, 150, 170));
        println!();
        return Ok(());
    }

    // Table header
    println!(
        "  {:<18} {:<10} {:>8} {:>8} {:<20} {:>7} {}",
        "NAME".truecolor(100, 110, 130),
        "STATE".truecolor(100, 110, 130),
        "ENERGY".truecolor(100, 110, 130),
        "MAX".truecolor(100, 110, 130),
        "".truecolor(100, 110, 130),
        "DECAY".truecolor(100, 110, 130),
        "OWNER".truecolor(100, 110, 130),
    );
    println!("  {}", separator());

    for obj in &objects {
        let bar = energy_bar(obj.current_energy, obj.max_energy, 16);
        let decay_str = format!("{:.0}%", obj.decay_percentage);
        let decay_colored = if obj.decay_percentage > 70.0 {
            decay_str.red()
        } else if obj.decay_percentage > 30.0 {
            decay_str.yellow()
        } else {
            decay_str.green()
        };

        println!(
            "  {:<18} {} {:>8} {:>8} {} {:>7} {}",
            obj.name.white().bold(),
            state_badge(&obj.state),
            obj.current_energy.to_string().white(),
            obj.max_energy.to_string().truecolor(100, 110, 130),
            bar,
            decay_colored,
            obj.owner_name.truecolor(140, 150, 170),
        );
    }
    println!();

    Ok(())
}

async fn cmd_accounts(base: &str, json_mode: bool) -> Result<()> {
    let accounts: Vec<AccountResponse> = api_get(base, "/api/accounts").await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&accounts)?);
        return Ok(());
    }

    print_header(&format!("Accounts ({})", accounts.len()));

    println!(
        "  {:<12} {:<20} {:>12} {:>8}",
        "NAME".truecolor(100, 110, 130),
        "ADDRESS".truecolor(100, 110, 130),
        "BALANCE".truecolor(100, 110, 130),
        "NONCE".truecolor(100, 110, 130),
    );
    println!("  {}", separator());

    for acc in &accounts {
        let addr_short = format!("{}...", &acc.address[..16]);
        println!(
            "  {:<12} {:<20} {:>12} {:>8}",
            acc.name.cyan().bold(),
            addr_short.truecolor(100, 110, 130),
            acc.balance.to_string().green().bold(),
            acc.nonce.to_string().truecolor(140, 150, 170),
        );
    }
    println!();

    Ok(())
}

async fn cmd_blocks(base: &str, limit: usize, json_mode: bool) -> Result<()> {
    let blocks: Vec<BlockRecord> =
        api_get(base, &format!("/api/blocks?limit={}", limit)).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&blocks)?);
        return Ok(());
    }

    print_header(&format!("Recent Blocks ({})", blocks.len()));

    println!(
        "  {:<8} {:<8} {:>5} {:>6} {:>6} {:>8} {:>8} {:<20}",
        "BLOCK".truecolor(100, 110, 130),
        "EPOCH".truecolor(100, 110, 130),
        "TXS".truecolor(100, 110, 130),
        "EVAP".truecolor(100, 110, 130),
        "GRACE".truecolor(100, 110, 130),
        "ACTIVE".truecolor(100, 110, 130),
        "GHOSTS".truecolor(100, 110, 130),
        "STATE ROOT".truecolor(100, 110, 130),
    );
    println!("  {}", separator());

    for b in &blocks {
        let evap_str = if b.evaporations > 0 {
            format!("\u{1F480}{}", b.evaporations).red().to_string()
        } else {
            "-".truecolor(60, 70, 80).to_string()
        };
        let grace_str = if b.entered_grace > 0 {
            format!("\u{26A0}{}", b.entered_grace).yellow().to_string()
        } else {
            "-".truecolor(60, 70, 80).to_string()
        };

        println!(
            "  {:<8} {:<8} {:>5} {:>6} {:>6} {:>8} {:>8} {:<20}",
            format!("#{}", b.number).cyan().bold(),
            format!("E{}", b.epoch).purple(),
            b.tx_count.to_string().green(),
            evap_str,
            grace_str,
            b.active_objects.to_string().truecolor(180, 190, 200),
            b.ghost_count.to_string().truecolor(140, 150, 170),
            format!("{}...", &b.state_root[..16]).truecolor(80, 90, 100),
        );
    }
    println!();

    Ok(())
}

async fn cmd_transfer(base: &str, from: u8, to: u8, amount: u64, nonce: u64, json_mode: bool) -> Result<()> {
    let body = serde_json::json!({
        "from": from,
        "to": to,
        "amount": amount,
        "nonce": nonce,
    });

    let result: TxResult = api_post(base, "/api/tx/transfer", &body).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if result.success {
        println!("  {} {}", "\u{2714}".green().bold(), result.message);
    } else {
        println!("  {} {}", "\u{2718}".red().bold(), result.message.red());
    }
    println!();

    Ok(())
}

async fn cmd_create_object(
    base: &str,
    creator: u8,
    id: u8,
    energy: u64,
    half_life: u64,
    json_mode: bool,
) -> Result<()> {
    let body = serde_json::json!({
        "creator": creator,
        "object_id": id,
        "energy": energy,
        "half_life": half_life,
    });

    let result: TxResult = api_post(base, "/api/tx/create-object", &body).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if result.success {
        println!("  {} {}", "\u{2714}".green().bold(), result.message);
    } else {
        println!("  {} {}", "\u{2718}".red().bold(), result.message.red());
    }
    println!();

    Ok(())
}

async fn cmd_refresh(base: &str, object: u8, energy: u64, json_mode: bool) -> Result<()> {
    let body = serde_json::json!({
        "object_id": object,
        "energy_deposit": energy,
    });

    let result: TxResult = api_post(base, "/api/tx/refresh", &body).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if result.success {
        println!("  {} {}", "\u{2714}".green().bold(), result.message);
    } else {
        println!("  {} {}", "\u{2718}".red().bold(), result.message.red());
    }
    println!();

    Ok(())
}

async fn cmd_resurrect(base: &str, object: u8, energy: u64, json_mode: bool) -> Result<()> {
    let body = serde_json::json!({
        "object_id": object,
        "energy_deposit": energy,
    });

    let result: TxResult = api_post(base, "/api/tx/resurrect", &body).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if result.success {
        println!("  {} {}", "\u{2714}".green().bold(), result.message);
    } else {
        println!("  {} {}", "\u{2718}".red().bold(), result.message.red());
    }
    println!();

    Ok(())
}

// ──────────────────────────── Faucet ─────────────────────────────────────

async fn cmd_faucet(base: &str, address: &str, json_mode: bool) -> Result<()> {
    let body = serde_json::json!({ "address": address });
    let result: serde_json::Value = api_post(base, "/api/faucet", &body).await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
        let amount = result.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
        let balance = result.get("new_balance").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("  {} Received {} EVAP (new balance: {})", "\u{2714}".green().bold(), amount, balance);
    } else {
        let msg = result.get("message").and_then(|v| v.as_str()).unwrap_or("failed");
        println!("  {} {}", "\u{2718}".red().bold(), msg.red());
    }
    Ok(())
}

// ──────────────────────────── Consensus ──────────────────────────────────

async fn cmd_consensus(base: &str, json_mode: bool) -> Result<()> {
    let status: StatusResponse = api_get(base, "/api/status").await?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    println!("  {} Consensus Status", "".bold());
    println!("  Block Height:  {}", status.block_height.to_string().cyan().bold());
    println!("  Epoch:         {}", status.epoch.to_string().cyan());
    println!("  State Root:    {}", &status.state_root[..16].dimmed());
    println!("  Peers:         {}", status.peer_count.to_string().yellow());
    println!("  Proving:       {}", if status.proving_enabled { "Nova IVC".green() } else { "Mock".dimmed() });
    println!();

    Ok(())
}

// ──────────────────────────── Devnet ─────────────────────────────────────

async fn cmd_devnet(validators: u32, demo: bool) -> Result<()> {
    println!("  {} Launching EvaporChain devnet with {} validators...", "".bold(), validators);

    let binary = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("evaporchain"))
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("evaporchain-node");

    if !binary.exists() {
        println!("  {} evaporchain-node binary not found at {:?}", "\u{2718}".red().bold(), binary);
        println!("  Build it first: cargo build --release -p evaporchain-node");
        return Ok(());
    }

    let mut children = Vec::new();

    for vid in 1..=validators {
        let api_port = 8079 + vid as u16;
        let p2p_port = 8999 + vid as u16;
        let data_dir = format!("/tmp/evaporchain-devnet-v{}", vid);

        // Clean previous data
        let _ = std::fs::remove_dir_all(&data_dir);

        let mut cmd = std::process::Command::new(&binary);
        cmd.args([
            "--tendermint", "--network", "--api",
            "--validator-id", &vid.to_string(),
            "--validators", &validators.to_string(),
            "--node-id", &format!("node-{}", vid),
            "--port", &p2p_port.to_string(),
            "--api-port", &api_port.to_string(),
            "--data-dir", &data_dir,
            "--startup-delay", "3000",
        ]);

        if demo {
            cmd.arg("--demo");
        }

        // Add bootstrap peers (all other validators)
        for other in 1..=validators {
            if other != vid {
                cmd.args(["--bootstrap", &format!("/ip4/127.0.0.1/tcp/{}", 8999 + other as u16)]);
            }
        }

        println!("  Starting validator {} (API :{}, P2P :{})", vid, api_port, p2p_port);
        let child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context(format!("Failed to spawn validator {}", vid))?;
        children.push((vid, child));
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    println!();
    println!("  {} All {} validators launched!", "\u{2714}".green().bold(), validators);
    println!();
    for vid in 1..=validators {
        println!("  Validator {}: http://localhost:{}", vid, 8079 + vid as u16);
    }
    println!();
    println!("  Press Ctrl+C to stop all validators");

    // Wait for signal
    tokio::signal::ctrl_c().await?;

    println!("\n  Stopping validators...");
    for (vid, mut child) in children {
        let _ = child.kill();
        println!("  Stopped validator {}", vid);
    }

    Ok(())
}

// ──────────────────────────── Main ───────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let base = cli.api_url.trim_end_matches('/');

    let result = match cli.command {
        Commands::Status => cmd_status(base, cli.json).await,
        Commands::Objects => cmd_objects(base, cli.json).await,
        Commands::Accounts => cmd_accounts(base, cli.json).await,
        Commands::Blocks { limit } => cmd_blocks(base, limit, cli.json).await,
        Commands::Transfer {
            from,
            to,
            amount,
            nonce,
        } => cmd_transfer(base, from, to, amount, nonce, cli.json).await,
        Commands::CreateObject {
            creator,
            id,
            energy,
            half_life,
        } => cmd_create_object(base, creator, id, energy, half_life, cli.json).await,
        Commands::Refresh { object, energy } => cmd_refresh(base, object, energy, cli.json).await,
        Commands::Resurrect { object, energy } => {
            cmd_resurrect(base, object, energy, cli.json).await
        }
        Commands::Faucet { address } => {
            cmd_faucet(base, &address, cli.json).await
        }
        Commands::Consensus => {
            cmd_consensus(base, cli.json).await
        }
        Commands::Devnet { validators, demo } => {
            cmd_devnet(validators, demo).await
        }
    };

    if let Err(e) = result {
        eprintln!(
            "  {} {}",
            "\u{2718}".red().bold(),
            format!("Error: {}", e).red()
        );
        std::process::exit(1);
    }

    Ok(())
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_parses_status() {
        let cli = Cli::parse_from(["evaporchain", "status"]);
        assert!(matches!(cli.command, Commands::Status));
        assert_eq!(cli.api_url, "http://localhost:8080");
        assert!(!cli.json);
    }

    #[test]
    fn test_cli_parses_objects() {
        let cli = Cli::parse_from(["evaporchain", "objects"]);
        assert!(matches!(cli.command, Commands::Objects));
    }

    #[test]
    fn test_cli_parses_accounts() {
        let cli = Cli::parse_from(["evaporchain", "accounts"]);
        assert!(matches!(cli.command, Commands::Accounts));
    }

    #[test]
    fn test_cli_parses_blocks_default_limit() {
        let cli = Cli::parse_from(["evaporchain", "blocks"]);
        if let Commands::Blocks { limit } = cli.command {
            assert_eq!(limit, 10);
        } else {
            panic!("Expected Blocks command");
        }
    }

    #[test]
    fn test_cli_parses_blocks_custom_limit() {
        let cli = Cli::parse_from(["evaporchain", "blocks", "--limit", "25"]);
        if let Commands::Blocks { limit } = cli.command {
            assert_eq!(limit, 25);
        } else {
            panic!("Expected Blocks command");
        }
    }

    #[test]
    fn test_cli_parses_transfer() {
        let cli = Cli::parse_from([
            "evaporchain", "transfer", "--from", "1", "--to", "2", "--amount", "500",
        ]);
        if let Commands::Transfer {
            from,
            to,
            amount,
            nonce,
        } = cli.command
        {
            assert_eq!(from, 1);
            assert_eq!(to, 2);
            assert_eq!(amount, 500);
            assert_eq!(nonce, 0); // default
        } else {
            panic!("Expected Transfer command");
        }
    }

    #[test]
    fn test_cli_parses_transfer_with_nonce() {
        let cli = Cli::parse_from([
            "evaporchain",
            "transfer",
            "--from",
            "1",
            "--to",
            "3",
            "--amount",
            "100",
            "--nonce",
            "5",
        ]);
        if let Commands::Transfer { nonce, .. } = cli.command {
            assert_eq!(nonce, 5);
        } else {
            panic!("Expected Transfer command");
        }
    }

    #[test]
    fn test_cli_parses_create_object() {
        let cli = Cli::parse_from([
            "evaporchain",
            "create-object",
            "--creator",
            "1",
            "--id",
            "50",
            "--energy",
            "1000",
            "--half-life",
            "10",
        ]);
        if let Commands::CreateObject {
            creator,
            id,
            energy,
            half_life,
        } = cli.command
        {
            assert_eq!(creator, 1);
            assert_eq!(id, 50);
            assert_eq!(energy, 1000);
            assert_eq!(half_life, 10);
        } else {
            panic!("Expected CreateObject command");
        }
    }

    #[test]
    fn test_cli_parses_refresh() {
        let cli = Cli::parse_from([
            "evaporchain",
            "refresh",
            "--object",
            "10",
            "--energy",
            "200",
        ]);
        if let Commands::Refresh { object, energy } = cli.command {
            assert_eq!(object, 10);
            assert_eq!(energy, 200);
        } else {
            panic!("Expected Refresh command");
        }
    }

    #[test]
    fn test_cli_parses_resurrect() {
        let cli = Cli::parse_from([
            "evaporchain",
            "resurrect",
            "--object",
            "10",
            "--energy",
            "500",
        ]);
        if let Commands::Resurrect { object, energy } = cli.command {
            assert_eq!(object, 10);
            assert_eq!(energy, 500);
        } else {
            panic!("Expected Resurrect command");
        }
    }

    #[test]
    fn test_cli_parses_json_flag() {
        let cli = Cli::parse_from(["evaporchain", "--json", "status"]);
        assert!(cli.json);
        assert!(matches!(cli.command, Commands::Status));
    }

    #[test]
    fn test_cli_parses_api_url() {
        let cli = Cli::parse_from([
            "evaporchain",
            "--api-url",
            "http://localhost:3333",
            "status",
        ]);
        assert_eq!(cli.api_url, "http://localhost:3333");
    }

    #[test]
    fn test_cli_rejects_missing_transfer_args() {
        let result = Cli::try_parse_from(["evaporchain", "transfer", "--from", "1"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_rejects_unknown_command() {
        let result = Cli::try_parse_from(["evaporchain", "foobar"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_help_does_not_panic() {
        // Verify the command structure is valid
        Cli::command().debug_assert();
    }

    #[test]
    fn test_energy_bar_full() {
        let bar = energy_bar(100, 100, 10);
        // Should contain full block characters
        assert!(bar.contains('\u{2588}'));
    }

    #[test]
    fn test_energy_bar_empty() {
        let bar = energy_bar(0, 100, 10);
        // Should contain light shade characters (or be mostly empty)
        assert!(bar.contains('\u{2591}'));
    }

    #[test]
    fn test_energy_bar_zero_max() {
        let bar = energy_bar(0, 0, 10);
        assert_eq!(bar.len(), 10); // Just spaces
    }

    #[test]
    fn test_format_uptime() {
        assert_eq!(format_uptime(30), "30s");
        assert_eq!(format_uptime(90), "1m 30s");
        assert_eq!(format_uptime(3661), "1h 1m");
    }
}
