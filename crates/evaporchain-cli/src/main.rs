use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::*;
use serde::{Deserialize, Serialize};

use evaporchain_crypto::{BlsKeypair, MlDsaKeypair, VrfKeypair};
use evaporchain_execution::genesis::{initialize_genesis, load_genesis_config};
use evaporchain_types::genesis::GenesisConfig;

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

    /// Genesis config tools (validate, show)
    Genesis {
        #[command(subcommand)]
        action: GenesisAction,
    },

    /// Generate a validator keypair bundle (BLS + ML-DSA + VRF)
    Keygen {
        /// Output file path (default: stdout)
        #[arg(long)]
        output: Option<String>,
    },

    /// Encrypt a plaintext bls_key.bin into the EVK1 encrypted format.
    /// Use this to migrate an existing validator key without regenerating.
    /// Passphrase is read from EVAPORCHAIN_VALIDATOR_KEY_PASS or --passphrase.
    EncryptBlsKey {
        /// Path to the plaintext bls_key.bin (32 raw secret bytes)
        #[arg(long)]
        in_file: String,

        /// Where to write the EVK1 encrypted blob (92 bytes)
        #[arg(long)]
        out_file: String,

        /// Passphrase (overrides EVAPORCHAIN_VALIDATOR_KEY_PASS)
        #[arg(long)]
        passphrase: Option<String>,
    },

    /// Decrypt an EVK1-encrypted bls_key.bin back to plaintext (32 bytes).
    /// Used for key recovery / inspection. Handle the output carefully.
    DecryptBlsKey {
        /// Path to the EVK1 encrypted blob (92 bytes)
        #[arg(long)]
        in_file: String,

        /// Where to write the plaintext 32-byte secret
        #[arg(long)]
        out_file: String,

        /// Passphrase (overrides EVAPORCHAIN_VALIDATOR_KEY_PASS)
        #[arg(long)]
        passphrase: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum GenesisAction {
    /// Validate a genesis JSON config file
    Validate {
        /// Path to genesis JSON file
        #[arg()]
        path: String,
    },

    /// Show a summary of a genesis config
    Show {
        /// Path to genesis JSON file
        #[arg()]
        path: String,
    },

    /// Generate the genesis block and print its hash (offline, deterministic)
    Init {
        /// Path to genesis JSON file
        #[arg()]
        path: String,
    },

    /// Create a new genesis config file
    Create {
        /// Output path for the genesis JSON file
        #[arg()]
        output: String,

        /// Chain ID
        #[arg(long, default_value = "evaporchain-testnet-1")]
        chain_id: String,

        /// Total token supply
        #[arg(long, default_value = "10000000")]
        total_supply: u64,

        /// Block interval in milliseconds
        #[arg(long, default_value = "3000")]
        block_interval: u64,

        /// Minimum validator stake
        #[arg(long, default_value = "100")]
        min_stake: u64,
    },

    /// Add a validator to a genesis config
    AddValidator {
        /// Path to genesis JSON file
        #[arg()]
        path: String,

        /// Validator name
        #[arg(long)]
        name: String,

        /// Validator stake
        #[arg(long)]
        stake: u64,

        /// P2P address (multiaddr)
        #[arg(long)]
        p2p: Option<String>,

        /// Path to keygen JSON file (from `evaporchain keygen`)
        #[arg(long)]
        keys: Option<String>,

        /// Initial account balance for this validator
        #[arg(long, default_value = "1000000")]
        balance: u64,
    },

    /// Set the BLS public key on an existing validator entry. Use this to
    /// retrofit older genesis files that were created before the validator
    /// onboarding flow required pre-registered pubkeys (closes K-07/K-08).
    SetValidatorBls {
        /// Path to genesis JSON file
        #[arg()]
        path: String,

        /// Validator ID to update
        #[arg(long)]
        validator_id: u64,

        /// Path to a keygen JSON bundle (from `evaporchain keygen`)
        #[arg(long, conflicts_with = "bls_pk_hex")]
        keys: Option<String>,

        /// BLS public key as hex (48-byte compressed). Use this OR --keys.
        #[arg(long)]
        bls_pk_hex: Option<String>,
    },

    /// Add an account to a genesis config
    AddAccount {
        /// Path to genesis JSON file
        #[arg()]
        path: String,

        /// Account label
        #[arg(long)]
        label: String,

        /// Account balance
        #[arg(long)]
        balance: u64,

        /// Address byte (first byte, rest zeroed)
        #[arg(long)]
        address_byte: Option<u8>,
    },

    /// Finalize a genesis config: validate, compute genesis hash, freeze
    Finalize {
        /// Path to genesis JSON file
        #[arg()]
        path: String,
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

// ──────────────────────────── Genesis ───────────────────────────────────

fn load_genesis_file(path: &str) -> Result<GenesisConfig> {
    let json = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read genesis file: {}", path))?;
    load_genesis_config(&json).map_err(|e| anyhow::anyhow!("Invalid genesis config: {}", e))
}

fn cmd_genesis_validate(path: &str, json_mode: bool) -> Result<()> {
    let config = load_genesis_file(path)?;

    // Run initialize_genesis with a temp in-memory DB to validate
    let mut db = evaporchain_state::InMemoryStateDB::new();
    let result = initialize_genesis(&mut db, &config)
        .map_err(|e| anyhow::anyhow!("Genesis validation failed: {}", e))?;

    if json_mode {
        let out = serde_json::json!({
            "valid": true,
            "chain_id": config.chain_params.chain_id,
            "validators": config.validators.len(),
            "accounts": config.accounts.len(),
            "objects": config.objects.len(),
            "genesis_block_number": result.block.number,
            "state_root": hex::encode(result.state_root),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("  {} Genesis config is valid!", "\u{2714}".green().bold());
    println!();
    println!("  {}  {}", "Chain:".truecolor(140, 150, 170), config.chain_params.chain_id.white().bold());
    println!("  {}  {} validators", "Vals: ".truecolor(140, 150, 170), config.validators.len().to_string().cyan());
    println!("  {}  {} accounts", "Accts:".truecolor(140, 150, 170), config.accounts.len().to_string().cyan());
    println!("  {}  {} objects", "Objs: ".truecolor(140, 150, 170), config.objects.len().to_string().cyan());
    println!("  {}  {}", "Root: ".truecolor(140, 150, 170), hex::encode(result.state_root).truecolor(100, 110, 130));
    println!();

    Ok(())
}

fn cmd_genesis_show(path: &str, json_mode: bool) -> Result<()> {
    let config = load_genesis_file(path)?;

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&config)?);
        return Ok(());
    }

    print_header("Genesis Configuration");

    // Chain params
    println!("  {}  {}", "Chain ID:".truecolor(140, 150, 170), config.chain_params.chain_id.white().bold());
    println!("  {}  {} ms", "Block:   ".truecolor(140, 150, 170), config.chain_params.block_interval_ms);
    println!("  {}  {}", "Time:    ".truecolor(140, 150, 170), config.genesis_time.truecolor(180, 190, 200));

    // Tokenomics
    println!();
    println!("  {}", "Tokenomics".bold());
    println!("  {}  {} EVAP", "Supply:  ".truecolor(140, 150, 170), config.tokenomics.total_supply.to_string().green().bold());
    println!("  {}  {}%", "Fee Brn: ".truecolor(140, 150, 170), config.tokenomics.fee_burn_rate);
    println!("  {}  {} EVAP", "Block Rw:".truecolor(140, 150, 170), config.tokenomics.block_reward);

    // Validators
    println!();
    println!("  {} ({})", "Validators".bold(), config.validators.len());
    println!("  {}", separator());
    for v in &config.validators {
        let addr_hex = hex::encode(v.address);
        let addr_short = format!("{}...", &addr_hex[..16]);
        let bls = v.bls_public_key.as_deref().unwrap_or("none");
        let bls_short = if bls.len() > 16 { format!("{}...", &bls[..16]) } else { bls.to_string() };
        println!(
            "  V{:<3} {:<12} {:>10} EVAP  addr={}  bls={}",
            v.id,
            v.name.cyan().bold(),
            v.stake.to_string().green(),
            addr_short.truecolor(100, 110, 130),
            bls_short.truecolor(100, 110, 130),
        );
    }

    // Accounts
    println!();
    println!("  {} ({})", "Accounts".bold(), config.accounts.len());
    println!("  {}", separator());
    for a in &config.accounts {
        let addr_hex = hex::encode(a.address);
        let addr_short = format!("{}...", &addr_hex[..16]);
        println!(
            "  {:<14} {:>12} EVAP  {}",
            a.label.white().bold(),
            a.balance.to_string().green(),
            addr_short.truecolor(100, 110, 130),
        );
    }

    // Bootstrap peers
    if !config.bootstrap_peers.is_empty() {
        println!();
        println!("  {} ({})", "Bootstrap Peers".bold(), config.bootstrap_peers.len());
        for p in &config.bootstrap_peers {
            println!("  - {}", p.truecolor(140, 150, 170));
        }
    }

    println!();
    Ok(())
}

fn cmd_genesis_init(path: &str, json_mode: bool) -> Result<()> {
    let config = load_genesis_file(path)?;

    let mut db = evaporchain_state::InMemoryStateDB::new();
    let result = initialize_genesis(&mut db, &config)
        .map_err(|e| anyhow::anyhow!("Genesis initialization failed: {}", e))?;

    let block_hash = hex::encode(evaporchain_crypto::blake3_hash(
        &serde_json::to_vec(&result.block).unwrap_or_default(),
    ));

    if json_mode {
        let out = serde_json::json!({
            "genesis_block": {
                "number": result.block.number,
                "epoch": result.block.epoch,
                "state_root": hex::encode(result.state_root),
                "block_hash": block_hash,
            },
            "accounts_created": result.accounts_created,
            "objects_created": result.objects_created,
            "validators_registered": result.validators_registered,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    print_header("Genesis Block Generated");
    println!("  {}  #{}", "Block: ".truecolor(140, 150, 170), "0".cyan().bold());
    println!("  {}  {}", "Hash:  ".truecolor(140, 150, 170), block_hash.truecolor(100, 110, 130));
    println!("  {}  {}", "Root:  ".truecolor(140, 150, 170), hex::encode(result.state_root).truecolor(100, 110, 130));
    println!("  {}  {} accounts, {} validators, {} objects",
        "State: ".truecolor(140, 150, 170),
        result.accounts_created.to_string().green(),
        result.validators_registered.to_string().cyan(),
        result.objects_created.to_string().yellow(),
    );
    println!();
    println!("  All nodes must produce this exact state root to join the network.");
    println!();

    Ok(())
}

// ──────────────────────────── Genesis Ceremony ──────────────────────────

fn cmd_genesis_create(
    output: &str,
    chain_id: &str,
    total_supply: u64,
    block_interval: u64,
    min_stake: u64,
    json_mode: bool,
) -> Result<()> {
    use evaporchain_types::genesis::*;

    let config = GenesisConfig {
        chain_params: ChainParams {
            chain_id: chain_id.to_string(),
            block_interval_ms: block_interval,
            grace_period: 5,
            block_gas_limit: 500_000,
            max_tx_size: 1_048_576,
            max_txs_per_block: 10_000,
            min_validator_stake: min_stake,
            unbonding_period: 10,
        },
        tokenomics: Tokenomics {
            total_supply,
            block_reward: 10,
            reward_half_life: 100_000,
            fee_burn_rate: 0.50,
            staker_fee_share: 0.50,
            target_staking_apy: 0.05,
        },
        genesis_time: {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("{}", secs)
        },
        validators: vec![],
        accounts: vec![],
        objects: vec![],
        bootstrap_peers: vec![],
        trusted_checkpoint: None,
    };

    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(output, &json).with_context(|| format!("Failed to write {}", output))?;

    if json_mode {
        println!("{}", json);
    } else {
        println!("  {} Genesis config created at {}", "\u{2714}".green().bold(), output);
        println!("  Chain ID:     {}", chain_id.white().bold());
        println!("  Supply:       {} EVAP", total_supply.to_string().green());
        println!("  Block time:   {} ms", block_interval);
        println!("  Min stake:    {}", min_stake);
        println!();
        println!("  Next: add validators with `evaporchain genesis add-validator {}`", output);
    }
    println!();
    Ok(())
}

fn cmd_genesis_add_validator(
    path: &str,
    name: &str,
    stake: u64,
    p2p: Option<&str>,
    keys_path: Option<&str>,
    balance: u64,
    json_mode: bool,
) -> Result<()> {
    use evaporchain_types::genesis::*;

    let json = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path))?;
    let mut config: GenesisConfig = serde_json::from_str(&json)
        .with_context(|| "Failed to parse genesis config")?;

    let next_id = config.validators.iter().map(|v| v.id).max().unwrap_or(0) + 1;

    let mut addr = [0u8; 32];
    addr[0] = next_id as u8;

    let bls_pk = if let Some(kp) = keys_path {
        let kf = std::fs::read_to_string(kp)
            .with_context(|| format!("Failed to read keys file {}", kp))?;
        let bundle: serde_json::Value = serde_json::from_str(&kf)?;
        bundle.get("bls")
            .and_then(|b| b.get("public_key"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    config.validators.push(GenesisValidator {
        id: next_id,
        name: name.to_string(),
        stake,
        address: addr,
        bls_public_key: bls_pk.clone(),
        p2p_address: p2p.map(|s| s.to_string()),
    });

    config.accounts.push(GenesisAccount {
        address: addr,
        balance,
        label: format!("Validator-{}", name),
    });

    let output = serde_json::to_string_pretty(&config)?;
    std::fs::write(path, &output)?;

    if json_mode {
        let result = serde_json::json!({
            "added": { "id": next_id, "name": name, "stake": stake, "bls": bls_pk },
            "total_validators": config.validators.len(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("  {} Validator added to {}", "\u{2714}".green().bold(), path);
        println!("  ID:     {}", next_id.to_string().cyan().bold());
        println!("  Name:   {}", name.white().bold());
        println!("  Stake:  {} EVAP", stake.to_string().green());
        println!("  Addr:   {}", hex::encode(&addr[..8]).truecolor(100, 110, 130));
        if let Some(ref pk) = bls_pk {
            println!("  BLS:    {}...", &pk[..32].to_string().truecolor(100, 110, 130));
        }
        println!("  Balance: {} EVAP", balance.to_string().green());
        println!("  Total validators: {}", config.validators.len());
    }
    println!();
    Ok(())
}

fn cmd_genesis_set_validator_bls(
    path: &str,
    validator_id: u64,
    keys_path: Option<&str>,
    bls_pk_hex: Option<&str>,
    json_mode: bool,
) -> Result<()> {
    use evaporchain_types::genesis::*;

    // Resolve the BLS pubkey from either source.
    let pk_hex: String = match (keys_path, bls_pk_hex) {
        (Some(kp), None) => {
            let kf = std::fs::read_to_string(kp)
                .with_context(|| format!("Failed to read keys file {}", kp))?;
            let bundle: serde_json::Value = serde_json::from_str(&kf)?;
            bundle
                .get("bls")
                .and_then(|b| b.get("public_key"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow::anyhow!("keys file missing bls.public_key"))?
        }
        (None, Some(hex_str)) => hex_str.to_string(),
        (Some(_), Some(_)) => anyhow::bail!("pass --keys OR --bls-pk-hex, not both"),
        (None, None) => anyhow::bail!("must pass --keys or --bls-pk-hex"),
    };

    // Sanity-check the hex decodes and is the expected length.
    let pk_bytes = hex::decode(pk_hex.trim_start_matches("0x"))
        .with_context(|| "BLS pubkey is not valid hex")?;
    if pk_bytes.len() != 48 {
        anyhow::bail!(
            "BLS pubkey must be 48 bytes (compressed BLS12-381 G1), got {} bytes",
            pk_bytes.len()
        );
    }

    let json = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path))?;
    let mut config: GenesisConfig = serde_json::from_str(&json)
        .with_context(|| "Failed to parse genesis config")?;

    let (prev, new_pk) = {
        let entry = config
            .validators
            .iter_mut()
            .find(|v| v.id == validator_id)
            .ok_or_else(|| anyhow::anyhow!("validator-id {} not in genesis", validator_id))?;
        let prev = entry.bls_public_key.clone();
        entry.bls_public_key = Some(pk_hex.to_lowercase());
        (prev, entry.bls_public_key.clone().unwrap())
    };

    let output = serde_json::to_string_pretty(&config)?;
    std::fs::write(path, &output)?;

    if json_mode {
        let result = serde_json::json!({
            "validator_id": validator_id,
            "previous_bls_public_key": prev,
            "new_bls_public_key": new_pk,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "  {} validator-id={} bls_public_key updated in {}",
            "\u{2714}".green().bold(),
            validator_id,
            path
        );
        if let Some(p) = prev {
            println!("  was:  {}...", &p[..32.min(p.len())].truecolor(100, 110, 130));
        }
        println!(
            "  now:  {}...",
            &new_pk[..32].truecolor(100, 110, 130)
        );
    }
    println!();
    Ok(())
}

fn cmd_genesis_add_account(
    path: &str,
    label: &str,
    balance: u64,
    address_byte: Option<u8>,
    json_mode: bool,
) -> Result<()> {
    use evaporchain_types::genesis::*;

    let json = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path))?;
    let mut config: GenesisConfig = serde_json::from_str(&json)
        .with_context(|| "Failed to parse genesis config")?;

    let byte = address_byte.unwrap_or_else(|| {
        let max_byte = config.accounts.iter().map(|a| a.address[0]).max().unwrap_or(0);
        if max_byte < 0xFE { max_byte + 1 } else { 0xFE }
    });

    let mut addr = [0u8; 32];
    addr[0] = byte;

    config.accounts.push(GenesisAccount {
        address: addr,
        balance,
        label: label.to_string(),
    });

    let output = serde_json::to_string_pretty(&config)?;
    std::fs::write(path, &output)?;

    if json_mode {
        let result = serde_json::json!({
            "added": { "label": label, "balance": balance, "address_byte": byte },
            "total_accounts": config.accounts.len(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("  {} Account added to {}", "\u{2714}".green().bold(), path);
        println!("  Label:   {}", label.white().bold());
        println!("  Balance: {} EVAP", balance.to_string().green());
        println!("  Addr:    {}", hex::encode(&addr[..8]).truecolor(100, 110, 130));
        println!("  Total accounts: {}", config.accounts.len());
    }
    println!();
    Ok(())
}

fn cmd_genesis_finalize(path: &str, json_mode: bool) -> Result<()> {
    let config = load_genesis_file(path)?;

    if let Err(errors) = config.validate() {
        if json_mode {
            let out = serde_json::json!({ "valid": false, "errors": errors });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("  {} Genesis validation failed:", "\u{2718}".red().bold());
            for e in &errors {
                println!("    - {}", e.red());
            }
        }
        anyhow::bail!("{} validation errors", errors.len());
    }

    let mut db = evaporchain_state::InMemoryStateDB::new();
    let result = initialize_genesis(&mut db, &config)
        .map_err(|e| anyhow::anyhow!("Genesis initialization failed: {}", e))?;

    let block_bytes = serde_json::to_vec(&result.block).unwrap_or_default();
    let block_hash = evaporchain_crypto::blake3_hash(&block_bytes);
    let config_bytes = serde_json::to_vec(&config).unwrap_or_default();
    let config_hash = evaporchain_crypto::blake3_hash(&config_bytes);

    if json_mode {
        let out = serde_json::json!({
            "valid": true,
            "chain_id": config.chain_params.chain_id,
            "genesis_hash": hex::encode(block_hash),
            "config_hash": hex::encode(config_hash),
            "state_root": hex::encode(result.state_root),
            "validators": config.validators.len(),
            "accounts": config.accounts.len(),
            "total_allocated": config.accounts.iter().map(|a| a.balance).sum::<u64>(),
            "total_supply": config.tokenomics.total_supply,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_header("Genesis Finalized");
        println!("  {} Genesis is valid and ready for deployment", "\u{2714}".green().bold());
        println!();
        println!("  {}  {}", "Chain:  ".truecolor(140, 150, 170), config.chain_params.chain_id.white().bold());
        println!("  {}  {}", "Hash:   ".truecolor(140, 150, 170), hex::encode(block_hash).truecolor(100, 110, 130));
        println!("  {}  {}", "Config: ".truecolor(140, 150, 170), hex::encode(config_hash).truecolor(100, 110, 130));
        println!("  {}  {}", "Root:   ".truecolor(140, 150, 170), hex::encode(result.state_root).truecolor(100, 110, 130));
        println!();
        println!("  {}  {} validators, {} accounts",
            "State: ".truecolor(140, 150, 170),
            config.validators.len().to_string().cyan(),
            config.accounts.len().to_string().green(),
        );

        let total_alloc: u64 = config.accounts.iter().map(|a| a.balance).sum();
        let remaining = config.tokenomics.total_supply.saturating_sub(total_alloc);
        println!("  {}  {} / {} EVAP allocated ({} unallocated)",
            "Supply:".truecolor(140, 150, 170),
            total_alloc.to_string().green(),
            config.tokenomics.total_supply.to_string().white(),
            remaining.to_string().yellow(),
        );
        println!();
        println!("  All nodes must initialize with this genesis file.");
        println!("  Verify: evaporchain genesis validate {}", path);
    }
    println!();
    Ok(())
}

// ──────────────────────────── Keygen ────────────────────────────────────

fn cmd_keygen(output: Option<&str>, json_mode: bool) -> Result<()> {
    let bls = BlsKeypair::generate();
    let mldsa = MlDsaKeypair::generate();
    let vrf = VrfKeypair::generate();

    let bundle = serde_json::json!({
        "bls": {
            "public_key": hex::encode(&bls.public_key_bytes().0),
            "secret_key": hex::encode(&bls.secret_key_bytes().0),
        },
        "ml_dsa": {
            "public_key": hex::encode(mldsa.public_key()),
            "secret_key": hex::encode(mldsa.secret_key()),
        },
        "vrf": {
            "public_key": hex::encode(vrf.public_key_bytes()),
        },
    });

    let pretty = serde_json::to_string_pretty(&bundle)?;

    if let Some(path) = output {
        std::fs::write(path, &pretty)
            .with_context(|| format!("Failed to write keypair to {}", path))?;
        // Restrict file permissions to owner-only (0600) to protect secret keys
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("Failed to set permissions on {}", path))?;
        }

        if !json_mode {
            println!("  {} Validator keypair written to {}", "\u{2714}".green().bold(), path);
            println!();
            println!("  {}  {}", "BLS pk:".truecolor(140, 150, 170),
                     hex::encode(&bls.public_key_bytes().0)[..32].truecolor(100, 110, 130));
            println!("  {}  {}...", "ML-DSA:".truecolor(140, 150, 170),
                     hex::encode(&mldsa.public_key()[..16]).truecolor(100, 110, 130));
            println!("  {}  {}...", "VRF pk:".truecolor(140, 150, 170),
                     hex::encode(&vrf.public_key_bytes()[..16]).truecolor(100, 110, 130));
            println!();
            println!("  {} Keep the secret keys safe!", "\u{26A0}".yellow().bold());
        } else {
            println!("{}", pretty);
        }
    } else {
        println!("{}", pretty);
    }

    println!();
    Ok(())
}

fn resolve_passphrase(arg: Option<&str>) -> Result<Vec<u8>> {
    if let Some(p) = arg {
        if p.is_empty() {
            anyhow::bail!("--passphrase value is empty");
        }
        return Ok(p.as_bytes().to_vec());
    }
    match evaporchain_crypto::bls_key_store::passphrase_from_env() {
        Some(p) => Ok(p),
        None => anyhow::bail!(
            "no passphrase: pass --passphrase or set {}",
            evaporchain_crypto::bls_key_store::ENV_PASSPHRASE
        ),
    }
}

fn write_secret_file_0600(path: &str, data: &[u8]) -> Result<()> {
    std::fs::write(path, data)
        .with_context(|| format!("Failed to write {}", path))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set 0600 on {}", path))?;
    }
    Ok(())
}

fn cmd_encrypt_bls_key(in_file: &str, out_file: &str, passphrase: Option<&str>) -> Result<()> {
    let pass = resolve_passphrase(passphrase)?;
    let secret = std::fs::read(in_file)
        .with_context(|| format!("Failed to read {}", in_file))?;
    if secret.len() != 32 {
        anyhow::bail!(
            "expected 32 plaintext BLS secret bytes in {}, got {}",
            in_file,
            secret.len()
        );
    }
    let blob = evaporchain_crypto::bls_key_store::encrypt_bls_secret(&secret, &pass)
        .map_err(|e| anyhow::anyhow!("encrypt failed: {}", e))?;
    write_secret_file_0600(out_file, &blob)?;
    println!(
        "  {} Wrote {} encrypted bytes (EVK1) to {}",
        "\u{2714}".green().bold(),
        blob.len(),
        out_file
    );
    println!(
        "  {} Original plaintext at {} is unchanged — delete it once you've verified the encrypted file works.",
        "\u{26A0}".yellow().bold(),
        in_file
    );
    Ok(())
}

fn cmd_decrypt_bls_key(in_file: &str, out_file: &str, passphrase: Option<&str>) -> Result<()> {
    let pass = resolve_passphrase(passphrase)?;
    let blob = std::fs::read(in_file)
        .with_context(|| format!("Failed to read {}", in_file))?;
    let plaintext = evaporchain_crypto::bls_key_store::decrypt_bls_secret(&blob, &pass)
        .map_err(|e| anyhow::anyhow!("decrypt failed: {}", e))?;
    write_secret_file_0600(out_file, &plaintext)?;
    println!(
        "  {} Wrote 32-byte plaintext BLS secret to {}",
        "\u{2714}".green().bold(),
        out_file
    );
    println!(
        "  {} Plaintext keys are recoverable from disk — handle this file carefully.",
        "\u{26A0}".yellow().bold()
    );
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
        Commands::Genesis { action } => {
            match action {
                GenesisAction::Validate { path } => cmd_genesis_validate(&path, cli.json),
                GenesisAction::Show { path } => cmd_genesis_show(&path, cli.json),
                GenesisAction::Init { path } => cmd_genesis_init(&path, cli.json),
                GenesisAction::Create { output, chain_id, total_supply, block_interval, min_stake } => {
                    cmd_genesis_create(&output, &chain_id, total_supply, block_interval, min_stake, cli.json)
                }
                GenesisAction::AddValidator { path, name, stake, p2p, keys, balance } => {
                    cmd_genesis_add_validator(&path, &name, stake, p2p.as_deref(), keys.as_deref(), balance, cli.json)
                }
                GenesisAction::SetValidatorBls { path, validator_id, keys, bls_pk_hex } => {
                    cmd_genesis_set_validator_bls(&path, validator_id, keys.as_deref(), bls_pk_hex.as_deref(), cli.json)
                }
                GenesisAction::AddAccount { path, label, balance, address_byte } => {
                    cmd_genesis_add_account(&path, &label, balance, address_byte, cli.json)
                }
                GenesisAction::Finalize { path } => {
                    cmd_genesis_finalize(&path, cli.json)
                }
            }
        }
        Commands::Keygen { output } => {
            cmd_keygen(output.as_deref(), cli.json)
        }
        Commands::EncryptBlsKey {
            in_file,
            out_file,
            passphrase,
        } => cmd_encrypt_bls_key(&in_file, &out_file, passphrase.as_deref()),
        Commands::DecryptBlsKey {
            in_file,
            out_file,
            passphrase,
        } => cmd_decrypt_bls_key(&in_file, &out_file, passphrase.as_deref()),
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
    fn test_cli_parses_genesis_validate() {
        let cli = Cli::parse_from(["evaporchain", "genesis", "validate", "genesis.json"]);
        if let Commands::Genesis { action: GenesisAction::Validate { path } } = cli.command {
            assert_eq!(path, "genesis.json");
        } else {
            panic!("Expected Genesis Validate command");
        }
    }

    #[test]
    fn test_cli_parses_genesis_show() {
        let cli = Cli::parse_from(["evaporchain", "genesis", "show", "genesis.json"]);
        if let Commands::Genesis { action: GenesisAction::Show { path } } = cli.command {
            assert_eq!(path, "genesis.json");
        } else {
            panic!("Expected Genesis Show command");
        }
    }

    #[test]
    fn test_cli_parses_genesis_init() {
        let cli = Cli::parse_from(["evaporchain", "genesis", "init", "genesis.json"]);
        if let Commands::Genesis { action: GenesisAction::Init { path } } = cli.command {
            assert_eq!(path, "genesis.json");
        } else {
            panic!("Expected Genesis Init command");
        }
    }

    #[test]
    fn test_genesis_init_deterministic() {
        let genesis_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../genesis-mainnet.json");
        if std::path::Path::new(genesis_path).exists() {
            // Run twice — must produce the same state root
            let result1 = {
                let json = std::fs::read_to_string(genesis_path).unwrap();
                let config = load_genesis_config(&json).unwrap();
                let mut db = evaporchain_state::InMemoryStateDB::new();
                initialize_genesis(&mut db, &config).unwrap()
            };
            let result2 = {
                let json = std::fs::read_to_string(genesis_path).unwrap();
                let config = load_genesis_config(&json).unwrap();
                let mut db = evaporchain_state::InMemoryStateDB::new();
                initialize_genesis(&mut db, &config).unwrap()
            };
            assert_eq!(result1.state_root, result2.state_root, "Genesis must be deterministic");
        }
    }

    #[test]
    fn test_cli_parses_keygen() {
        let cli = Cli::parse_from(["evaporchain", "keygen"]);
        if let Commands::Keygen { output } = cli.command {
            assert!(output.is_none());
        } else {
            panic!("Expected Keygen command");
        }
    }

    #[test]
    fn test_cli_parses_keygen_with_output() {
        let cli = Cli::parse_from(["evaporchain", "keygen", "--output", "keys.json"]);
        if let Commands::Keygen { output } = cli.command {
            assert_eq!(output.as_deref(), Some("keys.json"));
        } else {
            panic!("Expected Keygen command");
        }
    }

    #[test]
    fn test_genesis_validate_with_mainnet_config() {
        let genesis_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../genesis-mainnet.json");
        if std::path::Path::new(genesis_path).exists() {
            let result = cmd_genesis_validate(genesis_path, true);
            assert!(result.is_ok(), "Genesis validation failed: {:?}", result.err());
        }
    }

    #[test]
    fn test_genesis_validate_missing_file() {
        let result = cmd_genesis_validate("/nonexistent/path.json", true);
        assert!(result.is_err());
    }

    #[test]
    fn test_keygen_generates_valid_bundle() {
        let tmp = std::env::temp_dir().join("evaporchain-test-keygen.json");
        let result = cmd_keygen(Some(tmp.to_str().unwrap()), true);
        assert!(result.is_ok());

        let contents = std::fs::read_to_string(&tmp).unwrap();
        let bundle: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert!(bundle.get("bls").is_some());
        assert!(bundle.get("ml_dsa").is_some());
        assert!(bundle.get("vrf").is_some());

        // BLS public key should be 96 hex chars (48 bytes)
        let bls_pk = bundle["bls"]["public_key"].as_str().unwrap();
        assert_eq!(bls_pk.len(), 96);

        let _ = std::fs::remove_file(&tmp);
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

    // ─── Genesis Ceremony Tests ──────────────────────────────────────

    #[test]
    fn test_cli_parses_genesis_create() {
        let cli = Cli::parse_from([
            "evaporchain", "genesis", "create", "out.json",
            "--chain-id", "my-testnet",
            "--total-supply", "5000000",
        ]);
        if let Commands::Genesis { action: GenesisAction::Create { output, chain_id, total_supply, .. } } = cli.command {
            assert_eq!(output, "out.json");
            assert_eq!(chain_id, "my-testnet");
            assert_eq!(total_supply, 5000000);
        } else {
            panic!("Expected Genesis Create command");
        }
    }

    #[test]
    fn test_cli_parses_genesis_add_validator() {
        let cli = Cli::parse_from([
            "evaporchain", "genesis", "add-validator", "genesis.json",
            "--name", "node1", "--stake", "1000",
        ]);
        if let Commands::Genesis { action: GenesisAction::AddValidator { path, name, stake, .. } } = cli.command {
            assert_eq!(path, "genesis.json");
            assert_eq!(name, "node1");
            assert_eq!(stake, 1000);
        } else {
            panic!("Expected Genesis AddValidator command");
        }
    }

    #[test]
    fn test_cli_parses_genesis_add_account() {
        let cli = Cli::parse_from([
            "evaporchain", "genesis", "add-account", "genesis.json",
            "--label", "Faucet", "--balance", "5000000",
        ]);
        if let Commands::Genesis { action: GenesisAction::AddAccount { path, label, balance, .. } } = cli.command {
            assert_eq!(path, "genesis.json");
            assert_eq!(label, "Faucet");
            assert_eq!(balance, 5000000);
        } else {
            panic!("Expected Genesis AddAccount command");
        }
    }

    #[test]
    fn test_cli_parses_genesis_finalize() {
        let cli = Cli::parse_from(["evaporchain", "genesis", "finalize", "genesis.json"]);
        if let Commands::Genesis { action: GenesisAction::Finalize { path } } = cli.command {
            assert_eq!(path, "genesis.json");
        } else {
            panic!("Expected Genesis Finalize command");
        }
    }

    #[test]
    fn test_genesis_ceremony_full_flow() {
        let dir = std::env::temp_dir().join("evaporchain-ceremony-test");
        let _ = std::fs::create_dir_all(&dir);
        let genesis_path = dir.join("genesis.json");
        let path_str = genesis_path.to_str().unwrap();

        // Step 1: Create
        let result = cmd_genesis_create(path_str, "test-chain", 10_000_000, 3000, 100, true);
        assert!(result.is_ok(), "Create failed: {:?}", result.err());
        assert!(genesis_path.exists());

        // Step 2: Add validators
        let result = cmd_genesis_add_validator(path_str, "alpha", 1000, Some("/ip4/127.0.0.1/tcp/9000"), None, 1_000_000, true);
        assert!(result.is_ok(), "Add validator 1 failed: {:?}", result.err());

        let result = cmd_genesis_add_validator(path_str, "beta", 1000, Some("/ip4/127.0.0.1/tcp/9001"), None, 1_000_000, true);
        assert!(result.is_ok(), "Add validator 2 failed: {:?}", result.err());

        // Step 3: Add faucet account
        let result = cmd_genesis_add_account(path_str, "Faucet", 5_000_000, Some(0xFF), true);
        assert!(result.is_ok(), "Add account failed: {:?}", result.err());

        // Step 4: Finalize
        let result = cmd_genesis_finalize(path_str, true);
        assert!(result.is_ok(), "Finalize failed: {:?}", result.err());

        // Verify the config
        let json = std::fs::read_to_string(&genesis_path).unwrap();
        let config: GenesisConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.validators.len(), 2);
        assert_eq!(config.accounts.len(), 3); // 2 validator accounts + 1 faucet
        assert_eq!(config.chain_params.chain_id, "test-chain");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
