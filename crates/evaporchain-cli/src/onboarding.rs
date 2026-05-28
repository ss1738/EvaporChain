//! Multi-validator onboarding flow (closes audit K-07/K-08, mainnet P0).
//! Produces a single coordinator-signed `genesis-config.json` every
//! validator passes to its node via `--genesis-config <path>`, plus a
//! verify command operators run before launch. See
//! `docs/VALIDATOR_ONBOARDING.md` for the full runbook.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::Zeroize;

use evaporchain_crypto::{
    signatures::{Signer as _, Verifier as _},
    MlDsaKeypair, MlDsaVerifier,
};
use evaporchain_types::genesis::{
    ChainParams, GenesisAccount, GenesisConfig, GenesisValidator, Tokenomics,
};

// ─────────────────────────── Validator manifest ───────────────────────────

/// On-disk shape of `--validators <path>`. Operators submit one of these to
/// the coordinator (one entry per validator) before the ceremony.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorEntry {
    /// Validator id (1-based, must be unique across the manifest).
    pub id: u64,
    /// Human-readable moniker (logs / dashboards only).
    pub name: String,
    /// Validator's BLS12-381 G1 compressed public key (hex, 96 chars).
    pub bls_public_key: String,
    /// Validator's BLS12-381 proof-of-possession (hex). Defeats
    /// rogue-key attacks: the genesis-loader verifies this signature
    /// against `bls_public_key` before marking the validator
    /// `pop_verified`. Manifests without this field load with
    /// `pop_verified=false`. Audit-flagged 2026-04-27 §2; closed
    /// 2026-05-02.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bls_pop: String,
    /// Stake amount at genesis (must be >= chain_params.min_validator_stake).
    pub stake: u64,
    /// Optional initial token allocation paid to this validator's address.
    #[serde(default)]
    pub balance: u64,
    /// Optional libp2p multiaddress for bootstrap peers (legacy field name).
    /// Prefer `multiaddr` going forward; both are honoured by `build-genesis`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p2p_address: Option<String>,
    /// Operator-supplied libp2p multiaddr including `/p2p/<peer_id>` tail
    /// (e.g. `/ip4/100.119.53.101/tcp/19001/p2p/12D3Koo...`). When present
    /// it is folded into the signed `genesis.bootstrap_peers` so every
    /// node can dial every other node by both location AND identity.
    /// Generate one with `evaporchain onboarding generate-network-key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiaddr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorManifest {
    pub validators: Vec<ValidatorEntry>,
    /// Optional non-validator allocations (foundation, treasury, etc.).
    #[serde(default)]
    pub allocations: Vec<AllocationEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationEntry {
    /// 32-byte address as hex (with or without 0x prefix).
    pub address: String,
    /// Initial balance.
    pub balance: u64,
    /// Optional human label.
    #[serde(default)]
    pub label: String,
}

// ─────────────────────────── Helpers ──────────────────────────────────────

fn write_secret_0600(path: &Path, data: &[u8]) -> Result<()> {
    std::fs::write(path, data).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 {}", path.display()))?;
    }
    Ok(())
}

fn parse_hex_strict(s: &str, expected: usize, what: &str) -> Result<Vec<u8>> {
    let trimmed = s.trim().trim_start_matches("0x");
    let bytes = hex::decode(trimmed).with_context(|| format!("{} is not valid hex", what))?;
    if bytes.len() != expected {
        anyhow::bail!("{} must be {} bytes, got {}", what, expected, bytes.len());
    }
    Ok(bytes)
}

