//! Multi-validator onboarding flow (closes audit K-07/K-08, mainnet P0).
//! Produces a single coordinator-signed `genesis-config.json` every
//! validator passes to its node via `--genesis-config <path>`, plus a
//! verify command operators run before launch. See
//! `docs/VALIDATOR_ONBOARDING.md` for the full runbook.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    /// Stake amount at genesis (must be >= chain_params.min_validator_stake).
    pub stake: u64,
    /// Optional initial token allocation paid to this validator's address.
    #[serde(default)]
    pub balance: u64,
    /// Optional libp2p multiaddress for bootstrap peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p2p_address: Option<String>,
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
    let bytes =
        hex::decode(trimmed).with_context(|| format!("{} is not valid hex", what))?;
    if bytes.len() != expected {
        anyhow::bail!(
            "{} must be {} bytes, got {}",
            what,
            expected,
            bytes.len()
        );
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
    if !out_dir.exists() {
        std::fs::create_dir_all(out_dir)
            .with_context(|| format!("create {}", out_dir.display()))?;
    }
    let kp = MlDsaKeypair::generate();
    let pk_hex = hex::encode(kp.public_key());
    let sk_hex = hex::encode(kp.secret_key());

    let pk_path = out_dir.join("coordinator-pk.hex");
    let sk_path = out_dir.join("coordinator-sk.hex");

    std::fs::write(&pk_path, &pk_hex)
        .with_context(|| format!("write {}", pk_path.display()))?;
    write_secret_0600(&sk_path, sk_hex.as_bytes())?;

    println!("Coordinator ML-DSA-65 keypair written:");
    println!("  pk:  {}", pk_path.display());
    println!("  sk:  {} (0600)", sk_path.display());
    println!();
    println!("Distribute coordinator-pk.hex to every validator out-of-band.");
    println!("Keep coordinator-sk.hex offline; it only ever runs `onboarding build-genesis`.");
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
    for v in &manifest.validators {
        // Sanity: BLS pk is 48 bytes (96 hex chars).
        let _ = parse_hex_strict(&v.bls_public_key, 48, "validator bls_public_key")?;
        if v.id == 0 || v.id > 0xFF {
            anyhow::bail!("validator id {} must be 1..=255 (encoded in address byte 0)", v.id);
        }
        let mut addr = [0u8; 32];
        addr[0] = v.id as u8;
        validators.push(GenesisValidator {
            id: v.id,
            name: v.name.clone(),
            stake: v.stake,
            address: addr,
            bls_public_key: Some(v.bls_public_key.to_lowercase()),
            p2p_address: v.p2p_address.clone(),
        });
        if v.balance > 0 {
            accounts.push(GenesisAccount {
                address: addr,
                balance: v.balance,
                label: format!("Validator-{}", v.name),
            });
        }
    }
    for alloc in &manifest.allocations {
        let addr = address_from_hex(&alloc.address)?;
        accounts.push(GenesisAccount {
            address: addr,
            balance: alloc.balance,
            label: alloc.label.clone(),
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
    };

    let coordinator_pk_hex = hex::encode(kp.public_key());

    let mut config = GenesisConfig {
        chain_params,
        tokenomics,
        genesis_time: format!("{}", now_unix_ms()),
        validators,
        accounts,
        objects: vec![],
        bootstrap_peers: vec![],
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
    let stem = sk_path.file_name().and_then(|s| s.to_str()).unwrap_or("coordinator-sk.hex");
    let with_ext = sk_path.with_file_name(format!("{}.pub", stem));
    let candidates = [
        with_ext,
        sk_path.parent().unwrap_or(Path::new(".")).join("coordinator-pk.hex"),
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

/// Pure verification helper. Returns Err with a human reason on any failure;
/// this is the same logic the node uses on startup so behaviour stays aligned.
pub fn verify_signed_genesis(
    config: &GenesisConfig,
    expected_pk: &[u8],
) -> Result<()> {
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
        if claimed != expected_pk {
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
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn tempdir() -> TmpDir {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("evaporchain-onboard-{}", nonce));
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
        std::fs::write(&manifest_path, serde_json::json!({
            "validators": [{
                "id": 1, "name": "alpha",
                "bls_public_key": hex::encode([7u8; 48]),
                "stake": 200_000, "balance": 1_000_000,
            }]
        }).to_string()).unwrap();
        let genesis_path = tmp.path().join("genesis.json");
        cmd_build_genesis(
            &manifest_path, &sk_path, "evaporchain-test-1", &genesis_path,
            2000, 10_000_000, 100,
        ).unwrap();
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
        assert!(verify_signed_genesis(&config, &pk_bytes).is_err(),
            "tampered genesis must not verify");
    }

    #[test]
    fn wrong_coordinator_pk_fails_verification() {
        let (_tmp, genesis_path, _) = build_test_genesis();
        let other = MlDsaKeypair::generate();
        let config: GenesisConfig =
            serde_json::from_str(&std::fs::read_to_string(&genesis_path).unwrap()).unwrap();
        assert!(verify_signed_genesis(&config, other.public_key()).is_err(),
            "wrong pk must not verify");
    }
}
