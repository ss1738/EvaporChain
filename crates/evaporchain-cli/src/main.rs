use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::*;
use serde::{Deserialize, Serialize};

use evaporchain_crypto::{BlsKeypair, MlDsaKeypair, VrfKeypair};
use evaporchain_execution::genesis::{initialize_genesis, load_genesis_config};
use evaporchain_types::genesis::GenesisConfig;

mod onboarding;

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

    /// Multi-node local testnet orchestrator: init, up, status, down.
    ///
    /// Differs from `devnet` (which spawns nodes from scratch each run with
    /// no shared genesis): `testnet init` produces a reproducible directory
    /// layout with per-validator BLS keypairs, a shared genesis config that
    /// pre-registers every validator's BLS pubkey, and persistent data
    /// directories. `testnet up` spawns nodes against that layout and
    /// writes pid files; `testnet status` polls every node's API; `testnet
    /// down` kills the recorded pids.
    Testnet {
        #[command(subcommand)]
        action: TestnetAction,
    },

    /// Generate a validator keypair bundle (BLS + ML-DSA + VRF)
    Keygen {
        /// Output file path (default: stdout)
        #[arg(long)]
        output: Option<String>,
    },

    /// Multi-validator mainnet onboarding flow (closes audit K-07/K-08).
    /// Produces a single coordinator-signed genesis-config.json that every
    /// validator passes to its node via `--genesis-config <path>`.
    Onboarding {
        #[command(subcommand)]
        action: OnboardingAction,
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

    /// State snapshot tooling: create, verify, apply.
    /// Used by the Ansible deploy playbook (deploy/ansible/playbooks/snapshot.yml)
    /// and by operators restoring a node from a known-good backup.
    Snapshot {
        #[command(subcommand)]
        action: SnapshotAction,
    },

    /// Data-availability tooling. Light-client cell sampling against a
    /// remote node — closes the audit-memo gap (DA sampling over network
    /// with real 2D erasure coding) by wiring `LightClientSampler` to an
    /// HTTP cell source pointing at a real node URL.
    Da {
        #[command(subcommand)]
        action: DaAction,
    },

    /// Build (and optionally broadcast) an UpgradeContract transaction.
    ///
    /// Two authorisation modes — supply exactly one:
    ///   * `--admin-key path` — sign with the admin's ML-DSA-65 secret
    ///     key (Path A). Produces `admin_signature` + `admin_public_key`.
    ///   * `--governance-quorum path` — JSON file `[{"stake":1000}, …]`
    ///     plus `--required-stake N` for the chain's stake-quorum gate
    ///     (Path B; mirrors `/api/governance/fork_choice_mode`).
    ///
    /// By default prints the signed tx body as JSON. Pass `--broadcast`
    /// (with `--node URL`) to POST it to `/api/tx/upgrade_contract`.
    UpgradeContract {
        /// Sender (owner) address as hex (32-byte hex, with or without 0x).
        #[arg(long)]
        owner: String,

        /// Contract id to upgrade.
        #[arg(long)]
        contract_id: u64,

        /// New bytecode supplied as hex bytes.
        #[arg(long, conflicts_with = "new_bytecode_path")]
        new_bytecode_hex: Option<String>,

        /// Path to a file containing the new bytecode (UTF-8 EvaporScript
        /// source for the script engine; binary bytes for future VMs).
        #[arg(long)]
        new_bytecode_path: Option<String>,

        /// Sender nonce (the chain expects current `account.nonce`).
        #[arg(long)]
        nonce: u64,

        /// Path A — path to admin secret-key hex file (ML-DSA-65 SK).
        #[arg(long, conflicts_with = "governance_quorum")]
        admin_key: Option<String>,

        /// Path B — JSON file with endorser stakes
        /// (`[{"stake":1000}, …]`).
        #[arg(long)]
        governance_quorum: Option<String>,

        /// Path B — minimum stake total required for the amendment.
        #[arg(long, default_value = "0")]
        required_stake: u64,

        /// Broadcast to a running node instead of just printing the body.
        #[arg(long)]
        broadcast: bool,
    },
}

#[derive(Subcommand)]
pub enum DaAction {
    /// Run a light-client DA sampling round against a remote node. Fetches
    /// the 2D header from `/api/da/header/:block`, samples N random cells
    /// via `/api/da/cell/:block/:row/:col`, and verifies each proof
    /// locally. Exits 0 on `passes(threshold)`, non-zero on any failure.
    ///
    /// Catches: nodes that announce a data_root but can't actually serve
    /// the underlying cells (data unavailability), nodes that serve
    /// fabricated cells (wrong proofs), and node misconfiguration where
    /// the 2D matrix is empty / out of date.
    Verify {
        /// Node URL(s), e.g. `--node http://node1:8080 --node http://node2:8080`.
        /// Repeatable. When more than one is supplied, the sampler round-
        /// robins per cell; faulty-peer detection then names the specific
        /// URL that served a bad cell.
        #[arg(long = "node", required = true)]
        nodes: Vec<String>,
        /// Block number to sample.
        #[arg(long)]
        block: u64,
        /// Number of cells to sample. Celestia's analysis says 16 honest
        /// samples gives >99.998% confidence that ≥50% of cells are
        /// available. Default 16. Cap at 256 to avoid hammering nodes.
        #[arg(long, default_value = "16")]
        samples: usize,
        /// Confidence threshold the sampling must clear for a 0 exit.
        /// Default 0.999. Range (0.0, 1.0).
        #[arg(long, default_value = "0.999")]
        threshold: f64,
        /// Optional 32-byte hex seed for the cell-query RNG. Defaults to
        /// `blake3(block_le_bytes)` so two operators sampling the same
        /// block hit the same cells (good for deterministic CI checks).
        /// Pass a unique seed to spread the load across nodes.
        #[arg(long)]
        seed: Option<String>,
        /// Skip the on-chain `data_root` cross-check. Without this flag,
        /// the verifier fetches `/api/block/:N` from the FIRST `--node`
        /// and refuses to sample if its `data_root` disagrees with the
        /// served 2D header's `data_root`. Catches a producer that
        /// publishes a header for a block whose committed `data_root`
        /// was something else.
        #[arg(long)]
        skip_chain_attestation: bool,
    },
}

#[derive(Subcommand)]
pub enum SnapshotAction {
    /// Create a snapshot from a node data directory and write a `.zst`
    /// blob to the given output path. Reads from RocksDB directly so
    /// the node MUST be stopped (single-writer lock).
    Create {
        /// Path to the node data directory (the same `--data-dir` the
        /// node was started with).
        #[arg(long)]
        data_dir: String,

        /// Where to write the snapshot blob. Use a `.zst` extension.
        #[arg(long)]
        output: String,

        /// Chain id baked into the snapshot file. Verifiers reject
        /// snapshots whose chain_id doesn't match their own.
        #[arg(long, default_value = "evaporchain-mainnet-1")]
        chain_id: String,
    },

    /// Read + decompress + integrity-verify a snapshot blob and print
    /// its metadata. Exit 0 on success, 1 on verify failure.
    Verify {
        /// Path to the `.zst` snapshot blob.
        #[arg(long)]
        input: String,
    },