fn address_from_hex(s: &str) -> Result<[u8; 32]> {
    let bytes = parse_hex_strict(s, 32, "address")?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ─────────────────────────── generate-coordinator ─────────────────────────

pub fn cmd_generate_coordinator(out_dir: &Path) -> Result<()> {
    // Crypto-TOCTOU (re-audit 2026-05-02): drop the exists() check.
    // `create_dir_all` is idempotent (no-op if the dir already exists)
    // and atomic for the create-vs-symlink-swap window. Previous
    // exists()-then-create pattern let an attacker on the same host
    // replace `out_dir` with a symlink between the two calls, then
    // we'd write keys into the target.
    std::fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    let kp = MlDsaKeypair::generate();
    let pk_hex = hex::encode(kp.public_key());
    let sk_hex = hex::encode(kp.secret_key());

    let pk_path = out_dir.join("coordinator-pk.hex");
    let sk_path = out_dir.join("coordinator-sk.hex");

    std::fs::write(&pk_path, &pk_hex).with_context(|| format!("write {}", pk_path.display()))?;
    write_secret_0600(&sk_path, sk_hex.as_bytes())?;

    println!("Coordinator ML-DSA-65 keypair written:");
    println!("  pk:  {}", pk_path.display());
    println!("  sk:  {} (0600)", sk_path.display());
    println!();
    println!("Distribute coordinator-pk.hex to every validator out-of-band.");
    println!("Keep coordinator-sk.hex offline; it only ever runs `onboarding build-genesis`.");
    Ok(())
}

// ─────────────────────────── generate-network-key ────────────────────────

/// Generate a libp2p ed25519 identity for one validator and persist it as
/// `<out_dir>/network_key.bin` (mode 0600). Prints the PeerId and the
/// fully-formed multiaddr template so the operator can copy/paste both
/// the file (onto their validator host's `<data-dir>/network_key.bin`)
/// and the multiaddr (into the coordinator's manifest).
pub fn cmd_generate_network_key(out_dir: &Path, listen_ip: &str, port: u16) -> Result<()> {
    // Crypto-TOCTOU: idempotent create, no exists()-then-create window.
    std::fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    // Reuse the network crate's loader/generator so the on-disk format is
    // exactly what `P2pNetworkService::start` will read at node startup.
    let kp = evaporchain_network::load_or_generate_identity(out_dir)
        .with_context(|| format!("generate identity at {}", out_dir.display()))?;
    let peer_id = kp.public().to_peer_id();
    let multiaddr = format!("/ip4/{}/tcp/{}/p2p/{}", listen_ip, port, peer_id);

    let key_path = out_dir.join("network_key.bin");
    println!("libp2p identity written:");
    println!("  network_key.bin: {} (0600)", key_path.display());
    println!("  PeerId:          {}", peer_id);
    println!("  multiaddr:       {}", multiaddr);
    println!();
    println!("Add this to your coordinator's validator manifest:");
    println!(
        "  {{ \"id\": <vid>, ..., \"multiaddr\": \"{}\" }}",
        multiaddr
    );
    Ok(())
}

// ─────────────────────────── build-genesis ────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn cmd_build_genesis(
    validators_path: &Path,
    coordinator_sk_path: &Path,
    chain_id: &str,
    output_path: &Path,
    block_interval_ms: u64,
    total_supply: u64,
    min_stake: u64,
) -> Result<()> {
    // Load and decode the coordinator's secret key.
    let sk_hex = std::fs::read_to_string(coordinator_sk_path)
        .with_context(|| format!("read {}", coordinator_sk_path.display()))?;
    let sk_bytes = parse_hex_strict(&sk_hex, 4000, "coordinator secret key")?;

    // We only have the secret. Derive the keypair via the matching pk hint
    // file (coordinator-pk.hex next to it) when present, otherwise we'd be
    // unable to construct an MlDsaKeypair (no sk-only API). Operators should
    // keep them paired; we look for `<sk>.pub` or `coordinator-pk.hex` in the
    // same directory.
    let pk_hex_str = find_paired_pk(coordinator_sk_path)?;
    let pk_bytes = parse_hex_strict(&pk_hex_str, 1952, "coordinator public key")?;
    let kp = MlDsaKeypair::from_bytes(&pk_bytes, &sk_bytes)
        .map_err(|e| anyhow::anyhow!("coordinator keypair invalid: {}", e))?;

    // Load the validator manifest.
    let manifest_json = std::fs::read_to_string(validators_path)
        .with_context(|| format!("read {}", validators_path.display()))?;
    let manifest: ValidatorManifest =
        serde_json::from_str(&manifest_json).context("parse validator manifest")?;
    if manifest.validators.is_empty() {
        anyhow::bail!("validator manifest must list at least one validator");
    }

    // Build validator + account entries. Validator address is `id` in byte 0
    // (matches the existing genesis ceremony convention in cmd_genesis_add_validator).
    let mut validators = Vec::with_capacity(manifest.validators.len());
    let mut accounts = Vec::new();
    let mut bootstrap_peers: Vec<String> = Vec::new();
    for v in &manifest.validators {
        // Sanity: BLS pk is 48 bytes (96 hex chars).
        let _ = parse_hex_strict(&v.bls_public_key, 48, "validator bls_public_key")?;
        if v.id == 0 || v.id > 0xFF {
            anyhow::bail!(
                "validator id {} must be 1..=255 (encoded in address byte 0)",
                v.id
            );
        }
        let mut addr = [0u8; 32];
        addr[0] = v.id as u8;
        // Prefer the new explicit `multiaddr` field; fall back to the legacy
        // `p2p_address` for older manifests. Either way, an entry that
        // includes `/p2p/<peer_id>` is folded into `bootstrap_peers` so
        // every node ships with the canonical dialable address+identity
        // of every other validator at startup.
        let chosen_ma = v.multiaddr.clone().or_else(|| v.p2p_address.clone());
        if let Some(ref ma) = chosen_ma {
            if !ma.contains("/p2p/") {
                anyhow::bail!(
                    "validator id={} multiaddr `{}` is missing /p2p/<peer_id> suffix — \
                     run `evaporchain onboarding generate-network-key` first",
                    v.id,
                    ma
                );
            }
            bootstrap_peers.push(ma.clone());
        }
        validators.push(GenesisValidator {
            id: v.id,
            name: v.name.clone(),
            stake: v.stake,
            address: addr,
            bls_public_key: Some(v.bls_public_key.to_lowercase()),
            bls_pop: if v.bls_pop.is_empty() {
                None
            } else {
                Some(v.bls_pop.to_lowercase())
            },
            p2p_address: chosen_ma,
        });
        if v.balance > 0 {
            accounts.push(GenesisAccount {
                address: addr,
                balance: v.balance,
                label: format!("Validator-{}", v.name),
                vesting: None,
            });
        }
    }
    for alloc in &manifest.allocations {
        let addr = address_from_hex(&alloc.address)?;
        accounts.push(GenesisAccount {
            address: addr,
            balance: alloc.balance,
            label: alloc.label.clone(),
            vesting: None,
        });
    }

    // Assemble.
    let chain_params = ChainParams {
        chain_id: chain_id.to_string(),
        block_interval_ms,
        grace_period: 5,
        block_gas_limit: 500_000,
        max_tx_size: 1_048_576,
        max_txs_per_block: 10_000,
        min_validator_stake: min_stake,
        unbonding_period: 100,
    };
    let tokenomics = Tokenomics {
        total_supply,
        block_reward: 100,
        reward_half_life: 1_000_000,
        fee_burn_rate: 0.50,
        staker_fee_share: 0.50,
        target_staking_apy: 0.05,
        validator_commission_default: Tokenomics::default_commission(),
        max_supply_cap: None,
        emission: None,
        blocks_per_year: Tokenomics::default_blocks_per_year(),
    };

    let coordinator_pk_hex = hex::encode(kp.public_key());

    let mut config = GenesisConfig {
        chain_params,
        tokenomics,
        genesis_time: format!("{}", now_unix_ms()),
        validators,
        accounts,
        objects: vec![],
        bootstrap_peers,
        trusted_checkpoint: None,
        coordinator_pk: Some(coordinator_pk_hex.clone()),
        coordinator_signature: None, // filled below
    };

    // Validate before signing — refuse to sign a broken config.
    if let Err(errors) = config.validate() {
        for e in &errors {
            eprintln!("  - {}", e);
        }
        anyhow::bail!("{} genesis validation errors (see above)", errors.len());
    }

    // Sign the canonical bytes.
    let to_sign = config.canonical_signing_bytes();
    let sig = kp.sign(&to_sign);
    config.coordinator_signature = Some(hex::encode(&sig));

    // Pretty-print and write.
    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(output_path, &json)
        .with_context(|| format!("write {}", output_path.display()))?;

    println!(
        "Signed genesis-config written:\n  path:       {}\n  chain_id:   {}\n  validators: {}\n  accounts:   {}\n  coord pk:   {}…\n  signature:  {} bytes",
        output_path.display(),
        config.chain_params.chain_id,
        config.validators.len(),
        config.accounts.len(),
        &coordinator_pk_hex[..32.min(coordinator_pk_hex.len())],
        sig.len(),
    );
    println!(
        "\nDistribute this file + coordinator-pk.hex. Each validator runs:\n  evaporchain onboarding verify --genesis {} --coordinator-pk coordinator-pk.hex",
        output_path.display()
    );
    Ok(())
}

