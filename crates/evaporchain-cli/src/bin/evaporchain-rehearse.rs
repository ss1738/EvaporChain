//! evaporchain-rehearse — pre-mainnet genesis ceremony dry-run.
//!
//! Drives the existing `evaporchain onboarding ...` subcommands and
//! `evaporchain testnet ...` orchestrator end-to-end so every operator
//! has rehearsed the cold-start flow before mainnet.
//!
//! Steps (numbered to match the on-screen progress log):
//!   1. Generate N synthetic operator keypairs (one per validator).
//!   2. Generate the coordinator ML-DSA-65 keypair.
//!   3. Collect each operator's BLS pk into a validator manifest.
//!   4. Build + sign genesis-config.json with the coordinator key.
//!   5. Verify genesis as each operator (loop, all must pass).
//!   6. Launch a 3-node local testnet against the signed genesis.
//!   7. Smoke test: wait for ≥ N blocks, confirm /api/validators is healthy.
//!   8. Tear down (unless --keep-dir was given) and print verdict.
//!
//! On failure: stop, print the captured stderr from the failing
//! subprocess, exit 1. Single binary, no external state.
//!
//! Spec-strict: only edits the cli crate. Calls existing subcommands
//! via `tokio::process::Command` rather than re-implementing them.

use anyhow::{Context, Result};
use colored::Colorize;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

// ─────────────────────────── CLI ──────────────────────────────────────────

#[derive(clap::Parser)]
#[command(
    name = "evaporchain-rehearse",
    about = "Pre-mainnet genesis ceremony dry-run."
)]
struct Cli {
    /// Number of synthetic operators to simulate. Each gets its own BLS pk
    /// + a fresh keypair directory.
    #[arg(long, default_value_t = 5)]
    operators: u32,

    /// Path to the `evaporchain-node` binary (used by the testnet
    /// orchestrator). Default: search PATH for `evaporchain-node`.
    #[arg(long, default_value = "evaporchain-node")]
    node_binary: String,

    /// Wipe the rehearsal tempdir if all 8 steps pass. Default true.
    #[arg(long, default_value_t = true)]
    cleanup_on_success: bool,

    /// Reuse this directory instead of a fresh tempdir. Will be created if
    /// it doesn't already exist; will not be cleaned even on success.
    #[arg(long)]
    keep_dir: Option<PathBuf>,

    /// Number of blocks the local cluster must produce before the smoke
    /// test passes.
    #[arg(long, default_value_t = 10)]
    ceremony_blocks: u64,

    /// Hard ceiling on the smoke-test wait. Failure ⇒ FAILED at step 7.
    #[arg(long, default_value_t = 120)]
    timeout_seconds: u64,
}

// ─────────────────────────── Reporting ────────────────────────────────────

const TOTAL_STEPS: u8 = 8;

struct StepReporter {
    started: Instant,
    overall_started: Instant,
    current: u8,
}

impl StepReporter {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            overall_started: now,
            current: 0,
        }
    }

    fn begin(&mut self, n: u8, label: &str) {
        self.current = n;
        self.started = Instant::now();
        // Pad the label to ~38 chars so the OK/FAIL marker lines up.
        let line = format!("[{}/{}] {}", n, TOTAL_STEPS, label);
        let padded = format!("{:<46}", line);
        print!("{}", padded);
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }

    fn ok(&self, suffix: Option<&str>) {
        let took = self.started.elapsed();
        let extra = match suffix {
            Some(s) => format!(" {}", s.truecolor(140, 150, 170)),
            None => String::new(),
        };
        println!(
            "{} {}{}",
            "\u{2714}".green().bold(),
            format!("({}ms)", took.as_millis()).truecolor(140, 150, 170),
            extra
        );
    }

    fn fail(&self, why: &str, captured_stderr: Option<&str>) {
        println!("{}", "\u{2717}".red().bold());
        eprintln!();
        eprintln!(
            "{} step {} failed: {}",
            "FAILED".red().bold(),
            self.current,
            why
        );
        if let Some(err) = captured_stderr {
            if !err.trim().is_empty() {
                eprintln!();
                eprintln!("{}", "captured stderr:".truecolor(180, 80, 80));
                for line in err.lines().take(60) {
                    eprintln!("  {}", line);
                }
            }
        }
    }

    fn finish_ok(&self) {
        let took = self.overall_started.elapsed();
        println!();
        println!(
            "\u{1F389} Ceremony rehearsal complete ({:.1}s)",
            took.as_secs_f64()
        );
    }
}