    /// Apply a snapshot blob to a (pre-emptied) data directory. Wipes
    /// the existing state DB and replays every account/object/ghost
    /// from the snapshot. Used for restore-from-backup workflows.
    Apply {
        /// Path to the `.zst` snapshot blob.
        #[arg(long)]
        input: String,

        /// Path to the (empty or to-be-overwritten) data directory.
        #[arg(long)]
        data_dir: String,
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

    /// Operator-side: build a signed validator contribution envelope from a
    /// keygen bundle. Output is shareable with the coordinator running
    /// `genesis ceremony` — it carries the validator entry, an ML-DSA
    /// signature over its canonical bytes, and a BLS proof-of-possession.
    Contribute {
        /// Path to a keygen JSON bundle (from `evaporchain keygen`)
        #[arg(long)]
        keys: String,
        /// Validator id (must be unique across the ceremony)
        #[arg(long)]
        validator_id: u64,
        /// Operator-chosen validator name
        #[arg(long)]
        name: String,
        /// Initial stake
        #[arg(long)]
        stake: u64,
        /// Optional P2P multiaddr
        #[arg(long)]
        p2p: Option<String>,
        /// Initial account balance for this validator
        #[arg(long, default_value = "1000000")]
        balance: u64,
        /// Address byte (first byte of the 32-byte address; rest zeroed)
        #[arg(long)]
        address_byte: Option<u8>,
        /// Chain id this contribution is bound to
        #[arg(long)]
        chain_id: String,
        /// Genesis timestamp (ISO-8601)
        #[arg(long)]
        genesis_time: String,
        /// 32-byte hex ceremony nonce agreed by all operators
        #[arg(long)]
        ceremony_nonce: String,
        /// Output envelope JSON path
        #[arg(long)]
        out: String,
    },

    /// Coordinator-side: combine a directory of contribution envelopes into a
    /// finalized genesis.json + transcript. Verifies every envelope's
    /// ML-DSA signature and BLS PoP, rejects duplicate validator ids,
    /// sorts deterministically by validator_id.
    Ceremony {
        /// Directory of `*.json` contribution envelopes
        #[arg(long)]
        contributions: String,
        /// Chain id the ceremony was anchored to (must match every envelope)
        #[arg(long)]
        chain_id: String,
        /// Genesis timestamp (ISO-8601)
        #[arg(long)]
        genesis_time: String,
        /// 32-byte hex ceremony nonce
        #[arg(long)]
        ceremony_nonce: String,
        /// Total token supply
        #[arg(long, default_value = "10000000")]
        total_supply: u64,
        /// Block interval in milliseconds
        #[arg(long, default_value = "3000")]
        block_interval: u64,
        /// Minimum validator stake
        #[arg(long, default_value = "100")]
        min_stake: u64,
        /// Output genesis JSON path (transcript written to <out>.transcript.json)
        #[arg(long)]
        out: String,
    },

    /// Anyone-side: replay a ceremony from its envelopes and the produced
    /// genesis file. Verifies every signature, recomputes the deterministic
    /// genesis bytes, and exits non-zero on any mismatch.
    VerifyCeremony {
        /// Directory of `*.json` contribution envelopes
        #[arg(long)]
        contributions: String,
        /// Path to the finalized genesis.json
        #[arg(long)]
        genesis: String,
        /// Path to the transcript written by `genesis ceremony`
        #[arg(long)]
        transcript: String,
    },

    /// Take a `genesis run-gate --json` payload and stamp the latest decision
    /// into the §A1.8 section of `research/INVENTION_STACK.md` (or a
    /// custom doc) between auto-generated markers. On first run, inserts
    /// the marker block right after the section heading; subsequent runs
    /// rewrite only the marked region — surrounding prose stays intact.
    StampResult {
        /// Path to the `genesis run-gate --json` payload. Use `-` for stdin.
        #[arg(long)]
        from_json: String,
        /// Path to the markdown doc to update.
        #[arg(long, default_value = "research/INVENTION_STACK.md")]
        doc: String,
        /// Heading line that anchors the gate section (substring match).
        #[arg(long, default_value = "## A1.8")]
        section: String,
        /// Print the proposed update to stdout instead of writing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Pre-mainnet: replay the MERA empirical-entropy gate (§A1.8) in pure
    /// Rust to decide whether the chain commits to authenticated MERA, MPS,
    /// or Verkle as its state-commitment primitive. Source can be either a
    /// CSV of real account-touch telemetry (`--csv`) or one of the three
    /// synthetic regimes (`--regime`).
    ///
    /// CSV format: rows = accounts, columns = blocks. Each cell is `0` or
    /// `1` (touched / not-touched). Optional header line beginning with `#`
    /// is ignored. Comma-separated.
    ///
    /// Exit code: `0` = MERA, `1` = MPS, `2` = Verkle, `64` = invalid input.
    /// JSON output (`--json`) emits the full decision payload for
    /// downstream tooling.
    RunGate {
        /// CSV file of binary account-touch indicators (rows × cols).
        /// Mutually exclusive with `--regime`.
        #[arg(long, conflicts_with = "regime")]
        csv: Option<String>,
        /// Run against a built-in synthetic regime instead of real data.
        /// One of `log-correlated`, `area-law`, `flat-random`. Useful as
        /// a CI smoke test or to demonstrate the decision rule.
        #[arg(long)]
        regime: Option<String>,
        /// Number of accounts (rows) for synthetic regimes. Ignored for CSV.
        #[arg(long, default_value = "64")]
        n_accounts: usize,
        /// Number of blocks (cols) for synthetic regimes. Ignored for CSV.
        #[arg(long, default_value = "128")]
        n_blocks: usize,
        /// Top-K eigenvalues to fit. Default matches the Python gate.
        #[arg(long, default_value = "40")]
        k: usize,
        /// Histogram bin count for the mutual-information matrix.
        #[arg(long, default_value = "8")]
        bins: usize,
        /// Seed for the eigensolver's power-iteration starting vector and
        /// (when `--regime` is supplied) the synthetic generator.
        #[arg(long, default_value = "12345")]
        seed: u64,
    },
}

#[derive(Subcommand)]
pub enum TestnetAction {
    /// Generate a fresh testnet layout: per-validator BLS keys, shared
    /// genesis (pre-registering every validator), per-validator data dirs.
    Init {
        /// Where to lay out the testnet (directory will be created).
        #[arg(long, default_value = "./testnet")]
        out: String,
        /// Number of validators (≥ 1). 4 is the minimum for n>3f tolerance.
        #[arg(long, default_value = "4")]
        validators: u32,
        /// Chain id baked into the genesis config.
        #[arg(long, default_value = "evaporchain-testnet-1")]
        chain_id: String,
        /// Initial total supply (split equally among validators + faucet).
        #[arg(long, default_value = "1000000000")]
        total_supply: u64,
        /// Per-validator stake at genesis.
        #[arg(long, default_value = "1000000")]
        stake: u64,
        /// Block interval (ms) for this testnet.
        #[arg(long, default_value = "2000")]
        block_interval_ms: u64,
        /// Base P2P port (validator i listens on base+i).
        #[arg(long, default_value = "9000")]
        p2p_base: u16,
        /// Base API port (validator i listens on base+i).
        #[arg(long, default_value = "8080")]
        api_base: u16,
        /// IPv4 address baked into the generated bootstrap multiaddrs. For a
        /// single-host testnet leave the default. For a real cross-host
        /// cluster (e.g. 3-Mini Tailscale deploy) override per-validator
        /// after `init` or pass the cluster-wide IP for every node here.
        #[arg(long, default_value = "127.0.0.1")]
        listen_ip: String,
        /// If the output directory already exists, remove and recreate it.
        #[arg(long)]
        force: bool,
    },

    /// Spawn every validator in a previously-initialised testnet directory.
    /// Records per-validator pid files so `testnet down` can stop them.
    Up {
        /// Testnet layout directory (the same `--out` passed to `init`).
        #[arg(long, default_value = "./testnet")]
        dir: String,
        /// Write each node's stdout+stderr to its data dir's `node.log`.
        #[arg(long)]
        split_logs: bool,
    },

    /// Poll every node's `/api/status` endpoint and print a summary table.
    Status {
        /// Testnet layout directory.
        #[arg(long, default_value = "./testnet")]
        dir: String,
    },

    /// Kill every node recorded in the testnet layout's pid files.
    Down {
        /// Testnet layout directory.
        #[arg(long, default_value = "./testnet")]
        dir: String,
    },
}

#[derive(Subcommand)]
pub enum OnboardingAction {
    /// Generate the coordinator ML-DSA-65 keypair (writes coordinator-pk.hex
    /// and coordinator-sk.hex into out_dir).
    GenerateCoordinator {
        #[arg(long, default_value = ".")]
        out_dir: String,
    },

    /// Build a signed genesis-config.json from a validator manifest and
    /// coordinator secret key. Refuses to sign an invalid config.
    BuildGenesis {
        #[arg(long)]
        validators: String,
        #[arg(long)]
        coordinator_sk: String,
        #[arg(long)]
        chain_id: String,
        #[arg(long)]
        output: String,
        #[arg(long, default_value = "2000")]
        block_interval_ms: u64,
        #[arg(long, default_value = "1000000000")]
        total_supply: u64,
        #[arg(long, default_value = "100000")]
        min_stake: u64,
    },

    /// Verify a genesis-config.json against a coordinator pk. Exit 0 valid,
    /// 1 on any failure.
    Verify {
        #[arg(long)]
        genesis: String,
        #[arg(long)]
        coordinator_pk: String,
    },

    /// Generate a libp2p ed25519 identity key for one validator and print
    /// the resulting `/p2p/<peer_id>` so the operator can hand the
    /// multiaddr to the coordinator. Writes `network_key.bin` (mode 0600)
    /// into `--out-dir`. The same file MUST land at `<data-dir>/network_key.bin`
    /// on the validator's host before the node starts; otherwise the
    /// PeerId on disk will not match the one the cluster expects.
    GenerateNetworkKey {
        #[arg(long, default_value = ".")]
        out_dir: String,
        /// Optional listen IP for the printed multiaddr template
        /// (the operator can paste the result directly into the manifest).
        #[arg(long, default_value = "0.0.0.0")]
        listen_ip: String,
        /// Optional TCP port for the printed multiaddr template.
        #[arg(long, default_value = "9000")]
        port: u16,
    },

    /// Operator-side installer: lay out a node directory from a signed
    /// genesis + the operator's keygen bundle, write the validator's BLS
    /// secret to the path the node expects, emit a launchable run.sh, and
    /// optionally a systemd unit / launchd plist for autostart.
    ///
    /// Re-runnable: pass `--force` to overwrite an existing node-dir.
    Install {
        /// Path to the coordinator-signed `genesis-config.json`.
        #[arg(long)]
        genesis: String,
        /// Path to the operator's keygen JSON bundle (from `evaporchain keygen`).
        #[arg(long)]
        keys: String,
        /// Validator id this operator owns. Must match a genesis entry whose
        /// `bls_public_key` derives from `--keys`.
        #[arg(long)]
        validator_id: u64,
        /// Output directory for the node layout. Created if absent.
        #[arg(long)]
        node_dir: String,
        /// Optional coordinator pk hex file. When supplied, the genesis
        /// signature is verified before the install proceeds.
        #[arg(long)]
        coordinator_pk: Option<String>,
        /// HTTP API listen port.
        #[arg(long, default_value = "8080")]
        api_port: u16,
        /// libp2p P2P listen port.
        #[arg(long, default_value = "7000")]
        p2p_port: u16,
        /// Bootstrap peer multiaddrs (repeatable).
        #[arg(long = "bootstrap")]
        bootstrap: Vec<String>,
        /// Total validator count for `--validators` on the node command line.
        /// Defaults to the validator count in the genesis file.
        #[arg(long)]
        validators: Option<u64>,
        /// Path to the `evaporchain-node` binary that run.sh should exec.
        /// Defaults to `evaporchain-node` on PATH.
        #[arg(long)]
        node_binary: Option<String>,
        /// Emit a systemd unit at `<node-dir>/evaporchain.service`.
        #[arg(long)]
        systemd: bool,
        /// Emit a launchd plist at `<node-dir>/evaporchain.plist`.
        #[arg(long)]
        launchd: bool,
        /// Overwrite an existing node-dir layout in place.
        #[arg(long)]
        force: bool,
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
        format!("{}{}", filled_str.green(), empty_str.truecolor(60, 60, 60))
    } else if pct > 0.2 {
        format!("{}{}", filled_str.yellow(), empty_str.truecolor(60, 60, 60))
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
    println!("  {}  {}", "\u{25C6}".cyan(), title.bold());
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
        format!("{} v{}", status.chain_name, status.version)
            .white()
            .bold()
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
        format!("{}...", &status.state_root[..24]).truecolor(100, 110, 130)
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
    let blocks: Vec<BlockRecord> = api_get(base, &format!("/api/blocks?limit={}", limit)).await?;

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

async fn cmd_transfer(
    base: &str,
    from: u8,
    to: u8,
    amount: u64,
    nonce: u64,
    json_mode: bool,
) -> Result<()> {
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

    if result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let amount = result.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
        let balance = result
            .get("new_balance")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        println!(
            "  {} Received {} EVAP (new balance: {})",
            "\u{2714}".green().bold(),
            amount,
            balance
        );
    } else {
        let msg = result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("failed");
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
    println!(
        "  Block Height:  {}",
        status.block_height.to_string().cyan().bold()
    );
    println!("  Epoch:         {}", status.epoch.to_string().cyan());
    println!("  State Root:    {}", &status.state_root[..16].dimmed());
    println!(
        "  Peers:         {}",
        status.peer_count.to_string().yellow()
    );
    println!(
        "  Proving:       {}",
        if status.proving_enabled {
            "Nova IVC".green()
        } else {
            "Mock".dimmed()
        }
    );
    println!();

    Ok(())
}

// ──────────────────────────── Devnet ─────────────────────────────────────

async fn cmd_devnet(validators: u32, demo: bool) -> Result<()> {
    println!(
        "  {} Launching EvaporChain devnet with {} validators...",
        "".bold(),
        validators
    );

    let binary = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("evaporchain"))
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("evaporchain-node");

    if !binary.exists() {
        println!(
            "  {} evaporchain-node binary not found at {:?}",
            "\u{2718}".red().bold(),
            binary
        );
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
            "--tendermint",
            "--network",
            "--api",
            "--validator-id",
            &vid.to_string(),
            "--validators",
            &validators.to_string(),
            "--node-id",
            &format!("node-{}", vid),
            "--port",
            &p2p_port.to_string(),
            "--api-port",
            &api_port.to_string(),
            "--data-dir",
            &data_dir,
            "--startup-delay",
            "3000",
        ]);

        if demo {
            cmd.arg("--demo");
        }

        // Add bootstrap peers (all other validators)
        for other in 1..=validators {
            if other != vid {
                cmd.args([
                    "--bootstrap",
                    &format!("/ip4/127.0.0.1/tcp/{}", 8999 + other as u16),
                ]);
            }
        }

        println!(
            "  Starting validator {} (API :{}, P2P :{})",
            vid, api_port, p2p_port
        );
        let child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context(format!("Failed to spawn validator {}", vid))?;
        children.push((vid, child));
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    println!();
    println!(
        "  {} All {} validators launched!",
        "\u{2714}".green().bold(),
        validators
    );
    println!();
    for vid in 1..=validators {
        println!(
            "  Validator {}: http://localhost:{}",
            vid,
            8079 + vid as u16
        );
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

// ──────────────────────────── Testnet orchestrator ──────────────────────

#[derive(Serialize, Deserialize)]
struct TestnetLayout {
    chain_id: String,
    validators: u32,
    p2p_base: u16,
    api_base: u16,
    block_interval_ms: u64,
    stake: u64,
    /// Path (relative to layout dir) to the shared genesis JSON.
    genesis_path: String,
}

fn validator_address(id: u64) -> [u8; 32] {
    // Deterministic address: id encoded little-endian into the first 8 bytes,
    // 0xAA pad in the rest. Avoids needing an ML-DSA pubkey just to seat a
    // validator at genesis — the consensus engine only cares about (id, stake,
    // bls_pk).
    let mut a = [0xAAu8; 32];
    a[..8].copy_from_slice(&id.to_le_bytes());
    a
}

#[allow(clippy::too_many_arguments)]
fn cmd_testnet_init(
    out: &str,
    validators: u32,
    chain_id: &str,
    total_supply: u64,
    stake: u64,
    block_interval_ms: u64,
    p2p_base: u16,
    api_base: u16,
    listen_ip: &str,
    force: bool,
) -> Result<()> {
    use evaporchain_types::genesis::*;
    use std::path::PathBuf;

    if validators == 0 {
        anyhow::bail!("--validators must be ≥ 1");
    }

    let root = PathBuf::from(out);
    if root.exists() {
        if force {
            std::fs::remove_dir_all(&root)
                .with_context(|| format!("Failed to remove existing layout at {}", out))?;
        } else {
            anyhow::bail!(
                "{} already exists. Pass --force to overwrite, or pick a different --out.",
                out
            );
        }
    }
    std::fs::create_dir_all(&root)
        .with_context(|| format!("Failed to create layout at {}", out))?;

    let mut genesis_validators = Vec::with_capacity(validators as usize);
    let mut genesis_accounts = Vec::with_capacity(validators as usize + 1);
    // Bootstrap multiaddrs (with /p2p/<peer_id> suffix) collected as we
    // generate each validator's libp2p identity. These get baked into
    // genesis.bootstrap_peers so every node in the cluster knows the
    // canonical address+identity of every other node at startup,
    // closing the gap that prevented gossipsub mesh formation on the
    // 3-Mini Tailscale cluster (peer_count stuck at 0).
    let mut bootstrap_multiaddrs: Vec<String> = Vec::with_capacity(validators as usize);

    for vid in 1..=validators as u64 {
        let v_dir = root.join(format!("v{}", vid));
        let v_data_dir = v_dir.join("data");
        std::fs::create_dir_all(&v_data_dir)
            .with_context(|| format!("Failed to create v{}/data", vid))?;

        // Generate the BLS keypair for this validator and write the raw
        // 32-byte secret to <data_dir>/bls_key.bin (mode 0600). The node
        // binary auto-detects plaintext vs EVK1 by length and looks
        // inside its --data-dir, so colocating the key there is the
        // simplest contract. Operator-side encryption is out of scope.
        let kp = BlsKeypair::generate();
        let sk_bytes = kp.secret_key_bytes();
        let sk: &[u8] = &sk_bytes.0;
        let pk_hex = hex::encode(&kp.public_key_bytes().0);
        let bls_path = v_data_dir.join("bls_key.bin");
        std::fs::write(&bls_path, &sk)
            .with_context(|| format!("Failed to write {}", bls_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bls_path, std::fs::Permissions::from_mode(0o600))?;
        }

        // Generate (or reuse if already on disk) this validator's libp2p
        // identity ed25519 keypair, persisted as
        // <v_dir>/data/network_key.bin (mode 0600). Compute the PeerId
        // and assemble the deterministic dialable multiaddr so genesis
        // can publish a stable peer-id-suffixed entry.
        let v_port = p2p_base + vid as u16;
        let v_keypair = evaporchain_network::load_or_generate_identity(&v_data_dir)
            .with_context(|| {
                format!(
                    "Failed to load_or_generate libp2p identity for v{}",
                    vid
                )
            })?;
        let v_peer_id = v_keypair.public().to_peer_id();
        let v_multiaddr =
            format!("/ip4/{}/tcp/{}/p2p/{}", listen_ip, v_port, v_peer_id);
        bootstrap_multiaddrs.push(v_multiaddr.clone());

        genesis_validators.push(GenesisValidator {
            id: vid,
            name: format!("validator-{}", vid),
            stake,
            address: validator_address(vid),
            bls_public_key: Some(pk_hex),
            p2p_address: Some(v_multiaddr),
        });

        genesis_accounts.push(GenesisAccount {
            address: validator_address(vid),
            balance: total_supply / (validators as u64 + 1),
            label: format!("validator-{}-operator", vid),
        });
    }

    // Faucet: hold the remaining share so the testnet has a known funded
    // address tests/demos can transfer from.
    let faucet_share = total_supply
        .saturating_sub((total_supply / (validators as u64 + 1)) * (validators as u64));
    genesis_accounts.push(GenesisAccount {
        address: [0xFAu8; 32],
        balance: faucet_share,
        label: "faucet".into(),
    });

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let config = GenesisConfig {
        chain_params: ChainParams {
            chain_id: chain_id.to_string(),
            block_interval_ms,
            grace_period: 5,
            block_gas_limit: 500_000,
            max_tx_size: 1_048_576,
            max_txs_per_block: 200,
            min_validator_stake: stake / 2,
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
        genesis_time: format!("{}", now_secs),
        validators: genesis_validators,
        accounts: genesis_accounts,
        objects: vec![],
        bootstrap_peers: bootstrap_multiaddrs.clone(),
        trusted_checkpoint: None,
        coordinator_pk: None,
        coordinator_signature: None,
    };

    let genesis_path = root.join("genesis.json");
    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(&genesis_path, &json)
        .with_context(|| format!("Failed to write {}", genesis_path.display()))?;

    let layout = TestnetLayout {
        chain_id: chain_id.to_string(),
        validators,
        p2p_base,
        api_base,
        block_interval_ms,
        stake,
        genesis_path: "genesis.json".into(),
    };
    std::fs::write(
        root.join("layout.json"),
        serde_json::to_string_pretty(&layout)?,
    )?;

    println!(
        "  {} Testnet layout written to {}",
        "\u{2714}".green().bold(),
        out
    );
    println!("  Validators:   {}", validators);
    println!("  Chain id:     {}", chain_id);
    println!(
        "  P2P ports:    {}–{}",
        p2p_base + 1,
        p2p_base + validators as u16
    );
    println!(
        "  API ports:    {}–{}",
        api_base + 1,
        api_base + validators as u16
    );
    println!(
        "  Genesis:      {}",
        genesis_path.display().to_string().white().bold()
    );
    println!();
    println!("  Next: `evaporchain testnet up --dir {}`", out);
    Ok(())
}

async fn cmd_testnet_up(dir: &str, split_logs: bool) -> Result<()> {
    use std::path::PathBuf;

    let root = PathBuf::from(dir);
    let layout_path = root.join("layout.json");
    let layout: TestnetLayout = serde_json::from_str(
        &std::fs::read_to_string(&layout_path)
            .with_context(|| format!("Failed to read {}", layout_path.display()))?,
    )
    .context("layout.json is corrupt — re-run `testnet init`")?;

    let binary = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("evaporchain"))
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("evaporchain-node");
    if !binary.exists() {
        anyhow::bail!(
            "evaporchain-node not found at {:?}. Build it: cargo build --release -p evaporchain-node",
            binary
        );
    }

    let genesis_abs = root.join(&layout.genesis_path).canonicalize()?;
    let pid_dir = root.join("pids");
    std::fs::create_dir_all(&pid_dir)?;

    let mut bootstrap = Vec::new();
    for vid in 1..=layout.validators as u64 {
        bootstrap.push(format!(
            "/ip4/127.0.0.1/tcp/{}",
            layout.p2p_base + vid as u16
        ));
    }

    println!(
        "  {} Spawning {} validators against {}",
        "".bold(),
        layout.validators,
        layout.chain_id.cyan()
    );

    for vid in 1..=layout.validators {
        let v_dir = root.join(format!("v{}", vid));
        let data_dir = v_dir.join("data").canonicalize()?;
        let api_port = layout.api_base + vid as u16;
        let p2p_port = layout.p2p_base + vid as u16;

        let mut cmd = std::process::Command::new(&binary);
        cmd.args([
            "--tendermint",
            "--network",
            "--api",
            "--validator-id",
            &vid.to_string(),
            "--validators",
            &layout.validators.to_string(),
            "--node-id",
            &format!("v{}", vid),
            "--port",
            &p2p_port.to_string(),
            "--api-port",
            &api_port.to_string(),
            "--data-dir",
            &data_dir.to_string_lossy(),
            "--genesis-config",
            &genesis_abs.to_string_lossy(),
            "--startup-delay",
            "1500",
            // Release builds refuse to start without one of --prove
            // or --mock-prove (chain refuses to silently produce
            // un-attested blocks). Testnet is a devnet by definition,
            // so use the mock prover. Operators running a real chain
            // start the binary directly with --prove + a real prover.
            "--mock-prove",
        ]);
        for peer in &bootstrap {
            // Skip our own listener address so we don't dial ourselves.
            if peer.ends_with(&format!("/{}", p2p_port)) {
                continue;
            }
            cmd.args(["--bootstrap", peer]);
        }

        if split_logs {
            let log_path = v_dir.join("node.log");
            let log_file = std::fs::File::create(&log_path)
                .with_context(|| format!("Failed to create {}", log_path.display()))?;
            let log_clone = log_file.try_clone()?;
            cmd.stdout(std::process::Stdio::from(log_file))
                .stderr(std::process::Stdio::from(log_clone));
        }

        let child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn validator {}", vid))?;
        let pid = child.id();
        std::fs::write(pid_dir.join(format!("v{}.pid", vid)), pid.to_string())?;
        // Detach: forget the Child so the OS reaps it independently of this CLI.
        std::mem::forget(child);

        println!(
            "  Started v{} (pid={}, api=:{}, p2p=:{})",
            vid, pid, api_port, p2p_port
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    println!();
    println!(
        "  {} Run `evaporchain testnet status --dir {}` to check on the cluster",
        "\u{2714}".green().bold(),
        dir
    );
    println!(
        "  {} Run `evaporchain testnet down   --dir {}` to stop it",
        "".bold(),
        dir
    );
    Ok(())
}

#[derive(Deserialize)]
struct HealthSnap {
    block_height: u64,
    epoch: u64,
    last_block_age_secs: Option<u64>,
    peer_count: usize,
    mempool_size: usize,
    finalised_height: u64,
    finality_lag_blocks: u64,
    status: String,
}

async fn cmd_testnet_status(dir: &str) -> Result<()> {
    use std::path::PathBuf;

    let root = PathBuf::from(dir);
    let layout: TestnetLayout = serde_json::from_str(
        &std::fs::read_to_string(root.join("layout.json"))
            .with_context(|| format!("Failed to read {}/layout.json", dir))?,
    )?;

    println!(
        "  {} Testnet status ({} validators, chain={})",
        "".bold(),
        layout.validators,
        layout.chain_id.cyan()
    );
    println!(
        "  {:<5} {:>6}  {:>8}  {:>9}  {:>5}  {:>4}  {:>4}  {:<10}",
        "node", "api", "height", "finalised", "lag", "age", "mp", "status"
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    // Collect health snapshots in parallel — querying N nodes serially
    // gets noticeable past 8 validators.
    let mut futs = Vec::with_capacity(layout.validators as usize);
    for vid in 1..=layout.validators {
        let api_port = layout.api_base + vid as u16;
        let url = format!("http://127.0.0.1:{}/api/network/health", api_port);
        let client = client.clone();
        futs.push(async move {
            let resp = client.get(&url).send().await;
            (vid, api_port, resp)
        });
    }
    let results = futures::future::join_all(futs).await;

    let mut heights: Vec<u64> = Vec::new();
    for (vid, api_port, resp) in results {
        let line = match resp {
            Ok(r) if r.status().is_success() => match r.json::<HealthSnap>().await {
                Ok(s) => {
                    heights.push(s.block_height);
                    let status_color = match s.status.as_str() {
                        "healthy" => s.status.green().to_string(),
                        "syncing" => s.status.yellow().to_string(),
                        _ => s.status.red().to_string(),
                    };
                    format!(
                        "  {:<5} {:>6}  {:>8}  {:>9}  {:>5}  {:>4}  {:>4}  {:<10}",
                        format!("v{}", vid),
                        api_port,
                        s.block_height,
                        s.finalised_height,
                        s.finality_lag_blocks,
                        s.last_block_age_secs
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| "-".into()),
                        s.mempool_size,
                        status_color
                    )
                }
                Err(_) => format!(
                    "  {:<5} {:>6}  {:>8}  {:>9}  {:>5}  {:>4}  {:>4}  {}",
                    format!("v{}", vid),
                    api_port,
                    "?",
                    "?",
                    "?",
                    "?",
                    "?",
                    "bad-json".yellow()
                ),
            },
            _ => format!(
                "  {:<5} {:>6}  {:>8}  {:>9}  {:>5}  {:>4}  {:>4}  {}",
                format!("v{}", vid),
                api_port,
                "-",
                "-",
                "-",
                "-",
                "-",
                "down".red()
            ),
        };
        println!("{}", line);
    }

    // Cross-cluster sanity: spread of heights and any fork hints (state
    // root divergence is checked at /api/status; we spot-check the spread).
    if !heights.is_empty() {
        let min = heights.iter().min().copied().unwrap_or(0);
        let max = heights.iter().max().copied().unwrap_or(0);
        println!();
        let spread = max - min;
        let verdict = if spread <= 1 {
            "in lockstep".green()
        } else if spread <= 5 {
            "minor lag".yellow()
        } else {
            "DIVERGENT".red().bold()
        };
        println!(
            "  cluster:  height spread {} ({}–{}) — {}",
            spread, min, max, verdict
        );
    }
    Ok(())
}

fn cmd_testnet_down(dir: &str) -> Result<()> {
    use std::path::PathBuf;

    let root = PathBuf::from(dir);
    let pid_dir = root.join("pids");
    if !pid_dir.exists() {
        println!(
            "  {} No pids/ directory under {} — nothing recorded to stop.",
            "".bold(),
            dir
        );
        return Ok(());
    }

    let mut count = 0usize;
    for entry in std::fs::read_dir(&pid_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("pid") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(pid) = content.trim().parse::<i32>() {
                let killed = unsafe { libc::kill(pid, libc::SIGTERM) };
                if killed == 0 {
                    count += 1;
                    println!(
                        "  Stopped {} (pid={})",
                        path.file_stem().and_then(|s| s.to_str()).unwrap_or("?"),
                        pid
                    );
                }
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    println!();
    println!("  {} Stopped {} node(s)", "\u{2714}".green().bold(), count);
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
    println!(
        "  {}  {}",
        "Chain:".truecolor(140, 150, 170),
        config.chain_params.chain_id.white().bold()
    );
    println!(
        "  {}  {} validators",
        "Vals: ".truecolor(140, 150, 170),
        config.validators.len().to_string().cyan()
    );
    println!(
        "  {}  {} accounts",
        "Accts:".truecolor(140, 150, 170),
        config.accounts.len().to_string().cyan()
    );
    println!(
        "  {}  {} objects",
        "Objs: ".truecolor(140, 150, 170),
        config.objects.len().to_string().cyan()
    );
    println!(
        "  {}  {}",
        "Root: ".truecolor(140, 150, 170),
        hex::encode(result.state_root).truecolor(100, 110, 130)
    );
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
    println!(
        "  {}  {}",
        "Chain ID:".truecolor(140, 150, 170),
        config.chain_params.chain_id.white().bold()
    );
    println!(
        "  {}  {} ms",
        "Block:   ".truecolor(140, 150, 170),
        config.chain_params.block_interval_ms
    );
    println!(
        "  {}  {}",
        "Time:    ".truecolor(140, 150, 170),
        config.genesis_time.truecolor(180, 190, 200)
    );

    // Tokenomics
    println!();
    println!("  {}", "Tokenomics".bold());
    println!(
        "  {}  {} EVAP",
        "Supply:  ".truecolor(140, 150, 170),
        config.tokenomics.total_supply.to_string().green().bold()
    );
    println!(
        "  {}  {}%",
        "Fee Brn: ".truecolor(140, 150, 170),
        config.tokenomics.fee_burn_rate
    );
    println!(
        "  {}  {} EVAP",
        "Block Rw:".truecolor(140, 150, 170),
        config.tokenomics.block_reward
    );

    // Validators
    println!();
    println!("  {} ({})", "Validators".bold(), config.validators.len());
    println!("  {}", separator());
    for v in &config.validators {
        let addr_hex = hex::encode(v.address);
        let addr_short = format!("{}...", &addr_hex[..16]);
        let bls = v.bls_public_key.as_deref().unwrap_or("none");
        let bls_short = if bls.len() > 16 {
            format!("{}...", &bls[..16])
        } else {
            bls.to_string()
        };
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
        println!(
            "  {} ({})",
            "Bootstrap Peers".bold(),
            config.bootstrap_peers.len()
        );
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
    println!(
        "  {}  #{}",
        "Block: ".truecolor(140, 150, 170),
        "0".cyan().bold()
    );
    println!(
        "  {}  {}",
        "Hash:  ".truecolor(140, 150, 170),
        block_hash.truecolor(100, 110, 130)
    );
    println!(
        "  {}  {}",
        "Root:  ".truecolor(140, 150, 170),
        hex::encode(result.state_root).truecolor(100, 110, 130)
    );
    println!(
        "  {}  {} accounts, {} validators, {} objects",
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
        coordinator_pk: None,
        coordinator_signature: None,
    };

    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(output, &json).with_context(|| format!("Failed to write {}", output))?;

    if json_mode {
        println!("{}", json);
    } else {
        println!(
            "  {} Genesis config created at {}",
            "\u{2714}".green().bold(),
            output
        );
        println!("  Chain ID:     {}", chain_id.white().bold());
        println!("  Supply:       {} EVAP", total_supply.to_string().green());
        println!("  Block time:   {} ms", block_interval);
        println!("  Min stake:    {}", min_stake);
        println!();
        println!(
            "  Next: add validators with `evaporchain genesis add-validator {}`",
            output
        );
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

    let json = std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path))?;
    let mut config: GenesisConfig =
        serde_json::from_str(&json).with_context(|| "Failed to parse genesis config")?;

    let next_id = config.validators.iter().map(|v| v.id).max().unwrap_or(0) + 1;

    let mut addr = [0u8; 32];
    addr[0] = next_id as u8;

    let bls_pk = if let Some(kp) = keys_path {
        let kf = std::fs::read_to_string(kp)
            .with_context(|| format!("Failed to read keys file {}", kp))?;
        let bundle: serde_json::Value = serde_json::from_str(&kf)?;
        bundle
            .get("bls")
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
        println!(
            "  {} Validator added to {}",
            "\u{2714}".green().bold(),
            path
        );
        println!("  ID:     {}", next_id.to_string().cyan().bold());
        println!("  Name:   {}", name.white().bold());
        println!("  Stake:  {} EVAP", stake.to_string().green());
        println!(
            "  Addr:   {}",
            hex::encode(&addr[..8]).truecolor(100, 110, 130)
        );
        if let Some(ref pk) = bls_pk {
            println!(
                "  BLS:    {}...",
                &pk[..32].to_string().truecolor(100, 110, 130)
            );
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

    let json = std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path))?;
    let mut config: GenesisConfig =
        serde_json::from_str(&json).with_context(|| "Failed to parse genesis config")?;

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
            println!(
                "  was:  {}...",
                &p[..32.min(p.len())].truecolor(100, 110, 130)
            );
        }
        println!("  now:  {}...", &new_pk[..32].truecolor(100, 110, 130));
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

    let json = std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path))?;
    let mut config: GenesisConfig =
        serde_json::from_str(&json).with_context(|| "Failed to parse genesis config")?;

    let byte = address_byte.unwrap_or_else(|| {
        let max_byte = config
            .accounts
            .iter()
            .map(|a| a.address[0])
            .max()
            .unwrap_or(0);
        if max_byte < 0xFE {
            max_byte + 1
        } else {
            0xFE
        }
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
        println!(
            "  Addr:    {}",
            hex::encode(&addr[..8]).truecolor(100, 110, 130)
        );
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
        println!(
            "  {} Genesis is valid and ready for deployment",
            "\u{2714}".green().bold()
        );
        println!();
        println!(
            "  {}  {}",
            "Chain:  ".truecolor(140, 150, 170),
            config.chain_params.chain_id.white().bold()
        );
        println!(
            "  {}  {}",
            "Hash:   ".truecolor(140, 150, 170),
            hex::encode(block_hash).truecolor(100, 110, 130)
        );
        println!(
            "  {}  {}",
            "Config: ".truecolor(140, 150, 170),
            hex::encode(config_hash).truecolor(100, 110, 130)
        );
        println!(
            "  {}  {}",
            "Root:   ".truecolor(140, 150, 170),
            hex::encode(result.state_root).truecolor(100, 110, 130)
        );
        println!();
        println!(
            "  {}  {} validators, {} accounts",
            "State: ".truecolor(140, 150, 170),
            config.validators.len().to_string().cyan(),
            config.accounts.len().to_string().green(),
        );

        let total_alloc: u64 = config.accounts.iter().map(|a| a.balance).sum();
        let remaining = config.tokenomics.total_supply.saturating_sub(total_alloc);
        println!(
            "  {}  {} / {} EVAP allocated ({} unallocated)",
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

// ──────────────────────────── Genesis Ceremony (multi-party) ────────────
//
// Three-step sealed-envelope flow:
//   1. Each operator: `genesis contribute` → produces a signed envelope
//      binding their validator entry to (chain_id, genesis_time, nonce).
//   2. Coordinator: `genesis ceremony` → ingests every envelope, verifies
//      signatures + BLS PoP, builds a deterministic genesis.json, and emits
//      a transcript that names every contribution by its body hash.
//   3. Any operator: `genesis verify-ceremony` → replays the bundle and
//      checks the on-disk genesis is the bit-exact product of the inputs.

/// Canonical bytes the operator and coordinator both hash and sign. We
/// bind every field that influences the resulting GenesisValidator entry
/// PLUS the ceremony anchors (chain_id / genesis_time / nonce) so a
/// single envelope cannot be replayed across distinct ceremonies.
#[derive(Clone, Serialize, Deserialize)]
struct ContributionBody {
    chain_id: String,
    genesis_time: String,
    /// 32-byte hex; identical across all envelopes in a single ceremony.
    ceremony_nonce: String,
    validator_id: u64,
    name: String,
    stake: u64,
    /// 32-byte hex. Derived from `--address-byte` (or `validator_id` if not
    /// supplied) so the ceremony is fully reproducible.
    address: String,
    /// 48-byte BLS12-381 G1 public key (compressed, hex).
    bls_public_key: String,
    p2p_address: Option<String>,
    balance: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct ContributionEnvelope {
    body: ContributionBody,
    /// blake3 hash over the canonical body bytes (hex).
    body_hash: String,
    /// 1952-byte ML-DSA Dilithium3 public key (hex).
    ml_dsa_public_key: String,
    /// ML-DSA signature over `body_hash` raw bytes (hex).
    ml_dsa_signature: String,
    /// BLS PoP over the operator's bls_public_key (hex). Confirms the
    /// operator actually holds the BLS secret behind `bls_public_key`.
    bls_pop: String,
}

fn canonical_body_bytes(body: &ContributionBody) -> Result<Vec<u8>> {
    // serde_json with sort_keys would be ideal but isn't available; the
    // struct field order is fixed at the type level so to_vec produces
    // deterministic output across hosts as long as field types stay stable.
    serde_json::to_vec(body).context("failed to serialize contribution body")
}

#[allow(clippy::too_many_arguments)]
fn cmd_genesis_contribute(
    keys_path: &str,
    validator_id: u64,
    name: &str,
    stake: u64,
    p2p: Option<&str>,
    balance: u64,
    address_byte: Option<u8>,
    chain_id: &str,
    genesis_time: &str,
    ceremony_nonce_hex: &str,
    out_path: &str,
    json_mode: bool,
) -> Result<()> {
    use evaporchain_crypto::signatures::{BlsKeypair, MlDsaKeypair, Signer};

    // Validate ceremony nonce is exactly 32 bytes hex so a typo can't slip
    // through and split the ceremony silently.
    let nonce_bytes = hex::decode(ceremony_nonce_hex.trim_start_matches("0x"))
        .with_context(|| "ceremony_nonce must be hex")?;
    if nonce_bytes.len() != 32 {
        anyhow::bail!("ceremony_nonce must decode to exactly 32 bytes");
    }
    let nonce_canonical = hex::encode(&nonce_bytes);

    let keys_json = std::fs::read_to_string(keys_path)
        .with_context(|| format!("failed to read keys file {}", keys_path))?;
    let bundle: serde_json::Value = serde_json::from_str(&keys_json)
        .with_context(|| "keys file is not valid JSON")?;

    let bls_sk_hex = bundle
        .get("bls")
        .and_then(|b| b.get("secret_key"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("keys file missing bls.secret_key"))?;
    let bls_sk_bytes = hex::decode(bls_sk_hex).context("bls.secret_key not hex")?;

    let mldsa_pk_hex = bundle
        .get("ml_dsa")
        .and_then(|b| b.get("public_key"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("keys file missing ml_dsa.public_key"))?;
    let mldsa_sk_hex = bundle
        .get("ml_dsa")
        .and_then(|b| b.get("secret_key"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("keys file missing ml_dsa.secret_key"))?;
    let mldsa_pk_bytes = hex::decode(mldsa_pk_hex).context("ml_dsa.public_key not hex")?;
    let mldsa_sk_bytes = hex::decode(mldsa_sk_hex).context("ml_dsa.secret_key not hex")?;

    let bls_kp =
        BlsKeypair::from_secret_bytes(&bls_sk_bytes).context("failed to parse BLS secret")?;
    let mldsa_kp = MlDsaKeypair::from_bytes(&mldsa_pk_bytes, &mldsa_sk_bytes)
        .context("failed to parse ML-DSA keypair")?;

    let bls_pk_hex = hex::encode(&bls_kp.public_key_bytes().0);

    let mut address = [0u8; 32];
    address[0] = address_byte.unwrap_or(validator_id as u8);
    let body = ContributionBody {
        chain_id: chain_id.to_string(),
        genesis_time: genesis_time.to_string(),
        ceremony_nonce: nonce_canonical,
        validator_id,
        name: name.to_string(),
        stake,
        address: hex::encode(address),
        bls_public_key: bls_pk_hex.clone(),
        p2p_address: p2p.map(str::to_string),
        balance,
    };

    let body_bytes = canonical_body_bytes(&body)?;
    let body_hash = evaporchain_crypto::blake3_hash(&body_bytes);
    let body_hash_hex = hex::encode(body_hash);

    let mldsa_sig = mldsa_kp.sign(&body_hash);
    let bls_pop = bls_kp.proof_of_possession();

    let envelope = ContributionEnvelope {
        body,
        body_hash: body_hash_hex,
        ml_dsa_public_key: hex::encode(mldsa_kp.public_key()),
        ml_dsa_signature: hex::encode(&mldsa_sig),
        bls_pop: hex::encode(&bls_pop.0),
    };

    let pretty = serde_json::to_string_pretty(&envelope)?;
    std::fs::write(out_path, &pretty)
        .with_context(|| format!("failed to write envelope to {}", out_path))?;

    if json_mode {
        println!("{}", pretty);
    } else {
        print_header("Contribution Envelope");
        println!(
            "  {} Envelope written to {}",
            "\u{2714}".green().bold(),
            out_path
        );
        println!(
            "  {}  validator_id={} name={} stake={}",
            "Body:".truecolor(140, 150, 170),
            envelope.body.validator_id.to_string().cyan(),
            envelope.body.name.white(),
            envelope.body.stake.to_string().green(),
        );
        println!(
            "  {}  {}",
            "Hash:".truecolor(140, 150, 170),
            envelope.body_hash.truecolor(100, 110, 130)
        );
        println!(
            "  {}  {}",
            "BLS pk:".truecolor(140, 150, 170),
            envelope.body.bls_public_key[..32].truecolor(100, 110, 130)
        );
        println!();
        println!("  Share this file with the ceremony coordinator.");
    }
    Ok(())
}

/// Validate one envelope against the ceremony anchors. Returns the parsed
/// envelope on success.
fn verify_envelope(
    env: &ContributionEnvelope,
    chain_id: &str,
    genesis_time: &str,
    ceremony_nonce_hex: &str,
) -> Result<()> {
    use evaporchain_crypto::signatures::{
        BlsPublicKey, BlsSignature, BlsVerifier, MlDsaVerifier, Verifier,
    };

    if env.body.chain_id != chain_id {
        anyhow::bail!(
            "chain_id mismatch: envelope={}, ceremony={}",
            env.body.chain_id,
            chain_id
        );
    }
    if env.body.genesis_time != genesis_time {
        anyhow::bail!(
            "genesis_time mismatch: envelope={}, ceremony={}",
            env.body.genesis_time,
            genesis_time
        );
    }
    if env.body.ceremony_nonce.to_lowercase() != ceremony_nonce_hex.to_lowercase() {
        anyhow::bail!(
            "ceremony_nonce mismatch: envelope={}, ceremony={}",
            env.body.ceremony_nonce,
            ceremony_nonce_hex
        );
    }

    let body_bytes = canonical_body_bytes(&env.body)?;
    let computed = hex::encode(evaporchain_crypto::blake3_hash(&body_bytes));
    if computed != env.body_hash {
        anyhow::bail!(
            "body_hash mismatch: declared={}, recomputed={}",
            env.body_hash,
            computed
        );
    }
    let body_hash_bytes = hex::decode(&env.body_hash).context("body_hash not hex")?;

    let mldsa_pk = hex::decode(&env.ml_dsa_public_key).context("ml_dsa_public_key not hex")?;
    let mldsa_sig = hex::decode(&env.ml_dsa_signature).context("ml_dsa_signature not hex")?;
    if !MlDsaVerifier::verify(&body_hash_bytes, &mldsa_sig, &mldsa_pk) {
        anyhow::bail!(
            "ML-DSA signature failed for validator_id={}",
            env.body.validator_id
        );
    }

    let bls_pk_bytes = hex::decode(&env.body.bls_public_key).context("bls_public_key not hex")?;
    let bls_pop_bytes = hex::decode(&env.bls_pop).context("bls_pop not hex")?;
    let pk = BlsPublicKey(bls_pk_bytes);
    let pop = BlsSignature(bls_pop_bytes);
    if !BlsVerifier::verify_proof_of_possession(&pk, &pop) {
        anyhow::bail!(
            "BLS proof-of-possession failed for validator_id={}",
            env.body.validator_id
        );
    }
    Ok(())
}

fn load_envelopes(dir: &str) -> Result<Vec<(String, ContributionEnvelope)>> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read contribution dir {}", dir))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let env: ContributionEnvelope = serde_json::from_str(&text)
            .with_context(|| format!("envelope {} is not valid JSON", path.display()))?;
        out.push((path.display().to_string(), env));
    }
    if out.is_empty() {
        anyhow::bail!("no *.json envelopes found in {}", dir);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn cmd_genesis_ceremony(
    contributions_dir: &str,
    chain_id: &str,
    genesis_time: &str,
    ceremony_nonce_hex: &str,
    total_supply: u64,
    block_interval: u64,
    min_stake: u64,
    out_path: &str,
    json_mode: bool,
) -> Result<()> {
    use evaporchain_types::genesis::*;

    let nonce_bytes = hex::decode(ceremony_nonce_hex.trim_start_matches("0x"))
        .with_context(|| "ceremony_nonce must be hex")?;
    if nonce_bytes.len() != 32 {
        anyhow::bail!("ceremony_nonce must decode to exactly 32 bytes");
    }
    let nonce_canonical = hex::encode(&nonce_bytes);

    let mut envelopes = load_envelopes(contributions_dir)?;

    // Verify each envelope; collect (path, error) for clear reporting.
    let mut errors: Vec<(String, String)> = Vec::new();
    for (path, env) in &envelopes {
        if let Err(e) = verify_envelope(env, chain_id, genesis_time, &nonce_canonical) {
            errors.push((path.clone(), e.to_string()));
        }
    }
    if !errors.is_empty() {
        if json_mode {
            let out = serde_json::json!({
                "ok": false,
                "errors": errors.iter().map(|(p, e)| serde_json::json!({"path": p, "error": e})).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("  {} Ceremony failed: invalid envelopes:", "\u{2718}".red().bold());
            for (path, err) in &errors {
                println!("    - {}: {}", path.cyan(), err.red());
            }
        }
        anyhow::bail!("{} envelope verification error(s)", errors.len());
    }

    // Reject duplicate validator ids.
    let mut seen_ids = std::collections::HashSet::new();
    for (_, env) in &envelopes {
        if !seen_ids.insert(env.body.validator_id) {
            anyhow::bail!("duplicate validator_id={}", env.body.validator_id);
        }
    }

    // Deterministic order — sort by validator_id ascending.
    envelopes.sort_by_key(|(_, e)| e.body.validator_id);

    let validators: Vec<GenesisValidator> = envelopes
        .iter()
        .map(|(_, e)| {
            let mut addr = [0u8; 32];
            let addr_bytes = hex::decode(&e.body.address).unwrap_or_default();
            let n = addr_bytes.len().min(32);
            addr[..n].copy_from_slice(&addr_bytes[..n]);
            GenesisValidator {
                id: e.body.validator_id,
                name: e.body.name.clone(),
                stake: e.body.stake,
                address: addr,
                bls_public_key: Some(e.body.bls_public_key.clone()),
                p2p_address: e.body.p2p_address.clone(),
            }
        })
        .collect();

    let accounts: Vec<GenesisAccount> = envelopes
        .iter()
        .map(|(_, e)| {
            let mut addr = [0u8; 32];
            let addr_bytes = hex::decode(&e.body.address).unwrap_or_default();
            let n = addr_bytes.len().min(32);
            addr[..n].copy_from_slice(&addr_bytes[..n]);
            GenesisAccount {
                address: addr,
                balance: e.body.balance,
                label: format!("Validator-{}", e.body.name),
            }
        })
        .collect();

    let chain_params = ChainParams {
        chain_id: chain_id.to_string(),
        block_interval_ms: block_interval,
        min_validator_stake: min_stake,
        ..Default::default()
    };
    let tokenomics = Tokenomics {
        total_supply,
        ..Default::default()
    };

    let config = GenesisConfig {
        chain_params,
        tokenomics,
        genesis_time: genesis_time.to_string(),
        validators,
        accounts,
        objects: Vec::new(),
        bootstrap_peers: Vec::new(),
        trusted_checkpoint: None,
        coordinator_pk: None,
        coordinator_signature: None,
    };

    if let Err(errs) = config.validate() {
        if json_mode {
            let out = serde_json::json!({ "ok": false, "validation_errors": errs });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!(
                "  {} Built genesis failed validation:",
                "\u{2718}".red().bold()
            );
            for e in &errs {
                println!("    - {}", e.red());
            }
        }
        anyhow::bail!("{} validation error(s)", errs.len());
    }

    let config_bytes = serde_json::to_vec(&config).unwrap_or_default();
    let config_hash = evaporchain_crypto::blake3_hash(&config_bytes);
    let pretty = serde_json::to_string_pretty(&config)?;
    std::fs::write(out_path, &pretty)
        .with_context(|| format!("failed to write genesis to {}", out_path))?;

    // Transcript: every contribution by its body_hash + the produced
    // config_hash. Anyone replaying the ceremony can compare.
    let transcript_path = format!("{}.transcript.json", out_path);
    let transcript = serde_json::json!({
        "chain_id": chain_id,
        "genesis_time": genesis_time,
        "ceremony_nonce": nonce_canonical,
        "config_hash": hex::encode(config_hash),
        "validator_count": config.validators.len(),
        "contributions": envelopes.iter().map(|(path, e)| serde_json::json!({
            "path": path,
            "validator_id": e.body.validator_id,
            "body_hash": e.body_hash,
            "bls_public_key": e.body.bls_public_key,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(
        &transcript_path,
        serde_json::to_string_pretty(&transcript)?,
    )
    .with_context(|| format!("failed to write transcript to {}", transcript_path))?;

    if json_mode {
        let out = serde_json::json!({
            "ok": true,
            "genesis_path": out_path,
            "transcript_path": transcript_path,
            "config_hash": hex::encode(config_hash),
            "validators": config.validators.len(),
            "accounts": config.accounts.len(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_header("Genesis Ceremony Complete");
        println!(
            "  {} Genesis assembled from {} contributions",
            "\u{2714}".green().bold(),
            config.validators.len()
        );
        println!(
            "  {}  {}",
            "Genesis:   ".truecolor(140, 150, 170),
            out_path.white()
        );
        println!(
            "  {}  {}",
            "Transcript:".truecolor(140, 150, 170),
            transcript_path.white()
        );
        println!(
            "  {}  {}",
            "Hash:      ".truecolor(140, 150, 170),
            hex::encode(config_hash).truecolor(100, 110, 130)
        );
        println!();
        println!("  Distribute both files. Each operator should run:");
        println!(
            "    evaporchain genesis verify-ceremony --contributions {} \\\n      --genesis {} --transcript {}",
            contributions_dir, out_path, transcript_path
        );
    }

    Ok(())
}

fn cmd_genesis_verify_ceremony(
    contributions_dir: &str,
    genesis_path: &str,
    transcript_path: &str,
    json_mode: bool,
) -> Result<()> {
    let transcript_text = std::fs::read_to_string(transcript_path)
        .with_context(|| format!("failed to read transcript {}", transcript_path))?;
    let transcript: serde_json::Value = serde_json::from_str(&transcript_text)
        .with_context(|| "transcript is not valid JSON")?;

    let chain_id = transcript
        .get("chain_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("transcript missing chain_id"))?;
    let genesis_time = transcript
        .get("genesis_time")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("transcript missing genesis_time"))?;
    let ceremony_nonce = transcript
        .get("ceremony_nonce")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("transcript missing ceremony_nonce"))?;
    let declared_hash = transcript
        .get("config_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("transcript missing config_hash"))?;

    let envelopes = load_envelopes(contributions_dir)?;
    for (path, env) in &envelopes {
        verify_envelope(env, chain_id, genesis_time, ceremony_nonce)
            .with_context(|| format!("envelope {} failed verification", path))?;
    }

    let genesis_text = std::fs::read_to_string(genesis_path)
        .with_context(|| format!("failed to read genesis {}", genesis_path))?;
    let config: GenesisConfig =
        serde_json::from_str(&genesis_text).context("genesis file is not valid JSON")?;
    let recomputed_bytes = serde_json::to_vec(&config).unwrap_or_default();
    let recomputed_hash = hex::encode(evaporchain_crypto::blake3_hash(&recomputed_bytes));

    let on_disk_envelope_ids: std::collections::HashSet<u64> =
        envelopes.iter().map(|(_, e)| e.body.validator_id).collect();
    let on_disk_validator_ids: std::collections::HashSet<u64> =
        config.validators.iter().map(|v| v.id).collect();
    let mismatches: Vec<u64> = on_disk_envelope_ids
        .symmetric_difference(&on_disk_validator_ids)
        .copied()
        .collect();

    let hash_match = recomputed_hash.eq_ignore_ascii_case(declared_hash);
    let id_match = mismatches.is_empty();

    if json_mode {
        let out = serde_json::json!({
            "ok": hash_match && id_match,
            "config_hash_match": hash_match,
            "validator_id_match": id_match,
            "expected_config_hash": declared_hash,
            "recomputed_config_hash": recomputed_hash,
            "id_mismatches": mismatches,
            "envelope_count": envelopes.len(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_header("Ceremony Verification");
        if hash_match && id_match {
            println!(
                "  {} All {} envelopes verified; genesis hash matches transcript",
                "\u{2714}".green().bold(),
                envelopes.len()
            );
            println!(
                "  {}  {}",
                "Hash:".truecolor(140, 150, 170),
                recomputed_hash.truecolor(100, 110, 130)
            );
        } else {
            println!("  {} Verification failed", "\u{2718}".red().bold());
            if !hash_match {
                println!("    expected: {}", declared_hash.red());
                println!("    got:      {}", recomputed_hash.red());
            }
            if !id_match {
                println!(
                    "    validator_id set differs (envelope vs genesis): {:?}",
                    mismatches
                );
            }
        }
    }

    if !(hash_match && id_match) {
        anyhow::bail!("ceremony verification failed");
    }
    Ok(())
}

// ──────────────────────────── DA light-client verifier ──────────────────
//
// Closes the audit-memo gap "DA sampling over network with real 2D
// erasure coding". The DA library ships a `LightClientSampler` generic
// over a `CellSource` trait; the trait has been waiting for a real HTTP
// implementation that hits a node's `/api/da/cell/:block/:row/:col`
// endpoint instead of a local mock. This is that implementation.
//
// Operator flow:
//   evaporchain da verify --node http://node:8080 --block 200
//
// Catches three classes of fault:
//   1. Data unavailability — node announces a data_root but can't serve
//      the underlying cells (404 or transport error per cell).
//   2. Fabricated cells — node serves a CellProof whose Merkle path
//      doesn't reconstruct to the published row/col root.
//   3. Header drift — node's `/api/da/header/:block` no longer matches
//      what was committed on-chain (handled at a higher layer; this
//      command trusts the served header for now).

/// Round-robins per-cell across one or more node URLs. Each fetch_cell
/// call increments an atomic cursor and picks the next base URL — the
/// peer_id returned to the sampler is that exact URL, so faulty-peer
/// reports name the specific node that served a bad cell.
struct HttpCellSource {
    bases: Vec<String>,
    cursor: std::sync::atomic::AtomicUsize,
    client: reqwest::blocking::Client,
}

impl HttpCellSource {
    fn new(bases: &[String]) -> anyhow::Result<Self> {
        if bases.is_empty() {
            anyhow::bail!("HttpCellSource requires at least one --node URL");
        }
        let bases: Vec<String> = bases
            .iter()
            .map(|b| b.trim_end_matches('/').to_string())
            .collect();
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("build reqwest client for HttpCellSource")?;
        Ok(Self {
            bases,
            cursor: std::sync::atomic::AtomicUsize::new(0),
            client,
        })
    }

    /// Pick the next base URL for outgoing requests. Round-robin under
    /// concurrency-safe atomic increment.
    fn next_base(&self) -> &str {
        let i = self.cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        &self.bases[i % self.bases.len()]
    }

    /// First base URL — used for one-shot reads (header, on-chain block
    /// record) where round-robining doesn't add value.
    fn primary_base(&self) -> &str {
        &self.bases[0]
    }

    /// Fetch the on-chain `data_root` for `block`. Tri-state result:
    ///   - `BlockLookup::NotFound` — HTTP 404 (block aged out of ring).
    ///   - `BlockLookup::NoDataRoot` — block exists but its `data_root`
    ///     field is null (sentinel/no-DA-enforcement).
    ///   - `BlockLookup::Root(r)` — block exists with a 32-byte root.
    /// Other transport / parse failures bubble up via `Err`.
    fn fetch_block_data_root(&self, block: u64) -> anyhow::Result<BlockLookup> {
        let url = format!("{}/api/block/{}", self.primary_base(), block);
        let resp = self.client.get(&url).send().context("GET /api/block/:N")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(BlockLookup::NotFound);
        }
        if !resp.status().is_success() {
            anyhow::bail!(
                "{} returned {}: {}",
                url,
                resp.status(),
                resp.text().unwrap_or_default()
            );
        }
        let v: serde_json::Value = resp.json().context("parse block JSON")?;
        let Some(root_str) = v.get("data_root").and_then(|x| x.as_str()) else {
            return Ok(BlockLookup::NoDataRoot);
        };
        let bytes =
            hex::decode(root_str.trim_start_matches("0x")).context("decode data_root hex")?;
        if bytes.len() != 32 {
            anyhow::bail!("on-chain data_root is not 32 bytes");
        }
        let mut b = [0u8; 32];
        b.copy_from_slice(&bytes);
        Ok(BlockLookup::Root(b))
    }

    fn fetch_header(
        &self,
        block: u64,
    ) -> anyhow::Result<evaporchain_da::block_da_2d::BlockDA2DHeader> {
        let url = format!("{}/api/da/header/{}", self.primary_base(), block);
        let resp = self.client.get(&url).send().context("GET /api/da/header")?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "{} returned {}: {}",
                url,
                resp.status(),
                resp.text().unwrap_or_default()
            );
        }
        let v: serde_json::Value = resp.json().context("parse header JSON")?;
        let parse32 = |key: &str| -> anyhow::Result<[u8; 32]> {
            let s = v
                .get(key)
                .and_then(|x| x.as_str())
                .ok_or_else(|| anyhow::anyhow!("header missing `{}`", key))?;
            let bytes = hex::decode(s).with_context(|| format!("decode hex for `{}`", key))?;
            if bytes.len() != 32 {
                anyhow::bail!("`{}` is not 32 bytes", key);
            }
            let mut b = [0u8; 32];
            b.copy_from_slice(&bytes);
            Ok(b)
        };
        let parse_vec32 = |key: &str| -> anyhow::Result<Vec<[u8; 32]>> {
            let arr = v
                .get(key)
                .and_then(|x| x.as_array())
                .ok_or_else(|| anyhow::anyhow!("header missing `{}` array", key))?;
            arr.iter()
                .map(|x| {
                    let s = x.as_str().ok_or_else(|| {
                        anyhow::anyhow!("non-string in `{}`", key)
                    })?;
                    let bytes = hex::decode(s)
                        .with_context(|| format!("decode hex in `{}`", key))?;
                    if bytes.len() != 32 {
                        anyhow::bail!("entry in `{}` is not 32 bytes", key);
                    }
                    let mut b = [0u8; 32];
                    b.copy_from_slice(&bytes);
                    Ok(b)
                })
                .collect()
        };
        let extended_dim = v
            .get("extended_dim")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| anyhow::anyhow!("header missing extended_dim"))?
            as usize;
        let original_dim = v
            .get("original_dim")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| anyhow::anyhow!("header missing original_dim"))?
            as usize;
        let cell_size = v
            .get("cell_size")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| anyhow::anyhow!("header missing cell_size"))?
            as usize;
        let original_len = v
            .get("original_len")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| anyhow::anyhow!("header missing original_len"))?
            as usize;
        Ok(evaporchain_da::block_da_2d::BlockDA2DHeader {
            data_root: parse32("data_root")?,
            row_roots: parse_vec32("row_roots")?,
            col_roots: parse_vec32("col_roots")?,
            extended_dim,
            original_dim,
            cell_size,
            original_len,
            data_hash: parse32("data_hash")?,
            nmt_root: None,
            blob_commitments: vec![],
        })
    }
}

impl evaporchain_da::light_client::CellSource for HttpCellSource {
    fn fetch_cell(
        &self,
        height: u64,
        row: usize,
        col: usize,
    ) -> Result<(String, evaporchain_da::commitments::CellProof), evaporchain_da::light_client::CellSourceError>
    {
        use evaporchain_da::light_client::CellSourceError;
        let base = self.next_base();
        let url = format!("{}/api/da/cell/{}/{}/{}", base, height, row, col);
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| CellSourceError::Transport(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CellSourceError::NotFound);
        }
        if !resp.status().is_success() {
            return Err(CellSourceError::Transport(format!(
                "{} → {}",
                url,
                resp.status()
            )));
        }
        let v: serde_json::Value = resp
            .json()
            .map_err(|e| CellSourceError::Transport(format!("parse JSON: {}", e)))?;

        // Helper closures push parse errors through the same Transport
        // arm — the sampler treats Transport as "peer is unreachable or
        // misbehaving" which is the right severity for malformed JSON.
        let hex_bytes = |key: &str| -> Result<Vec<u8>, CellSourceError> {
            let s = v
                .get(key)
                .and_then(|x| x.as_str())
                .ok_or_else(|| CellSourceError::Transport(format!("missing `{}`", key)))?;
            hex::decode(s).map_err(|e| CellSourceError::Transport(e.to_string()))
        };
        let hex_32 = |key: &str| -> Result<[u8; 32], CellSourceError> {
            let bytes = hex_bytes(key)?;
            if bytes.len() != 32 {
                return Err(CellSourceError::Transport(format!(
                    "`{}` is not 32 bytes ({} got)",
                    key,
                    bytes.len()
                )));
            }
            let mut b = [0u8; 32];
            b.copy_from_slice(&bytes);
            Ok(b)
        };
        let hex_vec32 = |key: &str| -> Result<Vec<[u8; 32]>, CellSourceError> {
            let arr = v
                .get(key)
                .and_then(|x| x.as_array())
                .ok_or_else(|| CellSourceError::Transport(format!("missing `{}`", key)))?;
            arr.iter()
                .map(|x| {
                    let s = x.as_str().ok_or_else(|| {
                        CellSourceError::Transport(format!("non-string in `{}`", key))
                    })?;
                    let bytes = hex::decode(s)
                        .map_err(|e| CellSourceError::Transport(e.to_string()))?;
                    if bytes.len() != 32 {
                        return Err(CellSourceError::Transport(format!(
                            "entry in `{}` is not 32 bytes",
                            key
                        )));
                    }
                    let mut b = [0u8; 32];
                    b.copy_from_slice(&bytes);
                    Ok(b)
                })
                .collect()
        };

        let proof = evaporchain_da::commitments::CellProof {
            cell_data: hex_bytes("cell_data")?,
            row,
            col,
            cell_hash: hex_32("cell_hash")?,
            row_root: hex_32("row_root")?,
            col_root: hex_32("col_root")?,
            row_siblings: hex_vec32("row_proof_siblings")?,
            col_siblings: hex_vec32("col_proof_siblings")?,
            data_root: hex_32("data_root")?,
        };
        Ok((base.to_string(), proof))
    }
}

/// Tri-state result from `HttpCellSource::fetch_block_data_root`.
enum BlockLookup {
    NotFound,
    NoDataRoot,
    Root([u8; 32]),
}

/// Result of the optional on-chain attestation cross-check.
enum ChainAttestation {
    /// On-chain `data_root` matches the served 2D header's `data_root`.
    Verified,
    /// Operator passed `--skip-chain-attestation`; cross-check skipped.
    Skipped,
    /// Block has no on-chain `data_root` (sentinel/no-DA-enforcement).
    NoDataRoot,
    /// Block exists but isn't in the node's in-memory ring.
    BlockNotInRing,
    /// On-chain root and served header disagree. Hard fail.
    Mismatch { on_chain: [u8; 32], served: [u8; 32] },
}

/// Output of a DA verify run. Tests call `da_verify_inner` directly to
/// inspect this structure; `cmd_da_verify` wraps it with the exit-code
/// + pretty-printing logic.
#[derive(Debug)]
pub struct DaVerifyOutcome {
    pub attestation_label: &'static str,
    pub passes: bool,
    pub samples_requested: usize,
    pub samples_valid: usize,
    pub all_valid: bool,
    pub confidence: f64,
    pub faulty_peers: Vec<(String, String)>,
}

#[allow(clippy::too_many_arguments)]
async fn da_verify_inner(
    nodes: &[String],
    block: u64,
    samples: usize,
    threshold: f64,
    seed_hex: Option<&str>,
    skip_chain_attestation: bool,
) -> Result<DaVerifyOutcome> {
    if nodes.is_empty() {
        anyhow::bail!("at least one --node URL is required");
    }
    let samples = samples.clamp(1, 256);
    if !(0.0 < threshold && threshold < 1.0) {
        anyhow::bail!("threshold must be in (0.0, 1.0); got {}", threshold);
    }

    let nodes_owned: Vec<String> = nodes.to_vec();
    let seed_owned: Option<Vec<u8>> = match seed_hex {
        Some(s) => Some(
            hex::decode(s.trim_start_matches("0x")).context("--seed must be hex")?,
        ),
        None => None,
    };

    // Run all the blocking HTTP work on a dedicated thread so we don't
    // pin the tokio runtime. The sampler itself is sync.
    let nodes_for_blocking = nodes_owned.clone();
    let result: (
        ChainAttestation,
        evaporchain_da::light_client::SamplingReport,
    ) = tokio::task::spawn_blocking(move || -> Result<_> {
        let source = HttpCellSource::new(&nodes_for_blocking)?;
        let header = source.fetch_header(block)?;

        // Chain-attestation cross-check. Hard fail on mismatch — anything
        // sampled below would be against the wrong block's data.
        let attestation = if skip_chain_attestation {
            ChainAttestation::Skipped
        } else {
            match source.fetch_block_data_root(block)? {
                BlockLookup::NotFound => ChainAttestation::BlockNotInRing,
                BlockLookup::NoDataRoot => ChainAttestation::NoDataRoot,
                BlockLookup::Root(on_chain) if on_chain == header.data_root => {
                    ChainAttestation::Verified
                }
                BlockLookup::Root(on_chain) => ChainAttestation::Mismatch {
                    on_chain,
                    served: header.data_root,
                },
            }
        };
        if let ChainAttestation::Mismatch { on_chain, served } = &attestation {
            anyhow::bail!(
                "on-chain data_root {} does not match served 2D header data_root {} — \
                 producer is publishing a header for a block whose committed \
                 data_root was something else. Aborting before sampling.",
                hex::encode(on_chain),
                hex::encode(served)
            );
        }

        let sampler = evaporchain_da::light_client::LightClientSampler::new(source);
        let seed = seed_owned
            .unwrap_or_else(|| blake3::hash(&block.to_le_bytes()).as_bytes().to_vec());
        let report = sampler.sample_block(&header, block, samples, &seed);
        Ok((attestation, report))
    })
    .await
    .map_err(|e| anyhow::anyhow!("blocking task: {}", e))??;

    let (attestation, report) = result;
    let passes = report.passes(threshold);

    let attestation_label: &'static str = match &attestation {
        ChainAttestation::Verified => "verified",
        ChainAttestation::Skipped => "skipped",
        ChainAttestation::NoDataRoot => "no-data-root",
        ChainAttestation::BlockNotInRing => "block-not-in-ring",
        ChainAttestation::Mismatch { .. } => "mismatch", // unreachable
    };

    Ok(DaVerifyOutcome {
        attestation_label,
        passes,
        samples_requested: report.results.len(),
        samples_valid: report.metrics.valid_samples,
        all_valid: report.all_valid,
        confidence: report.metrics.confidence,
        faulty_peers: report
            .faulty_peers
            .iter()
            .map(|(p, r)| (p.clone(), format!("{:?}", r)))
            .collect(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn cmd_da_verify(
    nodes: &[String],
    block: u64,
    samples: usize,
    threshold: f64,
    seed_hex: Option<&str>,
    skip_chain_attestation: bool,
    json_mode: bool,
) -> Result<()> {
    let outcome = da_verify_inner(
        nodes,
        block,
        samples,
        threshold,
        seed_hex,
        skip_chain_attestation,
    )
    .await?;

    if json_mode {
        let payload = serde_json::json!({
            "nodes": nodes,
            "block": block,
            "samples_requested": outcome.samples_requested,
            "samples_valid": outcome.samples_valid,
            "all_valid": outcome.all_valid,
            "confidence": outcome.confidence,
            "faulty_peers": outcome
                .faulty_peers
                .iter()
                .map(|(p, r)| serde_json::json!({"peer": p, "reason": r}))
                .collect::<Vec<_>>(),
            "threshold": threshold,
            "chain_attestation": outcome.attestation_label,
            "passes": outcome.passes,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_header("DA Light-Client Verification");
        println!(
            "  {} block #{} across {} node(s)",
            if outcome.passes { "✅".green() } else { "❌".red() },
            block,
            nodes.len()
        );
        for n in nodes {
            println!("  {} {}", "Node:        ".truecolor(140, 150, 170), n);
        }
        println!(
            "  {} {}",
            "Chain attest:".truecolor(140, 150, 170),
            outcome.attestation_label
        );
        println!(
            "  {} {}/{} samples valid",
            "Cells:       ".truecolor(140, 150, 170),
            outcome.samples_valid,
            outcome.samples_requested,
        );
        println!(
            "  {} {:.6} (threshold {:.6})",
            "Confidence:  ".truecolor(140, 150, 170),
            outcome.confidence,
            threshold,
        );
        if !outcome.faulty_peers.is_empty() {
            println!(
                "  {} {} peer(s) served bad cells:",
                "⚠".yellow(),
                outcome.faulty_peers.len()
            );
            for (peer, reason) in &outcome.faulty_peers {
                println!("    - {} ({})", peer, reason);
            }
        }
    }

    if !outcome.passes {
        std::process::exit(1);
    }
    Ok(())
}

// ──────────────────────────── INVENTION_STACK stamp ─────────────────────
//
// Auto-generated marker pair the stamper rewrites. Surrounding prose stays
// untouched. First-run inserts the markers immediately after the section
// heading; subsequent runs replace the body between them.
const STAMP_BEGIN: &str = "<!-- mera-gate-result:begin -->";
const STAMP_END: &str = "<!-- mera-gate-result:end -->";

fn cmd_genesis_stamp_result(
    from_json: &str,
    doc_path: &str,
    section: &str,
    dry_run: bool,
    json_mode: bool,
) -> Result<()> {
    // 1. Load the JSON payload (file path or stdin via `-`).
    let payload_text = if from_json == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read JSON payload from stdin")?;
        buf
    } else {
        std::fs::read_to_string(from_json)
            .with_context(|| format!("failed to read JSON payload at {}", from_json))?
    };
    let payload: serde_json::Value =
        serde_json::from_str(&payload_text).context("payload is not valid JSON")?;

    let decision = payload
        .get("decision")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("payload missing `decision` field"))?;
    let reasoning = payload
        .get("reasoning")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source = payload
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)");
    let pl_r2 = payload
        .get("powerlaw_r2")
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::NAN);
    let pl_slope = payload
        .get("powerlaw_slope")
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::NAN);
    let exp_r2 = payload
        .get("exponential_r2")
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::NAN);
    let exp_rate = payload
        .get("exponential_rate")
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::NAN);
    let flat_ratio = payload
        .get("flat_ratio")
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::NAN);
    let n_accounts = payload.get("n_accounts").and_then(|v| v.as_u64()).unwrap_or(0);
    let n_blocks = payload.get("n_blocks").and_then(|v| v.as_u64()).unwrap_or(0);

    // 2. Build the body that goes between the markers. Stable shape so a
    // git diff between two stamps shows only the values that actually
    // changed.
    let now_utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stamp_body = format!(
        "{begin}\n\
         **Latest gate result** (auto-generated by `evaporchain genesis stamp-result`):\n\
         \n\
         | Field | Value |\n\
         | --- | --- |\n\
         | Decision | **{decision}** |\n\
         | Source | `{source}` |\n\
         | Shape | {n_accounts} accounts × {n_blocks} blocks |\n\
         | Power-law R² | {pl_r2:.4} (slope {pl_slope:.3}) |\n\
         | Exponential R² | {exp_r2:.4} (rate {exp_rate:.4}) |\n\
         | Flat ratio | {flat_ratio:.1}× |\n\
         | Stamped at (unix) | {now_utc} |\n\
         \n\
         > {reasoning}\n\
         {end}\n",
        begin = STAMP_BEGIN,
        end = STAMP_END,
        decision = decision,
        source = source,
        n_accounts = n_accounts,
        n_blocks = n_blocks,
        pl_r2 = pl_r2,
        pl_slope = pl_slope,
        exp_r2 = exp_r2,
        exp_rate = exp_rate,
        flat_ratio = flat_ratio,
        now_utc = now_utc,
        reasoning = reasoning,
    );

    // 3. Read the doc, locate the section heading + existing marker block.
    let doc_text = std::fs::read_to_string(doc_path)
        .with_context(|| format!("failed to read doc {}", doc_path))?;

    let updated = stamp_into_doc(&doc_text, section, &stamp_body)?;

    if dry_run {
        if json_mode {
            let out = serde_json::json!({
                "doc": doc_path,
                "decision": decision,
                "would_change": doc_text != updated,
                "preview_excerpt": stamp_body,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("--- {} (dry-run, would write) ---\n", doc_path);
            println!("{}", stamp_body);
        }
        return Ok(());
    }

    if doc_text == updated {
        if !json_mode {
            println!(
                "  {} {} already up-to-date (no change)",
                "\u{2714}".green().bold(),
                doc_path
            );
        }
        return Ok(());
    }

    // Atomic write: temp file + rename so a partial write doesn't corrupt
    // the doc if the process is interrupted.
    let tmp = format!("{}.stamp.tmp", doc_path);
    std::fs::write(&tmp, &updated).with_context(|| format!("write {}", tmp))?;
    std::fs::rename(&tmp, doc_path)
        .with_context(|| format!("rename {} → {}", tmp, doc_path))?;

    if json_mode {
        let out = serde_json::json!({
            "doc": doc_path,
            "decision": decision,
            "wrote": true,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!(
            "  {} stamped {} with decision {}",
            "\u{2714}".green().bold(),
            doc_path,
            decision
        );
    }
    Ok(())
}

/// Pure helper — injects or replaces the marker block under the named
/// section heading. Tested in isolation so the patching rules don't drift.
fn stamp_into_doc(doc: &str, section: &str, stamp_body: &str) -> Result<String> {
    // Locate the section heading. We accept any line whose trimmed prefix
    // matches `section` (e.g. "## A1.8" matches "## A1.8 Open empirical …").
    let section_line_idx = doc
        .lines()
        .enumerate()
        .find(|(_, line)| line.trim_start().starts_with(section))
        .map(|(i, _)| i)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "section heading prefixed `{}` not found in doc",
                section
            )
        })?;

    let lines: Vec<&str> = doc.lines().collect();

    // Search for an existing marker pair. Anywhere in the doc — we don't
    // require it to be inside the section, so a moved heading still works.
    let begin_idx = lines.iter().position(|l| l.trim() == STAMP_BEGIN);
    let end_idx = lines.iter().position(|l| l.trim() == STAMP_END);

    match (begin_idx, end_idx) {
        (Some(b), Some(e)) if b < e => {
            // Replace lines [b..=e] with stamp_body's lines.
            let mut out = String::new();
            for line in &lines[..b] {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(stamp_body);
            // stamp_body already ends with '\n'; append remaining lines.
            for line in &lines[(e + 1)..] {
                out.push_str(line);
                out.push('\n');
            }
            // Preserve trailing newline parity with the input.
            if !doc.ends_with('\n') && out.ends_with('\n') {
                out.pop();
            }
            Ok(out)
        }
        (Some(_), Some(_)) => {
            // Both present but begin >= end — pair is reversed or coincident.
            anyhow::bail!(
                "doc has a reversed marker pair (`{}` does not precede `{}`); fix manually",
                STAMP_BEGIN,
                STAMP_END
            );
        }
        (Some(_), None) | (None, Some(_)) => {
            anyhow::bail!(
                "doc has a malformed marker pair (only one of `{}` / `{}` present); fix manually",
                STAMP_BEGIN,
                STAMP_END
            );
        }
        (None, None) => {
            // First run: insert the marker block immediately after the
            // section heading + a blank-line spacer. We position it so the
            // existing prose follows naturally.
            let mut out = String::new();
            for (i, line) in lines.iter().enumerate() {
                out.push_str(line);
                out.push('\n');
                if i == section_line_idx {
                    out.push('\n'); // blank spacer
                    out.push_str(stamp_body);
                }
            }
            if !doc.ends_with('\n') && out.ends_with('\n') {
                out.pop();
            }
            Ok(out)
        }
    }
}

// ──────────────────────────── MERA gate replay ──────────────────────────

fn parse_csv_activations(path: &str) -> Result<Vec<Vec<f64>>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read CSV {}", path))?;
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let row: Result<Vec<f64>> = line
            .split(',')
            .map(|cell| {
                let c = cell.trim();
                match c {
                    "0" | "0.0" | "false" | "" => Ok(0.0),
                    "1" | "1.0" | "true" => Ok(1.0),
                    other => other
                        .parse::<f64>()
                        .map_err(|e| anyhow::anyhow!("line {}: {} ({})", line_no + 1, e, other)),
                }
            })
            .collect();
        rows.push(row?);
    }
    if rows.is_empty() {
        anyhow::bail!("CSV at {} contained no data rows", path);
    }
    let cols = rows[0].len();
    for (i, r) in rows.iter().enumerate() {
        if r.len() != cols {
            anyhow::bail!(
                "row {} has {} columns, expected {} (matrix must be rectangular)",
                i,
                r.len(),
                cols
            );
        }
    }
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn cmd_genesis_run_gate(
    csv: Option<&str>,
    regime: Option<&str>,
    n_accounts: usize,
    n_blocks: usize,
    k: usize,
    bins: usize,
    seed: u64,
    json_mode: bool,
) -> Result<()> {
    use evaporchain_mera::gate::{run_gate, GateDecision};
    use evaporchain_mera::synthetic::{
        area_law_matrix, flat_random_matrix, log_correlated_matrix, AreaLawParams,
        FlatRandomParams, LogCorrelatedParams,
    };

    let (activations, source_label) = match (csv, regime) {
        (Some(path), None) => (parse_csv_activations(path)?, format!("csv:{}", path)),
        (None, Some(name)) => {
            let mat = match name {
                "log-correlated" => log_correlated_matrix(
                    n_accounts,
                    &LogCorrelatedParams {
                        n_blocks,
                        ..Default::default()
                    },
                    seed,
                ),
                "area-law" => area_law_matrix(
                    n_accounts,
                    &AreaLawParams {
                        n_blocks,
                        ..Default::default()
                    },
                    seed,
                ),
                "flat-random" => flat_random_matrix(
                    n_accounts,
                    &FlatRandomParams {
                        n_blocks,
                        touch_prob: 0.1,
                        energy_per_touch: 1,
                    },
                    seed,
                ),
                other => {
                    eprintln!(
                        "unknown regime '{}'. Valid: log-correlated | area-law | flat-random",
                        other
                    );
                    std::process::exit(64);
                }
            };
            (mat, format!("regime:{}", name))
        }
        (Some(_), Some(_)) => {
            eprintln!("--csv and --regime are mutually exclusive");
            std::process::exit(64);
        }
        (None, None) => {
            eprintln!(
                "supply either --csv <path> (real telemetry) or --regime <name> (synthetic)"
            );
            std::process::exit(64);
        }
    };

    let result = run_gate(&activations, k, bins, seed);
    let exit_code = match result.decision {
        GateDecision::Mera => 0,
        GateDecision::Mps => 1,
        GateDecision::Verkle => 2,
    };

    if json_mode {
        let preview_n = result.eigvals.len().min(10);
        let payload = serde_json::json!({
            "source": source_label,
            "n_accounts": activations.len(),
            "n_blocks": activations.first().map(|r| r.len()).unwrap_or(0),
            "k": k,
            "bins": bins,
            "decision": match result.decision {
                GateDecision::Mera => "MERA",
                GateDecision::Mps => "MPS",
                GateDecision::Verkle => "VERKLE",
            },
            "powerlaw_slope": result.powerlaw_slope,
            "powerlaw_r2": result.powerlaw_r2,
            "exponential_rate": result.exponential_rate,
            "exponential_r2": result.exponential_r2,
            "flat_ratio": result.flat_ratio,
            "top_eigvals_preview": &result.eigvals[..preview_n],
            "reasoning": result.reasoning,
            "exit_code": exit_code,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_header("MERA Gate (§A1.8)");
        println!(
            "  {}  {}",
            "Source:    ".truecolor(140, 150, 170),
            source_label.white()
        );
        println!(
            "  {}  {} accounts × {} blocks",
            "Shape:     ".truecolor(140, 150, 170),
            activations.len(),
            activations.first().map(|r| r.len()).unwrap_or(0),
        );
        println!(
            "  {}  K={}, bins={}",
            "Params:    ".truecolor(140, 150, 170),
            k,
            bins
        );
        println!();
        println!(
            "  {}  R²={:.4}  slope={:.3}",
            "Power-law: ".truecolor(140, 150, 170),
            result.powerlaw_r2,
            result.powerlaw_slope
        );
        println!(
            "  {}  R²={:.4}  rate={:.4}",
            "Exponent:  ".truecolor(140, 150, 170),
            result.exponential_r2,
            result.exponential_rate
        );
        println!(
            "  {}  {:.1}x",
            "Flat ratio:".truecolor(140, 150, 170),
            result.flat_ratio
        );
        println!();
        let (label, colored_label) = match result.decision {
            GateDecision::Mera => ("MERA", "MERA".green().bold()),
            GateDecision::Mps => ("MPS", "MPS".yellow().bold()),
            GateDecision::Verkle => ("VERKLE", "VERKLE".red().bold()),
        };
        let _ = label;
        println!(
            "  {}  {}",
            "Decision:  ".truecolor(140, 150, 170),
            colored_label
        );
        println!("  {}", result.reasoning.truecolor(180, 180, 180));
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
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
            println!(
                "  {} Validator keypair written to {}",
                "\u{2714}".green().bold(),
                path
            );
            println!();
            println!(
                "  {}  {}",
                "BLS pk:".truecolor(140, 150, 170),
                hex::encode(&bls.public_key_bytes().0)[..32].truecolor(100, 110, 130)
            );
            println!(
                "  {}  {}...",
                "ML-DSA:".truecolor(140, 150, 170),
                hex::encode(&mldsa.public_key()[..16]).truecolor(100, 110, 130)
            );
            println!(
                "  {}  {}...",
                "VRF pk:".truecolor(140, 150, 170),
                hex::encode(&vrf.public_key_bytes()[..16]).truecolor(100, 110, 130)
            );
            println!();
            println!(
                "  {} Keep the secret keys safe!",
                "\u{26A0}".yellow().bold()
            );
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
    std::fs::write(path, data).with_context(|| format!("Failed to write {}", path))?;
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
    let secret = std::fs::read(in_file).with_context(|| format!("Failed to read {}", in_file))?;
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
    let blob = std::fs::read(in_file).with_context(|| format!("Failed to read {}", in_file))?;
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

// ──────────────────────────── Snapshot Subcommand ────────────────────────
//
// Implements `evaporchain snapshot {create,verify,apply}`. Used by
// deploy/ansible/playbooks/snapshot.yml in place of the legacy
// coordinated-tar fallback. The blob format lives in
// evaporchain_state::snapshot::SnapshotFile (zstd + bincode + magic
// header `EVSN` + version byte) and is what GET
// /api/snapshot/download/:height streams to peers.

fn cmd_snapshot_create(data_dir: &str, output: &str, chain_id: &str, json_mode: bool) -> Result<()> {
    use evaporchain_state::{RocksDBStateDB, SnapshotFile, ValidatorSetSnapshot};

    let mut db = RocksDBStateDB::open(data_dir)
        .map_err(|e| anyhow::anyhow!("open RocksDB at {}: {}", data_dir, e))?;

    // CLI-driven snapshot creation runs against a stopped node. We don't
    // have access to live consensus state here, so block_height / epoch /
    // parent_hash / bell_reading / validator_set come from a sentinel
    // ("offline-create") set; the operator-facing /api/snapshot/* path
    // populates those properly from a live TendermintConsensus. Verify
    // and apply still work on a CLI-created blob — but a fast-syncing
    // peer should always prefer one served by a running node so
    // consensus metadata is faithful.
    let height = 0u64;
    let epoch = 0u64;
    let parent_hash = [0u8; 32];
    let validator_set = ValidatorSetSnapshot::default();
    let bell_reading = None;

    let file = SnapshotFile::create(
        &mut db,
        chain_id.to_string(),
        height,
        epoch,
        parent_hash,
        bell_reading,
        validator_set,
    )
    .map_err(|e| anyhow::anyhow!("snapshot create: {}", e))?;

    let path = std::path::Path::new(output);
    let written = file
        .write_to_path(path)
        .map_err(|e| anyhow::anyhow!("write snapshot: {}", e))?;

    if json_mode {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "output": output,
                "size_bytes": written,
                "chain_id": chain_id,
                "block_height": height,
                "state_root": hex::encode(file.state_root),
                "integrity_hash": hex::encode(file.integrity_hash),
            }))?
        );
    } else {
        println!(
            "  {} Wrote {} bytes to {}",
            "\u{2714}".green().bold(),
            written,
            output
        );
        println!(
            "  state_root      = {}",
            hex::encode(file.state_root).truecolor(140, 150, 170)
        );
        println!(
            "  integrity_hash  = {}",
            hex::encode(file.integrity_hash).truecolor(140, 150, 170)
        );
    }
    Ok(())
}

fn cmd_snapshot_verify(input: &str, json_mode: bool) -> Result<()> {
    use evaporchain_state::SnapshotFile;
    let path = std::path::Path::new(input);
    let file =
        SnapshotFile::load_and_verify(path).map_err(|e| anyhow::anyhow!("verify: {}", e))?;

    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if json_mode {
        let meta = file.metadata(size_bytes);
        println!("{}", serde_json::to_string_pretty(&meta)?);
    } else {
        println!(
            "  {} Snapshot OK ({} bytes)",
            "\u{2714}".green().bold(),
            size_bytes
        );
        println!("  chain_id        = {}", file.chain_id);
        println!("  block_height    = {}", file.block_height);
        println!("  epoch           = {}", file.epoch);
        println!(
            "  state_root      = {}",
            hex::encode(file.state_root).truecolor(140, 150, 170)
        );
        println!("  accounts        = {}", file.accounts.len());
        println!("  objects         = {}", file.objects.len());
        println!("  ghosts          = {}", file.ghosts.len());
        println!(
            "  integrity_hash  = {}",
            hex::encode(file.integrity_hash).truecolor(140, 150, 170)
        );
    }
    Ok(())
}

fn cmd_snapshot_apply(input: &str, data_dir: &str, json_mode: bool) -> Result<()> {
    use evaporchain_state::{RocksDBStateDB, SnapshotFile};
    let path = std::path::Path::new(input);
    let file =
        SnapshotFile::load_and_verify(path).map_err(|e| anyhow::anyhow!("verify: {}", e))?;

    let mut db = RocksDBStateDB::open(data_dir)
        .map_err(|e| anyhow::anyhow!("open RocksDB at {}: {}", data_dir, e))?;
    let result = file
        .apply_to(&mut db)
        .map_err(|e| anyhow::anyhow!("apply: {}", e))?;

    if json_mode {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "block_height": file.block_height,
                "accounts_restored": result.accounts_restored,
                "objects_restored": result.objects_restored,
                "ghosts_restored": result.ghosts_restored,
                "nullifiers_restored": result.nullifiers_restored,
                "state_root": hex::encode(result.state_root),
                "elapsed_ms": result.elapsed_ms,
            }))?
        );
    } else {
        println!(
            "  {} Applied snapshot at height {} ({}ms)",
            "\u{2714}".green().bold(),
            file.block_height,
            result.elapsed_ms
        );
        println!("  accounts_restored = {}", result.accounts_restored);
        println!("  objects_restored  = {}", result.objects_restored);
        println!("  ghosts_restored   = {}", result.ghosts_restored);
        println!(
            "  state_root        = {}",
            hex::encode(result.state_root).truecolor(140, 150, 170)
        );
    }
    Ok(())
}

// ──────────────────────── UpgradeContract Helper ─────────────────────────

#[derive(Debug, Deserialize)]
struct EndorserStakeEntry {
    stake: u64,
}

#[allow(clippy::too_many_arguments)]
async fn cmd_upgrade_contract(
    base: &str,
    owner_hex: &str,
    contract_id: u64,
    new_bytecode_hex: Option<&str>,
    new_bytecode_path: Option<&str>,
    nonce: u64,
    admin_key_path: Option<&str>,
    governance_quorum_path: Option<&str>,
    required_stake: u64,
    broadcast: bool,
    json_mode: bool,
) -> Result<()> {
    use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};

    if admin_key_path.is_some() && governance_quorum_path.is_some() {
        anyhow::bail!("--admin-key and --governance-quorum are mutually exclusive");
    }
    if admin_key_path.is_none() && governance_quorum_path.is_none() {
        anyhow::bail!("supply exactly one of --admin-key or --governance-quorum");
    }

    // Resolve bytecode bytes.
    let new_bytecode: Vec<u8> = match (new_bytecode_hex, new_bytecode_path) {
        (Some(h), None) => {
            let h = h.strip_prefix("0x").unwrap_or(h);
            hex::decode(h).context("invalid --new-bytecode-hex")?
        }
        (None, Some(p)) => {
            std::fs::read(p).with_context(|| format!("Failed to read {}", p))?
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("supply at most one of --new-bytecode-hex / --new-bytecode-path");
        }
        (None, None) => {
            anyhow::bail!("supply --new-bytecode-hex or --new-bytecode-path");
        }
    };

    let new_bytecode_hash = blake3::hash(&new_bytecode);
    let new_bytecode_hash_bytes: [u8; 32] = *new_bytecode_hash.as_bytes();
    let new_bytecode_hash_hex = hex::encode(new_bytecode_hash_bytes);

    // Build per-path auth.
    let (admin_signature_hex, admin_public_key_hex, endorser_stakes) = if let Some(path) =
        admin_key_path
    {
        // Path A — load ML-DSA keypair, sign canonical payload.
        // Accept either: a JSON keygen bundle (`{"ml_dsa":{public_key,secret_key}}`)
        // or a file containing ONLY the hex secret key (legacy raw form).
        let file_text = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read --admin-key {}", path))?;
        let trimmed = file_text.trim();
        let (pk_hex, sk_hex) = if trimmed.starts_with('{') {
            let v: serde_json::Value =
                serde_json::from_str(trimmed).context("--admin-key JSON parse failed")?;
            let mldsa = v
                .get("ml_dsa")
                .ok_or_else(|| anyhow::anyhow!("--admin-key JSON missing `ml_dsa` field"))?;
            let pk = mldsa
                .get("public_key")
                .and_then(|x| x.as_str())
                .ok_or_else(|| anyhow::anyhow!("--admin-key JSON missing `ml_dsa.public_key`"))?
                .to_string();
            let sk = mldsa
                .get("secret_key")
                .and_then(|x| x.as_str())
                .ok_or_else(|| anyhow::anyhow!("--admin-key JSON missing `ml_dsa.secret_key`"))?
                .to_string();
            (pk, sk)
        } else {
            anyhow::bail!(
                "--admin-key file must be a keygen JSON bundle (with ml_dsa.public_key + ml_dsa.secret_key); raw-hex SK alone cannot reconstruct the keypair"
            );
        };

        let pk_bytes = hex::decode(pk_hex.trim()).context("invalid ml_dsa.public_key hex")?;
        let sk_bytes = hex::decode(sk_hex.trim()).context("invalid ml_dsa.secret_key hex")?;
        let kp = MlDsaKeypair::from_bytes(&pk_bytes, &sk_bytes)
            .map_err(|e| anyhow::anyhow!("MlDsaKeypair::from_bytes: {:?}", e))?;

        let canonical = format!(
            "{{\"type\":\"upgrade_contract\",\"contract_id\":{},\"new_bytecode_hash_hex\":\"{}\",\"nonce\":{}}}",
            contract_id, new_bytecode_hash_hex, nonce
        );
        let sig = kp.sign(canonical.as_bytes());
        (Some(hex::encode(&sig)), Some(hex::encode(&pk_bytes)), Vec::<u64>::new())
    } else {
        // Path B — load endorser-stakes JSON (`[{"stake": N}, ...]`).
        let path = governance_quorum_path.expect("checked above");
        let file_text = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read --governance-quorum {}", path))?;
        let entries: Vec<EndorserStakeEntry> =
            serde_json::from_str(&file_text).context("--governance-quorum JSON parse failed")?;
        if entries.is_empty() {
            anyhow::bail!("--governance-quorum file has no endorser entries");
        }
        if required_stake == 0 {
            anyhow::bail!("--required-stake must be > 0 for the governance path");
        }
        let stakes: Vec<u64> = entries.iter().map(|e| e.stake).collect();
        (None, None, stakes)
    };

    // Assemble request body for /api/tx/upgrade_contract. Whether we
    // broadcast or just print, the JSON shape is the same.
    let mut body = serde_json::json!({
        "owner": owner_hex,
        "contract_id": contract_id,
        "new_bytecode_hex": hex::encode(&new_bytecode),
        "new_bytecode_hash_hex": new_bytecode_hash_hex,
        "nonce": nonce,
        "endorser_stakes": endorser_stakes,
        "required_stake": required_stake,
    });
    if let Some(s) = &admin_signature_hex {
        body["admin_signature_hex"] = serde_json::Value::String(s.clone());
    }
    if let Some(p) = &admin_public_key_hex {
        body["admin_public_key_hex"] = serde_json::Value::String(p.clone());
    }

    if broadcast {
        let result: serde_json::Value =
            api_post(base, "/api/tx/upgrade_contract", &body).await?;
        if json_mode {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!(
                "  {} {}",
                "\u{2714}".green().bold(),
                "UpgradeContract submitted".bold()
            );
            println!("  {}", serde_json::to_string_pretty(&result)?);
        }
    } else {
        // Print the body — caller curls it themselves.
        println!("{}", serde_json::to_string_pretty(&body)?);
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
        Commands::Faucet { address } => cmd_faucet(base, &address, cli.json).await,
        Commands::Consensus => cmd_consensus(base, cli.json).await,
        Commands::Devnet { validators, demo } => cmd_devnet(validators, demo).await,
        Commands::Genesis { action } => match action {
            GenesisAction::Validate { path } => cmd_genesis_validate(&path, cli.json),
            GenesisAction::Show { path } => cmd_genesis_show(&path, cli.json),
            GenesisAction::Init { path } => cmd_genesis_init(&path, cli.json),
            GenesisAction::Create {
                output,
                chain_id,
                total_supply,
                block_interval,
                min_stake,
            } => cmd_genesis_create(
                &output,
                &chain_id,
                total_supply,
                block_interval,
                min_stake,
                cli.json,
            ),
            GenesisAction::AddValidator {
                path,
                name,
                stake,
                p2p,
                keys,
                balance,
            } => cmd_genesis_add_validator(
                &path,
                &name,
                stake,
                p2p.as_deref(),
                keys.as_deref(),
                balance,
                cli.json,
            ),
            GenesisAction::SetValidatorBls {
                path,
                validator_id,
                keys,
                bls_pk_hex,
            } => cmd_genesis_set_validator_bls(
                &path,
                validator_id,
                keys.as_deref(),
                bls_pk_hex.as_deref(),
                cli.json,
            ),
            GenesisAction::AddAccount {
                path,
                label,
                balance,
                address_byte,
            } => cmd_genesis_add_account(&path, &label, balance, address_byte, cli.json),
            GenesisAction::Finalize { path } => cmd_genesis_finalize(&path, cli.json),
            GenesisAction::Contribute {
                keys,
                validator_id,
                name,
                stake,
                p2p,
                balance,
                address_byte,
                chain_id,
                genesis_time,
                ceremony_nonce,
                out,
            } => cmd_genesis_contribute(
                &keys,
                validator_id,
                &name,
                stake,
                p2p.as_deref(),
                balance,
                address_byte,
                &chain_id,
                &genesis_time,
                &ceremony_nonce,
                &out,
                cli.json,
            ),
            GenesisAction::Ceremony {
                contributions,
                chain_id,
                genesis_time,
                ceremony_nonce,
                total_supply,
                block_interval,
                min_stake,
                out,
            } => cmd_genesis_ceremony(
                &contributions,
                &chain_id,
                &genesis_time,
                &ceremony_nonce,
                total_supply,
                block_interval,
                min_stake,
                &out,
                cli.json,
            ),
            GenesisAction::VerifyCeremony {
                contributions,
                genesis,
                transcript,
            } => cmd_genesis_verify_ceremony(&contributions, &genesis, &transcript, cli.json),
            GenesisAction::StampResult {
                from_json,
                doc,
                section,
                dry_run,
            } => cmd_genesis_stamp_result(&from_json, &doc, &section, dry_run, cli.json),
            GenesisAction::RunGate {
                csv,
                regime,
                n_accounts,
                n_blocks,
                k,
                bins,
                seed,
            } => cmd_genesis_run_gate(
                csv.as_deref(),
                regime.as_deref(),
                n_accounts,
                n_blocks,
                k,
                bins,
                seed,
                cli.json,
            ),
        },
        Commands::Testnet { action } => match action {
            TestnetAction::Init {
                out,
                validators,
                chain_id,
                total_supply,
                stake,
                block_interval_ms,
                p2p_base,
                api_base,
                listen_ip,
                force,
            } => cmd_testnet_init(
                &out,
                validators,
                &chain_id,
                total_supply,
                stake,
                block_interval_ms,
                p2p_base,
                api_base,
                &listen_ip,
                force,
            ),
            TestnetAction::Up { dir, split_logs } => cmd_testnet_up(&dir, split_logs).await,
            TestnetAction::Status { dir } => cmd_testnet_status(&dir).await,
            TestnetAction::Down { dir } => cmd_testnet_down(&dir),
        },
        Commands::Keygen { output } => cmd_keygen(output.as_deref(), cli.json),
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
        Commands::Onboarding { action } => match action {
            OnboardingAction::GenerateCoordinator { out_dir } => {
                onboarding::cmd_generate_coordinator(std::path::Path::new(&out_dir))
            }
            OnboardingAction::BuildGenesis {
                validators,
                coordinator_sk,
                chain_id,
                output,
                block_interval_ms,
                total_supply,
                min_stake,
            } => onboarding::cmd_build_genesis(
                std::path::Path::new(&validators),
                std::path::Path::new(&coordinator_sk),
                &chain_id,
                std::path::Path::new(&output),
                block_interval_ms,
                total_supply,
                min_stake,
            ),
            OnboardingAction::Verify {
                genesis,
                coordinator_pk,
            } => onboarding::cmd_verify(
                std::path::Path::new(&genesis),
                std::path::Path::new(&coordinator_pk),
            ),
            OnboardingAction::GenerateNetworkKey {
                out_dir,
                listen_ip,
                port,
            } => onboarding::cmd_generate_network_key(
                std::path::Path::new(&out_dir),
                &listen_ip,
                port,
            ),
            OnboardingAction::Install {
                genesis,
                keys,
                validator_id,
                node_dir,
                coordinator_pk,
                api_port,
                p2p_port,
                bootstrap,
                validators,
                node_binary,
                systemd,
                launchd,
                force,
            } => onboarding::cmd_install(onboarding::InstallArgs {
                genesis: std::path::PathBuf::from(genesis),
                keys: std::path::PathBuf::from(keys),
                validator_id,
                node_dir: std::path::PathBuf::from(node_dir),
                coordinator_pk: coordinator_pk.map(std::path::PathBuf::from),
                api_port,
                p2p_port,
                bootstrap,
                validators,
                node_binary,
                systemd,
                launchd,
                force,
            }),
        },
        Commands::Snapshot { action } => match action {
            SnapshotAction::Create {
                data_dir,
                output,
                chain_id,
            } => cmd_snapshot_create(&data_dir, &output, &chain_id, cli.json),
            SnapshotAction::Verify { input } => cmd_snapshot_verify(&input, cli.json),
            SnapshotAction::Apply { input, data_dir } => {
                cmd_snapshot_apply(&input, &data_dir, cli.json)
            }
        },
        Commands::Da { action } => match action {
            DaAction::Verify {
                nodes,
                block,
                samples,
                threshold,
                seed,
                skip_chain_attestation,
            } => {
                cmd_da_verify(
                    &nodes,
                    block,
                    samples,
                    threshold,
                    seed.as_deref(),
                    skip_chain_attestation,
                    cli.json,
                )
                .await
            }
        },
        Commands::UpgradeContract {
            owner,
            contract_id,
            new_bytecode_hex,
            new_bytecode_path,
            nonce,
            admin_key,
            governance_quorum,
            required_stake,
            broadcast,
        } => {
            cmd_upgrade_contract(
                base,
                &owner,
                contract_id,
                new_bytecode_hex.as_deref(),
                new_bytecode_path.as_deref(),
                nonce,
                admin_key.as_deref(),
                governance_quorum.as_deref(),
                required_stake,
                broadcast,
                cli.json,
            )
            .await
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
            "evaporchain",
            "transfer",
            "--from",
            "1",
            "--to",
            "2",
            "--amount",
            "500",
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
        if let Commands::Genesis {
            action: GenesisAction::Validate { path },
        } = cli.command
        {
            assert_eq!(path, "genesis.json");
        } else {
            panic!("Expected Genesis Validate command");
        }
    }

    #[test]
    fn test_cli_parses_genesis_show() {
        let cli = Cli::parse_from(["evaporchain", "genesis", "show", "genesis.json"]);
        if let Commands::Genesis {
            action: GenesisAction::Show { path },
        } = cli.command
        {
            assert_eq!(path, "genesis.json");
        } else {
            panic!("Expected Genesis Show command");
        }
    }

    #[test]
    fn test_cli_parses_genesis_init() {
        let cli = Cli::parse_from(["evaporchain", "genesis", "init", "genesis.json"]);
        if let Commands::Genesis {
            action: GenesisAction::Init { path },
        } = cli.command
        {
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
            assert_eq!(
                result1.state_root, result2.state_root,
                "Genesis must be deterministic"
            );
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
            assert!(
                result.is_ok(),
                "Genesis validation failed: {:?}",
                result.err()
            );
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
            "evaporchain",
            "genesis",
            "create",
            "out.json",
            "--chain-id",
            "my-testnet",
            "--total-supply",
            "5000000",
        ]);
        if let Commands::Genesis {
            action:
                GenesisAction::Create {
                    output,
                    chain_id,
                    total_supply,
                    ..
                },
        } = cli.command
        {
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
            "evaporchain",
            "genesis",
            "add-validator",
            "genesis.json",
            "--name",
            "node1",
            "--stake",
            "1000",
        ]);
        if let Commands::Genesis {
            action:
                GenesisAction::AddValidator {
                    path, name, stake, ..
                },
        } = cli.command
        {
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
            "evaporchain",
            "genesis",
            "add-account",
            "genesis.json",
            "--label",
            "Faucet",
            "--balance",
            "5000000",
        ]);
        if let Commands::Genesis {
            action:
                GenesisAction::AddAccount {
                    path,
                    label,
                    balance,
                    ..
                },
        } = cli.command
        {
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
        if let Commands::Genesis {
            action: GenesisAction::Finalize { path },
        } = cli.command
        {
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
        let result = cmd_genesis_add_validator(
            path_str,
            "alpha",
            1000,
            Some("/ip4/127.0.0.1/tcp/9000"),
            None,
            1_000_000,
            true,
        );
        assert!(result.is_ok(), "Add validator 1 failed: {:?}", result.err());

        let result = cmd_genesis_add_validator(
            path_str,
            "beta",
            1000,
            Some("/ip4/127.0.0.1/tcp/9001"),
            None,
            1_000_000,
            true,
        );
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

    // ─── Snapshot subcommand ───

    #[test]
    fn test_cli_parses_snapshot_create() {
        let cli = Cli::parse_from([
            "evaporchain",
            "snapshot",
            "create",
            "--data-dir",
            "/tmp/evdata",
            "--output",
            "/tmp/snap.zst",
            "--chain-id",
            "evaporchain-test-1",
        ]);
        if let Commands::Snapshot {
            action:
                SnapshotAction::Create {
                    data_dir,
                    output,
                    chain_id,
                },
        } = cli.command
        {
            assert_eq!(data_dir, "/tmp/evdata");
            assert_eq!(output, "/tmp/snap.zst");
            assert_eq!(chain_id, "evaporchain-test-1");
        } else {
            panic!("Expected Snapshot Create command");
        }
    }

    #[test]
    fn test_cli_parses_snapshot_verify() {
        let cli = Cli::parse_from([
            "evaporchain",
            "snapshot",
            "verify",
            "--input",
            "/tmp/snap.zst",
        ]);
        if let Commands::Snapshot {
            action: SnapshotAction::Verify { input },
        } = cli.command
        {
            assert_eq!(input, "/tmp/snap.zst");
        } else {
            panic!("Expected Snapshot Verify command");
        }
    }

    #[test]
    fn test_cli_parses_snapshot_apply() {
        let cli = Cli::parse_from([
            "evaporchain",
            "snapshot",
            "apply",
            "--input",
            "/tmp/snap.zst",
            "--data-dir",
            "/tmp/evdata-restore",
        ]);
        if let Commands::Snapshot {
            action: SnapshotAction::Apply { input, data_dir },
        } = cli.command
        {
            assert_eq!(input, "/tmp/snap.zst");
            assert_eq!(data_dir, "/tmp/evdata-restore");
        } else {
            panic!("Expected Snapshot Apply command");
        }
    }

    #[test]
    fn cli_snapshot_create_then_verify() {
        // End-to-end: open a fresh RocksDB data dir, populate a couple
        // of accounts via the state DB directly, then run the CLI
        // create + verify functions and confirm the on-disk blob
        // round-trips with matching state_root and integrity_hash.
        use evaporchain_state::{db::StateDB as _, RocksDBStateDB, SnapshotFile};
        use evaporchain_types::Account;
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("evaporchain-cli-snap-{}", pid));
        let data_dir = dir.join("data");
        let snap_path = dir.join("snap.zst");
        let _ = std::fs::create_dir_all(&data_dir);

        // Seed a tiny state DB.
        {
            let mut db = RocksDBStateDB::open(&data_dir).unwrap();
            db.put_account(Account {
                address: [1u8; 32],
                balance: 1_000_000,
                nonce: 0,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
            });
            db.put_account(Account {
                address: [2u8; 32],
                balance: 500_000,
                nonce: 0,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
            });
        } // drop -> release RocksDB lock

        // Invoke the CLI create command (json mode for stable output).
        let r = cmd_snapshot_create(
            data_dir.to_str().unwrap(),
            snap_path.to_str().unwrap(),
            "evaporchain-test-1",
            true,
        );
        assert!(r.is_ok(), "snapshot create failed: {:?}", r.err());
        assert!(snap_path.exists());

        // Verify via the CLI (just runs without error).
        let v = cmd_snapshot_verify(snap_path.to_str().unwrap(), true);
        assert!(v.is_ok(), "snapshot verify failed: {:?}", v.err());

        // Re-load via the library API and assert structural fields.
        let loaded = SnapshotFile::load_and_verify(&snap_path).unwrap();
        assert_eq!(loaded.chain_id, "evaporchain-test-1");
        assert_eq!(loaded.accounts.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ceremony_install_rehearsal_full_pipeline() {
        // End-to-end rehearsal of the full operator runbook against a temp
        // directory. Drives every command shipped this sprint sequentially
        // and asserts each artifact is consistent with the next:
        //
        //   1. `keygen` × N         → N keys.json bundles with BLS+ML-DSA+VRF
        //   2. `genesis contribute` × N → N signed envelopes in contribs/
        //   3. `genesis ceremony`   → single coordinator-signed genesis +
        //                             transcript pinned by config_hash
        //   4. `genesis verify-ceremony` → recomputes hashes, asserts match
        //   5. `onboarding install` × N → laid-out node-dirs with bls_key.bin
        //                                 (32 bytes, 0600) and runnable run.sh
        //
        // What this catches: any drift between the seven binaries (e.g.
        // canonical-bytes mismatch between contribute and ceremony, BLS
        // pubkey not propagating from keygen → genesis → install, transcript
        // hash not matching recomputation, install failing to find the
        // genesis entry for an operator's validator-id).
        use std::path::PathBuf;
        const N_VALIDATORS: u64 = 4;

        let pid = std::process::id();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root: PathBuf = std::env::temp_dir().join(format!("evap-rehearsal-{}-{}", pid, now));
        std::fs::create_dir_all(&root).unwrap();

        // Defer cleanup until the test ends, even on assert failure.
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());

        // Per-validator working dirs.
        let keys_dir = root.join("keys");
        let contribs_dir = root.join("contributions");
        let nodes_root = root.join("nodes");
        std::fs::create_dir_all(&keys_dir).unwrap();
        std::fs::create_dir_all(&contribs_dir).unwrap();
        std::fs::create_dir_all(&nodes_root).unwrap();

        let chain_id = "evaporchain-rehearsal-1";
        let genesis_time = "2026-05-01T00:00:00Z";
        // Deterministic 32-byte ceremony nonce (hex). Real ceremonies use
        // a freshly-randomized nonce drawn after every operator commits.
        let ceremony_nonce =
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

        // ── Stage 1: keygen × N ────────────────────────────────────────
        let mut keys_paths = Vec::with_capacity(N_VALIDATORS as usize);
        for vid in 1..=N_VALIDATORS {
            let path = keys_dir.join(format!("v{}.json", vid));
            cmd_keygen(Some(path.to_str().unwrap()), true).unwrap();
            assert!(path.is_file(), "keygen did not produce {}", path.display());
            // Sanity: bundle has the three pubkey/secret pairs we expect.
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert!(v.get("bls").and_then(|b| b.get("public_key")).is_some());
            assert!(v.get("bls").and_then(|b| b.get("secret_key")).is_some());
            assert!(v.get("ml_dsa").and_then(|m| m.get("public_key")).is_some());
            assert!(v.get("ml_dsa").and_then(|m| m.get("secret_key")).is_some());
            keys_paths.push(path);
        }

        // ── Stage 2: genesis contribute × N ────────────────────────────
        let mut envelope_paths = Vec::with_capacity(N_VALIDATORS as usize);
        for (i, keys_path) in keys_paths.iter().enumerate() {
            let vid = (i + 1) as u64;
            let env_path = contribs_dir.join(format!("v{}.json", vid));
            cmd_genesis_contribute(
                keys_path.to_str().unwrap(),
                vid,
                &format!("validator-{}", vid),
                500_000,                         // stake
                None,                            // no p2p in rehearsal
                1_000_000,                       // balance
                Some(vid as u8),                 // address_byte
                chain_id,
                genesis_time,
                ceremony_nonce,
                env_path.to_str().unwrap(),
                true,
            )
            .unwrap();
            assert!(env_path.is_file());
            envelope_paths.push(env_path);
        }

        // ── Stage 3: genesis ceremony ──────────────────────────────────
        let genesis_path = root.join("genesis.json");
        let transcript_path = root.join("genesis.json.transcript.json");
        cmd_genesis_ceremony(
            contribs_dir.to_str().unwrap(),
            chain_id,
            genesis_time,
            ceremony_nonce,
            10_000_000, // total_supply
            2000,       // block_interval
            100,        // min_stake
            genesis_path.to_str().unwrap(),
            true,
        )
        .unwrap();
        assert!(genesis_path.is_file());
        assert!(transcript_path.is_file());

        // Genesis must contain N validator entries with BLS pubkeys, sorted
        // ascending by id.
        let genesis_text = std::fs::read_to_string(&genesis_path).unwrap();
        let config: evaporchain_types::genesis::GenesisConfig =
            serde_json::from_str(&genesis_text).unwrap();
        assert_eq!(config.validators.len() as u64, N_VALIDATORS);
        for (i, v) in config.validators.iter().enumerate() {
            assert_eq!(v.id, (i as u64) + 1, "validators must be sorted by id");
            assert!(
                v.bls_public_key.is_some(),
                "validator {} missing bls_public_key after ceremony",
                v.id
            );
        }
        // Transcript's config_hash must reproduce from the on-disk genesis.
        let transcript: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&transcript_path).unwrap()).unwrap();
        let declared_hash = transcript
            .get("config_hash")
            .and_then(|v| v.as_str())
            .expect("transcript missing config_hash");
        let recomputed = hex::encode(evaporchain_crypto::blake3_hash(
            &serde_json::to_vec(&config).unwrap(),
        ));
        assert_eq!(
            recomputed, declared_hash,
            "transcript config_hash != recomputed hash (canonical-bytes drift between ceremony and verify)"
        );

        // ── Stage 4: genesis verify-ceremony ──────────────────────────
        cmd_genesis_verify_ceremony(
            contribs_dir.to_str().unwrap(),
            genesis_path.to_str().unwrap(),
            transcript_path.to_str().unwrap(),
            true,
        )
        .unwrap();

        // ── Stage 5: onboarding install × N ────────────────────────────
        for (i, keys_path) in keys_paths.iter().enumerate() {
            let vid = (i + 1) as u64;
            let node_dir = nodes_root.join(format!("v{}", vid));
            onboarding::cmd_install(onboarding::InstallArgs {
                genesis: genesis_path.clone(),
                keys: keys_path.clone(),
                validator_id: vid,
                node_dir: node_dir.clone(),
                coordinator_pk: None, // ceremony genesis isn't coordinator-signed
                api_port: 8080 + vid as u16,
                p2p_port: 7000 + vid as u16,
                bootstrap: (1..=N_VALIDATORS)
                    .filter(|&j| j != vid)
                    .map(|j| format!("/ip4/127.0.0.1/tcp/{}", 7000 + j))
                    .collect(),
                validators: Some(N_VALIDATORS),
                node_binary: Some("/usr/local/bin/evaporchain-node".to_string()),
                systemd: false,
                launchd: false,
                force: false,
            })
            .unwrap_or_else(|e| panic!("install for validator {} failed: {:#}", vid, e));

            // Per-validator artifact assertions.
            let bls_path = node_dir.join("data/bls_key.bin");
            assert!(bls_path.is_file(), "v{vid}: bls_key.bin missing");
            assert_eq!(
                std::fs::read(&bls_path).unwrap().len(),
                32,
                "v{vid}: bls_key.bin must be 32 raw bytes"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&bls_path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600, "v{vid}: bls_key.bin must be 0600");
            }
            assert!(node_dir.join("genesis.json").is_file());
            let run_sh = std::fs::read_to_string(node_dir.join("run.sh")).unwrap();
            assert!(run_sh.contains(&format!("--validator-id {}", vid)));
            assert!(run_sh.contains(&format!("--api-port {}", 8080 + vid as u16)));
            assert!(run_sh.contains(&format!("--port {}", 7000 + vid as u16)));
            assert!(
                run_sh.contains(&format!("--validators {}", N_VALIDATORS)),
                "v{vid}: run.sh missing validator count"
            );
            // Each install must dial every other validator, never itself.
            for j in 1..=N_VALIDATORS {
                let peer = format!("/ip4/127.0.0.1/tcp/{}", 7000 + j);
                if j == vid {
                    assert!(
                        !run_sh.contains(&peer),
                        "v{vid}: run.sh dials its own peer {}",
                        peer
                    );
                } else {
                    assert!(
                        run_sh.contains(&peer),
                        "v{vid}: run.sh missing bootstrap peer {}",
                        peer
                    );
                }
            }
        }

        // Cross-install assertion: every node-dir's copy of genesis.json
        // re-parses to a GenesisConfig whose canonical bytes hash to the
        // declared config_hash. Catches any silent mutation of the genesis
        // between ceremony and install.
        for vid in 1..=N_VALIDATORS {
            let copied = std::fs::read(nodes_root.join(format!("v{}", vid)).join("genesis.json"))
                .unwrap();
            let parsed: evaporchain_types::genesis::GenesisConfig =
                serde_json::from_slice(&copied).unwrap();
            let canonical = hex::encode(evaporchain_crypto::blake3_hash(
                &serde_json::to_vec(&parsed).unwrap(),
            ));
            assert_eq!(
                canonical, declared_hash,
                "v{vid}: genesis.json hash drift from transcript (re-parse → re-serialize → hash)"
            );
        }
    }

    #[test]
    fn stamp_into_doc_first_run_inserts_marker_block() {
        let doc = "# Doc\n\nIntro line.\n\n## A1.8 Open empirical question — the MERA gate\n\nOriginal prose A.\nOriginal prose B.\n\n## A1.9 Doctrine\n\nMore prose.\n";
        let body = format!(
            "{begin}\nDecision: MERA\n{end}\n",
            begin = STAMP_BEGIN,
            end = STAMP_END
        );
        let out = stamp_into_doc(doc, "## A1.8", &body).unwrap();

        // Marker block lands AFTER the section heading + spacer, BEFORE the
        // original prose; original prose untouched.
        assert!(out.contains("## A1.8 Open empirical question"));
        assert!(out.contains(STAMP_BEGIN));
        assert!(out.contains(STAMP_END));
        assert!(out.contains("Original prose A."));
        assert!(out.contains("Original prose B."));
        assert!(out.contains("## A1.9 Doctrine"));

        let begin_pos = out.find(STAMP_BEGIN).unwrap();
        let prose_pos = out.find("Original prose A.").unwrap();
        let next_section_pos = out.find("## A1.9").unwrap();
        assert!(begin_pos < prose_pos, "marker must precede original prose");
        assert!(prose_pos < next_section_pos, "next section must follow prose");
    }

    #[test]
    fn stamp_into_doc_re_run_replaces_only_marker_region() {
        let doc = format!(
            "# Doc\n\n## A1.8 Gate\n\n{begin}\nold body line 1\nold body line 2\n{end}\n\nProse that must survive.\n\n## A1.9 Next\n",
            begin = STAMP_BEGIN,
            end = STAMP_END
        );
        let new_body = format!(
            "{begin}\nNEW DECISION: VERKLE\n{end}\n",
            begin = STAMP_BEGIN,
            end = STAMP_END
        );
        let out = stamp_into_doc(&doc, "## A1.8", &new_body).unwrap();
        assert!(out.contains("NEW DECISION: VERKLE"));
        assert!(!out.contains("old body line 1"), "old body must be gone");
        assert!(!out.contains("old body line 2"));
        assert!(out.contains("Prose that must survive."));
        assert!(out.contains("## A1.9 Next"));
    }

    #[test]
    fn stamp_into_doc_rejects_orphaned_marker() {
        let doc = format!(
            "# Doc\n\n## A1.8 Gate\n\n{begin}\nbody without close\n",
            begin = STAMP_BEGIN
        );
        let body = format!(
            "{begin}\nshould-not-write\n{end}\n",
            begin = STAMP_BEGIN,
            end = STAMP_END
        );
        let err = stamp_into_doc(&doc, "## A1.8", &body).unwrap_err();
        assert!(format!("{err:#}").contains("malformed marker pair"));
    }

    #[test]
    fn stamp_into_doc_errors_on_missing_section() {
        let doc = "# Doc\n\n## A2.0 Different section\n\nProse.\n";
        let body = format!(
            "{begin}\nx\n{end}\n",
            begin = STAMP_BEGIN,
            end = STAMP_END
        );
        let err = stamp_into_doc(doc, "## A1.8", &body).unwrap_err();
        assert!(format!("{err:#}").contains("section heading"));
    }

    #[test]
    fn stamp_result_handler_writes_atomically() {
        // End-to-end: simulate `genesis run-gate --json` → file → stamper.
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("evap-stamp-{}", pid));
        std::fs::create_dir_all(&dir).unwrap();
        let doc_path = dir.join("INVENTION_STACK.md");
        let payload_path = dir.join("gate.json");
        std::fs::write(
            &doc_path,
            "# Doc\n\n## A1.8 Open empirical question\n\nProse here.\n",
        )
        .unwrap();
        let payload = serde_json::json!({
            "decision": "MERA",
            "reasoning": "Power-law fit dominates: R²=0.97, slope=1.5.",
            "source": "csv:tele.csv",
            "powerlaw_r2": 0.97,
            "powerlaw_slope": 1.5,
            "exponential_r2": 0.55,
            "exponential_rate": 0.12,
            "flat_ratio": 14.2,
            "n_accounts": 64,
            "n_blocks": 128,
        });
        std::fs::write(
            &payload_path,
            serde_json::to_string_pretty(&payload).unwrap(),
        )
        .unwrap();

        cmd_genesis_stamp_result(
            payload_path.to_str().unwrap(),
            doc_path.to_str().unwrap(),
            "## A1.8",
            false,
            false,
        )
        .unwrap();

        let out = std::fs::read_to_string(&doc_path).unwrap();
        assert!(out.contains("MERA"));
        assert!(out.contains("Power-law R²"));
        assert!(out.contains("Prose here."));
        // Re-run with the same payload should be a no-op text-wise (timestamp
        // changes but `would_change` only fires on real diff). Verify by
        // running again and comparing the structural body excerpt.
        let _re = cmd_genesis_stamp_result(
            payload_path.to_str().unwrap(),
            doc_path.to_str().unwrap(),
            "## A1.8",
            false,
            false,
        );
        let out2 = std::fs::read_to_string(&doc_path).unwrap();
        // Decision still pinned to MERA on second pass.
        assert!(out2.contains("Decision | **MERA**"));
        assert_eq!(
            out.matches(STAMP_BEGIN).count(),
            1,
            "first run produced multiple marker blocks"
        );
        assert_eq!(
            out2.matches(STAMP_BEGIN).count(),
            1,
            "second run duplicated the marker block instead of replacing it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_csv_activations_handles_binary_and_floats() {
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("evaporchain-cli-gate-{}.csv", pid));
        std::fs::write(
            &path,
            "# header is ignored\n0,1,0,1\n1,1,0,0\n0.0,1.0,0.0,0.5\n",
        )
        .unwrap();
        let mat = parse_csv_activations(path.to_str().unwrap()).unwrap();
        assert_eq!(mat.len(), 3);
        assert_eq!(mat[0], vec![0.0, 1.0, 0.0, 1.0]);
        assert_eq!(mat[1], vec![1.0, 1.0, 0.0, 0.0]);
        assert_eq!(mat[2], vec![0.0, 1.0, 0.0, 0.5]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_csv_activations_rejects_ragged_rows() {
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("evaporchain-cli-gate-ragged-{}.csv", pid));
        std::fs::write(&path, "0,1,0\n1,1\n").unwrap();
        let err = parse_csv_activations(path.to_str().unwrap()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("rectangular") || msg.contains("columns"),
            "expected ragged-row error, got: {msg}"
        );
        let _ = std::fs::remove_file(&path);
    }

    // ────────────────── DA verify integration tests ──────────────────
    //
    // Spin up an in-process axum server that mirrors the three real
    // endpoints `cmd_da_verify` consumes: /api/block/:N (chain
    // attestation), /api/da/header/:N (2D header), /api/da/cell/:N/:r/:c
    // (per-cell proof). Drive `da_verify_inner` against the server and
    // assert each ChainAttestation outcome plus multi-peer round-robin.
    //
    // Pure in-process — `cargo test` runs them; no cluster.

    use axum::{extract::Path as AxumPath, routing::get, Router};
    use std::sync::Arc as StdArc;

    /// Per-test mode driving how the in-process server responds.
    #[derive(Clone, Copy)]
    enum DaServerMode {
        /// Honest server — real header, real cells, real chain root match.
        Honest,
        /// `/api/block/:N` returns a `data_root` that disagrees with the
        /// served 2D header. Should trigger ChainAttestation::Mismatch.
        ChainRootMismatch,
        /// `/api/block/:N` returns 404 — block aged out of the ring.
        /// Should resolve to BlockNotInRing.
        BlockNotInRing,
        /// `/api/block/:N` returns a record with `data_root: null`.
        /// Should resolve to NoDataRoot.
        NoDataRoot,
        /// `/api/da/cell/:N/:r/:c` returns proofs whose cell_data hashes
        /// to a different cell_hash — the sampler's verify should mark
        /// this peer as faulty.
        FabricatedCells,
    }

    struct TestDaServer {
        addr: String,
        // Keep the JoinHandle alive for the duration of the test; abort
        // happens on drop via the runtime shutdown.
        _handle: tokio::task::JoinHandle<()>,
    }

    fn build_test_package() -> evaporchain_da::block_da_2d::BlockDA2DPackage {
        // Big enough that 2D encoding picks a non-trivial extended_dim
        // and we have multiple rows/cols to sample.
        let payload: Vec<u8> = (0..2048u32).map(|i| (i % 251) as u8).collect();
        evaporchain_da::block_da_2d::BlockDA2D::new()
            .encode_block(&payload)
            .expect("test package must encode")
    }

    /// Build an axum router that serves the DA endpoints. Both `block`
    /// and the package are captured into closures so each test gets its
    /// own state.
    fn router_for(
        block: u64,
        package: evaporchain_da::block_da_2d::BlockDA2DPackage,
        mode: DaServerMode,
    ) -> Router {
        let pkg = StdArc::new(package);
        let pkg_for_block = pkg.clone();
        let pkg_for_header = pkg.clone();
        let pkg_for_cell = pkg.clone();

        let block_handler = move |AxumPath(n): AxumPath<u64>| {
            let pkg = pkg_for_block.clone();
            async move {
                if n != block {
                    return (
                        axum::http::StatusCode::NOT_FOUND,
                        axum::Json(serde_json::json!({"error": "block not found"})),
                    );
                }
                match mode {
                    DaServerMode::BlockNotInRing => (
                        axum::http::StatusCode::NOT_FOUND,
                        axum::Json(serde_json::json!({"error": "block not in ring"})),
                    ),
                    DaServerMode::NoDataRoot => (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({"number": n, "data_root": null})),
                    ),
                    DaServerMode::ChainRootMismatch => (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "number": n,
                            "data_root": format!("0x{}", hex::encode([0xFFu8; 32])),
                        })),
                    ),
                    DaServerMode::Honest | DaServerMode::FabricatedCells => (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "number": n,
                            "data_root": format!("0x{}", hex::encode(pkg.header.data_root)),
                        })),
                    ),
                }
            }
        };

        let header_handler = move |AxumPath(_n): AxumPath<u64>| {
            let pkg = pkg_for_header.clone();
            async move {
                let h = &pkg.header;
                axum::Json(serde_json::json!({
                    "data_root": hex::encode(h.data_root),
                    "row_roots": h.row_roots.iter().map(hex::encode).collect::<Vec<_>>(),
                    "col_roots": h.col_roots.iter().map(hex::encode).collect::<Vec<_>>(),
                    "extended_dim": h.extended_dim,
                    "original_dim": h.original_dim,
                    "cell_size": h.cell_size,
                    "original_len": h.original_len,
                    "data_hash": hex::encode(h.data_hash),
                }))
            }
        };

        let cell_handler = move |AxumPath((_n, row, col)): AxumPath<(u64, usize, usize)>| {
            let pkg = pkg_for_cell.clone();
            async move {
                let da = evaporchain_da::block_da_2d::BlockDA2D::new();
                let mut proof = match da.prove_cell(&pkg, row, col) {
                    Ok(p) => p,
                    Err(_) => {
                        return (
                            axum::http::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({"error": "no cell"})),
                        );
                    }
                };
                if matches!(mode, DaServerMode::FabricatedCells) {
                    // Mutate cell_data so the on-the-wire bytes hash to
                    // something other than cell_hash. The sampler's
                    // verify_cell_proof recomputes cell_hash from
                    // cell_data and rejects.
                    if !proof.cell_data.is_empty() {
                        proof.cell_data[0] ^= 0xFF;
                    } else {
                        proof.cell_data = vec![0xAA];
                    }
                }
                (
                    axum::http::StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "block": _n,
                        "row": row,
                        "col": col,
                        "cell_data": hex::encode(&proof.cell_data),
                        "cell_hash": hex::encode(proof.cell_hash),
                        "row_root": hex::encode(proof.row_root),
                        "col_root": hex::encode(proof.col_root),
                        "data_root": hex::encode(proof.data_root),
                        "extended_dim": pkg.header.extended_dim,
                        "row_proof_siblings": proof.row_siblings.iter().map(hex::encode).collect::<Vec<_>>(),
                        "col_proof_siblings": proof.col_siblings.iter().map(hex::encode).collect::<Vec<_>>(),
                    })),
                )
            }
        };

        Router::new()
            .route("/api/block/:n", get(block_handler))
            .route("/api/da/header/:n", get(header_handler))
            .route("/api/da/cell/:n/:row/:col", get(cell_handler))
    }

    async fn spawn_server(
        block: u64,
        package: evaporchain_da::block_da_2d::BlockDA2DPackage,
        mode: DaServerMode,
    ) -> TestDaServer {
        let router = router_for(block, package, mode);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        TestDaServer {
            addr: format!("http://{}", addr),
            _handle: handle,
        }
    }

    #[tokio::test]
    async fn da_verify_honest_server_passes() {
        let pkg = build_test_package();
        let server = spawn_server(7, pkg, DaServerMode::Honest).await;
        let outcome = da_verify_inner(
            &[server.addr.clone()],
            7,
            8,
            0.99,
            None,
            false,
        )
        .await
        .expect("honest server must produce an outcome");
        assert_eq!(outcome.attestation_label, "verified");
        assert!(outcome.all_valid, "every cell from honest server must verify");
        assert!(outcome.passes, "honest server must clear threshold");
    }

    #[tokio::test]
    async fn da_verify_chain_root_mismatch_aborts_before_sampling() {
        let pkg = build_test_package();
        let server = spawn_server(7, pkg, DaServerMode::ChainRootMismatch).await;
        let err = da_verify_inner(
            &[server.addr.clone()],
            7,
            8,
            0.99,
            None,
            false,
        )
        .await
        .expect_err("mismatch must abort");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("does not match"),
            "abort message must mention mismatch: {msg}"
        );
    }

    #[tokio::test]
    async fn da_verify_skip_attestation_proceeds_despite_chain_disagreement() {
        let pkg = build_test_package();
        // Even with a chain mismatch, --skip-chain-attestation lets the
        // sampler run. Cells from this honest header still verify, so
        // the threshold passes. The label reflects the opt-out.
        let server = spawn_server(7, pkg, DaServerMode::ChainRootMismatch).await;
        let outcome = da_verify_inner(
            &[server.addr.clone()],
            7,
            8,
            0.99,
            None,
            true, // skip_chain_attestation
        )
        .await
        .expect("skip flag must bypass the cross-check");
        assert_eq!(outcome.attestation_label, "skipped");
        assert!(outcome.passes);
    }

    #[tokio::test]
    async fn da_verify_block_not_in_ring_proceeds_with_label() {
        let pkg = build_test_package();
        let server = spawn_server(7, pkg, DaServerMode::BlockNotInRing).await;
        let outcome = da_verify_inner(
            &[server.addr.clone()],
            7,
            8,
            0.99,
            None,
            false,
        )
        .await
        .expect("block-not-in-ring is not a hard fail");
        assert_eq!(outcome.attestation_label, "block-not-in-ring");
        // Cells still verify against the served header; sampling passes.
        assert!(outcome.passes);
    }

    #[tokio::test]
    async fn da_verify_no_data_root_proceeds_with_label() {
        let pkg = build_test_package();
        let server = spawn_server(7, pkg, DaServerMode::NoDataRoot).await;
        let outcome = da_verify_inner(
            &[server.addr.clone()],
            7,
            8,
            0.99,
            None,
            false,
        )
        .await
        .expect("no-data-root is not a hard fail");
        assert_eq!(outcome.attestation_label, "no-data-root");
        assert!(outcome.passes);
    }

    #[tokio::test]
    async fn da_verify_fabricated_cells_marks_peer_faulty() {
        let pkg = build_test_package();
        let server = spawn_server(7, pkg, DaServerMode::FabricatedCells).await;
        let outcome = da_verify_inner(
            &[server.addr.clone()],
            7,
            8,
            0.99,
            None,
            false,
        )
        .await
        .expect("fabricated cells produce an outcome (with faulty peers)");
        assert!(
            !outcome.faulty_peers.is_empty(),
            "fabricated cells must trip the faulty-peer detector; got {:?}",
            outcome.faulty_peers
        );
        assert!(
            outcome.faulty_peers.iter().any(|(p, _)| p == &server.addr),
            "faulty peer must be the URL we pointed at"
        );
        assert!(!outcome.passes, "any faulty peer must fail the verify");
    }

    #[tokio::test]
    async fn da_verify_multi_peer_round_robins_across_nodes() {
        // Two HONEST servers + one FABRICATING server. Round-robin means
        // every third cell goes to the bad node. With 9 samples we hit
        // it 3 times → faulty_peers names that URL specifically.
        let pkg = build_test_package();
        let s1 = spawn_server(7, pkg.clone(), DaServerMode::Honest).await;
        let s2 = spawn_server(7, pkg.clone(), DaServerMode::Honest).await;
        let s3 = spawn_server(7, pkg, DaServerMode::FabricatedCells).await;
        let outcome = da_verify_inner(
            &[s1.addr.clone(), s2.addr.clone(), s3.addr.clone()],
            7,
            9,
            0.99,
            None,
            false,
        )
        .await
        .expect("multi-peer outcome");
        // The bad node served at least one cell; faulty_peers includes
        // its URL, NOT the honest ones.
        assert!(
            outcome.faulty_peers.iter().any(|(p, _)| p == &s3.addr),
            "bad node must appear in faulty_peers; got {:?}",
            outcome.faulty_peers
        );
        assert!(
            outcome.faulty_peers.iter().all(|(p, _)| p != &s1.addr),
            "honest node 1 must NOT appear in faulty_peers"
        );
        assert!(
            outcome.faulty_peers.iter().all(|(p, _)| p != &s2.addr),
            "honest node 2 must NOT appear in faulty_peers"
        );
        assert!(!outcome.passes);
    }

    #[test]
    fn testnet_init_writes_bootstrap_peers_into_genesis() {
        // Run cmd_testnet_init programmatically and assert that the
        // generated genesis.json contains one /p2p/-suffixed multiaddr per
        // validator, that each validator's data dir holds a network_key.bin
        // matching the published PeerId, and that the multiaddr embeds the
        // listen-ip override.
        let n = 3u32;
        let listen_ip = "100.64.0.7";
        let p2p_base = 19_000u16;
        let api_base = 18_000u16;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp =
            std::env::temp_dir().join(format!("evapor-testnet-init-{nonce}"));
        let _ = std::fs::remove_dir_all(&tmp);

        cmd_testnet_init(
            tmp.to_str().unwrap(),
            n,
            "evaporchain-test-init-1",
            10_000_000,
            1_000_000,
            500,
            p2p_base,
            api_base,
            listen_ip,
            true,
        )
        .expect("cmd_testnet_init");

        let genesis_json = std::fs::read_to_string(tmp.join("genesis.json"))
            .expect("read genesis.json");
        let parsed: GenesisConfig = serde_json::from_str(&genesis_json)
            .expect("parse genesis.json");

        assert_eq!(
            parsed.bootstrap_peers.len(),
            n as usize,
            "bootstrap_peers must contain one entry per validator"
        );
        for (i, ma) in parsed.bootstrap_peers.iter().enumerate() {
            let vid = (i + 1) as u16;
            let expected_port = p2p_base + vid;
            assert!(
                ma.starts_with(&format!("/ip4/{}/tcp/{}/p2p/", listen_ip, expected_port)),
                "bootstrap_peer[{i}] {ma} must use --listen-ip and /p2p/<peer_id> suffix"
            );
            // PeerId tail must match the network_key on disk.
            let key_path = tmp
                .join(format!("v{}", vid))
                .join("data")
                .join("network_key.bin");
            assert!(key_path.is_file(), "network_key.bin missing for v{vid}");
            let bytes = std::fs::read(&key_path).unwrap();
            let kp = libp2p_identity::Keypair::from_protobuf_encoding(&bytes)
                .expect("decode network_key.bin");
            let pid = kp.public().to_peer_id().to_string();
            assert!(
                ma.ends_with(&format!("/p2p/{}", pid)),
                "bootstrap_peer[{i}] {ma} must end with /p2p/{pid}"
            );
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