/// Locate the coordinator pk hex paired with the sk file. Tries `<sk>.pub`,
/// then `coordinator-pk.hex` in the same directory.
fn find_paired_pk(sk_path: &Path) -> Result<String> {
    let stem = sk_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("coordinator-sk.hex");
    let with_ext = sk_path.with_file_name(format!("{}.pub", stem));
    let candidates = [
        with_ext,
        sk_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("coordinator-pk.hex"),
    ];
    for p in &candidates {
        if p.exists() {
            return std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()));
        }
    }
    anyhow::bail!(
        "could not find paired coordinator pk. Place it next to {} as `<sk>.pub` or `coordinator-pk.hex`.",
        sk_path.display()
    );
}

// ─────────────────────────── verify ───────────────────────────────────────

pub fn cmd_verify(genesis_path: &Path, coordinator_pk_path: &Path) -> Result<()> {
    let json = std::fs::read_to_string(genesis_path)
        .with_context(|| format!("read {}", genesis_path.display()))?;
    let config: GenesisConfig = serde_json::from_str(&json).context("parse genesis json")?;

    let pk_hex = std::fs::read_to_string(coordinator_pk_path)
        .with_context(|| format!("read {}", coordinator_pk_path.display()))?;
    let pk_bytes = parse_hex_strict(&pk_hex, 1952, "coordinator public key")?;

    if let Err(e) = verify_signed_genesis(&config, &pk_bytes) {
        eprintln!("FAIL: genesis-config signature did not verify: {}", e);
        std::process::exit(1);
    }

    println!(
        "OK: genesis-config signature verifies under {}",
        coordinator_pk_path.display()
    );
    println!("  chain_id:      {}", config.chain_params.chain_id);
    println!("  validators:    {}", config.validators.len());
    if let Some(ref pk) = config.coordinator_pk {
        let head = &pk[..32.min(pk.len())];
        println!("  coord pk hex:  {}…", head);
    }
    Ok(())
}

// ─────────────────────────── Operator-side installer ─────────────────────

pub struct InstallArgs {
    pub genesis: std::path::PathBuf,
    pub keys: std::path::PathBuf,
    pub validator_id: u64,
    pub node_dir: std::path::PathBuf,
    pub coordinator_pk: Option<std::path::PathBuf>,
    pub api_port: u16,
    pub p2p_port: u16,
    pub bootstrap: Vec<String>,
    pub validators: Option<u64>,
    pub node_binary: Option<String>,
    pub systemd: bool,
    pub launchd: bool,
    pub force: bool,
}