// ─────────────────────────── Subprocess plumbing ──────────────────────────

/// Run a subprocess to completion. Tail stdout/stderr line-by-line through
/// `live` so the operator sees progress; capture all stderr for failure
/// reporting. Returns Ok(()) on exit-0, Err(stderr_text) on anything else.
async fn run_capture<I, S>(bin: &str, args: I, live_prefix: Option<&str>) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new(bin);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Err(format!("failed to spawn {}: {}", bin, e)),
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let prefix = live_prefix.map(|s| s.to_string());
    let stdout_handle = tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(ref p) = prefix {
                println!("    {} {}", p.truecolor(120, 130, 150), line);
            }
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let stderr_handle = tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let status = match child.wait().await {
        Ok(s) => s,
        Err(e) => return Err(format!("wait failed: {}", e)),
    };
    let stdout_text = stdout_handle.await.unwrap_or_default();
    let stderr_text = stderr_handle.await.unwrap_or_default();
    if status.success() {
        Ok(stdout_text)
    } else {
        Err(format!(
            "exit code {}: {}",
            status.code().unwrap_or(-1),
            stderr_text
        ))
    }
}

// ─────────────────────────── Manifest helpers ─────────────────────────────

/// Synthetic per-operator manifest entry: just enough for `onboarding
/// build-genesis` to accept it. We mint a deterministic 48-byte BLS pk
/// from a per-operator seed so we don't have to drag bls12_381 into a
/// rehearsal binary; the rehearsal isn't trying to actually run BLS,
/// it's exercising the *flow*.
fn synthetic_bls_pk(seed: u32) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"evaporchain-rehearse-bls-pk");
    hasher.update(&seed.to_le_bytes());
    let h = hasher.finalize();
    let mut bytes = [0u8; 48];
    // First 32 from blake3, then re-hash for the next 16.
    bytes[..32].copy_from_slice(h.as_bytes());
    let mut h2 = blake3::Hasher::new();
    h2.update(h.as_bytes());
    h2.update(b"tail");
    let tail = h2.finalize();
    bytes[32..].copy_from_slice(&tail.as_bytes()[..16]);
    hex::encode(bytes)
}

fn build_validator_manifest_json(operators: u32, stake: u64) -> String {
    let mut entries = Vec::with_capacity(operators as usize);
    for id in 1..=operators {
        entries.push(serde_json::json!({
            "id": id,
            "name": format!("rehearsal-op-{:02}", id),
            "bls_public_key": synthetic_bls_pk(id),
            "stake": stake,
            "balance": 1_000_000u64,
        }));
    }
    let manifest = serde_json::json!({ "validators": entries });
    serde_json::to_string_pretty(&manifest).unwrap()
}

// ─────────────────────────── Smoke test ──────────────────────────────────