/// Lay out a runnable validator node directory from a signed genesis +
/// the operator's keygen bundle. The node binary expects:
///
/// ```text
/// <node-dir>/
///   genesis.json           ← coordinator-signed copy
///   data/bls_key.bin       ← raw 32-byte BLS secret, mode 0600
///   logs/                  ← created empty
///   run.sh                 ← invokes evaporchain-node with the right flags
///   evaporchain.service    ← optional, --systemd
///   evaporchain.plist      ← optional, --launchd
/// ```
///
/// Re-runnable: pass `--force` to overwrite. Defensive otherwise — refuses
/// to clobber an existing layout silently.
pub fn cmd_install(args: InstallArgs) -> Result<()> {
    use evaporchain_crypto::BlsKeypair;

    // 1. Load + parse genesis.
    let genesis_text = std::fs::read_to_string(&args.genesis)
        .with_context(|| format!("read {}", args.genesis.display()))?;
    let config: GenesisConfig =
        serde_json::from_str(&genesis_text).context("parse genesis JSON")?;

    // 2. Optional signature verification — strongly recommended; only
    // skipped when the operator has out-of-band assurance (e.g. checksum).
    if let Some(ref pk_path) = args.coordinator_pk {
        let pk_hex = std::fs::read_to_string(pk_path)
            .with_context(|| format!("read {}", pk_path.display()))?;
        let pk_bytes = parse_hex_strict(&pk_hex, 1952, "coordinator public key")?;
        verify_signed_genesis(&config, &pk_bytes).with_context(|| {
            format!(
                "genesis at {} did not verify against {}",
                args.genesis.display(),
                pk_path.display()
            )
        })?;
    }

    // 3. Load keygen bundle, derive BLS pubkey, match to genesis entry.
    let mut keys_text = std::fs::read_to_string(&args.keys)
        .with_context(|| format!("read {}", args.keys.display()))?;
    let bundle: serde_json::Value = serde_json::from_str(&keys_text).context("parse keys JSON")?;
    // Crypto-2 (re-audit 2026-05-02): the parsed `keys_text` String
    // and the JSON `bundle` both hold the plaintext BLS sk hex
    // until function return. Zeroize the raw text so a core dump or
    // signal handler can't leak it. The JSON `bundle` is a Value
    // tree whose internal allocations we can't reach from here, so
    // we drop it as soon as we've extracted the hex.
    let bls_sk_hex_owned: String = bundle
        .get("bls")
        .and_then(|b| b.get("secret_key"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("keys file missing bls.secret_key"))?;
    drop(bundle);
    keys_text.zeroize();
    let mut bls_sk_hex_buf = bls_sk_hex_owned;
    let mut bls_sk_bytes =
        hex::decode(bls_sk_hex_buf.trim_start_matches("0x")).context("bls.secret_key not hex")?;
    bls_sk_hex_buf.zeroize();
    let bls_kp = BlsKeypair::from_secret_bytes(&bls_sk_bytes)
        .context("BLS secret is not a valid 32-byte scalar")?;
    let derived_pk_hex = hex::encode(&bls_kp.public_key_bytes().0);

    let entry = config
        .validators
        .iter()
        .find(|v| v.id == args.validator_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "validator_id={} not found in genesis at {}",
                args.validator_id,
                args.genesis.display()
            )
        })?;
    let expected_pk_hex = entry
        .bls_public_key
        .as_deref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "genesis validator id={} has no bls_public_key — re-run the ceremony",
                args.validator_id
            )
        })?
        .trim_start_matches("0x")
        .to_lowercase();
    if derived_pk_hex.to_lowercase() != expected_pk_hex {
        anyhow::bail!(
            "BLS pubkey mismatch for validator_id={}: keys file derives {}, genesis has {}",
            args.validator_id,
            derived_pk_hex,
            expected_pk_hex
        );
    }

    // 4. Lay out the node directory. Refuse to clobber unless --force.
    if args.node_dir.exists() && !args.force {
        let any_content = std::fs::read_dir(&args.node_dir)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false);
        if any_content {
            anyhow::bail!(
                "{} exists and is non-empty — pass --force to overwrite",
                args.node_dir.display()
            );
        }
    }
    let data_dir = args.node_dir.join("data");
    let logs_dir = args.node_dir.join("logs");
    std::fs::create_dir_all(&data_dir).with_context(|| format!("mkdir {}", data_dir.display()))?;
    std::fs::create_dir_all(&logs_dir).with_context(|| format!("mkdir {}", logs_dir.display()))?;

    // 5. Write BLS secret to <data-dir>/bls_key.bin (mode 0600).
    let bls_path = data_dir.join("bls_key.bin");
    write_secret_0600(&bls_path, &bls_sk_bytes)?;
    // H7 (audit 2026-05-02): the plaintext sk lingers in this Vec
    // (and inside the JSON `bundle` parsed earlier) until function
    // return. A core dump or signal handler in the window between
    // verify and write would leak it. Zeroize the explicit copy as
    // soon as the on-disk write succeeds.
    bls_sk_bytes.zeroize();

    // 6. Copy the genesis into the node-dir so a single bundle is
    // self-contained even if the operator moves the original.
    let genesis_dst = args.node_dir.join("genesis.json");
    std::fs::write(&genesis_dst, &genesis_text)
        .with_context(|| format!("write {}", genesis_dst.display()))?;

    // 7. Resolve --validators count: prefer explicit, else genesis size.
    let validator_count = args.validators.unwrap_or(config.validators.len() as u64);

    // 8. Emit run.sh — the canonical start command. Quoting is conservative
    // (no shell expansion of bootstrap multiaddrs).
    let node_binary = args.node_binary.as_deref().unwrap_or("evaporchain-node");
    let mut bootstrap_lines = String::new();
    for ma in &args.bootstrap {
        bootstrap_lines.push_str(&format!("  --bootstrap '{}' \\\n", ma));
    }
    let chain_id_for_log = &config.chain_params.chain_id;
    let abs_node_dir = args
        .node_dir
        .canonicalize()
        .unwrap_or_else(|_| args.node_dir.clone());
    let abs_genesis = genesis_dst
        .canonicalize()
        .unwrap_or_else(|_| genesis_dst.clone());
    let abs_data = data_dir.canonicalize().unwrap_or_else(|_| data_dir.clone());
    let run_sh = format!(
        "#!/usr/bin/env bash\n\
         # Generated by `evaporchain onboarding install` for {chain}.\n\
         # Re-run the installer with --force to regenerate.\n\
         set -euo pipefail\n\
         cd '{node_dir}'\n\
         exec '{bin}' \\\n  \
           --tendermint \\\n  \
           --network \\\n  \
           --api \\\n  \
           --validator-id {vid} \\\n  \
           --validators {vcount} \\\n  \
           --node-id v{vid} \\\n  \
           --port {p2p} \\\n  \
           --api-port {api} \\\n  \
           --data-dir '{data}' \\\n  \
           --genesis-config '{genesis}' \\\n\
{boot}\
           --startup-delay 1500\n",
        chain = chain_id_for_log,
        node_dir = abs_node_dir.display(),
        bin = node_binary,
        vid = args.validator_id,
        vcount = validator_count,
        p2p = args.p2p_port,
        api = args.api_port,
        data = abs_data.display(),
        genesis = abs_genesis.display(),
        boot = bootstrap_lines,
    );
    let run_path = args.node_dir.join("run.sh");
    std::fs::write(&run_path, run_sh).with_context(|| format!("write {}", run_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&run_path, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod 0755 {}", run_path.display()))?;
    }

    // 9. Optional service files. systemd unit:
    if args.systemd {
        let unit = format!(
            "[Unit]\n\
             Description=EvaporChain validator (chain={chain}, vid={vid})\n\
             After=network-online.target\n\
             Wants=network-online.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart={run}\n\
             Restart=on-failure\n\
             RestartSec=5s\n\
             # Run as the operator, not root. Override before installing the unit.\n\
             # User=evaporchain\n\
             # Group=evaporchain\n\
             WorkingDirectory={node_dir}\n\
             StandardOutput=append:{logs}/node.log\n\
             StandardError=append:{logs}/node.err\n\
             # Hardening: read-only system, /tmp private, no new privileges.\n\
             NoNewPrivileges=yes\n\
             ProtectSystem=strict\n\
             ProtectHome=yes\n\
             PrivateTmp=yes\n\
             ReadWritePaths={node_dir}\n\
             \n\
             [Install]\n\
             WantedBy=multi-user.target\n",
            chain = chain_id_for_log,
            vid = args.validator_id,
            run = run_path
                .canonicalize()
                .unwrap_or_else(|_| run_path.clone())
                .display(),
            node_dir = abs_node_dir.display(),
            logs = logs_dir
                .canonicalize()
                .unwrap_or_else(|_| logs_dir.clone())
                .display(),
        );
        let unit_path = args.node_dir.join("evaporchain.service");
        std::fs::write(&unit_path, unit)
            .with_context(|| format!("write {}", unit_path.display()))?;
    }

    // launchd plist (macOS):
    if args.launchd {
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.evaporchain.validator.{vid}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{run}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{node_dir}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{logs}/node.log</string>
    <key>StandardErrorPath</key>
    <string>{logs}/node.err</string>
</dict>
</plist>
"#,
            vid = args.validator_id,
            run = run_path
                .canonicalize()
                .unwrap_or_else(|_| run_path.clone())
                .display(),
            node_dir = abs_node_dir.display(),
            logs = logs_dir
                .canonicalize()
                .unwrap_or_else(|_| logs_dir.clone())
                .display(),
        );
        let plist_path = args.node_dir.join("evaporchain.plist");
        std::fs::write(&plist_path, plist)
            .with_context(|| format!("write {}", plist_path.display()))?;
    }

    // 10. Operator-facing summary.
    println!(
        "OK: validator {} installed at {}",
        args.validator_id,
        abs_node_dir.display()
    );
    println!("  chain_id:    {}", chain_id_for_log);
    println!(
        "  bls_pubkey:  {}",
        &derived_pk_hex[..32.min(derived_pk_hex.len())]
    );
    println!("  data_dir:    {}", abs_data.display());
    println!("  genesis:     {}", abs_genesis.display());
    println!("  api_port:    {}", args.api_port);
    println!("  p2p_port:    {}", args.p2p_port);
    if !args.bootstrap.is_empty() {
        println!("  bootstrap:   {} peer(s)", args.bootstrap.len());
    }
    println!();
    println!("Next steps:");
    println!("  cd {} && ./run.sh", abs_node_dir.display());
    if args.systemd {
        println!(
            "  sudo cp {}/evaporchain.service /etc/systemd/system/",
            abs_node_dir.display()
        );
        println!("  sudo systemctl daemon-reload && sudo systemctl enable --now evaporchain");
    }
    if args.launchd {
        println!(
            "  cp {}/evaporchain.plist ~/Library/LaunchAgents/",
            abs_node_dir.display()
        );
        println!("  launchctl load ~/Library/LaunchAgents/evaporchain.plist");
    }
    Ok(())
}

/// Pure verification helper. Returns Err with a human reason on any failure;
/// this is the same logic the node uses on startup so behaviour stays aligned.
pub fn verify_signed_genesis(config: &GenesisConfig, expected_pk: &[u8]) -> Result<()> {
    let sig_hex = config
        .coordinator_signature
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("genesis is missing coordinator_signature"))?;
    let sig = hex::decode(sig_hex.trim_start_matches("0x"))
        .context("coordinator_signature is not valid hex")?;

    // Optionally enforce coordinator_pk matches the supplied expected_pk.
    if let Some(ref claimed_pk_hex) = config.coordinator_pk {
        let claimed = hex::decode(claimed_pk_hex.trim_start_matches("0x"))
            .context("coordinator_pk is not valid hex")?;
        // Crypto-3 (re-audit 2026-05-02): constant-time comparison
        // for the 1952-byte ML-DSA pk so a network attacker can't
        // brute-force coordinator_pk byte-by-byte by measuring
        // verification latency. Length mismatch short-circuits to
        // bail (it's already public information). Equal-length
        // payloads always run the full subtle compare.
        if claimed.len() != expected_pk.len()
            || !bool::from(<[u8] as subtle::ConstantTimeEq>::ct_eq(
                &claimed[..],
                expected_pk,
            ))
        {
            anyhow::bail!(
                "coordinator_pk in genesis ({} bytes) does not match expected ({} bytes / different value)",
                claimed.len(),
                expected_pk.len()
            );
        }
    }

    let canonical = config.canonical_signing_bytes();
    if !MlDsaVerifier::verify(&canonical, &sig, expected_pk) {
        anyhow::bail!("ML-DSA verification returned false");
    }
    Ok(())
}