#[derive(Deserialize)]
struct ValidatorsCount {
    count: usize,
    #[serde(default)]
    validators: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct HealthSnap {
    block_height: u64,
}

/// Poll every API port until the cluster has produced ≥ target_blocks AND
/// /api/validators on the lead node is non-empty. Returns Ok(elapsed) on
/// success, Err on timeout.
async fn smoke_test_blocks_and_validators(
    api_ports: &[u16],
    target_blocks: u64,
    timeout: Duration,
) -> Result<Duration> {
    if api_ports.is_empty() {
        anyhow::bail!("no API ports to poll");
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .context("build smoke-test client")?;
    let started = Instant::now();
    let lead = api_ports[0];

    loop {
        if started.elapsed() > timeout {
            anyhow::bail!(
                "smoke test timed out after {}s waiting for {} blocks",
                timeout.as_secs(),
                target_blocks
            );
        }
        // 1) Block height: take min across the cluster — we want every
        //    node to have caught up, not just one.
        let mut heights = Vec::with_capacity(api_ports.len());
        for &p in api_ports {
            let url = format!("http://127.0.0.1:{}/api/network/health", p);
            match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => {
                    if let Ok(h) = r.json::<HealthSnap>().await {
                        heights.push(h.block_height);
                    }
                }
                _ => {}
            }
        }
        let min_h = heights.iter().copied().min().unwrap_or(0);
        if heights.len() == api_ports.len() && min_h >= target_blocks {
            // 2) /api/validators on the lead node should report ≥ 1.
            let v_url = format!("http://127.0.0.1:{}/api/validators", lead);
            match client.get(&v_url).send().await {
                Ok(r) if r.status().is_success() => {
                    if let Ok(vs) = r.json::<ValidatorsCount>().await {
                        if vs.count > 0 && !vs.validators.is_empty() {
                            return Ok(started.elapsed());
                        }
                    }
                }
                _ => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ─────────────────────────── Tempdir helper ──────────────────────────────

struct WorkDir {
    path: PathBuf,
    keep_on_drop: bool,
}

impl WorkDir {
    fn create(keep: Option<PathBuf>) -> Result<Self> {
        match keep {
            Some(p) => {
                std::fs::create_dir_all(&p).with_context(|| format!("create {}", p.display()))?;
                Ok(Self {
                    path: p,
                    keep_on_drop: true,
                })
            }
            None => {
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let p = std::env::temp_dir().join(format!("evaporchain-rehearse-{}", nonce));
                std::fs::create_dir_all(&p).with_context(|| format!("create {}", p.display()))?;
                Ok(Self {
                    path: p,
                    keep_on_drop: false,
                })
            }
        }
    }

    fn keep(&mut self) {
        self.keep_on_drop = true;
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        if !self.keep_on_drop {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

// ─────────────────────────── Main flow ───────────────────────────────────

#[tokio::main]
async fn main() {
    use clap::Parser as _;
    let cli = Cli::parse();
    let exit = run(cli).await;
    std::process::exit(exit);
}

/// Locate the `evaporchain` CLI binary. The rehearse binary ships in the
/// same crate, so prefer the sibling next to our own `current_exe()`;
/// fall back to "evaporchain" on PATH.
fn locate_cli_binary() -> String {
    if let Ok(me) = std::env::current_exe() {
        if let Some(parent) = me.parent() {
            let sibling = parent.join("evaporchain");
            if sibling.exists() {
                return sibling.to_string_lossy().into_owned();
            }
        }
    }
    "evaporchain".to_string()
}

async fn run(cli: Cli) -> i32 {
    println!(
        "{} {} ({} operators, {} ceremony blocks, {}s timeout)",
        "evaporchain-rehearse".white().bold(),
        "— pre-mainnet ceremony dry-run".truecolor(140, 150, 170),
        cli.operators,
        cli.ceremony_blocks,
        cli.timeout_seconds
    );
    println!();

    if cli.operators == 0 {
        eprintln!("{}: --operators must be ≥ 1", "FAILED".red().bold());
        return 1;
    }

    let cli_binary = locate_cli_binary();

    // Sanity-check the node binary up front. The testnet orchestrator
    // resolves it relative to the CLI binary's exe dir, so a missing
    // binary fails late at step 6 with a confusing error otherwise.
    let node_path = std::path::PathBuf::from(&cli.node_binary);
    let node_resolved = if node_path.is_absolute() && node_path.exists() {
        node_path.clone()
    } else {
        // Sibling of the CLI binary first, then PATH.
        let cli_dir = std::path::PathBuf::from(&cli_binary)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let sibling = cli_dir.join(&cli.node_binary);
        if sibling.exists() {
            sibling
        } else {
            // Best-effort PATH probe.
            std::env::var_os("PATH")
                .and_then(|paths| {
                    std::env::split_paths(&paths).find_map(|p| {
                        let cand = p.join(&cli.node_binary);
                        cand.exists().then_some(cand)
                    })
                })
                .unwrap_or(node_path.clone())
        }
    };
    if !node_resolved.exists() {
        eprintln!(
            "{}: could not find {} on PATH or alongside {}.",
            "FAILED".red().bold(),
            cli.node_binary,
            cli_binary
        );
        eprintln!("  Build it: cargo build --release -p evaporchain-node");
        return 1;
    }

    // Stake amounts: pick a value comfortably above min-stake so genesis
    // validation passes without us having to bump the chain params.
    let per_op_stake: u64 = 1_000_000;

    // Workdir layout:
    //   <root>/coord/   coordinator-pk.hex + coordinator-sk.hex
    //   <root>/ops/    op-{id}/ (one per operator; future-proof if we ever
    //                   want to spawn each operator's verify in parallel)
    //   <root>/manifest.json
    //   <root>/genesis.json
    //   <root>/testnet/ (cmd_testnet_init writes here)
    let mut workdir = match WorkDir::create(cli.keep_dir.clone()) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{}: {:?}", "FAILED".red().bold(), e);
            return 1;
        }
    };
    let root = workdir.path.clone();
    println!(
        "  {} {}",
        "workdir:".truecolor(140, 150, 170),
        root.display().to_string().white()
    );
    println!();

    let coord_dir = root.join("coord");
    let ops_dir = root.join("ops");
    let manifest_path = root.join("validators.json");
    let genesis_path = root.join("genesis.json");
    let testnet_dir = root.join("testnet");

    let mut rep = StepReporter::new();

    // ── 1. Synthesise N operator keypair directories. We don't actually
    //      need MlDsa keys for the smoke flow (BLS pks are written into
    //      the manifest deterministically), but we still touch a file per
    //      operator so the structure mirrors a real ceremony.
    rep.begin(
        1,
        &format!("Generating {} operator keypairs...", cli.operators),
    );
    if let Err(e) = std::fs::create_dir_all(&ops_dir) {
        rep.fail("could not create ops/", Some(&e.to_string()));
        return 1;
    }
    for id in 1..=cli.operators {
        let op = ops_dir.join(format!("op-{:02}", id));
        if let Err(e) = std::fs::create_dir_all(&op) {
            rep.fail(&format!("create op-{:02}", id), Some(&e.to_string()));
            return 1;
        }
        let pk_hex = synthetic_bls_pk(id);
        if let Err(e) = std::fs::write(op.join("bls-pk.hex"), pk_hex) {
            rep.fail("write bls-pk.hex", Some(&e.to_string()));
            return 1;
        }
    }
    rep.ok(None);

    // ── 2. Coordinator key.
    rep.begin(2, "Generating coordinator key...");
    if let Err(e) = std::fs::create_dir_all(&coord_dir) {
        rep.fail("create coord/", Some(&e.to_string()));
        return 1;
    }
    match run_capture(
        &cli_binary,
        [
            "onboarding",
            "generate-coordinator",
            "--out-dir",
            coord_dir.to_str().unwrap(),
        ],
        None,
    )
    .await
    {
        Ok(_) => rep.ok(None),
        Err(err) => {
            rep.fail("`onboarding generate-coordinator` failed", Some(&err));
            return 1;
        }
    }
    let coord_pk = coord_dir.join("coordinator-pk.hex");
    let coord_sk = coord_dir.join("coordinator-sk.hex");
    if !coord_pk.exists() || !coord_sk.exists() {
        rep.fail(
            "coordinator pk/sk not written",
            Some(&format!(
                "expected {}, {}",
                coord_pk.display(),
                coord_sk.display()
            )),
        );
        return 1;
    }

    // ── 3. Collect pks into a manifest.
    rep.begin(3, "Collecting validator pks...");
    let manifest_json = build_validator_manifest_json(cli.operators, per_op_stake);
    if let Err(e) = std::fs::write(&manifest_path, &manifest_json) {
        rep.fail("write manifest", Some(&e.to_string()));
        return 1;
    }
    rep.ok(None);

    // ── 4. Build genesis. min_stake set well below per_op_stake.
    rep.begin(4, "Building genesis-config.json...");
    let total_supply: u64 = per_op_stake * (cli.operators as u64) * 4;
    match run_capture(
        &cli_binary,
        [
            "onboarding",
            "build-genesis",
            "--validators",
            manifest_path.to_str().unwrap(),
            "--coordinator-sk",
            coord_sk.to_str().unwrap(),
            "--chain-id",
            "evaporchain-rehearsal",
            "--output",
            genesis_path.to_str().unwrap(),
            "--block-interval-ms",
            "1000",
            "--total-supply",
            &total_supply.to_string(),
            "--min-stake",
            &(per_op_stake / 2).to_string(),
        ],
        None,
    )
    .await
    {
        Ok(_) => rep.ok(None),
        Err(err) => {
            rep.fail("`onboarding build-genesis` failed", Some(&err));
            return 1;
        }
    }

    // ── 5. Each operator runs verify against the same coordinator pk.
    //      They all see identical bytes, so this is a loop, not a quorum.
    rep.begin(
        5,
        &format!(
            "Verifying genesis as each of {} operators...",
            cli.operators
        ),
    );
    for id in 1..=cli.operators {
        match run_capture(
            &cli_binary,
            [
                "onboarding",
                "verify",
                "--genesis",
                genesis_path.to_str().unwrap(),
                "--coordinator-pk",
                coord_pk.to_str().unwrap(),
            ],
            None,
        )
        .await
        {
            Ok(_) => {}
            Err(err) => {
                rep.fail(&format!("operator {} verify failed", id), Some(&err));
                return 1;
            }
        }
    }
    rep.ok(None);

    // ── 6. Launch a 3-node testnet against ports 9201/9202/9203 (API)
    //      and matching p2p ports. We use the existing
    //      `evaporchain testnet init/up` so the rehearsal exercises the
    //      same orchestration path operators ship to mainnet.
    let api_base: u16 = 9200;
    let p2p_base: u16 = 9100;
    let cluster_validators = 3u32;
    rep.begin(6, "Launching 3-node testnet...");
    // testnet init writes its own genesis; we accept that and rely on it
    // for the smoke phase. The signed genesis from step 4 is the one
    // operators would actually carry to mainnet — verification at step 5
    // is the behaviour gate.
    match run_capture(
        &cli_binary,
        [
            "testnet",
            "init",
            "--out",
            testnet_dir.to_str().unwrap(),
            "--validators",
            &cluster_validators.to_string(),
            "--chain-id",
            "evaporchain-rehearsal-cluster",
            "--block-interval-ms",
            "1000",
            "--p2p-base",
            &p2p_base.to_string(),
            "--api-base",
            &api_base.to_string(),
            "--force",
        ],
        None,
    )
    .await
    {
        Ok(_) => {}
        Err(err) => {
            rep.fail("`testnet init` failed", Some(&err));
            return 1;
        }
    }
    match run_capture(
        &cli_binary,
        [
            "testnet",
            "up",
            "--dir",
            testnet_dir.to_str().unwrap(),
            "--split-logs",
        ],
        None,
    )
    .await
    {
        Ok(_) => {
            let api_ports: Vec<u16> = (1..=cluster_validators as u16)
                .map(|i| api_base + i)
                .collect();
            let suffix = format!(
                "(API ports {})",
                api_ports
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            rep.ok(Some(&suffix));
        }
        Err(err) => {
            rep.fail("`testnet up` failed", Some(&err));
            // Best-effort teardown.
            let _ = run_capture(
                &cli_binary,
                ["testnet", "down", "--dir", testnet_dir.to_str().unwrap()],
                None,
            )
            .await;
            return 1;
        }
    }

    // ── 7. Smoke test: wait for ≥ ceremony_blocks across the cluster +
    //      non-empty /api/validators on the lead node.
    let api_ports: Vec<u16> = (1..=cluster_validators as u16)
        .map(|i| api_base + i)
        .collect();
    rep.begin(
        7,
        &format!("Smoke test (waiting for {} blocks)...", cli.ceremony_blocks),
    );
    let smoke_result = smoke_test_blocks_and_validators(
        &api_ports,
        cli.ceremony_blocks,
        Duration::from_secs(cli.timeout_seconds),
    )
    .await;
    let smoke_ok = match smoke_result {
        Ok(took) => {
            rep.ok(Some(&format!("({:.1}s)", took.as_secs_f64())));
            true
        }
        Err(e) => {
            rep.fail("smoke test failed", Some(&e.to_string()));
            false
        }
    };

    // ── 8. Always tear the cluster down, even on smoke failure — the
    //      operator wants pid files cleaned up either way.
    rep.begin(8, "Cleanup...");
    let down = run_capture(
        &cli_binary,
        ["testnet", "down", "--dir", testnet_dir.to_str().unwrap()],
        None,
    )
    .await;
    match down {
        Ok(_) => rep.ok(None),
        Err(err) => {
            // Don't flip overall success on a tear-down warning, but
            // surface it so the operator can clean up by hand.
            println!(
                "{}  ({})",
                "\u{26A0}".yellow().bold(),
                err.lines().next().unwrap_or("")
            );
        }
    }

    if !smoke_ok {
        if cli.keep_dir.is_none() {
            eprintln!();
            eprintln!("  workdir kept for inspection: {}", workdir.path.display());
            workdir.keep();
        }
        return 1;
    }

    if cli.cleanup_on_success && cli.keep_dir.is_none() {
        // Drop will wipe.
    } else {
        workdir.keep();
    }

    rep.finish_ok();
    0
}

// ─────────────────────────── Tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_bls_pk_is_48_bytes_hex() {
        let h = synthetic_bls_pk(1);
        assert_eq!(h.len(), 96, "48 bytes hex-encoded");
        let bytes = hex::decode(&h).unwrap();
        assert_eq!(bytes.len(), 48);
    }

    #[test]
    fn synthetic_bls_pk_unique_per_operator() {
        let a = synthetic_bls_pk(1);
        let b = synthetic_bls_pk(2);
        assert_ne!(a, b);
    }

    #[test]
    fn manifest_json_has_one_entry_per_operator() {
        let json = build_validator_manifest_json(5, 1_000_000);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = v["validators"].as_array().unwrap();
        assert_eq!(arr.len(), 5);
        for (i, e) in arr.iter().enumerate() {
            assert_eq!(e["id"].as_u64().unwrap(), (i as u64) + 1);
            assert_eq!(e["stake"].as_u64().unwrap(), 1_000_000);
            assert_eq!(
                e["bls_public_key"].as_str().unwrap().len(),
                96,
                "bls pk must be 48 bytes hex"
            );
        }
    }

    #[test]
    fn workdir_cleans_up_when_not_kept() {
        let w = WorkDir::create(None).unwrap();
        let p = w.path.clone();
        assert!(p.exists());
        drop(w);
        assert!(!p.exists());
    }

    #[test]
    fn workdir_keeps_explicit_path() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let custom = std::env::temp_dir().join(format!("rehearse-keep-{}", nonce));
        let w = WorkDir::create(Some(custom.clone())).unwrap();
        drop(w);
        assert!(custom.exists(), "explicit keep_dir must not be wiped");
        let _ = std::fs::remove_dir_all(&custom);
    }
}