// ─────────────────────────── Tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Self-cleaning temp dir, no `tempfile` dep.
    struct TmpDir(std::path::PathBuf);
    impl TmpDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir() -> TmpDir {
        // SystemTime::now() can return identical nanosecond stamps across
        // parallel cargo-test threads on the same machine, leading to
        // shared temp-dirs and cross-test contamination (one test writes
        // genesis.json with its BLS pk, a sibling test reads it and sees
        // a stranger's keys). Mix in a process-wide atomic so every call
        // produces a unique path even when threads race.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("evaporchain-onboard-{}-{}", nonce, n));
        std::fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }

    /// Build a signed genesis for tests. Returns (tmpdir, genesis_path, pk_path).
    fn build_test_genesis() -> (TmpDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempdir();
        cmd_generate_coordinator(tmp.path()).unwrap();
        let pk_path = tmp.path().join("coordinator-pk.hex");
        let sk_path = tmp.path().join("coordinator-sk.hex");
        let manifest_path = tmp.path().join("validators.json");
        std::fs::write(
            &manifest_path,
            serde_json::json!({
                "validators": [{
                    "id": 1, "name": "alpha",
                    "bls_public_key": hex::encode([7u8; 48]),
                    "stake": 200_000, "balance": 1_000_000,
                }]
            })
            .to_string(),
        )
        .unwrap();
        let genesis_path = tmp.path().join("genesis.json");
        cmd_build_genesis(
            &manifest_path,
            &sk_path,
            "evaporchain-test-1",
            &genesis_path,
            2000,
            10_000_000,
            100,
        )
        .unwrap();
        (tmp, genesis_path, pk_path)
    }

    #[test]
    fn build_then_verify_roundtrip() {
        let (_tmp, genesis_path, pk_path) = build_test_genesis();
        let json = std::fs::read_to_string(&genesis_path).unwrap();
        let config: GenesisConfig = serde_json::from_str(&json).unwrap();
        assert!(config.coordinator_signature.is_some());
        assert!(config.coordinator_pk.is_some());
        let pk_bytes = hex::decode(std::fs::read_to_string(&pk_path).unwrap().trim()).unwrap();
        verify_signed_genesis(&config, &pk_bytes).expect("signature should verify");
    }

    #[test]
    fn tampered_genesis_fails_verification() {
        let (_tmp, genesis_path, pk_path) = build_test_genesis();
        let mut config: GenesisConfig =
            serde_json::from_str(&std::fs::read_to_string(&genesis_path).unwrap()).unwrap();
        config.chain_params.chain_id = "evaporchain-mainnet-WRONG".to_string();
        let pk_bytes = hex::decode(std::fs::read_to_string(&pk_path).unwrap().trim()).unwrap();
        assert!(
            verify_signed_genesis(&config, &pk_bytes).is_err(),
            "tampered genesis must not verify"
        );
    }

    #[test]
    fn wrong_coordinator_pk_fails_verification() {
        let (_tmp, genesis_path, _) = build_test_genesis();
        let other = MlDsaKeypair::generate();
        let config: GenesisConfig =
            serde_json::from_str(&std::fs::read_to_string(&genesis_path).unwrap()).unwrap();
        assert!(
            verify_signed_genesis(&config, other.public_key()).is_err(),
            "wrong pk must not verify"
        );
    }

    /// Build a genesis whose validator's `bls_public_key` matches a real BLS
    /// keypair we control, so the installer's pubkey-match check passes.
    /// Returns (tmpdir, genesis_path, pk_path, keys_path).
    fn build_matched_genesis(
        validator_id: u64,
    ) -> (
        TmpDir,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        use evaporchain_crypto::BlsKeypair;
        let tmp = tempdir();
        cmd_generate_coordinator(tmp.path()).unwrap();
        let pk_path = tmp.path().join("coordinator-pk.hex");
        let sk_path = tmp.path().join("coordinator-sk.hex");

        let bls = BlsKeypair::generate();
        let bls_pk_hex = hex::encode(&bls.public_key_bytes().0);
        let bls_sk_hex = hex::encode(&bls.secret_key_bytes().0);

        let manifest_path = tmp.path().join("validators.json");
        std::fs::write(
            &manifest_path,
            serde_json::json!({
                "validators": [{
                    "id": validator_id, "name": "alpha",
                    "bls_public_key": bls_pk_hex,
                    "stake": 200_000, "balance": 1_000_000,
                }]
            })
            .to_string(),
        )
        .unwrap();
        let genesis_path = tmp.path().join("genesis.json");
        cmd_build_genesis(
            &manifest_path,
            &sk_path,
            "evaporchain-install-1",
            &genesis_path,
            2000,
            10_000_000,
            100,
        )
        .unwrap();

        let keys_path = tmp.path().join("keys.json");
        std::fs::write(
            &keys_path,
            serde_json::json!({
                "bls": {
                    "public_key": bls_pk_hex,
                    "secret_key": bls_sk_hex,
                }
            })
            .to_string(),
        )
        .unwrap();

        (tmp, genesis_path, pk_path, keys_path)
    }

    #[test]
    fn install_lays_out_runnable_node_dir() {
        let (tmp, genesis_path, pk_path, keys_path) = build_matched_genesis(1);
        let node_dir = tmp.path().join("node1");
        cmd_install(InstallArgs {
            genesis: genesis_path.clone(),
            keys: keys_path,
            validator_id: 1,
            node_dir: node_dir.clone(),
            coordinator_pk: Some(pk_path.clone()),
            api_port: 8080,
            p2p_port: 7000,
            bootstrap: vec![
                "/ip4/127.0.0.1/tcp/7001".to_string(),
                "/ip4/127.0.0.1/tcp/7002".to_string(),
            ],
            validators: None,
            node_binary: Some("/usr/local/bin/evaporchain-node".to_string()),
            systemd: true,
            launchd: true,
            force: false,
        })
        .unwrap();

        // Layout assertions.
        let bls_path = node_dir.join("data/bls_key.bin");
        assert!(bls_path.is_file(), "bls_key.bin missing");
        let bls_bytes = std::fs::read(&bls_path).unwrap();
        assert_eq!(bls_bytes.len(), 32, "bls_key.bin must be 32 raw bytes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bls_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "bls_key.bin must be 0600");
        }

        assert!(node_dir.join("genesis.json").is_file());
        assert!(node_dir.join("logs").is_dir());

        let run = std::fs::read_to_string(node_dir.join("run.sh")).unwrap();
        assert!(run.starts_with("#!/usr/bin/env bash"));
        assert!(run.contains("--validator-id 1"));
        assert!(run.contains("--api-port 8080"));
        assert!(run.contains("--port 7000"));
        assert!(run.contains("/usr/local/bin/evaporchain-node"));
        assert!(run.contains("/ip4/127.0.0.1/tcp/7001"));
        assert!(run.contains("/ip4/127.0.0.1/tcp/7002"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(node_dir.join("run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "run.sh must be executable");
        }

        let unit = std::fs::read_to_string(node_dir.join("evaporchain.service")).unwrap();
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("evaporchain-install-1"));

        let plist = std::fs::read_to_string(node_dir.join("evaporchain.plist")).unwrap();
        assert!(plist.contains("com.evaporchain.validator.1"));
    }

    #[test]
    fn install_rejects_mismatched_bls_keys() {
        use evaporchain_crypto::BlsKeypair;
        // Genesis built with one BLS key; operator hands install a different
        // BLS secret. Installer must refuse rather than silently set up a
        // node that will fail consensus quorum.
        let (tmp, genesis_path, pk_path, _orig_keys) = build_matched_genesis(1);

        let other = BlsKeypair::generate();
        let other_keys = tmp.path().join("attacker-keys.json");
        std::fs::write(
            &other_keys,
            serde_json::json!({
                "bls": {
                    "public_key": hex::encode(&other.public_key_bytes().0),
                    "secret_key": hex::encode(&other.secret_key_bytes().0),
                }
            })
            .to_string(),
        )
        .unwrap();

        let node_dir = tmp.path().join("node-attacker");
        let err = cmd_install(InstallArgs {
            genesis: genesis_path,
            keys: other_keys,
            validator_id: 1,
            node_dir: node_dir.clone(),
            coordinator_pk: Some(pk_path),
            api_port: 8080,
            p2p_port: 7000,
            bootstrap: vec![],
            validators: None,
            node_binary: None,
            systemd: false,
            launchd: false,
            force: false,
        })
        .expect_err("mismatched BLS key must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("BLS pubkey mismatch"),
            "error must name BLS pubkey mismatch, got: {msg}"
        );
    }

    #[test]
    fn install_refuses_to_clobber_without_force() {
        let (tmp, genesis_path, pk_path, keys_path) = build_matched_genesis(1);
        let node_dir = tmp.path().join("node1");
        std::fs::create_dir_all(&node_dir).unwrap();
        std::fs::write(node_dir.join("PRECIOUS.txt"), "do not delete").unwrap();
        let err = cmd_install(InstallArgs {
            genesis: genesis_path.clone(),
            keys: keys_path.clone(),
            validator_id: 1,
            node_dir: node_dir.clone(),
            coordinator_pk: Some(pk_path.clone()),
            api_port: 8080,
            p2p_port: 7000,
            bootstrap: vec![],
            validators: None,
            node_binary: None,
            systemd: false,
            launchd: false,
            force: false,
        })
        .expect_err("non-empty node-dir without --force must abort");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("--force"),
            "error must mention --force, got: {msg}"
        );
        // Pre-existing file untouched.
        assert!(node_dir.join("PRECIOUS.txt").is_file());

        // With --force the install proceeds.
        cmd_install(InstallArgs {
            genesis: genesis_path,
            keys: keys_path,
            validator_id: 1,
            node_dir: node_dir.clone(),
            coordinator_pk: Some(pk_path),
            api_port: 8080,
            p2p_port: 7000,
            bootstrap: vec![],
            validators: None,
            node_binary: None,
            systemd: false,
            launchd: false,
            force: true,
        })
        .unwrap();
        assert!(node_dir.join("data/bls_key.bin").is_file());
    }

    /// T1.20 — cmd_generate_coordinator creates the expected files
    /// (coordinator-pk.hex + coordinator-sk.hex) with valid ML-DSA-65
    /// keypair. Previously 0%-covered.
    #[test]
    fn t1_20_cmd_generate_coordinator_writes_keypair_files() {
        let tmp = tempdir();
        cmd_generate_coordinator(tmp.path()).expect("generate coordinator");
        let pk_path = tmp.path().join("coordinator-pk.hex");
        let sk_path = tmp.path().join("coordinator-sk.hex");
        assert!(pk_path.exists());
        assert!(sk_path.exists());
        // pk + sk are valid hex.
        let pk_hex = std::fs::read_to_string(&pk_path).unwrap();
        let pk_bytes = hex::decode(pk_hex.trim()).expect("pk is hex");
        let sk_hex = std::fs::read_to_string(&sk_path).unwrap();
        let _sk_bytes = hex::decode(sk_hex.trim()).expect("sk is hex");
        // Re-running into same dir is idempotent (no-op on dir create).
        cmd_generate_coordinator(tmp.path()).expect("regenerate ok");
        // pk + sk files now hold the NEW values — but file existence
        // is preserved.
        assert!(pk_path.exists());
        // pk has the expected ML-DSA-65 byte length.
        assert_eq!(pk_bytes.len(), 1952);
    }

    /// T1.20 — cmd_generate_network_key writes network_key.bin to
    /// the given dir + prints listening multiaddr.
    #[test]
    fn t1_20_cmd_generate_network_key_writes_file() {
        let tmp = tempdir();
        cmd_generate_network_key(tmp.path(), "127.0.0.1", 30333).expect("generate net key");
        let path = tmp.path().join("network_key.bin");
        assert!(path.exists());
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.is_empty(), "network key bytes should be non-empty");
    }

    /// T1.20 — parse_hex_strict rejects wrong-length input.
    #[test]
    fn t1_20_parse_hex_strict_wrong_length_rejects() {
        let r = parse_hex_strict("deadbeef", 16, "test");
        assert!(r.is_err(), "expected length-mismatch error");
        let e = format!("{}", r.unwrap_err());
        assert!(e.contains("must be 16 bytes"), "got: {e}");
    }

    /// T1.20 — parse_hex_strict accepts both bare and 0x-prefixed
    /// inputs of the right length.
    #[test]
    fn t1_20_parse_hex_strict_accepts_0x_prefix() {
        let bare = parse_hex_strict("deadbeef", 4, "x").unwrap();
        let prefixed = parse_hex_strict("0xdeadbeef", 4, "x").unwrap();
        assert_eq!(bare, prefixed);
        assert_eq!(bare, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    /// T1.20 — parse_hex_strict rejects non-hex input.
    #[test]
    fn t1_20_parse_hex_strict_rejects_non_hex() {
        let r = parse_hex_strict("not-hex-at-all", 4, "x");
        assert!(r.is_err());
    }

    /// T1.20 — address_from_hex parses 32-byte hex; wraps
    /// parse_hex_strict. Length mismatch errors propagate.
    #[test]
    fn t1_20_address_from_hex_round_trip() {
        let addr_hex = "00".repeat(32);
        let addr = address_from_hex(&addr_hex).unwrap();
        assert_eq!(addr, [0u8; 32]);

        let bad = address_from_hex("deadbeef");
        assert!(bad.is_err());
    }

    /// T1.20 — cmd_verify on non-existent genesis path returns Err
    /// (file-read failure, not panic).
    #[test]
    fn t1_20_cmd_verify_missing_genesis_errors() {
        let tmp = tempdir();
        let pk_path = tmp.path().join("pk.hex");
        std::fs::write(&pk_path, "ab".repeat(1952)).unwrap();
        let missing = tmp.path().join("does-not-exist.json");
        let r = cmd_verify(&missing, &pk_path);
        assert!(r.is_err());
    }

    /// T1.20 — cmd_verify on malformed genesis JSON returns Err.
    #[test]
    fn t1_20_cmd_verify_malformed_genesis_errors() {
        let tmp = tempdir();
        let genesis = tmp.path().join("g.json");
        std::fs::write(&genesis, "not json").unwrap();
        let pk_path = tmp.path().join("pk.hex");
        std::fs::write(&pk_path, "ab".repeat(1952)).unwrap();
        let r = cmd_verify(&genesis, &pk_path);
        assert!(r.is_err());
    }
}
