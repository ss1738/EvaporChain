//! CLI command definitions and handlers for the EvaporChain wallet.
//!
//! Uses clap derive for argument parsing. Each subcommand maps to a
//! module method. Output is formatted with colored text.

use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;

use crate::account::AccountManager;
use crate::address::{format_address, parse_address};
use crate::assets::AssetManager;
use crate::auto_refresh::{AutoRefreshConfig, AutoRefresher};
use crate::backup::BackupManager;
use crate::config::WalletConfig;
use crate::contacts::AddressBook;
use crate::energy::{AlertSeverity, EnergyMonitor};
use crate::gas::GasEstimator;
use crate::history::TxHistory;
use crate::keystore::KeyStore;
use crate::mnemonic::{Mnemonic, MnemonicBackup};
use crate::offline::{Broadcaster, OfflineSigner, SignedTransaction};
use crate::pipeline::TxPipeline;
use crate::rpc::RpcClient;
use crate::staking::{GovernanceManager, StakingManager};
use crate::validation;

// ──────────────────────────── CLI Definitions ──────────────────────────

#[derive(Parser)]
#[command(name = "evaporchain-wallet")]
#[command(about = "EvaporChain Post-Quantum Wallet", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Node RPC URL.
    #[arg(long, default_value = "http://localhost:3000", global = true)]
    pub node: String,

    /// Path to keystore file.
    #[arg(long, default_value = "~/.evaporchain/keystore.json", global = true)]
    pub keystore: String,

    /// Output as JSON (for scripts and bots).
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Account management.
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// Send EVAP tokens.
    Send {
        /// Recipient address (0x hex).
        to: String,
        /// Amount to send.
        amount: u64,
        /// Wait for on-chain confirmation.
        #[arg(long)]
        wait: bool,
    },
    /// Request testnet tokens from faucet.
    Faucet,
    /// View state objects.
    Objects,
    /// View a single object.
    Object {
        /// Object ID (hex).
        id: String,
    },
    /// Refresh an object's energy.
    Refresh {
        /// Object ID (hex).
        id: String,
        /// Energy to deposit.
        energy: u64,
        /// Wait for on-chain confirmation.
        #[arg(long)]
        wait: bool,
    },
    /// NFT operations.
    Nft {
        #[command(subcommand)]
        action: NftAction,
    },
    /// Token operations.
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    /// Energy monitoring and decay forecasting.
    Energy {
        #[command(subcommand)]
        action: EnergyAction,
    },
    /// Ghost (evaporated) objects.
    Ghost {
        #[command(subcommand)]
        action: GhostAction,
    },
    /// Staking operations.
    Stake {
        #[command(subcommand)]
        action: StakeAction,
    },
    /// DAO governance.
    Dao {
        #[command(subcommand)]
        action: DaoAction,
    },
    /// Backup and recovery.
    Backup {
        #[command(subcommand)]
        action: BackupAction,
    },
    /// Seed phrase (mnemonic) management.
    Seed {
        #[command(subcommand)]
        action: SeedAction,
    },
    /// Address book (contacts).
    Contacts {
        #[command(subcommand)]
        action: ContactAction,
    },
    /// Transaction history.
    History {
        #[command(subcommand)]
        action: HistoryAction,
    },
    /// Gas and fee estimation.
    Gas {
        #[command(subcommand)]
        action: GasAction,
    },
    /// Wallet configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Offline signing and broadcast.
    Offline {
        #[command(subcommand)]
        action: OfflineAction,
    },
    /// Show detailed version and build info.
    Version,
    /// Run self-diagnostic checks.
    Doctor,
    /// Execute a batch of transactions from a JSON file.
    Batch {
        /// Path to batch JSON file.
        file: PathBuf,
        /// Dry-run: validate and show summary without executing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Multi-account portfolio dashboard.
    Dashboard,
    /// Watch any address (no keys needed).
    Watch {
        /// Address to monitor (0x hex).
        address: String,
    },
    /// Interactive guided mode for new users.
    Interactive,
    /// Generate shell completions.
    Completions {
        /// Shell to generate completions for.
        shell: String,
    },
    /// Chain status.
    Status,
    /// View recent blocks.
    Blocks {
        /// Number of blocks to show.
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// View a transaction.
    Tx {
        /// Transaction hash.
        hash: String,
    },
    /// Simulate a transaction (dry-run without signing).
    Simulate {
        #[command(subcommand)]
        action: SimulateAction,
    },
    /// Spending limits and address policies.
    Spending {
        #[command(subcommand)]
        action: SpendingAction,
    },
    /// Multi-signature transaction management.
    Multisig {
        #[command(subcommand)]
        action: MultisigAction,
    },
    /// Transaction lifecycle hooks.
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },
    /// Address labels and tx annotations.
    Labels {
        #[command(subcommand)]
        action: LabelsAction,
    },
    /// Fee market analytics and timing.
    Fees {
        #[command(subcommand)]
        action: FeesAction,
    },
    /// Hardware wallet management.
    Hardware {
        #[command(subcommand)]
        action: HardwareAction,
    },
    /// dApp session management.
    Dapp {
        #[command(subcommand)]
        action: DappAction,
    },
    /// Notification center.
    Notifications {
        #[command(subcommand)]
        action: NotificationsAction,
    },
    /// Session keys and account abstraction.
    SessionKeys {
        #[command(subcommand)]
        action: SessionKeysAction,
    },
    /// Cross-chain bridge.
    Bridge {
        #[command(subcommand)]
        action: BridgeAction,
    },
    /// Language and locale settings.
    Lang {
        #[command(subcommand)]
        action: LangAction,
    },
    /// Transaction templates and recurring payments.
    Templates {
        #[command(subcommand)]
        action: TemplatesAction,
    },
    /// Portfolio analytics and spending trends.
    Analytics {
        #[command(subcommand)]
        action: AnalyticsAction,
    },
    /// Address reputation and risk scoring.
    Reputation {
        #[command(subcommand)]
        action: ReputationAction,
    },
    /// Watchtower daemon — background monitoring.
    Watchtower {
        #[command(subcommand)]
        action: WatchtowerAction,
    },
    /// Tamper-evident audit log.
    Audit {
        #[command(subcommand)]
        action: AuditAction2,
    },
    /// Tax reporting and cost basis.
    Tax {
        #[command(subcommand)]
        action: TaxAction,
    },
    /// Composable transaction policies.
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Export data in various formats.
    Export {
        #[command(subcommand)]
        action: ExportAction,
    },
    /// Scripting DSL for automated workflows.
    Script {
        #[command(subcommand)]
        action: ScriptAction,
    },
    /// Prometheus-style metrics and telemetry.
    Metrics {
        #[command(subcommand)]
        action: MetricsAction,
    },
    /// Migrate wallets from external formats.
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
    /// QR code generation.
    Qr {
        #[command(subcommand)]
        action: QrAction,
    },
    /// Performance benchmarks.
    Bench {
        #[command(subcommand)]
        action: BenchAction,
    },
    /// Wallet health checks.
    Health {
        #[command(subcommand)]
        action: HealthAction,
    },
    /// Plugin management.
    Plugin {
        #[command(subcommand)]
        action: PluginAction2,
    },
    /// Scheduled recurring tasks.
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
    /// Address allowlist/denylist management.
    Allowlist {
        #[command(subcommand)]
        action: AllowlistAction,
    },
    /// Time-locked transactions and vesting.
    Timelock {
        #[command(subcommand)]
        action: TimelockAction,
    },
    /// Transaction memos.
    Memo {
        #[command(subcommand)]
        action: MemoAction2,
    },
    /// Wallet recovery (dead man's switch + social).
    Recovery {
        #[command(subcommand)]
        action: RecoveryAction2,
    },
    /// Token delegation management.
    Delegation {
        #[command(subcommand)]
        action: DelegationAction,
    },
    /// Address book sync across devices.
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// Gas station network (meta-transactions).
    GasStation {
        #[command(subcommand)]
        action: GasStationAction,
    },
    /// Intent-based transactions.
    Intent {
        #[command(subcommand)]
        action: IntentAction,
    },
    /// Token metadata registry.
    TokenRegistry {
        #[command(subcommand)]
        action: TokenRegistryAction,
    },
    /// Fee bumping for stuck transactions.
    FeeBump {
        #[command(subcommand)]
        action: FeeBumpAction,
    },
    /// Wallet state snapshots.
    Snapshot {
        #[command(subcommand)]
        action: SnapshotAction,
    },
    /// Watch-only account tracking.
    WatchOnly {
        #[command(subcommand)]
        action: WatchOnlyAction,
    },
    /// Peer discovery and RPC failover.
    Peers {
        #[command(subcommand)]
        action: PeerAction,
    },
    /// Mempool monitoring and gas oracle.
    Mempool {
        #[command(subcommand)]
        action: MempoolAction,
    },
    /// Chain event indexer.
    Indexer {
        #[command(subcommand)]
        action: IndexerAction,
    },
    /// Network health dashboard.
    NetHealth {
        #[command(subcommand)]
        action: NetHealthAction,
    },
    /// Price feed and portfolio valuation.
    Price {
        #[command(subcommand)]
        action: PriceAction,
    },
    /// Address risk scoring.
    Risk {
        #[command(subcommand)]
        action: RiskAction,
    },
    /// Transaction decoder.
    Decode {
        #[command(subcommand)]
        action: DecodeAction,
    },
    /// Notification rules engine.
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },
    /// Contract ABI management.
    Abi {
        #[command(subcommand)]
        action: AbiAction,
    },
    /// Name service (ENS-style).
    Names {
        #[command(subcommand)]
        action: NameAction,
    },
    /// Transaction preview and simulation.
    Preview {
        #[command(subcommand)]
        action: PreviewAction,
    },
    /// WalletConnect session management.
    Connect {
        #[command(subcommand)]
        action: ConnectAction,
    },
    /// Privacy shield (stealth addresses, mixing).
    Privacy {
        #[command(subcommand)]
        action: PrivacyAction,
    },
    /// Key rotation management.
    KeyRotation {
        #[command(subcommand)]
        action: KeyRotAction,
    },
    /// Access control (RBAC).
    Access {
        #[command(subcommand)]
        action: AccessAction2,
    },
    /// Threat monitoring.
    Threats {
        #[command(subcommand)]
        action: ThreatAction,
    },
    /// Liquidity pool management.
    Pool {
        #[command(subcommand)]
        action: PoolAction,
    },
    /// Yield farming.
    Farm {
        #[command(subcommand)]
        action: FarmAction,
    },
    /// Cross-chain swaps.
    CrossSwap {
        #[command(subcommand)]
        action: CrossSwapAction,
    },
    /// Flash loan composition.
    Flash {
        #[command(subcommand)]
        action: FlashAction2,
    },
    /// Dollar-cost averaging.
    Dca {
        #[command(subcommand)]
        action: DcaAction,
    },
    /// Limit orders.
    LimitOrder {
        #[command(subcommand)]
        action: LimitAction,
    },
    /// Portfolio rebalancing.
    Rebalance {
        #[command(subcommand)]
        action: RebalAction,
    },
    /// Smart alerts.
    Alerts {
        #[command(subcommand)]
        action: SmartAlertAction,
    },
    /// Social recovery (guardians).
    Recovery2 {
        #[command(subcommand)]
        action: RecoveryAction3,
    },
    /// Shared vaults.
    Vault {
        #[command(subcommand)]
        action: VaultAction,
    },
    /// Payment streams.
    Stream {
        #[command(subcommand)]
        action: StreamAction,
    },
    /// Escrow management.
    Escrow {
        #[command(subcommand)]
        action: EscrowAction,
    },
    /// Profit & Loss tracking.
    Pnl {
        #[command(subcommand)]
        action: PnlAction,
    },
    /// Portfolio analytics & risk metrics.
    Analytics2 {
        #[command(subcommand)]
        action: AnalyticsAction2,
    },
    /// Compliance reporting.
    Compliance {
        #[command(subcommand)]
        action: ComplianceAction,
    },
    /// Whale tracking.
    Whale {
        #[command(subcommand)]
        action: WhaleAction,
    },
    /// Energy decay optimizer (EvaporChain-specific).
    EnergyOpt {
        #[command(subcommand)]
        action: EnergyOptAction,
    },
    /// Object lifecycle manager.
    Objects2 {
        #[command(subcommand)]
        action: ObjMgrAction,
    },
    /// Contract deployment.
    Deploy {
        #[command(subcommand)]
        action: DeployAction,
    },
    /// Governance dashboard.
    Gov {
        #[command(subcommand)]
        action: GovAction,
    },
    /// Fee optimization.
    FeeOpt {
        #[command(subcommand)]
        action: FeeOptAction,
    },
    /// Batch transaction execution.
    BatchExec {
        #[command(subcommand)]
        action: BatchExecAction,
    },
    /// Wallet migration/import.
    Migrate2 {
        #[command(subcommand)]
        action: MigrateAction2,
    },
    /// Wallet diagnostics.
    Diag {
        #[command(subcommand)]
        action: DiagAction,
    },
    /// WebSocket event subscriptions.
    Ws {
        #[command(subcommand)]
        action: WsAction,
    },
    /// Internal event bus.
    EventBus {
        #[command(subcommand)]
        action: EventBusAction,
    },
    /// Transaction receipt store.
    Receipts {
        #[command(subcommand)]
        action: ReceiptAction,
    },
    /// Blockchain state sync.
    StateSync {
        #[command(subcommand)]
        action: StateSyncAction,
    },
    /// Debug console.
    Debug {
        #[command(subcommand)]
        action: DebugAction,
    },
    /// Gas profiler.
    GasProfile {
        #[command(subcommand)]
        action: GasProfileAction,
    },
    /// Contract verification.
    Verify {
        #[command(subcommand)]
        action: VerifyAction,
    },
    /// Transaction simulation.
    Simulate2 {
        #[command(subcommand)]
        action: SimAction,
    },
    /// Immutable audit trail.
    AuditTrail {
        #[command(subcommand)]
        action: AuditTrailAction,
    },
    /// Anomaly detection.
    Anomaly {
        #[command(subcommand)]
        action: AnomalyAction,
    },
    /// Secure enclave (key isolation).
    Enclave {
        #[command(subcommand)]
        action: EnclaveAction,
    },
    /// Permission management.
    Perms {
        #[command(subcommand)]
        action: PermAction,
    },
    /// UI theme management.
    Theme {
        #[command(subcommand)]
        action: ThemeAction,
    },
    /// Command palette.
    Palette {
        #[command(subcommand)]
        action: PaletteAction,
    },
    /// Onboarding wizard.
    Onboard {
        #[command(subcommand)]
        action: OnboardAction,
    },
    /// Help system.
    HelpTopic {
        #[command(subcommand)]
        action: HelpAction,
    },
    /// Circuit breaker management.
    Breaker {
        #[command(subcommand)]
        action: BreakerAction,
    },
    /// Cache management.
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Configuration validation.
    ConfigVal {
        #[command(subcommand)]
        action: ConfigValAction,
    },
    /// Background task queue.
    Tasks {
        #[command(subcommand)]
        action: TaskQueueAction,
    },
    /// Changelog management.
    Changelog {
        #[command(subcommand)]
        action: ChangelogAction,
    },
    /// Feature flags.
    Flags {
        #[command(subcommand)]
        action: FlagAction,
    },
    /// Telemetry management.
    Telemetry {
        #[command(subcommand)]
        action: TelemetryAction,
    },
    /// Wallet API management.
    Api {
        #[command(subcommand)]
        action: ApiAction,
    },
    /// Test harness.
    Harness {
        #[command(subcommand)]
        action: HarnessAction,
    },
    /// Fuzz testing.
    Fuzz {
        #[command(subcommand)]
        action: FuzzAction,
    },
    /// Regression tracking.
    Regression {
        #[command(subcommand)]
        action: RegressionAction,
    },
    /// Code coverage.
    Coverage {
        #[command(subcommand)]
        action: CoverageAction,
    },
}

#[derive(Subcommand)]
pub enum AccountAction {
    /// Create a new account.
    Create { name: String },
    /// List all accounts.
    List,
    /// Switch active account.
    Switch { name: String },
    /// Show account balance.
    Balance { name: Option<String> },
    /// Show detailed account info.
    Detail { name: Option<String> },
}

#[derive(Subcommand)]
pub enum NftAction {
    /// List all NFTs.
    List,
    /// View a single NFT.
    Show { id: u64 },
    /// Mint a new NFT.
    Mint {
        name: String,
        energy: u64,
        half_life: u64,
        #[arg(long)]
        collection: Option<String>,
        #[arg(long, default_value = "{}")]
        metadata: String,
    },
    /// Transfer an NFT.
    Transfer { id: u64, to: String },
    /// Refresh an NFT's energy.
    Refresh { id: u64, energy: u64 },
}

#[derive(Subcommand)]
pub enum TokenAction {
    /// List all tokens.
    List,
    /// View a single token.
    Show { id: u64 },
    /// Deploy a new token.
    Deploy {
        name: String,
        symbol: String,
        supply: u64,
        half_life: u64,
    },
    /// Transfer tokens.
    Transfer {
        token_id: u64,
        to: String,
        amount: u64,
    },
}

#[derive(Subcommand)]
pub enum EnergyAction {
    /// Scan all assets and show energy alerts.
    Scan,
    /// Show detailed decay forecast for an object.
    Forecast { id: String },
    /// Start auto-refresh daemon (monitors and refreshes low-energy assets).
    AutoRefresh {
        /// Energy threshold percentage (refresh when below this).
        #[arg(long, default_value = "25.0")]
        threshold: f64,
        /// Poll interval in seconds.
        #[arg(long, default_value = "60")]
        interval: u64,
        /// Maximum energy per refresh.
        #[arg(long, default_value = "10000")]
        max_energy: u64,
        /// Run one cycle only (don't loop).
        #[arg(long)]
        once: bool,
    },
}

#[derive(Subcommand)]
pub enum GhostAction {
    /// List all ghost (evaporated) objects.
    List,
    /// Estimate resurrection cost.
    Cost {
        /// Half-life of the object to resurrect.
        half_life: u64,
    },
}

#[derive(Subcommand)]
pub enum StakeAction {
    /// List staking pools.
    Pools,
    /// Stake tokens in a pool.
    In { pool_id: u64, amount: u64 },
    /// Unstake tokens from a pool.
    Out { pool_id: u64, amount: u64 },
    /// Claim staking rewards.
    Claim { pool_id: u64 },
    /// Forecast rewards.
    Forecast {
        pool_id: u64,
        amount: u64,
        epochs: u64,
    },
}

#[derive(Subcommand)]
pub enum DaoAction {
    /// List proposals.
    Proposals,
    /// View a proposal.
    Show { id: u64 },
    /// Vote on a proposal.
    Vote {
        id: u64,
        option: String,
        weight: u64,
    },
}

#[derive(Subcommand)]
pub enum BackupAction {
    /// Export encrypted backup.
    Export { file: PathBuf },
    /// Import encrypted backup.
    Import { file: PathBuf },
    /// Rotate all key passwords.
    Rotate,
    /// List public keys (view-only export).
    Keys,
}

#[derive(Subcommand)]
pub enum SeedAction {
    /// Generate a new 24-word seed phrase.
    Generate,
    /// Backup a keypair under a seed phrase.
    Backup {
        /// Account name to back up.
        name: String,
        /// Output file for the encrypted backup.
        file: PathBuf,
    },
    /// Recover a keypair from seed phrase + backup file.
    Recover {
        /// Backup file to decrypt.
        file: PathBuf,
        /// Name for the recovered account.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum ContactAction {
    /// Add a contact.
    Add {
        /// Contact name.
        name: String,
        /// Contact address (0x hex).
        address: String,
        /// Optional note.
        #[arg(long)]
        note: Option<String>,
    },
    /// List all contacts.
    List,
    /// Remove a contact.
    Remove { name: String },
    /// Show a contact's details.
    Show { name: String },
    /// Export contacts to CSV or JSON.
    Export {
        /// Output file (.csv or .json).
        file: PathBuf,
    },
    /// Import contacts from CSV or JSON.
    Import {
        /// Input file (.csv or .json).
        file: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum HistoryAction {
    /// Show recent transaction history.
    List {
        /// Number of entries to show.
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Show transactions for a specific address.
    For { address: String },
    /// Export history to CSV file.
    Export {
        /// Output file path.
        file: PathBuf,
    },
    /// Clear all history.
    Clear,
}

#[derive(Subcommand)]
pub enum GasAction {
    /// Estimate gas for a transfer.
    Transfer,
    /// Estimate gas for creating an object.
    Create {
        /// Data size in bytes.
        #[arg(long, default_value = "100")]
        size: usize,
    },
    /// Estimate gas for refreshing an object.
    Refresh {
        /// Energy to deposit.
        energy: u64,
    },
    /// Show current base fee from latest block.
    BaseFee,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show current configuration.
    Show,
    /// Set a config value.
    Set {
        /// Key to set (node_url, active_account, default_half_life, default_energy).
        key: String,
        /// New value.
        value: String,
    },
    /// Reset config to defaults.
    Reset,
}

#[derive(Subcommand)]
pub enum OfflineAction {
    /// Sign a transfer transaction offline (air-gapped).
    Sign {
        /// Recipient address (0x hex).
        to: String,
        /// Amount to send.
        amount: u64,
        /// Nonce to use.
        nonce: u64,
        /// Output file for signed transaction.
        #[arg(short, long, default_value = "signed_tx.json")]
        file: PathBuf,
    },
    /// Sign a refresh transaction offline.
    SignRefresh {
        /// Object ID (hex).
        id: String,
        /// Energy to deposit.
        energy: u64,
        /// Output file for signed transaction.
        #[arg(short, long, default_value = "signed_tx.json")]
        file: PathBuf,
    },
    /// Broadcast a previously signed transaction.
    Broadcast {
        /// Signed transaction JSON file.
        file: PathBuf,
    },
    /// Inspect a signed transaction file.
    Inspect {
        /// Signed transaction JSON file.
        file: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum SimulateAction {
    /// Simulate a transfer.
    Send {
        /// Recipient address (0x hex or contact name).
        to: String,
        /// Amount to send.
        amount: u64,
    },
    /// Simulate a refresh.
    Refresh {
        /// Object ID (hex).
        id: String,
        /// Energy to deposit.
        energy: u64,
    },
}

#[derive(Subcommand)]
pub enum SpendingAction {
    /// Show current spending policy.
    Show,
    /// Set per-transaction limit.
    SetTxLimit { amount: u64 },
    /// Set daily spending limit.
    SetDailyLimit { amount: u64 },
    /// Set enforcement mode (enforce / warn / disabled).
    SetMode { mode: String },
    /// Add address to allowlist.
    Allow { address: String },
    /// Remove address from allowlist.
    Unallow { address: String },
    /// Add address to blocklist.
    Block { address: String },
    /// Remove address from blocklist.
    Unblock { address: String },
    /// Show daily spending status.
    Status,
    /// Reset daily spending counter.
    ResetDaily,
}

#[derive(Subcommand)]
pub enum MultisigAction {
    /// Create a new multisig group.
    CreateGroup {
        /// Group name.
        name: String,
        /// Member addresses (comma-separated).
        members: String,
        /// Required approvals.
        threshold: usize,
    },
    /// List all multisig groups.
    Groups,
    /// Propose a transfer.
    Propose {
        /// Group name.
        group: String,
        /// Recipient address.
        to: String,
        /// Amount.
        amount: u64,
        /// Optional memo.
        #[arg(long)]
        memo: Option<String>,
    },
    /// Approve a proposal.
    Approve {
        /// Proposal ID.
        id: String,
    },
    /// List proposals for a group.
    Proposals {
        /// Group name.
        group: String,
    },
    /// Show proposal details.
    ShowProposal {
        /// Proposal ID.
        id: String,
    },
    /// Remove a multisig group.
    RemoveGroup {
        /// Group name.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum HooksAction {
    /// List all hooks.
    List,
    /// Add a shell hook.
    AddShell {
        /// Hook name.
        name: String,
        /// Event (pre_send, post_send, pre_refresh, post_refresh, on_error).
        event: String,
        /// Shell command.
        command: String,
        /// Block transaction on failure.
        #[arg(long)]
        blocking: bool,
    },
    /// Add a log hook.
    AddLog {
        /// Hook name.
        name: String,
        /// Event.
        event: String,
        /// Log file path.
        file: String,
        /// Log format (with {event}, {from}, {to}, {amount}, etc.).
        #[arg(long)]
        format: Option<String>,
    },
    /// Remove a hook.
    Remove {
        /// Hook name.
        name: String,
    },
    /// Enable a hook.
    Enable {
        /// Hook name.
        name: String,
    },
    /// Disable a hook.
    Disable {
        /// Hook name.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum LabelsAction {
    /// Label an address.
    Add {
        /// Address (0x hex).
        address: String,
        /// Human-readable name.
        name: String,
        /// Category (personal/exchange/defi/contract/dao/staking/nft/faucet).
        #[arg(long, default_value = "unknown")]
        category: String,
        /// Tags (comma-separated).
        #[arg(long)]
        tags: Option<String>,
        /// Note.
        #[arg(long)]
        note: Option<String>,
    },
    /// List all address labels.
    List,
    /// Search labels by name/address/tag.
    Search { query: String },
    /// Remove an address label.
    Remove { address: String },
    /// Annotate a transaction.
    Annotate {
        /// Transaction hash.
        tx_hash: String,
        /// Note.
        #[arg(long)]
        note: Option<String>,
        /// Tags (comma-separated).
        #[arg(long)]
        tags: Option<String>,
        /// Category.
        #[arg(long)]
        category: Option<String>,
    },
    /// List transaction annotations.
    Annotations,
}

#[derive(Subcommand)]
pub enum FeesAction {
    /// Show fee statistics and trend.
    Stats,
    /// Get timing advice (is now a good time to transact?).
    Timing,
    /// Record current block fees (run periodically to build history).
    Record,
    /// Add a fee alert.
    Alert {
        /// Alert name.
        name: String,
        /// Target base fee (alert when fee drops below this).
        target: u64,
    },
    /// List fee alerts.
    Alerts,
    /// Remove a fee alert.
    RemoveAlert { name: String },
}

#[derive(Subcommand)]
pub enum HardwareAction {
    /// List registered hardware devices.
    List,
    /// Register a simulated device (for testing).
    AddSimulated {
        /// Device name.
        name: String,
    },
    /// Remove a device.
    Remove {
        /// Device ID.
        id: String,
    },
    /// Show device info.
    Info {
        /// Device ID.
        id: String,
    },
}

#[derive(Subcommand)]
pub enum DappAction {
    /// List dApp sessions.
    Sessions,
    /// Create a new dApp session.
    Connect {
        /// dApp origin URL.
        origin: String,
        /// dApp name.
        name: String,
        /// Permissions (comma-separated: view_account,request_sign,etc.).
        permissions: String,
        /// Session duration in hours.
        #[arg(long, default_value = "24")]
        hours: u64,
    },
    /// Revoke a dApp session.
    Revoke {
        /// Session ID.
        id: String,
    },
    /// Revoke all sessions for an origin.
    RevokeOrigin {
        /// dApp origin URL.
        origin: String,
    },
    /// Show session details.
    Show {
        /// Session ID.
        id: String,
    },
}

#[derive(Subcommand)]
pub enum NotificationsAction {
    /// Show unread notifications.
    Unread,
    /// Show recent notifications.
    Recent {
        /// Number of notifications to show.
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Mark a notification as read.
    Read { id: String },
    /// Mark all notifications as read.
    ReadAll,
    /// Filter by category (energy_decay, tx_confirmed, tx_failed, fee_alert, security, session_expiry, system).
    Filter {
        /// Category to filter by.
        category: String,
    },
    /// Clear all notification history.
    Clear,
    /// Show notification count.
    Count,
}

#[derive(Subcommand)]
pub enum SessionKeysAction {
    /// List active session keys.
    List,
    /// Create a new session key.
    Create {
        /// Label for the session key.
        label: String,
        /// Max amount per transaction.
        #[arg(long, default_value = "1000")]
        max_per_tx: u64,
        /// Total spending limit (0 for unlimited).
        #[arg(long, default_value = "0")]
        total_limit: u64,
        /// Allowed operations (comma-separated: transfer, refresh, stake).
        #[arg(long, default_value = "transfer")]
        ops: String,
        /// Duration in hours.
        #[arg(long, default_value = "24")]
        hours: u64,
    },
    /// Revoke a session key.
    Revoke { id: String },
    /// Show session key details.
    Show { id: String },
    /// Set up social recovery.
    SetupRecovery {
        /// Required guardian approvals.
        threshold: usize,
        /// Recovery delay in hours.
        #[arg(long, default_value = "48")]
        delay_hours: u64,
    },
    /// Add a recovery guardian.
    AddGuardian {
        /// Guardian address.
        address: String,
        /// Guardian name.
        name: String,
    },
    /// Show recovery configuration.
    RecoveryInfo,
    /// Set up gas sponsorship.
    SetSponsor {
        /// Sponsor address.
        address: String,
        /// Max gas per transaction.
        #[arg(long, default_value = "100000")]
        max_gas: u64,
        /// Daily gas budget.
        #[arg(long, default_value = "1000000")]
        daily_budget: u64,
    },
    /// Show sponsor configuration.
    SponsorInfo,
}

#[derive(Subcommand)]
pub enum BridgeAction {
    /// List registered bridges.
    List,
    /// Find bridges between two chains.
    Find {
        /// Source chain (evaporchain, ethereum, solana, sui, aptos, polygon, arbitrum).
        source: String,
        /// Destination chain.
        dest: String,
    },
    /// Register a new bridge.
    Register {
        /// Bridge name.
        name: String,
        /// Source chain.
        source: String,
        /// Destination chain.
        dest: String,
        /// Bridge type (lock_mint, burn_mint, liquidity_pool, native).
        #[arg(long, default_value = "lock_mint")]
        bridge_type: String,
        /// Fee percentage (e.g. 0.1 for 0.1%).
        #[arg(long, default_value = "0.1")]
        fee_pct: f64,
    },
    /// Initiate a bridge transfer.
    Transfer {
        /// Bridge ID.
        bridge_id: String,
        /// Token symbol (e.g. EVAP, WETH).
        token: String,
        /// Amount to bridge.
        amount: u64,
        /// Sender address.
        sender: String,
        /// Recipient address on destination chain.
        recipient: String,
    },
    /// Show pending bridge transfers.
    Pending,
    /// Show a specific transfer.
    Show { id: String },
    /// Remove a bridge.
    Remove { id: String },
}

#[derive(Subcommand)]
pub enum LangAction {
    /// Show current locale.
    Show,
    /// Set locale.
    Set {
        /// Locale code (en, es, fr, de, ja, zh, ko, hi, ar, pt, ru).
        locale: String,
    },
    /// List all supported locales.
    List,
    /// Show a sample message in the current locale.
    Test {
        /// Message key to test (welcome, success, error, etc.).
        #[arg(default_value = "welcome")]
        key: String,
    },
}

#[derive(Subcommand)]
pub enum TemplatesAction {
    /// List all templates.
    List,
    /// Create a transfer template.
    CreateTransfer {
        /// Template name.
        name: String,
        /// Recipient address.
        to: String,
        /// Amount.
        amount: u64,
        /// Frequency (once, daily, weekly, monthly, hourly:N, daily:N).
        #[arg(long, default_value = "once")]
        frequency: String,
    },
    /// Create a refresh template.
    CreateRefresh {
        /// Template name.
        name: String,
        /// Object ID.
        object_id: String,
        /// Energy amount.
        energy: u64,
        /// Frequency.
        #[arg(long, default_value = "weekly")]
        frequency: String,
    },
    /// Show template details.
    Show { name: String },
    /// Remove a template.
    Remove { name: String },
    /// Enable a template.
    Enable { name: String },
    /// Disable a template.
    Disable { name: String },
    /// Mark a template as executed.
    Execute { name: String },
    /// List templates due for execution.
    Due,
    /// Show recurring templates only.
    Recurring,
    /// Search templates.
    Search { query: String },
}

#[derive(Subcommand)]
pub enum AnalyticsAction {
    /// Show period summary (day, week, month, all).
    Summary {
        #[arg(default_value = "week")]
        period: String,
    },
    /// Show spending breakdown by category.
    Breakdown {
        #[arg(default_value = "month")]
        period: String,
    },
    /// Show trend comparison (current vs previous period).
    Trend {
        #[arg(default_value = "week")]
        period: String,
    },
    /// Record an event manually.
    Record {
        /// Event type (transfer_out, transfer_in, energy_spend, gas_fee, etc.).
        event: String,
        /// Amount.
        amount: u64,
        /// Balance after event.
        balance: u64,
        /// Reference (tx hash, etc.).
        #[arg(long, default_value = "manual")]
        reference: String,
    },
    /// Show current balance from latest data point.
    Balance,
    /// Clear all analytics data.
    Clear,
}

#[derive(Subcommand)]
pub enum ReputationAction {
    /// Check an address reputation.
    Check { address: String },
    /// Flag an address.
    Flag {
        address: String,
        /// Flag type (scam, phishing, fresh_wallet, dust_attack, mixer, community_report).
        flag: String,
        /// Optional note.
        #[arg(long)]
        note: Option<String>,
    },
    /// Remove a flag.
    Unflag {
        address: String,
        /// Flag to remove.
        flag: String,
    },
    /// Verify (whitelist) an address.
    Verify {
        address: String,
        /// Optional label.
        #[arg(long)]
        label: Option<String>,
    },
    /// List dangerous addresses.
    Dangerous,
    /// List verified addresses.
    Verified,
    /// Search reputation database.
    Search { query: String },
    /// Set thresholds.
    Thresholds {
        /// Block threshold (0-100).
        #[arg(long)]
        block: Option<u8>,
        /// Warn threshold (0-100).
        #[arg(long)]
        warn: Option<u8>,
    },
}

#[derive(Subcommand)]
pub enum WatchtowerAction {
    /// List all watches.
    List,
    /// Add a balance watch.
    WatchBalance {
        /// Watch name.
        name: String,
        /// Address to monitor.
        address: String,
        /// Alert when balance drops below this.
        threshold: f64,
        /// Poll interval in seconds.
        #[arg(long, default_value = "60")]
        interval: u64,
    },
    /// Add an energy watch.
    WatchEnergy {
        /// Watch name.
        name: String,
        /// Object ID.
        object_id: String,
        /// Alert when energy drops below this percentage.
        threshold: f64,
        /// Poll interval in seconds.
        #[arg(long, default_value = "120")]
        interval: u64,
        /// Auto-refresh energy amount (0 = notify only).
        #[arg(long, default_value = "0")]
        auto_refresh: u64,
    },
    /// Add a bridge status watch.
    WatchBridge {
        /// Watch name.
        name: String,
        /// Transfer ID.
        transfer_id: String,
        /// Poll interval in seconds.
        #[arg(long, default_value = "30")]
        interval: u64,
    },
    /// Remove a watch.
    Remove { name: String },
    /// Enable a watch.
    Enable { name: String },
    /// Disable a watch.
    Disable { name: String },
    /// Show watch details.
    Show { name: String },
    /// Show recent alerts.
    Alerts {
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Clear alert history.
    ClearAlerts,
    /// Show watchtower status.
    Status,
}

// Note: named AuditAction2 to avoid conflict with existing AuditAction in other context
#[derive(Subcommand)]
pub enum AuditAction2 {
    /// Show recent audit entries.
    Recent {
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Verify chain integrity.
    Verify,
    /// Search audit log.
    Search { query: String },
    /// Filter by severity (info, warning, critical).
    Filter { severity: String },
    /// Export audit log to CSV.
    Export {
        /// Output file.
        file: std::path::PathBuf,
    },
    /// Show audit stats.
    Stats,
}

#[derive(Subcommand)]
pub enum TaxAction {
    /// Record a token acquisition.
    Acquire {
        /// Amount of tokens.
        amount: u64,
        /// Cost per unit.
        cost: f64,
        /// Source (faucet, purchase, reward, etc.).
        #[arg(long, default_value = "purchase")]
        source: String,
        /// Reference.
        #[arg(long, default_value = "manual")]
        reference: String,
    },
    /// Record a disposal (sell/send/spend).
    Dispose {
        /// Amount to dispose.
        amount: u64,
        /// Proceeds per unit.
        proceeds: f64,
        /// Disposal type (sell, send, spend).
        #[arg(long, default_value = "sell")]
        disposal_type: String,
        /// Reference.
        #[arg(long, default_value = "manual")]
        reference: String,
    },
    /// Show open lots (unrealized holdings).
    Lots,
    /// Show disposal history.
    Disposals,
    /// Generate annual summary.
    Summary {
        /// Tax year.
        year: u32,
    },
    /// Set cost basis method (fifo, lifo, hifo).
    SetMethod { method: String },
    /// Record an energy cost (deductible expense).
    EnergyCost {
        amount: f64,
        description: String,
        #[arg(long, default_value = "manual")]
        reference: String,
    },
    /// Export disposals to CSV.
    ExportCsv {
        /// Output file.
        file: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
pub enum PolicyAction {
    /// List all policies.
    List,
    /// Add a max-amount policy.
    AddMaxAmount {
        /// Policy name.
        name: String,
        /// Max amount.
        max: u64,
        /// Enforcement (block, warn, log).
        #[arg(long, default_value = "block")]
        enforcement: String,
    },
    /// Add a blocked-recipients policy.
    AddBlocklist {
        /// Policy name.
        name: String,
        /// Blocked addresses (comma-separated).
        addresses: String,
        /// Enforcement.
        #[arg(long, default_value = "block")]
        enforcement: String,
    },
    /// Add a time restriction policy.
    AddTimelock {
        /// Policy name.
        name: String,
        /// Deny after hour (0-23).
        deny_after: u8,
        /// Deny before hour (0-23).
        deny_before: u8,
        /// Enforcement.
        #[arg(long, default_value = "block")]
        enforcement: String,
    },
    /// Show policy details.
    Show { name: String },
    /// Remove a policy.
    Remove { name: String },
    /// Enable a policy.
    Enable { name: String },
    /// Disable a policy.
    Disable { name: String },
    /// Test a transaction against policies.
    Test {
        /// Recipient.
        to: String,
        /// Amount.
        amount: u64,
    },
}

#[derive(Subcommand)]
pub enum ExportAction {
    /// Export transaction history.
    History {
        /// Output file (.csv, .json, .txt).
        file: std::path::PathBuf,
    },
    /// Export account summary.
    Summary {
        /// Output file.
        file: std::path::PathBuf,
    },
    /// Export full wallet state dump (JSON).
    Dump {
        /// Output file.
        file: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
pub enum ScriptAction {
    /// List saved scripts.
    List,
    /// Show a script's steps.
    Show { name: String },
    /// Create a new script from JSON file.
    Load { file: std::path::PathBuf },
    /// Run a script (dry-run by default).
    Run {
        name: String,
        /// Actually execute (not dry-run).
        #[arg(long)]
        live: bool,
    },
    /// Delete a script.
    Delete { name: String },
}

#[derive(Subcommand)]
pub enum MetricsAction {
    /// Show all metrics.
    Show,
    /// Export metrics in Prometheus format.
    Prometheus,
    /// Export metrics as JSON.
    Json,
    /// Reset all metric values.
    Reset,
    /// Register default wallet metrics.
    Init,
}

#[derive(Subcommand)]
pub enum MigrateAction {
    /// Detect format of input (mnemonic, key, or file).
    Detect { input: String },
    /// Plan a migration (dry run).
    Plan { input: String },
    /// Show migration history.
    History,
    /// Show supported source formats.
    Formats,
    /// Validate a mnemonic phrase.
    ValidateMnemonic { phrase: String },
    /// Validate a private key.
    ValidateKey { key: String },
}

#[derive(Subcommand)]
pub enum QrAction {
    /// Generate QR code for an address.
    Address { address: String },
    /// Generate QR code for a payment request.
    Pay {
        address: String,
        #[arg(long)]
        amount: Option<u64>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        message: Option<String>,
    },
    /// Generate QR code for arbitrary data.
    Encode { data: String },
    /// Generate QR as SVG file.
    Svg {
        data: String,
        /// Output file path.
        file: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
pub enum BenchAction {
    /// Run all wallet benchmarks.
    Run {
        /// Use quick mode (fewer iterations).
        #[arg(long)]
        quick: bool,
    },
    /// Show last benchmark results.
    Show,
    /// Check for performance regressions.
    Regressions,
}

#[derive(Subcommand)]
pub enum HealthAction {
    /// Run all health checks.
    Check,
    /// Show only warnings and critical issues.
    Issues,
    /// Quick pass/fail check.
    Quick,
}

#[derive(Subcommand)]
pub enum PluginAction2 {
    /// List installed plugins.
    List,
    /// Show plugin details.
    Show { name: String },
    /// Install a plugin from JSON manifest.
    Install { file: std::path::PathBuf },
    /// Uninstall a plugin.
    Uninstall { name: String },
    /// Enable a plugin.
    Enable { name: String },
    /// Disable a plugin.
    Disable { name: String },
    /// Audit plugin permissions.
    Audit,
}

#[derive(Subcommand)]
pub enum ScheduleAction {
    /// List all scheduled jobs.
    List,
    /// Show a specific job.
    Show { id: String },
    /// Add a new scheduled job.
    Add {
        /// Job ID.
        id: String,
        /// Job name.
        name: String,
        /// Schedule interval (e.g. 30s, 5m, 2h, 1d, 1w).
        interval: String,
        /// Action type: log, backup, energy_scan.
        action: String,
    },
    /// Remove a scheduled job.
    Remove { id: String },
    /// Enable a job.
    Enable { id: String },
    /// Disable a job.
    Disable { id: String },
    /// Show scheduler statistics.
    Stats,
}

#[derive(Subcommand)]
pub enum AllowlistAction {
    /// Add an address to allow list.
    Allow { address: String, #[arg(long, default_value = "")] note: String },
    /// Add an address to deny list.
    Deny { address: String, #[arg(long, default_value = "")] note: String },
    /// Remove an address from lists.
    Remove { address: String },
    /// Check if an address is allowed/denied.
    Check { address: String },
    /// List all entries.
    List,
    /// Export as CSV.
    Export,
    /// Import from CSV file.
    Import { file: std::path::PathBuf },
    /// Purge expired entries.
    Purge,
}

#[derive(Subcommand)]
pub enum TimelockAction {
    /// Create a time-locked transfer.
    Create {
        recipient: String,
        amount: u64,
        /// Unlock date (RFC3339).
        unlock_at: String,
        #[arg(long)]
        cancellable: bool,
    },
    /// List all timelocks.
    List,
    /// Show a timelock.
    Show { id: String },
    /// Claim an unlocked timelock.
    Claim { id: String },
    /// Cancel a cancellable timelock.
    Cancel { id: String },
    /// Create a vesting schedule.
    Vest {
        beneficiary: String,
        amount: u64,
        start: String,
        cliff: String,
        end: String,
    },
    /// List vesting schedules.
    Vestings,
}

#[derive(Subcommand)]
pub enum MemoAction2 {
    /// Attach a public memo.
    Add {
        recipient: String,
        content: String,
        #[arg(long)]
        tx_hash: Option<String>,
    },
    /// List memos.
    List,
    /// Search memos.
    Search { query: String },
    /// Show a memo.
    Show { id: String },
    /// Delete a memo.
    Delete { id: String },
}

#[derive(Subcommand)]
pub enum RecoveryAction2 {
    /// Set up dead man's switch.
    DeadmanSetup {
        beneficiary: String,
        /// Check-in interval in days.
        interval_days: u32,
    },
    /// Check in (reset deadline).
    DeadmanCheckin,
    /// Show dead man's switch status.
    DeadmanStatus,
    /// Disable dead man's switch.
    DeadmanDisable,
    /// Set up social recovery.
    SocialSetup {
        /// Approval threshold.
        threshold: usize,
        /// Recovery delay in hours.
        #[arg(long, default_value = "24")]
        delay_hours: u32,
    },
    /// Add a guardian.
    AddGuardian { address: String, name: String },
    /// Remove a guardian.
    RemoveGuardian { address: String },
    /// List guardians.
    Guardians,
    /// Show recovery status.
    Status,
}

#[derive(Subcommand)]
pub enum DelegationAction {
    /// Create a new delegation.
    Create {
        delegate: String,
        cap: u64,
        #[arg(long, default_value = "transfer")]
        delegation_type: String,
        #[arg(long)]
        per_tx_limit: Option<u64>,
    },
    /// List all delegations.
    List,
    /// Show a delegation.
    Show { id: String },
    /// Revoke a delegation.
    Revoke { id: String },
    /// Revoke all delegations.
    RevokeAll,
}

#[derive(Subcommand)]
pub enum SyncAction {
    /// Export address book for sync.
    Export { file: std::path::PathBuf },
    /// Import and merge from another device.
    Import { file: std::path::PathBuf },
    /// Show sync status.
    Status,
}

#[derive(Subcommand)]
pub enum GasStationAction {
    /// List relays.
    Relays,
    /// Add a relay.
    AddRelay { url: String, name: String },
    /// Remove a relay.
    RemoveRelay { url: String },
    /// Show best relay.
    BestRelay,
    /// List sponsors.
    Sponsors,
    /// Add a gas sponsor.
    AddSponsor { address: String, name: String, budget: u64 },
    /// Show gas station stats.
    Stats,
}

#[derive(Subcommand)]
pub enum IntentAction {
    /// Submit a new intent.
    Submit {
        description: String,
        #[arg(long, default_value = "transfer")]
        intent_type: String,
    },
    /// List intents.
    List,
    /// Show an intent.
    Show { id: String },
    /// Cancel an intent.
    Cancel { id: String },
    /// List registered solvers.
    Solvers,
    /// Show intent stats.
    Stats,
}

#[derive(Subcommand)]
pub enum TokenRegistryAction {
    /// Register a new token.
    Register {
        address: String,
        name: String,
        symbol: String,
        #[arg(long, default_value = "18")]
        decimals: u8,
    },
    /// List all tokens.
    List,
    /// Show token details.
    Show { address: String },
    /// Remove a token.
    Remove { address: String },
    /// Search tokens.
    Search { query: String },
    /// Verify a token.
    Verify { address: String },
    /// Flag a token as scam.
    Flag { address: String, reason: String },
    /// Show registry stats.
    Stats,
}

#[derive(Subcommand)]
pub enum FeeBumpAction {
    /// Track a pending transaction.
    Track { tx_hash: String, sender: String, nonce: u64, fee: u64 },
    /// List tracked transactions.
    List,
    /// Show a tracked transaction.
    Show { tx_hash: String },
    /// Detect stuck transactions.
    DetectStuck,
    /// Bump fee on a transaction.
    Bump { tx_hash: String },
    /// Bump all stuck transactions.
    BumpAll,
    /// Remove confirmed transactions.
    Cleanup,
    /// Show fee bumper stats.
    Stats,
}

#[derive(Subcommand)]
pub enum SnapshotAction {
    /// Capture current wallet state.
    Capture { label: String },
    /// List all snapshots.
    List,
    /// Show a snapshot.
    Show { id: String },
    /// Diff two snapshots.
    Diff { from: String, to: String },
    /// Remove a snapshot.
    Remove { id: String },
    /// Show total snapshot stats.
    Stats,
}

#[derive(Subcommand)]
pub enum WatchOnlyAction {
    /// Watch an address.
    Add { address: String, label: String },
    /// Stop watching an address.
    Remove { address: String },
    /// List watched addresses.
    List,
    /// Show a watched address.
    Show { address: String },
    /// Update balance for an address.
    UpdateBalance { address: String, balance: u64 },
    /// Show unread alerts.
    Alerts,
    /// Mark all alerts as read.
    MarkRead,
    /// Show watch stats.
    Stats,
}

#[derive(Subcommand)]
pub enum PeerAction {
    /// Add a peer node.
    Add { url: String, name: String },
    /// Remove a peer node.
    Remove { url: String },
    /// List all peers.
    List,
    /// Show best peer.
    Best,
    /// Record a success for a peer.
    RecordSuccess { url: String, latency_ms: u64 },
    /// Record a failure for a peer.
    RecordFailure { url: String },
    /// Unban all peers.
    UnbanAll,
    /// Show peer stats.
    Stats,
}

#[derive(Subcommand)]
pub enum MempoolAction {
    /// Add a pending transaction.
    Add { tx_hash: String, sender: String, receiver: String, amount: u64, fee: u64, nonce: u64 },
    /// Remove a transaction.
    Remove { tx_hash: String },
    /// List pending transactions.
    List,
    /// Detect front-running.
    FrontRun { victim: String, attacker: String },
    /// Get recommended fee.
    RecommendFee { priority: String },
    /// Show congestion level.
    Congestion,
    /// Show mempool stats.
    Stats,
}

#[derive(Subcommand)]
pub enum IndexerAction {
    /// Query events with filters.
    Query {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        event_type: Option<String>,
        #[arg(long)]
        min_block: Option<u64>,
        #[arg(long)]
        max_block: Option<u64>,
    },
    /// Show a transaction receipt.
    Receipt { tx_hash: String },
    /// Show latest indexed block.
    Latest,
    /// Show indexer stats.
    Stats,
}

#[derive(Subcommand)]
pub enum NetHealthAction {
    /// Show network health grade.
    Grade,
    /// Show block time stats.
    BlockTimes,
    /// Show reorg history.
    Reorgs,
    /// Show epoch progress.
    Epoch { expected_blocks: u64 },
    /// Show recent events.
    Events { #[arg(default_value = "20")] count: usize },
    /// Show full network stats.
    Stats,
}

#[derive(Subcommand)]
pub enum PriceAction {
    /// Register a token for price tracking.
    Register { token_id: String, symbol: String, price: f64 },
    /// Update a token price.
    Update { token_id: String, price: f64 },
    /// Show token price.
    Show { token_id: String },
    /// List all tracked tokens.
    List,
    /// Set a price alert.
    Alert { token_id: String, #[arg(long)] above: Option<f64>, #[arg(long)] below: Option<f64> },
    /// Check all alerts.
    CheckAlerts,
    /// Valuate portfolio.
    Portfolio,
    /// Show top gainers/losers.
    Movers { #[arg(default_value = "5")] count: usize },
    /// Show feed stats.
    Stats,
}

#[derive(Subcommand)]
pub enum RiskAction {
    /// Score an address.
    Score { address: String },
    /// Show address profile.
    Show { address: String },
    /// Add to blacklist.
    Blacklist { address: String },
    /// Remove from blacklist.
    Unblacklist { address: String },
    /// List risky addresses.
    Risky,
    /// Show scoring stats.
    Stats,
}

#[derive(Subcommand)]
pub enum DecodeAction {
    /// Decode a transaction.
    Tx { tx_hash: String, selector: String, from: String, #[arg(long)] to: Option<String>, #[arg(long, default_value = "0")] value: u64 },
    /// List registered methods.
    Methods,
    /// List known contracts.
    Contracts,
    /// Register a contract name.
    RegisterContract { address: String, name: String },
    /// Show decoder stats.
    Stats,
}

#[derive(Subcommand)]
pub enum RulesAction {
    /// Add a notification rule.
    Add { id: String, name: String },
    /// Remove a rule.
    Remove { id: String },
    /// List all rules.
    List,
    /// Show a rule.
    Show { id: String },
    /// Enable a rule.
    Enable { id: String },
    /// Disable a rule.
    Disable { id: String },
    /// Show rule engine stats.
    Stats,
}

#[derive(Subcommand)]
pub enum AbiAction {
    /// Register a contract ABI.
    Register { address: String, name: String },
    /// Remove a contract ABI.
    Remove { address: String },
    /// List registered contracts.
    List,
    /// Show a contract's ABI.
    Show { address: String },
    /// Search contracts.
    Search { query: String },
    /// Show ABI stats.
    Stats,
}

#[derive(Subcommand)]
pub enum NameAction {
    /// Register a name.
    Register { name: String, owner: String, #[arg(long, default_value = "2099-01-01T00:00:00+00:00")] expires: String },
    /// Resolve a name to address.
    Resolve { name: String },
    /// Reverse resolve address to name.
    Reverse { address: String },
    /// Transfer a name.
    Transfer { name: String, new_owner: String },
    /// List all names.
    List,
    /// Show name service stats.
    Stats,
}

#[derive(Subcommand)]
pub enum PreviewAction {
    /// Preview a transfer.
    Transfer { from: String, to: String, value: u64, #[arg(long, default_value = "100000")] balance: u64 },
    /// Show recent previews.
    Recent { #[arg(default_value = "10")] count: usize },
    /// Show preview stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ConnectAction {
    /// List active sessions.
    Sessions,
    /// Disconnect a session.
    Disconnect { id: String },
    /// List pending requests.
    Pending,
    /// Approve a request.
    Approve { id: String, result: String },
    /// Reject a request.
    Reject { id: String, reason: String },
    /// Cleanup expired sessions.
    Cleanup,
    /// Show connect stats.
    Stats,
}

#[derive(Subcommand)]
pub enum PrivacyAction {
    /// Generate a stealth address.
    Stealth { public_key: String },
    /// Blind an amount.
    Blind { amount: u64 },
    /// Create a mix request.
    Mix { amount: u64, #[arg(long, default_value = "single")] strategy: String },
    /// Score address privacy.
    Score { address: String },
    /// Show privacy stats.
    Stats,
}

#[derive(Subcommand)]
pub enum KeyRotAction {
    /// Add a managed key.
    Add { id: String, key_type: String, public_key: String },
    /// Rotate a key.
    Rotate { key_id: String, new_public_key: String },
    /// List managed keys.
    List,
    /// Show key rotation stats.
    Stats,
}

#[derive(Subcommand)]
pub enum AccessAction2 {
    /// Add a user.
    AddUser { id: String, name: String, #[arg(long, default_value = "viewer")] role: String },
    /// Remove a user.
    RemoveUser { id: String },
    /// Check access for a user.
    Check { user_id: String, action: String },
    /// List users.
    Users,
    /// Show access stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ThreatAction {
    /// Check a URL for phishing.
    CheckUrl { url: String },
    /// Check a contract address.
    CheckContract { address: String },
    /// Report a phishing URL.
    ReportPhishing { url: String },
    /// Report a malicious contract.
    ReportContract { address: String, reason: String },
    /// List active threats.
    Active,
    /// Show threat stats.
    Stats,
}

#[derive(Subcommand)]
pub enum PoolAction {
    /// Add a liquidity pool.
    Add { id: String, token_a: String, token_b: String, #[arg(long, default_value = "constant_product")] pool_type: String, #[arg(long, default_value_t = 30)] fee_bps: u32 },
    /// Add a LP position.
    Deposit { id: String, pool_id: String, amount_a: u64, amount_b: u64, lp_tokens: u64 },
    /// Withdraw a position.
    Withdraw { position_id: String },
    /// Claim rewards for a position.
    Claim { position_id: String },
    /// Estimate swap output.
    Estimate { pool_id: String, amount: u64, #[arg(long)] a_to_b: bool },
    /// List pools sorted by APY.
    List,
    /// Show pool stats.
    Stats,
}

#[derive(Subcommand)]
pub enum FarmAction {
    /// Add a yield farm.
    Add { id: String, name: String, protocol: String, stake_token: String, reward_token: String, #[arg(long, default_value_t = 12.0)] apy: f64 },
    /// Stake into a farm.
    Stake { id: String, farm_id: String, amount: u64, #[arg(long, default_value = "manual")] compound: String },
    /// Unstake a position.
    Unstake { position_id: String },
    /// Harvest rewards.
    Harvest { position_id: String },
    /// Auto-compound a position.
    Compound { position_id: String },
    /// Show top farms by APY.
    Best { #[arg(long, default_value_t = 10)] n: usize },
    /// Show farming stats.
    Stats,
}

#[derive(Subcommand)]
pub enum CrossSwapAction {
    /// Add a swap route.
    AddRoute { id: String, source_chain: String, dest_chain: String, source_token: String, dest_token: String, #[arg(long, default_value_t = 1.0)] rate: f64, #[arg(long, default_value_t = 25)] fee_bps: u32, #[arg(long, default_value = "bridge")] provider: String },
    /// Initiate a cross-chain swap.
    Swap { route_id: String, amount: u64 },
    /// Lock a pending swap.
    Lock { swap_id: String },
    /// Complete a swap.
    Complete { swap_id: String, actual_output: u64, dest_tx: String },
    /// Refund a swap.
    Refund { swap_id: String },
    /// List active swaps.
    Active,
    /// Show swap stats.
    Stats,
}

#[derive(Subcommand)]
pub enum FlashAction2 {
    /// Create a flash loan plan.
    Create { name: String, token: String, amount: u64, #[arg(long, default_value_t = 9)] fee_bps: u32 },
    /// Add borrow action.
    Borrow { plan_id: String, token: String, amount: u64 },
    /// Add swap action.
    Swap { plan_id: String, from: String, to: String, amount: u64 },
    /// Add repay action.
    Repay { plan_id: String, token: String, amount: u64 },
    /// Simulate a plan.
    Simulate { plan_id: String },
    /// Execute a plan.
    Execute { plan_id: String },
    /// Cancel a plan.
    Cancel { plan_id: String },
    /// List all plans.
    List,
    /// Show flash loan stats.
    Stats,
}

#[derive(Subcommand)]
pub enum DcaAction {
    /// Create a DCA plan.
    Create { id: String, name: String, token_from: String, token_to: String, amount: u64, #[arg(long, default_value = "daily")] frequency: String, #[arg(long)] max_buys: Option<u32>, #[arg(long)] budget: Option<u64> },
    /// Pause a plan.
    Pause { id: String },
    /// Resume a plan.
    Resume { id: String },
    /// Cancel a plan.
    Cancel { id: String },
    /// Simulate a buy execution.
    Buy { plan_id: String, price: f64, received: u64 },
    /// List active plans.
    Active,
    /// Show DCA stats.
    Stats,
}

#[derive(Subcommand)]
pub enum LimitAction {
    /// Place a limit order.
    Place { id: String, token_from: String, token_to: String, #[arg(long, default_value = "buy")] side: String, amount: u64, price: f64, #[arg(long)] expires: Option<String> },
    /// Cancel an order.
    Cancel { id: String },
    /// Fill an order (simulate).
    Fill { id: String, amount: u64, price: f64 },
    /// Check trigger prices.
    CheckTriggers { token_from: String, token_to: String, current_price: f64 },
    /// List open orders.
    Open,
    /// Show order stats.
    Stats,
}

#[derive(Subcommand)]
pub enum RebalAction {
    /// Create a portfolio with target allocations (token:pct pairs).
    Create { id: String, name: String, #[arg(long, default_value_t = 5.0)] threshold: f64 },
    /// Set a target allocation.
    SetTarget { portfolio_id: String, token: String, pct: f64 },
    /// Update holdings value.
    SetHolding { portfolio_id: String, token: String, value: u64 },
    /// Check drift and show allocations.
    Check { portfolio_id: String },
    /// Generate and execute a rebalance plan.
    Execute { portfolio_id: String },
    /// List portfolios.
    List,
    /// Show rebalance stats.
    Stats,
}

#[derive(Subcommand)]
pub enum SmartAlertAction {
    /// Create a price alert.
    PriceAbove { id: String, token: String, threshold: f64 },
    /// Create a price-below alert.
    PriceBelow { id: String, token: String, threshold: f64 },
    /// Create a balance-below alert.
    BalanceBelow { id: String, token: String, threshold: u64 },
    /// Acknowledge an alert.
    Ack { id: String },
    /// Dismiss an alert.
    Dismiss { id: String },
    /// List active alerts.
    Active,
    /// Show alert stats.
    Stats,
}

#[derive(Subcommand)]
pub enum RecoveryAction3 {
    /// Add a guardian.
    AddGuardian { id: String, name: String, address: String, public_key: String },
    /// Remove a guardian.
    RemoveGuardian { id: String },
    /// Revoke a guardian.
    RevokeGuardian { id: String },
    /// List active guardians.
    Guardians,
    /// Initiate recovery.
    Initiate { requester: String, new_key: String },
    /// Approve a recovery request.
    Approve { request_id: String, guardian_id: String, signature: String },
    /// Complete a recovery.
    Complete { request_id: String },
    /// Show recovery stats.
    Stats,
}

#[derive(Subcommand)]
pub enum VaultAction {
    /// Create a shared vault.
    Create { id: String, name: String, #[arg(long, default_value_t = 2)] threshold: u32 },
    /// Add a member.
    AddMember { vault_id: String, id: String, name: String, address: String, #[arg(long, default_value = "signer")] role: String },
    /// Remove a member.
    RemoveMember { vault_id: String, member_id: String },
    /// Propose a transfer.
    Propose { vault_id: String, proposer: String, to: String, amount: u64, token: String },
    /// Approve a proposal.
    ApproveProposal { proposal_id: String, member_id: String },
    /// Execute a proposal.
    ExecuteProposal { proposal_id: String },
    /// List vaults.
    List,
    /// Show vault stats.
    Stats,
}

#[derive(Subcommand)]
pub enum StreamAction {
    /// Create a payment stream.
    Create { id: String, name: String, sender: String, recipient: String, token: String, total: u64, rate: u64, #[arg(long, default_value = "salary")] stream_type: String },
    /// Pause a stream.
    Pause { id: String },
    /// Resume a stream.
    Resume { id: String },
    /// Cancel a stream.
    Cancel { id: String },
    /// Withdraw from a stream.
    Withdraw { id: String, amount: u64 },
    /// List active streams.
    Active,
    /// Show stream stats.
    Stats,
}

#[derive(Subcommand)]
pub enum EscrowAction {
    /// Create an escrow.
    Create { id: String, buyer: String, seller: String, token: String, amount: u64, #[arg(long, default_value_t = 100)] fee_bps: u32, description: String },
    /// Fund an escrow.
    Fund { id: String },
    /// Release escrow to seller.
    Release { id: String },
    /// Refund escrow to buyer.
    Refund { id: String },
    /// Dispute an escrow.
    Dispute { id: String, reason: String },
    /// Resolve a dispute.
    Resolve { id: String, #[arg(long, default_value = "seller")] to: String },
    /// List active escrows.
    Active,
    /// Show escrow stats.
    Stats,
}

#[derive(Subcommand)]
pub enum PnlAction {
    /// Record a buy.
    Buy { token: String, amount: u64, price: f64 },
    /// Record a sale.
    Sell { token: String, amount: u64, price: f64, #[arg(long, default_value = "fifo")] method: String },
    /// Show unrealized P&L.
    Unrealized { token: String, current_price: f64 },
    /// Show token P&L summary.
    Token { token: String, current_price: f64 },
    /// Show total realized P&L.
    Realized,
    /// Show P&L stats.
    Stats,
}

#[derive(Subcommand)]
pub enum AnalyticsAction2 {
    /// Record a daily portfolio value.
    Record { token: String, date: String, value: f64 },
    /// Show Sharpe ratio.
    Sharpe { token: String, #[arg(long, default_value_t = 0.02)] risk_free: f64 },
    /// Show max drawdown.
    Drawdown { token: String },
    /// Show diversification score.
    Diversify,
    /// Show full risk metrics.
    Risk { token: String, #[arg(long, default_value_t = 0.02)] risk_free: f64 },
    /// Show analytics stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ComplianceAction {
    /// Add a transaction for compliance tracking.
    AddTx { tx_hash: String, token: String, amount: u64, value_usd: f64, #[arg(long, default_value = "unknown")] category: String },
    /// Flag a transaction.
    Flag { tx_hash: String },
    /// Generate a report.
    Report { #[arg(long, default_value = "annual")] report_type: String, #[arg(long, default_value = "US")] jurisdiction: String },
    /// Mark a report as reviewed.
    Review { report_id: String },
    /// Show compliance stats.
    Stats,
}

#[derive(Subcommand)]
pub enum WhaleAction {
    /// Track a whale address.
    Track { address: String, #[arg(long)] label: Option<String>, balance: u64 },
    /// Untrack a whale.
    Untrack { address: String },
    /// Update whale balance.
    Update { address: String, balance: u64 },
    /// Show top whales.
    Top { #[arg(long, default_value_t = 10)] n: usize },
    /// Show recent whale movements.
    Movements { #[arg(long, default_value_t = 20)] n: usize },
    /// Show whale stats.
    Stats,
}

#[derive(Subcommand)]
pub enum EnergyOptAction {
    /// Track an object for energy optimization.
    Track { id: String, owner: String, energy: u64, max_energy: u64, decay_rate: u64, #[arg(long, default_value_t = 5)] priority: u32 },
    /// Forecast decay for an object.
    Forecast { id: String },
    /// Forecast all tracked objects.
    ForecastAll,
    /// Show critical objects.
    Critical,
    /// Optimize a batch of objects.
    Batch { #[arg(required = true)] ids: Vec<String> },
    /// Auto-create a refresh plan.
    AutoPlan,
    /// Execute a refresh plan.
    Execute { plan_id: String },
    /// Show energy optimizer stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ObjMgrAction {
    /// Add an object.
    Add { id: String, name: String, owner: String, #[arg(long, default_value = "data")] obj_type: String, energy: u64, max_energy: u64 },
    /// Refresh an object's energy.
    Refresh { id: String, energy: u64 },
    /// Transfer ownership.
    Transfer { id: String, new_owner: String },
    /// Freeze an object.
    Freeze { id: String },
    /// Unfreeze an object.
    Unfreeze { id: String },
    /// Plan resurrection of a ghost.
    Resurrect { id: String },
    /// List low-energy objects.
    LowEnergy { #[arg(long, default_value_t = 0.25)] threshold: f64 },
    /// Show object stats.
    Stats,
}

#[derive(Subcommand)]
pub enum DeployAction {
    /// Create a contract deployment.
    Create { id: String, name: String, deployer: String, #[arg(long, default_value = "standard")] contract_type: String },
    /// Compile contract bytecode.
    Compile { id: String, bytecode_hex: String },
    /// Deploy to chain.
    DeployContract { id: String, address: String, tx_hash: String, #[arg(long, default_value_t = 0)] gas: u64 },
    /// Upgrade a deployed contract.
    Upgrade { id: String, new_bytecode_hex: String, version: String, #[arg(long, default_value = "")] notes: String },
    /// List deployed contracts.
    List,
    /// Show deployer stats.
    Stats,
}

#[derive(Subcommand)]
pub enum GovAction {
    /// Add a proposal.
    Propose { id: String, title: String, proposer: String, #[arg(long, default_value_t = 1000)] quorum: u64, #[arg(long, default_value_t = 10000)] voting_power: u64 },
    /// Start voting on a proposal.
    StartVoting { id: String, #[arg(long, default_value = "2099-01-01T00:00:00Z")] end: String },
    /// Cast a vote.
    Vote { proposal_id: String, voter: String, choice: String, #[arg(long, default_value_t = 1)] power: u64 },
    /// Finalize a proposal.
    Finalize { id: String },
    /// Execute a passed proposal.
    ExecuteProposal { id: String },
    /// Delegate voting power.
    Delegate { from: String, to: String, power: u64 },
    /// Show active proposals.
    Active,
    /// Show governance stats.
    Stats,
}

#[derive(Subcommand)]
pub enum FeeOptAction {
    /// Record a fee data point.
    Record { gas_price: u64, #[arg(long, default_value_t = 50.0)] utilization: f64, #[arg(long, default_value_t = 100)] tx_count: u32 },
    /// Get fee estimates for all speeds.
    Estimate,
    /// Market analysis.
    Market,
    /// Find optimal submission windows.
    Windows,
    /// Check if now is good to submit.
    ShouldSubmit { max_gas: u64 },
    /// Show fee stats.
    Stats,
}

#[derive(Subcommand)]
pub enum BatchExecAction {
    /// Create a batch job.
    Create { name: String, #[arg(long, default_value = "stop")] policy: String },
    /// Add a transaction to a batch.
    Add { batch_id: String, description: String, to: String, amount: u64, #[arg(long, default_value = "EVP")] token: String },
    /// Validate a batch.
    Validate { batch_id: String },
    /// Execute a batch.
    Execute { batch_id: String },
    /// Rollback a batch.
    Rollback { batch_id: String },
    /// List pending batches.
    Pending,
    /// Show batch stats.
    Stats,
}

#[derive(Subcommand)]
pub enum MigrateAction2 {
    /// Start migration from another wallet.
    Start { #[arg(long, default_value = "metamask")] source: String },
    /// Import an account.
    Import { job_id: String, original_address: String, new_address: String, #[arg(long, default_value = "hex")] format: String },
    /// Complete a migration.
    Complete { job_id: String },
    /// Show active migrations.
    Active,
    /// Show migration stats.
    Stats,
}

#[derive(Subcommand)]
pub enum DiagAction {
    /// Run all diagnostic checks.
    RunAll,
    /// Run a single check.
    Run { check_id: String },
    /// Attempt auto-repair.
    Repair { check_id: String },
    /// Register default checks.
    Init,
    /// Show latest report.
    Report,
    /// Show diagnostic stats.
    Stats,
}

#[derive(Subcommand)]
pub enum WsAction {
    /// Subscribe to an event type.
    Subscribe {
        /// Subscription ID.
        id: String,
        /// Event type (new_block, pending_tx, confirmed_tx, token_transfer, contract_event, energy_decay, price_update, custom:<name>).
        event_type: String,
        /// WebSocket endpoint URL.
        #[arg(long, default_value = "ws://localhost:3001")]
        endpoint: String,
    },
    /// Unsubscribe.
    Unsubscribe { id: String },
    /// Pause a subscription.
    Pause { id: String },
    /// Resume a paused subscription.
    Resume { id: String },
    /// Reconnect a disconnected subscription.
    Reconnect { id: String },
    /// List all subscriptions.
    List,
    /// Show recent events.
    Events {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// Show subscriber stats.
    Stats,
}

#[derive(Subcommand)]
pub enum EventBusAction {
    /// Register a handler.
    Register {
        /// Handler ID.
        id: String,
        /// Topic filter (supports wildcards e.g. "tx.*").
        topic: String,
        /// Description.
        #[arg(long, default_value = "")]
        desc: String,
    },
    /// Unregister a handler.
    Unregister { id: String },
    /// Enable a handler.
    Enable { id: String },
    /// Disable a handler.
    Disable { id: String },
    /// Publish an event.
    Publish {
        /// Topic.
        topic: String,
        /// Priority (low, normal, high, critical).
        #[arg(long, default_value = "normal")]
        priority: String,
        /// Source module.
        #[arg(long, default_value = "cli")]
        source: String,
    },
    /// Process a pending event.
    Process { event_id: String },
    /// List handlers.
    Handlers,
    /// List pending events.
    Pending,
    /// Show recent logs.
    Logs {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// Show bus stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ReceiptAction {
    /// Store a new receipt.
    Store {
        tx_hash: String,
        from: String,
        to: String,
        amount: u64,
        /// Token (default: EVAP).
        #[arg(long, default_value = "EVAP")]
        token: String,
        /// Tx type (transfer, contract_deploy, contract_call, refresh, stake, unstake, governance, nft_mint, token_transfer, bridge).
        #[arg(long, default_value = "transfer")]
        tx_type: String,
        /// Status (success, failed, pending, dropped).
        #[arg(long, default_value = "pending")]
        status: String,
        #[arg(long, default_value = "0")]
        gas_used: u64,
        #[arg(long, default_value = "0")]
        fee: u64,
    },
    /// Show a receipt.
    Show { tx_hash: String },
    /// Update receipt status.
    Update {
        tx_hash: String,
        /// New status.
        status: String,
        /// Confirmations.
        #[arg(long)]
        confirmations: Option<u32>,
        /// Block number.
        #[arg(long)]
        block: Option<u64>,
    },
    /// Add a note.
    Note { tx_hash: String, note: String },
    /// List receipts for an address.
    ForAddress { address: String },
    /// Search receipts.
    Search { query: String },
    /// Show summary.
    Summary,
    /// Show summary for an address.
    SummaryAddr { address: String },
    /// Show recent receipts.
    Recent {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// List pending receipts.
    Pending,
    /// List failed receipts.
    Failed,
}

#[derive(Subcommand)]
pub enum StateSyncAction {
    /// Track a new account.
    Track {
        account: String,
        /// Sync mode (full, light, checkpoint).
        #[arg(long, default_value = "full")]
        mode: String,
    },
    /// Stop tracking.
    Untrack { account: String },
    /// Sync an account now.
    Sync { account: String },
    /// Set remote block height.
    SetRemote { account: String, block: u64 },
    /// Record a state conflict.
    Conflict {
        account: String,
        field: String,
        local_value: String,
        remote_value: String,
    },
    /// Resolve a conflict.
    Resolve {
        conflict_id: String,
        /// Resolution strategy (prefer_local, prefer_remote, latest).
        strategy: String,
    },
    /// Create a checkpoint.
    Checkpoint {
        block_number: u64,
        block_hash: String,
        state_root: String,
    },
    /// Show accounts behind.
    Behind,
    /// Show accounts in error.
    Errors,
    /// Show sync stats.
    Stats,
}

#[derive(Subcommand)]
pub enum DebugAction {
    /// Create a new debug session.
    Create { name: String },
    /// End a session.
    End { id: String },
    /// Pause a session.
    Pause { id: String },
    /// Resume a session.
    Resume { id: String },
    /// Add a breakpoint.
    Break {
        id: String,
        /// Type: event_match, balance_threshold, block_number, tx_hash, gas_above, custom:<value>.
        bp_type: String,
        /// Condition string.
        condition: String,
    },
    /// Remove a breakpoint.
    RemoveBreak { id: String },
    /// Enable a breakpoint.
    EnableBreak { id: String },
    /// Disable a breakpoint.
    DisableBreak { id: String },
    /// List active sessions.
    Sessions,
    /// List enabled breakpoints.
    Breakpoints,
    /// Show recent logs.
    Logs {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// Show recent replays.
    Replays {
        #[arg(default_value = "10")]
        count: usize,
    },
    /// Show debug stats.
    Stats,
}

#[derive(Subcommand)]
pub enum GasProfileAction {
    /// Create a gas profile.
    Create {
        id: String,
        /// Op type: transfer, contract_call, contract_deploy, refresh, stake, unstake, nft_mint, token_transfer, bridge.
        op_type: String,
    },
    /// Remove a profile.
    Remove { id: String },
    /// Add a gas sample to a profile.
    Sample {
        profile_id: String,
        tx_hash: String,
        gas_used: u64,
        gas_limit: u64,
        #[arg(long, default_value = "1")]
        gas_price: u64,
        #[arg(long, default_value = "0")]
        block: u64,
    },
    /// Show profile details.
    Show { id: String },
    /// Detect gas hotspots.
    Hotspots,
    /// Generate optimization suggestions.
    Suggest,
    /// Show profiler stats.
    Stats,
}

#[derive(Subcommand)]
pub enum VerifyAction {
    /// Register contract source code.
    Register {
        address: String,
        /// Source code (or path placeholder).
        source: String,
        /// Compiler version: v1, v2, v3.
        #[arg(long, default_value = "v1")]
        compiler: String,
    },
    /// Unregister contract source.
    Unregister { address: String },
    /// Update source code.
    Update { address: String, source: String },
    /// Verify deployed bytecode against registered source.
    Check {
        address: String,
        /// Deployed bytecode (hex string).
        bytecode: String,
    },
    /// Show verification report.
    Report { address: String },
    /// List verified contracts.
    Verified,
    /// List unverified contracts.
    Unverified,
    /// Search contracts.
    Search { query: String },
    /// Show verifier stats.
    Stats,
}

#[derive(Subcommand)]
pub enum SimAction {
    /// Create a state fork.
    Fork {
        id: String,
        /// Block number.
        block: u64,
        /// Source: latest, block:<n>, snapshot:<id>.
        #[arg(long, default_value = "latest")]
        source: String,
    },
    /// Remove a fork.
    RemoveFork { id: String },
    /// Simulate a transaction.
    Run {
        from: String,
        to: String,
        amount: u64,
        #[arg(long, default_value = "100000")]
        gas_limit: u64,
    },
    /// Create a what-if scenario.
    Scenario {
        id: String,
        name: String,
        fork_id: String,
    },
    /// Run all txs in a scenario.
    RunScenario { id: String },
    /// Show simulation result.
    Show { id: String },
    /// Analyze reverts.
    Revert { id: String },
    /// Show recent simulations.
    Recent {
        #[arg(default_value = "10")]
        count: usize,
    },
    /// Show simulator stats.
    Stats,
}

#[derive(Subcommand)]
pub enum AuditTrailAction {
    /// Record an audit event.
    Record {
        /// Action: key_generated, key_imported, key_deleted, tx_signed, tx_submitted, tx_confirmed, setting_changed, login_attempt, backup_created, backup_restored, permission_granted, permission_revoked.
        action_type: String,
        /// Severity: info, warning, critical.
        #[arg(long, default_value = "info")]
        severity: String,
        /// Actor.
        actor: String,
        /// Target.
        target: String,
    },
    /// Verify chain integrity.
    Verify,
    /// Show recent entries.
    Recent {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// Show critical entries.
    Critical,
    /// Search entries.
    Search { query: String },
    /// Export all entries.
    Export,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum AnomalyAction {
    /// Add a detection rule.
    AddRule {
        id: String,
        /// Type: unusual_amount, high_velocity, new_recipient, large_gas, off_hours, rapid_sequence, dust_attack.
        anomaly_type: String,
        /// Threshold value.
        threshold: f64,
        /// Description.
        #[arg(long, default_value = "")]
        desc: String,
    },
    /// Remove a rule.
    RemoveRule { id: String },
    /// Enable a rule.
    EnableRule { id: String },
    /// Disable a rule.
    DisableRule { id: String },
    /// Show behavior profile for an address.
    Profile { address: String },
    /// Show unacknowledged alerts.
    Alerts,
    /// Acknowledge an alert.
    Ack { alert_id: String },
    /// Show risk score for an address.
    Risk { address: String },
    /// Show recent alerts.
    Recent {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum EnclaveAction {
    /// Store a key in the enclave.
    Store {
        id: String,
        /// Key material (or reference).
        material: String,
        /// Purpose: signing, encryption, authentication, derivation.
        #[arg(long, default_value = "signing")]
        purpose: String,
        /// Expiration (RFC3339).
        #[arg(long)]
        expires: Option<String>,
    },
    /// Remove a key.
    Remove { id: String },
    /// Lock a key.
    Lock { id: String },
    /// Unlock a key.
    Unlock { id: String },
    /// Wipe a key.
    Wipe { id: String },
    /// Seal the enclave.
    Seal,
    /// Unseal the enclave.
    Unseal,
    /// Verify key integrity.
    VerifyKey {
        id: String,
        /// Original material to verify against.
        material: String,
    },
    /// List active keys.
    Keys,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum PermAction {
    /// Grant a permission to a dApp.
    Grant {
        /// Permission ID.
        id: String,
        /// dApp ID.
        dapp: String,
        /// Type: read_balance, sign_transaction, send_tokens, deploy_contract, manage_nft, access_private_key, connect_dapp.
        perm_type: String,
        /// Max uses (0 = unlimited).
        #[arg(long, default_value = "0")]
        max_uses: u64,
        /// Expiration (RFC3339).
        #[arg(long)]
        expires: Option<String>,
    },
    /// Revoke a permission.
    Revoke { id: String },
    /// Deny a permission.
    Deny { id: String },
    /// Set spend limit for a dApp.
    Limit {
        dapp: String,
        /// Token.
        #[arg(long, default_value = "EVAP")]
        token: String,
        /// Max per tx.
        max_per_tx: u64,
        /// Max daily.
        max_daily: u64,
    },
    /// Check spend.
    CheckSpend { dapp: String, amount: u64 },
    /// Record a spend.
    Spend { dapp: String, amount: u64 },
    /// Reset daily spend.
    ResetSpend { dapp: String },
    /// Show permissions for a dApp.
    ForDapp { dapp: String },
    /// Show pending approval requests.
    Pending,
    /// Show granted permissions.
    Granted,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ThemeAction {
    /// Register built-in themes.
    Init,
    /// Add a custom theme.
    Add {
        id: String,
        name: String,
        /// Color scheme: light, dark, high_contrast.
        #[arg(long, default_value = "light")]
        scheme: String,
        /// Layout: compact, standard, detailed, minimal.
        #[arg(long, default_value = "standard")]
        layout: String,
    },
    /// Remove a custom theme.
    Remove { id: String },
    /// Set active theme.
    Use { id: String },
    /// Show active theme.
    Active,
    /// List all themes.
    List,
    /// Duplicate a theme.
    Duplicate { id: String, new_id: String, new_name: String },
    /// Set a custom variable on a theme.
    SetVar { theme_id: String, key: String, value: String },
    /// Export a theme as JSON.
    Export { id: String },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum PaletteAction {
    /// Register a command.
    Register {
        id: String,
        name: String,
        /// Description.
        #[arg(long, default_value = "")]
        desc: String,
        /// Category: account, transaction, energy, staking, defi, security, network, utility.
        #[arg(long, default_value = "utility")]
        category: String,
        /// Usage string.
        #[arg(long, default_value = "")]
        usage: String,
    },
    /// Remove a command.
    Remove { id: String },
    /// Add an alias.
    Alias { alias: String, command_id: String },
    /// Remove an alias.
    Unalias { alias: String },
    /// Fuzzy search commands.
    Search { query: String },
    /// Show most used commands.
    TopUsed {
        #[arg(default_value = "10")]
        count: usize,
    },
    /// Show recent commands.
    Recent {
        #[arg(default_value = "10")]
        count: usize,
    },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum OnboardAction {
    /// Create and start an onboarding flow.
    Start {
        /// Flow type: new_user, import, developer, quick.
        #[arg(default_value = "new_user")]
        flow_type: String,
    },
    /// Complete a step.
    Complete { step_id: String },
    /// Skip a step.
    Skip { step_id: String },
    /// Show current step.
    Current,
    /// Show progress.
    Progress,
    /// Reset the active flow.
    Reset,
    /// Show tips.
    Tips,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum HelpAction {
    /// Search help topics.
    Search { query: String },
    /// View a topic.
    View { id: String },
    /// List topics by category.
    Category {
        /// Category: getting_started, accounts, transactions, energy, security, defi, advanced, troubleshooting.
        cat: String,
    },
    /// Search FAQ.
    Faq { query: String },
    /// List tutorials.
    Tutorials {
        /// Difficulty: beginner, intermediate, advanced.
        #[arg(long)]
        difficulty: Option<String>,
    },
    /// Explain an error code.
    Explain { code: String },
    /// Show most popular topics.
    Popular {
        #[arg(default_value = "10")]
        count: usize,
    },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum BreakerAction {
    /// Register a circuit breaker.
    Register {
        id: String,
        service: String,
        /// Failure threshold.
        #[arg(long, default_value = "5")]
        threshold: u32,
        /// Timeout seconds before half-open.
        #[arg(long, default_value = "30")]
        timeout: u64,
    },
    /// Unregister a circuit.
    Unregister { id: String },
    /// Record a success.
    Success { id: String },
    /// Record a failure.
    Failure { id: String },
    /// Check if circuit allows execution.
    Check { id: String },
    /// Force open a circuit.
    ForceOpen { id: String },
    /// Force close a circuit.
    ForceClose { id: String },
    /// List open circuits.
    Open,
    /// Show recent events.
    Events {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum CacheAction {
    /// Create a cache.
    Create {
        id: String,
        name: String,
        /// Layer: memory, disk, remote.
        #[arg(long, default_value = "memory")]
        layer: String,
        /// Max entries.
        #[arg(long, default_value = "1000")]
        max_entries: usize,
        /// Eviction: lru, lfu, fifo, ttl.
        #[arg(long, default_value = "lru")]
        eviction: String,
    },
    /// Remove a cache.
    Remove { id: String },
    /// Put a value.
    Put {
        cache_id: String,
        key: String,
        value: String,
        /// TTL in seconds.
        #[arg(long)]
        ttl: Option<u64>,
    },
    /// Get a value.
    Get { cache_id: String, key: String },
    /// Invalidate a key.
    Invalidate { cache_id: String, key: String },
    /// Clear a cache.
    Clear { cache_id: String },
    /// Evict expired entries.
    EvictExpired,
    /// Show cache size.
    Size { cache_id: String },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ConfigValAction {
    /// Validate a config against a schema.
    Validate {
        schema_id: String,
        /// Config as JSON string.
        config_json: String,
    },
    /// List schemas.
    Schemas,
    /// Show pending migrations.
    Pending,
    /// Show applied migrations.
    Applied,
    /// Show latest backup.
    Backup,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum TaskQueueAction {
    /// Enqueue a task.
    Enqueue {
        id: String,
        /// Type: tx_submit, balance_refresh, energy_check, backup, sync, notification.
        task_type: String,
        /// Priority: critical, high, normal, low.
        #[arg(long, default_value = "normal")]
        priority: String,
        /// Max retries.
        #[arg(long, default_value = "3")]
        max_retries: u32,
    },
    /// Dequeue next task.
    Dequeue,
    /// Complete a task.
    Complete { id: String, result: String },
    /// Fail a task.
    Fail { id: String, error: String },
    /// Cancel a task.
    Cancel { id: String },
    /// Update progress.
    Progress { id: String, pct: f64 },
    /// Retry a dead letter task.
    Retry { id: String },
    /// Show running tasks.
    Running,
    /// Show dead letter queue.
    DeadLetter,
    /// Purge completed tasks.
    Purge,
    /// Show queue depth.
    Depth,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ChangelogAction {
    /// Add a changelog entry.
    Add {
        id: String,
        /// Type: added, changed, fixed, removed, security, deprecated, performance.
        change_type: String,
        /// Scope: wallet, transaction, energy, staking, defi, security, ui, internal.
        scope: String,
        /// Description.
        description: String,
        /// Author.
        #[arg(long, default_value = "dev")]
        author: String,
        /// Breaking change.
        #[arg(long)]
        breaking: bool,
    },
    /// Remove an entry.
    Remove { id: String },
    /// Tag a version.
    Tag {
        version: String,
        name: String,
        /// Comma-separated entry IDs.
        #[arg(long, default_value = "")]
        entries: String,
        /// Release notes.
        #[arg(long)]
        notes: Option<String>,
    },
    /// Show unversioned entries.
    Unversioned,
    /// Show breaking changes.
    Breaking,
    /// Generate markdown.
    Markdown,
    /// Search entries.
    Search { query: String },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum FlagAction {
    /// Register a feature flag.
    Register {
        id: String,
        name: String,
        /// Category: core, experimental, beta, deprecated.
        #[arg(long, default_value = "experimental")]
        category: String,
        /// Description.
        #[arg(long, default_value = "")]
        desc: String,
    },
    /// Remove a flag.
    Remove { id: String },
    /// Enable a flag.
    Enable { id: String },
    /// Disable a flag.
    Disable { id: String },
    /// Set rollout percentage.
    Rollout { id: String, pct: u8 },
    /// Kill switch (emergency disable).
    Kill { id: String },
    /// Check if flag is enabled for a user.
    Check { flag_id: String, user_id: String },
    /// Add user override.
    Override {
        flag_id: String,
        user_id: String,
        #[arg(long)]
        enabled: bool,
    },
    /// List enabled flags.
    Enabled,
    /// List killed flags.
    Killed,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum TelemetryAction {
    /// Opt in to telemetry.
    OptIn {
        /// Level: basic, detailed, full.
        #[arg(default_value = "basic")]
        level: String,
    },
    /// Opt out of telemetry.
    OptOut,
    /// Show status.
    Status,
    /// Show top commands.
    TopCommands {
        #[arg(default_value = "10")]
        count: usize,
    },
    /// Show recent events.
    Events {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// Flush collected events.
    Flush,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum ApiAction {
    /// Register an API endpoint.
    Register {
        id: String,
        path: String,
        /// Method: get, post, put, delete.
        #[arg(long, default_value = "get")]
        method: String,
        /// Version: v1, v2.
        #[arg(long, default_value = "v1")]
        version: String,
        /// Description.
        #[arg(long, default_value = "")]
        desc: String,
    },
    /// Remove an endpoint.
    Remove { id: String },
    /// Create an API key.
    CreateKey {
        key: String,
        name: String,
        /// Comma-separated permissions.
        #[arg(long, default_value = "read")]
        permissions: String,
    },
    /// Revoke an API key.
    RevokeKey { key: String },
    /// List active API keys.
    Keys,
    /// List endpoints.
    Endpoints,
    /// Search endpoints.
    Search { query: String },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum HarnessAction {
    /// Add a test fixture.
    AddFixture {
        id: String,
        name: String,
        /// Type: account, transaction, block, token, nft, contract.
        #[arg(long, default_value = "account")]
        fixture_type: String,
    },
    /// Remove a fixture.
    RemoveFixture { id: String },
    /// Create a mock.
    CreateMock {
        id: String,
        name: String,
        /// Return value for the mock.
        #[arg(long, default_value = "ok")]
        value: String,
    },
    /// Invoke a mock.
    InvokeMock { id: String },
    /// List fixtures.
    Fixtures,
    /// List mocks.
    Mocks,
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum FuzzAction {
    /// Add a fuzz target.
    AddTarget {
        id: String,
        name: String,
        #[arg(long, default_value = "")]
        desc: String,
    },
    /// Remove a target.
    RemoveTarget { id: String },
    /// Run a fuzz campaign.
    Campaign {
        target_id: String,
        #[arg(default_value = "100")]
        runs: u64,
    },
    /// Show failing runs.
    Failures,
    /// Show recent runs.
    Recent {
        #[arg(default_value = "20")]
        count: usize,
    },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum RegressionAction {
    /// Report a known issue.
    Report {
        id: String,
        title: String,
        /// Severity: critical, high, medium, low.
        #[arg(long, default_value = "medium")]
        severity: String,
        /// Module name.
        #[arg(long, default_value = "general")]
        module: String,
        #[arg(long, default_value = "")]
        desc: String,
    },
    /// Close an issue.
    Close {
        id: String,
        #[arg(long)]
        version: Option<String>,
    },
    /// Reopen an issue.
    Reopen { id: String },
    /// Show open issues.
    Open,
    /// Show regressed issues.
    Regressed,
    /// Search issues.
    Search { query: String },
    /// Show stats.
    Stats,
}

#[derive(Subcommand)]
pub enum CoverageAction {
    /// Add a module coverage record.
    Add {
        id: String,
        module_name: String,
        total_lines: u32,
        covered_lines: u32,
        #[arg(long, default_value = "0")]
        total_functions: u32,
        #[arg(long, default_value = "0")]
        covered_functions: u32,
        #[arg(long, default_value = "0")]
        total_branches: u32,
        #[arg(long, default_value = "0")]
        covered_branches: u32,
    },
    /// Update module coverage.
    Update {
        id: String,
        covered_lines: u32,
        #[arg(long, default_value = "0")]
        covered_functions: u32,
        #[arg(long, default_value = "0")]
        covered_branches: u32,
    },
    /// Generate a coverage report.
    Report {
        #[arg(default_value = "latest")]
        name: String,
    },
    /// Show modules below threshold.
    Below {
        #[arg(default_value = "75")]
        threshold: f64,
    },
    /// Show overall coverage.
    Overall,
    /// Show stats.
    Stats,
}

// ──────────────────────────── CLI Runner ───────────────────────────────

/// Execute the CLI command.
pub async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    // Set output mode
    crate::output::set_json_mode(cli.json);

    // Load persistent config, apply CLI overrides
    let config_path = WalletConfig::default_path();
    let config = WalletConfig::load_or_default(&config_path)?;

    let node_url = if cli.node != "http://localhost:3000" {
        &cli.node
    } else {
        &config.node_url
    };
    let keystore_path = if cli.keystore != "~/.evaporchain/keystore.json" {
        expand_path(&cli.keystore)
    } else {
        config.keystore_path.clone()
    };
    let contacts_path = config.contacts_path.clone();
    let history_path = config.history_path.clone();

    let rpc = RpcClient::new(node_url)?;

    match cli.command {
        Commands::Version => {
            let info = crate::build_info::VersionInfo::current();
            crate::output::json_or(&info, || {
                println!("{}", crate::build_info::full_version());
                println!("  Compiler:   {}", crate::build_info::RUSTC_VERSION);
                println!("  Signatures: ML-DSA-65 (FIPS 204)");
                println!("  Encryption: AES-256-GCM + Argon2id");
                println!("  Hashing:    BLAKE3");
            });
            return Ok(());
        }
        Commands::Doctor => {
            cmd_doctor(node_url, &keystore_path).await?;
            return Ok(());
        }
        Commands::Batch { file, dry_run } => {
            cmd_batch(rpc, &keystore_path, &history_path, &file, dry_run).await?;
            return Ok(());
        }
        Commands::Dashboard => {
            cmd_dashboard(rpc, &keystore_path).await?;
            return Ok(());
        }
        Commands::Watch { address } => {
            cmd_watch(rpc, &address).await?;
            return Ok(());
        }
        Commands::Interactive => {
            cmd_interactive(rpc, &keystore_path, &contacts_path, &history_path).await?;
            return Ok(());
        }
        Commands::Completions { shell } => {
            cmd_completions(&shell)?;
            return Ok(());
        }
        Commands::Status => cmd_status(rpc).await?,
        Commands::Blocks { limit } => cmd_blocks(rpc, limit).await?,
        Commands::Tx { hash } => cmd_tx(rpc, &hash).await?,
        Commands::Account { action } => cmd_account(action, rpc, &keystore_path).await?,
        Commands::Send { to, amount, wait } => {
            cmd_send(rpc, &keystore_path, &contacts_path, &history_path, &to, amount, wait).await?
        }
        Commands::Faucet => cmd_faucet(rpc, &keystore_path).await?,
        Commands::Objects => cmd_objects(rpc).await?,
        Commands::Object { id } => cmd_object(rpc, &id).await?,
        Commands::Refresh { id, energy, wait } => {
            cmd_refresh(rpc, &keystore_path, &history_path, &id, energy, wait).await?
        }
        Commands::Nft { action } => cmd_nft(action, rpc, &keystore_path, &history_path).await?,
        Commands::Token { action } => cmd_token(action, rpc, &keystore_path, &history_path).await?,
        Commands::Energy { action } => cmd_energy(action, rpc, &keystore_path).await?,
        Commands::Ghost { action } => cmd_ghost(action, rpc).await?,
        Commands::Stake { action } => cmd_stake(action, rpc, &keystore_path, &history_path).await?,
        Commands::Dao { action } => cmd_dao(action, rpc).await?,
        Commands::Backup { action } => cmd_backup(action, &keystore_path)?,
        Commands::Seed { action } => cmd_seed(action, &keystore_path)?,
        Commands::Contacts { action } => cmd_contacts(action, &contacts_path)?,
        Commands::History { action } => cmd_history(action, &history_path)?,
        Commands::Gas { action } => cmd_gas(action, rpc).await?,
        Commands::Config { action } => cmd_config(action, &config_path)?,
        Commands::Offline { action } => cmd_offline(action, rpc, &keystore_path).await?,
        Commands::Simulate { action } => cmd_simulate(action, rpc, &keystore_path, &contacts_path).await?,
        Commands::Spending { action } => cmd_spending(action)?,
        Commands::Multisig { action } => cmd_multisig(action, &keystore_path)?,
        Commands::Hooks { action } => cmd_hooks(action)?,
        Commands::Labels { action } => cmd_labels(action)?,
        Commands::Fees { action } => cmd_fees(action, rpc).await?,
        Commands::Hardware { action } => cmd_hardware(action)?,
        Commands::Dapp { action } => cmd_dapp(action, &keystore_path)?,
        Commands::Notifications { action } => cmd_notifications(action)?,
        Commands::SessionKeys { action } => cmd_session_keys(action, &keystore_path)?,
        Commands::Bridge { action } => cmd_bridge(action)?,
        Commands::Lang { action } => cmd_lang(action)?,
        Commands::Templates { action } => cmd_templates(action)?,
        Commands::Analytics { action } => cmd_analytics(action)?,
        Commands::Reputation { action } => cmd_reputation(action)?,
        Commands::Watchtower { action } => cmd_watchtower(action)?,
        Commands::Audit { action } => cmd_audit(action)?,
        Commands::Tax { action } => cmd_tax(action)?,
        Commands::Policy { action } => cmd_policy(action)?,
        Commands::Export { action } => cmd_export(action)?,
        Commands::Script { action } => cmd_script(action)?,
        Commands::Metrics { action } => cmd_metrics(action)?,
        Commands::Migrate { action } => cmd_migrate(action)?,
        Commands::Qr { action } => cmd_qr(action)?,
        Commands::Bench { action } => cmd_bench(action)?,
        Commands::Health { action } => cmd_health(action)?,
        Commands::Plugin { action } => cmd_plugin(action)?,
        Commands::Schedule { action } => cmd_schedule(action)?,
        Commands::Allowlist { action } => cmd_allowlist(action)?,
        Commands::Timelock { action } => cmd_timelock(action)?,
        Commands::Memo { action } => cmd_memo(action)?,
        Commands::Recovery { action } => cmd_recovery(action)?,
        Commands::Delegation { action } => cmd_delegation(action)?,
        Commands::Sync { action } => cmd_sync(action)?,
        Commands::GasStation { action } => cmd_gas_station(action)?,
        Commands::Intent { action } => cmd_intent(action)?,
        Commands::TokenRegistry { action } => cmd_token_registry(action)?,
        Commands::FeeBump { action } => cmd_fee_bump(action)?,
        Commands::Snapshot { action } => cmd_snapshot(action)?,
        Commands::WatchOnly { action } => cmd_watchonly(action)?,
        Commands::Peers { action } => cmd_peers(action)?,
        Commands::Mempool { action } => cmd_mempool(action)?,
        Commands::Indexer { action } => cmd_indexer(action)?,
        Commands::NetHealth { action } => cmd_net_health(action)?,
        Commands::Price { action } => cmd_price(action)?,
        Commands::Risk { action } => cmd_risk(action)?,
        Commands::Decode { action } => cmd_decode(action)?,
        Commands::Rules { action } => cmd_rules(action)?,
        Commands::Abi { action } => cmd_abi(action)?,
        Commands::Names { action } => cmd_names(action)?,
        Commands::Preview { action } => cmd_preview(action)?,
        Commands::Connect { action } => cmd_connect(action)?,
        Commands::Privacy { action } => cmd_privacy(action)?,
        Commands::KeyRotation { action } => cmd_key_rotation(action)?,
        Commands::Access { action } => cmd_access(action)?,
        Commands::Threats { action } => cmd_threats(action)?,
        Commands::Pool { action } => cmd_pool(action)?,
        Commands::Farm { action } => cmd_farm(action)?,
        Commands::CrossSwap { action } => cmd_cross_swap(action)?,
        Commands::Flash { action } => cmd_flash(action)?,
        Commands::Dca { action } => cmd_dca(action)?,
        Commands::LimitOrder { action } => cmd_limit_order(action)?,
        Commands::Rebalance { action } => cmd_rebalance(action)?,
        Commands::Alerts { action } => cmd_smart_alerts(action)?,
        Commands::Recovery2 { action } => cmd_social_recovery(action)?,
        Commands::Vault { action } => cmd_vault(action)?,
        Commands::Stream { action } => cmd_stream(action)?,
        Commands::Escrow { action } => cmd_escrow(action)?,
        Commands::Pnl { action } => cmd_pnl(action)?,
        Commands::Analytics2 { action } => cmd_analytics2(action)?,
        Commands::Compliance { action } => cmd_compliance(action)?,
        Commands::Whale { action } => cmd_whale(action)?,
        Commands::EnergyOpt { action } => cmd_energy_opt(action)?,
        Commands::Objects2 { action } => cmd_obj_mgr(action)?,
        Commands::Deploy { action } => cmd_deploy(action)?,
        Commands::Gov { action } => cmd_gov(action)?,
        Commands::FeeOpt { action } => cmd_fee_opt(action)?,
        Commands::BatchExec { action } => cmd_batch_exec(action)?,
        Commands::Migrate2 { action } => cmd_migrate2(action)?,
        Commands::Diag { action } => cmd_diag(action)?,
        Commands::Ws { action } => cmd_ws(action)?,
        Commands::EventBus { action } => cmd_event_bus(action)?,
        Commands::Receipts { action } => cmd_receipts(action)?,
        Commands::StateSync { action } => cmd_state_sync(action)?,
        Commands::Debug { action } => cmd_debug(action)?,
        Commands::GasProfile { action } => cmd_gas_profile(action)?,
        Commands::Verify { action } => cmd_verify(action)?,
        Commands::Simulate2 { action } => cmd_simulate2(action)?,
        Commands::AuditTrail { action } => cmd_audit_trail(action)?,
        Commands::Anomaly { action } => cmd_anomaly(action)?,
        Commands::Enclave { action } => cmd_enclave(action)?,
        Commands::Perms { action } => cmd_perms(action)?,
        Commands::Theme { action } => cmd_theme(action)?,
        Commands::Palette { action } => cmd_palette(action)?,
        Commands::Onboard { action } => cmd_onboard(action)?,
        Commands::HelpTopic { action } => cmd_help(action)?,
        Commands::Breaker { action } => cmd_breaker(action)?,
        Commands::Cache { action } => cmd_cache(action)?,
        Commands::ConfigVal { action } => cmd_config_val(action)?,
        Commands::Tasks { action } => cmd_tasks(action)?,
        Commands::Changelog { action } => cmd_changelog(action)?,
        Commands::Flags { action } => cmd_flags(action)?,
        Commands::Telemetry { action } => cmd_telemetry(action)?,
        Commands::Api { action } => cmd_api(action)?,
        Commands::Harness { action } => cmd_harness(action)?,
        Commands::Fuzz { action } => cmd_fuzz(action)?,
        Commands::Regression { action } => cmd_regression(action)?,
        Commands::Coverage { action } => cmd_coverage(action)?,
    }

    Ok(())
}

// ──────────────────────────── Command Handlers ─────────────────────────

async fn cmd_status(rpc: RpcClient) -> Result<(), Box<dyn std::error::Error>> {
    let status = rpc.get_status().await?;
    crate::output::json_or(&status, || {
        println!("{}", "EvaporChain Status".bold().cyan());
        println!("  Block Height:    {}", status.block_height);
        println!("  Epoch:           {}", status.epoch);
        println!("  Active Objects:  {}", status.active_objects);
        println!("  Ghost Count:     {}", status.ghost_count);
        println!("  Total Evaporated:{}", status.total_evaporated);
        println!("  Peers:           {}", status.peer_count);
        println!("  State Root:      {}", status.state_root);
        println!("  Proving:         {}", status.proving_enabled);
        println!("  Uptime:          {}s", status.uptime_seconds);
    });
    Ok(())
}

async fn cmd_blocks(rpc: RpcClient, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let blocks = rpc.get_blocks(Some(limit)).await?;
    println!("{}", "Recent Blocks".bold().cyan());
    for b in &blocks {
        println!(
            "  #{:<6} epoch={:<6} txs={:<4} gas={:<8} evap={}",
            b.number, b.epoch, b.tx_count, b.gas_used, b.evaporations
        );
    }
    Ok(())
}

async fn cmd_tx(rpc: RpcClient, hash: &str) -> Result<(), Box<dyn std::error::Error>> {
    let tx = rpc.get_tx(hash).await?;
    println!("{}", "Transaction".bold().cyan());
    println!("  Hash:   {}", tx.hash);
    println!("  Type:   {}", tx.tx_type);
    println!("  From:   {}", tx.from);
    println!("  To:     {}", tx.to);
    if let Some(amt) = tx.amount {
        println!("  Amount: {}", amt);
    }
    println!("  Gas:    {}", tx.gas);
    println!("  Block:  {}", tx.block_number);
    println!("  Status: {}", tx.status);
    Ok(())
}

async fn cmd_account(
    action: AccountAction,
    rpc: RpcClient,
    keystore_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let keystore = load_or_create_keystore(keystore_path);
    let mut mgr = AccountManager::new(keystore, rpc);

    match action {
        AccountAction::Create { name } => {
            validation::validate_name(&name)?;
            let password = prompt_password("Enter password for new key")?;
            validation::validate_password(&password)?;
            let addr = mgr.create_account(&name, &password)?;
            mgr.save(keystore_path)?;
            println!("{} Account '{}' created", "OK".green().bold(), name);
            println!("   Address: {}", format_address(&addr));
        }
        AccountAction::List => {
            let accounts = mgr.list_accounts();
            if crate::output::is_json_mode() {
                let json_accounts: Vec<crate::output::AccountInfo> = accounts
                    .iter()
                    .map(|a| crate::output::AccountInfo {
                        name: a.name.clone(),
                        address: a.address.clone(),
                        balance: a.balance,
                        is_active: a.is_active,
                    })
                    .collect();
                crate::output::print_json(&json_accounts);
            } else if accounts.is_empty() {
                println!("No accounts. Run: {} account create <name>", "wallet".bold());
            } else {
                println!("{}", "Accounts".bold().cyan());
                for a in &accounts {
                    let marker = if a.is_active { " *" } else { "  " };
                    let bal = a
                        .balance
                        .map(|b| format!("{} EVAP", b))
                        .unwrap_or_else(|| "?".to_string());
                    println!("{}{:<12} {} ({})", marker, a.name, a.address, bal);
                }
            }
        }
        AccountAction::Switch { name } => {
            mgr.set_active(&name)?;
            mgr.save(keystore_path)?;
            println!("{} Active account set to '{}'", "OK".green().bold(), name);
        }
        AccountAction::Balance { name } => {
            let target = name.or_else(|| mgr.active_name().map(|s| s.to_string()));
            if let Some(n) = target {
                let (balance, nonce) = mgr.refresh_balance(&n).await?;
                println!("{}: {} EVAP (nonce: {})", n, balance, nonce);
            } else {
                println!("No active account. Run: {} account create <name>", "wallet".bold());
            }
        }
        AccountAction::Detail { name } => {
            let target = name.or_else(|| mgr.active_name().map(|s| s.to_string()));
            if let Some(n) = target {
                if let Some(addr) = mgr.keystore().get_address(&n) {
                    let addr_hex = format_address(&addr);
                    let detail = mgr.rpc().get_address_detail(&addr_hex).await?;
                    println!("{} {}", "Account Detail".bold().cyan(), n);
                    println!("  Address: {}", detail.address);
                    println!("  Balance: {} EVAP", detail.balance);
                    println!("  Nonce:   {}", detail.nonce);
                    println!("  Objects: {}", detail.objects.len());
                    println!("  NFTs:    {}", detail.nfts.len());
                    println!("  Tokens:  {}", detail.tokens.len());
                }
            }
        }
    }
    Ok(())
}

async fn cmd_send(
    rpc: RpcClient,
    keystore_path: &str,
    contacts_path: &str,
    history_path: &str,
    to: &str,
    amount: u64,
    wait: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Validate inputs
    validation::validate_recipient(to)?;
    validation::validate_amount(amount)?;

    let book = AddressBook::load(contacts_path).unwrap_or_default();
    let resolved = book.resolve(to);
    if resolved != to {
        println!("  Resolved '{}' → {}", to, resolved);
    }

    let keystore = load_or_create_keystore(keystore_path);
    let mut mgr = AccountManager::new(keystore, rpc);
    let name = require_active(&mgr)?;
    let from_addr = mgr.active_address_hex().unwrap_or_default();

    show_gas_estimate(mgr.rpc().base_url(), "transfer").await;

    let password = prompt_password("Enter password")?;
    let signer = mgr.get_signer(&name, &password)?;
    let (_, nonce) = mgr.refresh_balance(&name).await?;
    let to_addr = parse_address(&resolved)?;

    let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
    let mut pipeline = TxPipeline::new(rpc2);
    let result = pipeline.transfer(&signer, &to_addr, amount, nonce).await?;

    if let Some(ref hash) = result.tx_hash {
        record_tx(history_path, "Transfer", &from_addr, Some(&resolved), Some(amount), hash);
    }

    if crate::output::is_json_mode() {
        crate::output::print_json(&result);
    } else {
        println!("{} {}", "OK".green().bold(), result.message);
        if let Some(ref hash) = result.tx_hash {
            println!("   Tx Hash: {}", hash);
            if wait {
                await_confirmation(&pipeline, hash).await;
            }
        }
    }
    Ok(())
}

async fn cmd_faucet(
    rpc: RpcClient,
    keystore_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let keystore = load_or_create_keystore(keystore_path);
    let mgr = AccountManager::new(keystore, rpc);
    let addr = mgr.active_address_hex()
        .ok_or("No active account. Run: wallet account create <name>")?;

    let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
    let mut pipeline = TxPipeline::new(rpc2);
    let result = pipeline.faucet(&addr).await?;
    if result.success {
        println!("{} Received tokens! Balance: {} EVAP", "OK".green().bold(), result.balance);
    } else {
        println!("{} {}", "FAIL".red().bold(), result.message.unwrap_or_default());
    }
    Ok(())
}

async fn cmd_objects(rpc: RpcClient) -> Result<(), Box<dyn std::error::Error>> {
    let objects = rpc.get_objects().await?;
    println!("{} ({} total)", "State Objects".bold().cyan(), objects.len());
    for o in &objects {
        let state_color = match o.state.as_str() {
            "Active" => o.state.green(),
            "Grace" => o.state.yellow(),
            "Ghost" => o.state.red(),
            _ => o.state.normal(),
        };
        println!(
            "  {} {:<20} energy={}/{} hl={} [{}]",
            &o.id[..8],
            o.name,
            o.current_energy,
            o.max_energy,
            o.half_life,
            state_color
        );
    }
    Ok(())
}

async fn cmd_object(rpc: RpcClient, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let o = rpc.get_object(id).await?;
    println!("{}", "Object Detail".bold().cyan());
    println!("  ID:          {}", o.id);
    println!("  Name:        {}", o.name);
    println!("  Owner:       {}", o.owner);
    println!("  State:       {}", o.state);
    println!("  Energy:      {}/{}", o.current_energy, o.max_energy);
    println!("  Half-Life:   {}", o.half_life);
    println!("  Created:     epoch {}", o.created_epoch);
    println!("  Refreshed:   epoch {}", o.last_refreshed);
    println!("  Decay:       {:.1}%", o.decay_percentage);
    Ok(())
}

async fn cmd_refresh(
    rpc: RpcClient,
    keystore_path: &str,
    history_path: &str,
    id: &str,
    energy: u64,
    wait: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    validation::validate_address(id)?;
    validation::validate_energy(energy)?;

    let keystore = load_or_create_keystore(keystore_path);
    let mgr = AccountManager::new(keystore, rpc);
    let name = require_active(&mgr)?;
    let from_addr = mgr.active_address_hex().unwrap_or_default();

    show_gas_estimate(mgr.rpc().base_url(), "refresh").await;

    let password = prompt_password("Enter password")?;
    let signer = mgr.get_signer(&name, &password)?;

    let obj_id = parse_address(id)?;
    let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
    let mut pipeline = TxPipeline::new(rpc2);
    let result = pipeline.refresh_object(&signer, &obj_id, energy).await?;

    if let Some(ref hash) = result.tx_hash {
        record_tx(history_path, "Refresh", &from_addr, Some(id), None, hash);
    }

    println!("{} {}", "OK".green().bold(), result.message);
    if wait {
        if let Some(ref hash) = result.tx_hash {
            await_confirmation(&pipeline, hash).await;
        }
    }
    Ok(())
}

async fn cmd_nft(
    action: NftAction,
    rpc: RpcClient,
    _keystore_path: &str,
    history_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        NftAction::List => {
            let nfts = rpc.get_nfts().await?;
            println!("{} ({} total)", "NFTs".bold().cyan(), nfts.len());
            for n in &nfts {
                let state_color = match n.state.as_str() {
                    "Active" => n.state.green(),
                    "Ghost" => n.state.red(),
                    _ => n.state.normal(),
                };
                println!(
                    "  #{:<4} {:<20} energy={}/{} [{}]",
                    n.id, n.name, n.current_energy, n.max_energy, state_color
                );
            }
        }
        NftAction::Show { id } => {
            let n = rpc.get_nft(id).await?;
            println!("{}", "NFT Detail".bold().cyan());
            println!("  ID:         {}", n.id);
            println!("  Name:       {}", n.name);
            println!("  Collection: {}", n.collection);
            println!("  Owner:      {}", n.owner);
            println!("  Energy:     {}/{}", n.current_energy, n.max_energy);
            println!("  Half-Life:  {}", n.half_life);
            println!("  Decay:      {:.1}%", n.decay_percentage);
            println!("  Remaining:  {} epochs", n.epochs_remaining);
        }
        NftAction::Mint { name, energy, half_life, collection, metadata } => {
            show_gas_estimate(rpc.base_url(), "nft mint").await;
            let rpc2 = RpcClient::new(rpc.base_url())?;
            let mut pipeline = TxPipeline::new(rpc2);
            let result = pipeline.mint_nft(&name, collection.as_deref(), &metadata, energy, half_life, None).await?;
            if let Some(ref hash) = result.tx_hash {
                record_tx(history_path, "MintNFT", "", None, None, hash);
            }
            println!("{} {}", "OK".green().bold(), result.message);
        }
        NftAction::Transfer { id, to } => {
            show_gas_estimate(rpc.base_url(), "nft transfer").await;
            let rpc2 = RpcClient::new(rpc.base_url())?;
            let mut pipeline = TxPipeline::new(rpc2);
            let result = pipeline.transfer_nft(id, &to).await?;
            if let Some(ref hash) = result.tx_hash {
                record_tx(history_path, "TransferNFT", "", Some(&to), None, hash);
            }
            println!("{} {}", "OK".green().bold(), result.message);
        }
        NftAction::Refresh { id, energy } => {
            show_gas_estimate(rpc.base_url(), "nft refresh").await;
            let rpc2 = RpcClient::new(rpc.base_url())?;
            let mut pipeline = TxPipeline::new(rpc2);
            let result = pipeline.refresh_nft(id, energy).await?;
            if let Some(ref hash) = result.tx_hash {
                record_tx(history_path, "RefreshNFT", "", None, None, hash);
            }
            println!("{} {}", "OK".green().bold(), result.message);
        }
    }
    Ok(())
}

async fn cmd_token(
    action: TokenAction,
    rpc: RpcClient,
    _keystore_path: &str,
    history_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        TokenAction::List => {
            let tokens = rpc.get_tokens().await?;
            println!("{} ({} total)", "Tokens".bold().cyan(), tokens.len());
            for t in &tokens {
                println!(
                    "  #{:<4} {} ({}) supply={} holders={}",
                    t.id, t.name, t.symbol, t.current_supply, t.holder_count
                );
            }
        }
        TokenAction::Show { id } => {
            let t = rpc.get_token(id).await?;
            println!("{}", "Token Detail".bold().cyan());
            println!("  ID:         {}", t.id);
            println!("  Name:       {} ({})", t.name, t.symbol);
            println!("  Supply:     {}/{}", t.current_supply, t.total_supply);
            println!("  Decay HL:   {}", t.decay_half_life);
            println!("  Decay:      {:.1}%", t.decay_percentage);
            println!("  Holders:    {}", t.holder_count);
        }
        TokenAction::Deploy { name, symbol, supply, half_life } => {
            show_gas_estimate(rpc.base_url(), "token deploy").await;
            let rpc2 = RpcClient::new(rpc.base_url())?;
            let mut pipeline = TxPipeline::new(rpc2);
            let result = pipeline.deploy_token(&name, &symbol, supply, half_life, None).await?;
            if let Some(ref hash) = result.tx_hash {
                record_tx(history_path, "DeployToken", "", None, None, hash);
            }
            println!("{} {}", "OK".green().bold(), result.message);
        }
        TokenAction::Transfer { token_id, to, amount } => {
            show_gas_estimate(rpc.base_url(), "token transfer").await;
            let rpc2 = RpcClient::new(rpc.base_url())?;
            let mut pipeline = TxPipeline::new(rpc2);
            let result = pipeline.transfer_token(token_id, "", &to, amount).await?;
            if let Some(ref hash) = result.tx_hash {
                record_tx(history_path, "TransferToken", "", Some(&to), Some(amount), hash);
            }
            println!("{} {}", "OK".green().bold(), result.message);
        }
    }
    Ok(())
}

async fn cmd_energy(
    action: EnergyAction,
    rpc: RpcClient,
    keystore_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let keystore = load_or_create_keystore(keystore_path);
    let mgr = AccountManager::new(keystore, rpc);

    match action {
        EnergyAction::Scan => {
            if let Some(addr_hex) = mgr.active_address_hex() {
                let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
                let asset_mgr = AssetManager::new(rpc2);
                let portfolio = asset_mgr.fetch_portfolio(&addr_hex).await?;
                let monitor = EnergyMonitor::new();
                let alerts = monitor.scan_portfolio(&portfolio);

                if crate::output::is_json_mode() {
                    let json_alerts: Vec<crate::output::EnergyAlertJson> = alerts
                        .iter()
                        .map(|a| crate::output::EnergyAlertJson {
                            asset_id: a.asset_id.clone(),
                            asset_name: a.asset_name.clone(),
                            severity: format!("{:?}", a.severity).to_lowercase(),
                            energy_pct: a.current_energy_pct,
                            epochs_until_zero: a.epochs_until_zero,
                        })
                        .collect();
                    crate::output::print_json(&json_alerts);
                } else if alerts.is_empty() {
                    println!("{} All assets above 50% energy", "OK".green().bold());
                } else {
                    println!("{} ({} alerts)", "Energy Alerts".bold().cyan(), alerts.len());
                    for a in &alerts {
                        let sev = match a.severity {
                            AlertSeverity::Critical => "CRITICAL".red().bold(),
                            AlertSeverity::Warning => "WARNING".yellow().bold(),
                            AlertSeverity::Info => "INFO".blue(),
                        };
                        let eta = a
                            .epochs_until_zero
                            .map(|e| format!("{} epochs", e))
                            .unwrap_or_else(|| "?".to_string());
                        println!(
                            "  [{}] {} ({}) — {:.1}% energy, evaporates in {}",
                            sev, a.asset_name, a.asset_id, a.current_energy_pct, eta
                        );
                    }
                    println!("\n  Total energy at risk: {}", portfolio.total_energy_at_risk);
                }
            }
        }
        EnergyAction::Forecast { id } => {
            let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
            let obj = rpc2.get_object(&id).await?;
            let monitor = EnergyMonitor::new();
            let owned = crate::assets::OwnedObject {
                id: obj.id.clone(),
                name: obj.name.clone(),
                energy: obj.energy,
                max_energy: obj.max_energy,
                current_energy: obj.current_energy,
                half_life: obj.half_life,
                state: obj.state.clone(),
                created_epoch: obj.created_epoch,
                last_refreshed: obj.last_refreshed,
                decay_percentage: obj.decay_percentage,
                epochs_until_zero: crate::assets::epochs_until_zero(obj.current_energy, obj.half_life),
            };
            let forecast = monitor.forecast_object(&owned);

            println!("{} {}", "Decay Forecast".bold().cyan(), forecast.asset_name);
            println!("  Energy: {}/{}", forecast.current_energy, forecast.max_energy);
            println!("  Half-Life: {} epochs", forecast.half_life);
            println!("\n  {} Milestones:", "Decay".yellow());
            for m in &forecast.milestones {
                println!("    {}% — epoch +{} (energy={})", m.target_pct, m.at_epoch, m.energy_at);
            }
            if let Some(zero) = forecast.zero_epoch {
                println!("    {}  — epoch +{}", "EVAPORATES".red().bold(), zero);
            }
        }
        EnergyAction::AutoRefresh { threshold, interval, max_energy, once } => {
            validation::validate_threshold(threshold)?;
            validation::validate_energy(max_energy)?;

            let addr_hex = mgr.active_address_hex()
                .ok_or("No active account. Run: wallet account create <name>")?;
            let password = prompt_password("Enter password")?;
            let name = require_active(&mgr)?;
            let signer = mgr.get_signer(&name, &password)?;

            let config = AutoRefreshConfig {
                threshold_pct: threshold,
                poll_interval_secs: interval,
                max_energy_per_refresh: max_energy,
                ..Default::default()
            };

            let refresher = AutoRefresher::new(config.clone());
            println!("{}", "Auto-Refresh".bold().cyan());
            println!("  Threshold:    {}%", config.threshold_pct);
            println!("  Poll Interval: {}s", config.poll_interval_secs);
            println!("  Max Energy:   {}", config.max_energy_per_refresh);
            println!();

            if once {
                let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
                let actions = refresher.execute_cycle(&rpc2, &signer, &addr_hex).await
                    .map_err(|e| -> Box<dyn std::error::Error> { Box::new(std::io::Error::other(e.to_string())) })?;
                if actions.is_empty() {
                    println!("{} All assets above {}% energy", "OK".green().bold(), threshold);
                } else {
                    for a in &actions {
                        let status = if a.submitted {
                            "REFRESHED".green().bold()
                        } else {
                            "FAILED".red().bold()
                        };
                        println!(
                            "  [{}] {} — {:.1}% → +{} energy {}",
                            status,
                            a.asset_name,
                            a.current_pct,
                            a.energy_to_deposit,
                            a.tx_hash.as_deref().unwrap_or("")
                        );
                        if let Some(ref err) = a.error {
                            println!("         Error: {}", err.red());
                        }
                    }
                }
            } else {
                println!("  Starting auto-refresh loop (Ctrl+C to stop)...\n");
                let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
                let summary = refresher
                    .run_loop_graceful(&rpc2, &signer, &addr_hex, |actions| {
                        for a in actions {
                            let status = if a.submitted {
                                "REFRESHED".green().bold()
                            } else {
                                "FAILED".red().bold()
                            };
                            println!(
                                "  [{}] {} — {:.1}% → +{} energy",
                                status, a.asset_name, a.current_pct, a.energy_to_deposit
                            );
                        }
                    })
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error> { Box::new(std::io::Error::other(e.to_string())) })?;

                println!("\n{}", "Auto-Refresh Stopped".bold().yellow());
                println!("  Reason:    {}", summary.shutdown_reason);
                println!("  Cycles:    {}", summary.cycles);
                println!("  Refreshed: {}", summary.refreshes);
                println!("  Failed:    {}", summary.failures);
            }
        }
    }
    Ok(())
}

async fn cmd_ghost(
    action: GhostAction,
    rpc: RpcClient,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        GhostAction::List => {
            let ghosts = rpc.get_ghosts().await?;
            println!("{} ({} total)", "Ghost Records".bold().cyan(), ghosts.len());
            for g in &ghosts {
                println!(
                    "  {} owner={} evaporated=epoch {}",
                    &g.id[..16],
                    &g.original_owner[..16],
                    g.evaporated_epoch
                );
            }
        }
        GhostAction::Cost { half_life } => {
            let cost = crate::energy::resurrection_cost(half_life, 5);
            println!("{}", "Resurrection Cost".bold().cyan());
            println!("  Half-Life:    {}", half_life);
            println!("  Grace Period: 5 epochs");
            println!("  Min Energy:   {} units", cost);
        }
    }
    Ok(())
}

async fn cmd_stake(
    action: StakeAction,
    rpc: RpcClient,
    keystore_path: &str,
    history_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        StakeAction::Pools => {
            let mgr = StakingManager::new(rpc);
            let pools = mgr.list_pools().await?;
            println!("{} ({} pools)", "Staking Pools".bold().cyan(), pools.len());
            for p in &pools {
                println!(
                    "  #{:<4} {:<20} rate={} staked={} stakers={}",
                    p.id, p.name, p.reward_rate, p.total_staked, p.staker_count
                );
            }
        }
        StakeAction::In { pool_id, amount } => {
            let keystore = load_or_create_keystore(keystore_path);
            let acct_mgr = AccountManager::new(keystore, rpc);
            let addr = acct_mgr.active_address_hex()
                .ok_or("No active account. Run: wallet account create <name>")?;
            show_gas_estimate(acct_mgr.rpc().base_url(), "stake").await;
            let rpc2 = RpcClient::new(acct_mgr.rpc().base_url())?;
            let staking = StakingManager::new(rpc2);
            let result = staking.stake(pool_id, &addr, amount).await?;
            if let Some(ref hash) = result.tx_hash {
                record_tx(history_path, "Stake", &addr, None, Some(amount), hash);
            }
            println!("{} {}", "OK".green().bold(), result.message);
        }
        StakeAction::Out { pool_id, amount } => {
            let keystore = load_or_create_keystore(keystore_path);
            let acct_mgr = AccountManager::new(keystore, rpc);
            let addr = acct_mgr.active_address_hex()
                .ok_or("No active account. Run: wallet account create <name>")?;
            let rpc2 = RpcClient::new(acct_mgr.rpc().base_url())?;
            let staking = StakingManager::new(rpc2);
            let result = staking.unstake(pool_id, &addr, amount).await?;
            if let Some(ref hash) = result.tx_hash {
                record_tx(history_path, "Unstake", &addr, None, Some(amount), hash);
            }
            println!("{} {}", "OK".green().bold(), result.message);
        }
        StakeAction::Claim { pool_id } => {
            let keystore = load_or_create_keystore(keystore_path);
            let acct_mgr = AccountManager::new(keystore, rpc);
            let addr = acct_mgr.active_address_hex()
                .ok_or("No active account. Run: wallet account create <name>")?;
            let rpc2 = RpcClient::new(acct_mgr.rpc().base_url())?;
            let staking = StakingManager::new(rpc2);
            let result = staking.claim(pool_id, &addr).await?;
            if let Some(ref hash) = result.tx_hash {
                record_tx(history_path, "ClaimRewards", &addr, None, None, hash);
            }
            println!("{} {}", "OK".green().bold(), result.message);
        }
        StakeAction::Forecast { pool_id, amount, epochs } => {
            let staking = StakingManager::new(rpc);
            let pool = staking.get_pool(pool_id).await?;
            let forecast = StakingManager::reward_forecast(
                amount,
                pool.reward_rate,
                pool.reward_decay_hl,
                pool.total_staked,
                epochs,
            );
            println!("{}", "Reward Forecast".bold().cyan());
            println!("  Pool:           {} (#{}) ", pool.name, pool.id);
            println!("  Stake Amount:   {}", amount);
            println!("  Epochs:         {}", epochs);
            println!("  Gross Rewards:  {}", forecast.gross_rewards);
            println!("  Decay Loss:     {} ({})", forecast.decay_loss, "rewards decay too!".yellow());
            println!("  Net Rewards:    {}", forecast.net_rewards.to_string().green());
        }
    }
    Ok(())
}

async fn cmd_dao(
    action: DaoAction,
    rpc: RpcClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let gov = GovernanceManager::new(rpc);
    match action {
        DaoAction::Proposals => {
            let proposals = gov.list_proposals().await?;
            println!("{} ({} total)", "DAO Proposals".bold().cyan(), proposals.len());
            for p in &proposals {
                let status_color = match p.status.as_str() {
                    "Active" => p.status.green(),
                    _ => p.status.yellow(),
                };
                println!(
                    "  #{:<4} {:<30} [{}] votes={} remaining={} epochs",
                    p.id, p.title, status_color, p.total_votes, p.epochs_remaining
                );
            }
        }
        DaoAction::Show { id } => {
            let p = gov.get_proposal(id).await?;
            println!("{}", "Proposal Detail".bold().cyan());
            println!("  ID:          {}", p.id);
            println!("  Title:       {}", p.title);
            println!("  Description: {}", p.description);
            println!("  Status:      {}", p.status);
            println!("  Options:     {}", p.options.join(", "));
            println!("  Votes:       {}", p.total_votes);
            for (opt, count) in &p.vote_totals {
                println!("    {}: {}", opt, count);
            }
            println!("  Remaining:   {} epochs", p.epochs_remaining);
        }
        DaoAction::Vote { id, option, weight } => {
            let result = gov.vote(id, &option, weight, None).await?;
            println!("{} {}", "OK".green().bold(), result.message);
        }
    }
    Ok(())
}

fn cmd_backup(
    action: BackupAction,
    keystore_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        BackupAction::Export { file } => {
            let keystore = load_or_create_keystore(keystore_path);
            let password = prompt_password("Enter backup password")?;
            BackupManager::export_to_file(&keystore, &file, &password)?;
            println!("{} Backup exported to {:?}", "OK".green().bold(), file);
        }
        BackupAction::Import { file } => {
            let password = prompt_password("Enter backup password")?;
            let keystore = BackupManager::import_from_file(&file, &password)?;
            keystore.save(keystore_path)?;
            println!(
                "{} Imported {} keys from backup",
                "OK".green().bold(),
                keystore.len()
            );
        }
        BackupAction::Rotate => {
            let mut keystore = load_or_create_keystore(keystore_path);
            let old_pass = prompt_password("Enter current password")?;
            let new_pass = prompt_password("Enter new password")?;
            let count = BackupManager::rotate_all(&mut keystore, &old_pass, &new_pass)?;
            keystore.save(keystore_path)?;
            println!("{} Rotated {} keys", "OK".green().bold(), count);
        }
        BackupAction::Keys => {
            let keystore = load_or_create_keystore(keystore_path);
            let keys = BackupManager::export_public_keys(&keystore);
            println!("{}", "Public Keys".bold().cyan());
            for (name, pk) in &keys {
                println!("  {}: {}...{}", name, &pk[..16], &pk[pk.len() - 16..]);
            }
        }
    }
    Ok(())
}

// ──────────────────────────── Seed Commands ───────────────────────────

fn cmd_seed(
    action: SeedAction,
    keystore_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        SeedAction::Generate => {
            let mnemonic = Mnemonic::generate();
            println!("{}", "New Seed Phrase".bold().cyan());
            println!();
            let words = mnemonic.words();
            for (i, word) in words.iter().enumerate() {
                print!("  {:>2}. {:<12}", i + 1, word);
                if (i + 1) % 4 == 0 {
                    println!();
                }
            }
            println!();
            println!(
                "  {} Write these words down and store them safely!",
                "WARNING".red().bold()
            );
            println!("  {} Never share your seed phrase with anyone.", "WARNING".red().bold());
        }
        SeedAction::Backup { name, file } => {
            let keystore = load_or_create_keystore(keystore_path);
            let password = prompt_password("Enter account password")?;
            let keypair = keystore.unlock_key(&name, &password)?;

            println!("Enter your 24-word seed phrase (space-separated):");
            let mut phrase = String::new();
            std::io::stdin().read_line(&mut phrase)?;
            let mnemonic = Mnemonic::from_phrase(phrase.trim())?;

            let backup = mnemonic.backup_keypair(&keypair)?;
            let json = backup.to_json()?;
            std::fs::write(&file, &json)?;

            println!(
                "{} Keypair '{}' backed up to {:?}",
                "OK".green().bold(),
                name,
                file
            );
            println!("  Address: {}", backup.address);
        }
        SeedAction::Recover { file, name } => {
            let json = std::fs::read_to_string(&file)?;
            let backup = MnemonicBackup::from_json(&json)?;

            println!("Enter your 24-word seed phrase (space-separated):");
            let mut phrase = String::new();
            std::io::stdin().read_line(&mut phrase)?;
            let mnemonic = Mnemonic::from_phrase(phrase.trim())?;

            let keypair = mnemonic.recover_keypair(&backup)?;

            let password = prompt_password("Enter password for the recovered key")?;
            let pk = {
                use evaporchain_crypto::signatures::Signer;
                keypair.public_key_bytes()
            };
            let sk = keypair.secret_key();

            let mut keystore = load_or_create_keystore(keystore_path);
            let addr = keystore.import_key(&name, &password, &pk, sk)?;
            keystore.save(keystore_path)?;

            println!(
                "{} Recovered keypair as '{}'",
                "OK".green().bold(),
                name
            );
            println!("  Address: {}", format_address(&addr));
        }
    }
    Ok(())
}

// ──────────────────────────── Contact Commands ────────────────────────

fn cmd_contacts(
    action: ContactAction,
    contacts_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut book = AddressBook::load(contacts_path).unwrap_or_default();

    match action {
        ContactAction::Add { name, address, note } => {
            book.add(&name, &address, note.as_deref())?;
            book.save(contacts_path)?;
            println!("{} Contact '{}' added ({})", "OK".green().bold(), name, address);
        }
        ContactAction::List => {
            let contacts = book.list();
            if contacts.is_empty() {
                println!("No contacts. Run: {} contacts add <name> <address>", "wallet".bold());
            } else {
                println!("{} ({} contacts)", "Address Book".bold().cyan(), contacts.len());
                for c in contacts {
                    let note = c.note.as_deref().unwrap_or("");
                    println!("  {:<15} {} {}", c.name, c.address, note.dimmed());
                }
            }
        }
        ContactAction::Remove { name } => {
            book.remove(&name)?;
            book.save(contacts_path)?;
            println!("{} Contact '{}' removed", "OK".green().bold(), name);
        }
        ContactAction::Show { name } => {
            if let Some(c) = book.get_by_name(&name) {
                println!("{}", "Contact".bold().cyan());
                println!("  Name:    {}", c.name);
                println!("  Address: {}", c.address);
                if let Some(note) = &c.note {
                    println!("  Note:    {}", note);
                }
                println!("  Added:   {}", c.created_at);
            } else {
                println!("Contact '{}' not found", name);
            }
        }
        ContactAction::Export { file } => {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("csv");
            if ext == "json" {
                let json = book.export_json()?;
                std::fs::write(&file, json)?;
            } else {
                book.export_csv(&file)?;
            }
            println!(
                "{} Exported {} contacts to {:?}",
                "OK".green().bold(),
                book.len(),
                file
            );
        }
        ContactAction::Import { file } => {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("csv");
            let count = if ext == "json" {
                let data = std::fs::read_to_string(&file)?;
                book.import_json(&data)?
            } else {
                book.import_csv_file(&file)?
            };
            book.save(contacts_path)?;
            println!(
                "{} Imported {} contacts from {:?}",
                "OK".green().bold(),
                count,
                file
            );
        }
    }
    Ok(())
}

// ──────────────────────────── History Commands ────────────────────────

fn cmd_history(
    action: HistoryAction,
    history_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        HistoryAction::List { limit } => {
            let history = TxHistory::load_or_empty(history_path)?;
            let entries = history.recent(limit);
            if crate::output::is_json_mode() {
                let json_entries: Vec<crate::output::HistoryEntryJson> = entries
                    .iter()
                    .map(|e| crate::output::HistoryEntryJson {
                        tx_hash: e.tx_hash.clone(),
                        tx_type: e.tx_type.clone(),
                        from: e.from.clone(),
                        to: e.to.clone(),
                        amount: e.amount,
                        outcome: format!("{:?}", e.outcome).to_lowercase(),
                        submitted_at: e.submitted_at.clone(),
                    })
                    .collect();
                crate::output::print_json(&json_entries);
            } else if entries.is_empty() {
                println!("No transaction history yet.");
            } else {
                println!(
                    "{} (showing {} of {})",
                    "Transaction History".bold().cyan(),
                    entries.len(),
                    history.len()
                );
                for e in entries {
                    let status = match e.outcome {
                        crate::history::TxOutcome::Accepted => "OK".green(),
                        crate::history::TxOutcome::Confirmed => "CONFIRMED".green().bold(),
                        crate::history::TxOutcome::Rejected => "FAIL".red(),
                        crate::history::TxOutcome::Pending => "PENDING".yellow(),
                    };
                    let hash = e
                        .tx_hash
                        .as_deref()
                        .map(|h| if h.len() > 16 { &h[..16] } else { h })
                        .unwrap_or("—");
                    let amount_str = e
                        .amount
                        .map(|a| format!(" {} EVAP", a))
                        .unwrap_or_default();
                    println!(
                        "  [{}] {:<12} {}...{} → {}{}",
                        status,
                        e.tx_type,
                        &e.from[..8.min(e.from.len())],
                        hash,
                        e.to.as_deref().unwrap_or("—"),
                        amount_str
                    );
                }
            }
        }
        HistoryAction::For { address } => {
            let history = TxHistory::load_or_empty(history_path)?;
            let entries = history.for_address(&address);
            println!(
                "{} {} ({} transactions)",
                "History for".bold().cyan(),
                address,
                entries.len()
            );
            for e in entries {
                let dir = if e.from == address { "SENT" } else { "RECV" };
                println!(
                    "  [{}] {} {} {}",
                    dir,
                    e.tx_type,
                    e.amount.map(|a| format!("{} EVAP", a)).unwrap_or_default(),
                    e.submitted_at
                );
            }
        }
        HistoryAction::Export { file } => {
            let history = TxHistory::load_or_empty(history_path)?;
            if history.is_empty() {
                println!("No transaction history to export.");
            } else {
                history.export_csv(&file)?;
                println!(
                    "{} Exported {} transactions to {:?}",
                    "OK".green().bold(),
                    history.len(),
                    file
                );
            }
        }
        HistoryAction::Clear => {
            let mut history = TxHistory::load_or_empty(history_path)?;
            let count = history.len();
            history.clear();
            history.save(history_path)?;
            println!("{} Cleared {} entries", "OK".green().bold(), count);
        }
    }
    Ok(())
}

// ──────────────────────────── Gas Commands ────────────────────────────

async fn cmd_gas(
    action: GasAction,
    rpc: RpcClient,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        GasAction::Transfer => {
            let estimator = GasEstimator::from_rpc(&rpc).await?;
            let est = estimator.estimate_transfer();
            if crate::output::is_json_mode() {
                crate::output::print_json(&crate::output::GasEstimateJson {
                    gas_used: est.gas_used,
                    base_fee: est.base_fee,
                    gas_fee: est.gas_fee,
                    extra_fee: est.extra_fee,
                    total_fee: est.total_fee,
                });
            } else {
                println!("{}", "Transfer Fee Estimate".bold().cyan());
                println!("  Gas Used:  {}", est.gas_used);
                println!("  Base Fee:  {}", est.base_fee);
                println!("  Gas Fee:   {}", est.gas_fee);
                println!("  Total:     {} units", est.total_fee);
            }
        }
        GasAction::Create { size } => {
            let estimator = GasEstimator::from_rpc(&rpc).await?;
            let est = estimator.estimate_create_object(size);
            println!("{}", "Create Object Fee Estimate".bold().cyan());
            println!("  Data Size: {} bytes", size);
            println!("  Gas Used:  {}", est.gas_used);
            println!("  Base Fee:  {}", est.base_fee);
            println!("  Gas Fee:   {}", est.gas_fee);
            println!("  Deposit:   {}", est.extra_fee);
            println!("  Total:     {} units", est.total_fee);
        }
        GasAction::Refresh { energy } => {
            let estimator = GasEstimator::from_rpc(&rpc).await?;
            let gas_fee = estimator.base_fee() * crate::gas::GAS_REFRESH;
            let refresh_fee = estimator.refresh_fee(energy);
            let total = gas_fee + refresh_fee;
            println!("{}", "Refresh Fee Estimate".bold().cyan());
            println!("  Energy:      {}", energy);
            println!("  Gas Fee:     {}", gas_fee);
            println!("  Refresh Fee: {} (20% of energy)", refresh_fee);
            println!("  Total:       {} units", total);
        }
        GasAction::BaseFee => {
            let block = rpc.get_latest_block().await?;
            println!("{}", "Current Fee Market".bold().cyan());
            println!("  Block:    #{}", block.number);
            println!("  Base Fee: {}", block.base_fee);
            println!("  Gas Used: {} (block total)", block.gas_used);
            println!("  Fees:     {} (block total)", block.total_fees);
        }
    }
    Ok(())
}

// ──────────────────────────── Config Commands ─────────────────────────

fn cmd_config(
    action: ConfigAction,
    config_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        ConfigAction::Show => {
            let config = WalletConfig::load_or_default(config_path)?;
            println!("{}", "Wallet Configuration".bold().cyan());
            println!("  Node URL:       {}", config.node_url);
            println!("  Keystore:       {}", config.keystore_path);
            println!("  Contacts:       {}", config.contacts_path);
            println!("  History:        {}", config.history_path);
            println!(
                "  Active Account: {}",
                config.active_account.as_deref().unwrap_or("(none)")
            );
            println!("  Default HL:     {} epochs", config.default_half_life);
            println!("  Default Energy: {}", config.default_energy);
            println!("\n  Config file: {:?}", config_path);
        }
        ConfigAction::Set { key, value } => {
            let mut config = WalletConfig::load_or_default(config_path)?;
            match key.as_str() {
                "node_url" => config.node_url = value.clone(),
                "active_account" => config.active_account = Some(value.clone()),
                "default_half_life" => {
                    config.default_half_life = value.parse().map_err(|_| "invalid number")?
                }
                "default_energy" => {
                    config.default_energy = value.parse().map_err(|_| "invalid number")?
                }
                other => {
                    return Err(format!(
                        "Unknown key '{}'. Valid: node_url, active_account, default_half_life, default_energy",
                        other
                    )
                    .into())
                }
            }
            config.save(config_path)?;
            println!("{} {} = {}", "OK".green().bold(), key, value);
        }
        ConfigAction::Reset => {
            let config = WalletConfig::default();
            config.save(config_path)?;
            println!("{} Config reset to defaults", "OK".green().bold());
        }
    }
    Ok(())
}

// ──────────────────────────── Batch ───────────────────────────────────────

async fn cmd_batch(
    rpc: RpcClient,
    keystore_path: &str,
    history_path: &str,
    file: &std::path::Path,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::batch::{BatchFile, BatchOperation, BatchResult, BatchSummary};

    let batch = BatchFile::load(file)?;

    // Validate
    let errors = batch.validate();
    if !errors.is_empty() {
        if crate::output::is_json_mode() {
            crate::output::print_json(&serde_json::json!({
                "status": "validation_failed",
                "errors": errors
            }));
        } else {
            println!("{} Batch validation failed:", "ERROR".red().bold());
            for e in &errors {
                println!("  - {}", e);
            }
        }
        return Ok(());
    }

    // Summary
    let summary_text = batch.summary();
    if !crate::output::is_json_mode() {
        println!("{}", "Batch Execution".bold().cyan());
        println!("  {}", summary_text);
        println!();
    }

    if dry_run {
        if crate::output::is_json_mode() {
            crate::output::print_json(&serde_json::json!({
                "status": "dry_run",
                "summary": summary_text,
                "operations": batch.operations.len()
            }));
        } else {
            println!("  {} Dry run — no transactions submitted.", "OK".green().bold());
            for (i, op) in batch.operations.iter().enumerate() {
                match op {
                    BatchOperation::Transfer { to, amount } => {
                        println!("  #{}: Transfer {} EVAP → {}", i + 1, amount, to);
                    }
                    BatchOperation::Refresh { object_id, energy } => {
                        println!("  #{}: Refresh {} +{} energy", i + 1, &object_id[..16], energy);
                    }
                }
            }
        }
        return Ok(());
    }

    // Execute
    let keystore = load_or_create_keystore(keystore_path);
    let mgr = AccountManager::new(keystore, rpc);
    let name = require_active(&mgr)?;
    let from_addr = mgr.active_address_hex().unwrap_or_default();
    let password = prompt_password("Enter password")?;
    let signer = mgr.get_signer(&name, &password)?;

    let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
    let mut pipeline = TxPipeline::new(rpc2);

    let mut results = Vec::new();

    for (i, op) in batch.operations.iter().enumerate() {
        match op {
            BatchOperation::Transfer { to, amount } => {
                let to_addr = parse_address(to)?;
                // Fetch current nonce for each tx
                let rpc_tmp = RpcClient::new(mgr.rpc().base_url())?;
                let detail = rpc_tmp.get_address_detail(&from_addr).await?;
                let nonce = detail.nonce + i as u64;

                match pipeline.transfer(&signer, &to_addr, *amount, nonce).await {
                    Ok(resp) => {
                        let hash = resp.tx_hash.clone();
                        if let Some(ref h) = hash {
                            record_tx(history_path, "Transfer", &from_addr, Some(to), Some(*amount), h);
                        }
                        if !crate::output::is_json_mode() {
                            println!(
                                "  [{}] #{}: Transfer {} EVAP → {} {}",
                                "OK".green().bold(),
                                i + 1,
                                amount,
                                &to[..16],
                                hash.as_deref().unwrap_or("")
                            );
                        }
                        results.push(BatchResult {
                            index: i,
                            operation: "transfer".to_string(),
                            success: true,
                            tx_hash: hash,
                            error: None,
                        });
                    }
                    Err(e) => {
                        if !crate::output::is_json_mode() {
                            println!(
                                "  [{}] #{}: Transfer failed: {}",
                                "FAIL".red().bold(),
                                i + 1,
                                e
                            );
                        }
                        results.push(BatchResult {
                            index: i,
                            operation: "transfer".to_string(),
                            success: false,
                            tx_hash: None,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
            BatchOperation::Refresh { object_id, energy } => {
                let obj_id = parse_address(object_id)?;
                match pipeline.refresh_object(&signer, &obj_id, *energy).await {
                    Ok(resp) => {
                        let hash = resp.tx_hash.clone();
                        if let Some(ref h) = hash {
                            record_tx(history_path, "Refresh", &from_addr, Some(object_id), None, h);
                        }
                        if !crate::output::is_json_mode() {
                            println!(
                                "  [{}] #{}: Refresh {} +{} energy {}",
                                "OK".green().bold(),
                                i + 1,
                                &object_id[..16],
                                energy,
                                hash.as_deref().unwrap_or("")
                            );
                        }
                        results.push(BatchResult {
                            index: i,
                            operation: "refresh".to_string(),
                            success: true,
                            tx_hash: hash,
                            error: None,
                        });
                    }
                    Err(e) => {
                        if !crate::output::is_json_mode() {
                            println!(
                                "  [{}] #{}: Refresh failed: {}",
                                "FAIL".red().bold(),
                                i + 1,
                                e
                            );
                        }
                        results.push(BatchResult {
                            index: i,
                            operation: "refresh".to_string(),
                            success: false,
                            tx_hash: None,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
        }
    }

    let succeeded = results.iter().filter(|r| r.success).count();
    let failed = results.iter().filter(|r| !r.success).count();

    let summary = BatchSummary {
        total: results.len(),
        succeeded,
        failed,
        results,
    };

    if crate::output::is_json_mode() {
        crate::output::print_json(&summary);
    } else {
        println!();
        println!(
            "  {} {} succeeded, {} failed",
            "Done:".bold(),
            succeeded.to_string().green(),
            if failed > 0 {
                failed.to_string().red().to_string()
            } else {
                "0".to_string()
            }
        );
    }

    Ok(())
}

// ──────────────────────────── Doctor ──────────────────────────────────────

async fn cmd_doctor(
    node_url: &str,
    keystore_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::doctor::{run_checks, CheckStatus};

    let results = run_checks(node_url, keystore_path).await;

    if crate::output::is_json_mode() {
        crate::output::print_json(&results);
        return Ok(());
    }

    println!("{}", "Wallet Doctor".bold().cyan());
    println!("{}", "─".repeat(50));

    let mut pass_count = 0;
    let mut warn_count = 0;
    let mut fail_count = 0;

    for r in &results {
        let (_icon, color_status) = match r.status {
            CheckStatus::Pass => {
                pass_count += 1;
                ("PASS", "PASS".green().bold())
            }
            CheckStatus::Warn => {
                warn_count += 1;
                ("WARN", "WARN".yellow().bold())
            }
            CheckStatus::Fail => {
                fail_count += 1;
                ("FAIL", "FAIL".red().bold())
            }
        };
        println!("  [{}] {}: {}", color_status, r.name.bold(), r.message);
        if let Some(ref fix) = r.fix {
            println!("         Fix: {}", fix.dimmed());
        }
    }

    println!("{}", "─".repeat(50));
    println!(
        "  {} pass, {} warn, {} fail",
        pass_count.to_string().green(),
        warn_count.to_string().yellow(),
        fail_count.to_string().red()
    );

    if fail_count > 0 {
        println!("\n  {} Fix the issues above before using the wallet.", "Action needed:".red().bold());
    } else if warn_count > 0 {
        println!("\n  {} Wallet is usable but some things could be better.", "Note:".yellow());
    } else {
        println!("\n  {} Everything looks good!", "All clear!".green().bold());
    }

    Ok(())
}

// ──────────────────────────── Dashboard ───────────────────────────────────

async fn cmd_dashboard(
    rpc: RpcClient,
    keystore_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let keystore = load_or_create_keystore(keystore_path);
    let mgr = AccountManager::new(keystore, rpc);
    let accounts = mgr.list_accounts();

    if accounts.is_empty() {
        println!("No accounts. Run: {} account create <name>", "wallet".bold());
        return Ok(());
    }

    let mut total_balance: u64 = 0;
    let mut total_objects: usize = 0;
    let mut total_nfts: usize = 0;
    let mut total_energy_at_risk: u64 = 0;
    let mut all_alerts = Vec::new();

    #[derive(serde::Serialize)]
    struct DashboardAccount {
        name: String,
        address: String,
        balance: u64,
        objects: usize,
        nfts: usize,
        energy_at_risk: u64,
        active: bool,
    }

    #[derive(serde::Serialize)]
    struct DashboardJson {
        accounts: Vec<DashboardAccount>,
        total_balance: u64,
        total_objects: usize,
        total_nfts: usize,
        total_energy_at_risk: u64,
        alerts: usize,
    }

    let mut dashboard_accounts = Vec::new();

    if !crate::output::is_json_mode() {
        println!("{}", "Portfolio Dashboard".bold().cyan());
        println!("{}", "═".repeat(60));
    }

    for acct in &accounts {
        let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
        let asset_mgr = AssetManager::new(rpc2);

        match asset_mgr.fetch_portfolio(&acct.address).await {
            Ok(portfolio) => {
                let balance = portfolio.balance;
                let obj_count = portfolio.objects.len();
                let nft_count = portfolio.nfts.len();
                let energy = portfolio.total_energy_at_risk;

                total_balance += balance;
                total_objects += obj_count;
                total_nfts += nft_count;
                total_energy_at_risk += energy;

                let monitor = EnergyMonitor::new();
                let alerts = monitor.scan_portfolio(&portfolio);
                let alert_count = alerts.len();
                all_alerts.extend(alerts);

                dashboard_accounts.push(DashboardAccount {
                    name: acct.name.clone(),
                    address: acct.address.clone(),
                    balance,
                    objects: obj_count,
                    nfts: nft_count,
                    energy_at_risk: energy,
                    active: acct.is_active,
                });

                if !crate::output::is_json_mode() {
                    let marker = if acct.is_active { "*" } else { " " };
                    let alert_str = if alert_count > 0 {
                        format!(" {} alerts", alert_count).red().to_string()
                    } else {
                        String::new()
                    };
                    println!(
                        " {} {:<12} {:>10} EVAP  {:>3} objs  {:>3} nfts{}",
                        marker, acct.name, balance, obj_count, nft_count, alert_str
                    );
                }
            }
            Err(e) => {
                if !crate::output::is_json_mode() {
                    println!("   {:<12} {} {}", acct.name, "Error:".red(), e);
                }
            }
        }
    }

    if crate::output::is_json_mode() {
        crate::output::print_json(&DashboardJson {
            accounts: dashboard_accounts,
            total_balance,
            total_objects,
            total_nfts,
            total_energy_at_risk,
            alerts: all_alerts.len(),
        });
    } else {
        println!("{}", "═".repeat(60));
        println!(
            "   {:<12} {:>10} EVAP  {:>3} objs  {:>3} nfts",
            "TOTAL".bold(),
            total_balance,
            total_objects,
            total_nfts
        );

        if !all_alerts.is_empty() {
            println!();
            println!(
                "  {} ({} across all accounts)",
                "Energy Alerts".bold().yellow(),
                all_alerts.len()
            );
            for a in all_alerts.iter().take(10) {
                let sev = match a.severity {
                    AlertSeverity::Critical => "CRIT".red().bold(),
                    AlertSeverity::Warning => "WARN".yellow().bold(),
                    AlertSeverity::Info => "INFO".blue(),
                };
                println!(
                    "    [{}] {} — {:.1}%",
                    sev, a.asset_name, a.current_energy_pct
                );
            }
            if all_alerts.len() > 10 {
                println!("    ... and {} more", all_alerts.len() - 10);
            }
        }

        println!();
        println!("  Total energy at risk: {}", total_energy_at_risk);
    }

    Ok(())
}

// ──────────────────────────── Watch Mode ──────────────────────────────────

async fn cmd_watch(
    rpc: RpcClient,
    address: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    validation::validate_address(address)?;

    let rpc2 = RpcClient::new(rpc.base_url())?;
    let asset_mgr = AssetManager::new(rpc2);
    let detail = rpc.get_address_detail(address).await?;
    let portfolio = asset_mgr.fetch_portfolio(address).await?;

    if crate::output::is_json_mode() {
        crate::output::print_json(&crate::output::PortfolioJson {
            address: detail.address.clone(),
            balance: detail.balance,
            objects: detail.objects.len(),
            nfts: detail.nfts.len(),
            tokens: detail.tokens.len(),
            total_energy_at_risk: portfolio.total_energy_at_risk,
        });
        return Ok(());
    }

    println!("{} {}", "Watching Address".bold().cyan(), address);
    println!();

    // Balance
    println!("  {} {} EVAP", "Balance:".bold(), detail.balance);
    println!("  {} {}", "Nonce:".bold(), detail.nonce);
    println!();

    // Objects
    if detail.objects.is_empty() {
        println!("  No state objects.");
    } else {
        println!("  {} ({} total)", "State Objects".bold(), detail.objects.len());
        for o in &detail.objects {
            let state_color = match o.state.as_str() {
                "Active" => o.state.green(),
                "Grace" => o.state.yellow(),
                "Ghost" => o.state.red(),
                _ => o.state.normal(),
            };
            let pct = if o.max_energy > 0 {
                format!("{:.1}%", (o.current_energy as f64 / o.max_energy as f64) * 100.0)
            } else {
                "—".to_string()
            };
            println!(
                "    {} {:<20} energy={}/{} ({}) [{}]",
                &o.id[..8],
                o.name,
                o.current_energy,
                o.max_energy,
                pct,
                state_color
            );
        }
    }
    println!();

    // NFTs
    if !detail.nfts.is_empty() {
        println!("  {} ({} total)", "NFTs".bold(), detail.nfts.len());
        for n in &detail.nfts {
            println!(
                "    #{:<4} {:<20} energy={}/{} [{}]",
                n.id, n.name, n.current_energy, n.max_energy, n.state
            );
        }
        println!();
    }

    // Tokens
    if !detail.tokens.is_empty() {
        println!("  {} ({} total)", "Tokens".bold(), detail.tokens.len());
        for t in &detail.tokens {
            println!("    #{:<4} {} — balance: {}", t.token_id, t.name, t.balance);
        }
        println!();
    }

    // Energy alerts
    let monitor = EnergyMonitor::new();
    let alerts = monitor.scan_portfolio(&portfolio);
    if !alerts.is_empty() {
        println!("  {} ({} alerts)", "Energy Alerts".bold().yellow(), alerts.len());
        for a in &alerts {
            let sev = match a.severity {
                AlertSeverity::Critical => "CRIT".red().bold(),
                AlertSeverity::Warning => "WARN".yellow().bold(),
                AlertSeverity::Info => "INFO".blue(),
            };
            let eta = a
                .epochs_until_zero
                .map(|e| format!("{} epochs", e))
                .unwrap_or_else(|| "?".to_string());
            println!(
                "    [{}] {} — {:.1}% energy, evaporates in {}",
                sev, a.asset_name, a.current_energy_pct, eta
            );
        }
        println!();
    }

    println!("  {} {}", "Energy at risk:".bold(), portfolio.total_energy_at_risk);

    Ok(())
}

// ──────────────────────────── Interactive Mode ────────────────────────────

async fn cmd_interactive(
    rpc: RpcClient,
    keystore_path: &str,
    contacts_path: &str,
    history_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use dialoguer::{Input, Select};

    println!();
    println!("{}", "Welcome to EvaporChain Wallet".bold().cyan());
    println!("  The world's first post-quantum wallet with energy decay management.");
    println!();

    loop {
        let actions = vec![
            "Check Status       — View chain status",
            "Account             — Create or manage accounts",
            "Balance             — Check your balance",
            "Send                — Transfer EVAP tokens",
            "Faucet              — Get testnet tokens",
            "Objects             — View your state objects",
            "Energy Scan         — Check asset energy levels",
            "Gas Estimate        — Estimate transaction fees",
            "History             — View transaction history",
            "Quit                — Exit wallet",
        ];

        println!();
        let selection = Select::new()
            .with_prompt("What would you like to do?")
            .items(&actions)
            .default(0)
            .interact_opt()?;

        let Some(idx) = selection else {
            println!("Goodbye!");
            break;
        };

        println!();

        match idx {
            0 => {
                // Status
                let rpc2 = RpcClient::new(rpc.base_url())?;
                if let Err(e) = cmd_status(rpc2).await {
                    println!("  {} {}", "Error:".red(), e);
                }
            }
            1 => {
                // Account
                let account_actions = vec!["Create new account", "List accounts", "Switch account", "Back"];
                let sel = Select::new()
                    .with_prompt("Account action")
                    .items(&account_actions)
                    .default(0)
                    .interact_opt()?;

                match sel {
                    Some(0) => {
                        let name: String = Input::new()
                            .with_prompt("Account name")
                            .interact_text()?;
                        let rpc2 = RpcClient::new(rpc.base_url())?;
                        if let Err(e) = cmd_account(AccountAction::Create { name }, rpc2, keystore_path).await {
                            println!("  {} {}", "Error:".red(), e);
                        }
                    }
                    Some(1) => {
                        let rpc2 = RpcClient::new(rpc.base_url())?;
                        if let Err(e) = cmd_account(AccountAction::List, rpc2, keystore_path).await {
                            println!("  {} {}", "Error:".red(), e);
                        }
                    }
                    Some(2) => {
                        let name: String = Input::new()
                            .with_prompt("Account name to switch to")
                            .interact_text()?;
                        let rpc2 = RpcClient::new(rpc.base_url())?;
                        if let Err(e) = cmd_account(AccountAction::Switch { name }, rpc2, keystore_path).await {
                            println!("  {} {}", "Error:".red(), e);
                        }
                    }
                    _ => continue,
                }
            }
            2 => {
                // Balance
                let rpc2 = RpcClient::new(rpc.base_url())?;
                if let Err(e) = cmd_account(AccountAction::Balance { name: None }, rpc2, keystore_path).await {
                    println!("  {} {}", "Error:".red(), e);
                }
            }
            3 => {
                // Send
                let to: String = Input::new()
                    .with_prompt("Recipient (address or contact name)")
                    .interact_text()?;
                let amount: u64 = Input::new()
                    .with_prompt("Amount (EVAP)")
                    .interact_text()?;

                let rpc2 = RpcClient::new(rpc.base_url())?;
                if let Err(e) = cmd_send(rpc2, keystore_path, contacts_path, history_path, &to, amount, false).await {
                    println!("  {} {}", "Error:".red(), e);
                }
            }
            4 => {
                // Faucet
                let rpc2 = RpcClient::new(rpc.base_url())?;
                if let Err(e) = cmd_faucet(rpc2, keystore_path).await {
                    println!("  {} {}", "Error:".red(), e);
                }
            }
            5 => {
                // Objects
                let rpc2 = RpcClient::new(rpc.base_url())?;
                if let Err(e) = cmd_objects(rpc2).await {
                    println!("  {} {}", "Error:".red(), e);
                }
            }
            6 => {
                // Energy Scan
                let rpc2 = RpcClient::new(rpc.base_url())?;
                if let Err(e) = cmd_energy(EnergyAction::Scan, rpc2, keystore_path).await {
                    println!("  {} {}", "Error:".red(), e);
                }
            }
            7 => {
                // Gas Estimate
                let rpc2 = RpcClient::new(rpc.base_url())?;
                if let Err(e) = cmd_gas(GasAction::Transfer, rpc2).await {
                    println!("  {} {}", "Error:".red(), e);
                }
            }
            8 => {
                // History
                if let Err(e) = cmd_history(HistoryAction::List { limit: 10 }, history_path) {
                    println!("  {} {}", "Error:".red(), e);
                }
            }
            9 => {
                println!("Goodbye!");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

// ──────────────────────────── Completions ─────────────────────────────────

fn cmd_completions(shell: &str) -> Result<(), Box<dyn std::error::Error>> {
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};

    let shell = match shell.to_lowercase().as_str() {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        "powershell" | "ps" => Shell::PowerShell,
        "elvish" => Shell::Elvish,
        other => return Err(format!(
            "Unsupported shell '{}'. Supported: bash, zsh, fish, powershell, elvish",
            other
        ).into()),
    };

    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "evaporchain-wallet", &mut std::io::stdout());
    Ok(())
}

// ──────────────────────────── Offline Commands ───────────────────────────

async fn cmd_offline(
    action: OfflineAction,
    rpc: RpcClient,
    keystore_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        OfflineAction::Sign { to, amount, nonce, file } => {
            validation::validate_address(&to)?;
            validation::validate_amount(amount)?;

            let keystore = load_or_create_keystore(keystore_path);
            let mgr = AccountManager::new(keystore, rpc);
            let name = require_active(&mgr)?;
            let password = prompt_password("Enter password")?;
            let signer = mgr.get_signer(&name, &password)?;

            let to_addr = parse_address(&to)?;
            let signed = OfflineSigner::sign_transfer(&signer, &to_addr, amount, nonce);
            signed.save(&file)?;

            println!("{} Signed transfer saved to {:?}", "OK".green().bold(), file);
            println!("  From:   {}", signed.from);
            println!("  To:     {}", signed.to.as_deref().unwrap_or("—"));
            println!("  Amount: {}", amount);
            println!("  Nonce:  {}", nonce);
            println!("\n  Transfer this file to an online machine and run:");
            println!("  {} offline broadcast {:?}", "wallet".bold(), file);
        }
        OfflineAction::SignRefresh { id, energy, file } => {
            validation::validate_address(&id)?;
            validation::validate_energy(energy)?;

            let keystore = load_or_create_keystore(keystore_path);
            let mgr = AccountManager::new(keystore, rpc);
            let name = require_active(&mgr)?;
            let password = prompt_password("Enter password")?;
            let signer = mgr.get_signer(&name, &password)?;

            let obj_id = parse_address(&id)?;
            let signed = OfflineSigner::sign_refresh(&signer, &obj_id, energy);
            signed.save(&file)?;

            println!("{} Signed refresh saved to {:?}", "OK".green().bold(), file);
            println!("  Object:  {}", id);
            println!("  Energy:  {}", energy);
        }
        OfflineAction::Broadcast { file } => {
            let signed = SignedTransaction::load(&file)?;
            println!("  Broadcasting {} from {:?}...", signed.tx_type, file);

            let result = Broadcaster::broadcast(&rpc, &signed).await?;
            println!("{} {}", "OK".green().bold(), result.message);
            if let Some(hash) = result.tx_hash {
                println!("  Tx Hash: {}", hash);
            }
        }
        OfflineAction::Inspect { file } => {
            let signed = SignedTransaction::load(&file)?;
            println!("{}", "Signed Transaction".bold().cyan());
            println!("  Type:      {}", signed.tx_type);
            println!("  From:      {}", signed.from);
            println!("  To:        {}", signed.to.as_deref().unwrap_or("—"));
            if let Some(amt) = signed.amount {
                println!("  Amount:    {}", amt);
            }
            println!("  Nonce:     {}", signed.nonce);
            println!("  Signed At: {}", signed.signed_at);
            println!("  Signature: {}...{}", &signed.signature[..16], &signed.signature[signed.signature.len().saturating_sub(16)..]);
            println!("  Public Key: {}...{}", &signed.public_key[..16], &signed.public_key[signed.public_key.len().saturating_sub(16)..]);
            if let Some(ref extra) = signed.extra {
                println!("  Extra:     {}", serde_json::to_string_pretty(extra)?);
            }
        }
    }
    Ok(())
}

// ──────────────────────────── Helpers ──────────────────────────────────

fn expand_path(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = dirs_home() {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

fn dirs_home() -> Option<String> {
    std::env::var("HOME").ok()
}

fn load_or_create_keystore(path: &str) -> KeyStore {
    KeyStore::load(path).unwrap_or_default()
}

fn prompt_password(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Check env var first (for scripts/CI).
    if let Ok(pass) = std::env::var("EVAPORCHAIN_PASSWORD") {
        return Ok(pass);
    }
    // Secure masked input via rpassword.
    eprint!("{}: ", prompt);
    let pass = rpassword::read_password()?;
    Ok(pass)
}

/// Wait for a transaction to be confirmed on-chain.
async fn await_confirmation(pipeline: &TxPipeline, tx_hash: &str) {
    print!("  Waiting for confirmation");
    for i in 0..30 {
        print!(".");
        if let Ok(Some(tx)) = pipeline.confirm_tx(tx_hash, 1, 0).await {
            println!();
            println!(
                "  {} Confirmed in block #{}",
                "CONFIRMED".green().bold(),
                tx.block_number
            );
            return;
        }
        if i < 29 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    println!();
    println!(
        "  {} Transaction not confirmed after 60s — check later with: {} tx {}",
        "TIMEOUT".yellow().bold(),
        "wallet".bold(),
        tx_hash
    );
}

/// Show a gas estimate for a transfer (non-fatal if node unreachable).
async fn show_gas_estimate(rpc_url: &str, label: &str) {
    let Ok(rpc) = RpcClient::new(rpc_url) else { return };
    if let Ok(est) = GasEstimator::from_rpc(&rpc).await {
        let fee = est.estimate_transfer();
        println!("  {} {}: ~{} units (base_fee={})", "Gas".yellow(), label, fee.total_fee, fee.base_fee);
    }
}

/// Record a successful tx in history (non-fatal on error).
fn record_tx(
    history_path: &str,
    tx_type: &str,
    from: &str,
    to: Option<&str>,
    amount: Option<u64>,
    tx_hash: &str,
) {
    if let Ok(mut history) = TxHistory::load_or_empty(history_path) {
        history.record_success(tx_type, from, to, amount, tx_hash);
        let _ = history.save(history_path);
    }
}

/// Get active account name or return user-friendly error.
fn require_active(mgr: &AccountManager) -> Result<String, Box<dyn std::error::Error>> {
    mgr.active_name()
        .map(|s| s.to_string())
        .ok_or_else(|| "No active account. Run: wallet account create <name>".into())
}

// ──────────────────────────── Simulate ────────────────────────────────

async fn cmd_simulate(
    action: SimulateAction,
    rpc: RpcClient,
    keystore_path: &str,
    contacts_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let keystore = load_or_create_keystore(keystore_path);
    let mgr = AccountManager::new(keystore, rpc);
    let _active = require_active(&mgr)?;
    let from_addr = mgr.active_address_hex().unwrap_or_default();

    let rpc2 = RpcClient::new(mgr.rpc().base_url())?;
    let simulator = crate::simulate::Simulator::new(rpc2);

    match action {
        SimulateAction::Send { to, amount } => {
            validation::validate_recipient(&to)?;
            validation::validate_amount(amount)?;

            // Resolve contact name
            let to_addr = if std::path::Path::new(contacts_path).exists() {
                let book = AddressBook::load(contacts_path)?;
                book.resolve(&to)
            } else {
                to.clone()
            };

            println!("{}", "Simulating Transfer...".bold().cyan());
            let result = simulator.simulate_transfer(&from_addr, &to_addr, amount).await?;

            crate::output::json_or(&result, || {
                if result.success {
                    println!("  {} {}", "PASS".green().bold(), result.summary);
                } else {
                    println!("  {} {}", "FAIL".red().bold(), result.summary);
                }
                println!();
                println!("  {}", "Fee Breakdown:".bold());
                println!("    {}", result.fee.breakdown);
                println!();

                for bc in &result.balance_changes {
                    println!("  {} {} — {} EVAP → {} EVAP (Δ {})",
                        "Balance:".bold(),
                        bc.label,
                        bc.before,
                        bc.after,
                        bc.delta
                    );
                }

                for w in &result.warnings {
                    println!("  {} {}", "⚠ Warning:".yellow(), w);
                }
                for e in &result.errors {
                    println!("  {} {}", "✗ Error:".red(), e);
                }
            });
        }
        SimulateAction::Refresh { id, energy } => {
            validation::validate_address(&id)?;
            validation::validate_energy(energy)?;

            println!("{}", "Simulating Refresh...".bold().cyan());
            let result = simulator.simulate_refresh(&from_addr, &id, energy).await?;

            crate::output::json_or(&result, || {
                if result.success {
                    println!("  {} {}", "PASS".green().bold(), result.summary);
                } else {
                    println!("  {} {}", "FAIL".red().bold(), result.summary);
                }
                println!();
                println!("  {}", "Fee Breakdown:".bold());
                println!("    {}", result.fee.breakdown);

                if let Some(ref ec) = result.energy_change {
                    println!();
                    println!("  {} {} → {} energy (Δ +{})",
                        "Energy:".bold(),
                        ec.energy_before,
                        ec.energy_after,
                        ec.delta
                    );
                }

                for w in &result.warnings {
                    println!("  {} {}", "⚠ Warning:".yellow(), w);
                }
                for e in &result.errors {
                    println!("  {} {}", "✗ Error:".red(), e);
                }
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Spending ────────────────────────────────

fn cmd_spending(action: SpendingAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::spending::{SpendingPolicy, EnforcementMode};

    let policy_path = crate::spending::default_policy_path();
    let mut policy = if policy_path.exists() {
        SpendingPolicy::load(&policy_path)?
    } else {
        SpendingPolicy::default()
    };

    match action {
        SpendingAction::Show => {
            println!("{}", "Spending Policy".bold().cyan());
            println!("  Mode:         {:?}", policy.mode);
            println!("  Per-tx limit: {}", if policy.per_tx_limit == 0 { "unlimited".to_string() } else { format!("{} EVAP", policy.per_tx_limit) });
            println!("  Daily limit:  {}", if policy.daily_limit == 0 { "unlimited".to_string() } else { format!("{} EVAP", policy.daily_limit) });
            println!("  Allowlist:    {} addresses", policy.allowlist.len());
            for addr in &policy.allowlist {
                println!("    - {}", addr);
            }
            println!("  Blocklist:    {} addresses", policy.blocklist.len());
            for addr in &policy.blocklist {
                println!("    - {}", addr);
            }
        }
        SpendingAction::SetTxLimit { amount } => {
            policy.per_tx_limit = amount;
            policy.save(&policy_path)?;
            println!("{} Per-tx limit set to {} EVAP", "✓".green(), amount);
        }
        SpendingAction::SetDailyLimit { amount } => {
            policy.daily_limit = amount;
            policy.save(&policy_path)?;
            println!("{} Daily limit set to {} EVAP", "✓".green(), amount);
        }
        SpendingAction::SetMode { mode } => {
            let m = match mode.to_lowercase().as_str() {
                "enforce" => EnforcementMode::Enforce,
                "warn" => EnforcementMode::Warn,
                "disabled" | "off" => EnforcementMode::Disabled,
                _ => return Err(format!("Invalid mode: {} (use enforce/warn/disabled)", mode).into()),
            };
            policy.mode = m;
            policy.save(&policy_path)?;
            println!("{} Mode set to {:?}", "✓".green(), m);
        }
        SpendingAction::Allow { address } => {
            policy.add_to_allowlist(&address);
            policy.save(&policy_path)?;
            println!("{} Added {} to allowlist", "✓".green(), address);
        }
        SpendingAction::Unallow { address } => {
            policy.remove_from_allowlist(&address);
            policy.save(&policy_path)?;
            println!("{} Removed {} from allowlist", "✓".green(), address);
        }
        SpendingAction::Block { address } => {
            policy.add_to_blocklist(&address);
            policy.save(&policy_path)?;
            println!("{} Added {} to blocklist", "✓".green(), address);
        }
        SpendingAction::Unblock { address } => {
            policy.remove_from_blocklist(&address);
            policy.save(&policy_path)?;
            println!("{} Removed {} from blocklist", "✓".green(), address);
        }
        SpendingAction::Status => {
            let remaining = policy.daily_remaining();
            println!("{}", "Daily Spending Status".bold().cyan());
            println!("  Date:      {}", policy.daily_spent.date);
            println!("  Spent:     {} EVAP", policy.daily_spent.spent);
            match remaining {
                Some(r) => println!("  Remaining: {} EVAP", r),
                None => println!("  Remaining: unlimited"),
            }
        }
        SpendingAction::ResetDaily => {
            policy.reset_daily();
            policy.save(&policy_path)?;
            println!("{} Daily spending counter reset", "✓".green());
        }
    }
    Ok(())
}

// ──────────────────────────── Multisig ────────────────────────────────

fn cmd_multisig(action: MultisigAction, keystore_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use crate::multisig::MultisigStore;

    let ms_path = crate::multisig::default_multisig_path();
    let mut store = if ms_path.exists() {
        MultisigStore::load(&ms_path)?
    } else {
        MultisigStore::new()
    };

    match action {
        MultisigAction::CreateGroup { name, members, threshold } => {
            let member_list: Vec<String> = members.split(',').map(|s| s.trim().to_string()).collect();
            store.create_group(&name, member_list, threshold)?;
            store.save(&ms_path)?;
            println!("{} Created multisig group '{}' ({}-of-{})",
                "✓".green(), name, threshold, members.split(',').count());
        }
        MultisigAction::Groups => {
            let groups = store.list_groups();
            if groups.is_empty() {
                println!("No multisig groups configured.");
            } else {
                println!("{}", "Multisig Groups".bold().cyan());
                for g in groups {
                    println!("  {} — {}-of-{} ({} members)",
                        g.name.bold(), g.threshold, g.members.len(), g.members.len());
                    for m in &g.members {
                        println!("    - {}", m);
                    }
                }
            }
        }
        MultisigAction::Propose { group, to, amount, memo } => {
            let config = WalletConfig::load_or_default(WalletConfig::default_path())?;
            let active_name = config.active_account
                .ok_or("No active account — run: wallet account switch <name>")?;
            let ks = KeyStore::load(keystore_path)?;
            let from_addr = ks.get_address(&active_name)
                .map(|a| crate::address::format_address(&a))
                .ok_or("Active account not found in keystore")?;

            let tx = crate::multisig::ProposedTx::Transfer {
                to: to.clone(),
                amount,
            };
            let proposal = store.propose(&group, &from_addr, tx, memo.as_deref(), 24)?;
            let prop_id = proposal.id.clone();
            let status = proposal.status;
            store.save(&ms_path)?;
            println!("{} Proposal created: {}", "✓".green(), prop_id);
            println!("  Transfer {} EVAP to {}", amount, to);
            println!("  Status: {:?}", status);
        }
        MultisigAction::Approve { id } => {
            let config = WalletConfig::load_or_default(WalletConfig::default_path())?;
            let active_name = config.active_account
                .ok_or("No active account — run: wallet account switch <name>")?;
            let ks = KeyStore::load(keystore_path)?;
            let signer_addr = ks.get_address(&active_name)
                .map(|a| crate::address::format_address(&a))
                .ok_or("Active account not found in keystore")?;

            let proposal = store.approve(&id, &signer_addr)?;
            let approvals = proposal.approvals.len();
            let status = proposal.status;
            store.save(&ms_path)?;
            println!("{} Approved proposal {}", "✓".green(), id);
            println!("  Approvals: {}", approvals);
            println!("  Status: {:?}", status);
            if status == crate::multisig::ProposalStatus::Approved {
                println!("  {} Threshold met — ready to execute!", "✓".green().bold());
            }
        }
        MultisigAction::Proposals { group } => {
            let proposals = store.list_proposals(&group);
            if proposals.is_empty() {
                println!("No proposals for group '{}'.", group);
            } else {
                println!("{}", format!("Proposals for '{}'", group).bold().cyan());
                for p in proposals {
                    println!("  {} — {:?} — {} approvals — {}",
                        p.id.bold(), p.status, p.approvals.len(), p.tx.describe());
                }
            }
        }
        MultisigAction::ShowProposal { id } => {
            let proposal = store.get_proposal(&id)
                .ok_or_else(|| format!("Proposal not found: {}", id))?;
            println!("{}", "Proposal Details".bold().cyan());
            println!("  ID:       {}", proposal.id);
            println!("  Group:    {}", proposal.group_name);
            println!("  Proposer: {}", proposal.proposer);
            println!("  TX:       {}", proposal.tx.describe());
            println!("  Status:   {:?}", proposal.status);
            println!("  Approvals: {:?}", proposal.approvals);
            println!("  Created:  {}", proposal.created_at);
            println!("  Expires:  {}", proposal.expires_at);
            if let Some(ref memo) = proposal.memo {
                println!("  Memo:     {}", memo);
            }
        }
        MultisigAction::RemoveGroup { name } => {
            store.remove_group(&name)?;
            store.save(&ms_path)?;
            println!("{} Removed group '{}'", "✓".green(), name);
        }
    }
    Ok(())
}

// ──────────────────────────── Hooks ──────────────────────────────────

fn cmd_hooks(action: HooksAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::hooks::{HookRegistry, Hook, HookAction};

    let hooks_path = crate::hooks::default_hooks_path();
    let mut registry = if hooks_path.exists() {
        HookRegistry::load(&hooks_path)?
    } else {
        HookRegistry::new()
    };

    match action {
        HooksAction::List => {
            let hooks = registry.list();
            if hooks.is_empty() {
                println!("No hooks configured.");
            } else {
                println!("{}", "Transaction Hooks".bold().cyan());
                for h in hooks {
                    let status = if h.enabled { "enabled".green() } else { "disabled".red() };
                    let block = if h.blocking { " [blocking]" } else { "" };
                    println!("  {} — {} on {} — {} {}",
                        h.name.bold(),
                        h.action.describe(),
                        h.event.label(),
                        status,
                        block,
                    );
                }
            }
        }
        HooksAction::AddShell { name, event, command, blocking } => {
            let ev = parse_hook_event(&event)?;
            registry.register(Hook {
                name: name.clone(),
                event: ev,
                action: HookAction::Shell { command },
                enabled: true,
                blocking,
            });
            registry.save(&hooks_path)?;
            println!("{} Shell hook '{}' added on {}", "✓".green(), name, event);
        }
        HooksAction::AddLog { name, event, file, format } => {
            let ev = parse_hook_event(&event)?;
            registry.register(Hook {
                name: name.clone(),
                event: ev,
                action: HookAction::Log { file, format },
                enabled: true,
                blocking: false,
            });
            registry.save(&hooks_path)?;
            println!("{} Log hook '{}' added on {}", "✓".green(), name, event);
        }
        HooksAction::Remove { name } => {
            registry.remove(&name)?;
            registry.save(&hooks_path)?;
            println!("{} Hook '{}' removed", "✓".green(), name);
        }
        HooksAction::Enable { name } => {
            registry.set_enabled(&name, true)?;
            registry.save(&hooks_path)?;
            println!("{} Hook '{}' enabled", "✓".green(), name);
        }
        HooksAction::Disable { name } => {
            registry.set_enabled(&name, false)?;
            registry.save(&hooks_path)?;
            println!("{} Hook '{}' disabled", "✓".green(), name);
        }
    }
    Ok(())
}

fn parse_hook_event(s: &str) -> Result<crate::hooks::HookEvent, Box<dyn std::error::Error>> {
    match s {
        "pre_send" => Ok(crate::hooks::HookEvent::PreSend),
        "post_send" => Ok(crate::hooks::HookEvent::PostSend),
        "pre_refresh" => Ok(crate::hooks::HookEvent::PreRefresh),
        "post_refresh" => Ok(crate::hooks::HookEvent::PostRefresh),
        "on_error" => Ok(crate::hooks::HookEvent::OnError),
        _ => Err(format!("Invalid event: {} (use pre_send/post_send/pre_refresh/post_refresh/on_error)", s).into()),
    }
}

// ──────────────────────────── Labels ──────────────────────────────────

fn cmd_labels(action: LabelsAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::labels::{LabelStore, AddressCategory};

    let labels_path = crate::labels::default_labels_path();
    let mut store = if labels_path.exists() {
        LabelStore::load(&labels_path)?
    } else {
        LabelStore::new()
    };

    match action {
        LabelsAction::Add { address, name, category, tags, note } => {
            let cat = AddressCategory::from_str(&category)
                .ok_or_else(|| format!("Invalid category: {} (use personal/exchange/defi/contract/dao/staking/nft/faucet)", category))?;
            let tag_list: Vec<String> = tags
                .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();
            store.label_address(&address, &name, cat, tag_list, note.as_deref())?;
            store.save(&labels_path)?;
            println!("{} Labeled {} as '{}' ({})", "✓".green(), address, name, category);
        }
        LabelsAction::List => {
            let labels = store.list_address_labels();
            if labels.is_empty() {
                println!("No address labels.");
            } else {
                println!("{}", "Address Labels".bold().cyan());
                for l in labels {
                    let tags = if l.tags.is_empty() { String::new() } else { format!(" [{}]", l.tags.join(", ")) };
                    println!("  {} — {} ({}){}",
                        l.name.bold(), l.address, l.category.label(), tags);
                    if let Some(ref note) = l.note {
                        println!("    Note: {}", note);
                    }
                }
            }
            // Also show tx annotations
            let anns = store.list_tx_annotations();
            if !anns.is_empty() {
                println!();
                println!("{} ({} annotations)", "Tx Annotations".bold().cyan(), anns.len());
                for a in anns.iter().rev().take(10) {
                    let tags = if a.tags.is_empty() { String::new() } else { format!(" [{}]", a.tags.join(", ")) };
                    println!("  {} — {}{}", a.tx_hash, a.note.as_deref().unwrap_or("-"), tags);
                }
            }
        }
        LabelsAction::Search { query } => {
            let results = store.search_addresses(&query);
            if results.is_empty() {
                println!("No labels matching '{}'.", query);
            } else {
                println!("{}", format!("Search: '{}' ({} results)", query, results.len()).bold().cyan());
                for l in results {
                    println!("  {} — {} ({})", l.name.bold(), l.address, l.category.label());
                }
            }
        }
        LabelsAction::Remove { address } => {
            store.remove_address_label(&address)?;
            store.save(&labels_path)?;
            println!("{} Label removed for {}", "✓".green(), address);
        }
        LabelsAction::Annotate { tx_hash, note, tags, category } => {
            let tag_list: Vec<String> = tags
                .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();
            store.annotate_tx(&tx_hash, note.as_deref(), tag_list, category.as_deref())?;
            store.save(&labels_path)?;
            println!("{} Annotated tx {}", "✓".green(), tx_hash);
        }
        LabelsAction::Annotations => {
            let anns = store.list_tx_annotations();
            if anns.is_empty() {
                println!("No transaction annotations.");
            } else {
                println!("{}", "Transaction Annotations".bold().cyan());
                for a in anns {
                    let tags = if a.tags.is_empty() { String::new() } else { format!(" [{}]", a.tags.join(", ")) };
                    let cat = a.category.as_deref().unwrap_or("-");
                    println!("  {} — {} — cat:{}{}", a.tx_hash, a.note.as_deref().unwrap_or("-"), cat, tags);
                }
            }
        }
    }
    Ok(())
}

// ──────────────────────────── Fees ───────────────────────────────────

async fn cmd_fees(action: FeesAction, rpc: RpcClient) -> Result<(), Box<dyn std::error::Error>> {
    use crate::fee_analytics::FeeTracker;

    let fee_path = crate::fee_analytics::default_fee_path();
    let mut tracker = if fee_path.exists() {
        FeeTracker::load(&fee_path)?
    } else {
        FeeTracker::default_capacity()
    };

    match action {
        FeesAction::Stats => {
            match tracker.stats() {
                Ok(stats) => {
                    crate::output::json_or(&stats, || {
                        println!("{}", "Fee Market Stats".bold().cyan());
                        println!("  Samples:     {}", stats.samples);
                        println!("  Current:     {} ({}th percentile)", stats.current, stats.current_percentile as u64);
                        println!("  Min / Max:   {} / {}", stats.min, stats.max);
                        println!("  Median:      {}", stats.median);
                        println!("  P25 / P75:   {} / {}", stats.p25, stats.p75);
                        println!("  P90:         {}", stats.p90);
                        println!("  Average:     {}", stats.avg);
                        println!("  Trend:       {:?}", stats.trend);
                    });
                }
                Err(e) => {
                    println!("{} {} — run 'wallet fees record' to collect data", "⚠".yellow(), e);
                }
            }
        }
        FeesAction::Timing => {
            match tracker.timing_advice() {
                Ok(advice) => {
                    crate::output::json_or(&advice, || {
                        println!("{}", "Fee Timing Advice".bold().cyan());
                        println!("  Level: {}", advice.level.to_uppercase());
                        println!("  {}", advice.advice);
                        if advice.potential_savings_pct > 0.0 {
                            println!("  Potential savings: ~{:.1}% by waiting", advice.potential_savings_pct);
                        }
                    });
                }
                Err(e) => {
                    println!("{} {} — run 'wallet fees record' first", "⚠".yellow(), e);
                }
            }
        }
        FeesAction::Record => {
            let blocks = rpc.get_blocks(Some(10)).await?;
            let mut recorded = 0;
            for b in &blocks {
                tracker.record_block(b.number, b.epoch, b.base_fee, b.gas_used, b.tx_count as u64);
                recorded += 1;
            }
            // Check alerts
            let triggered = tracker.check_alerts();
            tracker.save(&fee_path)?;
            println!("{} Recorded {} fee samples ({} total)", "✓".green(), recorded, tracker.len());
            for alert_name in &triggered {
                println!("  {} Fee alert '{}' triggered!", "🔔".yellow(), alert_name);
            }
        }
        FeesAction::Alert { name, target } => {
            tracker.add_alert(&name, target);
            tracker.save(&fee_path)?;
            println!("{} Fee alert '{}' set (target: ≤ {})", "✓".green(), name, target);
        }
        FeesAction::Alerts => {
            let alerts = tracker.list_alerts();
            if alerts.is_empty() {
                println!("No fee alerts configured.");
            } else {
                println!("{}", "Fee Alerts".bold().cyan());
                for a in alerts {
                    let status = if !a.enabled {
                        "disabled".red().to_string()
                    } else if a.fired {
                        "fired".yellow().to_string()
                    } else {
                        "active".green().to_string()
                    };
                    println!("  {} — target ≤ {} — {}", a.name.bold(), a.target_fee, status);
                }
            }
        }
        FeesAction::RemoveAlert { name } => {
            if tracker.remove_alert(&name) {
                tracker.save(&fee_path)?;
                println!("{} Alert '{}' removed", "✓".green(), name);
            } else {
                println!("Alert '{}' not found.", name);
            }
        }
    }
    Ok(())
}

// ──────────────────────────── Hardware ────────────────────────────────

fn cmd_hardware(action: HardwareAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::hardware::{DeviceRegistry, SimulatedDevice, HardwareWallet};

    let device_path = crate::hardware::default_device_path();
    let mut registry = if device_path.exists() {
        DeviceRegistry::load(&device_path)?
    } else {
        DeviceRegistry::new()
    };

    match action {
        HardwareAction::List => {
            let devices = registry.list();
            if devices.is_empty() {
                println!("No hardware devices registered.");
                println!("  Add a test device: wallet hardware add-simulated <name>");
            } else {
                println!("{}", "Hardware Devices".bold().cyan());
                for d in devices {
                    println!("  {} — {} ({}) addr: {}",
                        d.id.bold(),
                        d.name,
                        d.device_type.label(),
                        d.address.as_deref().unwrap_or("-"));
                }
            }
        }
        HardwareAction::AddSimulated { name } => {
            let device = SimulatedDevice::new(&name);
            let info = device.info();
            let id = info.id.clone();
            let addr = device.get_address().unwrap_or_default();
            registry.register(info, &name);
            registry.save(&device_path)?;
            println!("{} Simulated device '{}' registered", "✓".green(), name);
            println!("  ID:      {}", id);
            println!("  Address: {}", addr);
        }
        HardwareAction::Remove { id } => {
            if registry.remove(&id) {
                registry.save(&device_path)?;
                println!("{} Device '{}' removed", "✓".green(), id);
            } else {
                println!("Device '{}' not found.", id);
            }
        }
        HardwareAction::Info { id } => {
            match registry.get(&id) {
                Some(d) => {
                    println!("{}", "Device Info".bold().cyan());
                    println!("  ID:           {}", d.id);
                    println!("  Name:         {}", d.name);
                    println!("  Type:         {}", d.device_type.label());
                    println!("  Address:      {}", d.address.as_deref().unwrap_or("-"));
                    println!("  Registered:   {}", d.registered_at);
                    println!("  Last used:    {}", d.last_used.as_deref().unwrap_or("never"));
                }
                None => println!("Device '{}' not found.", id),
            }
        }
    }
    Ok(())
}

// ──────────────────────────── dApp ──────────────────────────────────

fn cmd_dapp(action: DappAction, keystore_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use crate::dapp::{DappConnector, Permission};

    let dapp_path = crate::dapp::default_dapp_path();
    let mut connector = if dapp_path.exists() {
        DappConnector::load(&dapp_path)?
    } else {
        DappConnector::new()
    };

    // Expire old sessions
    connector.expire_sessions();

    match action {
        DappAction::Sessions => {
            let sessions = connector.active_sessions();
            if sessions.is_empty() {
                println!("No active dApp sessions.");
            } else {
                println!("{}", format!("Active dApp Sessions ({})", sessions.len()).bold().cyan());
                for s in sessions {
                    let perms: Vec<&str> = s.permissions.iter().map(|p| p.label()).collect();
                    println!("  {} — {} ({})", s.id.bold(), s.name, s.origin);
                    println!("    Account:     {}", s.account);
                    println!("    Permissions: {}", perms.join(", "));
                    println!("    Requests:    {}", s.request_count);
                    println!("    Expires:     {}", s.expires_at);
                }
            }
            connector.save(&dapp_path)?;
        }
        DappAction::Connect { origin, name, permissions, hours } => {
            let perms: Vec<Permission> = permissions
                .split(',')
                .filter_map(|s| Permission::from_str(s.trim()))
                .collect();
            if perms.is_empty() {
                return Err("No valid permissions specified. Use: view_account,request_sign,view_history,view_assets,view_energy".into());
            }

            let config = WalletConfig::load_or_default(WalletConfig::default_path())?;
            let account = config.active_account
                .ok_or("No active account")?;
            let ks = KeyStore::load(keystore_path)?;
            let account_addr = ks.get_address(&account)
                .map(|a| crate::address::format_address(&a))
                .ok_or("Active account not found in keystore")?;

            let session = connector.create_session(&origin, &name, perms.clone(), &account_addr, hours)?;
            let sess_id = session.id.clone();
            connector.save(&dapp_path)?;

            println!("{} Connected to '{}' ({})", "✓".green(), name, origin);
            println!("  Session ID:    {}", sess_id);
            println!("  Permissions:   {}", permissions);
            println!("  Duration:      {} hours", hours);
        }
        DappAction::Revoke { id } => {
            connector.revoke_session(&id)?;
            connector.save(&dapp_path)?;
            println!("{} Session '{}' revoked", "✓".green(), id);
        }
        DappAction::RevokeOrigin { origin } => {
            let count = connector.revoke_origin(&origin);
            connector.save(&dapp_path)?;
            println!("{} Revoked {} session(s) for {}", "✓".green(), count, origin);
        }
        DappAction::Show { id } => {
            match connector.get_session(&id) {
                Some(s) => {
                    let perms: Vec<&str> = s.permissions.iter().map(|p| p.label()).collect();
                    println!("{}", "dApp Session".bold().cyan());
                    println!("  ID:          {}", s.id);
                    println!("  Origin:      {}", s.origin);
                    println!("  Name:        {}", s.name);
                    println!("  Account:     {}", s.account);
                    println!("  Status:      {:?}", s.status);
                    println!("  Permissions: {}", perms.join(", "));
                    println!("  Requests:    {}", s.request_count);
                    println!("  Created:     {}", s.created_at);
                    println!("  Expires:     {}", s.expires_at);

                    let reqs = connector.session_requests(&id);
                    if !reqs.is_empty() {
                        println!("  Recent requests:");
                        for r in reqs.iter().rev().take(5) {
                            let status = if r.approved == Some(true) { "✓" } else { "✗" };
                            println!("    {} {} — {}", status, r.request_type, r.timestamp);
                        }
                    }
                }
                None => println!("Session '{}' not found.", id),
            }
        }
    }
    Ok(())
}

// ── Notifications handler ──

fn cmd_notifications(action: NotificationsAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::notifications::{NotificationCenter, EventCategory};

    let data_dir = crate::config::default_data_dir();
    let path = data_dir.join("notifications.json");
    let mut center = if path.exists() {
        NotificationCenter::load(&path)?
    } else {
        NotificationCenter::new()
    };

    match action {
        NotificationsAction::Unread => {
            let unread = center.unread();
            if unread.is_empty() {
                println!("{}", "No unread notifications.".dimmed());
            } else {
                println!("{} ({})", "Unread Notifications".bold().cyan(), unread.len());
                for n in &unread {
                    let icon = n.priority.icon();
                    println!("  {} [{}] {} — {}", icon, &n.id[..8], n.title.bold(), n.message);
                }
            }
        }
        NotificationsAction::Recent { limit } => {
            let recent = center.recent(limit);
            if recent.is_empty() {
                println!("{}", "No notifications.".dimmed());
            } else {
                println!("{}", "Recent Notifications".bold().cyan());
                for n in &recent {
                    let icon = n.priority.icon();
                    let read_mark = if n.read { " " } else { "*" };
                    println!("  {}{} [{}] {} — {}", read_mark, icon, &n.id[..8], n.title.bold(), n.message);
                }
            }
        }
        NotificationsAction::Read { id } => {
            center.mark_read(&id);
            center.save(&path)?;
            println!("{} Notification marked as read.", "✓".green());
        }
        NotificationsAction::ReadAll => {
            center.mark_all_read();
            center.save(&path)?;
            println!("{} All notifications marked as read.", "✓".green());
        }
        NotificationsAction::Filter { category } => {
            let cat = match category.to_lowercase().as_str() {
                "energy_decay" | "energy" => EventCategory::EnergyDecay,
                "tx_confirmed" | "confirmed" => EventCategory::TxConfirmed,
                "tx_failed" | "failed" => EventCategory::TxFailed,
                "fee_alert" | "fee" => EventCategory::FeeAlert,
                "security" => EventCategory::Security,
                "session_expiry" | "session" => EventCategory::SessionExpiry,
                "system" => EventCategory::System,
                _ => {
                    eprintln!("{} Unknown category: {}", "Error:".red().bold(), category);
                    return Ok(());
                }
            };
            let filtered = center.filter_by_category(cat);
            if filtered.is_empty() {
                println!("{}", "No notifications in this category.".dimmed());
            } else {
                println!("{} ({} results)", "Filtered Notifications".bold().cyan(), filtered.len());
                for n in &filtered {
                    let icon = n.priority.icon();
                    println!("  {} [{}] {} — {}", icon, &n.id[..8], n.title.bold(), n.message);
                }
            }
        }
        NotificationsAction::Clear => {
            center.clear_history();
            center.save(&path)?;
            println!("{} Notification history cleared.", "✓".green());
        }
        NotificationsAction::Count => {
            let total = center.len();
            let unread = center.unread_count();
            println!("{}", "Notification Stats".bold().cyan());
            println!("  Total:  {}", total);
            println!("  Unread: {}", unread);
        }
    }
    Ok(())
}

// ── Session Keys handler ──

fn cmd_session_keys(action: SessionKeysAction, keystore_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use crate::session_keys::AbstractionStore;

    let data_dir = crate::config::default_data_dir();
    let path = data_dir.join("abstraction.json");
    let mut store = if path.exists() {
        AbstractionStore::load(&path)?
    } else {
        AbstractionStore::new()
    };

    match action {
        SessionKeysAction::List => {
            let keys = store.active_session_keys();
            if keys.is_empty() {
                println!("{}", "No active session keys.".dimmed());
            } else {
                println!("{} ({} active)", "Session Keys".bold().cyan(), keys.len());
                for k in &keys {
                    println!("  {} {} — max/tx: {}, ops: {:?}", k.id[..8].dimmed(), k.label.bold(), k.max_per_tx, k.allowed_ops);
                }
            }
        }
        SessionKeysAction::Create { label, max_per_tx, total_limit, ops, hours } => {
            let config = WalletConfig::load_or_default(WalletConfig::default_path())?;
            let active_name = config.active_account.as_ref().ok_or("No active account")?;
            let ks = KeyStore::load(keystore_path)?;
            let addr = ks.get_address(active_name).ok_or("Account not found in keystore")?;
            let addr_hex = format!("0x{}", hex::encode(addr));

            let allowed: Vec<String> = ops.split(',').map(|s| s.trim().to_string()).collect();
            let key = store.create_session_key(&label, &addr_hex, max_per_tx, total_limit, allowed, hours);
            let key_id = key.id.clone();
            let key_label = key.label.clone();
            let key_max = key.max_per_tx;
            let key_exp = key.expires_at.clone();
            store.save(&path)?;
            println!("{} Session key created: {}", "✓".green(), key_id);
            println!("  Label:    {}", key_label);
            println!("  Max/tx:   {}", key_max);
            println!("  Expires:  {}", key_exp);
        }
        SessionKeysAction::Revoke { id } => {
            store.revoke_session_key(&id)?;
            store.save(&path)?;
            println!("{} Session key revoked.", "✓".green());
        }
        SessionKeysAction::Show { id } => {
            match store.get_session_key(&id) {
                Some(k) => {
                    println!("{}", "Session Key".bold().cyan());
                    println!("  ID:       {}", k.id);
                    println!("  Label:    {}", k.label);
                    println!("  Account:  {}", k.account);
                    println!("  Max/tx:   {}", k.max_per_tx);
                    println!("  Used:     {}", k.total_spent);
                    println!("  Ops:      {:?}", k.allowed_ops);
                    println!("  Active:   {}", k.active);
                    println!("  Expires:  {}", k.expires_at);
                }
                None => eprintln!("{} Session key not found: {}", "Error:".red().bold(), id),
            }
        }
        SessionKeysAction::SetupRecovery { threshold, delay_hours } => {
            let config = WalletConfig::load_or_default(WalletConfig::default_path())?;
            let active_name = config.active_account.as_ref().ok_or("No active account")?;
            let ks = KeyStore::load(keystore_path)?;
            let addr = ks.get_address(active_name).ok_or("Account not found in keystore")?;
            let addr_hex = format!("0x{}", hex::encode(addr));

            store.setup_recovery(&addr_hex, threshold, delay_hours)?;
            store.save(&path)?;
            println!("{} Social recovery configured.", "✓".green());
            println!("  Threshold:   {}", threshold);
            println!("  Delay:       {}h", delay_hours);
        }
        SessionKeysAction::AddGuardian { address, name } => {
            let recovery = store.recovery.as_mut().ok_or("Social recovery not set up. Run: wallet session-keys setup-recovery <threshold>")?;
            recovery.add_guardian(&address, &name)?;
            store.save(&path)?;
            println!("{} Guardian added: {} ({})", "✓".green(), name, address);
        }
        SessionKeysAction::RecoveryInfo => {
            match &store.recovery {
                Some(r) => {
                    println!("{}", "Social Recovery".bold().cyan());
                    println!("  Account:   {}", r.account);
                    println!("  Threshold: {}/{}", r.threshold, r.guardian_count());
                    println!("  Delay:     {}h", r.delay_hours);
                    if r.guardians.is_empty() {
                        println!("  Guardians: (none)");
                    } else {
                        println!("  Guardians:");
                        for g in &r.guardians {
                            println!("    {} — {}", g.name, g.address);
                        }
                    }
                }
                None => println!("{}", "Social recovery not configured.".dimmed()),
            }
        }
        SessionKeysAction::SetSponsor { address, max_gas, daily_budget } => {
            store.setup_sponsor(&address, max_gas, daily_budget);
            store.save(&path)?;
            println!("{} Gas sponsor configured.", "✓".green());
            println!("  Sponsor:      {}", address);
            println!("  Max gas/tx:   {}", max_gas);
            println!("  Daily budget: {}", daily_budget);
        }
        SessionKeysAction::SponsorInfo => {
            match store.gas_sponsor.as_mut() {
                Some(s) => {
                    let remaining = s.remaining();
                    println!("{}", "Gas Sponsor".bold().cyan());
                    println!("  Sponsor:      {}", s.sponsor_address);
                    println!("  Max gas/tx:   {}", s.max_gas_per_tx);
                    println!("  Daily budget: {}", s.daily_budget);
                    println!("  Remaining:    {}", remaining);
                }
                None => println!("{}", "No gas sponsor configured.".dimmed()),
            }
        }
    }
    Ok(())
}

// ── Bridge handler ──

fn cmd_bridge(action: BridgeAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::bridge::{BridgeManager, ChainId, Bridge, BridgeType};

    let path = crate::bridge::default_bridge_path();
    let mut mgr = if path.exists() {
        BridgeManager::load(&path)?
    } else {
        BridgeManager::new()
    };

    match action {
        BridgeAction::List => {
            let bridges = mgr.list_bridges();
            if bridges.is_empty() {
                println!("{}", "No bridges registered.".dimmed());
            } else {
                println!("{} ({} bridges)", "Cross-Chain Bridges".bold().cyan(), bridges.len());
                for b in bridges {
                    println!("  {} {} → {} ({:?}) fee: {}%",
                        b.id[..8].dimmed(), b.source_chain.label(), b.dest_chain.label(),
                        b.bridge_type, b.fee_pct);
                }
            }
        }
        BridgeAction::Find { source, dest } => {
            let src = ChainId::from_str(&source);
            let dst = ChainId::from_str(&dest);
            let found = mgr.find_bridges(&src, &dst);
            if found.is_empty() {
                println!("{}", "No bridges found for this route.".dimmed());
            } else {
                println!("{} ({} results)", "Bridges Found".bold().cyan(), found.len());
                for b in &found {
                    println!("  {} {} — fee: {}%, ~{}min",
                        b.id[..8].dimmed(), b.name, b.fee_pct, b.estimated_time_min);
                }
            }
        }
        BridgeAction::Register { name, source, dest, bridge_type, fee_pct } => {
            let bt = match bridge_type.to_lowercase().as_str() {
                "lock_mint" | "lockmint" => BridgeType::LockMint,
                "burn_mint" | "burnmint" => BridgeType::BurnMint,
                "liquidity_pool" | "liquiditypool" | "lp" => BridgeType::LiquidityPool,
                "native" => BridgeType::Native,
                _ => BridgeType::LockMint,
            };
            let bridge = Bridge {
                id: format!("br_{}", &blake3::hash(format!("{}{}{}", name, source, dest).as_bytes()).to_hex()[..12]),
                name,
                source_chain: ChainId::from_str(&source),
                dest_chain: ChainId::from_str(&dest),
                bridge_type: bt,
                source_contract: String::new(),
                dest_contract: String::new(),
                supported_tokens: vec!["EVAP".to_string()],
                estimated_time_min: 15,
                fee_pct,
                active: true,
                added_at: chrono::Utc::now().to_rfc3339(),
            };
            let id = bridge.id.clone();
            mgr.register_bridge(bridge)?;
            mgr.save(&path)?;
            println!("{} Bridge registered: {}", "✓".green(), id);
        }
        BridgeAction::Transfer { bridge_id, token, amount, sender, recipient } => {
            let transfer = mgr.initiate_transfer(&bridge_id, &token, amount, &sender, &recipient)?;
            let tid = transfer.id.clone();
            mgr.save(&path)?;
            println!("{} Bridge transfer initiated: {}", "✓".green(), tid);
            println!("  Token:     {} {}", amount, token);
            println!("  From:      {}", sender);
            println!("  To:        {}", recipient);
        }
        BridgeAction::Pending => {
            let pending = mgr.pending_transfers();
            if pending.is_empty() {
                println!("{}", "No pending bridge transfers.".dimmed());
            } else {
                println!("{} ({} pending)", "Pending Transfers".bold().cyan(), pending.len());
                for t in &pending {
                    println!("  {} {} {} — {:?}",
                        t.id[..8].dimmed(), t.amount, t.token, t.status);
                }
            }
        }
        BridgeAction::Show { id } => {
            match mgr.get_transfer(&id) {
                Some(t) => {
                    println!("{}", "Bridge Transfer".bold().cyan());
                    println!("  ID:       {}", t.id);
                    println!("  Bridge:   {}", t.bridge_id);
                    println!("  Token:    {} {}", t.amount, t.token);
                    println!("  Sender:   {}", t.sender);
                    println!("  Recipient:{}", t.recipient);
                    println!("  Status:   {:?}", t.status);
                    println!("  Created:  {}", t.created_at);
                }
                None => eprintln!("{} Transfer not found: {}", "Error:".red().bold(), id),
            }
        }
        BridgeAction::Remove { id } => {
            mgr.remove_bridge(&id)?;
            mgr.save(&path)?;
            println!("{} Bridge removed.", "✓".green());
        }
    }
    Ok(())
}

// ── Language handler ──

fn cmd_lang(action: LangAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::i18n::{I18n, MsgKey};

    let data_dir = crate::config::default_data_dir();
    let locale_path = data_dir.join("locale.txt");

    // Load saved locale or detect from env
    let mut i18n = if locale_path.exists() {
        let saved = std::fs::read_to_string(&locale_path)?;
        let mut engine = I18n::new();
        let _ = engine.set_locale_str(saved.trim());
        engine
    } else {
        I18n::from_env()
    };

    match action {
        LangAction::Show => {
            let locale = i18n.locale();
            println!("{}", "Current Locale".bold().cyan());
            println!("  Code:   {}", locale.code());
            println!("  Name:   {}", locale.native_name());
            println!("  Sample: {}", i18n.get(MsgKey::Welcome));
        }
        LangAction::Set { locale } => {
            i18n.set_locale_str(&locale)?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::write(&locale_path, i18n.locale().code())?;
            println!("{} Locale set to {} ({})", "✓".green(), i18n.locale().code(), i18n.locale().native_name());
            println!("  {}", i18n.get(MsgKey::Welcome));
        }
        LangAction::List => {
            println!("{}", "Supported Locales".bold().cyan());
            for (locale, name) in i18n.supported_locales() {
                let current = if locale == i18n.locale() { " (current)" } else { "" };
                let pct = i18n.completeness(locale);
                println!("  {} — {} [{:.0}%]{}", locale.code(), name, pct, current);
            }
        }
        LangAction::Test { key } => {
            let msg_key = match key.to_lowercase().as_str() {
                "welcome" => MsgKey::Welcome,
                "success" => MsgKey::Success,
                "error" => MsgKey::Error,
                "confirm" => MsgKey::Confirm,
                "cancel" => MsgKey::Cancel,
                "loading" => MsgKey::Loading,
                "done" => MsgKey::Done,
                "no_active" | "no_active_account" => MsgKey::NoActiveAccount,
                "energy_low" => MsgKey::EnergyLow,
                "energy_critical" => MsgKey::EnergyCritical,
                _ => {
                    eprintln!("{} Unknown message key: {}", "Error:".red().bold(), key);
                    return Ok(());
                }
            };
            println!("{} [{}] {}", "Message".bold().cyan(), i18n.locale().code(), i18n.get(msg_key));
        }
    }
    Ok(())
}

// ── Templates handler ──

fn cmd_templates(action: TemplatesAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::templates::{TemplateStore, Frequency};

    let path = crate::templates::default_templates_path();
    let mut store = if path.exists() {
        TemplateStore::load(&path)?
    } else {
        TemplateStore::new()
    };

    match action {
        TemplatesAction::List => {
            let templates = store.list();
            if templates.is_empty() {
                println!("{}", "No templates saved.".dimmed());
            } else {
                println!("{} ({} templates)", "Transaction Templates".bold().cyan(), templates.len());
                for t in templates {
                    let status = if t.enabled { "active".green() } else { "disabled".dimmed() };
                    println!("  {} [{}] {} — {} ({}x executed, {})",
                        t.name.bold(), t.tx_type.label(), t.description,
                        t.frequency.label(), t.exec_count, status);
                }
            }
        }
        TemplatesAction::CreateTransfer { name, to, amount, frequency } => {
            let freq = Frequency::from_str(&frequency)?;
            let tmpl = store.create_transfer(&name, &to, amount, freq)?;
            let tname = tmpl.name.clone();
            let tdesc = tmpl.description.clone();
            store.save(&path)?;
            println!("{} Template created: {}", "✓".green(), tname);
            println!("  {}", tdesc);
        }
        TemplatesAction::CreateRefresh { name, object_id, energy, frequency } => {
            let freq = Frequency::from_str(&frequency)?;
            let tmpl = store.create_refresh(&name, &object_id, energy, freq)?;
            let tname = tmpl.name.clone();
            let tdesc = tmpl.description.clone();
            store.save(&path)?;
            println!("{} Template created: {}", "✓".green(), tname);
            println!("  {}", tdesc);
        }
        TemplatesAction::Show { name } => {
            match store.get(&name) {
                Some(t) => {
                    println!("{}", "Template".bold().cyan());
                    println!("  Name:      {}", t.name);
                    println!("  Type:      {}", t.tx_type.label());
                    println!("  Desc:      {}", t.description);
                    println!("  Frequency: {}", t.frequency.label());
                    println!("  Enabled:   {}", t.enabled);
                    println!("  Executed:  {}x", t.exec_count);
                    if let Some(ref last) = t.last_executed {
                        println!("  Last exec: {}", last);
                    }
                    if let Some(ref next) = t.next_execution {
                        println!("  Next exec: {}", next);
                    }
                    println!("  Params:");
                    for (k, v) in &t.params {
                        println!("    {}: {}", k, v);
                    }
                }
                None => eprintln!("{} Template not found: {}", "Error:".red().bold(), name),
            }
        }
        TemplatesAction::Remove { name } => {
            store.remove(&name)?;
            store.save(&path)?;
            println!("{} Template removed.", "✓".green());
        }
        TemplatesAction::Enable { name } => {
            store.enable(&name)?;
            store.save(&path)?;
            println!("{} Template enabled.", "✓".green());
        }
        TemplatesAction::Disable { name } => {
            store.disable(&name)?;
            store.save(&path)?;
            println!("{} Template disabled.", "✓".green());
        }
        TemplatesAction::Execute { name } => {
            store.record_execution(&name)?;
            store.save(&path)?;
            println!("{} Template '{}' marked as executed.", "✓".green(), name);
        }
        TemplatesAction::Due => {
            let due = store.due();
            if due.is_empty() {
                println!("{}", "No templates due for execution.".dimmed());
            } else {
                println!("{} ({} due)", "Due Templates".bold().yellow(), due.len());
                for t in &due {
                    println!("  {} — {}", t.name.bold(), t.description);
                }
            }
        }
        TemplatesAction::Recurring => {
            let recurring = store.recurring();
            if recurring.is_empty() {
                println!("{}", "No recurring templates.".dimmed());
            } else {
                println!("{} ({} recurring)", "Recurring Templates".bold().cyan(), recurring.len());
                for t in &recurring {
                    println!("  {} — {} ({})", t.name.bold(), t.description, t.frequency.label());
                }
            }
        }
        TemplatesAction::Search { query } => {
            let results = store.search(&query);
            if results.is_empty() {
                println!("{}", "No matching templates.".dimmed());
            } else {
                println!("{} ({} results)", "Search Results".bold().cyan(), results.len());
                for t in &results {
                    println!("  {} — {}", t.name.bold(), t.description);
                }
            }
        }
    }
    Ok(())
}

// ── Analytics handler ──

fn cmd_analytics(action: AnalyticsAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::analytics::{AnalyticsTracker, Period, EventType};

    let path = crate::analytics::default_analytics_path();
    let mut tracker = if path.exists() {
        AnalyticsTracker::load(&path)?
    } else {
        AnalyticsTracker::new()
    };

    match action {
        AnalyticsAction::Summary { period } => {
            let p = Period::from_str(&period)?;
            let s = tracker.summarize(p);
            println!("{} ({})", "Portfolio Summary".bold().cyan(), p.label());
            println!("  Events:    {}", s.events);
            println!("  Inflow:    {} EVAP", s.total_inflow);
            println!("  Outflow:   {} EVAP", s.total_outflow);
            println!("  Net flow:  {} EVAP", s.net_flow);
            println!("  Energy:    {} EVAP spent", s.energy_spent);
            println!("  Gas fees:  {} EVAP", s.gas_spent);
            println!("  Transfers: {}", s.transfer_count);
            if s.largest_transfer > 0 {
                println!("  Largest:   {} EVAP", s.largest_transfer);
            }
        }
        AnalyticsAction::Breakdown { period } => {
            let p = Period::from_str(&period)?;
            let bd = tracker.breakdown(p);
            if bd.is_empty() {
                println!("{}", "No data for this period.".dimmed());
            } else {
                println!("{} ({})", "Spending Breakdown".bold().cyan(), p.label());
                for b in &bd {
                    println!("  {:16} {:>8} EVAP ({:>5.1}%) [{} txns]",
                        b.category, b.total_amount, b.percentage, b.count);
                }
            }
        }
        AnalyticsAction::Trend { period } => {
            let p = Period::from_str(&period)?;
            let t = tracker.trend(p);
            println!("{} ({} vs prev {})", "Trend Report".bold().cyan(), p.label(), p.label());
            println!("  Outflow:  {:+.1}%", t.outflow_change_pct);
            println!("  Inflow:   {:+.1}%", t.inflow_change_pct);
            println!("  Energy:   {:+.1}%", t.energy_change_pct);
            println!("  Volume:   {:+.1}%", t.volume_change_pct);
            println!("  Current:  {} events, net {} EVAP", t.current.events, t.current.net_flow);
            println!("  Previous: {} events, net {} EVAP", t.previous.events, t.previous.net_flow);
        }
        AnalyticsAction::Record { event, amount, balance, reference } => {
            let ev = match event.to_lowercase().as_str() {
                "transfer_out" | "out" => EventType::TransferOut,
                "transfer_in" | "in" => EventType::TransferIn,
                "energy_spend" | "energy" => EventType::EnergySpend,
                "stake" | "stake_deposit" => EventType::StakeDeposit,
                "unstake" | "stake_withdraw" => EventType::StakeWithdraw,
                "reward" | "stake_reward" => EventType::StakeReward,
                "faucet" => EventType::FaucetReceive,
                "gas" | "gas_fee" => EventType::GasFee,
                "nft_mint" => EventType::NftMint,
                "token_deploy" => EventType::TokenDeploy,
                "bridge_out" => EventType::BridgeOut,
                "bridge_in" => EventType::BridgeIn,
                _ => {
                    eprintln!("{} Unknown event type: {}", "Error:".red().bold(), event);
                    return Ok(());
                }
            };
            tracker.record(ev, amount, balance, &reference);
            tracker.save(&path)?;
            println!("{} Event recorded.", "✓".green());
        }
        AnalyticsAction::Balance => {
            match tracker.latest_balance() {
                Some(b) => println!("Latest tracked balance: {} EVAP", b),
                None => println!("{}", "No analytics data yet.".dimmed()),
            }
        }
        AnalyticsAction::Clear => {
            tracker.clear();
            tracker.save(&path)?;
            println!("{} Analytics data cleared.", "✓".green());
        }
    }
    Ok(())
}

// ── Reputation handler ──

fn cmd_reputation(action: ReputationAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::reputation::{ReputationStore, RiskFlag};

    let path = crate::reputation::default_reputation_path();
    let mut store = if path.exists() {
        ReputationStore::load(&path)?
    } else {
        ReputationStore::new()
    };

    match action {
        ReputationAction::Check { address } => {
            let assessment = store.assess(&address);
            let icon = assessment.trust_level.icon();
            println!("{} {} {}", icon, "Risk Assessment".bold().cyan(), address);
            println!("  Trust:  {} ({})", assessment.trust_level.label(), assessment.trust_level.icon());
            println!("  Score:  {}/100", assessment.risk_score);
            println!("  Block:  {}", if assessment.should_block { "YES".red().bold().to_string() } else { "no".to_string() });
            println!("  Warn:   {}", if assessment.should_warn { "yes".yellow().to_string() } else { "no".to_string() });
            if !assessment.warnings.is_empty() {
                println!("  Warnings:");
                for w in &assessment.warnings {
                    println!("    - {}", w);
                }
            }
        }
        ReputationAction::Flag { address, flag, note } => {
            let rf = match flag.to_lowercase().as_str() {
                "scam" => RiskFlag::Scam,
                "phishing" => RiskFlag::Phishing,
                "fresh_wallet" | "fresh" => RiskFlag::FreshWallet,
                "dust_attack" | "dust" => RiskFlag::DustAttack,
                "tainted" | "tainted_funds" => RiskFlag::TaintedFunds,
                "unverified" | "unverified_contract" => RiskFlag::UnverifiedContract,
                "mixer" | "tumbler" => RiskFlag::Mixer,
                "community" | "community_report" => RiskFlag::CommunityReport,
                other => RiskFlag::Custom(other.to_string()),
            };
            store.flag(&address, rf, note.as_deref())?;
            store.save(&path)?;
            println!("{} Address flagged.", "✓".green());
        }
        ReputationAction::Unflag { address, flag } => {
            let rf = match flag.to_lowercase().as_str() {
                "scam" => RiskFlag::Scam,
                "phishing" => RiskFlag::Phishing,
                "fresh_wallet" | "fresh" => RiskFlag::FreshWallet,
                "dust_attack" | "dust" => RiskFlag::DustAttack,
                "tainted" | "tainted_funds" => RiskFlag::TaintedFunds,
                "unverified" | "unverified_contract" => RiskFlag::UnverifiedContract,
                "mixer" | "tumbler" => RiskFlag::Mixer,
                "community" | "community_report" => RiskFlag::CommunityReport,
                other => RiskFlag::Custom(other.to_string()),
            };
            store.unflag(&address, &rf)?;
            store.save(&path)?;
            println!("{} Flag removed.", "✓".green());
        }
        ReputationAction::Verify { address, label } => {
            store.verify(&address, label.as_deref());
            store.save(&path)?;
            println!("{} Address verified.", "✓".green());
        }
        ReputationAction::Dangerous => {
            let dangerous = store.dangerous();
            if dangerous.is_empty() {
                println!("{}", "No dangerous addresses flagged.".dimmed());
            } else {
                println!("{} ({} addresses)", "Dangerous Addresses".bold().red(), dangerous.len());
                for r in &dangerous {
                    let label = r.label.as_deref().unwrap_or("—");
                    println!("  {} {} [score: {}] flags: {}",
                        r.address, label, r.risk_score,
                        r.flags.iter().map(|f| f.label()).collect::<Vec<_>>().join(", "));
                }
            }
        }
        ReputationAction::Verified => {
            let verified = store.verified();
            if verified.is_empty() {
                println!("{}", "No verified addresses.".dimmed());
            } else {
                println!("{} ({} addresses)", "Verified Addresses".bold().green(), verified.len());
                for r in &verified {
                    let label = r.label.as_deref().unwrap_or("—");
                    println!("  {} — {}", r.address, label);
                }
            }
        }
        ReputationAction::Search { query } => {
            let results = store.search(&query);
            if results.is_empty() {
                println!("{}", "No matching addresses.".dimmed());
            } else {
                println!("{} ({} results)", "Reputation Search".bold().cyan(), results.len());
                for r in &results {
                    println!("  {} {} [{}] score: {}",
                        r.trust_level.icon(), r.address, r.trust_level.label(), r.risk_score);
                }
            }
        }
        ReputationAction::Thresholds { block, warn } => {
            if let Some(b) = block { store.set_block_threshold(b); }
            if let Some(w) = warn { store.set_warn_threshold(w); }
            store.save(&path)?;
            println!("{} Thresholds updated: block={}, warn={}", "✓".green(), store.block_threshold, store.warn_threshold);
        }
    }
    Ok(())
}

// ── Watchtower handler ──

fn cmd_watchtower(action: WatchtowerAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::watchtower::{Watchtower, WatchTarget, Condition, WatchAction};

    let path = crate::watchtower::default_watchtower_path();
    let mut wt = if path.exists() {
        Watchtower::load(&path)?
    } else {
        Watchtower::new()
    };

    match action {
        WatchtowerAction::List => {
            let watches = wt.list();
            if watches.is_empty() {
                println!("{}", "No watches configured.".dimmed());
            } else {
                println!("{} ({} watches)", "Watchtower".bold().cyan(), watches.len());
                for w in watches {
                    let status = if w.enabled { "active".green() } else { "disabled".dimmed() };
                    println!("  {} {} — {} ({}s interval, {}x triggered, {})",
                        w.name.bold(), w.target.label(), w.condition.label(),
                        w.interval_secs, w.trigger_count, status);
                }
            }
        }
        WatchtowerAction::WatchBalance { name, address, threshold, interval } => {
            let w = wt.add_watch(
                &name,
                WatchTarget::Balance { address: address.clone() },
                Condition::Below(threshold),
                WatchAction::Notify,
                interval,
            )?;
            let wname = w.name.clone();
            wt.save(&path)?;
            println!("{} Watch created: {} (alert when balance < {})", "✓".green(), wname, threshold);
        }
        WatchtowerAction::WatchEnergy { name, object_id, threshold, interval, auto_refresh } => {
            let action = if auto_refresh > 0 {
                WatchAction::AutoRefresh { energy: auto_refresh }
            } else {
                WatchAction::Notify
            };
            let w = wt.add_watch(
                &name,
                WatchTarget::Energy { object_id: object_id.clone() },
                Condition::Below(threshold),
                action,
                interval,
            )?;
            let wname = w.name.clone();
            wt.save(&path)?;
            println!("{} Watch created: {} (alert when energy < {}%)", "✓".green(), wname, threshold);
        }
        WatchtowerAction::WatchBridge { name, transfer_id, interval } => {
            let w = wt.add_watch(
                &name,
                WatchTarget::BridgeTransfer { transfer_id: transfer_id.clone() },
                Condition::StatusEquals("completed".into()),
                WatchAction::Notify,
                interval,
            )?;
            let wname = w.name.clone();
            wt.save(&path)?;
            println!("{} Watch created: {} (alert when bridge completes)", "✓".green(), wname);
        }
        WatchtowerAction::Remove { name } => {
            wt.remove_watch(&name)?;
            wt.save(&path)?;
            println!("{} Watch removed.", "✓".green());
        }
        WatchtowerAction::Enable { name } => {
            wt.enable(&name)?;
            wt.save(&path)?;
            println!("{} Watch enabled.", "✓".green());
        }
        WatchtowerAction::Disable { name } => {
            wt.disable(&name)?;
            wt.save(&path)?;
            println!("{} Watch disabled.", "✓".green());
        }
        WatchtowerAction::Show { name } => {
            match wt.get(&name) {
                Some(w) => {
                    println!("{}", "Watch Details".bold().cyan());
                    println!("  ID:        {}", w.id);
                    println!("  Name:      {}", w.name);
                    println!("  Target:    {}", w.target.label());
                    println!("  Condition: {}", w.condition.label());
                    println!("  Action:    {}", w.action.label());
                    println!("  Enabled:   {}", w.enabled);
                    println!("  Interval:  {}s", w.interval_secs);
                    println!("  Triggered: {}x", w.trigger_count);
                    if let Some(ref v) = w.last_value {
                        println!("  Last val:  {}", v);
                    }
                    if let Some(ref t) = w.last_triggered {
                        println!("  Last fire: {}", t);
                    }
                }
                None => eprintln!("{} Watch not found: {}", "Error:".red().bold(), name),
            }
        }
        WatchtowerAction::Alerts { limit } => {
            let alerts = wt.recent_alerts(limit);
            if alerts.is_empty() {
                println!("{}", "No alerts fired.".dimmed());
            } else {
                println!("{} ({} alerts)", "Recent Alerts".bold().yellow(), alerts.len());
                for a in &alerts {
                    println!("  [{}] {} — {} ({})",
                        &a.timestamp[..19], a.watch_name.bold(), a.condition, a.action);
                }
            }
        }
        WatchtowerAction::ClearAlerts => {
            wt.clear_alerts();
            wt.save(&path)?;
            println!("{} Alert history cleared.", "✓".green());
        }
        WatchtowerAction::Status => {
            println!("{}", "Watchtower Status".bold().cyan());
            println!("  Total watches: {}", wt.watch_count());
            println!("  Active:        {}", wt.active().len());
            println!("  Due now:       {}", wt.due_watches().len());
            println!("  Total alerts:  {}", wt.alert_count());
        }
    }
    Ok(())
}

// ── Audit handler ──

fn cmd_audit(action: AuditAction2) -> Result<(), Box<dyn std::error::Error>> {
    use crate::audit_log::{AuditLog, Severity};

    let path = crate::audit_log::default_audit_path();
    let log = if path.exists() {
        AuditLog::load(&path)?
    } else {
        AuditLog::new()
    };

    match action {
        AuditAction2::Recent { limit } => {
            let recent = log.recent(limit);
            if recent.is_empty() {
                println!("{}", "No audit entries.".dimmed());
            } else {
                println!("{} ({} entries)", "Audit Log".bold().cyan(), log.len());
                for e in &recent {
                    let sev = match e.severity {
                        Severity::Info => "INFO".dimmed(),
                        Severity::Warning => "WARN".yellow(),
                        Severity::Critical => "CRIT".red().bold(),
                    };
                    println!("  [{}] {} {} — {} ({})",
                        &e.timestamp[..19], sev, e.action.label(), e.description, e.account);
                }
            }
        }
        AuditAction2::Verify => {
            match log.verify_chain() {
                Ok(()) => println!("{} Audit chain integrity verified ({} entries).", "✓".green(), log.len()),
                Err(e) => println!("{} {}", "INTEGRITY VIOLATION:".red().bold(), e),
            }
        }
        AuditAction2::Search { query } => {
            let results = log.search(&query);
            if results.is_empty() {
                println!("{}", "No matching entries.".dimmed());
            } else {
                println!("{} ({} results)", "Audit Search".bold().cyan(), results.len());
                for e in &results {
                    println!("  [{}] {} — {}", &e.timestamp[..19], e.action.label(), e.description);
                }
            }
        }
        AuditAction2::Filter { severity } => {
            let min = match severity.to_lowercase().as_str() {
                "info" => Severity::Info,
                "warning" | "warn" => Severity::Warning,
                "critical" | "crit" => Severity::Critical,
                _ => {
                    eprintln!("{} Unknown severity: {}", "Error:".red().bold(), severity);
                    return Ok(());
                }
            };
            let filtered = log.filter_severity(min);
            println!("{} ({} entries >= {:?})", "Audit Filter".bold().cyan(), filtered.len(), min);
            for e in &filtered {
                println!("  [{}] {:?} {} — {}", &e.timestamp[..19], e.severity, e.action.label(), e.description);
            }
        }
        AuditAction2::Export { file } => {
            let csv = log.to_csv();
            std::fs::write(&file, &csv)?;
            println!("{} Audit log exported to {} ({} entries)", "✓".green(), file.display(), log.len());
        }
        AuditAction2::Stats => {
            println!("{}", "Audit Stats".bold().cyan());
            println!("  Total entries: {}", log.len());
            if let Some(latest) = log.latest() {
                println!("  Latest:       {} — {}", &latest.timestamp[..19], latest.action.label());
                println!("  Chain hash:   {}...", &latest.hash[..16]);
            }
            let info_count = log.filter_severity(Severity::Info).len();
            let warn_count = log.filter_severity(Severity::Warning).len();
            let crit_count = log.filter_severity(Severity::Critical).len();
            println!("  Info:         {}", info_count);
            println!("  Warnings:     {}", warn_count - crit_count);
            println!("  Critical:     {}", crit_count);
        }
    }
    Ok(())
}

// ── Tax handler ──

fn cmd_tax(action: TaxAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::tax::{TaxTracker, CostBasisMethod};

    let path = crate::tax::default_tax_path();
    let mut tracker = if path.exists() {
        TaxTracker::load(&path)?
    } else {
        TaxTracker::new(CostBasisMethod::Fifo)
    };

    match action {
        TaxAction::Acquire { amount, cost, source, reference } => {
            tracker.acquire(amount, cost, &source, &reference);
            tracker.save(&path)?;
            println!("{} Acquired {} tokens @ {:.4}/unit ({})",
                "✓".green(), amount, cost, source);
        }
        TaxAction::Dispose { amount, proceeds, disposal_type, reference } => {
            let d = tracker.dispose(amount, proceeds, &disposal_type, &reference)?;
            tracker.save(&path)?;
            println!("{} Disposed {} tokens @ {:.4}/unit", "✓".green(), amount, proceeds);
            println!("  Proceeds:   {:.2}", d.total_proceeds);
            println!("  Cost basis: {:.2}", d.cost_basis);
            println!("  Gain/Loss:  {:.2}{}", d.gain_loss,
                if d.gain_loss >= 0.0 { " (gain)".green().to_string() } else { " (loss)".red().to_string() });
            println!("  Term:       {}", if d.long_term { "long-term" } else { "short-term" });
        }
        TaxAction::Lots => {
            if tracker.lots.is_empty() {
                println!("{}", "No open lots.".dimmed());
            } else {
                println!("{} ({} lots, {} tokens)", "Open Lots".bold().cyan(),
                    tracker.lots.len(), tracker.total_holdings());
                for l in &tracker.lots {
                    println!("  {} — {} remaining (of {}) @ {:.4}/unit [{}]",
                        &l.acquired_at[..10], l.amount, l.original_amount,
                        l.cost_per_unit, l.source);
                }
                println!("  Total cost basis: {:.2}", tracker.total_cost_basis());
            }
        }
        TaxAction::Disposals => {
            if tracker.disposals.is_empty() {
                println!("{}", "No disposals recorded.".dimmed());
            } else {
                println!("{} ({} disposals)", "Disposal History".bold().cyan(), tracker.disposals.len());
                for d in &tracker.disposals {
                    let gl = if d.gain_loss >= 0.0 {
                        format!("+{:.2}", d.gain_loss).to_string()
                    } else {
                        format!("{:.2}", d.gain_loss)
                    };
                    println!("  {} — {} tokens, proceeds {:.2}, basis {:.2}, {} ({})",
                        &d.timestamp[..10], d.amount, d.total_proceeds, d.cost_basis,
                        gl, d.method.label());
                }
            }
        }
        TaxAction::Summary { year } => {
            let s = tracker.annual_summary(year);
            println!("{} ({})", "Annual Tax Summary".bold().cyan(), year);
            println!("  Method:        {}", s.method.label());
            println!("  Disposals:     {} ({} tokens)", s.disposal_count, s.total_disposals);
            println!("  Proceeds:      {:.2}", s.total_proceeds);
            println!("  Cost basis:    {:.2}", s.total_cost_basis);
            println!("  Gain/Loss:     {:.2}", s.total_gain_loss);
            println!("  Short-term:    {:.2}", s.short_term_gain);
            println!("  Long-term:     {:.2}", s.long_term_gain);
            println!("  Energy costs:  {:.2}", s.energy_costs);
            println!("  Gas costs:     {:.2}", s.gas_costs);
        }
        TaxAction::SetMethod { method } => {
            let m = CostBasisMethod::from_str(&method)
                .ok_or_else(|| format!("Unknown method: {}. Use fifo, lifo, or hifo", method))?;
            tracker.set_method(m);
            tracker.save(&path)?;
            println!("{} Cost basis method set to {}", "✓".green(), m.label());
        }
        TaxAction::EnergyCost { amount, description, reference } => {
            tracker.record_energy_cost(amount, &description, &reference);
            tracker.save(&path)?;
            println!("{} Energy cost recorded: {:.2} ({})", "✓".green(), amount, description);
        }
        TaxAction::ExportCsv { file } => {
            let csv = tracker.disposals_csv();
            std::fs::write(&file, &csv)?;
            println!("{} Disposals exported to {} ({} entries)",
                "✓".green(), file.display(), tracker.disposals.len());
        }
    }
    Ok(())
}

// ── Policy handler ──

fn cmd_policy(action: PolicyAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::policy_engine::{PolicyEngine, Rule, Enforcement, TxContext};

    let path = crate::policy_engine::default_policy_path();
    let mut engine = if path.exists() {
        PolicyEngine::load(&path)?
    } else {
        PolicyEngine::new()
    };

    let parse_enforcement = |s: &str| -> Enforcement {
        match s.to_lowercase().as_str() {
            "block" => Enforcement::Block,
            "warn" => Enforcement::Warn,
            "log" => Enforcement::Log,
            _ => Enforcement::Block,
        }
    };

    match action {
        PolicyAction::List => {
            let policies = engine.list();
            if policies.is_empty() {
                println!("{}", "No policies configured.".dimmed());
            } else {
                println!("{} ({} policies, {} active)", "Transaction Policies".bold().cyan(),
                    engine.count(), engine.active_count());
                for p in policies {
                    let status = if p.enabled { "active".green() } else { "disabled".dimmed() };
                    let rules: Vec<String> = p.rules.iter().map(|r| r.label()).collect();
                    println!("  {} [{}] {:?} — {} ({})",
                        p.name.bold(), status, p.enforcement, p.description,
                        rules.join(", "));
                }
            }
        }
        PolicyAction::AddMaxAmount { name, max, enforcement } => {
            let policy = crate::policy_engine::make_policy(
                &name, &format!("Block if amount > {}", max),
                vec![Rule::MaxAmount(max)], parse_enforcement(&enforcement), 10,
            );
            engine.add_policy(policy)?;
            engine.save(&path)?;
            println!("{} Policy '{}' created: max amount {}", "✓".green(), name, max);
        }
        PolicyAction::AddBlocklist { name, addresses, enforcement } => {
            let addrs: Vec<String> = addresses.split(',').map(|s| s.trim().to_string()).collect();
            let count = addrs.len();
            let policy = crate::policy_engine::make_policy(
                &name, &format!("Block {} addresses", count),
                vec![Rule::BlockedRecipients(addrs)], parse_enforcement(&enforcement), 10,
            );
            engine.add_policy(policy)?;
            engine.save(&path)?;
            println!("{} Policy '{}' created: {} blocked addresses", "✓".green(), name, count);
        }
        PolicyAction::AddTimelock { name, deny_after, deny_before, enforcement } => {
            let policy = crate::policy_engine::make_policy(
                &name, &format!("Block between {}h-{}h", deny_after, deny_before),
                vec![Rule::TimeRestriction { deny_after, deny_before }], parse_enforcement(&enforcement), 10,
            );
            engine.add_policy(policy)?;
            engine.save(&path)?;
            println!("{} Policy '{}' created: time lock {}h-{}h", "✓".green(), name, deny_after, deny_before);
        }
        PolicyAction::Show { name } => {
            match engine.get(&name) {
                Some(p) => {
                    println!("{}", "Policy".bold().cyan());
                    println!("  Name:        {}", p.name);
                    println!("  Description: {}", p.description);
                    println!("  Enforcement: {:?}", p.enforcement);
                    println!("  Combine:     {:?}", p.combine);
                    println!("  Enabled:     {}", p.enabled);
                    println!("  Priority:    {}", p.priority);
                    println!("  Rules:");
                    for r in &p.rules {
                        println!("    - {}", r.label());
                    }
                }
                None => eprintln!("{} Policy not found: {}", "Error:".red().bold(), name),
            }
        }
        PolicyAction::Remove { name } => {
            engine.remove_policy(&name)?;
            engine.save(&path)?;
            println!("{} Policy removed.", "✓".green());
        }
        PolicyAction::Enable { name } => {
            engine.enable(&name)?;
            engine.save(&path)?;
            println!("{} Policy enabled.", "✓".green());
        }
        PolicyAction::Disable { name } => {
            engine.disable(&name)?;
            engine.save(&path)?;
            println!("{} Policy disabled.", "✓".green());
        }
        PolicyAction::Test { to, amount } => {
            let ctx = TxContext::new("transfer", &to, amount, "self");
            let results = engine.evaluate(&ctx);
            let blocked = results.iter().any(|r| !r.passed && r.enforcement == Enforcement::Block);
            if blocked {
                println!("{} Transaction would be BLOCKED", "✗".red().bold());
            } else {
                println!("{} Transaction would be ALLOWED", "✓".green());
            }
            for r in &results {
                if !r.passed {
                    println!("  {} {} — {:?}: {:?}", "✗".red(), r.policy_name, r.enforcement, r.violations);
                } else {
                    println!("  {} {}", "✓".green(), r.policy_name);
                }
            }
        }
    }
    Ok(())
}

// ── Export handler ──

fn cmd_export(action: ExportAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::export::{Exporter, AccountSummary, HistoryRow};

    match action {
        ExportAction::History { file } => {
            // Load from analytics if available
            let analytics_path = crate::analytics::default_analytics_path();
            if analytics_path.exists() {
                let tracker = crate::analytics::AnalyticsTracker::load(&analytics_path)?;
                let rows: Vec<HistoryRow> = tracker.data_points.iter().map(|dp| {
                    HistoryRow {
                        timestamp: dp.timestamp.clone(),
                        tx_type: dp.event.label().to_string(),
                        from: "self".into(),
                        to: dp.reference.clone(),
                        amount: dp.amount,
                        fee: 0,
                        status: "confirmed".into(),
                        reference: dp.reference.clone(),
                    }
                }).collect();
                let fmt = Exporter::detect_format(&file.to_string_lossy());
                let count = Exporter::export_history(&rows, &file, fmt)?;
                println!("{} Exported {} history entries to {}", "✓".green(), count, file.display());
            } else {
                println!("{}", "No analytics data to export. Record events first with: wallet analytics record".dimmed());
            }
        }
        ExportAction::Summary { file } => {
            let summary = AccountSummary {
                address: "N/A".into(),
                name: "default".into(),
                balance: 0,
                total_sent: 0,
                total_received: 0,
                total_energy_spent: 0,
                total_gas_spent: 0,
                object_count: 0,
                nft_count: 0,
                token_count: 0,
                created_at: "N/A".into(),
                exported_at: chrono::Utc::now().to_rfc3339(),
            };
            let fmt = Exporter::detect_format(&file.to_string_lossy());
            Exporter::export_summary(&summary, &file, fmt)?;
            println!("{} Account summary exported to {}", "✓".green(), file.display());
        }
        ExportAction::Dump { file } => {
            let mut data = std::collections::HashMap::new();
            data.insert("version".into(), serde_json::json!("1.0.0"));
            data.insert("exported_at".into(), serde_json::json!(chrono::Utc::now().to_rfc3339()));

            // Include config
            let config_path = WalletConfig::default_path();
            if let Ok(config) = WalletConfig::load_or_default(&config_path) {
                data.insert("config".into(), serde_json::to_value(&config)?);
            }

            Exporter::export_state_dump(&data, &file)?;
            println!("{} State dump exported to {}", "✓".green(), file.display());
        }
    }
    Ok(())
}

fn cmd_script(action: ScriptAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir().join("scripts");
    std::fs::create_dir_all(&dir)?;

    match action {
        ScriptAction::List => {
            let entries = std::fs::read_dir(&dir)?;
            let mut found = false;
            println!("{}", "Saved Scripts".bold().cyan());
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".json") {
                    println!("  {}", name.trim_end_matches(".json"));
                    found = true;
                }
            }
            if !found {
                println!("  (none)");
            }
        }
        ScriptAction::Show { name } => {
            let path = dir.join(format!("{}.json", name));
            let data = std::fs::read_to_string(&path)?;
            let script: crate::scripting::Script = serde_json::from_str(&data)?;
            println!("{} {}", "Script:".bold().cyan(), script.name);
            if !script.description.is_empty() {
                println!("  {}", script.description);
            }
            println!("  Steps: {}", script.steps.len());
            for (i, step) in script.steps.iter().enumerate() {
                println!("    {}. {} — {:?}", i + 1, step.name, step.operation);
            }
        }
        ScriptAction::Load { file } => {
            let data = std::fs::read_to_string(&file)?;
            let script: crate::scripting::Script = serde_json::from_str(&data)?;
            let dest = dir.join(format!("{}.json", script.name));
            std::fs::write(&dest, &data)?;
            println!(
                "{} Script '{}' loaded ({} steps)",
                "✓".green(),
                script.name,
                script.steps.len()
            );
        }
        ScriptAction::Run { name, live } => {
            let path = dir.join(format!("{}.json", name));
            let data = std::fs::read_to_string(&path)?;
            let script: crate::scripting::Script = serde_json::from_str(&data)?;
            let mut exec = crate::scripting::ScriptExecutor::new(!live);
            let result = exec.execute(&script)?;
            if live {
                println!("{} Script '{}' executed", "✓".green(), name);
            } else {
                println!("{} Script '{}' dry-run complete", "✓".green(), name);
            }
            println!("  Steps run: {}", result.executed);
            println!("  Skipped:   {}", result.skipped);
            if !result.step_results.is_empty() {
                for sr in &result.step_results {
                    let status = if sr.skipped {
                        "SKIP"
                    } else if sr.success {
                        "OK"
                    } else {
                        "FAIL"
                    };
                    println!("    [{}] {}", status, sr.step_name);
                }
            }
        }
        ScriptAction::Delete { name } => {
            let path = dir.join(format!("{}.json", name));
            if path.exists() {
                std::fs::remove_file(&path)?;
                println!("{} Script '{}' deleted", "✓".green(), name);
            } else {
                println!("{} Script '{}' not found", "✗".red(), name);
            }
        }
    }
    Ok(())
}

fn cmd_metrics(action: MetricsAction) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::config::default_data_dir().join("metrics.json");

    let mut registry = if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        crate::metrics::MetricsRegistry::new()
    };

    match action {
        MetricsAction::Show => {
            let metrics = registry.list();
            if metrics.is_empty() {
                println!("No metrics registered. Run 'metrics init' first.");
            } else {
                println!("{}", "Wallet Metrics".bold().cyan());
                for m in metrics {
                    match m {
                        crate::metrics::Metric::Counter(c) => {
                            println!("  [counter] {} = {}", c.name, c.value);
                        }
                        crate::metrics::Metric::Gauge(g) => {
                            println!("  [gauge]   {} = {}", g.name, g.value);
                        }
                        crate::metrics::Metric::Histogram(h) => {
                            println!(
                                "  [hist]    {} count={} sum={}",
                                h.name, h.count, h.sum
                            );
                        }
                    }
                }
            }
        }
        MetricsAction::Prometheus => {
            print!("{}", registry.to_prometheus());
        }
        MetricsAction::Json => {
            println!("{}", registry.to_json());
        }
        MetricsAction::Reset => {
            registry.reset();
            let json = serde_json::to_string_pretty(&registry)?;
            std::fs::write(&path, json)?;
            println!("{} All metrics reset", "✓".green());
        }
        MetricsAction::Init => {
            registry.register_wallet_defaults();
            let json = serde_json::to_string_pretty(&registry)?;
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, json)?;
            println!("{} Default wallet metrics registered ({})", "✓".green(), registry.list().len());
        }
    }
    Ok(())
}

fn cmd_migrate(action: MigrateAction) -> Result<(), Box<dyn std::error::Error>> {
    let migrator = crate::migrate::Migrator::new();

    match action {
        MigrateAction::Detect { input } => {
            // Check if it's a file path
            let content = if std::path::Path::new(&input).exists() {
                std::fs::read_to_string(&input)?
            } else {
                input
            };
            match crate::migrate::detect_format(&content) {
                Some(fmt) => {
                    crate::output::json_or(&serde_json::json!({"format": fmt.name()}), || {
                        println!("{} Detected: {}", "✓".green(), fmt.name());
                    });
                }
                None => {
                    println!("{} Could not detect format", "✗".red());
                }
            }
        }
        MigrateAction::Plan { input } => {
            let content = if std::path::Path::new(&input).exists() {
                std::fs::read_to_string(&input)?
            } else {
                input
            };
            let plan = migrator.plan_from_input(&content)?;
            crate::output::json_or(&plan, || {
                println!("{}", "Migration Plan".bold().cyan());
                println!("  Format:   {}", plan.format);
                println!("  Accounts: {}", plan.accounts_found);
                for acc in &plan.accounts {
                    println!("    - {} ({})", acc.label, acc.note);
                    if let Some(ref addr) = acc.original_address {
                        println!("      Original: {}", addr);
                    }
                }
                if !plan.warnings.is_empty() {
                    println!("  {}:", "Warnings".yellow());
                    for w in &plan.warnings {
                        println!("    ⚠ {}", w);
                    }
                }
            });
        }
        MigrateAction::History => {
            let path = crate::config::default_data_dir().join("migrations.json");
            if path.exists() {
                let m = crate::migrate::Migrator::load(&path)?;
                if m.history.is_empty() {
                    println!("No migration history.");
                } else {
                    println!("{}", "Migration History".bold().cyan());
                    for rec in m.list_history() {
                        println!(
                            "  {} — {} ({} accounts) [{:?}]",
                            rec.id, rec.format, rec.accounts_imported, rec.status
                        );
                    }
                }
            } else {
                println!("No migration history.");
            }
        }
        MigrateAction::Formats => {
            let matrix = crate::migrate::Migrator::compatibility_matrix();
            println!("{}", "Supported Migration Sources".bold().cyan());
            for (wallet, notes) in &matrix {
                println!("  {}:", wallet.bold());
                for n in notes {
                    println!("    - {}", n);
                }
            }
        }
        MigrateAction::ValidateMnemonic { phrase } => {
            match crate::migrate::Migrator::validate_mnemonic(&phrase) {
                Ok(count) => println!("{} Valid {}-word mnemonic", "✓".green(), count),
                Err(e) => println!("{} Invalid: {}", "✗".red(), e),
            }
        }
        MigrateAction::ValidateKey { key } => {
            match crate::migrate::Migrator::validate_private_key(&key) {
                Ok(bytes) => println!("{} Valid private key ({} bytes)", "✓".green(), bytes),
                Err(e) => println!("{} Invalid: {}", "✗".red(), e),
            }
        }
    }
    Ok(())
}

fn cmd_qr(action: QrAction) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        QrAction::Address { address } => {
            let qr = crate::qr::QrCode::encode(&address)?;
            println!("{}", "Address QR Code".bold().cyan());
            println!("{}", address);
            println!();
            println!("{}", qr.to_terminal());
        }
        QrAction::Pay {
            address,
            amount,
            label,
            message,
        } => {
            let mut uri = crate::qr::PaymentUri::new(&address);
            if let Some(amt) = amount {
                uri = uri.with_amount(amt);
            }
            if let Some(ref l) = label {
                uri = uri.with_label(l);
            }
            if let Some(ref m) = message {
                uri = uri.with_message(m);
            }
            let uri_str = uri.to_uri();
            let qr = crate::qr::QrCode::encode(&uri_str)?;
            println!("{}", "Payment QR Code".bold().cyan());
            println!("{}", uri_str);
            println!();
            println!("{}", qr.to_terminal());
        }
        QrAction::Encode { data } => {
            let qr = crate::qr::QrCode::encode(&data)?;
            println!("{}", qr.to_terminal());
        }
        QrAction::Svg { data, file } => {
            let qr = crate::qr::QrCode::encode(&data)?;
            let svg = qr.to_svg(4);
            std::fs::write(&file, svg)?;
            println!("{} QR SVG exported to {}", "✓".green(), file.display());
        }
    }
    Ok(())
}

fn cmd_bench(action: BenchAction) -> Result<(), Box<dyn std::error::Error>> {
    let hist_path = crate::config::default_data_dir().join("bench_history.json");

    match action {
        BenchAction::Run { quick } => {
            let config = if quick {
                crate::benchmark::BenchConfig::quick()
            } else {
                crate::benchmark::BenchConfig::default()
            };
            println!("{}", "Running wallet benchmarks...".bold().cyan());
            let suite = crate::benchmark::run_wallet_benchmarks(&config)?;
            println!("{}", suite.to_report());

            // Save to history
            let mut history = if hist_path.exists() {
                crate::benchmark::BenchHistory::load(&hist_path).unwrap_or_default()
            } else {
                crate::benchmark::BenchHistory::new()
            };
            history.add_run(suite);
            std::fs::create_dir_all(hist_path.parent().unwrap())?;
            history.save(&hist_path)?;
            println!("{} Results saved to history", "✓".green());
        }
        BenchAction::Show => {
            if !hist_path.exists() {
                println!("No benchmark history. Run 'bench run' first.");
                return Ok(());
            }
            let history = crate::benchmark::BenchHistory::load(&hist_path)?;
            if let Some(latest) = history.latest() {
                println!("{}", latest.to_report());
            } else {
                println!("No benchmark runs recorded.");
            }
        }
        BenchAction::Regressions => {
            if !hist_path.exists() {
                println!("No benchmark history. Run 'bench run' at least twice.");
                return Ok(());
            }
            let history = crate::benchmark::BenchHistory::load(&hist_path)?;
            let regressions = history.check_regressions(0.1);
            if regressions.is_empty() {
                println!("{} No performance regressions detected", "✓".green());
            } else {
                println!("{}", "Performance Regressions".bold().red());
                for r in &regressions {
                    println!(
                        "  {} — {:.1}x slower (baseline: {}, current: {})",
                        r.name,
                        1.0 / r.speedup,
                        crate::benchmark::format_ns(r.baseline.mean_ns),
                        crate::benchmark::format_ns(r.current.mean_ns),
                    );
                }
            }
        }
    }
    Ok(())
}

fn cmd_health(action: HealthAction) -> Result<(), Box<dyn std::error::Error>> {
    let checker = crate::health::HealthChecker::from_defaults();

    match action {
        HealthAction::Check => {
            let report = checker.run_all();
            crate::output::json_or(&report, || {
                print!("{}", report.to_text());
            });
        }
        HealthAction::Issues => {
            let report = checker.run_all();
            let issues: Vec<_> = report
                .checks
                .iter()
                .filter(|c| c.status != crate::health::HealthStatus::Healthy)
                .collect();
            if issues.is_empty() {
                println!("{} No issues found", "✓".green());
            } else {
                println!("{}", "Issues Found".bold().yellow());
                for check in issues {
                    println!(
                        "  [{}] {} — {}",
                        check.status.emoji(),
                        check.name,
                        check.message
                    );
                    if let Some(ref fix) = check.fix {
                        println!("        Fix: {}", fix);
                    }
                }
            }
        }
        HealthAction::Quick => {
            let data_dir = crate::config::default_data_dir();
            if crate::health::quick_check(&data_dir) {
                println!("{} Wallet OK", "✓".green());
            } else {
                println!("{} Wallet needs attention. Run 'health check' for details.", "✗".red());
            }
        }
    }
    Ok(())
}

fn cmd_plugin(action: PluginAction2) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::config::default_data_dir().join("plugins.json");
    let mut registry = crate::plugin::PluginRegistry::load_or_default(&path);

    match action {
        PluginAction2::List => {
            let plugins = registry.list();
            if plugins.is_empty() {
                println!("No plugins installed.");
            } else {
                println!("{}", "Installed Plugins".bold().cyan());
                for p in plugins {
                    let status = if p.enabled { "enabled" } else { "disabled" };
                    println!(
                        "  {} v{} [{}] — {}",
                        p.manifest.name, p.manifest.version, status, p.manifest.description
                    );
                }
            }
        }
        PluginAction2::Show { name } => {
            match registry.get(&name) {
                Some(p) => {
                    println!("{} v{}", p.manifest.name.bold().cyan(), p.manifest.version);
                    println!("  Author:      {}", p.manifest.author);
                    println!("  Description: {}", p.manifest.description);
                    println!("  Enabled:     {}", p.enabled);
                    println!("  Installed:   {}", p.installed_at);
                    println!("  Executions:  {}", p.execution_count);
                    println!("  Hooks:       {:?}", p.manifest.hooks);
                    println!("  Permissions: {:?}", p.manifest.permissions);
                    if !p.config.is_empty() {
                        println!("  Config:");
                        for (k, v) in &p.config {
                            println!("    {} = {}", k, v);
                        }
                    }
                    let dangerous = p.dangerous_permissions();
                    if !dangerous.is_empty() {
                        println!(
                            "  {} Dangerous permissions: {:?}",
                            "⚠".yellow(),
                            dangerous
                        );
                    }
                }
                None => println!("{} Plugin '{}' not found", "✗".red(), name),
            }
        }
        PluginAction2::Install { file } => {
            let data = std::fs::read_to_string(&file)?;
            let manifest: crate::plugin::PluginManifest = serde_json::from_str(&data)?;
            let name = manifest.name.clone();
            let dangerous = manifest
                .permissions
                .iter()
                .filter(|p| p.is_dangerous())
                .count();
            registry.install(manifest)?;
            std::fs::create_dir_all(path.parent().unwrap())?;
            registry.save(&path)?;
            println!("{} Plugin '{}' installed", "✓".green(), name);
            if dangerous > 0 {
                println!(
                    "  {} This plugin has {} dangerous permission(s). Review with 'plugin show {}'",
                    "⚠".yellow(),
                    dangerous,
                    name
                );
            }
        }
        PluginAction2::Uninstall { name } => {
            registry.uninstall(&name)?;
            registry.save(&path)?;
            println!("{} Plugin '{}' uninstalled", "✓".green(), name);
        }
        PluginAction2::Enable { name } => {
            registry.enable(&name)?;
            registry.save(&path)?;
            println!("{} Plugin '{}' enabled", "✓".green(), name);
        }
        PluginAction2::Disable { name } => {
            registry.disable(&name)?;
            registry.save(&path)?;
            println!("{} Plugin '{}' disabled", "✓".green(), name);
        }
        PluginAction2::Audit => {
            let audit = registry.audit_permissions();
            if audit.is_empty() {
                println!("{} No plugins with dangerous permissions", "✓".green());
            } else {
                println!("{}", "Plugin Permission Audit".bold().yellow());
                for (name, perms) in &audit {
                    println!("  {}:", name.bold());
                    for p in perms {
                        println!("    - {} (dangerous)", p.name());
                    }
                }
            }
        }
    }
    Ok(())
}

fn cmd_schedule(action: ScheduleAction) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::config::default_data_dir().join("scheduler.json");
    let mut scheduler = crate::scheduler::Scheduler::load_or_default(&path);

    match action {
        ScheduleAction::List => {
            let jobs = scheduler.list();
            if jobs.is_empty() {
                println!("No scheduled jobs.");
            } else {
                println!("{}", "Scheduled Jobs".bold().cyan());
                for j in jobs {
                    let status = if j.is_auto_disabled() {
                        "auto-disabled"
                    } else if j.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    println!(
                        "  {} — {} [{}] ({})",
                        j.id,
                        j.name,
                        status,
                        j.schedule.to_human()
                    );
                }
            }
        }
        ScheduleAction::Show { id } => {
            match scheduler.get(&id) {
                Some(j) => {
                    println!("{} ({})", j.name.bold().cyan(), j.id);
                    println!("  Schedule:    {}", j.schedule.to_human());
                    println!("  Enabled:     {}", j.enabled);
                    println!("  Run count:   {}", j.run_count);
                    println!("  Fail count:  {}", j.fail_count);
                    println!("  Action:      {:?}", j.action);
                    if let Some(ref last) = j.last_run {
                        println!("  Last run:    {}", last);
                    }
                    if let Some(ref next) = j.next_run {
                        println!("  Next run:    {}", next);
                    }
                    if let Some(ref err) = j.last_error {
                        println!("  Last error:  {}", err);
                    }
                }
                None => println!("{} Job '{}' not found", "✗".red(), id),
            }
        }
        ScheduleAction::Add {
            id,
            name,
            interval,
            action,
        } => {
            let schedule = crate::scheduler::Schedule::from_str(&interval)?;
            let job_action = match action.as_str() {
                "backup" => crate::scheduler::JobAction::Backup,
                "energy_scan" => crate::scheduler::JobAction::EnergyScan,
                _ => crate::scheduler::JobAction::Log {
                    message: format!("Scheduled: {}", action),
                },
            };
            let job = crate::scheduler::Job::new(&id, &name, job_action, schedule);
            scheduler.add(job)?;
            std::fs::create_dir_all(path.parent().unwrap())?;
            scheduler.save(&path)?;
            println!("{} Job '{}' scheduled ({})", "✓".green(), id, interval);
        }
        ScheduleAction::Remove { id } => {
            scheduler.remove(&id)?;
            scheduler.save(&path)?;
            println!("{} Job '{}' removed", "✓".green(), id);
        }
        ScheduleAction::Enable { id } => {
            scheduler.enable(&id)?;
            scheduler.save(&path)?;
            println!("{} Job '{}' enabled", "✓".green(), id);
        }
        ScheduleAction::Disable { id } => {
            scheduler.disable(&id)?;
            scheduler.save(&path)?;
            println!("{} Job '{}' disabled", "✓".green(), id);
        }
        ScheduleAction::Stats => {
            let stats = scheduler.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Scheduler Stats".bold().cyan());
                println!("  Total jobs:    {}", stats.total_jobs);
                println!("  Enabled:       {}", stats.enabled);
                println!("  Disabled:      {}", stats.disabled);
                println!("  Auto-disabled: {}", stats.auto_disabled);
                println!("  Total runs:    {}", stats.total_runs);
                println!("  Total fails:   {}", stats.total_failures);
            });
        }
    }
    Ok(())
}

fn cmd_allowlist(action: AllowlistAction) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::config::default_data_dir().join("allowlist.json");
    let mut store = crate::allowlist::AddressListStore::load_or_default(&path);

    match action {
        AllowlistAction::Allow { address, note } => {
            store.add(&address, crate::allowlist::ListType::Allow, &note, "cli")?;
            store.save(&path)?;
            println!("{} {} added to allow list", "✓".green(), address);
        }
        AllowlistAction::Deny { address, note } => {
            store.add(&address, crate::allowlist::ListType::Deny, &note, "cli")?;
            store.save(&path)?;
            println!("{} {} added to deny list", "✓".green(), address);
        }
        AllowlistAction::Remove { address } => {
            store.remove(&address)?;
            store.save(&path)?;
            println!("{} {} removed", "✓".green(), address);
        }
        AllowlistAction::Check { address } => {
            let verdict = store.check(&address);
            crate::output::json_or(&verdict, || {
                match &verdict {
                    crate::allowlist::Verdict::Allowed => {
                        println!("{} {} is allowed", "✓".green(), address);
                    }
                    crate::allowlist::Verdict::Denied { reason } => {
                        println!("{} {} is denied: {}", "✗".red(), address, reason);
                    }
                    crate::allowlist::Verdict::NotListed => {
                        println!("  {} is not listed (default policy applies)", address);
                    }
                }
            });
        }
        AllowlistAction::List => {
            let allowed = store.allowed();
            let denied = store.denied();
            if allowed.is_empty() && denied.is_empty() {
                println!("No entries.");
            } else {
                if !allowed.is_empty() {
                    println!("{}", "Allowed".bold().green());
                    for e in &allowed {
                        println!("  {} {}", e.address, if e.note.is_empty() { "" } else { &e.note });
                    }
                }
                if !denied.is_empty() {
                    println!("{}", "Denied".bold().red());
                    for e in &denied {
                        println!("  {} {}", e.address, if e.note.is_empty() { "" } else { &e.note });
                    }
                }
            }
        }
        AllowlistAction::Export => {
            print!("{}", store.to_csv());
        }
        AllowlistAction::Import { file } => {
            let csv = std::fs::read_to_string(&file)?;
            let count = store.import_csv(&csv)?;
            store.save(&path)?;
            println!("{} Imported {} entries", "✓".green(), count);
        }
        AllowlistAction::Purge => {
            let purged = store.purge_expired();
            store.save(&path)?;
            println!("{} Purged {} expired entries", "✓".green(), purged);
        }
    }
    Ok(())
}

fn cmd_timelock(action: TimelockAction) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::config::default_data_dir().join("timelocks.json");
    let mut store = crate::timelock::TimelockStore::load_or_default(&path);

    match action {
        TimelockAction::Create {
            recipient,
            amount,
            unlock_at,
            cancellable,
        } => {
            let id = format!("tl-{}", store.timelocks.len() + 1);
            let lock = crate::timelock::Timelock::new(&id, "self", &recipient, amount, &unlock_at, cancellable);
            store.add_timelock(lock)?;
            std::fs::create_dir_all(path.parent().unwrap())?;
            store.save(&path)?;
            println!("{} Timelock {} created — {} EVAP locked until {}", "✓".green(), id, amount, unlock_at);
        }
        TimelockAction::List => {
            let locks = store.list_timelocks();
            if locks.is_empty() {
                println!("No timelocks.");
            } else {
                println!("{}", "Timelocks".bold().cyan());
                for l in locks {
                    println!(
                        "  {} — {} EVAP to {} [unlock: {}] ({:?})",
                        l.id, l.remaining(), l.recipient, l.unlock_at, l.status
                    );
                }
            }
        }
        TimelockAction::Show { id } => {
            match store.get_timelock(&id) {
                Some(l) => {
                    println!("{} ({:?})", l.id.bold().cyan(), l.status);
                    println!("  Recipient:  {}", l.recipient);
                    println!("  Amount:     {} EVAP", l.total_amount);
                    println!("  Claimed:    {} EVAP", l.claimed_amount);
                    println!("  Unlock at:  {}", l.unlock_at);
                    println!("  Cancellable: {}", l.cancellable);
                }
                None => println!("{} Timelock '{}' not found", "✗".red(), id),
            }
        }
        TimelockAction::Claim { id } => {
            let now = chrono::Utc::now().to_rfc3339();
            match store.get_timelock_mut(&id) {
                Some(l) => {
                    let claimed = l.claim(&now)?;
                    store.save(&path)?;
                    println!("{} Claimed {} EVAP from timelock {}", "✓".green(), claimed, id);
                }
                None => println!("{} Timelock '{}' not found", "✗".red(), id),
            }
        }
        TimelockAction::Cancel { id } => {
            match store.get_timelock_mut(&id) {
                Some(l) => {
                    let refund = l.cancel()?;
                    store.save(&path)?;
                    println!("{} Cancelled timelock {} — {} EVAP refunded", "✓".green(), id, refund);
                }
                None => println!("{} Timelock '{}' not found", "✗".red(), id),
            }
        }
        TimelockAction::Vest {
            beneficiary,
            amount,
            start,
            cliff,
            end,
        } => {
            let id = format!("vest-{}", store.vestings.len() + 1);
            let v = crate::timelock::VestingSchedule::new(&id, &beneficiary, amount, &start, &cliff, &end)?;
            store.add_vesting(v)?;
            store.save(&path)?;
            println!("{} Vesting {} created — {} EVAP for {}", "✓".green(), id, amount, beneficiary);
        }
        TimelockAction::Vestings => {
            let vestings = store.list_vestings();
            if vestings.is_empty() {
                println!("No vesting schedules.");
            } else {
                println!("{}", "Vesting Schedules".bold().cyan());
                for v in vestings {
                    let now = chrono::Utc::now().timestamp();
                    println!(
                        "  {} — {} EVAP for {} ({:.1}% vested, {} remaining)",
                        v.id, v.total_amount, v.beneficiary,
                        v.percent_vested(now), v.remaining()
                    );
                }
            }
        }
    }
    Ok(())
}

fn cmd_memo(action: MemoAction2) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::config::default_data_dir().join("memos.json");
    let mut store = crate::memo::MemoStore::load_or_default(&path);

    match action {
        MemoAction2::Add {
            recipient,
            content,
            tx_hash,
        } => {
            let id = format!("memo-{}", store.memos.len() + 1);
            let mut memo = crate::memo::Memo::public(&id, "self", &recipient, &content)?;
            if let Some(hash) = tx_hash {
                memo = memo.with_tx_hash(&hash);
            }
            store.add(memo);
            std::fs::create_dir_all(path.parent().unwrap())?;
            store.save(&path)?;
            println!("{} Memo {} saved", "✓".green(), id);
        }
        MemoAction2::List => {
            let memos = store.list();
            if memos.is_empty() {
                println!("No memos.");
            } else {
                println!("{}", "Memos".bold().cyan());
                for m in memos {
                    let tx = m.tx_hash.as_deref().unwrap_or("(no tx)");
                    println!(
                        "  {} → {} [{}] \"{}\"",
                        m.id, m.recipient, tx, m.display_content()
                    );
                }
            }
        }
        MemoAction2::Search { query } => {
            let results = store.search(&query);
            if results.is_empty() {
                println!("No memos matching '{}'", query);
            } else {
                for m in results {
                    println!("  {} — \"{}\"", m.id, m.content);
                }
            }
        }
        MemoAction2::Show { id } => {
            match store.get(&id) {
                Some(m) => {
                    println!("{}", m.id.bold().cyan());
                    println!("  Sender:    {}", m.sender);
                    println!("  Recipient: {}", m.recipient);
                    println!("  Content:   {}", m.display_content());
                    println!("  Visibility: {:?}", m.visibility);
                    if let Some(ref tx) = m.tx_hash {
                        println!("  Tx hash:   {}", tx);
                    }
                    println!("  Created:   {}", m.created_at);
                }
                None => println!("{} Memo '{}' not found", "✗".red(), id),
            }
        }
        MemoAction2::Delete { id } => {
            if store.remove(&id).is_some() {
                store.save(&path)?;
                println!("{} Memo '{}' deleted", "✓".green(), id);
            } else {
                println!("{} Memo '{}' not found", "✗".red(), id);
            }
        }
    }
    Ok(())
}

fn cmd_recovery(action: RecoveryAction2) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::config::default_data_dir().join("recovery.json");
    let mut store = crate::recovery::RecoveryStore::load_or_default(&path);

    match action {
        RecoveryAction2::DeadmanSetup {
            beneficiary,
            interval_days,
        } => {
            let dms = crate::recovery::DeadManSwitch::new(&beneficiary, interval_days)?;
            store.dead_man_switch = Some(dms);
            std::fs::create_dir_all(path.parent().unwrap())?;
            store.save(&path)?;
            println!(
                "{} Dead man's switch configured — check in every {} days, beneficiary: {}",
                "✓".green(),
                interval_days,
                beneficiary
            );
        }
        RecoveryAction2::DeadmanCheckin => {
            if let Some(dms) = store.dead_man_switch.as_mut() {
                dms.check_in();
                let days = dms.days_remaining();
                store.save(&path)?;
                println!("{} Checked in — next deadline in {} days", "✓".green(), days);
            } else {
                println!("{} No dead man's switch configured", "✗".red());
            }
        }
        RecoveryAction2::DeadmanStatus => {
            match &store.dead_man_switch {
                Some(dms) => {
                    println!("{}", "Dead Man's Switch".bold().cyan());
                    println!("  Enabled:     {}", dms.enabled);
                    println!("  Beneficiary: {}", dms.beneficiary);
                    println!("  Interval:    {} days", dms.check_in_interval_days);
                    println!("  Last check:  {}", dms.last_check_in);
                    println!("  Deadline:    {}", dms.deadline);
                    println!("  Days left:   {}", dms.days_remaining());
                    println!("  Status:      {:?}", dms.status);
                }
                None => println!("No dead man's switch configured."),
            }
        }
        RecoveryAction2::DeadmanDisable => {
            match store.dead_man_switch.as_mut() {
                Some(dms) => {
                    dms.disable();
                    store.save(&path)?;
                    println!("{} Dead man's switch disabled", "✓".green());
                }
                None => println!("{} No dead man's switch configured", "✗".red()),
            }
        }
        RecoveryAction2::SocialSetup {
            threshold,
            delay_hours,
        } => {
            let sr = crate::recovery::SocialRecovery::new(threshold, delay_hours)?;
            store.social_recovery = Some(sr);
            store.save(&path)?;
            println!(
                "{} Social recovery configured — {}-of-N threshold, {} hour delay",
                "✓".green(),
                threshold,
                delay_hours
            );
        }
        RecoveryAction2::AddGuardian { address, name } => {
            match store.social_recovery.as_mut() {
                Some(sr) => {
                    sr.add_guardian(&address, &name)?;
                    store.save(&path)?;
                    println!("{} Guardian '{}' ({}) added", "✓".green(), name, address);
                }
                None => println!("{} Social recovery not configured. Run 'recovery social-setup' first.", "✗".red()),
            }
        }
        RecoveryAction2::RemoveGuardian { address } => {
            match store.social_recovery.as_mut() {
                Some(sr) => {
                    let g = sr.remove_guardian(&address)?;
                    store.save(&path)?;
                    println!("{} Guardian '{}' removed", "✓".green(), g.name);
                }
                None => println!("{} Social recovery not configured", "✗".red()),
            }
        }
        RecoveryAction2::Guardians => {
            match &store.social_recovery {
                Some(sr) => {
                    let guardians = sr.list_guardians();
                    if guardians.is_empty() {
                        println!("No guardians configured.");
                    } else {
                        println!("{} (threshold: {})", "Guardians".bold().cyan(), sr.threshold);
                        for g in guardians {
                            println!("  {} — {}", g.name, g.address);
                        }
                        if !sr.is_valid() {
                            println!(
                                "  {} Need at least {} guardians (have {})",
                                "⚠".yellow(),
                                sr.threshold,
                                sr.guardians.len()
                            );
                        }
                    }
                }
                None => println!("Social recovery not configured."),
            }
        }
        RecoveryAction2::Status => {
            println!("{}", "Recovery Status".bold().cyan());
            match &store.dead_man_switch {
                Some(dms) => println!(
                    "  Dead Man Switch: {} ({}d remaining)",
                    if dms.enabled { "active" } else { "disabled" },
                    dms.days_remaining()
                ),
                None => println!("  Dead Man Switch: not configured"),
            }
            match &store.social_recovery {
                Some(sr) => {
                    let valid = if sr.is_valid() { "valid" } else { "incomplete" };
                    println!(
                        "  Social Recovery: {}-of-{} ({}, {} pending)",
                        sr.threshold,
                        sr.guardians.len(),
                        valid,
                        sr.pending_requests().len()
                    );
                }
                None => println!("  Social Recovery: not configured"),
            }
        }
    }
    Ok(())
}

// ──────────────────────────── Delegation ──────────────────────────────

fn cmd_delegation(action: DelegationAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("delegations.json");
    let mut store = crate::delegation::DelegationStore::load_or_default(&path);

    match action {
        DelegationAction::Create {
            delegate,
            cap,
            delegation_type,
            per_tx_limit,
        } => {
            let dt = match delegation_type.to_lowercase().as_str() {
                "transfer" => crate::delegation::DelegationType::Transfer,
                "staking" => crate::delegation::DelegationType::Staking,
                "governance" => crate::delegation::DelegationType::Governance,
                "contract_call" => crate::delegation::DelegationType::ContractCall,
                "any" => crate::delegation::DelegationType::Any,
                _ => {
                    println!("Unknown delegation type: {}", delegation_type);
                    return Ok(());
                }
            };
            let id = format!("del_{}", chrono::Utc::now().timestamp_millis());
            let mut delegation =
                crate::delegation::Delegation::new(&id, "self", &delegate, dt, cap);
            if let Some(limit) = per_tx_limit {
                delegation = delegation.with_per_tx_limit(limit);
            }
            store.add(delegation)?;
            store.save(&path)?;
            crate::output::json_or(&serde_json::json!({"created": id}), || {
                println!("Delegation created: {}", id);
                println!("  Delegate: {}", delegate);
                println!("  Type:     {}", delegation_type);
                println!("  Cap:      {}", cap);
            });
        }
        DelegationAction::List => {
            let list = store.list();
            crate::output::json_or(&list, || {
                if list.is_empty() {
                    println!("No delegations.");
                } else {
                    println!("{}", "Delegations".bold().cyan());
                    for d in &list {
                        let status = format!("{:?}", d.status);
                        println!(
                            "  {} → {} ({}) cap={} spent={} [{}]",
                            d.id, d.delegate, d.delegation_type.name(), d.spending_cap, d.spent, status
                        );
                    }
                }
            });
        }
        DelegationAction::Show { id } => {
            match store.get(&id) {
                Some(d) => {
                    crate::output::json_or(&d, || {
                        println!("{}", "Delegation Detail".bold().cyan());
                        println!("  ID:        {}", d.id);
                        println!("  Delegate:  {}", d.delegate);
                        println!("  Type:      {}", d.delegation_type.name());
                        println!("  Cap:       {}", d.spending_cap);
                        println!("  Spent:     {}", d.spent);
                        println!("  Remaining: {}", d.remaining());
                        println!("  Status:    {:?}", d.status);
                        println!("  Created:   {}", d.created_at);
                        if let Some(ref exp) = d.expires_at {
                            println!("  Expires:   {}", exp);
                        }
                        if let Some(limit) = d.per_tx_limit {
                            println!("  Per-Tx:    {}", limit);
                        }
                        println!("  History:   {} transactions", d.spend_history.len());
                    });
                }
                None => println!("Delegation not found: {}", id),
            }
        }
        DelegationAction::Revoke { id } => {
            match store.get_mut(&id) {
                Some(d) => {
                    d.revoke()?;
                    store.save(&path)?;
                    println!("Delegation {} revoked.", id);
                }
                None => println!("Delegation not found: {}", id),
            }
        }
        DelegationAction::RevokeAll => {
            let count = store.revoke_all("self");
            store.save(&path)?;
            println!("Revoked {} delegation(s).", count);
        }
    }
    Ok(())
}

// ──────────────────────────── Sync ───────────────────────────────────

fn cmd_sync(action: SyncAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("addressbook_sync.json");
    let store = if path.exists() {
        crate::addressbook_sync::SyncableAddressBook::load(&path)?
    } else {
        let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".into());
        crate::addressbook_sync::SyncableAddressBook::new(&hostname)
    };

    match action {
        SyncAction::Export { file } => {
            let data = store.export_for_sync();
            std::fs::write(&file, &data)?;
            crate::output::json_or(
                &serde_json::json!({"exported_to": file.display().to_string(), "contacts": store.contacts.len()}),
                || {
                    println!("Exported {} contacts to {}", store.contacts.len(), file.display());
                },
            );
        }
        SyncAction::Import { file } => {
            let mut local = store;
            let data = std::fs::read_to_string(&file)?;
            let remote: crate::addressbook_sync::SyncableAddressBook =
                serde_json::from_str(&data)?;
            let result = local.merge(
                &remote,
                crate::addressbook_sync::ConflictResolution::KeepNewer,
            );
            local.save(&path)?;
            crate::output::json_or(&result, || {
                println!("{}", "Sync Complete".bold().cyan());
                println!("  Added:     {}", result.added);
                println!("  Updated:   {}", result.updated);
                println!("  Deleted:   {}", result.deleted);
                println!("  Conflicts: {}", result.conflicts.len());
            });
        }
        SyncAction::Status => {
            crate::output::json_or(&store, || {
                println!("{}", "Address Book Sync".bold().cyan());
                println!("  Device:     {}", store.device_id);
                println!("  Contacts:   {}", store.contacts.len());
                println!("  Sync Count: {}", store.sync_count);
                match &store.last_sync {
                    Some(ts) => println!("  Last Sync:  {}", ts),
                    None => println!("  Last Sync:  never"),
                }
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Gas Station ─────────────────────────────

fn cmd_gas_station(action: GasStationAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("gas_station.json");
    let mut station = crate::gas_station::GasStation::load_or_default(&path);

    match action {
        GasStationAction::Relays => {
            let relays = station.list_relays();
            crate::output::json_or(&relays, || {
                if relays.is_empty() {
                    println!("No relays configured.");
                } else {
                    println!("{}", "Gas Relays".bold().cyan());
                    for r in &relays {
                        let reliability = r.reliability();
                        println!(
                            "  {} ({}) — {:?} | reliability={:.1}% latency={}ms fee={:.2}%",
                            r.name, r.url, r.status, reliability * 100.0, r.avg_latency_ms, r.fee_percent
                        );
                    }
                }
            });
        }
        GasStationAction::AddRelay { url, name } => {
            let relay = crate::gas_station::Relay::new(&url, &name);
            station.add_relay(relay)?;
            station.save(&path)?;
            println!("Relay added: {} ({})", name, url);
        }
        GasStationAction::RemoveRelay { url } => {
            station.remove_relay(&url)?;
            station.save(&path)?;
            println!("Relay removed: {}", url);
        }
        GasStationAction::BestRelay => {
            match station.best_relay() {
                Some(r) => {
                    crate::output::json_or(&r, || {
                        println!("{}", "Best Relay".bold().cyan());
                        println!("  Name:        {}", r.name);
                        println!("  URL:         {}", r.url);
                        println!("  Reliability: {:.1}%", r.reliability() * 100.0);
                        println!("  Latency:     {}ms", r.avg_latency_ms);
                        println!("  Fee:         {:.2}%", r.fee_percent);
                    });
                }
                None => println!("No active relays available."),
            }
        }
        GasStationAction::Sponsors => {
            let sponsors: Vec<&crate::gas_station::GasSponsor> =
                station.sponsors.values().collect();
            crate::output::json_or(&sponsors, || {
                if sponsors.is_empty() {
                    println!("No gas sponsors configured.");
                } else {
                    println!("{}", "Gas Sponsors".bold().cyan());
                    for s in &sponsors {
                        println!(
                            "  {} ({}) budget={} spent={} remaining={} txs={}",
                            s.name, s.address, s.budget, s.spent, s.remaining(), s.tx_count
                        );
                    }
                }
            });
        }
        GasStationAction::AddSponsor {
            address,
            name,
            budget,
        } => {
            let sponsor = crate::gas_station::GasSponsor::new(&address, &name, budget);
            station.add_sponsor(sponsor);
            station.save(&path)?;
            println!("Sponsor added: {} ({}) budget={}", name, address, budget);
        }
        GasStationAction::Stats => {
            let total_relays = station.relays.len();
            let active_relays = station
                .relays
                .values()
                .filter(|r| r.status == crate::gas_station::RelayStatus::Active)
                .count();
            let total_sponsors = station.sponsors.len();
            let total_budget: u64 = station.sponsors.values().map(|s| s.budget).sum();
            let total_spent: u64 = station.sponsors.values().map(|s| s.spent).sum();
            let meta_txs = station.meta_txs.len();

            let stats = serde_json::json!({
                "total_relays": total_relays,
                "active_relays": active_relays,
                "total_sponsors": total_sponsors,
                "total_budget": total_budget,
                "total_spent": total_spent,
                "meta_transactions": meta_txs,
            });
            crate::output::json_or(&stats, || {
                println!("{}", "Gas Station Stats".bold().cyan());
                println!("  Relays:       {} ({} active)", total_relays, active_relays);
                println!("  Sponsors:     {}", total_sponsors);
                println!("  Total Budget: {}", total_budget);
                println!("  Total Spent:  {}", total_spent);
                println!("  Meta-Txs:     {}", meta_txs);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Intent ─────────────────────────────────

fn cmd_intent(action: IntentAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("intents.json");
    let mut engine = crate::intent::IntentEngine::load_or_default(&path);

    match action {
        IntentAction::Submit {
            description,
            intent_type,
        } => {
            let it = match intent_type.to_lowercase().as_str() {
                "swap" => crate::intent::IntentType::Swap,
                "transfer" => crate::intent::IntentType::Transfer,
                "batch_transfer" => crate::intent::IntentType::BatchTransfer,
                "conditional" => crate::intent::IntentType::Conditional,
                "recurring" => crate::intent::IntentType::Recurring,
                "bridge" => crate::intent::IntentType::Bridge,
                _ => {
                    println!("Unknown intent type: {}", intent_type);
                    return Ok(());
                }
            };
            let id = format!("int_{}", chrono::Utc::now().timestamp_millis());
            let intent = crate::intent::Intent::new(&id, it, "self", &description);
            engine.submit_intent(intent)?;
            engine.save(&path)?;
            crate::output::json_or(&serde_json::json!({"submitted": id}), || {
                println!("Intent submitted: {}", id);
                println!("  Type:        {}", intent_type);
                println!("  Description: {}", description);
            });
        }
        IntentAction::List => {
            let intents: Vec<&crate::intent::Intent> =
                engine.intents.values().collect();
            crate::output::json_or(&intents, || {
                if intents.is_empty() {
                    println!("No intents.");
                } else {
                    println!("{}", "Intents".bold().cyan());
                    for i in &intents {
                        println!(
                            "  {} [{}] {:?} — {}",
                            i.id,
                            i.intent_type.name(),
                            i.status,
                            i.description
                        );
                    }
                }
            });
        }
        IntentAction::Show { id } => {
            match engine.get_intent(&id) {
                Some(i) => {
                    crate::output::json_or(&i, || {
                        println!("{}", "Intent Detail".bold().cyan());
                        println!("  ID:          {}", i.id);
                        println!("  Type:        {}", i.intent_type.name());
                        println!("  Status:      {:?}", i.status);
                        println!("  Sender:      {}", i.sender);
                        println!("  Description: {}", i.description);
                        println!("  Created:     {}", i.created_at);
                        println!("  Constraints: {}", i.constraints.len());
                        let solutions = engine.solutions_for(&i.id);
                        println!("  Solutions:   {}", solutions.len());
                        if let Some(best) = engine.best_solution(&i.id) {
                            println!("  Best Solver: {} (score={})", best.solver_id, best.score);
                        }
                    });
                }
                None => println!("Intent not found: {}", id),
            }
        }
        IntentAction::Cancel { id } => {
            match engine.get_intent_mut(&id) {
                Some(i) => {
                    i.cancel()?;
                    engine.save(&path)?;
                    println!("Intent {} cancelled.", id);
                }
                None => println!("Intent not found: {}", id),
            }
        }
        IntentAction::Solvers => {
            let solvers = engine.list_solvers();
            crate::output::json_or(&solvers, || {
                if solvers.is_empty() {
                    println!("No solvers registered.");
                } else {
                    println!("{}", "Solvers".bold().cyan());
                    for s in &solvers {
                        println!(
                            "  {} — {} | success={} failed={} savings={:.1}%",
                            s.id, s.name, s.success_count, s.failure_count, s.avg_savings_percent
                        );
                    }
                }
            });
        }
        IntentAction::Stats => {
            let stats = engine.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Intent Stats".bold().cyan());
                println!("  Total Intents:   {}", stats.total_intents);
                println!("  Open:            {}", stats.open);
                println!("  Completed:       {}", stats.completed);
                println!("  Failed:          {}", stats.failed);
                println!("  Total Solutions: {}", stats.total_solutions);
                println!("  Total Solvers:   {}", stats.total_solvers);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Token Registry ─────────────────────────

fn cmd_token_registry(action: TokenRegistryAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("token_registry.json");
    let mut registry = crate::token_registry::TokenRegistry::load_or_default(&path)?;

    match action {
        TokenRegistryAction::Register { address, name, symbol, decimals } => {
            let token = crate::token_registry::TokenInfo::new(&address, &name, &symbol, decimals);
            registry.register(token)?;
            registry.save(&path)?;
            crate::output::json_or(&serde_json::json!({"registered": address}), || {
                println!("Token registered: {} ({}) at {}", name, symbol, address);
            });
        }
        TokenRegistryAction::List => {
            let tokens: Vec<&crate::token_registry::TokenInfo> = registry.tokens.values().collect();
            crate::output::json_or(&tokens, || {
                if tokens.is_empty() {
                    println!("No tokens registered.");
                } else {
                    println!("{}", "Token Registry".bold().cyan());
                    for t in &tokens {
                        let flags = if t.verified { " [verified]" } else if t.flagged_scam { " [SCAM]" } else { "" };
                        println!("  {} ({}) dec={}{} — {}", t.name, t.symbol, t.decimals, flags, t.address);
                    }
                }
            });
        }
        TokenRegistryAction::Show { address } => {
            match registry.get(&address) {
                Some(t) => {
                    crate::output::json_or(&t, || {
                        println!("{}", "Token Detail".bold().cyan());
                        println!("  Address:   {}", t.address);
                        println!("  Name:      {}", t.name);
                        println!("  Symbol:    {}", t.symbol);
                        println!("  Decimals:  {}", t.decimals);
                        println!("  Verified:  {}", t.verified);
                        println!("  Scam:      {}", t.flagged_scam);
                        if let Some(ref url) = t.logo_url {
                            println!("  Logo:      {}", url);
                        }
                        if let Some(ref url) = t.website {
                            println!("  Website:   {}", url);
                        }
                        println!("  Tags:      {}", t.tags.join(", "));
                    });
                }
                None => println!("Token not found: {}", address),
            }
        }
        TokenRegistryAction::Remove { address } => {
            registry.remove(&address)?;
            registry.save(&path)?;
            println!("Token removed: {}", address);
        }
        TokenRegistryAction::Search { query } => {
            let results = registry.search(&query);
            crate::output::json_or(&results, || {
                if results.is_empty() {
                    println!("No tokens matching '{}'.", query);
                } else {
                    println!("{}", format!("Search: '{}' ({} results)", query, results.len()).bold().cyan());
                    for t in &results {
                        println!("  {} ({}) — {}", t.name, t.symbol, t.address);
                    }
                }
            });
        }
        TokenRegistryAction::Verify { address } => {
            registry.verify_token(&address)?;
            registry.save(&path)?;
            println!("Token {} marked as verified.", address);
        }
        TokenRegistryAction::Flag { address, reason } => {
            registry.flag_token(&address, &reason)?;
            registry.save(&path)?;
            println!("Token {} flagged as scam: {}", address, reason);
        }
        TokenRegistryAction::Stats => {
            let stats = registry.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Token Registry Stats".bold().cyan());
                println!("  Total:       {}", stats.total);
                println!("  Verified:    {}", stats.verified);
                println!("  Flagged:     {}", stats.flagged);
                println!("  Unique Tags: {}", stats.unique_tags);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Fee Bumper ──────────────────────────────

fn cmd_fee_bump(action: FeeBumpAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("fee_bumper.json");
    let mut bumper = crate::fee_bumper::FeeBumper::load_or_default(&path);

    match action {
        FeeBumpAction::Track { tx_hash, sender, nonce, fee } => {
            let tx = crate::fee_bumper::TrackedTx::new(&tx_hash, &sender, nonce, fee);
            bumper.track(tx)?;
            bumper.save(&path)?;
            crate::output::json_or(&serde_json::json!({"tracked": tx_hash}), || {
                println!("Tracking tx: {} (fee={})", tx_hash, fee);
            });
        }
        FeeBumpAction::List => {
            let txs: Vec<&crate::fee_bumper::TrackedTx> = bumper.tracked.values().collect();
            crate::output::json_or(&txs, || {
                if txs.is_empty() {
                    println!("No tracked transactions.");
                } else {
                    println!("{}", "Tracked Transactions".bold().cyan());
                    for t in &txs {
                        println!(
                            "  {} [{:?}] fee={} bumps={} sender={}",
                            t.tx_hash, t.state, t.current_fee, t.bump_count, t.sender
                        );
                    }
                }
            });
        }
        FeeBumpAction::Show { tx_hash } => {
            match bumper.get(&tx_hash) {
                Some(t) => {
                    crate::output::json_or(&t, || {
                        println!("{}", "Transaction Detail".bold().cyan());
                        println!("  Hash:         {}", t.tx_hash);
                        println!("  Sender:       {}", t.sender);
                        println!("  Nonce:        {}", t.nonce);
                        println!("  Original Fee: {}", t.original_fee);
                        println!("  Current Fee:  {}", t.current_fee);
                        println!("  State:        {:?}", t.state);
                        println!("  Bumps:        {}/{}", t.bump_count, t.max_bumps);
                        println!("  Submitted:    {}", t.submitted_at);
                        println!("  Fee Increase: +{}", t.total_fee_increase());
                    });
                }
                None => println!("Transaction not found: {}", tx_hash),
            }
        }
        FeeBumpAction::DetectStuck => {
            let stuck = bumper.detect_stuck();
            bumper.save(&path)?;
            crate::output::json_or(&stuck, || {
                if stuck.is_empty() {
                    println!("No stuck transactions detected.");
                } else {
                    println!("{}", format!("{} stuck transaction(s):", stuck.len()).bold().yellow());
                    for hash in &stuck {
                        println!("  {}", hash);
                    }
                }
            });
        }
        FeeBumpAction::Bump { tx_hash } => {
            let new_fee = bumper.bump(&tx_hash, None)?;
            bumper.save(&path)?;
            println!("Fee bumped for {}: new_fee={}", tx_hash, new_fee);
        }
        FeeBumpAction::BumpAll => {
            let results = bumper.bump_all_stuck();
            bumper.save(&path)?;
            crate::output::json_or(&results, || {
                if results.is_empty() {
                    println!("No stuck transactions to bump.");
                } else {
                    println!("{}", format!("Bumped {} transaction(s):", results.len()).bold().cyan());
                    for (hash, fee) in &results {
                        println!("  {} → fee={}", hash, fee);
                    }
                }
            });
        }
        FeeBumpAction::Cleanup => {
            let count = bumper.cleanup_confirmed();
            bumper.save(&path)?;
            println!("Removed {} confirmed transaction(s).", count);
        }
        FeeBumpAction::Stats => {
            let stats = bumper.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Fee Bumper Stats".bold().cyan());
                println!("  Tracked:    {}", stats.total_tracked);
                println!("  Pending:    {}", stats.pending);
                println!("  Stuck:      {}", stats.stuck);
                println!("  Confirmed:  {}", stats.confirmed);
                println!("  Failed:     {}", stats.failed);
                println!("  Total Bumps:{}", stats.total_bumps);
                println!("  Fee Spent:  {}", stats.total_fee_spent);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Snapshot ────────────────────────────────

fn cmd_snapshot(action: SnapshotAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("snapshots.json");
    let mut store = crate::snapshot::SnapshotStore::load_or_default(&path);

    match action {
        SnapshotAction::Capture { label } => {
            // Capture key wallet state
            let config_path = crate::config::WalletConfig::default_path();
            let config = crate::config::WalletConfig::load_or_default(&config_path)?;
            let mut entries: Vec<(String, String, String)> = vec![
                ("config.node_url".into(), config.node_url.clone(), "config".into()),
                ("config.default_account".into(), config.active_account.clone().unwrap_or_default(), "config".into()),
            ];
            let id = store.capture(&label, entries);
            store.save(&path)?;
            crate::output::json_or(&serde_json::json!({"captured": id}), || {
                println!("Snapshot captured: {} ({})", id, label);
            });
        }
        SnapshotAction::List => {
            let snaps = store.list();
            crate::output::json_or(&snaps, || {
                if snaps.is_empty() {
                    println!("No snapshots.");
                } else {
                    println!("{}", "Snapshots".bold().cyan());
                    for s in snaps {
                        println!("  {} — {} ({} entries, {}B)", s.id, s.label, s.entry_count(), s.size_bytes);
                    }
                }
            });
        }
        SnapshotAction::Show { id } => {
            match store.get(&id) {
                Some(s) => {
                    crate::output::json_or(&s, || {
                        println!("{}", "Snapshot Detail".bold().cyan());
                        println!("  ID:       {}", s.id);
                        println!("  Label:    {}", s.label);
                        println!("  Created:  {}", s.created_at);
                        println!("  Entries:  {}", s.entry_count());
                        println!("  Size:     {}B", s.size_bytes);
                        println!("  Checksum: {}", s.checksum);
                        println!("  Valid:    {}", s.verify());
                    });
                }
                None => println!("Snapshot not found: {}", id),
            }
        }
        SnapshotAction::Diff { from, to } => {
            let diff = store.diff(&from, &to)?;
            crate::output::json_or(&diff, || {
                println!("{}", "Snapshot Diff".bold().cyan());
                println!("  {}", diff.summary());
                for k in &diff.added {
                    println!("  + {}", k);
                }
                for k in &diff.removed {
                    println!("  - {}", k);
                }
                for (k, old, new) in &diff.changed {
                    println!("  ~ {} : {} → {}", k, old, new);
                }
            });
        }
        SnapshotAction::Remove { id } => {
            store.remove(&id)?;
            store.save(&path)?;
            println!("Snapshot removed: {}", id);
        }
        SnapshotAction::Stats => {
            let count = store.list().len();
            let total_size = store.total_size();
            let stats = serde_json::json!({
                "count": count,
                "total_size_bytes": total_size,
            });
            crate::output::json_or(&stats, || {
                println!("{}", "Snapshot Stats".bold().cyan());
                println!("  Count:      {}", count);
                println!("  Total Size: {}B", total_size);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Watch-Only ──────────────────────────────

fn cmd_watchonly(action: WatchOnlyAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("watchonly.json");
    let mut store = crate::watchonly::WatchStore::load_or_default(&path);

    match action {
        WatchOnlyAction::Add { address, label } => {
            let account = crate::watchonly::WatchedAccount::new(&address, &label);
            store.watch(account)?;
            store.save(&path)?;
            crate::output::json_or(&serde_json::json!({"watching": address}), || {
                println!("Now watching: {} ({})", address, label);
            });
        }
        WatchOnlyAction::Remove { address } => {
            store.unwatch(&address)?;
            store.save(&path)?;
            println!("Stopped watching: {}", address);
        }
        WatchOnlyAction::List => {
            let accounts = store.list();
            crate::output::json_or(&accounts, || {
                if accounts.is_empty() {
                    println!("No watched accounts.");
                } else {
                    println!("{}", "Watched Accounts".bold().cyan());
                    for a in &accounts {
                        println!(
                            "  {} ({}) balance={} [{:?}]",
                            a.address, a.label, a.last_balance, a.priority
                        );
                    }
                }
            });
        }
        WatchOnlyAction::Show { address } => {
            match store.get(&address) {
                Some(a) => {
                    crate::output::json_or(&a, || {
                        println!("{}", "Watched Account".bold().cyan());
                        println!("  Address:   {}", a.address);
                        println!("  Label:     {}", a.label);
                        println!("  Balance:   {}", a.last_balance);
                        println!("  Priority:  {:?}", a.priority);
                        println!("  Threshold: {}", a.alert_threshold);
                        println!("  Alerts:    {}", if a.alerts_enabled { "on" } else { "off" });
                        println!("  Active:    {}", a.active);
                        println!("  Added:     {}", a.added_at);
                        println!("  History:   {} snapshots", a.balance_history.len());
                    });
                }
                None => println!("Not watching: {}", address),
            }
        }
        WatchOnlyAction::UpdateBalance { address, balance } => {
            match store.update_balance(&address, balance)? {
                Some(alert) => {
                    store.save(&path)?;
                    println!("Balance updated. Alert: {}", alert.message);
                }
                None => {
                    store.save(&path)?;
                    println!("Balance updated for {}.", address);
                }
            }
        }
        WatchOnlyAction::Alerts => {
            let alerts = store.unread_alerts();
            crate::output::json_or(&alerts, || {
                if alerts.is_empty() {
                    println!("No unread alerts.");
                } else {
                    println!("{}", format!("{} Unread Alert(s)", alerts.len()).bold().yellow());
                    for a in &alerts {
                        println!("  [{}] {} — {}", a.created_at, a.address, a.message);
                    }
                }
            });
        }
        WatchOnlyAction::MarkRead => {
            store.mark_all_read();
            store.save(&path)?;
            println!("All alerts marked as read.");
        }
        WatchOnlyAction::Stats => {
            let stats = store.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Watch Stats".bold().cyan());
                println!("  Accounts:      {}", stats.total_accounts);
                println!("  Active:        {}", stats.active);
                println!("  Disabled:      {}", stats.disabled);
                println!("  Total Balance: {}", stats.total_balance);
                println!("  Alerts:        {}", stats.total_alerts);
                println!("  Unread:        {}", stats.unread_alerts);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Peers ───────────────────────────────────

fn cmd_peers(action: PeerAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("peers.json");
    let mut registry = crate::peer_discovery::PeerRegistry::load_or_default(&path);

    match action {
        PeerAction::Add { url, name } => {
            let peer = crate::peer_discovery::Peer::new(&url, &name);
            registry.add_peer(peer)?;
            registry.save(&path)?;
            println!("Peer added: {} ({})", name, url);
        }
        PeerAction::Remove { url } => {
            registry.remove_peer(&url)?;
            registry.save(&path)?;
            println!("Peer removed: {}", url);
        }
        PeerAction::List => {
            let peers = registry.list();
            crate::output::json_or(&peers, || {
                if peers.is_empty() {
                    println!("No peers configured.");
                } else {
                    println!("{}", "Peers".bold().cyan());
                    for p in &peers {
                        println!(
                            "  {} ({}) [{:?}] score={:.2} latency={}ms reliability={:.1}%",
                            p.name, p.url, p.status, p.score(), p.latency_ms, p.reliability() * 100.0
                        );
                    }
                }
            });
        }
        PeerAction::Best => {
            match registry.best_peer() {
                Some(p) => {
                    crate::output::json_or(&p, || {
                        println!("{}", "Best Peer".bold().cyan());
                        println!("  Name:        {}", p.name);
                        println!("  URL:         {}", p.url);
                        println!("  Score:       {:.2}", p.score());
                        println!("  Latency:     {}ms", p.latency_ms);
                        println!("  Reliability: {:.1}%", p.reliability() * 100.0);
                    });
                }
                None => println!("No available peers."),
            }
        }
        PeerAction::RecordSuccess { url, latency_ms } => {
            registry.record_success(&url, latency_ms)?;
            registry.save(&path)?;
            println!("Recorded success for {} ({}ms)", url, latency_ms);
        }
        PeerAction::RecordFailure { url } => {
            registry.record_failure(&url)?;
            registry.save(&path)?;
            println!("Recorded failure for {}", url);
        }
        PeerAction::UnbanAll => {
            let count = registry.unban_all();
            registry.save(&path)?;
            println!("Unbanned {} peer(s).", count);
        }
        PeerAction::Stats => {
            let stats = registry.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Peer Stats".bold().cyan());
                println!("  Total:      {}", stats.total);
                println!("  Active:     {}", stats.active);
                println!("  Degraded:   {}", stats.degraded);
                println!("  Down:       {}", stats.down);
                println!("  Banned:     {}", stats.banned);
                println!("  Avg Latency:{}ms", stats.avg_latency_ms);
                println!("  Failovers:  {}", stats.failover_count);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Mempool ─────────────────────────────────

fn cmd_mempool(action: MempoolAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("mempool.json");
    let mut monitor = crate::mempool_monitor::MempoolMonitor::load_or_default(&path);

    match action {
        MempoolAction::Add { tx_hash, sender, receiver, amount, fee, nonce } => {
            let tx = crate::mempool_monitor::PendingTx::new(tx_hash.clone(), sender, receiver, amount, fee, nonce, "transfer".into());
            monitor.add_tx(tx)?;
            monitor.save(&path)?;
            println!("Tracking pending tx: {} (fee={})", tx_hash, fee);
        }
        MempoolAction::Remove { tx_hash } => {
            monitor.remove_tx(&tx_hash)?;
            monitor.save(&path)?;
            println!("Removed tx: {}", tx_hash);
        }
        MempoolAction::List => {
            let txs: Vec<&crate::mempool_monitor::PendingTx> = monitor.pending.values().collect();
            crate::output::json_or(&txs, || {
                if txs.is_empty() {
                    println!("Mempool empty.");
                } else {
                    println!("{}", format!("Mempool ({} pending)", txs.len()).bold().cyan());
                    for t in &txs {
                        println!("  {} {} → {} amt={} fee={} [{:?}]", t.tx_hash, t.sender, t.receiver, t.amount, t.fee, t.priority);
                    }
                }
            });
        }
        MempoolAction::FrontRun { victim, attacker } => {
            let risk = monitor.detect_front_run(&victim, &attacker)?;
            monitor.save(&path)?;
            crate::output::json_or(&serde_json::json!({"risk": format!("{:?}", risk)}), || {
                println!("Front-run risk: {:?}", risk);
            });
        }
        MempoolAction::RecommendFee { priority } => {
            let p = match priority.to_lowercase().as_str() {
                "urgent" => crate::mempool_monitor::TxPriority::Urgent,
                "high" => crate::mempool_monitor::TxPriority::High,
                "medium" => crate::mempool_monitor::TxPriority::Medium,
                "low" => crate::mempool_monitor::TxPriority::Low,
                _ => {
                    println!("Unknown priority: {} (use urgent/high/medium/low)", priority);
                    return Ok(());
                }
            };
            let fee = monitor.recommend_fee(&p);
            crate::output::json_or(&serde_json::json!({"priority": priority, "recommended_fee": fee}), || {
                println!("Recommended fee ({:?}): {}", p, fee);
            });
        }
        MempoolAction::Congestion => {
            let level = monitor.congestion();
            crate::output::json_or(&serde_json::json!({"congestion": format!("{:?}", level)}), || {
                println!("Congestion: {:?}", level);
            });
        }
        MempoolAction::Stats => {
            let stats = monitor.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Mempool Stats".bold().cyan());
                println!("  Pending:    {}", stats.pending_count);
                println!("  Total Fees: {}", stats.total_fees);
                println!("  Avg Fee:    {}", stats.avg_fee);
                println!("  Removed:    {}", stats.removed_count);
                println!("  Alerts:     {}", stats.alert_count);
                println!("  Congestion: {:?}", stats.congestion);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Indexer ─────────────────────────────────

fn cmd_indexer(action: IndexerAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("chain_indexer.json");
    let indexer = crate::chain_indexer::ChainIndexer::load_or_default(&path);

    match action {
        IndexerAction::Query { from, to, event_type: _, min_block, max_block } => {
            let mut filter = crate::chain_indexer::EventFilter::new();
            if let Some(ref addr) = from {
                filter = filter.with_from(addr);
            }
            if let Some(ref addr) = to {
                filter = filter.with_to(addr);
            }
            if let (Some(min), Some(max)) = (min_block, max_block) {
                filter = filter.with_block_range(min, max);
            }
            let events = indexer.query_events(&filter);
            crate::output::json_or(&events, || {
                if events.is_empty() {
                    println!("No events matching filter.");
                } else {
                    println!("{}", format!("{} Event(s)", events.len()).bold().cyan());
                    for e in &events {
                        println!(
                            "  [{}] {:?} block={} tx={} from={}",
                            e.id, e.event_type, e.block_height, e.tx_hash, e.from_address
                        );
                    }
                }
            });
        }
        IndexerAction::Receipt { tx_hash } => {
            match indexer.get_receipt(&tx_hash) {
                Some(r) => {
                    crate::output::json_or(&r, || {
                        println!("{}", "Transaction Receipt".bold().cyan());
                        println!("  Hash:    {}", r.tx_hash);
                        println!("  Block:   {}", r.block_height);
                        println!("  Status:  {:?}", r.status);
                        println!("  Gas:     {}", r.gas_used);
                        println!("  Fee:     {}", r.fee_paid);
                        if let Some(ref err) = r.error_message {
                            println!("  Error:   {}", err);
                        }
                    });
                }
                None => println!("Receipt not found: {}", tx_hash),
            }
        }
        IndexerAction::Latest => {
            match indexer.latest_block() {
                Some(b) => {
                    crate::output::json_or(&b, || {
                        println!("{}", "Latest Indexed Block".bold().cyan());
                        println!("  Height: {}", b.height);
                        println!("  Hash:   {}", b.hash);
                        println!("  Txs:    {}", b.tx_count);
                        println!("  Events: {}", b.event_count);
                        println!("  Time:   {}", b.timestamp);
                    });
                }
                None => println!("No blocks indexed yet."),
            }
        }
        IndexerAction::Stats => {
            let stats = indexer.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Indexer Stats".bold().cyan());
                println!("  Events:       {}", stats.total_events);
                println!("  Receipts:     {}", stats.total_receipts);
                println!("  Blocks:       {}", stats.total_blocks);
                println!("  Last Block:   {}", stats.last_indexed_block);
                println!("  Success Txs:  {}", stats.success_receipts);
                println!("  Failed Txs:   {}", stats.failed_receipts);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Network Health ──────────────────────────

fn cmd_net_health(action: NetHealthAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("network_health.json");
    let monitor = crate::network_health::NetworkHealthMonitor::load_or_default(&path);

    match action {
        NetHealthAction::Grade => {
            let grade = monitor.health_grade();
            let desc = crate::network_health::NetworkHealthMonitor::grade_description(&grade);
            crate::output::json_or(&serde_json::json!({"grade": format!("{:?}", grade), "description": desc}), || {
                println!("{}", "Network Health".bold().cyan());
                println!("  Grade: {:?}", grade);
                println!("  {}", desc);
            });
        }
        NetHealthAction::BlockTimes => {
            let avg = monitor.avg_block_time();
            let median = monitor.median_block_time();
            crate::output::json_or(&serde_json::json!({"avg_ms": avg, "median_ms": median}), || {
                println!("{}", "Block Times".bold().cyan());
                println!("  Average: {}ms", avg);
                println!("  Median:  {}ms", median);
            });
        }
        NetHealthAction::Reorgs => {
            let count = monitor.reorg_count();
            let max_depth = monitor.max_reorg_depth();
            crate::output::json_or(&serde_json::json!({"count": count, "max_depth": max_depth}), || {
                println!("{}", "Reorg History".bold().cyan());
                println!("  Total Reorgs: {}", count);
                println!("  Max Depth:    {}", max_depth);
            });
        }
        NetHealthAction::Epoch { expected_blocks } => {
            let progress = monitor.epoch_progress(expected_blocks);
            match monitor.current_epoch_info() {
                Some(e) => {
                    crate::output::json_or(&serde_json::json!({
                        "epoch": e.epoch,
                        "progress": format!("{:.1}%", progress * 100.0),
                        "block_count": e.block_count,
                    }), || {
                        println!("{}", "Epoch Progress".bold().cyan());
                        println!("  Epoch:    {}", e.epoch);
                        println!("  Progress: {:.1}%", progress * 100.0);
                        println!("  Blocks:   {}", e.block_count);
                    });
                }
                None => println!("No epoch data available."),
            }
        }
        NetHealthAction::Events { count } => {
            let events = monitor.recent_events(count);
            crate::output::json_or(&events, || {
                if events.is_empty() {
                    println!("No network events.");
                } else {
                    println!("{}", format!("Recent Events ({})", events.len()).bold().cyan());
                    for (ts, ev) in &events {
                        println!("  [{}] {:?}", ts, ev);
                    }
                }
            });
        }
        NetHealthAction::Stats => {
            let stats = monitor.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Network Stats".bold().cyan());
                println!("  Height:       {}", stats.current_height);
                println!("  Epoch:        {}", stats.current_epoch);
                println!("  Avg Block:    {}ms", stats.avg_block_time_ms);
                println!("  Median Block: {}ms", stats.median_block_time_ms);
                println!("  Avg TPS:      {:.2}", stats.avg_tps);
                println!("  Peers:        {}", stats.peer_count);
                println!("  Reorgs:       {}", stats.reorg_count);
                println!("  Health:       {:?}", stats.health_grade);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Price Feed ──────────────────────────────

fn cmd_price(action: PriceAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("price_feed.json");
    let mut feed = crate::price_feed::PriceFeed::load_or_default(&path);

    match action {
        PriceAction::Register { token_id, symbol, price } => {
            let token = crate::price_feed::TokenPrice::new(&token_id, &symbol, price, crate::price_feed::Currency::Usd);
            feed.register_token(token);
            feed.save(&path)?;
            println!("Token registered: {} ({}) @ ${:.4}", token_id, symbol, price);
        }
        PriceAction::Update { token_id, price } => {
            feed.update_price(&token_id, price, 0.0)?;
            feed.save(&path)?;
            println!("Price updated: {} → ${:.4}", token_id, price);
        }
        PriceAction::Show { token_id } => {
            match feed.get_price(&token_id) {
                Some(t) => {
                    crate::output::json_or(&t, || {
                        println!("{}", "Token Price".bold().cyan());
                        println!("  Token:    {} ({})", t.token_id, t.symbol);
                        println!("  Price:    ${:.4}", t.current_price);
                        println!("  24h:      {:.2}%", t.change_24h);
                        println!("  High:     ${:.4}", t.high_24h);
                        println!("  Low:      ${:.4}", t.low_24h);
                        println!("  Avg:      ${:.4}", t.average_price());
                        println!("  Trend:    {:.2}%", t.trend());
                        println!("  History:  {} points", t.history.len());
                    });
                }
                None => println!("Token not found: {}", token_id),
            }
        }
        PriceAction::List => {
            let tokens = feed.list_tokens();
            crate::output::json_or(&tokens, || {
                if tokens.is_empty() {
                    println!("No tokens tracked.");
                } else {
                    println!("{}", "Price Feed".bold().cyan());
                    for t in &tokens {
                        println!("  {} ({}) ${:.4} [{:+.2}%]", t.token_id, t.symbol, t.current_price, t.change_24h);
                    }
                }
            });
        }
        PriceAction::Alert { token_id, above, below } => {
            let condition = if let Some(v) = above {
                crate::price_feed::PriceAlertCondition::Above(v)
            } else if let Some(v) = below {
                crate::price_feed::PriceAlertCondition::Below(v)
            } else {
                println!("Specify --above or --below");
                return Ok(());
            };
            let id = format!("pa_{}", chrono::Utc::now().timestamp_millis());
            let alert = crate::price_feed::PriceAlert::new(&id, &token_id, condition);
            feed.add_alert(alert);
            feed.save(&path)?;
            println!("Alert created: {}", id);
        }
        PriceAction::CheckAlerts => {
            let triggered = feed.check_alerts();
            feed.save(&path)?;
            if triggered.is_empty() {
                println!("No alerts triggered.");
            } else {
                println!("{}", format!("{} alert(s) triggered:", triggered.len()).bold().yellow());
                for id in &triggered {
                    println!("  {}", id);
                }
            }
        }
        PriceAction::Portfolio => {
            let holdings: Vec<(String, f64)> = feed.list_tokens().iter().map(|t| (t.token_id.clone(), 1.0)).collect();
            let val = feed.valuate_portfolio(&holdings);
            crate::output::json_or(&val, || {
                println!("{}", "Portfolio Valuation".bold().cyan());
                println!("  Total: ${:.2}", val.total_value);
                for (sym, amt, value) in &val.holdings {
                    println!("  {} × {:.4} = ${:.2}", sym, amt, value);
                }
            });
        }
        PriceAction::Movers { count } => {
            let gainers = feed.top_gainers(count);
            let losers = feed.top_losers(count);
            crate::output::json_or(&serde_json::json!({"gainers": gainers, "losers": losers}), || {
                println!("{}", "Top Gainers".bold().green());
                for t in &gainers {
                    println!("  {} {:+.2}%", t.symbol, t.change_24h);
                }
                println!("{}", "Top Losers".bold().red());
                for t in &losers {
                    println!("  {} {:+.2}%", t.symbol, t.change_24h);
                }
            });
        }
        PriceAction::Stats => {
            let stats = feed.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Price Feed Stats".bold().cyan());
                println!("  Tokens:     {}", stats.total_tokens);
                println!("  Alerts:     {}", stats.total_alerts);
                println!("  Active:     {}", stats.active_alerts);
                println!("  Triggered:  {}", stats.triggered_alerts);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Risk Scoring ────────────────────────────

fn cmd_risk(action: RiskAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("address_scoring.json");
    let mut scorer = crate::address_scoring::AddressScorer::load_or_default(&path);

    match action {
        RiskAction::Score { address } => {
            let profile = scorer.score_address(&address);
            crate::output::json_or(&serde_json::json!({
                "address": profile.address,
                "risk_level": format!("{:?}", profile.risk_level),
                "risk_score": profile.risk_score,
            }), || {
                println!("{}", "Risk Score".bold().cyan());
                println!("  Address: {}", profile.address);
                println!("  Level:   {:?}", profile.risk_level);
                println!("  Score:   {}/100", profile.risk_score);
                if !profile.factors.is_empty() {
                    println!("  Factors: {:?}", profile.factors);
                }
            });
            scorer.save(&path)?;
        }
        RiskAction::Show { address } => {
            match scorer.get_profile(&address) {
                Some(p) => {
                    crate::output::json_or(&p, || {
                        println!("{}", "Address Profile".bold().cyan());
                        println!("  Address:  {}", p.address);
                        println!("  Level:    {:?}", p.risk_level);
                        println!("  Score:    {}/100", p.risk_score);
                        println!("  Factors:  {:?}", p.factors);
                        println!("  Labels:   {}", p.labels.join(", "));
                        println!("  Txs:      {}", p.tx_count);
                        println!("  Volume:   {}", p.total_volume);
                        println!("  Verified: {}", p.verified);
                    });
                }
                None => println!("No profile for: {}", address),
            }
        }
        RiskAction::Blacklist { address } => {
            scorer.add_to_blacklist(&address);
            scorer.save(&path)?;
            println!("Blacklisted: {}", address);
        }
        RiskAction::Unblacklist { address } => {
            if scorer.remove_from_blacklist(&address) {
                scorer.save(&path)?;
                println!("Removed from blacklist: {}", address);
            } else {
                println!("Not blacklisted: {}", address);
            }
        }
        RiskAction::Risky => {
            let risky = scorer.risky_addresses();
            crate::output::json_or(&risky, || {
                if risky.is_empty() {
                    println!("No risky addresses.");
                } else {
                    println!("{}", "Risky Addresses".bold().red());
                    for p in &risky {
                        println!("  {} [{:?}] score={}", p.address, p.risk_level, p.risk_score);
                    }
                }
            });
        }
        RiskAction::Stats => {
            let stats = scorer.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Scoring Stats".bold().cyan());
                println!("  Profiles:    {}", stats.total_profiles);
                println!("  Safe:        {}", stats.safe);
                println!("  Low:         {}", stats.low);
                println!("  Medium:      {}", stats.medium);
                println!("  High:        {}", stats.high);
                println!("  Critical:    {}", stats.critical);
                println!("  Blacklisted: {}", stats.blacklisted);
                println!("  Rules:       {}", stats.rules);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Tx Decoder ──────────────────────────────

fn cmd_decode(action: DecodeAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("tx_decoder.json");
    let mut decoder = crate::tx_decoder::TxDecoder::load_or_default(&path);
    decoder.register_defaults();

    match action {
        DecodeAction::Tx { tx_hash, selector, from, to, value } => {
            let decoded = decoder.decode(&tx_hash, &selector, &from, to.as_deref(), value, 0, &[]);
            decoder.save(&path)?;
            crate::output::json_or(&decoded, || {
                println!("{}", decoded.display());
            });
        }
        DecodeAction::Methods => {
            let methods = decoder.list_methods();
            crate::output::json_or(&methods, || {
                println!("{}", "Registered Methods".bold().cyan());
                for m in &methods {
                    println!("  {} → {}", m.selector, m.display());
                }
            });
        }
        DecodeAction::Contracts => {
            let contracts = decoder.list_contracts();
            crate::output::json_or(&serde_json::json!(contracts.iter().map(|(a,n)| serde_json::json!({"address": a, "name": n})).collect::<Vec<_>>()), || {
                if contracts.is_empty() {
                    println!("No known contracts.");
                } else {
                    println!("{}", "Known Contracts".bold().cyan());
                    for (addr, name) in &contracts {
                        println!("  {} → {}", addr, name);
                    }
                }
            });
        }
        DecodeAction::RegisterContract { address, name } => {
            decoder.register_contract(&address, &name);
            decoder.save(&path)?;
            println!("Contract registered: {} → {}", address, name);
        }
        DecodeAction::Stats => {
            let stats = decoder.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Decoder Stats".bold().cyan());
                println!("  Methods:   {}", stats.total_methods);
                println!("  Contracts: {}", stats.total_contracts);
                println!("  Cached:    {}", stats.cached_decodings);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Notification Rules ─────────────────────

fn cmd_rules(action: RulesAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("notification_rules.json");
    let mut engine = crate::notification_rules::RuleEngine::load_or_default(&path);

    match action {
        RulesAction::Add { id, name } => {
            let rule = crate::notification_rules::NotificationRule::new(&id, &name);
            engine.add_rule(rule)?;
            engine.save(&path)?;
            println!("Rule added: {} ({})", id, name);
        }
        RulesAction::Remove { id } => {
            engine.remove_rule(&id)?;
            engine.save(&path)?;
            println!("Rule removed: {}", id);
        }
        RulesAction::List => {
            let rules = engine.list_rules();
            crate::output::json_or(&rules, || {
                if rules.is_empty() {
                    println!("No notification rules.");
                } else {
                    println!("{}", "Notification Rules".bold().cyan());
                    for r in &rules {
                        let status = if r.enabled { "on" } else { "off" };
                        println!("  {} ({}) [{:?}] {} — triggers={}", r.id, r.name, r.priority, status, r.trigger_count);
                    }
                }
            });
        }
        RulesAction::Show { id } => {
            match engine.get_rule(&id) {
                Some(r) => {
                    crate::output::json_or(&r, || {
                        println!("{}", "Rule Detail".bold().cyan());
                        println!("  ID:         {}", r.id);
                        println!("  Name:       {}", r.name);
                        println!("  Priority:   {:?}", r.priority);
                        println!("  Enabled:    {}", r.enabled);
                        println!("  Conditions: {}", r.conditions.len());
                        println!("  Actions:    {}", r.actions.len());
                        println!("  Channels:   {}", r.channels.len());
                        println!("  Triggers:   {}", r.trigger_count);
                        println!("  Cooldown:   {}s", r.cooldown_secs);
                    });
                }
                None => println!("Rule not found: {}", id),
            }
        }
        RulesAction::Enable { id } => {
            match engine.get_rule_mut(&id) {
                Some(r) => {
                    r.enable();
                    engine.save(&path)?;
                    println!("Rule {} enabled.", id);
                }
                None => println!("Rule not found: {}", id),
            }
        }
        RulesAction::Disable { id } => {
            match engine.get_rule_mut(&id) {
                Some(r) => {
                    r.disable();
                    engine.save(&path)?;
                    println!("Rule {} disabled.", id);
                }
                None => println!("Rule not found: {}", id),
            }
        }
        RulesAction::Stats => {
            let stats = engine.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Rule Engine Stats".bold().cyan());
                println!("  Total Rules:     {}", stats.total_rules);
                println!("  Active:          {}", stats.active_rules);
                println!("  Disabled:        {}", stats.disabled_rules);
                println!("  Notifications:   {}", stats.total_notifications);
                println!("  In Cooldown:     {}", stats.rules_in_cooldown);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Contract ABI ────────────────────────────

fn cmd_abi(action: AbiAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("contract_abi.json");
    let mut store = crate::contract_abi::AbiStore::load_or_default(&path);

    match action {
        AbiAction::Register { address, name } => {
            let abi = crate::contract_abi::ContractAbi::new(&address, &name);
            store.register(abi)?;
            store.save(&path)?;
            println!("Contract registered: {} ({})", name, address);
        }
        AbiAction::Remove { address } => {
            store.remove(&address)?;
            store.save(&path)?;
            println!("Contract removed: {}", address);
        }
        AbiAction::List => {
            let contracts = store.list();
            crate::output::json_or(&contracts, || {
                if contracts.is_empty() {
                    println!("No contracts registered.");
                } else {
                    println!("{}", "Contract ABIs".bold().cyan());
                    for c in &contracts {
                        let v = if c.verified { " [verified]" } else { "" };
                        println!("  {} ({}) — {} entries{}", c.name, c.address, c.entry_count(), v);
                    }
                }
            });
        }
        AbiAction::Show { address } => {
            match store.get(&address) {
                Some(c) => {
                    crate::output::json_or(&c, || {
                        println!("{}", "Contract ABI".bold().cyan());
                        println!("  Address:   {}", c.address);
                        println!("  Name:      {}", c.name);
                        println!("  Verified:  {}", c.verified);
                        println!("  Functions: {}", c.functions().len());
                        println!("  Events:    {}", c.events().len());
                        for f in c.functions() {
                            println!("    fn {} [{:?}]", f.signature(), f.state_mutability);
                        }
                        for e in c.events() {
                            println!("    event {}", e.signature());
                        }
                    });
                }
                None => println!("Contract not found: {}", address),
            }
        }
        AbiAction::Search { query } => {
            let results = store.search(&query);
            crate::output::json_or(&results, || {
                if results.is_empty() {
                    println!("No contracts matching '{}'.", query);
                } else {
                    for c in &results {
                        println!("  {} ({})", c.name, c.address);
                    }
                }
            });
        }
        AbiAction::Stats => {
            let stats = store.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "ABI Store Stats".bold().cyan());
                println!("  Contracts:  {}", stats.total_contracts);
                println!("  Functions:  {}", stats.total_functions);
                println!("  Events:     {}", stats.total_events);
                println!("  Event Logs: {}", stats.total_event_logs);
                println!("  Verified:   {}", stats.verified_contracts);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Name Service ────────────────────────────

fn cmd_names(action: NameAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("name_service.json");
    let mut ns = crate::name_service::NameService::load_or_default(&path);

    match action {
        NameAction::Register { name, owner, expires } => {
            let reg = crate::name_service::RegisteredName::new(&name, &owner, &expires);
            ns.register(reg)?;
            ns.save(&path)?;
            println!("Name registered: {} → {}", name, owner);
        }
        NameAction::Resolve { name } => {
            match ns.resolve(&name) {
                Some(addr) => {
                    crate::output::json_or(&serde_json::json!({"name": name, "address": addr}), || {
                        println!("{} → {}", name, addr);
                    });
                }
                None => println!("Name not found or expired: {}", name),
            }
        }
        NameAction::Reverse { address } => {
            match ns.reverse_resolve(&address) {
                Some(name) => {
                    crate::output::json_or(&serde_json::json!({"address": address, "name": name}), || {
                        println!("{} → {}", address, name);
                    });
                }
                None => println!("No name for address: {}", address),
            }
        }
        NameAction::Transfer { name, new_owner } => {
            ns.transfer(&name, &new_owner)?;
            ns.save(&path)?;
            println!("Name {} transferred to {}", name, new_owner);
        }
        NameAction::List => {
            let names: Vec<&crate::name_service::RegisteredName> = ns.names.values().collect();
            crate::output::json_or(&names, || {
                if names.is_empty() {
                    println!("No registered names.");
                } else {
                    println!("{}", "Name Service".bold().cyan());
                    for n in &names {
                        let status = if n.is_active() { "active" } else { "expired" };
                        println!("  {} → {} [{}]", n.name, n.owner, status);
                    }
                }
            });
        }
        NameAction::Stats => {
            let stats = ns.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Name Service Stats".bold().cyan());
                println!("  Total:    {}", stats.total_names);
                println!("  Active:   {}", stats.active);
                println!("  Expired:  {}", stats.expired);
                println!("  Reserved: {}", stats.reserved);
                println!("  Records:  {}", stats.total_records);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Tx Preview ──────────────────────────────

fn cmd_preview(action: PreviewAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("tx_previewer.json");
    let mut previewer = crate::tx_preview::TxPreviewer::load_or_default(&path);

    match action {
        PreviewAction::Transfer { from, to, value, balance } => {
            let gas = crate::tx_preview::GasEstimate::new(1000, 100, 2000);
            let preview = previewer.preview_transfer(&from, &to, value, gas, balance);
            previewer.save(&path)?;
            crate::output::json_or(&preview, || {
                println!("{}", preview.display());
            });
        }
        PreviewAction::Recent { count } => {
            let previews = previewer.recent_previews(count);
            crate::output::json_or(&previews, || {
                if previews.is_empty() {
                    println!("No recent previews.");
                } else {
                    println!("{}", "Recent Previews".bold().cyan());
                    for p in &previews {
                        println!("  [{:?}] {} → {:?} val={} warnings={}", p.status, p.from_address, p.to_address, p.value, p.warning_count());
                    }
                }
            });
        }
        PreviewAction::Stats => {
            let stats = previewer.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Preview Stats".bold().cyan());
                println!("  Total:    {}", stats.total_previews);
                println!("  Safe:     {}", stats.safe);
                println!("  Caution:  {}", stats.caution);
                println!("  Warning:  {}", stats.warning);
                println!("  Danger:   {}", stats.danger);
                println!("  Known:    {}", stats.known_addresses);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── WalletConnect ───────────────────────────

fn cmd_connect(action: ConnectAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = std::path::Path::new(&dir).join("wallet_connect.json");
    let mut mgr = crate::wallet_connect::WalletConnectManager::load_or_default(&path);

    match action {
        ConnectAction::Sessions => {
            let sessions = mgr.active_sessions();
            crate::output::json_or(&sessions, || {
                if sessions.is_empty() {
                    println!("No active sessions.");
                } else {
                    println!("{}", "Active Sessions".bold().cyan());
                    for s in &sessions {
                        println!("  {} — {} ({}) reqs={}", s.id, s.dapp.name, s.dapp.url, s.request_count);
                    }
                }
            });
        }
        ConnectAction::Disconnect { id } => {
            mgr.disconnect_session(&id)?;
            mgr.save(&path)?;
            println!("Session {} disconnected.", id);
        }
        ConnectAction::Pending => {
            let reqs = mgr.pending_requests();
            crate::output::json_or(&reqs, || {
                if reqs.is_empty() {
                    println!("No pending requests.");
                } else {
                    println!("{}", "Pending Requests".bold().yellow());
                    for r in &reqs {
                        println!("  {} [{:?}] session={} from={}", r.id, r.request_type, r.session_id, r.from_address);
                    }
                }
            });
        }
        ConnectAction::Approve { id, result } => {
            mgr.approve_request(&id, &result)?;
            mgr.save(&path)?;
            println!("Request {} approved.", id);
        }
        ConnectAction::Reject { id, reason } => {
            mgr.reject_request(&id, &reason)?;
            mgr.save(&path)?;
            println!("Request {} rejected.", id);
        }
        ConnectAction::Cleanup => {
            let count = mgr.cleanup_expired();
            mgr.save(&path)?;
            println!("Cleaned up {} expired session(s).", count);
        }
        ConnectAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "WalletConnect Stats".bold().cyan());
                println!("  Sessions:     {}", stats.total_sessions);
                println!("  Active:       {}", stats.active);
                println!("  Expired:      {}", stats.expired);
                println!("  Disconnected: {}", stats.disconnected);
                println!("  Pending Reqs: {}", stats.pending_requests);
                println!("  Total Reqs:   {}", stats.total_requests);
            });
        }
    }
    Ok(())
}

// ──────────────────────── Tier 14 handlers ────────────────────────────

fn cmd_privacy(action: PrivacyAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::privacy_shield::{PrivacyShield, MixStrategy};
    let dir = crate::config::default_data_dir();
    let path = dir.join("privacy_shield.json");
    let mut shield = PrivacyShield::load_or_default(&path);

    match action {
        PrivacyAction::Stealth { public_key } => {
            let sa = shield.generate_stealth(&public_key);
            let addr = sa.one_time_key.clone();
            let ephemeral = sa.shared_secret.clone();
            shield.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "one_time_key": addr,
                "shared_secret": ephemeral,
            }), || {
                println!("{}", "Stealth Address Generated".bold().cyan());
                println!("  One-Time Key:   {}", addr);
                println!("  Shared Secret:  {}", ephemeral);
            });
        }
        PrivacyAction::Blind { amount } => {
            let ba = shield.blind_amount(amount);
            let commitment = ba.commitment.clone();
            shield.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "amount": amount,
                "commitment": commitment,
            }), || {
                println!("{}", "Amount Blinded".bold().cyan());
                println!("  Original:   {}", amount);
                println!("  Commitment: {}", commitment);
            });
        }
        PrivacyAction::Mix { amount, strategy } => {
            let strat = match strategy.as_str() {
                "multi" => MixStrategy::MultiHop(3),
                "timed" => MixStrategy::TimedDelay(60),
                "split" => MixStrategy::SplitAmount(4),
                _ => MixStrategy::SingleHop,
            };
            let id = shield.create_mix(amount, strat);
            shield.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "mix_id": id,
                "amount": amount,
                "strategy": strategy,
            }), || {
                println!("{}", "Mix Request Created".bold().cyan());
                println!("  ID:       {}", id);
                println!("  Amount:   {}", amount);
                println!("  Strategy: {}", strategy);
            });
        }
        PrivacyAction::Score { address } => {
            let score = shield.score_address(&address, 10, 5, false, false);
            shield.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "address": address,
                "privacy_score": score,
            }), || {
                println!("{}", "Privacy Score".bold().cyan());
                println!("  Address: {}", address);
                println!("  Score:   {}/100", score);
            });
        }
        PrivacyAction::Stats => {
            let stats = shield.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Privacy Shield Stats".bold().cyan());
                println!("  Stealth Generated: {}", stats.stealth_generated);
                println!("  Stealth Used:      {}", stats.stealth_used);
                println!("  Blinded Amounts:   {}", stats.blinded_amounts);
                println!("  Total Mixes:       {}", stats.total_mixes);
                println!("  Active Mixes:      {}", stats.active_mixes);
                println!("  Completed Mixes:   {}", stats.completed_mixes);
                println!("  Avg Privacy Score: {}", stats.avg_privacy_score);
            });
        }
    }
    Ok(())
}

fn cmd_key_rotation(action: KeyRotAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::key_rotation::{KeyRotationManager, ManagedKey, KeyType, RotationReason};
    let dir = crate::config::default_data_dir();
    let path = dir.join("key_rotation.json");
    let mut mgr = KeyRotationManager::load_or_default(&path);

    match action {
        KeyRotAction::Add { id, key_type, public_key } => {
            let kt = match key_type.to_lowercase().as_str() {
                "encryption" => KeyType::Encryption,
                "authentication" | "auth" => KeyType::Authentication,
                "derivation" => KeyType::Derivation,
                _ => KeyType::Signing,
            };
            let key = ManagedKey::new(&id, kt, &public_key);
            mgr.add_key(key)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "id": id,
                "key_type": key_type,
                "status": "active",
            }), || {
                println!("{}", "Key Added".bold().cyan());
                println!("  ID:   {}", id);
                println!("  Type: {}", key_type);
            });
        }
        KeyRotAction::Rotate { key_id, new_public_key } => {
            let new_id = mgr.rotate_key(&key_id, &new_public_key, RotationReason::Manual)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "old_key": key_id,
                "new_key": new_id,
            }), || {
                println!("{}", "Key Rotated".bold().cyan());
                println!("  Old Key: {}", key_id);
                println!("  New Key: {}", new_id);
            });
        }
        KeyRotAction::List => {
            let keys = mgr.active_keys();
            crate::output::json_or(&serde_json::json!({
                "active_keys": keys.len(),
            }), || {
                println!("{}", "Active Keys".bold().cyan());
                for k in &keys {
                    println!("  {} ({:?}) — {}", k.id, k.key_type, k.public_key);
                }
                if keys.is_empty() {
                    println!("  (none)");
                }
            });
        }
        KeyRotAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Key Rotation Stats".bold().cyan());
                println!("  Total Keys:  {}", stats.total_keys);
                println!("  Active:      {}", stats.active);
                println!("  Rotated:     {}", stats.rotated);
                println!("  Compromised: {}", stats.compromised);
                println!("  Revoked:     {}", stats.revoked);
                println!("  Rotations:   {}", stats.total_rotations);
                println!("  Policies:    {}", stats.policies);
            });
        }
    }
    Ok(())
}

fn cmd_access(action: AccessAction2) -> Result<(), Box<dyn std::error::Error>> {
    use crate::access_control::{AccessController, WalletUser, Role, Action};
    let dir = crate::config::default_data_dir();
    let path = dir.join("access_control.json");
    let mut ctrl = AccessController::load_or_default(&path);

    match action {
        AccessAction2::AddUser { id, name, role } => {
            let r = match role.to_lowercase().as_str() {
                "owner" => Role::Owner,
                "admin" => Role::Admin,
                "operator" => Role::Operator,
                _ => Role::Viewer,
            };
            let user = WalletUser::new(&id, &name, r);
            ctrl.add_user(user)?;
            ctrl.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "id": id,
                "name": name,
                "role": role,
            }), || {
                println!("{}", "User Added".bold().cyan());
                println!("  ID:   {}", id);
                println!("  Name: {}", name);
                println!("  Role: {}", role);
            });
        }
        AccessAction2::RemoveUser { id } => {
            let user = ctrl.remove_user(&id)?;
            ctrl.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "removed": user.id,
                "name": user.name,
            }), || {
                println!("Removed user '{}' ({}).", user.id, user.name);
            });
        }
        AccessAction2::Check { user_id, action } => {
            let act = match action.to_lowercase().as_str() {
                "transfer" => Action::Transfer,
                "sign" => Action::Sign,
                "view_balance" | "balance" => Action::ViewBalance,
                "view_history" | "history" => Action::ViewHistory,
                "manage_keys" | "keys" => Action::ManageKeys,
                "manage_contacts" | "contacts" => Action::ManageContacts,
                "configure" | "config" => Action::ConfigureWallet,
                "stake" => Action::Stake,
                "govern" => Action::Govern,
                "export" => Action::Export,
                _ => Action::ViewBalance,
            };
            let decision = ctrl.check_access(&user_id, &act)?;
            ctrl.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "user_id": user_id,
                "action": action,
                "decision": format!("{:?}", decision),
            }), || {
                println!("{}", "Access Check".bold().cyan());
                println!("  User:     {}", user_id);
                println!("  Action:   {}", action);
                println!("  Decision: {:?}", decision);
            });
        }
        AccessAction2::Users => {
            let users = ctrl.list_users();
            crate::output::json_or(&serde_json::json!({
                "users": users.len(),
            }), || {
                println!("{}", "Wallet Users".bold().cyan());
                for u in &users {
                    let status = if u.active { "active" } else { "inactive" };
                    println!("  {} — {} ({:?}, {})", u.id, u.name, u.role, status);
                }
                if users.is_empty() {
                    println!("  (none)");
                }
            });
        }
        AccessAction2::Stats => {
            let stats = ctrl.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Access Control Stats".bold().cyan());
                println!("  Total Users:      {}", stats.total_users);
                println!("  Active Users:     {}", stats.active_users);
                println!("  Roles Configured: {}", stats.roles_configured);
                println!("  Log Entries:      {}", stats.total_log_entries);
                println!("  Recent Denials:   {}", stats.recent_denials);
            });
        }
    }
    Ok(())
}

fn cmd_threats(action: ThreatAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::threat_monitor::ThreatMonitor;
    let dir = crate::config::default_data_dir();
    let path = dir.join("threat_monitor.json");
    let mut mon = ThreatMonitor::load_or_default(&path);

    match action {
        ThreatAction::CheckUrl { url } => {
            let level = mon.check_url(&url);
            crate::output::json_or(&serde_json::json!({
                "url": url,
                "threat_level": format!("{:?}", level),
            }), || {
                println!("{}", "URL Threat Check".bold().cyan());
                println!("  URL:   {}", url);
                println!("  Level: {:?}", level);
            });
        }
        ThreatAction::CheckContract { address } => {
            let level = mon.check_contract(&address);
            crate::output::json_or(&serde_json::json!({
                "address": address,
                "threat_level": format!("{:?}", level),
            }), || {
                println!("{}", "Contract Threat Check".bold().cyan());
                println!("  Address: {}", address);
                println!("  Level:   {:?}", level);
            });
        }
        ThreatAction::ReportPhishing { url } => {
            mon.report_phishing(&url, "cli_user");
            mon.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "reported": url,
                "type": "phishing",
            }), || {
                println!("Reported phishing URL: {}", url);
            });
        }
        ThreatAction::ReportContract { address, reason } => {
            use crate::threat_monitor::ThreatLevel;
            mon.report_malicious_contract(&address, &reason, ThreatLevel::High);
            mon.save(&path)?;
            crate::output::json_or(&serde_json::json!({
                "reported": address,
                "reason": reason,
                "type": "malicious_contract",
            }), || {
                println!("Reported malicious contract: {} ({})", address, reason);
            });
        }
        ThreatAction::Active => {
            let threats = mon.active_threats();
            crate::output::json_or(&serde_json::json!({
                "active_threats": threats.len(),
            }), || {
                println!("{}", "Active Threats".bold().cyan());
                for t in &threats {
                    println!("  [{}] {:?} — {:?}: {}", t.id, t.level, t.threat_type, t.description);
                }
                if threats.is_empty() {
                    println!("  No active threats.");
                }
            });
        }
        ThreatAction::Stats => {
            let stats = mon.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Threat Monitor Stats".bold().cyan());
                println!("  Total Threats:      {}", stats.total_threats);
                println!("  Active:             {}", stats.active_threats);
                println!("  Resolved:           {}", stats.resolved);
                println!("  False Positives:    {}", stats.false_positives);
                println!("  Phishing URLs:      {}", stats.phishing_urls);
                println!("  Malicious Contracts:{}", stats.malicious_contracts);
                println!("  Safe URLs:          {}", stats.safe_urls);
                println!("  Total Scans:        {}", stats.scan_count);
            });
        }
    }
    Ok(())
}

// ──────────────────────── Tier 15 handlers ────────────────────────────

fn cmd_pool(action: PoolAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::liquidity_pool::{LiquidityPoolManager, LiquidityPool, LpPosition, PoolType, PositionStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("liquidity_pool.json");
    let mut mgr = LiquidityPoolManager::load_or_default(&path);

    match action {
        PoolAction::Add { id, token_a, token_b, pool_type, fee_bps } => {
            let pt = match pool_type.as_str() {
                "stable" | "stableswap" => PoolType::StableSwap,
                "concentrated" => PoolType::Concentrated,
                _ => PoolType::ConstantProduct,
            };
            let pool = LiquidityPool {
                id: id.clone(), token_a: token_a.clone(), token_b: token_b.clone(),
                pool_type: pt, reserve_a: 0, reserve_b: 0, total_lp_tokens: 0,
                fee_bps, created_at: chrono::Utc::now().to_rfc3339(),
                volume_24h: 0, apy_estimate: 0.0,
            };
            mgr.add_pool(pool)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Pool '{}' added ({}/{}).", id, token_a, token_b);
            });
        }
        PoolAction::Deposit { id, pool_id, amount_a, amount_b, lp_tokens } => {
            let pos = LpPosition {
                id: id.clone(), pool_id, lp_tokens, deposited_a: amount_a, deposited_b: amount_b,
                deposit_time: chrono::Utc::now().to_rfc3339(), status: PositionStatus::Active,
                rewards_claimed: 0, pending_rewards: 0,
            };
            mgr.add_position(pos)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"position": id}), || {
                println!("Position '{}' deposited.", id);
            });
        }
        PoolAction::Withdraw { position_id } => {
            mgr.withdraw_position(&position_id)?;
            mgr.save(&path)?;
            println!("Position '{}' withdrawn.", position_id);
        }
        PoolAction::Claim { position_id } => {
            let claimed = mgr.claim_rewards(&position_id)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"claimed": claimed}), || {
                println!("Claimed {} rewards from position '{}'.", claimed, position_id);
            });
        }
        PoolAction::Estimate { pool_id, amount, a_to_b } => {
            let out = mgr.estimate_output(&pool_id, amount, a_to_b)?;
            crate::output::json_or(&serde_json::json!({"input": amount, "output": out}), || {
                println!("Estimated output: {} -> {}", amount, out);
            });
        }
        PoolAction::List => {
            let pools = mgr.pools_by_apy();
            crate::output::json_or(&serde_json::json!({"pools": pools.len()}), || {
                println!("{}", "Liquidity Pools (by APY)".bold().cyan());
                for p in &pools {
                    println!("  {} {}/{} — APY {:.2}% TVL {} fee {}bp",
                        p.id, p.token_a, p.token_b, p.apy_estimate, p.reserve_a + p.reserve_b, p.fee_bps);
                }
                if pools.is_empty() { println!("  (none)"); }
            });
        }
        PoolAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Liquidity Pool Stats".bold().cyan());
                println!("  Total Pools:      {}", stats.total_pools);
                println!("  Total Positions:  {}", stats.total_positions);
                println!("  Active Positions: {}", stats.active_positions);
                println!("  Total Deposited:  {}", stats.total_deposited_value);
                println!("  Rewards Claimed:  {}", stats.total_rewards_claimed);
                println!("  Rewards Pending:  {}", stats.total_pending_rewards);
            });
        }
    }
    Ok(())
}

fn cmd_farm(action: FarmAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::yield_farming::{YieldFarmManager, YieldFarm, FarmPosition, FarmStatus, CompoundStrategy, RewardType};
    let dir = crate::config::default_data_dir();
    let path = dir.join("yield_farming.json");
    let mut mgr = YieldFarmManager::load_or_default(&path);

    match action {
        FarmAction::Add { id, name, protocol, stake_token, reward_token, apy } => {
            let farm = YieldFarm {
                id: id.clone(), name: name.clone(), protocol, stake_token, reward_token,
                reward_type: RewardType::Token, apy, tvl: 0, status: FarmStatus::Active,
                start_date: chrono::Utc::now().to_rfc3339(), end_date: None, min_stake: 0,
            };
            mgr.add_farm(farm)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Farm '{}' ({}) added — APY {:.2}%.", id, name, apy);
            });
        }
        FarmAction::Stake { id, farm_id, amount, compound } => {
            let strat = match compound.as_str() {
                "daily" => CompoundStrategy::AutoDaily,
                "weekly" => CompoundStrategy::AutoWeekly,
                _ => CompoundStrategy::Manual,
            };
            let pos = FarmPosition {
                id: id.clone(), farm_id, staked_amount: amount,
                entry_time: chrono::Utc::now().to_rfc3339(), last_harvest: None,
                total_harvested: 0, pending_rewards: 0, compound_strategy: strat,
                auto_compound_count: 0,
            };
            mgr.stake(pos)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"position": id}), || {
                println!("Staked {} into position '{}'.", amount, id);
            });
        }
        FarmAction::Unstake { position_id } => {
            let pos = mgr.unstake(&position_id)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"unstaked": pos.staked_amount}), || {
                println!("Unstaked {} from '{}'.", pos.staked_amount, position_id);
            });
        }
        FarmAction::Harvest { position_id } => {
            let harvested = mgr.harvest(&position_id)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"harvested": harvested}), || {
                println!("Harvested {} from '{}'.", harvested, position_id);
            });
        }
        FarmAction::Compound { position_id } => {
            let compounded = mgr.auto_compound(&position_id)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"compounded": compounded}), || {
                println!("Auto-compounded {} into '{}'.", compounded, position_id);
            });
        }
        FarmAction::Best { n } => {
            let farms = mgr.best_farms(n);
            crate::output::json_or(&serde_json::json!({"farms": farms.len()}), || {
                println!("{}", "Top Yield Farms".bold().cyan());
                for f in &farms {
                    println!("  {} — {} APY {:.2}% TVL {}", f.id, f.name, f.apy, f.tvl);
                }
                if farms.is_empty() { println!("  (none)"); }
            });
        }
        FarmAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Yield Farming Stats".bold().cyan());
                println!("  Total Farms:    {}", stats.total_farms);
                println!("  Active Farms:   {}", stats.active_farms);
                println!("  Positions:      {}", stats.total_positions);
                println!("  Total Staked:   {}", stats.total_staked);
                println!("  Total Harvested:{}", stats.total_harvested);
                println!("  Total Pending:  {}", stats.total_pending);
                println!("  Avg APY:        {:.2}%", stats.avg_apy);
            });
        }
    }
    Ok(())
}

fn cmd_cross_swap(action: CrossSwapAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::cross_chain_swap::{CrossChainManager, SwapRoute, ChainId};
    let dir = crate::config::default_data_dir();
    let path = dir.join("cross_chain_swap.json");
    let mut mgr = CrossChainManager::load_or_default(&path);

    fn parse_chain(s: &str) -> ChainId {
        match s.to_lowercase().as_str() {
            "evaporchain" | "evapor" => ChainId::EvaporChain,
            "ethereum" | "eth" => ChainId::Ethereum,
            "solana" | "sol" => ChainId::Solana,
            "bitcoin" | "btc" => ChainId::Bitcoin,
            "polygon" | "matic" => ChainId::Polygon,
            "arbitrum" | "arb" => ChainId::Arbitrum,
            other => ChainId::Custom(other.to_string()),
        }
    }

    match action {
        CrossSwapAction::AddRoute { id, source_chain, dest_chain, source_token, dest_token, rate, fee_bps, provider } => {
            let route = SwapRoute {
                id: id.clone(), source_chain: parse_chain(&source_chain),
                dest_chain: parse_chain(&dest_chain), source_token, dest_token,
                exchange_rate: rate, fee_bps, estimated_time_secs: 300,
                provider, max_slippage_bps: 100,
            };
            mgr.add_route(route)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"route": id}), || {
                println!("Route '{}' added ({} → {}).", id, source_chain, dest_chain);
            });
        }
        CrossSwapAction::Swap { route_id, amount } => {
            let swap_id = mgr.initiate_swap(&route_id, amount)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"swap_id": swap_id}), || {
                println!("Swap initiated: {} (amount: {}).", swap_id, amount);
            });
        }
        CrossSwapAction::Lock { swap_id } => {
            mgr.lock_swap(&swap_id)?;
            mgr.save(&path)?;
            println!("Swap '{}' locked.", swap_id);
        }
        CrossSwapAction::Complete { swap_id, actual_output, dest_tx } => {
            mgr.complete_swap(&swap_id, actual_output, &dest_tx)?;
            mgr.save(&path)?;
            println!("Swap '{}' completed (output: {}).", swap_id, actual_output);
        }
        CrossSwapAction::Refund { swap_id } => {
            mgr.refund_swap(&swap_id)?;
            mgr.save(&path)?;
            println!("Swap '{}' refunded.", swap_id);
        }
        CrossSwapAction::Active => {
            let swaps = mgr.active_swaps();
            crate::output::json_or(&serde_json::json!({"active": swaps.len()}), || {
                println!("{}", "Active Cross-Chain Swaps".bold().cyan());
                for s in &swaps {
                    println!("  {} — {} → {:?} (amount: {})", s.id, s.source_amount, s.status, s.expected_output);
                }
                if swaps.is_empty() { println!("  (none)"); }
            });
        }
        CrossSwapAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Cross-Chain Swap Stats".bold().cyan());
                println!("  Total Swaps:  {}", stats.total_swaps);
                println!("  Completed:    {}", stats.completed);
                println!("  Pending:      {}", stats.pending);
                println!("  Failed:       {}", stats.failed);
                println!("  Refunded:     {}", stats.refunded);
                println!("  Volume In:    {}", stats.total_volume_in);
                println!("  Volume Out:   {}", stats.total_volume_out);
                println!("  Routes:       {}", stats.total_routes);
                println!("  Avg Slippage: {}bp", stats.avg_slippage_bps);
            });
        }
    }
    Ok(())
}

fn cmd_flash(action: FlashAction2) -> Result<(), Box<dyn std::error::Error>> {
    use crate::flash_loan::{FlashLoanManager, FlashAction};
    let dir = crate::config::default_data_dir();
    let path = dir.join("flash_loan.json");
    let mut mgr = FlashLoanManager::load_or_default(&path);

    match action {
        FlashAction2::Create { name, token, amount, fee_bps } => {
            let id = mgr.create_plan(&name, &token, amount, fee_bps);
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"plan_id": id}), || {
                println!("Flash loan plan created: {}", id);
            });
        }
        FlashAction2::Borrow { plan_id, token, amount } => {
            mgr.add_action(&plan_id, FlashAction::Borrow { token: token.clone(), amount })?;
            mgr.save(&path)?;
            println!("Added borrow({} {}) to plan '{}'.", amount, token, plan_id);
        }
        FlashAction2::Swap { plan_id, from, to, amount } => {
            mgr.add_action(&plan_id, FlashAction::Swap { from: from.clone(), to: to.clone(), amount })?;
            mgr.save(&path)?;
            println!("Added swap({} {} → {}) to plan '{}'.", amount, from, to, plan_id);
        }
        FlashAction2::Repay { plan_id, token, amount } => {
            mgr.add_action(&plan_id, FlashAction::Repay { token: token.clone(), amount })?;
            mgr.save(&path)?;
            println!("Added repay({} {}) to plan '{}'.", amount, token, plan_id);
        }
        FlashAction2::Simulate { plan_id } => {
            let result = mgr.simulate(&plan_id)?;
            mgr.save(&path)?;
            crate::output::json_or(&result, || {
                println!("{}", "Simulation Result".bold().cyan());
                println!("  Success: {}", result.success);
                println!("  Profit:  {}", result.profit);
                println!("  Gas:     {}", result.gas_used);
                println!("  Steps:   {}", result.steps_completed);
                if let Some(step) = result.failure_step {
                    println!("  Failed at step: {}", step);
                }
                if let Some(ref reason) = result.failure_reason {
                    println!("  Reason: {}", reason);
                }
            });
        }
        FlashAction2::Execute { plan_id } => {
            mgr.execute(&plan_id)?;
            mgr.save(&path)?;
            println!("Flash loan plan '{}' executed.", plan_id);
        }
        FlashAction2::Cancel { plan_id } => {
            mgr.cancel(&plan_id)?;
            mgr.save(&path)?;
            println!("Flash loan plan '{}' cancelled.", plan_id);
        }
        FlashAction2::List => {
            let plans = mgr.list_plans();
            crate::output::json_or(&serde_json::json!({"plans": plans.len()}), || {
                println!("{}", "Flash Loan Plans".bold().cyan());
                for p in &plans {
                    println!("  {} — {} ({:?}) borrow {} {}", p.id, p.name, p.status, p.borrow_amount, p.borrow_token);
                }
                if plans.is_empty() { println!("  (none)"); }
            });
        }
        FlashAction2::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Flash Loan Stats".bold().cyan());
                println!("  Total Plans:   {}", stats.total_plans);
                println!("  Executed:      {}", stats.executed);
                println!("  Successful:    {}", stats.successful);
                println!("  Failed:        {}", stats.failed);
                println!("  Total Borrowed:{}", stats.total_borrowed);
                println!("  Total Profit:  {}", stats.total_profit);
                println!("  Total Fees:    {}", stats.total_fees_paid);
                println!("  Avg Risk:      {}", stats.avg_risk_score);
            });
        }
    }
    Ok(())
}

// ──────────────────────── Tier 16 handlers ────────────────────────────

fn cmd_dca(action: DcaAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::dca_engine::{DcaEngine, DcaPlan, DcaFrequency, DcaStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("dca_engine.json");
    let mut engine = DcaEngine::load_or_default(&path);

    match action {
        DcaAction::Create { id, name, token_from, token_to, amount, frequency, max_buys, budget } => {
            let freq = match frequency.as_str() {
                "hourly" => DcaFrequency::Hourly,
                "weekly" => DcaFrequency::Weekly,
                "monthly" => DcaFrequency::Monthly,
                _ => DcaFrequency::Daily,
            };
            let plan = DcaPlan {
                id: id.clone(), name: name.clone(), token_from, token_to,
                amount_per_buy: amount, frequency: freq, total_budget: budget,
                spent: 0, buys_completed: 0, max_buys, status: DcaStatus::Active,
                created_at: chrono::Utc::now().to_rfc3339(), last_buy: None, next_buy: None,
                min_price: None, max_price: None, total_received: 0, avg_price: 0.0,
            };
            engine.create_plan(plan)?;
            engine.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("DCA plan '{}' ({}) created.", id, name);
            });
        }
        DcaAction::Pause { id } => {
            engine.pause_plan(&id)?;
            engine.save(&path)?;
            println!("DCA plan '{}' paused.", id);
        }
        DcaAction::Resume { id } => {
            engine.resume_plan(&id)?;
            engine.save(&path)?;
            println!("DCA plan '{}' resumed.", id);
        }
        DcaAction::Cancel { id } => {
            engine.cancel_plan(&id)?;
            engine.save(&path)?;
            println!("DCA plan '{}' cancelled.", id);
        }
        DcaAction::Buy { plan_id, price, received } => {
            let exec = engine.execute_buy(&plan_id, price, received, None)?;
            let ts = exec.timestamp.clone();
            let spent = exec.amount_spent;
            engine.save(&path)?;
            crate::output::json_or(&serde_json::json!({"plan": plan_id, "spent": spent, "received": received, "price": price}), || {
                println!("Buy executed at {} — spent {} received {} @ {:.4}", ts, spent, received, price);
            });
        }
        DcaAction::Active => {
            let plans = engine.active_plans();
            crate::output::json_or(&serde_json::json!({"active": plans.len()}), || {
                println!("{}", "Active DCA Plans".bold().cyan());
                for p in &plans {
                    println!("  {} — {} {}->{} ({}ea, {} buys)", p.id, p.name, p.token_from, p.token_to, p.amount_per_buy, p.buys_completed);
                }
                if plans.is_empty() { println!("  (none)"); }
            });
        }
        DcaAction::Stats => {
            let stats = engine.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "DCA Stats".bold().cyan());
                println!("  Total Plans:  {}", stats.total_plans);
                println!("  Active:       {}", stats.active_plans);
                println!("  Paused:       {}", stats.paused_plans);
                println!("  Completed:    {}", stats.completed_plans);
                println!("  Total Spent:  {}", stats.total_spent);
                println!("  Total Recv:   {}", stats.total_received);
                println!("  Total Buys:   {}", stats.total_buys);
                println!("  Avg Price:    {:.4}", stats.avg_price_all);
            });
        }
    }
    Ok(())
}

fn cmd_limit_order(action: LimitAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::limit_order::{LimitOrderManager, LimitOrder, OrderSide, OrderType, OrderStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("limit_order.json");
    let mut mgr = LimitOrderManager::load_or_default(&path);

    match action {
        LimitAction::Place { id, token_from, token_to, side, amount, price, expires } => {
            let s = match side.to_lowercase().as_str() {
                "sell" => OrderSide::Sell,
                _ => OrderSide::Buy,
            };
            let order = LimitOrder {
                id: id.clone(), token_from, token_to, side: s, order_type: OrderType::Limit,
                amount, filled_amount: 0, price, trigger_price: None,
                status: OrderStatus::Open, created_at: chrono::Utc::now().to_rfc3339(),
                expires_at: expires, filled_at: None, fills: Vec::new(),
            };
            mgr.place_order(order)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Limit order '{}' placed @ {:.4}.", id, price);
            });
        }
        LimitAction::Cancel { id } => {
            mgr.cancel_order(&id)?;
            mgr.save(&path)?;
            println!("Order '{}' cancelled.", id);
        }
        LimitAction::Fill { id, amount, price } => {
            mgr.fill_order(&id, amount, price, None)?;
            mgr.save(&path)?;
            println!("Filled {} @ {:.4} on order '{}'.", amount, price, id);
        }
        LimitAction::CheckTriggers { token_from, token_to, current_price } => {
            let pair = format!("{}/{}", token_from, token_to);
            let triggered = mgr.check_triggers(current_price, &pair);
            crate::output::json_or(&serde_json::json!({"triggered": triggered}), || {
                println!("{}", "Triggered Orders".bold().cyan());
                for id in &triggered {
                    println!("  {}", id);
                }
                if triggered.is_empty() { println!("  (none)"); }
            });
        }
        LimitAction::Open => {
            let orders = mgr.open_orders();
            crate::output::json_or(&serde_json::json!({"open": orders.len()}), || {
                println!("{}", "Open Orders".bold().cyan());
                for o in &orders {
                    println!("  {} {:?} {} @ {:.4} (filled {}/{})", o.id, o.side, o.token_from, o.price, o.filled_amount, o.amount);
                }
                if orders.is_empty() { println!("  (none)"); }
            });
        }
        LimitAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Limit Order Stats".bold().cyan());
                println!("  Total Orders: {}", stats.total_orders);
                println!("  Open:         {}", stats.open_orders);
                println!("  Filled:       {}", stats.filled_orders);
                println!("  Cancelled:    {}", stats.cancelled_orders);
                println!("  Expired:      {}", stats.expired_orders);
                println!("  Volume:       {}", stats.total_volume);
                println!("  Avg Fill:     {:.4}", stats.avg_fill_price);
            });
        }
    }
    Ok(())
}

fn cmd_rebalance(action: RebalAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::portfolio_rebalance::{RebalanceManager, Portfolio, RebalanceStrategy};
    let dir = crate::config::default_data_dir();
    let path = dir.join("portfolio_rebalance.json");
    let mut mgr = RebalanceManager::load_or_default(&path);

    match action {
        RebalAction::Create { id, name, threshold } => {
            let portfolio = Portfolio {
                id: id.clone(), name: name.clone(),
                targets: std::collections::HashMap::new(),
                holdings: std::collections::HashMap::new(),
                strategy: RebalanceStrategy::Threshold(threshold),
                threshold_pct: threshold, last_rebalance: None, rebalance_count: 0,
            };
            mgr.create_portfolio(portfolio)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Portfolio '{}' ({}) created with {}% threshold.", id, name, threshold);
            });
        }
        RebalAction::SetTarget { portfolio_id, token, pct } => {
            let p = mgr.get_portfolio(&portfolio_id)
                .ok_or_else(|| format!("Portfolio '{}' not found", portfolio_id))?;
            let mut targets = p.targets.clone();
            targets.insert(token.clone(), pct);
            // We need mutable access — get portfolio mutably
            if let Some(p) = mgr.portfolios.get_mut(&portfolio_id) {
                p.targets.insert(token.clone(), pct);
            }
            mgr.save(&path)?;
            println!("Set {} target to {:.1}% in '{}'.", token, pct, portfolio_id);
        }
        RebalAction::SetHolding { portfolio_id, token, value } => {
            let mut holdings = std::collections::HashMap::new();
            if let Some(p) = mgr.get_portfolio(&portfolio_id) {
                holdings = p.holdings.clone();
            }
            holdings.insert(token.clone(), value);
            mgr.update_holdings(&portfolio_id, holdings)?;
            mgr.save(&path)?;
            println!("Set {} holding to {} in '{}'.", token, value, portfolio_id);
        }
        RebalAction::Check { portfolio_id } => {
            let allocs = mgr.calculate_allocations(&portfolio_id)?;
            let drift = mgr.check_drift(&portfolio_id)?;
            let needs = mgr.needs_rebalance(&portfolio_id)?;
            crate::output::json_or(&serde_json::json!({"drift": drift, "needs_rebalance": needs}), || {
                println!("{}", "Portfolio Allocations".bold().cyan());
                for a in &allocs {
                    println!("  {} — target {:.1}% current {:.1}% drift {:+.1}%", a.token, a.target_pct, a.current_pct, a.drift);
                }
                println!("  Max Drift: {:.1}%  Needs Rebalance: {}", drift, needs);
            });
        }
        RebalAction::Execute { portfolio_id } => {
            let plan_id = mgr.generate_plan(&portfolio_id)?;
            mgr.execute_plan(&plan_id)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"plan_id": plan_id}), || {
                println!("Rebalance plan '{}' executed for '{}'.", plan_id, portfolio_id);
            });
        }
        RebalAction::List => {
            let portfolios = mgr.list_portfolios();
            crate::output::json_or(&serde_json::json!({"portfolios": portfolios.len()}), || {
                println!("{}", "Portfolios".bold().cyan());
                for p in &portfolios {
                    println!("  {} — {} ({} targets, {} rebalances)", p.id, p.name, p.targets.len(), p.rebalance_count);
                }
                if portfolios.is_empty() { println!("  (none)"); }
            });
        }
        RebalAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Rebalance Stats".bold().cyan());
                println!("  Portfolios:   {}", stats.total_portfolios);
                println!("  Rebalances:   {}", stats.total_rebalances);
                println!("  Completed:    {}", stats.completed_rebalances);
                println!("  Trade Volume: {}", stats.total_trade_volume);
                println!("  Avg Drift:    {:.2}%", stats.avg_drift);
            });
        }
    }
    Ok(())
}

fn cmd_smart_alerts(action: SmartAlertAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::smart_alerts::{SmartAlertEngine, SmartAlert, AlertCondition, AlertSeverity, AlertStatus, AlertAction};
    let dir = crate::config::default_data_dir();
    let path = dir.join("smart_alerts.json");
    let mut engine = SmartAlertEngine::load_or_default(&path);

    match action {
        SmartAlertAction::PriceAbove { id, token, threshold } => {
            let alert = SmartAlert {
                id: id.clone(), name: format!("{} > {}", token, threshold),
                conditions: vec![AlertCondition::PriceAbove { token: token.clone(), threshold }],
                actions: vec![AlertAction::Notify { message: format!("{} price above {}", token, threshold) }],
                severity: AlertSeverity::Warning, status: AlertStatus::Active,
                created_at: chrono::Utc::now().to_rfc3339(), triggered_at: None,
                trigger_count: 0, cooldown_secs: 300, last_triggered: None, expires_at: None,
            };
            engine.create_alert(alert)?;
            engine.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Alert '{}' created: {} > {}.", id, token, threshold);
            });
        }
        SmartAlertAction::PriceBelow { id, token, threshold } => {
            let alert = SmartAlert {
                id: id.clone(), name: format!("{} < {}", token, threshold),
                conditions: vec![AlertCondition::PriceBelow { token: token.clone(), threshold }],
                actions: vec![AlertAction::Notify { message: format!("{} price below {}", token, threshold) }],
                severity: AlertSeverity::Critical, status: AlertStatus::Active,
                created_at: chrono::Utc::now().to_rfc3339(), triggered_at: None,
                trigger_count: 0, cooldown_secs: 300, last_triggered: None, expires_at: None,
            };
            engine.create_alert(alert)?;
            engine.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Alert '{}' created: {} < {}.", id, token, threshold);
            });
        }
        SmartAlertAction::BalanceBelow { id, token, threshold } => {
            let alert = SmartAlert {
                id: id.clone(), name: format!("{} balance < {}", token, threshold),
                conditions: vec![AlertCondition::BalanceBelow { token: token.clone(), threshold }],
                actions: vec![AlertAction::Notify { message: format!("{} balance below {}", token, threshold) }],
                severity: AlertSeverity::Emergency, status: AlertStatus::Active,
                created_at: chrono::Utc::now().to_rfc3339(), triggered_at: None,
                trigger_count: 0, cooldown_secs: 60, last_triggered: None, expires_at: None,
            };
            engine.create_alert(alert)?;
            engine.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Alert '{}' created: {} balance < {}.", id, token, threshold);
            });
        }
        SmartAlertAction::Ack { id } => {
            engine.acknowledge(&id)?;
            engine.save(&path)?;
            println!("Alert '{}' acknowledged.", id);
        }
        SmartAlertAction::Dismiss { id } => {
            engine.dismiss(&id)?;
            engine.save(&path)?;
            println!("Alert '{}' dismissed.", id);
        }
        SmartAlertAction::Active => {
            let alerts = engine.active_alerts();
            crate::output::json_or(&serde_json::json!({"active": alerts.len()}), || {
                println!("{}", "Active Smart Alerts".bold().cyan());
                for a in &alerts {
                    println!("  {} — {} ({:?})", a.id, a.name, a.severity);
                }
                if alerts.is_empty() { println!("  (none)"); }
            });
        }
        SmartAlertAction::Stats => {
            let stats = engine.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Smart Alert Stats".bold().cyan());
                println!("  Total Alerts:  {}", stats.total_alerts);
                println!("  Active:        {}", stats.active);
                println!("  Triggered:     {}", stats.triggered);
                println!("  Dismissed:     {}", stats.dismissed);
                println!("  Total Events:  {}", stats.total_events);
                println!("  Trigger Count: {}", stats.total_trigger_count);
            });
        }
    }
    Ok(())
}

// ──────────────────────── Tier 17 handlers ────────────────────────────

fn cmd_social_recovery(action: RecoveryAction3) -> Result<(), Box<dyn std::error::Error>> {
    use crate::social_recovery::{SocialRecoveryManager, Guardian, GuardianStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("social_recovery.json");
    let mut mgr = SocialRecoveryManager::load_or_default(&path);

    match action {
        RecoveryAction3::AddGuardian { id, name, address, public_key } => {
            let guardian = Guardian {
                id: id.clone(), name: name.clone(), address, public_key,
                status: GuardianStatus::Active, added_at: chrono::Utc::now().to_rfc3339(),
                last_confirmed: None, trust_score: 80,
            };
            mgr.add_guardian(guardian)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Guardian '{}' ({}) added.", id, name);
            });
        }
        RecoveryAction3::RemoveGuardian { id } => {
            mgr.remove_guardian(&id)?;
            mgr.save(&path)?;
            println!("Guardian '{}' removed.", id);
        }
        RecoveryAction3::RevokeGuardian { id } => {
            mgr.revoke_guardian(&id)?;
            mgr.save(&path)?;
            println!("Guardian '{}' revoked.", id);
        }
        RecoveryAction3::Guardians => {
            let guardians = mgr.active_guardians();
            crate::output::json_or(&serde_json::json!({"guardians": guardians.len()}), || {
                println!("{}", "Active Guardians".bold().cyan());
                for g in &guardians {
                    println!("  {} — {} (trust: {})", g.id, g.name, g.trust_score);
                }
                if guardians.is_empty() { println!("  (none)"); }
            });
        }
        RecoveryAction3::Initiate { requester, new_key } => {
            let req_id = mgr.initiate_recovery(&requester, &new_key)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"request_id": req_id}), || {
                println!("Recovery initiated: {}", req_id);
            });
        }
        RecoveryAction3::Approve { request_id, guardian_id, signature } => {
            mgr.approve_recovery(&request_id, &guardian_id, &signature)?;
            mgr.save(&path)?;
            println!("Guardian '{}' approved recovery '{}'.", guardian_id, request_id);
        }
        RecoveryAction3::Complete { request_id } => {
            mgr.complete_recovery(&request_id)?;
            mgr.save(&path)?;
            println!("Recovery '{}' completed.", request_id);
        }
        RecoveryAction3::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Social Recovery Stats".bold().cyan());
                println!("  Guardians:     {}", stats.total_guardians);
                println!("  Active:        {}", stats.active_guardians);
                println!("  Requests:      {}", stats.total_requests);
                println!("  Completed:     {}", stats.completed_recoveries);
                println!("  Rejected:      {}", stats.rejected_recoveries);
                println!("  Pending:       {}", stats.pending_requests);
            });
        }
    }
    Ok(())
}

fn cmd_vault(action: VaultAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::shared_vault::{SharedVaultManager, Vault, VaultMember, VaultProposal, VaultRole, ProposalType, ProposalStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("shared_vault.json");
    let mut mgr = SharedVaultManager::load_or_default(&path);

    match action {
        VaultAction::Create { id, name, threshold } => {
            let vault = Vault {
                id: id.clone(), name: name.clone(),
                members: std::collections::HashMap::new(),
                threshold, balance: 0, created_at: chrono::Utc::now().to_rfc3339(),
                total_proposals: 0, spending_limit_daily: None, spent_today: 0,
            };
            mgr.create_vault(vault)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Vault '{}' ({}) created with threshold {}.", id, name, threshold);
            });
        }
        VaultAction::AddMember { vault_id, id, name, address, role } => {
            let r = match role.to_lowercase().as_str() {
                "owner" => VaultRole::Owner,
                "admin" => VaultRole::Admin,
                "viewer" => VaultRole::Viewer,
                _ => VaultRole::Signer,
            };
            let member = VaultMember {
                id: id.clone(), name: name.clone(), address, role: r,
                added_at: chrono::Utc::now().to_rfc3339(), last_active: None,
            };
            mgr.add_member(&vault_id, member)?;
            mgr.save(&path)?;
            println!("Member '{}' added to vault '{}'.", id, vault_id);
        }
        VaultAction::RemoveMember { vault_id, member_id } => {
            mgr.remove_member(&vault_id, &member_id)?;
            mgr.save(&path)?;
            println!("Member '{}' removed from vault '{}'.", member_id, vault_id);
        }
        VaultAction::Propose { vault_id, proposer, to, amount, token } => {
            let prop_id = format!("prop_{}", chrono::Utc::now().timestamp_millis());
            let proposal = VaultProposal {
                id: prop_id.clone(), vault_id: vault_id.clone(), proposer,
                proposal_type: ProposalType::Transfer { to, amount, token },
                status: ProposalStatus::Pending, created_at: chrono::Utc::now().to_rfc3339(),
                expires_at: (chrono::Utc::now() + chrono::Duration::hours(72)).to_rfc3339(),
                approvals: Vec::new(), rejections: Vec::new(), executed_at: None,
            };
            mgr.create_proposal(proposal)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"proposal_id": prop_id}), || {
                println!("Proposal '{}' created for vault '{}'.", prop_id, vault_id);
            });
        }
        VaultAction::ApproveProposal { proposal_id, member_id } => {
            mgr.approve_proposal(&proposal_id, &member_id)?;
            mgr.save(&path)?;
            println!("Member '{}' approved proposal '{}'.", member_id, proposal_id);
        }
        VaultAction::ExecuteProposal { proposal_id } => {
            mgr.execute_proposal(&proposal_id)?;
            mgr.save(&path)?;
            println!("Proposal '{}' executed.", proposal_id);
        }
        VaultAction::List => {
            let vaults = mgr.list_vaults();
            crate::output::json_or(&serde_json::json!({"vaults": vaults.len()}), || {
                println!("{}", "Shared Vaults".bold().cyan());
                for v in &vaults {
                    println!("  {} — {} ({} members, threshold {})", v.id, v.name, v.members.len(), v.threshold);
                }
                if vaults.is_empty() { println!("  (none)"); }
            });
        }
        VaultAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Shared Vault Stats".bold().cyan());
                println!("  Vaults:     {}", stats.total_vaults);
                println!("  Members:    {}", stats.total_members);
                println!("  Proposals:  {}", stats.total_proposals);
                println!("  Pending:    {}", stats.pending_proposals);
                println!("  Executed:   {}", stats.executed_proposals);
                println!("  Balance:    {}", stats.total_balance);
            });
        }
    }
    Ok(())
}

fn cmd_stream(action: StreamAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::payment_stream::{PaymentStreamManager, PaymentStream, StreamType, StreamStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("payment_stream.json");
    let mut mgr = PaymentStreamManager::load_or_default(&path);

    match action {
        StreamAction::Create { id, name, sender, recipient, token, total, rate, stream_type } => {
            let st = match stream_type.as_str() {
                "subscription" => StreamType::Subscription,
                "vesting" => StreamType::Vesting,
                _ => StreamType::Salary,
            };
            let now = chrono::Utc::now();
            #[allow(clippy::manual_checked_ops)]
            let duration_secs = if rate > 0 { total / rate } else { 3600 };
            let end = now + chrono::Duration::seconds(duration_secs as i64);
            let stream = PaymentStream {
                id: id.clone(), name: name.clone(), sender, recipient, token,
                total_amount: total, withdrawn: 0, rate_per_second: rate,
                stream_type: st, status: StreamStatus::Active,
                created_at: now.to_rfc3339(), start_time: now.to_rfc3339(),
                end_time: end.to_rfc3339(), last_withdrawal: None, cancellable: true,
            };
            mgr.create_stream(stream)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Stream '{}' ({}) created — {} total @ {}/s.", id, name, total, rate);
            });
        }
        StreamAction::Pause { id } => {
            mgr.pause_stream(&id)?;
            mgr.save(&path)?;
            println!("Stream '{}' paused.", id);
        }
        StreamAction::Resume { id } => {
            mgr.resume_stream(&id)?;
            mgr.save(&path)?;
            println!("Stream '{}' resumed.", id);
        }
        StreamAction::Cancel { id } => {
            mgr.cancel_stream(&id)?;
            mgr.save(&path)?;
            println!("Stream '{}' cancelled.", id);
        }
        StreamAction::Withdraw { id, amount } => {
            let rec = mgr.withdraw(&id, amount)?;
            let ts = rec.timestamp.clone();
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"withdrawn": amount}), || {
                println!("Withdrew {} from stream '{}' at {}.", amount, id, ts);
            });
        }
        StreamAction::Active => {
            let streams = mgr.active_streams();
            crate::output::json_or(&serde_json::json!({"active": streams.len()}), || {
                println!("{}", "Active Payment Streams".bold().cyan());
                for s in &streams {
                    println!("  {} — {} {}->{} ({}/s)", s.id, s.name, s.sender, s.recipient, s.rate_per_second);
                }
                if streams.is_empty() { println!("  (none)"); }
            });
        }
        StreamAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Payment Stream Stats".bold().cyan());
                println!("  Total Streams: {}", stats.total_streams);
                println!("  Active:        {}", stats.active_streams);
                println!("  Paused:        {}", stats.paused_streams);
                println!("  Completed:     {}", stats.completed_streams);
                println!("  Total Streamed:{}", stats.total_streamed);
                println!("  Withdrawn:     {}", stats.total_withdrawn);
                println!("  Pending:       {}", stats.total_pending);
            });
        }
    }
    Ok(())
}

fn cmd_escrow(action: EscrowAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::escrow::{EscrowManager, Escrow, EscrowStatus, DisputeResolution};
    let dir = crate::config::default_data_dir();
    let path = dir.join("escrow.json");
    let mut mgr = EscrowManager::load_or_default(&path);

    match action {
        EscrowAction::Create { id, buyer, seller, token, amount, fee_bps, description } => {
            let token_display = token.clone();
            let escrow = Escrow {
                id: id.clone(), buyer, seller, arbiter: None, token, amount, fee_bps,
                status: EscrowStatus::Created, created_at: chrono::Utc::now().to_rfc3339(),
                funded_at: None,
                expires_at: (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
                released_at: None, description, milestones: Vec::new(), dispute_reason: None,
            };
            mgr.create_escrow(escrow)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id": id}), || {
                println!("Escrow '{}' created ({} {}).", id, amount, token_display);
            });
        }
        EscrowAction::Fund { id } => {
            mgr.fund_escrow(&id)?;
            mgr.save(&path)?;
            println!("Escrow '{}' funded.", id);
        }
        EscrowAction::Release { id } => {
            mgr.release_escrow(&id)?;
            mgr.save(&path)?;
            println!("Escrow '{}' released to seller.", id);
        }
        EscrowAction::Refund { id } => {
            mgr.refund_escrow(&id)?;
            mgr.save(&path)?;
            println!("Escrow '{}' refunded to buyer.", id);
        }
        EscrowAction::Dispute { id, reason } => {
            mgr.dispute_escrow(&id, &reason)?;
            mgr.save(&path)?;
            println!("Escrow '{}' disputed: {}", id, reason);
        }
        EscrowAction::Resolve { id, to } => {
            let resolution = match to.as_str() {
                "buyer" => DisputeResolution::ReleaseToBuyer,
                _ => DisputeResolution::ReleaseToSeller,
            };
            mgr.resolve_dispute(&id, resolution)?;
            mgr.save(&path)?;
            println!("Escrow '{}' dispute resolved (to {}).", id, to);
        }
        EscrowAction::Active => {
            let escrows = mgr.active_escrows();
            crate::output::json_or(&serde_json::json!({"active": escrows.len()}), || {
                println!("{}", "Active Escrows".bold().cyan());
                for e in &escrows {
                    println!("  {} — {} {} ({:?})", e.id, e.amount, e.token, e.status);
                }
                if escrows.is_empty() { println!("  (none)"); }
            });
        }
        EscrowAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Escrow Stats".bold().cyan());
                println!("  Total:    {}", stats.total_escrows);
                println!("  Active:   {}", stats.active_escrows);
                println!("  Released: {}", stats.released);
                println!("  Refunded: {}", stats.refunded);
                println!("  Disputed: {}", stats.disputed);
                println!("  Volume:   {}", stats.total_volume);
                println!("  Fees:     {}", stats.total_fees);
            });
        }
    }
    Ok(())
}

// ──────────────────────── Tier 18 handlers ────────────────────────────

fn cmd_pnl(action: PnlAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::pnl_tracker::{PnlTracker, CostBasisMethod, TradeType};
    let dir = crate::config::default_data_dir();
    let path = dir.join("pnl_tracker.json");
    let mut tracker = PnlTracker::load_or_default(&path);

    match action {
        PnlAction::Buy { token, amount, price } => {
            let lot_id = tracker.record_buy(&token, amount, price, TradeType::Buy);
            tracker.save(&path)?;
            crate::output::json_or(&serde_json::json!({"lot_id": lot_id}), || {
                println!("Recorded buy: {} {} @ {:.4} (lot {})", amount, token, price, lot_id);
            });
        }
        PnlAction::Sell { token, amount, price, method } => {
            let m = match method.to_lowercase().as_str() {
                "lifo" => CostBasisMethod::Lifo,
                "hifo" => CostBasisMethod::Hifo,
                "avg" | "avgcost" => CostBasisMethod::AvgCost,
                _ => CostBasisMethod::Fifo,
            };
            let sale = tracker.record_sale(&token, amount, price, Some(m))?;
            tracker.save(&path)?;
            crate::output::json_or(&serde_json::json!({"realized_pnl": sale.realized_pnl}), || {
                println!("Sold {} {} @ {:.4} — P&L: {:.2}", amount, token, price, sale.realized_pnl);
            });
        }
        PnlAction::Unrealized { token, current_price } => {
            let pnl = tracker.unrealized_pnl(&token, current_price);
            crate::output::json_or(&serde_json::json!({"token": token, "unrealized_pnl": pnl}), || {
                println!("Unrealized P&L for {}: {:.2}", token, pnl);
            });
        }
        PnlAction::Token { token, current_price } => {
            let tp = tracker.token_pnl(&token, current_price);
            crate::output::json_or(&tp, || {
                println!("{}", format!("P&L — {}", token).bold().cyan());
                println!("  Realized:     {:.2}", tp.realized_pnl);
                println!("  Unrealized:   {:.2}", tp.unrealized_pnl);
                println!("  Cost Basis:   {:.2}", tp.total_cost_basis);
                println!("  Current Val:  {:.2}", tp.current_value);
                println!("  Avg Buy:      {:.4}", tp.avg_buy_price);
            });
        }
        PnlAction::Realized => {
            let total = tracker.total_realized_pnl();
            crate::output::json_or(&serde_json::json!({"total_realized_pnl": total}), || {
                println!("Total Realized P&L: {:.2}", total);
            });
        }
        PnlAction::Stats => {
            let prices = std::collections::HashMap::new();
            let stats = tracker.stats(&prices);
            crate::output::json_or(&stats, || {
                println!("{}", "P&L Stats".bold().cyan());
                println!("  Tokens Tracked: {}", stats.total_tokens_tracked);
                println!("  Total Lots:     {}", stats.total_lots);
                println!("  Open Lots:      {}", stats.open_lots);
                println!("  Total Sales:    {}", stats.total_sales);
                println!("  Realized P&L:   {:.2}", stats.total_realized_pnl);
                println!("  Best Trade:     {:.2}", stats.best_trade);
                println!("  Worst Trade:    {:.2}", stats.worst_trade);
            });
        }
    }
    Ok(())
}

fn cmd_analytics2(action: AnalyticsAction2) -> Result<(), Box<dyn std::error::Error>> {
    use crate::portfolio_analytics::PortfolioAnalytics;
    let dir = crate::config::default_data_dir();
    let path = dir.join("portfolio_analytics.json");
    let mut analytics = PortfolioAnalytics::load_or_default(&path);

    match action {
        AnalyticsAction2::Record { token, date, value } => {
            analytics.record_value(&token, &date, value);
            analytics.save(&path)?;
            println!("Recorded {} = {:.2} on {}.", token, value, date);
        }
        AnalyticsAction2::Sharpe { token, risk_free } => {
            let ratio = analytics.sharpe_ratio(&token, risk_free)?;
            crate::output::json_or(&serde_json::json!({"token": token, "sharpe_ratio": ratio}), || {
                println!("Sharpe Ratio for {}: {:.4}", token, ratio);
            });
        }
        AnalyticsAction2::Drawdown { token } => {
            let dd = analytics.max_drawdown(&token)?;
            crate::output::json_or(&serde_json::json!({"token": token, "max_drawdown_pct": dd}), || {
                println!("Max Drawdown for {}: {:.2}%", token, dd);
            });
        }
        AnalyticsAction2::Diversify => {
            let holdings = std::collections::HashMap::new();
            let score = analytics.diversification_score(&holdings);
            crate::output::json_or(&score, || {
                println!("{}", "Diversification Score".bold().cyan());
                println!("  Score:            {:.1}/100", score.score);
                println!("  HHI:              {:.4}", score.hhi);
                println!("  Effective Assets: {:.1}", score.effective_assets);
                println!("  Concentration:    {}", score.concentration_risk);
            });
        }
        AnalyticsAction2::Risk { token, risk_free } => {
            let metrics = analytics.risk_metrics(&token, risk_free)?;
            crate::output::json_or(&metrics, || {
                println!("{}", format!("Risk Metrics — {}", token).bold().cyan());
                println!("  Sharpe:     {:.4}", metrics.sharpe_ratio);
                println!("  Sortino:    {:.4}", metrics.sortino_ratio);
                println!("  Max DD:     {:.2}%", metrics.max_drawdown_pct);
                println!("  Volatility: {:.4}", metrics.volatility);
                println!("  Beta:       {:.4}", metrics.beta);
                println!("  VaR 95%:    {:.4}", metrics.var_95);
            });
        }
        AnalyticsAction2::Stats => {
            let holdings = std::collections::HashMap::new();
            let stats = analytics.stats(&holdings);
            crate::output::json_or(&stats, || {
                println!("{}", "Portfolio Analytics Stats".bold().cyan());
                println!("  Tokens:      {}", stats.tokens_tracked);
                println!("  Data Points: {}", stats.data_points);
                println!("  Date Range:  {} days", stats.date_range_days);
                println!("  Portfolio:   {}", stats.total_portfolio_value);
            });
        }
    }
    Ok(())
}

fn cmd_compliance(action: ComplianceAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::compliance_report::{ComplianceManager, CategorizedTx, TxCategory, ReportType, Jurisdiction};
    let dir = crate::config::default_data_dir();
    let path = dir.join("compliance_report.json");
    let mut mgr = ComplianceManager::load_or_default(&path);

    match action {
        ComplianceAction::AddTx { tx_hash, token, amount, value_usd, category } => {
            let cat = match category.to_lowercase().as_str() {
                "trade" => TxCategory::Trade,
                "income" => TxCategory::Income,
                "gift" => TxCategory::Gift,
                "airdrop" => TxCategory::Airdrop,
                "staking" => TxCategory::Staking,
                "mining" => TxCategory::Mining,
                "fee" => TxCategory::Fee,
                "transfer" => TxCategory::Transfer,
                _ => TxCategory::Unknown,
            };
            let tx = CategorizedTx {
                tx_hash: tx_hash.clone(), timestamp: chrono::Utc::now().to_rfc3339(),
                category: cat, token, amount, value_usd,
                counterparty: None, notes: String::new(), flagged: false,
            };
            mgr.add_transaction(tx);
            mgr.save(&path)?;
            println!("Transaction '{}' added for compliance.", tx_hash);
        }
        ComplianceAction::Flag { tx_hash } => {
            mgr.flag_transaction(&tx_hash)?;
            mgr.save(&path)?;
            println!("Transaction '{}' flagged.", tx_hash);
        }
        ComplianceAction::Report { report_type, jurisdiction } => {
            let rt = match report_type.to_lowercase().as_str() {
                "quarterly" => ReportType::Quarterly,
                "monthly" => ReportType::Monthly,
                _ => ReportType::Annual,
            };
            let jur = match jurisdiction.to_uppercase().as_str() {
                "UK" => Jurisdiction::UK,
                "EU" => Jurisdiction::EU,
                "SG" | "SINGAPORE" => Jurisdiction::Singapore,
                "JP" | "JAPAN" => Jurisdiction::Japan,
                "AU" | "AUSTRALIA" => Jurisdiction::Australia,
                _ => Jurisdiction::US,
            };
            let report_id = mgr.generate_report(rt, jur)?;
            mgr.save(&path)?;
            crate::output::json_or(&serde_json::json!({"report_id": report_id}), || {
                println!("Report generated: {}", report_id);
            });
        }
        ComplianceAction::Review { report_id } => {
            mgr.mark_reviewed(&report_id)?;
            mgr.save(&path)?;
            println!("Report '{}' marked reviewed.", report_id);
        }
        ComplianceAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Compliance Stats".bold().cyan());
                println!("  Transactions:   {}", stats.total_transactions);
                println!("  Categorized:    {}", stats.categorized);
                println!("  Uncategorized:  {}", stats.uncategorized);
                println!("  Flagged:        {}", stats.flagged);
                println!("  Reports:        {}", stats.reports_generated);
                println!("  Jurisdictions:  {}", stats.jurisdictions);
            });
        }
    }
    Ok(())
}

fn cmd_whale(action: WhaleAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::whale_tracker::{WhaleTracker, WhaleAccount, WhaleActivity};
    let dir = crate::config::default_data_dir();
    let path = dir.join("whale_tracker.json");
    let mut tracker = WhaleTracker::load_or_default(&path);

    match action {
        WhaleAction::Track { address, label, balance } => {
            let account = WhaleAccount {
                address: address.clone(), label, balance,
                first_seen: chrono::Utc::now().to_rfc3339(),
                last_active: chrono::Utc::now().to_rfc3339(),
                activity: WhaleActivity::Holding, cluster_id: None,
                is_exchange: false, total_inflow: 0, total_outflow: 0, tx_count: 0,
            };
            tracker.track_whale(account)?;
            tracker.save(&path)?;
            println!("Now tracking whale: {} (balance: {}).", address, balance);
        }
        WhaleAction::Untrack { address } => {
            tracker.untrack_whale(&address)?;
            tracker.save(&path)?;
            println!("Stopped tracking: {}.", address);
        }
        WhaleAction::Update { address, balance } => {
            tracker.update_balance(&address, balance)?;
            tracker.save(&path)?;
            println!("Updated {} balance to {}.", address, balance);
        }
        WhaleAction::Top { n } => {
            let whales = tracker.top_whales(n);
            crate::output::json_or(&serde_json::json!({"whales": whales.len()}), || {
                println!("{}", "Top Whales".bold().cyan());
                for w in &whales {
                    let lbl = w.label.as_deref().unwrap_or("—");
                    println!("  {} ({}) — {} ({:?})", w.address, lbl, w.balance, w.activity);
                }
                if whales.is_empty() { println!("  (none)"); }
            });
        }
        WhaleAction::Movements { n } => {
            let moves = tracker.recent_movements(n);
            crate::output::json_or(&serde_json::json!({"movements": moves.len()}), || {
                println!("{}", "Recent Whale Movements".bold().cyan());
                for m in &moves {
                    println!("  {} -> {} : {} {} ({:?})", m.from, m.to, m.amount, m.token, m.movement_type);
                }
                if moves.is_empty() { println!("  (none)"); }
            });
        }
        WhaleAction::Stats => {
            let stats = tracker.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Whale Tracker Stats".bold().cyan());
                println!("  Tracked:       {}", stats.tracked_whales);
                println!("  Movements:     {}", stats.total_movements);
                println!("  Clusters:      {}", stats.clusters);
                println!("  Accumulating:  {}", stats.accumulating);
                println!("  Distributing:  {}", stats.distributing);
                println!("  Dormant:       {}", stats.dormant);
                println!("  Total Balance: {}", stats.total_whale_balance);
            });
        }
    }
    Ok(())
}

// ──────────────────────── Tier 19 handlers ────────────────────────────

fn cmd_energy_opt(action: EnergyOptAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::energy_optimizer::{EnergyOptimizer, TrackedObject, ObjectState};
    let dir = crate::config::default_data_dir();
    let path = dir.join("energy_optimizer.json");
    let mut opt = EnergyOptimizer::load_or_default(&path);

    match action {
        EnergyOptAction::Track { id, owner, energy, max_energy, decay_rate, priority } => {
            let obj = TrackedObject {
                object_id: id.clone(), owner, current_energy: energy, max_energy,
                decay_rate, last_refresh: chrono::Utc::now().to_rfc3339(),
                epochs_since_refresh: 0,
                #[allow(clippy::manual_checked_ops)]
                estimated_grace_epoch: if decay_rate > 0 { (energy * 80 / 100 / decay_rate) as u32 } else { 0 },
                #[allow(clippy::manual_checked_ops)]
                estimated_evaporation_epoch: if decay_rate > 0 { (energy / decay_rate) as u32 } else { 0 },
                state: ObjectState::Healthy, refresh_cost: decay_rate * 10, priority,
            };
            opt.track_object(obj)?;
            opt.save(&path)?;
            println!("Tracking object '{}' ({}/{}energy, decay {}/epoch).", id, energy, max_energy, decay_rate);
        }
        EnergyOptAction::Forecast { id } => {
            let fc = opt.forecast(&id)?;
            crate::output::json_or(&fc, || {
                println!("{}", format!("Decay Forecast — {}", id).bold().cyan());
                println!("  Energy:       {:.1}%", fc.current_energy_pct);
                println!("  To Grace:     {} epochs", fc.epochs_to_grace);
                println!("  To Evaporate: {} epochs", fc.epochs_to_evaporation);
                println!("  Urgency:      {:?}", fc.urgency);
                for (epoch, pct) in &fc.milestones {
                    println!("    Epoch {}: {:.1}%", epoch, pct);
                }
            });
        }
        EnergyOptAction::ForecastAll => {
            let forecasts = opt.forecast_all();
            crate::output::json_or(&serde_json::json!({"forecasts": forecasts.len()}), || {
                println!("{}", "All Decay Forecasts".bold().cyan());
                for fc in &forecasts {
                    println!("  {} — {:.1}% ({:?}), grace in {} epochs",
                        fc.object_id, fc.current_energy_pct, fc.urgency, fc.epochs_to_grace);
                }
                if forecasts.is_empty() { println!("  (none)"); }
            });
        }
        EnergyOptAction::Critical => {
            let critical = opt.critical_objects();
            crate::output::json_or(&serde_json::json!({"critical": critical.len()}), || {
                println!("{}", "Critical Objects (<10% energy)".bold().red());
                for o in &critical {
                    let pct = if o.max_energy > 0 { o.current_energy as f64 / o.max_energy as f64 * 100.0 } else { 0.0 };
                    println!("  {} — {:.1}% ({:?})", o.object_id, pct, o.state);
                }
                if critical.is_empty() { println!("  All objects healthy!"); }
            });
        }
        EnergyOptAction::Batch { ids } => {
            let result = opt.batch_optimize(&ids)?;
            crate::output::json_or(&result, || {
                println!("{}", "Batch Optimization".bold().cyan());
                println!("  Objects:         {}", result.batch_size);
                println!("  Individual Cost: {}", result.total_individual_cost);
                println!("  Batch Cost:      {}", result.total_batch_cost);
                println!("  Savings:         {} ({:.1}%)", result.savings, result.savings_pct);
            });
        }
        EnergyOptAction::AutoPlan => {
            match opt.auto_plan() {
                Some(plan_id) => {
                    opt.save(&path)?;
                    crate::output::json_or(&serde_json::json!({"plan_id": plan_id}), || {
                        println!("Auto-plan created: {}", plan_id);
                    });
                }
                None => println!("No objects need urgent refresh."),
            }
        }
        EnergyOptAction::Execute { plan_id } => {
            opt.execute_plan(&plan_id)?;
            opt.save(&path)?;
            println!("Plan '{}' executed.", plan_id);
        }
        EnergyOptAction::Stats => {
            let stats = opt.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Energy Optimizer Stats".bold().cyan());
                println!("  Tracked:   {}", stats.tracked_objects);
                println!("  Critical:  {}", stats.critical_count);
                println!("  High:      {}", stats.high_count);
                println!("  Healthy:   {}", stats.healthy_count);
                println!("  Ghost:     {}", stats.ghost_count);
                println!("  Refresh $: {}", stats.total_refresh_cost);
                println!("  Avg Energy:{:.1}%", stats.avg_energy_pct);
                println!("  Plans:     {}/{} executed", stats.plans_executed, stats.plans_created);
            });
        }
    }
    Ok(())
}

fn cmd_obj_mgr(action: ObjMgrAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::object_manager::{ObjectManager, ManagedObject, ObjType, ObjLifecycle};
    let dir = crate::config::default_data_dir();
    let path = dir.join("object_manager.json");
    let mut mgr = ObjectManager::load_or_default(&path);

    match action {
        ObjMgrAction::Add { id, name, owner, obj_type, energy, max_energy } => {
            let ot = match obj_type.as_str() {
                "contract" => ObjType::Contract,
                "nft" => ObjType::NFT,
                "token" => ObjType::Token,
                _ => ObjType::Data,
            };
            let obj = ManagedObject {
                id: id.clone(), obj_type: ot, owner, name: name.clone(), energy, max_energy,
                lifecycle: ObjLifecycle::Active, created_at: chrono::Utc::now().to_rfc3339(),
                last_refreshed: None, transfer_count: 0,
                metadata: std::collections::HashMap::new(), frozen: false, size_bytes: 0, tags: Vec::new(),
            };
            mgr.add_object(obj)?;
            mgr.save(&path)?;
            println!("Object '{}' ({}) added.", id, name);
        }
        ObjMgrAction::Refresh { id, energy } => {
            mgr.refresh_object(&id, energy)?;
            mgr.save(&path)?;
            println!("Refreshed '{}' with {} energy.", id, energy);
        }
        ObjMgrAction::Transfer { id, new_owner } => {
            mgr.transfer_object(&id, &new_owner)?;
            mgr.save(&path)?;
            println!("Transferred '{}' to {}.", id, new_owner);
        }
        ObjMgrAction::Freeze { id } => {
            mgr.freeze_object(&id)?;
            mgr.save(&path)?;
            println!("Object '{}' frozen.", id);
        }
        ObjMgrAction::Unfreeze { id } => {
            mgr.unfreeze_object(&id)?;
            mgr.save(&path)?;
            println!("Object '{}' unfrozen.", id);
        }
        ObjMgrAction::Resurrect { id } => {
            let plan = mgr.plan_resurrection(&id)?;
            crate::output::json_or(&plan, || {
                println!("{}", "Resurrection Plan".bold().cyan());
                println!("  Object:  {}", plan.object_id);
                println!("  Cost:    {}", plan.cost);
                println!("  Energy:  {}", plan.energy_restored);
                println!("  Viable:  {}", plan.viable);
                if let Some(ref r) = plan.reason { println!("  Note:    {}", r); }
            });
        }
        ObjMgrAction::LowEnergy { threshold } => {
            let objs = mgr.low_energy_objects(threshold);
            crate::output::json_or(&serde_json::json!({"low_energy": objs.len()}), || {
                println!("{}", format!("Objects Below {:.0}% Energy", threshold * 100.0).bold().cyan());
                for o in &objs {
                    let pct = if o.max_energy > 0 { o.energy as f64 / o.max_energy as f64 * 100.0 } else { 0.0 };
                    println!("  {} — {:.1}% ({:?})", o.id, pct, o.lifecycle);
                }
                if objs.is_empty() { println!("  All objects healthy."); }
            });
        }
        ObjMgrAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Object Manager Stats".bold().cyan());
                println!("  Total:      {}", stats.total_objects);
                println!("  Active:     {}", stats.active);
                println!("  Grace:      {}", stats.grace);
                println!("  Ghost:      {}", stats.ghost);
                println!("  Evaporated: {}", stats.evaporated);
                println!("  Frozen:     {}", stats.frozen);
                println!("  Avg Energy: {:.1}%", stats.avg_energy_pct);
                println!("  Events:     {}", stats.total_events);
            });
        }
    }
    Ok(())
}

fn cmd_deploy(action: DeployAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::contract_deployer::{ContractDeployer, ContractDeployment, ContractType, DeployStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("contract_deployer.json");
    let mut dep = ContractDeployer::load_or_default(&path);

    match action {
        DeployAction::Create { id, name, deployer, contract_type } => {
            let ct = match contract_type.as_str() {
                "proxy" => ContractType::Proxy,
                "library" => ContractType::Library,
                "factory" => ContractType::Factory,
                _ => ContractType::Standard,
            };
            let deployment = ContractDeployment {
                id: id.clone(), name: name.clone(), contract_type: ct,
                bytecode_hash: String::new(), source_hash: None,
                status: DeployStatus::Draft, address: None, deployer,
                deploy_tx: None, created_at: chrono::Utc::now().to_rfc3339(),
                deployed_at: None, gas_used: None, constructor_args: Vec::new(),
                version: "1.0.0".to_string(), previous_version: None,
            };
            dep.create_contract(deployment)?;
            dep.save(&path)?;
            println!("Contract '{}' ({}) created as draft.", id, name);
        }
        DeployAction::Compile { id, bytecode_hex } => {
            dep.compile(&id, bytecode_hex.as_bytes())?;
            dep.save(&path)?;
            println!("Contract '{}' compiled.", id);
        }
        DeployAction::DeployContract { id, address, tx_hash, gas } => {
            dep.deploy(&id, &address, &tx_hash, gas)?;
            dep.save(&path)?;
            println!("Contract '{}' deployed at {}.", id, address);
        }
        DeployAction::Upgrade { id, new_bytecode_hex, version, notes } => {
            dep.upgrade(&id, new_bytecode_hex.as_bytes(), &version, &notes)?;
            dep.save(&path)?;
            println!("Contract '{}' upgraded to v{}.", id, version);
        }
        DeployAction::List => {
            let deployed = dep.deployed_contracts();
            crate::output::json_or(&serde_json::json!({"deployed": deployed.len()}), || {
                println!("{}", "Deployed Contracts".bold().cyan());
                for c in &deployed {
                    let addr = c.address.as_deref().unwrap_or("—");
                    println!("  {} — {} v{} @ {}", c.id, c.name, c.version, addr);
                }
                if deployed.is_empty() { println!("  (none)"); }
            });
        }
        DeployAction::Stats => {
            let stats = dep.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Contract Deployer Stats".bold().cyan());
                println!("  Total:    {}", stats.total_contracts);
                println!("  Deployed: {}", stats.deployed);
                println!("  Verified: {}", stats.verified);
                println!("  Failed:   {}", stats.failed);
                println!("  Upgrades: {}", stats.upgrades);
                println!("  Gas Used: {}", stats.total_gas);
            });
        }
    }
    Ok(())
}

fn cmd_gov(action: GovAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::governance_dashboard::{GovernanceDashboard, GovernanceProposal, Vote, VoteChoice, ProposalState};
    let dir = crate::config::default_data_dir();
    let path = dir.join("governance_dashboard.json");
    let mut gov = GovernanceDashboard::load_or_default(&path);

    match action {
        GovAction::Propose { id, title, proposer, quorum, voting_power } => {
            let proposal = GovernanceProposal {
                id: id.clone(), title: title.clone(), description: String::new(),
                proposer, state: ProposalState::Discussion,
                created_at: chrono::Utc::now().to_rfc3339(),
                voting_start: None, voting_end: None, executed_at: None,
                votes_for: 0, votes_against: 0, votes_abstain: 0,
                quorum_required: quorum, total_voting_power: voting_power,
            };
            gov.add_proposal(proposal)?;
            gov.save(&path)?;
            println!("Proposal '{}' ({}) created.", id, title);
        }
        GovAction::StartVoting { id, end } => {
            gov.start_voting(&id, &end)?;
            gov.save(&path)?;
            println!("Voting started on proposal '{}'.", id);
        }
        GovAction::Vote { proposal_id, voter, choice, power } => {
            let c = match choice.to_lowercase().as_str() {
                "against" | "no" => VoteChoice::Against,
                "abstain" => VoteChoice::Abstain,
                _ => VoteChoice::For,
            };
            let vote = Vote {
                proposal_id: proposal_id.clone(), voter: voter.clone(), choice: c,
                voting_power: power, timestamp: chrono::Utc::now().to_rfc3339(), reason: None,
            };
            gov.cast_vote(vote)?;
            gov.save(&path)?;
            println!("{} voted '{}' on proposal '{}' (power: {}).", voter, choice, proposal_id, power);
        }
        GovAction::Finalize { id } => {
            gov.finalize_proposal(&id)?;
            gov.save(&path)?;
            let state = gov.get_proposal(&id).map(|p| format!("{:?}", p.state)).unwrap_or_default();
            println!("Proposal '{}' finalized: {}.", id, state);
        }
        GovAction::ExecuteProposal { id } => {
            gov.execute_proposal(&id)?;
            gov.save(&path)?;
            println!("Proposal '{}' executed.", id);
        }
        GovAction::Delegate { from, to, power } => {
            gov.delegate(&from, &to, power);
            gov.save(&path)?;
            println!("Delegated {} power from {} to {}.", power, from, to);
        }
        GovAction::Active => {
            let proposals = gov.active_proposals();
            crate::output::json_or(&serde_json::json!({"active": proposals.len()}), || {
                println!("{}", "Active Proposals".bold().cyan());
                for p in &proposals {
                    println!("  {} — {} ({:?})", p.id, p.title, p.state);
                }
                if proposals.is_empty() { println!("  (none)"); }
            });
        }
        GovAction::Stats => {
            let stats = gov.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Governance Stats".bold().cyan());
                println!("  Proposals:     {}", stats.total_proposals);
                println!("  Active:        {}", stats.active_proposals);
                println!("  Passed:        {}", stats.passed);
                println!("  Rejected:      {}", stats.rejected);
                println!("  Votes Cast:    {}", stats.total_votes_cast);
                println!("  Delegations:   {}", stats.total_delegations);
                println!("  Participation: {:.1}%", stats.participation_rate);
                println!("  Avg Turnout:   {:.1}%", stats.avg_turnout_pct);
            });
        }
    }
    Ok(())
}

// ──────────────────────── Tier 20 handlers ────────────────────────────

fn cmd_fee_opt(action: FeeOptAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::fee_optimizer::{FeeOptimizer, FeeHistoryEntry};
    let dir = crate::config::default_data_dir();
    let path = dir.join("fee_optimizer.json");
    let mut opt = FeeOptimizer::load_or_default(&path);

    match action {
        FeeOptAction::Record { gas_price, utilization, tx_count } => {
            let entry = FeeHistoryEntry {
                timestamp: chrono::Utc::now().to_rfc3339(),
                gas_price, block_utilization_pct: utilization, tx_count,
            };
            opt.record_fee(entry);
            opt.save(&path)?;
            println!("Recorded fee: gas={} util={:.1}% txs={}", gas_price, utilization, tx_count);
        }
        FeeOptAction::Estimate => {
            let estimates = opt.estimate_fees();
            crate::output::json_or(&serde_json::json!({"estimates": estimates.len()}), || {
                println!("{}", "Fee Estimates".bold().cyan());
                for e in &estimates {
                    println!("  {:?}: {} gas (~{}s, {:.0}% confidence)", e.speed, e.gas_price, e.estimated_time_secs, e.confidence_pct);
                }
                if estimates.is_empty() { println!("  Not enough data."); }
            });
        }
        FeeOptAction::Market => {
            let analysis = opt.market_analysis();
            crate::output::json_or(&analysis, || {
                println!("{}", "Fee Market Analysis".bold().cyan());
                println!("  Condition: {:?}", analysis.current_condition);
                println!("  Avg 24h:   {:.1}", analysis.avg_gas_24h);
                println!("  Median:    {:.1}", analysis.median_gas_24h);
                println!("  Min/Max:   {}/{}", analysis.min_gas_24h, analysis.max_gas_24h);
                println!("  Trend:     {:.2}", analysis.trend);
                println!("  Best Hour: {}:00", analysis.best_hour);
                println!("  Worst Hour:{}:00", analysis.worst_hour);
            });
        }
        FeeOptAction::Windows => {
            let windows = opt.optimal_windows();
            crate::output::json_or(&serde_json::json!({"windows": windows.len()}), || {
                println!("{}", "Optimal Submission Windows".bold().cyan());
                for w in &windows {
                    let rec = if w.recommended { " *" } else { "" };
                    println!("  {:02}:00-{:02}:00 — {:.1}% savings{}", w.start_hour, w.end_hour, w.avg_savings_pct, rec);
                }
            });
        }
        FeeOptAction::ShouldSubmit { max_gas } => {
            let should = opt.should_submit_now(max_gas);
            crate::output::json_or(&serde_json::json!({"should_submit": should}), || {
                if should { println!("Yes, current gas is within budget."); }
                else {
                    let wait = opt.wait_recommendation(max_gas);
                    match wait {
                        Some(h) => println!("No. Wait ~{} hours for lower gas.", h),
                        None => println!("No data to make a recommendation."),
                    }
                }
            });
        }
        FeeOptAction::Stats => {
            let stats = opt.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Fee Optimizer Stats".bold().cyan());
                println!("  Data Points: {}", stats.data_points);
                println!("  Avg Gas:     {:.1}", stats.avg_gas_price);
                println!("  Median Gas:  {:.1}", stats.median_gas_price);
                println!("  Condition:   {:?}", stats.current_condition);
            });
        }
    }
    Ok(())
}

fn cmd_batch_exec(action: BatchExecAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::batch_executor::{BatchExecutor, BatchTx, TxStatus, RollbackPolicy};
    let dir = crate::config::default_data_dir();
    let path = dir.join("batch_executor.json");
    let mut exec = BatchExecutor::load_or_default(&path);

    match action {
        BatchExecAction::Create { name, policy } => {
            let p = match policy.as_str() {
                "none" => RollbackPolicy::None,
                "rollback" | "rollback_all" => RollbackPolicy::RollbackAll,
                "continue" => RollbackPolicy::ContinueOnFailure,
                _ => RollbackPolicy::StopOnFailure,
            };
            let id = exec.create_batch(&name, p);
            exec.save(&path)?;
            crate::output::json_or(&serde_json::json!({"batch_id": id}), || {
                println!("Batch '{}' created ({}).", id, name);
            });
        }
        BatchExecAction::Add { batch_id, description, to, amount, token } => {
            let tx_id = format!("tx_{}", chrono::Utc::now().timestamp_millis());
            let order = exec.get_batch(&batch_id).map(|b| b.transactions.len() as u32).unwrap_or(0);
            let tx = BatchTx {
                id: tx_id.clone(), description, tx_type: "transfer".to_string(),
                to, amount, token, status: TxStatus::Pending,
                tx_hash: None, error: None, gas_used: None, order, depends_on: None,
            };
            exec.add_tx(&batch_id, tx)?;
            exec.save(&path)?;
            println!("Added tx '{}' to batch '{}'.", tx_id, batch_id);
        }
        BatchExecAction::Validate { batch_id } => {
            let warnings = exec.validate_batch(&batch_id)?;
            exec.save(&path)?;
            crate::output::json_or(&serde_json::json!({"warnings": warnings}), || {
                println!("Batch '{}' validated.", batch_id);
                for w in &warnings { println!("  Warning: {}", w); }
            });
        }
        BatchExecAction::Execute { batch_id } => {
            let result = exec.execute_batch(&batch_id)?;
            exec.save(&path)?;
            crate::output::json_or(&result, || {
                println!("{}", "Batch Execution Result".bold().cyan());
                println!("  Status:    {:?}", result.status);
                println!("  Completed: {}", result.completed);
                println!("  Failed:    {}", result.failed);
                println!("  Skipped:   {}", result.skipped);
                println!("  Gas:       {}", result.total_gas);
            });
        }
        BatchExecAction::Rollback { batch_id } => {
            exec.rollback_batch(&batch_id)?;
            exec.save(&path)?;
            println!("Batch '{}' rolled back.", batch_id);
        }
        BatchExecAction::Pending => {
            let batches = exec.pending_batches();
            crate::output::json_or(&serde_json::json!({"pending": batches.len()}), || {
                println!("{}", "Pending Batches".bold().cyan());
                for b in &batches {
                    println!("  {} — {} ({} txs, {:?})", b.id, b.name, b.transactions.len(), b.status);
                }
                if batches.is_empty() { println!("  (none)"); }
            });
        }
        BatchExecAction::Stats => {
            let stats = exec.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Batch Executor Stats".bold().cyan());
                println!("  Total Batches: {}", stats.total_batches);
                println!("  Completed:     {}", stats.completed_batches);
                println!("  Failed:        {}", stats.failed_batches);
                println!("  Total Txs:     {}", stats.total_transactions);
                println!("  Gas Used:      {}", stats.total_gas_used);
                println!("  Avg Size:      {:.1}", stats.avg_batch_size);
                println!("  Success Rate:  {:.1}%", stats.success_rate);
            });
        }
    }
    Ok(())
}

fn cmd_migrate2(action: MigrateAction2) -> Result<(), Box<dyn std::error::Error>> {
    use crate::wallet_migration::{WalletMigrator, SourceWallet, ImportedAccount, KeyFormat};
    let dir = crate::config::default_data_dir();
    let path = dir.join("wallet_migration.json");
    let mut migrator = WalletMigrator::load_or_default(&path);

    match action {
        MigrateAction2::Start { source } => {
            let src = match source.to_lowercase().as_str() {
                "phantom" => SourceWallet::Phantom,
                "trust" | "trustwallet" => SourceWallet::TrustWallet,
                "ledger" => SourceWallet::Ledger,
                "trezor" => SourceWallet::Trezor,
                "exodus" => SourceWallet::Exodus,
                _ => SourceWallet::MetaMask,
            };
            let id = migrator.start_migration(src);
            migrator.save(&path)?;
            crate::output::json_or(&serde_json::json!({"job_id": id}), || {
                println!("Migration started from {}: {}", source, id);
            });
        }
        MigrateAction2::Import { job_id, original_address, new_address, format } => {
            let fmt = match format.to_lowercase().as_str() {
                "base58" => KeyFormat::Base58,
                "bech32" => KeyFormat::Bech32,
                "mnemonic12" => KeyFormat::Mnemonic12,
                "mnemonic24" => KeyFormat::Mnemonic24,
                _ => KeyFormat::Hex,
            };
            let account = ImportedAccount {
                original_address: original_address.clone(), new_address,
                source: SourceWallet::MetaMask, key_format: fmt,
                label: None, imported_at: chrono::Utc::now().to_rfc3339(), balance_snapshot: 0,
            };
            migrator.import_account(&job_id, account)?;
            migrator.save(&path)?;
            println!("Imported account {} into job '{}'.", original_address, job_id);
        }
        MigrateAction2::Complete { job_id } => {
            let report = migrator.complete_migration(&job_id)?;
            migrator.save(&path)?;
            crate::output::json_or(&report, || {
                println!("{}", "Migration Complete".bold().cyan());
                println!("  Accounts: {}", report.accounts_imported);
                println!("  Tokens:   {}", report.tokens_imported);
                println!("  NFTs:     {}", report.nfts_imported);
                println!("  Success:  {}", report.success);
            });
        }
        MigrateAction2::Active => {
            let jobs = migrator.active_migrations();
            crate::output::json_or(&serde_json::json!({"active": jobs.len()}), || {
                println!("{}", "Active Migrations".bold().cyan());
                for j in &jobs {
                    println!("  {} — {:?} ({:?})", j.id, j.source, j.status);
                }
                if jobs.is_empty() { println!("  (none)"); }
            });
        }
        MigrateAction2::Stats => {
            let stats = migrator.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Migration Stats".bold().cyan());
                println!("  Total:     {}", stats.total_migrations);
                println!("  Completed: {}", stats.completed);
                println!("  Failed:    {}", stats.failed);
                println!("  Accounts:  {}", stats.total_accounts_imported);
                println!("  Tokens:    {}", stats.total_tokens_imported);
            });
        }
    }
    Ok(())
}

fn cmd_diag(action: DiagAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::diagnostic::DiagnosticEngine;
    let dir = crate::config::default_data_dir();
    let path = dir.join("diagnostic.json");
    let mut engine = DiagnosticEngine::load_or_default(&path);

    match action {
        DiagAction::Init => {
            engine.register_default_checks();
            engine.save(&path)?;
            println!("Registered {} default diagnostic checks.", engine.registered_checks.len());
        }
        DiagAction::RunAll => {
            let report = engine.run_all_checks();
            engine.save(&path)?;
            crate::output::json_or(&report, || {
                println!("{}", "Diagnostic Report".bold().cyan());
                println!("  Overall: {:?}", report.overall_status);
                println!("  Pass: {}  Warn: {}  Fail: {}  Skip: {}",
                    report.pass_count, report.warn_count, report.fail_count, report.skip_count);
                for c in &report.checks {
                    let icon = match c.status {
                        crate::diagnostic::CheckStatus::Pass => "+",
                        crate::diagnostic::CheckStatus::Warn => "!",
                        crate::diagnostic::CheckStatus::Fail => "X",
                        crate::diagnostic::CheckStatus::Skip => "-",
                    };
                    println!("  [{}] {} — {}", icon, c.name, c.message);
                }
            });
        }
        DiagAction::Run { check_id } => {
            let check = engine.run_check(&check_id)?;
            engine.save(&path)?;
            crate::output::json_or(&check, || {
                println!("[{:?}] {} — {}", check.status, check.name, check.message);
            });
        }
        DiagAction::Repair { check_id } => {
            let result = engine.attempt_repair(&check_id)?;
            engine.save(&path)?;
            crate::output::json_or(&result, || {
                println!("Repair {:?}: {}", result.action, result.message);
            });
        }
        DiagAction::Report => {
            match engine.latest_report() {
                Some(report) => {
                    crate::output::json_or(report, || {
                        println!("{}", "Latest Diagnostic Report".bold().cyan());
                        println!("  Date:    {}", report.created_at);
                        println!("  Status:  {:?}", report.overall_status);
                        println!("  Pass: {}  Warn: {}  Fail: {}", report.pass_count, report.warn_count, report.fail_count);
                    });
                }
                None => println!("No reports yet. Run 'diag run-all' first."),
            }
        }
        DiagAction::Stats => {
            let stats = engine.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Diagnostic Stats".bold().cyan());
                println!("  Reports:    {}", stats.total_reports);
                println!("  Checks Run: {}", stats.total_checks_run);
                println!("  Passes:     {}", stats.total_passes);
                println!("  Warnings:   {}", stats.total_warnings);
                println!("  Failures:   {}", stats.total_failures);
                println!("  Repairs:    {}", stats.total_repairs);
                println!("  Auto-Fixed: {}", stats.auto_fixed);
            });
        }
    }
    Ok(())
}

fn cmd_ws(action: WsAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::ws_subscriber::{WsSubscriber, Subscription, SubStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("ws_subscriber.json");
    let mut ws = WsSubscriber::load_or_default(&path);

    match action {
        WsAction::Subscribe { id, event_type, endpoint } => {
            let et = parse_event_type(&event_type);
            let sub = Subscription {
                id: id.clone(),
                event_type: et,
                status: SubStatus::Active,
                filters: vec![],
                created_at: chrono::Utc::now().to_rfc3339(),
                last_event: None,
                event_count: 0,
                error_count: 0,
                endpoint,
            };
            ws.subscribe(sub)?;
            ws.save(&path)?;
            println!("Subscribed: {}", id);
        }
        WsAction::Unsubscribe { id } => {
            ws.unsubscribe(&id)?;
            ws.save(&path)?;
            println!("Unsubscribed: {}", id);
        }
        WsAction::Pause { id } => {
            ws.pause(&id)?;
            ws.save(&path)?;
            println!("Paused: {}", id);
        }
        WsAction::Resume { id } => {
            ws.resume(&id)?;
            ws.save(&path)?;
            println!("Resumed: {}", id);
        }
        WsAction::Reconnect { id } => {
            ws.reconnect(&id)?;
            ws.save(&path)?;
            println!("Reconnected: {}", id);
        }
        WsAction::List => {
            let subs: Vec<_> = ws.subscriptions.values().collect();
            crate::output::json_or(&subs, || {
                if subs.is_empty() {
                    println!("No subscriptions.");
                } else {
                    println!("{}", "WebSocket Subscriptions".bold().cyan());
                    for s in &subs {
                        println!("  {} — {:?} [{:?}] events={} endpoint={}",
                            s.id, s.event_type, s.status, s.event_count, s.endpoint);
                    }
                }
            });
        }
        WsAction::Events { count } => {
            let events = ws.recent_events(count);
            crate::output::json_or(&events, || {
                if events.is_empty() {
                    println!("No events recorded.");
                } else {
                    println!("{}", "Recent Events".bold().cyan());
                    for e in &events {
                        println!("  [{}] {:?} sub={} block={:?}",
                            e.timestamp, e.event_type, e.subscription_id, e.block_number);
                    }
                }
            });
        }
        WsAction::Stats => {
            let stats = ws.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "WebSocket Stats".bold().cyan());
                println!("  Total:        {}", stats.total_subscriptions);
                println!("  Active:       {}", stats.active);
                println!("  Paused:       {}", stats.paused);
                println!("  Disconnected: {}", stats.disconnected);
                println!("  Events:       {}", stats.total_events);
                println!("  Errors:       {}", stats.total_errors);
            });
        }
    }
    Ok(())
}

fn parse_event_type(s: &str) -> crate::ws_subscriber::EventType {
    use crate::ws_subscriber::EventType;
    match s.to_lowercase().as_str() {
        "new_block" | "block" => EventType::NewBlock,
        "pending_tx" => EventType::PendingTx,
        "confirmed_tx" => EventType::ConfirmedTx,
        "token_transfer" => EventType::TokenTransfer,
        "contract_event" => EventType::ContractEvent,
        "energy_decay" => EventType::EnergyDecay,
        "price_update" => EventType::PriceUpdate,
        other => EventType::Custom(other.to_string()),
    }
}

fn cmd_event_bus(action: EventBusAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::event_bus::{EventBus, EventHandler, HandlerStatus, BusEvent, EventPriority};
    let dir = crate::config::default_data_dir();
    let path = dir.join("event_bus.json");
    let mut bus = EventBus::load_or_default(&path);

    match action {
        EventBusAction::Register { id, topic, desc } => {
            let handler = EventHandler {
                id: id.clone(),
                topic_filter: topic,
                description: desc,
                status: HandlerStatus::Active,
                created_at: chrono::Utc::now().to_rfc3339(),
                invocation_count: 0,
                last_invoked: None,
                error_count: 0,
            };
            bus.register_handler(handler)?;
            bus.save(&path)?;
            println!("Handler registered: {}", id);
        }
        EventBusAction::Unregister { id } => {
            bus.unregister_handler(&id)?;
            bus.save(&path)?;
            println!("Handler removed: {}", id);
        }
        EventBusAction::Enable { id } => {
            bus.enable_handler(&id)?;
            bus.save(&path)?;
            println!("Handler enabled: {}", id);
        }
        EventBusAction::Disable { id } => {
            bus.disable_handler(&id)?;
            bus.save(&path)?;
            println!("Handler disabled: {}", id);
        }
        EventBusAction::Publish { topic, priority, source } => {
            let prio = match priority.to_lowercase().as_str() {
                "low" => EventPriority::Low,
                "high" => EventPriority::High,
                "critical" => EventPriority::Critical,
                _ => EventPriority::Normal,
            };
            let id = format!("evt_{}", chrono::Utc::now().timestamp_millis());
            let event = BusEvent {
                id: id.clone(),
                topic,
                payload: std::collections::HashMap::new(),
                priority: prio,
                timestamp: chrono::Utc::now().to_rfc3339(),
                source,
                processed: false,
            };
            bus.publish(event);
            bus.save(&path)?;
            println!("Event published: {}", id);
        }
        EventBusAction::Process { event_id } => {
            let matched = bus.process_event(&event_id)?;
            bus.save(&path)?;
            println!("Processed event {}. Matched {} handlers.", event_id, matched.len());
            for h in &matched {
                println!("  → {}", h);
            }
        }
        EventBusAction::Handlers => {
            let handlers: Vec<_> = bus.handlers.values().collect();
            crate::output::json_or(&handlers, || {
                if handlers.is_empty() {
                    println!("No handlers registered.");
                } else {
                    println!("{}", "Event Handlers".bold().cyan());
                    for h in &handlers {
                        println!("  {} — topic={} [{:?}] invocations={}",
                            h.id, h.topic_filter, h.status, h.invocation_count);
                    }
                }
            });
        }
        EventBusAction::Pending => {
            let pending = bus.pending_events();
            crate::output::json_or(&pending, || {
                if pending.is_empty() {
                    println!("No pending events.");
                } else {
                    println!("{}", "Pending Events".bold().cyan());
                    for e in &pending {
                        println!("  {} — topic={} [{:?}] from={}",
                            e.id, e.topic, e.priority, e.source);
                    }
                }
            });
        }
        EventBusAction::Logs { count } => {
            let logs = bus.recent_logs(count);
            crate::output::json_or(&logs, || {
                if logs.is_empty() {
                    println!("No logs.");
                } else {
                    println!("{}", "Event Logs".bold().cyan());
                    for l in &logs {
                        let icon = if l.success { "+" } else { "X" };
                        println!("  [{}] evt={} handler={} {}ms",
                            icon, l.event_id, l.handler_id, l.duration_ms);
                    }
                }
            });
        }
        EventBusAction::Stats => {
            let stats = bus.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Event Bus Stats".bold().cyan());
                println!("  Total Events:   {}", stats.total_events);
                println!("  Processed:      {}", stats.processed_events);
                println!("  Pending:        {}", stats.pending_events);
                println!("  Handlers:       {}", stats.total_handlers);
                println!("  Active:         {}", stats.active_handlers);
                println!("  Invocations:    {}", stats.total_invocations);
                println!("  Errors:         {}", stats.total_errors);
            });
        }
    }
    Ok(())
}

fn cmd_receipts(action: ReceiptAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::tx_receipt_store::{TxReceiptStore, TxReceipt};
    let dir = crate::config::default_data_dir();
    let path = dir.join("tx_receipts.json");
    let mut store = TxReceiptStore::load_or_default(&path);

    match action {
        ReceiptAction::Store { tx_hash, from, to, amount, token, tx_type, status, gas_used, fee } => {
            let tt = parse_tx_type2(&tx_type);
            let st = parse_receipt_status(&status);
            let receipt = TxReceipt {
                tx_hash: tx_hash.clone(),
                block_number: None,
                block_hash: None,
                from,
                to,
                amount,
                token,
                tx_type: tt,
                status: st,
                gas_used,
                gas_price: if gas_used > 0 { fee / gas_used.max(1) } else { 0 },
                fee,
                nonce: 0,
                timestamp: chrono::Utc::now().to_rfc3339(),
                confirmations: 0,
                logs: vec![],
                error_message: None,
                notes: None,
            };
            store.store_receipt(receipt)?;
            store.save(&path)?;
            println!("Receipt stored: {}", tx_hash);
        }
        ReceiptAction::Show { tx_hash } => {
            match store.get_receipt(&tx_hash) {
                Some(r) => {
                    crate::output::json_or(r, || {
                        println!("{}", "Transaction Receipt".bold().cyan());
                        println!("  Hash:     {}", r.tx_hash);
                        println!("  From:     {}", r.from);
                        println!("  To:       {}", r.to);
                        println!("  Amount:   {} {}", r.amount, r.token);
                        println!("  Type:     {:?}", r.tx_type);
                        println!("  Status:   {:?}", r.status);
                        println!("  Gas:      {} (fee: {})", r.gas_used, r.fee);
                        println!("  Block:    {:?}", r.block_number);
                        println!("  Confirms: {}", r.confirmations);
                        println!("  Time:     {}", r.timestamp);
                        if let Some(note) = &r.notes {
                            println!("  Note:     {}", note);
                        }
                    });
                }
                None => println!("Receipt not found: {}", tx_hash),
            }
        }
        ReceiptAction::Update { tx_hash, status, confirmations, block } => {
            let st = parse_receipt_status(&status);
            store.update_receipt(&tx_hash, st, block, confirmations.unwrap_or(0))?;
            store.save(&path)?;
            println!("Receipt updated: {}", tx_hash);
        }
        ReceiptAction::Note { tx_hash, note } => {
            store.add_note(&tx_hash, &note)?;
            store.save(&path)?;
            println!("Note added to {}", tx_hash);
        }
        ReceiptAction::ForAddress { address } => {
            let receipts = store.receipts_by_address(&address);
            crate::output::json_or(&receipts, || {
                if receipts.is_empty() {
                    println!("No receipts for {}", address);
                } else {
                    println!("{} receipts for {}", receipts.len(), address);
                    for r in &receipts {
                        println!("  {} {:?} {} {} [{:?}]",
                            r.tx_hash, r.tx_type, r.amount, r.token, r.status);
                    }
                }
            });
        }
        ReceiptAction::Search { query } => {
            let results = store.search(&query);
            crate::output::json_or(&results, || {
                println!("{} results for '{}'", results.len(), query);
                for r in &results {
                    println!("  {} — {:?} {} {} [{:?}]",
                        r.tx_hash, r.tx_type, r.amount, r.token, r.status);
                }
            });
        }
        ReceiptAction::Summary => {
            let summary = store.summary();
            crate::output::json_or(&summary, || {
                println!("{}", "Receipt Summary".bold().cyan());
                println!("  Total:    {}", summary.total_txs);
                println!("  Success:  {}", summary.successful);
                println!("  Failed:   {}", summary.failed);
                println!("  Pending:  {}", summary.pending);
                println!("  Gas:      {}", summary.total_gas_spent);
                println!("  Fees:     {}", summary.total_fees_paid);
                println!("  Sent:     {}", summary.total_sent);
                println!("  Received: {}", summary.total_received);
            });
        }
        ReceiptAction::SummaryAddr { address } => {
            let summary = store.summary_for_address(&address);
            crate::output::json_or(&summary, || {
                println!("{}", format!("Summary for {}", address).bold().cyan());
                println!("  Total:    {}", summary.total_txs);
                println!("  Success:  {}", summary.successful);
                println!("  Failed:   {}", summary.failed);
                println!("  Pending:  {}", summary.pending);
                println!("  Sent:     {}", summary.total_sent);
                println!("  Received: {}", summary.total_received);
            });
        }
        ReceiptAction::Recent { count } => {
            let recent = store.recent_receipts(count);
            crate::output::json_or(&recent, || {
                if recent.is_empty() {
                    println!("No receipts.");
                } else {
                    println!("{}", "Recent Receipts".bold().cyan());
                    for r in &recent {
                        println!("  {} {:?} {} {} [{:?}] {}",
                            r.tx_hash, r.tx_type, r.amount, r.token, r.status, r.timestamp);
                    }
                }
            });
        }
        ReceiptAction::Pending => {
            let pending = store.pending_receipts();
            crate::output::json_or(&pending, || {
                if pending.is_empty() {
                    println!("No pending receipts.");
                } else {
                    println!("{}", "Pending Receipts".bold().cyan());
                    for r in &pending {
                        println!("  {} → {} {} {}", r.tx_hash, r.to, r.amount, r.token);
                    }
                }
            });
        }
        ReceiptAction::Failed => {
            let failed = store.failed_receipts();
            crate::output::json_or(&failed, || {
                if failed.is_empty() {
                    println!("No failed receipts.");
                } else {
                    println!("{}", "Failed Receipts".bold().cyan());
                    for r in &failed {
                        println!("  {} — {:?} err={:?}",
                            r.tx_hash, r.tx_type, r.error_message);
                    }
                }
            });
        }
    }
    Ok(())
}

fn parse_tx_type2(s: &str) -> crate::tx_receipt_store::TxType2 {
    use crate::tx_receipt_store::TxType2;
    match s.to_lowercase().as_str() {
        "transfer" => TxType2::Transfer,
        "contract_deploy" | "deploy" => TxType2::ContractDeploy,
        "contract_call" | "call" => TxType2::ContractCall,
        "refresh" => TxType2::Refresh,
        "stake" => TxType2::Stake,
        "unstake" => TxType2::Unstake,
        "governance" | "gov" => TxType2::Governance,
        "nft_mint" | "mint" => TxType2::NFTMint,
        "token_transfer" => TxType2::TokenTransfer,
        "bridge" => TxType2::Bridge,
        other => TxType2::Custom(other.to_string()),
    }
}

fn parse_receipt_status(s: &str) -> crate::tx_receipt_store::TxReceiptStatus {
    use crate::tx_receipt_store::TxReceiptStatus;
    match s.to_lowercase().as_str() {
        "success" | "ok" => TxReceiptStatus::Success,
        "failed" | "fail" => TxReceiptStatus::Failed,
        "dropped" => TxReceiptStatus::Dropped,
        _ => TxReceiptStatus::Pending,
    }
}

fn cmd_state_sync(action: StateSyncAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::state_sync::{StateSyncManager, SyncMode, SyncConflict, ConflictResolution};
    let dir = crate::config::default_data_dir();
    let path = dir.join("state_sync.json");
    let mut mgr = StateSyncManager::load_or_default(&path);

    match action {
        StateSyncAction::Track { account, mode } => {
            let m = match mode.to_lowercase().as_str() {
                "light" => SyncMode::Light,
                "checkpoint" => SyncMode::Checkpoint,
                _ => SyncMode::Full,
            };
            mgr.track_account(&account, m)?;
            mgr.save(&path)?;
            println!("Tracking: {}", account);
        }
        StateSyncAction::Untrack { account } => {
            mgr.untrack_account(&account)?;
            mgr.save(&path)?;
            println!("Untracked: {}", account);
        }
        StateSyncAction::Sync { account } => {
            mgr.sync_account(&account)?;
            mgr.save(&path)?;
            let progress = mgr.sync_progress(&account)?;
            println!("Synced: {} ({:.1}%)", account, progress);
        }
        StateSyncAction::SetRemote { account, block } => {
            mgr.update_remote_block(&account, block)?;
            mgr.save(&path)?;
            println!("Remote block set to {} for {}", block, account);
        }
        StateSyncAction::Conflict { account, field, local_value, remote_value } => {
            let id = format!("conflict_{}", chrono::Utc::now().timestamp_millis());
            let conflict = SyncConflict {
                id: id.clone(),
                account,
                field,
                local_value,
                remote_value,
                detected_at: chrono::Utc::now().to_rfc3339(),
                resolved: false,
                resolution: None,
            };
            mgr.record_conflict(conflict);
            mgr.save(&path)?;
            println!("Conflict recorded: {}", id);
        }
        StateSyncAction::Resolve { conflict_id, strategy } => {
            let res = match strategy.to_lowercase().as_str() {
                "prefer_local" | "local" => ConflictResolution::PreferLocal,
                "prefer_remote" | "remote" => ConflictResolution::PreferRemote,
                "latest" => ConflictResolution::Latest,
                _ => ConflictResolution::Manual,
            };
            mgr.resolve_conflict(&conflict_id, res)?;
            mgr.save(&path)?;
            println!("Conflict resolved: {}", conflict_id);
        }
        StateSyncAction::Checkpoint { block_number, block_hash, state_root } => {
            let synced = mgr.accounts.len() as u32;
            mgr.create_checkpoint(block_number, &block_hash, &state_root, synced);
            mgr.save(&path)?;
            println!("Checkpoint created at block {}", block_number);
        }
        StateSyncAction::Behind => {
            let behind = mgr.accounts_behind();
            crate::output::json_or(&behind, || {
                if behind.is_empty() {
                    println!("All accounts synced.");
                } else {
                    println!("{}", "Accounts Behind".bold().cyan());
                    for a in &behind {
                        println!("  {} — {} blocks behind (local={}, remote={})",
                            a.account, a.blocks_behind, a.local_block, a.remote_block);
                    }
                }
            });
        }
        StateSyncAction::Errors => {
            let errs = mgr.accounts_in_error();
            crate::output::json_or(&errs, || {
                if errs.is_empty() {
                    println!("No accounts in error.");
                } else {
                    println!("{}", "Accounts in Error".bold().cyan());
                    for a in &errs {
                        println!("  {} — {:?}", a.account, a.error_message);
                    }
                }
            });
        }
        StateSyncAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "State Sync Stats".bold().cyan());
                println!("  Tracked:    {}", stats.tracked_accounts);
                println!("  Synced:     {}", stats.synced);
                println!("  Behind:     {}", stats.behind);
                println!("  Errors:     {}", stats.errors);
                println!("  Conflicts:  {} ({} resolved)", stats.total_conflicts, stats.resolved_conflicts);
                println!("  Checkpoints: {}", stats.checkpoints);
                if let Some(last) = &stats.last_full_sync {
                    println!("  Last Sync:  {}", last);
                }
            });
        }
    }
    Ok(())
}

fn cmd_debug(action: DebugAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::debug_console::{DebugConsole, Breakpoint, BreakpointStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("debug_console.json");
    let mut console = DebugConsole::load_or_default(&path);

    match action {
        DebugAction::Create { name } => {
            let id = console.create_session(&name)?;
            console.save(&path)?;
            println!("Debug session created: {} ({})", name, id);
        }
        DebugAction::End { id } => {
            console.end_session(&id)?;
            console.save(&path)?;
            println!("Session ended: {}", id);
        }
        DebugAction::Pause { id } => {
            console.pause_session(&id)?;
            console.save(&path)?;
            println!("Session paused: {}", id);
        }
        DebugAction::Resume { id } => {
            console.resume_session(&id)?;
            console.save(&path)?;
            println!("Session resumed: {}", id);
        }
        DebugAction::Break { id, bp_type, condition } => {
            let bpt = parse_bp_type(&bp_type);
            let bp = Breakpoint {
                id: id.clone(),
                bp_type: bpt,
                condition,
                status: BreakpointStatus::Enabled,
                hit_count: 0,
                created_at: chrono::Utc::now().to_rfc3339(),
                last_hit: None,
            };
            console.add_breakpoint(bp)?;
            console.save(&path)?;
            println!("Breakpoint added: {}", id);
        }
        DebugAction::RemoveBreak { id } => {
            console.remove_breakpoint(&id)?;
            console.save(&path)?;
            println!("Breakpoint removed: {}", id);
        }
        DebugAction::EnableBreak { id } => {
            console.enable_breakpoint(&id)?;
            console.save(&path)?;
            println!("Breakpoint enabled: {}", id);
        }
        DebugAction::DisableBreak { id } => {
            console.disable_breakpoint(&id)?;
            console.save(&path)?;
            println!("Breakpoint disabled: {}", id);
        }
        DebugAction::Sessions => {
            let sessions = console.active_sessions();
            crate::output::json_or(&sessions, || {
                if sessions.is_empty() {
                    println!("No active sessions.");
                } else {
                    println!("{}", "Active Debug Sessions".bold().cyan());
                    for s in &sessions {
                        println!("  {} — {} [{:?}] cmds={}", s.id, s.name, s.status, s.commands_run);
                    }
                }
            });
        }
        DebugAction::Breakpoints => {
            let bps = console.enabled_breakpoints();
            crate::output::json_or(&bps, || {
                if bps.is_empty() {
                    println!("No enabled breakpoints.");
                } else {
                    println!("{}", "Enabled Breakpoints".bold().cyan());
                    for b in &bps {
                        println!("  {} — {:?} cond='{}' hits={}",
                            b.id, b.bp_type, b.condition, b.hit_count);
                    }
                }
            });
        }
        DebugAction::Logs { count } => {
            let logs = console.recent_logs(count);
            crate::output::json_or(&logs, || {
                if logs.is_empty() {
                    println!("No debug logs.");
                } else {
                    println!("{}", "Debug Logs".bold().cyan());
                    for l in &logs {
                        println!("  [{:?}] {} — {}", l.level, l.timestamp, l.message);
                    }
                }
            });
        }
        DebugAction::Replays { count } => {
            let replays = console.recent_replays(count);
            crate::output::json_or(&replays, || {
                if replays.is_empty() {
                    println!("No replays.");
                } else {
                    println!("{}", "Recent Replays".bold().cyan());
                    for r in &replays {
                        let icon = if r.success { "+" } else { "X" };
                        println!("  [{}] {} gas={} replayed={}",
                            icon, r.tx_hash, r.gas_used, r.replayed_at);
                    }
                }
            });
        }
        DebugAction::Stats => {
            let stats = console.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Debug Console Stats".bold().cyan());
                println!("  Sessions:     {} ({} active)", stats.total_sessions, stats.active_sessions);
                println!("  Breakpoints:  {} ({} enabled)", stats.total_breakpoints, stats.enabled_breakpoints);
                println!("  Logs:         {}", stats.total_logs);
                println!("  Replays:      {}", stats.total_replays);
                println!("  Commands:     {}", stats.commands_executed);
            });
        }
    }
    Ok(())
}

fn parse_bp_type(s: &str) -> crate::debug_console::BreakpointType {
    use crate::debug_console::BreakpointType;
    match s.to_lowercase().as_str() {
        "event_match" | "event" => BreakpointType::EventMatch,
        "balance_threshold" | "balance" => BreakpointType::BalanceThreshold,
        "block_number" | "block" => BreakpointType::BlockNumber,
        "tx_hash" | "tx" => BreakpointType::TxHash,
        "gas_above" | "gas" => BreakpointType::GasAbove,
        other => BreakpointType::Custom(other.to_string()),
    }
}

fn cmd_gas_profile(action: GasProfileAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::gas_profiler::{GasProfiler, GasSample, OpType};
    let dir = crate::config::default_data_dir();
    let path = dir.join("gas_profiler.json");
    let mut profiler = GasProfiler::load_or_default(&path);

    match action {
        GasProfileAction::Create { id, op_type } => {
            let ot = parse_op_type(&op_type);
            profiler.create_profile(&id, ot)?;
            profiler.save(&path)?;
            println!("Gas profile created: {}", id);
        }
        GasProfileAction::Remove { id } => {
            profiler.remove_profile(&id)?;
            profiler.save(&path)?;
            println!("Profile removed: {}", id);
        }
        GasProfileAction::Sample { profile_id, tx_hash, gas_used, gas_limit, gas_price, block } => {
            let sample = GasSample {
                tx_hash,
                op_type: OpType::Custom("sample".into()),
                gas_used,
                gas_limit,
                gas_price,
                timestamp: chrono::Utc::now().to_rfc3339(),
                block_number: block,
                success: true,
            };
            profiler.add_sample(&profile_id, sample)?;
            profiler.save(&path)?;
            println!("Sample added to {}", profile_id);
        }
        GasProfileAction::Show { id } => {
            match profiler.get_profile(&id) {
                Some(p) => {
                    crate::output::json_or(p, || {
                        println!("{}", format!("Profile: {}", p.id).bold().cyan());
                        println!("  Type:     {:?}", p.op_type);
                        println!("  Samples:  {}", p.samples.len());
                        if !p.samples.is_empty() {
                            println!("  Avg Gas:  {:.0}", p.avg_gas());
                            println!("  Min:      {}", p.min_gas());
                            println!("  Max:      {}", p.max_gas());
                            println!("  Median:   {}", p.median_gas());
                            println!("  P95:      {}", p.p95_gas());
                            println!("  Eff:      {:.1}%", p.efficiency());
                            println!("  Cost:     {}", p.total_cost());
                        }
                    });
                }
                None => println!("Profile not found: {}", id),
            }
        }
        GasProfileAction::Hotspots => {
            let hotspots = profiler.detect_hotspots();
            crate::output::json_or(&hotspots, || {
                if hotspots.is_empty() {
                    println!("No hotspots detected.");
                } else {
                    println!("{}", "Gas Hotspots".bold().cyan());
                    for h in &hotspots {
                        println!("  {:?} — avg={:.0} samples={} cost={} ({:.1}%)",
                            h.op_type, h.avg_gas, h.sample_count, h.total_cost, h.percentage_of_total);
                    }
                }
            });
        }
        GasProfileAction::Suggest => {
            let suggestions = profiler.generate_suggestions();
            crate::output::json_or(&suggestions, || {
                if suggestions.is_empty() {
                    println!("No optimization suggestions.");
                } else {
                    println!("{}", "Optimization Suggestions".bold().cyan());
                    for s in &suggestions {
                        println!("  [{:?}] {:?} — {} (est. savings: {})",
                            s.priority, s.op_type, s.suggestion, s.estimated_savings);
                    }
                }
            });
        }
        GasProfileAction::Stats => {
            let stats = profiler.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Gas Profiler Stats".bold().cyan());
                println!("  Profiles:    {}", stats.total_profiles);
                println!("  Samples:     {}", stats.total_samples);
                println!("  Total Gas:   {}", stats.total_gas_spent);
                println!("  Avg/Tx:      {:.0}", stats.avg_gas_per_tx);
                println!("  Hotspots:    {}", stats.hotspot_count);
                println!("  Suggestions: {}", stats.suggestions_count);
                if let Some(op) = &stats.most_expensive_op {
                    println!("  Most Costly: {}", op);
                }
            });
        }
    }
    Ok(())
}

fn parse_op_type(s: &str) -> crate::gas_profiler::OpType {
    use crate::gas_profiler::OpType;
    match s.to_lowercase().as_str() {
        "transfer" => OpType::Transfer,
        "contract_call" | "call" => OpType::ContractCall,
        "contract_deploy" | "deploy" => OpType::ContractDeploy,
        "refresh" => OpType::Refresh,
        "stake" => OpType::Stake,
        "unstake" => OpType::Unstake,
        "nft_mint" | "mint" => OpType::NFTMint,
        "token_transfer" => OpType::TokenTransfer,
        "bridge" => OpType::Bridge,
        other => OpType::Custom(other.to_string()),
    }
}

fn cmd_verify(action: VerifyAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::contract_verifier::{ContractVerifier, CompilerVersion};
    let dir = crate::config::default_data_dir();
    let path = dir.join("contract_verifier.json");
    let mut verifier = ContractVerifier::load_or_default(&path);

    match action {
        VerifyAction::Register { address, source, compiler } => {
            let cv = match compiler.to_lowercase().as_str() {
                "v2" => CompilerVersion::V2,
                "v3" => CompilerVersion::V3,
                _ => CompilerVersion::V1,
            };
            verifier.register_source(&address, &source, cv)?;
            verifier.save(&path)?;
            println!("Source registered for {}", address);
        }
        VerifyAction::Unregister { address } => {
            verifier.unregister_source(&address)?;
            verifier.save(&path)?;
            println!("Source unregistered: {}", address);
        }
        VerifyAction::Update { address, source } => {
            verifier.update_source(&address, &source)?;
            verifier.save(&path)?;
            println!("Source updated for {}", address);
        }
        VerifyAction::Check { address, bytecode } => {
            let report = verifier.verify_contract(&address, &bytecode)?;
            verifier.save(&path)?;
            crate::output::json_or(&report, || {
                println!("{}", "Verification Report".bold().cyan());
                println!("  Contract: {}", report.contract_address);
                println!("  Status:   {:?}", report.status);
                println!("  Source:   {}", &report.source_hash[..16]);
                println!("  Deployed: {}", &report.deployed_hash[..16]);
                if !report.diffs.is_empty() {
                    println!("  Diffs:    {}", report.diffs.len());
                }
            });
        }
        VerifyAction::Report { address } => {
            match verifier.get_latest_report(&address) {
                Some(r) => {
                    crate::output::json_or(r, || {
                        println!("{}", "Latest Report".bold().cyan());
                        println!("  Status:   {:?}", r.status);
                        println!("  Verified: {}", r.verified_at);
                    });
                }
                None => println!("No report for {}", address),
            }
        }
        VerifyAction::Verified => {
            let list = verifier.verified_contracts();
            crate::output::json_or(&list, || {
                if list.is_empty() {
                    println!("No verified contracts.");
                } else {
                    println!("{}", "Verified Contracts".bold().cyan());
                    for c in &list {
                        println!("  {} — {:?}", c.contract_address, c.compiler_version);
                    }
                }
            });
        }
        VerifyAction::Unverified => {
            let list = verifier.unverified_contracts();
            crate::output::json_or(&list, || {
                if list.is_empty() {
                    println!("All contracts verified.");
                } else {
                    println!("{}", "Unverified Contracts".bold().cyan());
                    for c in &list {
                        println!("  {}", c.contract_address);
                    }
                }
            });
        }
        VerifyAction::Search { query } => {
            let results = verifier.search_contracts(&query);
            crate::output::json_or(&results, || {
                println!("{} contracts matching '{}'", results.len(), query);
                for c in &results {
                    println!("  {} — {:?}", c.contract_address, c.compiler_version);
                }
            });
        }
        VerifyAction::Stats => {
            let stats = verifier.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Contract Verifier Stats".bold().cyan());
                println!("  Total:         {}", stats.total_contracts);
                println!("  Verified:      {}", stats.verified);
                println!("  Failed:        {}", stats.failed);
                println!("  Unverified:    {}", stats.unverified);
                println!("  Verifications: {}", stats.total_verifications);
            });
        }
    }
    Ok(())
}

fn cmd_simulate2(action: SimAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::tx_simulator::{TxSimulator, SimulatedTx, ForkSource};
    let dir = crate::config::default_data_dir();
    let path = dir.join("tx_simulator.json");
    let mut sim = TxSimulator::load_or_default(&path);

    match action {
        SimAction::Fork { id, block, source } => {
            let src = if let Some(rest) = source.strip_prefix("block:") {
                let n: u64 = rest.parse().unwrap_or(block);
                ForkSource::SpecificBlock(n)
            } else if let Some(rest) = source.strip_prefix("snapshot:") {
                ForkSource::Snapshot(rest.to_string())
            } else {
                ForkSource::LatestBlock
            };
            sim.create_fork(&id, src, block)?;
            sim.save(&path)?;
            println!("Fork created: {} at block {}", id, block);
        }
        SimAction::RemoveFork { id } => {
            sim.remove_fork(&id)?;
            sim.save(&path)?;
            println!("Fork removed: {}", id);
        }
        SimAction::Run { from, to, amount, gas_limit } => {
            let tx_id = format!("sim_{}", chrono::Utc::now().timestamp_millis());
            let tx = SimulatedTx {
                id: tx_id.clone(),
                from,
                to,
                amount,
                gas_limit,
                data: None,
                nonce: 0,
            };
            let result = sim.simulate_tx(tx);
            let status = result.status.clone();
            sim.store_result(result)?;
            sim.save(&path)?;
            println!("Simulation {}: {:?}", tx_id, status);
        }
        SimAction::Scenario { id, name, fork_id } => {
            sim.create_scenario(&id, &name, &fork_id)?;
            sim.save(&path)?;
            println!("Scenario created: {} (fork: {})", id, fork_id);
        }
        SimAction::RunScenario { id } => {
            let results = sim.run_scenario(&id)?;
            sim.save(&path)?;
            println!("Scenario {} — {} transactions simulated:", id, results.len());
            for r in &results {
                println!("  {} — {:?} gas={}", r.id, r.status, r.gas_used);
            }
        }
        SimAction::Show { id } => {
            match sim.get_result(&id) {
                Some(r) => {
                    crate::output::json_or(r, || {
                        println!("{}", "Simulation Result".bold().cyan());
                        println!("  ID:      {}", r.id);
                        println!("  Status:  {:?}", r.status);
                        println!("  From:    {}", r.tx.from);
                        println!("  To:      {}", r.tx.to);
                        println!("  Amount:  {}", r.tx.amount);
                        println!("  Gas:     {} / {}", r.gas_used, r.tx.gas_limit);
                        println!("  Changes: {}", r.state_changes.len());
                        if let Some(reason) = &r.revert_reason {
                            println!("  Revert:  {:?}", reason);
                        }
                    });
                }
                None => println!("Simulation not found: {}", id),
            }
        }
        SimAction::Revert { id } => {
            match sim.revert_analysis(&id)? {
                Some(reason) => {
                    println!("Revert reason for {}: {:?}", id, reason);
                }
                None => println!("No revert for simulation {}", id),
            }
        }
        SimAction::Recent { count } => {
            let recent = sim.recent_simulations(count);
            crate::output::json_or(&recent, || {
                if recent.is_empty() {
                    println!("No simulations.");
                } else {
                    println!("{}", "Recent Simulations".bold().cyan());
                    for r in &recent {
                        println!("  {} — {:?} gas={} {}", r.id, r.status, r.gas_used, r.executed_at);
                    }
                }
            });
        }
        SimAction::Stats => {
            let stats = sim.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Simulator Stats".bold().cyan());
                println!("  Total:     {}", stats.total_simulations);
                println!("  Success:   {}", stats.successful);
                println!("  Failed:    {}", stats.failed);
                println!("  Reverted:  {}", stats.reverted);
                println!("  Forks:     {}", stats.total_forks);
                println!("  Scenarios: {}", stats.total_scenarios);
                println!("  Avg Gas:   {:.0}", stats.avg_gas_used);
            });
        }
    }
    Ok(())
}

fn cmd_audit_trail(action: AuditTrailAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::audit_trail::{AuditTrail, AuditSeverity};
    let dir = crate::config::default_data_dir();
    let path = dir.join("audit_trail.json");
    let mut trail = AuditTrail::load_or_default(&path);

    match action {
        AuditTrailAction::Record { action_type, severity, actor, target } => {
            let act = parse_audit_action(&action_type);
            let sev = match severity.to_lowercase().as_str() {
                "warning" | "warn" => AuditSeverity::Warning,
                "critical" | "crit" => AuditSeverity::Critical,
                _ => AuditSeverity::Info,
            };
            let id = trail.record(act, sev, &actor, &target, std::collections::HashMap::new());
            trail.save(&path)?;
            println!("Audit entry recorded: {}", id);
        }
        AuditTrailAction::Verify => {
            let result = trail.verify_chain();
            crate::output::json_or(&result, || {
                match &result {
                    crate::audit_trail::VerifyResult::Valid => {
                        println!("{}", "Chain integrity: VALID".bold().green());
                    }
                    crate::audit_trail::VerifyResult::Broken(idx) => {
                        println!("{}", format!("Chain integrity: BROKEN at index {}", idx).bold().red());
                    }
                }
            });
        }
        AuditTrailAction::Recent { count } => {
            let entries = trail.recent_entries(count);
            crate::output::json_or(&entries, || {
                if entries.is_empty() {
                    println!("No audit entries.");
                } else {
                    println!("{}", "Recent Audit Entries".bold().cyan());
                    for e in &entries {
                        println!("  [{}] {:?} ({:?}) {} → {} [{}]",
                            e.sequence, e.action, e.severity, e.actor, e.target, e.timestamp);
                    }
                }
            });
        }
        AuditTrailAction::Critical => {
            let entries = trail.critical_entries();
            crate::output::json_or(&entries, || {
                if entries.is_empty() {
                    println!("No critical entries.");
                } else {
                    println!("{}", "Critical Audit Entries".bold().red());
                    for e in &entries {
                        println!("  [{}] {:?} {} → {}", e.sequence, e.action, e.actor, e.target);
                    }
                }
            });
        }
        AuditTrailAction::Search { query } => {
            let results = trail.search(&query);
            crate::output::json_or(&results, || {
                println!("{} results for '{}'", results.len(), query);
                for e in &results {
                    println!("  [{}] {:?} {} → {}", e.sequence, e.action, e.actor, e.target);
                }
            });
        }
        AuditTrailAction::Export => {
            let export = trail.export_all();
            crate::output::json_or(&export, || {
                println!("{}", "Audit Export".bold().cyan());
                println!("  Entries: {}", export.total_entries);
                println!("  Valid:   {}", export.chain_valid);
                println!("  Date:    {}", export.exported_at);
            });
        }
        AuditTrailAction::Stats => {
            let stats = trail.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Audit Trail Stats".bold().cyan());
                println!("  Entries:  {}", stats.total_entries);
                println!("  Valid:    {}", stats.chain_valid);
                println!("  Actors:   {}", stats.unique_actors);
                if let Some(first) = &stats.first_entry {
                    println!("  First:    {}", first);
                }
                if let Some(last) = &stats.last_entry {
                    println!("  Last:     {}", last);
                }
            });
        }
    }
    Ok(())
}

fn parse_audit_action(s: &str) -> crate::audit_trail::AuditAction {
    use crate::audit_trail::AuditAction;
    match s.to_lowercase().as_str() {
        "key_generated" => AuditAction::KeyGenerated,
        "key_imported" => AuditAction::KeyImported,
        "key_deleted" => AuditAction::KeyDeleted,
        "tx_signed" => AuditAction::TxSigned,
        "tx_submitted" => AuditAction::TxSubmitted,
        "tx_confirmed" => AuditAction::TxConfirmed,
        "setting_changed" => AuditAction::SettingChanged,
        "login_attempt" | "login" => AuditAction::LoginAttempt,
        "backup_created" => AuditAction::BackupCreated,
        "backup_restored" => AuditAction::BackupRestored,
        "permission_granted" => AuditAction::PermissionGranted,
        "permission_revoked" => AuditAction::PermissionRevoked,
        other => AuditAction::Custom(other.to_string()),
    }
}

fn cmd_anomaly(action: AnomalyAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::anomaly_detector::{AnomalyDetector, DetectionRule, RuleStatus};
    let dir = crate::config::default_data_dir();
    let path = dir.join("anomaly_detector.json");
    let mut detector = AnomalyDetector::load_or_default(&path);

    match action {
        AnomalyAction::AddRule { id, anomaly_type, threshold, desc } => {
            let at = parse_anomaly_type(&anomaly_type);
            let rule = DetectionRule {
                id: id.clone(),
                anomaly_type: at,
                threshold,
                status: RuleStatus::Enabled,
                description: desc,
                created_at: chrono::Utc::now().to_rfc3339(),
                triggers: 0,
            };
            detector.add_rule(rule)?;
            detector.save(&path)?;
            println!("Rule added: {}", id);
        }
        AnomalyAction::RemoveRule { id } => {
            detector.remove_rule(&id)?;
            detector.save(&path)?;
            println!("Rule removed: {}", id);
        }
        AnomalyAction::EnableRule { id } => {
            detector.enable_rule(&id)?;
            detector.save(&path)?;
            println!("Rule enabled: {}", id);
        }
        AnomalyAction::DisableRule { id } => {
            detector.disable_rule(&id)?;
            detector.save(&path)?;
            println!("Rule disabled: {}", id);
        }
        AnomalyAction::Profile { address } => {
            match detector.get_profile(&address) {
                Some(p) => {
                    crate::output::json_or(p, || {
                        println!("{}", format!("Profile: {}", p.address).bold().cyan());
                        println!("  Avg Amount:    {:.2}", p.avg_amount);
                        println!("  Max Amount:    {}", p.max_amount);
                        println!("  Tx Count:      {}", p.tx_count);
                        println!("  Recipients:    {}", p.unique_recipients);
                        println!("  Avg Gas:       {:.2}", p.avg_gas);
                        println!("  Common Hours:  {:?}", p.common_hours);
                    });
                }
                None => println!("No profile for {}", address),
            }
        }
        AnomalyAction::Alerts => {
            let alerts = detector.unacknowledged_alerts();
            crate::output::json_or(&alerts, || {
                if alerts.is_empty() {
                    println!("No unacknowledged alerts.");
                } else {
                    println!("{}", "Unacknowledged Alerts".bold().red());
                    for a in &alerts {
                        println!("  [{}] {:?} ({:?}) tx={} — {}",
                            a.id, a.anomaly_type, a.risk_level, a.tx_hash, a.details);
                    }
                }
            });
        }
        AnomalyAction::Ack { alert_id } => {
            detector.acknowledge_alert(&alert_id)?;
            detector.save(&path)?;
            println!("Alert acknowledged: {}", alert_id);
        }
        AnomalyAction::Risk { address } => {
            let score = detector.risk_score(&address);
            println!("Risk score for {}: {:.0}/100", address, score);
        }
        AnomalyAction::Recent { count } => {
            let alerts = detector.recent_alerts(count);
            crate::output::json_or(&alerts, || {
                if alerts.is_empty() {
                    println!("No alerts.");
                } else {
                    println!("{}", "Recent Alerts".bold().cyan());
                    for a in &alerts {
                        let ack = if a.acknowledged { "ack" } else { "NEW" };
                        println!("  [{}] {:?} ({:?}) [{}]", a.id, a.anomaly_type, a.risk_level, ack);
                    }
                }
            });
        }
        AnomalyAction::Stats => {
            let stats = detector.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Anomaly Detector Stats".bold().cyan());
                println!("  Rules:     {} ({} enabled)", stats.total_rules, stats.enabled_rules);
                println!("  Alerts:    {} ({} unack)", stats.total_alerts, stats.unacknowledged);
                println!("  Samples:   {}", stats.total_samples);
                println!("  Profiles:  {}", stats.profiles);
            });
        }
    }
    Ok(())
}

fn parse_anomaly_type(s: &str) -> crate::anomaly_detector::AnomalyType {
    use crate::anomaly_detector::AnomalyType;
    match s.to_lowercase().as_str() {
        "unusual_amount" | "amount" => AnomalyType::UnusualAmount,
        "high_velocity" | "velocity" => AnomalyType::HighVelocity,
        "new_recipient" | "recipient" => AnomalyType::NewRecipient,
        "large_gas" | "gas" => AnomalyType::LargeGas,
        "off_hours" | "hours" => AnomalyType::OffHoursActivity,
        "rapid_sequence" | "rapid" => AnomalyType::RapidSequence,
        "dust_attack" | "dust" => AnomalyType::DustAttack,
        other => AnomalyType::Custom(other.to_string()),
    }
}

fn cmd_enclave(action: EnclaveAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::secure_enclave::{SecureEnclave, KeyPurpose};
    let dir = crate::config::default_data_dir();
    let path = dir.join("secure_enclave.json");
    let mut enclave = SecureEnclave::load_or_default(&path);

    match action {
        EnclaveAction::Store { id, material, purpose, expires } => {
            let p = match purpose.to_lowercase().as_str() {
                "encryption" => KeyPurpose::Encryption,
                "authentication" | "auth" => KeyPurpose::Authentication,
                "derivation" => KeyPurpose::Derivation,
                _ => KeyPurpose::Signing,
            };
            enclave.store_key(&id, &material, p, expires)?;
            enclave.save(&path)?;
            println!("Key stored in enclave: {}", id);
        }
        EnclaveAction::Remove { id } => {
            enclave.remove_key(&id)?;
            enclave.save(&path)?;
            println!("Key removed: {}", id);
        }
        EnclaveAction::Lock { id } => {
            enclave.lock_key(&id)?;
            enclave.save(&path)?;
            println!("Key locked: {}", id);
        }
        EnclaveAction::Unlock { id } => {
            enclave.unlock_key(&id)?;
            enclave.save(&path)?;
            println!("Key unlocked: {}", id);
        }
        EnclaveAction::Wipe { id } => {
            enclave.wipe_key(&id)?;
            enclave.save(&path)?;
            println!("Key wiped: {}", id);
        }
        EnclaveAction::Seal => {
            enclave.seal_enclave();
            enclave.save(&path)?;
            println!("{}", "Enclave sealed.".bold().yellow());
        }
        EnclaveAction::Unseal => {
            enclave.unseal_enclave();
            enclave.save(&path)?;
            println!("Enclave unsealed.");
        }
        EnclaveAction::VerifyKey { id, material } => {
            let valid = enclave.verify_key_integrity(&id, &material)?;
            if valid {
                println!("{}", "Key integrity: VALID".bold().green());
            } else {
                println!("{}", "Key integrity: MISMATCH".bold().red());
            }
        }
        EnclaveAction::Keys => {
            let keys = enclave.active_keys();
            crate::output::json_or(&keys, || {
                if keys.is_empty() {
                    println!("No active keys.");
                } else {
                    println!("{}", "Active Enclave Keys".bold().cyan());
                    for k in &keys {
                        println!("  {} — {:?} [{:?}] accesses={}",
                            k.id, k.purpose, k.status, k.access_count);
                    }
                }
            });
        }
        EnclaveAction::Stats => {
            let stats = enclave.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Secure Enclave Stats".bold().cyan());
                println!("  Status:   {:?}", stats.enclave_status);
                println!("  Keys:     {} ({} active)", stats.total_keys, stats.active_keys);
                println!("  Locked:   {}", stats.locked_keys);
                println!("  Expired:  {}", stats.expired_keys);
                println!("  Wiped:    {}", stats.wiped_keys);
                println!("  Accesses: {}", stats.total_accesses);
                println!("  Tampers:  {}", stats.tamper_events);
            });
        }
    }
    Ok(())
}

fn cmd_perms(action: PermAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::permission_manager::{PermissionManager, Permission, PermissionStatus3, SpendLimit};
    let dir = crate::config::default_data_dir();
    let path = dir.join("permission_manager.json");
    let mut mgr = PermissionManager::load_or_default(&path);

    match action {
        PermAction::Grant { id, dapp, perm_type, max_uses, expires } => {
            let pt = parse_perm_type(&perm_type);
            let perm = Permission {
                id: id.clone(),
                dapp_id: dapp,
                permission_type: pt,
                status: PermissionStatus3::Pending,
                granted_at: None,
                expires_at: expires,
                max_uses: if max_uses == 0 { None } else { Some(max_uses) },
                use_count: 0,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            mgr.grant_permission(perm)?;
            mgr.save(&path)?;
            println!("Permission granted: {}", id);
        }
        PermAction::Revoke { id } => {
            mgr.revoke_permission(&id)?;
            mgr.save(&path)?;
            println!("Permission revoked: {}", id);
        }
        PermAction::Deny { id } => {
            mgr.deny_permission(&id)?;
            mgr.save(&path)?;
            println!("Permission denied: {}", id);
        }
        PermAction::Limit { dapp, token, max_per_tx, max_daily } => {
            let limit = SpendLimit {
                dapp_id: dapp.clone(),
                contract_address: None,
                token,
                max_per_tx,
                max_daily,
                spent_today: 0,
                last_reset: chrono::Utc::now().to_rfc3339(),
            };
            mgr.set_spend_limit(limit);
            mgr.save(&path)?;
            println!("Spend limit set for {}: {} per tx, {} daily", dapp, max_per_tx, max_daily);
        }
        PermAction::CheckSpend { dapp, amount } => {
            match mgr.check_spend(&dapp, amount) {
                Ok(()) => println!("{}", "Spend allowed.".bold().green()),
                Err(e) => println!("{}", format!("Spend denied: {}", e).bold().red()),
            }
        }
        PermAction::Spend { dapp, amount } => {
            mgr.record_spend(&dapp, amount)?;
            mgr.save(&path)?;
            println!("Recorded spend of {} for {}", amount, dapp);
        }
        PermAction::ResetSpend { dapp } => {
            mgr.reset_daily_spend(&dapp)?;
            mgr.save(&path)?;
            println!("Daily spend reset for {}", dapp);
        }
        PermAction::ForDapp { dapp } => {
            let perms = mgr.permissions_for_dapp(&dapp);
            crate::output::json_or(&perms, || {
                if perms.is_empty() {
                    println!("No permissions for {}", dapp);
                } else {
                    println!("{}", format!("Permissions for {}", dapp).bold().cyan());
                    for p in &perms {
                        println!("  {} — {:?} [{:?}] uses={}/{}",
                            p.id, p.permission_type, p.status, p.use_count,
                            p.max_uses.map(|m| m.to_string()).unwrap_or("∞".into()));
                    }
                }
            });
        }
        PermAction::Pending => {
            let pending = mgr.pending_approvals();
            crate::output::json_or(&pending, || {
                if pending.is_empty() {
                    println!("No pending approvals.");
                } else {
                    println!("{}", "Pending Approvals".bold().yellow());
                    for a in &pending {
                        println!("  {} — dapp={} {:?} reason={}",
                            a.id, a.dapp_id, a.permission_type, a.reason);
                    }
                }
            });
        }
        PermAction::Granted => {
            let granted = mgr.granted_permissions();
            crate::output::json_or(&granted, || {
                if granted.is_empty() {
                    println!("No granted permissions.");
                } else {
                    println!("{}", "Granted Permissions".bold().green());
                    for p in &granted {
                        println!("  {} — {:?} dapp={}", p.id, p.permission_type, p.dapp_id);
                    }
                }
            });
        }
        PermAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Permission Stats".bold().cyan());
                println!("  Total:    {}", stats.total_permissions);
                println!("  Granted:  {}", stats.granted);
                println!("  Denied:   {}", stats.denied);
                println!("  Pending:  {}", stats.pending);
                println!("  Expired:  {}", stats.expired);
                println!("  Revoked:  {}", stats.revoked);
                println!("  Limits:   {}", stats.total_spend_limits);
                println!("  Approvals: {} ({} pending)", stats.total_approvals, stats.pending_approvals);
            });
        }
    }
    Ok(())
}

fn parse_perm_type(s: &str) -> crate::permission_manager::PermissionType {
    use crate::permission_manager::PermissionType;
    match s.to_lowercase().as_str() {
        "read_balance" | "balance" => PermissionType::ReadBalance,
        "sign_transaction" | "sign" => PermissionType::SignTransaction,
        "send_tokens" | "send" => PermissionType::SendTokens,
        "deploy_contract" | "deploy" => PermissionType::DeployContract,
        "manage_nft" | "nft" => PermissionType::ManageNFT,
        "access_private_key" | "key" => PermissionType::AccessPrivateKey,
        "connect_dapp" | "connect" => PermissionType::ConnectDapp,
        other => PermissionType::Custom(other.to_string()),
    }
}

fn cmd_theme(action: ThemeAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::theme_engine::{ThemeEngine, Theme, ThemeColors, ColorScheme, LayoutPreset};
    let dir = crate::config::default_data_dir();
    let path = dir.join("theme_engine.json");
    let mut engine = ThemeEngine::load_or_default(&path);

    match action {
        ThemeAction::Init => {
            engine.register_defaults();
            engine.save(&path)?;
            println!("Registered {} built-in themes.", engine.list_themes().len());
        }
        ThemeAction::Add { id, name, scheme, layout } => {
            let cs = match scheme.to_lowercase().as_str() {
                "dark" => ColorScheme::Dark,
                "high_contrast" => ColorScheme::HighContrast,
                _ => ColorScheme::Light,
            };
            let lp = match layout.to_lowercase().as_str() {
                "compact" => LayoutPreset::Compact,
                "detailed" => LayoutPreset::Detailed,
                "minimal" => LayoutPreset::Minimal,
                _ => LayoutPreset::Standard,
            };
            let colors = match cs {
                ColorScheme::Dark => ThemeColors::default_dark(),
                _ => ThemeColors::default_light(),
            };
            let theme = Theme {
                id: id.clone(),
                name,
                description: String::new(),
                scheme: cs,
                layout: lp,
                colors,
                custom_vars: std::collections::HashMap::new(),
                created_at: chrono::Utc::now().to_rfc3339(),
                is_builtin: false,
            };
            engine.add_theme(theme)?;
            engine.save(&path)?;
            println!("Theme added: {}", id);
        }
        ThemeAction::Remove { id } => {
            engine.remove_theme(&id)?;
            engine.save(&path)?;
            println!("Theme removed: {}", id);
        }
        ThemeAction::Use { id } => {
            engine.set_active(&id)?;
            engine.save(&path)?;
            println!("Active theme: {}", id);
        }
        ThemeAction::Active => {
            match engine.get_active() {
                Some(t) => {
                    crate::output::json_or(t, || {
                        println!("{}", format!("Active: {} ({})", t.name, t.id).bold().cyan());
                        println!("  Scheme: {:?}  Layout: {:?}", t.scheme, t.layout);
                    });
                }
                None => println!("No active theme set."),
            }
        }
        ThemeAction::List => {
            let themes = engine.list_themes();
            crate::output::json_or(&themes, || {
                println!("{}", "Themes".bold().cyan());
                for t in &themes {
                    let builtin = if t.is_builtin { " [builtin]" } else { "" };
                    println!("  {} — {} {:?}/{:?}{}", t.id, t.name, t.scheme, t.layout, builtin);
                }
            });
        }
        ThemeAction::Duplicate { id, new_id, new_name } => {
            engine.duplicate_theme(&id, &new_id, &new_name)?;
            engine.save(&path)?;
            println!("Theme duplicated: {} → {}", id, new_id);
        }
        ThemeAction::SetVar { theme_id, key, value } => {
            engine.set_custom_var(&theme_id, &key, &value)?;
            engine.save(&path)?;
            println!("Set {}={} on {}", key, value, theme_id);
        }
        ThemeAction::Export { id } => {
            let json = engine.export_theme(&id)?;
            println!("{}", json);
        }
        ThemeAction::Stats => {
            let stats = engine.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Theme Stats".bold().cyan());
                println!("  Total:   {}", stats.total_themes);
                println!("  Builtin: {}", stats.builtin);
                println!("  Custom:  {}", stats.custom);
                if let Some(active) = &stats.active_theme {
                    println!("  Active:  {}", active);
                }
            });
        }
    }
    Ok(())
}

fn cmd_palette(action: PaletteAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::command_palette::{CommandPalette, PaletteCommand};
    let dir = crate::config::default_data_dir();
    let path = dir.join("command_palette.json");
    let mut palette = CommandPalette::load_or_default(&path);

    match action {
        PaletteAction::Register { id, name, desc, category, usage } => {
            let cat = parse_cmd_category(&category);
            let cmd = PaletteCommand {
                id: id.clone(),
                name,
                description: desc,
                category: cat,
                usage,
                examples: vec![],
                tags: vec![],
                use_count: 0,
                last_used: None,
            };
            palette.register_command(cmd)?;
            palette.save(&path)?;
            println!("Command registered: {}", id);
        }
        PaletteAction::Remove { id } => {
            palette.remove_command(&id)?;
            palette.save(&path)?;
            println!("Command removed: {}", id);
        }
        PaletteAction::Alias { alias, command_id } => {
            palette.add_alias(&alias, &command_id)?;
            palette.save(&path)?;
            println!("Alias '{}' → {}", alias, command_id);
        }
        PaletteAction::Unalias { alias } => {
            palette.remove_alias(&alias)?;
            palette.save(&path)?;
            println!("Alias removed: {}", alias);
        }
        PaletteAction::Search { query } => {
            let results = palette.fuzzy_search(&query);
            crate::output::json_or(&results, || {
                if results.is_empty() {
                    println!("No matches for '{}'", query);
                } else {
                    println!("{}", format!("Results for '{}'", query).bold().cyan());
                    for r in &results {
                        println!("  [{:.0}] {} — {}", r.score, r.command.name, r.command.description);
                    }
                }
            });
        }
        PaletteAction::TopUsed { count } => {
            let top = palette.most_used(count);
            crate::output::json_or(&top, || {
                if top.is_empty() {
                    println!("No commands used yet.");
                } else {
                    println!("{}", "Most Used Commands".bold().cyan());
                    for c in &top {
                        println!("  {} — {} uses", c.name, c.use_count);
                    }
                }
            });
        }
        PaletteAction::Recent { count } => {
            let recent = palette.recent_commands(count);
            crate::output::json_or(&recent, || {
                if recent.is_empty() {
                    println!("No recent commands.");
                } else {
                    println!("{}", "Recent Commands".bold().cyan());
                    for c in &recent {
                        println!("  {}", c.name);
                    }
                }
            });
        }
        PaletteAction::Stats => {
            let stats = palette.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Command Palette Stats".bold().cyan());
                println!("  Commands: {}", stats.total_commands);
                println!("  Aliases:  {}", stats.total_aliases);
                println!("  Uses:     {}", stats.total_uses);
                if let Some(top) = &stats.most_used {
                    println!("  Top:      {}", top);
                }
            });
        }
    }
    Ok(())
}

fn parse_cmd_category(s: &str) -> crate::command_palette::CommandCategory {
    use crate::command_palette::CommandCategory;
    match s.to_lowercase().as_str() {
        "account" => CommandCategory::Account,
        "transaction" | "tx" => CommandCategory::Transaction,
        "energy" => CommandCategory::Energy,
        "staking" => CommandCategory::Staking,
        "defi" => CommandCategory::DeFi,
        "security" => CommandCategory::Security,
        "network" => CommandCategory::Network,
        "utility" => CommandCategory::Utility,
        other => CommandCategory::Custom(other.to_string()),
    }
}

fn cmd_onboard(action: OnboardAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::onboarding::{OnboardingManager, FlowType};
    let dir = crate::config::default_data_dir();
    let path = dir.join("onboarding.json");
    let mut mgr = OnboardingManager::load_or_default(&path);

    match action {
        OnboardAction::Start { flow_type } => {
            let ft = match flow_type.to_lowercase().as_str() {
                "import" => FlowType::ImportUser,
                "developer" | "dev" => FlowType::DeveloperSetup,
                "quick" => FlowType::QuickStart,
                _ => FlowType::NewUser,
            };
            let flow_id = mgr.create_flow(ft);
            mgr.start_flow(&flow_id)?;
            mgr.save(&path)?;
            println!("Onboarding started: {}", flow_id);
            if let Some(step) = mgr.get_current_step() {
                println!("  First step: {} — {}", step.title, step.description);
            }
        }
        OnboardAction::Complete { step_id } => {
            mgr.complete_step(&step_id)?;
            mgr.save(&path)?;
            println!("Step completed: {}", step_id);
            if let Some(step) = mgr.get_current_step() {
                println!("  Next: {} — {}", step.title, step.description);
            } else {
                println!("  All steps complete!");
            }
        }
        OnboardAction::Skip { step_id } => {
            mgr.skip_step(&step_id)?;
            mgr.save(&path)?;
            println!("Step skipped: {}", step_id);
        }
        OnboardAction::Current => {
            match mgr.get_current_step() {
                Some(s) => {
                    crate::output::json_or(s, || {
                        println!("{}", format!("Current: {}", s.title).bold().cyan());
                        println!("  {}", s.description);
                        for tip in &s.tips {
                            println!("  Tip: {}", tip);
                        }
                    });
                }
                None => println!("No current step (flow complete or not started)."),
            }
        }
        OnboardAction::Progress => {
            if let Some(flow) = mgr.active_flow() {
                let flow_id = flow.id.clone();
                let pct = mgr.flow_progress(&flow_id)?;
                println!("Progress: {:.0}%", pct);
            } else {
                println!("No active flow.");
            }
        }
        OnboardAction::Reset => {
            if let Some(flow) = mgr.active_flow() {
                let flow_id = flow.id.clone();
                mgr.reset_flow(&flow_id)?;
                mgr.save(&path)?;
                println!("Flow reset.");
            } else {
                println!("No active flow to reset.");
            }
        }
        OnboardAction::Tips => {
            let tips = mgr.unshown_tips();
            crate::output::json_or(&tips, || {
                if tips.is_empty() {
                    println!("No new tips.");
                } else {
                    println!("{}", "Tips".bold().cyan());
                    for t in &tips {
                        println!("  {} — {}", t.title, t.content);
                    }
                }
            });
        }
        OnboardAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Onboarding Stats".bold().cyan());
                println!("  Flows:     {} ({} completed)", stats.total_flows, stats.completed_flows);
                println!("  Steps:     {} ({} done, {} skipped)",
                    stats.total_steps, stats.completed_steps, stats.skipped_steps);
                println!("  Progress:  {:.0}%", stats.progress_pct);
                println!("  Tips:      {}/{} shown", stats.tips_shown, stats.tips_total);
            });
        }
    }
    Ok(())
}

fn cmd_help(action: HelpAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::help_system::{HelpSystem, Difficulty};
    let dir = crate::config::default_data_dir();
    let path = dir.join("help_system.json");
    let mut help = HelpSystem::load_or_default(&path);

    match action {
        HelpAction::Search { query } => {
            let results = help.search_topics(&query);
            crate::output::json_or(&results, || {
                if results.is_empty() {
                    println!("No topics found for '{}'", query);
                } else {
                    println!("{}", format!("Topics for '{}'", query).bold().cyan());
                    for t in &results {
                        println!("  {} — {}", t.id, t.title);
                    }
                }
            });
        }
        HelpAction::View { id } => {
            let topic = help.view_topic(&id)?;
            crate::output::json_or(topic, || {
                println!("{}", topic.title.bold().cyan());
                println!("{}", topic.content);
                if !topic.tags.is_empty() {
                    println!("\nTags: {}", topic.tags.join(", "));
                }
            });
            help.save(&path)?;
        }
        HelpAction::Category { cat } => {
            let hc = parse_help_category(&cat);
            let topics = help.topics_by_category(&hc);
            crate::output::json_or(&topics, || {
                if topics.is_empty() {
                    println!("No topics in category '{}'", cat);
                } else {
                    println!("{}", format!("Category: {}", cat).bold().cyan());
                    for t in &topics {
                        println!("  {} — {}", t.id, t.title);
                    }
                }
            });
        }
        HelpAction::Faq { query } => {
            let results = help.search_faq(&query);
            crate::output::json_or(&results, || {
                if results.is_empty() {
                    println!("No FAQ matches for '{}'", query);
                } else {
                    for f in &results {
                        println!("{}", format!("Q: {}", f.question).bold());
                        println!("A: {}\n", f.answer);
                    }
                }
            });
        }
        HelpAction::Tutorials { difficulty } => {
            let tutorials: Vec<_> = if let Some(d) = difficulty {
                let diff = match d.to_lowercase().as_str() {
                    "intermediate" => Difficulty::Intermediate,
                    "advanced" => Difficulty::Advanced,
                    _ => Difficulty::Beginner,
                };
                help.tutorials_by_difficulty(&diff)
            } else {
                help.tutorials_by_difficulty(&Difficulty::Beginner)
            };
            crate::output::json_or(&tutorials, || {
                if tutorials.is_empty() {
                    println!("No tutorials found.");
                } else {
                    println!("{}", "Tutorials".bold().cyan());
                    for t in &tutorials {
                        let done = if t.completed { " [done]" } else { "" };
                        println!("  {} — {} ({:?}, ~{}min){}",
                            t.id, t.title, t.difficulty, t.estimated_minutes, done);
                    }
                }
            });
        }
        HelpAction::Explain { code } => {
            match help.explain_error(&code) {
                Some(e) => {
                    crate::output::json_or(e, || {
                        println!("{}", format!("Error: {} — {}", e.error_code, e.title).bold().red());
                        println!("{}", e.explanation);
                        println!("\n{}", "Solution:".bold().green());
                        println!("{}", e.solution);
                    });
                }
                None => println!("No explanation for error code '{}'", code),
            }
        }
        HelpAction::Popular { count } => {
            let topics = help.popular_topics(count);
            crate::output::json_or(&topics, || {
                if topics.is_empty() {
                    println!("No topics yet.");
                } else {
                    println!("{}", "Popular Topics".bold().cyan());
                    for t in &topics {
                        println!("  {} — {} ({} views)", t.id, t.title, t.views);
                    }
                }
            });
        }
        HelpAction::Stats => {
            let stats = help.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Help System Stats".bold().cyan());
                println!("  Topics:    {}", stats.total_topics);
                println!("  FAQs:      {}", stats.total_faqs);
                println!("  Tutorials: {} ({} done)", stats.total_tutorials, stats.completed_tutorials);
                println!("  Views:     {}", stats.total_views);
                println!("  Errors:    {}", stats.error_explanations);
                if let Some(top) = &stats.most_viewed {
                    println!("  Popular:   {}", top);
                }
            });
        }
    }
    Ok(())
}

fn parse_help_category(s: &str) -> crate::help_system::HelpCategory2 {
    use crate::help_system::HelpCategory2;
    match s.to_lowercase().as_str() {
        "getting_started" | "start" => HelpCategory2::GettingStarted,
        "accounts" | "account" => HelpCategory2::Accounts,
        "transactions" | "tx" => HelpCategory2::Transactions,
        "energy" => HelpCategory2::Energy,
        "security" => HelpCategory2::Security,
        "defi" => HelpCategory2::DeFi,
        "advanced" => HelpCategory2::Advanced,
        _ => HelpCategory2::Troubleshooting,
    }
}

fn cmd_breaker(action: BreakerAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::circuit_breaker::{CircuitBreakerManager, CircuitConfig};
    let dir = crate::config::default_data_dir();
    let path = dir.join("circuit_breaker.json");
    let mut mgr = CircuitBreakerManager::load_or_default(&path);

    match action {
        BreakerAction::Register { id, service, threshold, timeout } => {
            let config = CircuitConfig {
                failure_threshold: threshold,
                timeout_seconds: timeout,
                ..CircuitConfig::default()
            };
            mgr.register(&id, &service, config)?;
            mgr.save(&path)?;
            println!("Circuit registered: {} ({})", id, service);
        }
        BreakerAction::Unregister { id } => {
            mgr.unregister(&id)?;
            mgr.save(&path)?;
            println!("Circuit removed: {}", id);
        }
        BreakerAction::Success { id } => {
            mgr.record_success(&id)?;
            mgr.save(&path)?;
            let state = mgr.get_circuit(&id).map(|c| format!("{:?}", c.state)).unwrap_or_default();
            println!("Success recorded for {} [{}]", id, state);
        }
        BreakerAction::Failure { id } => {
            mgr.record_failure(&id)?;
            mgr.save(&path)?;
            let state = mgr.get_circuit(&id).map(|c| format!("{:?}", c.state)).unwrap_or_default();
            println!("Failure recorded for {} [{}]", id, state);
        }
        BreakerAction::Check { id } => {
            let allowed = mgr.can_execute(&id)?;
            mgr.save(&path)?;
            if allowed {
                println!("{}", "Circuit allows execution.".bold().green());
            } else {
                println!("{}", "Circuit is OPEN — request blocked.".bold().red());
            }
        }
        BreakerAction::ForceOpen { id } => {
            mgr.force_open(&id)?;
            mgr.save(&path)?;
            println!("Circuit force-opened: {}", id);
        }
        BreakerAction::ForceClose { id } => {
            mgr.force_close(&id)?;
            mgr.save(&path)?;
            println!("Circuit force-closed: {}", id);
        }
        BreakerAction::Open => {
            let open = mgr.open_circuits();
            crate::output::json_or(&open, || {
                if open.is_empty() {
                    println!("No open circuits.");
                } else {
                    println!("{}", "Open Circuits".bold().red());
                    for c in &open {
                        println!("  {} — {} failures={}", c.id, c.service_name, c.failure_count);
                    }
                }
            });
        }
        BreakerAction::Events { count } => {
            let events = mgr.recent_events(count);
            crate::output::json_or(&events, || {
                if events.is_empty() {
                    println!("No events.");
                } else {
                    println!("{}", "Circuit Events".bold().cyan());
                    for e in &events {
                        println!("  {} — {} {:?}→{:?}", e.circuit_id, e.event_type, e.from_state, e.to_state);
                    }
                }
            });
        }
        BreakerAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Circuit Breaker Stats".bold().cyan());
                println!("  Circuits:  {}", stats.total_circuits);
                println!("  Closed:    {}", stats.closed);
                println!("  Open:      {}", stats.open);
                println!("  HalfOpen:  {}", stats.half_open);
                println!("  Requests:  {}", stats.total_requests);
                println!("  Failures:  {}", stats.total_failures);
            });
        }
    }
    Ok(())
}

fn cmd_cache(action: CacheAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::cache_manager::{CacheManager, CacheConfig2, CacheLayer, EvictionPolicy};
    let dir = crate::config::default_data_dir();
    let path = dir.join("cache_manager.json");
    let mut mgr = CacheManager::load_or_default(&path);

    match action {
        CacheAction::Create { id, name, layer, max_entries, eviction } => {
            let l = match layer.to_lowercase().as_str() {
                "disk" => CacheLayer::Disk,
                "remote" => CacheLayer::Remote,
                _ => CacheLayer::Memory,
            };
            let e = match eviction.to_lowercase().as_str() {
                "lfu" => EvictionPolicy::Lfu,
                "fifo" => EvictionPolicy::Fifo,
                "ttl" => EvictionPolicy::Ttl,
                _ => EvictionPolicy::Lru,
            };
            let config = CacheConfig2 {
                id: id.clone(),
                name,
                layer: l,
                max_entries,
                eviction: e,
                default_ttl_seconds: None,
            };
            mgr.create_cache(config)?;
            mgr.save(&path)?;
            println!("Cache created: {}", id);
        }
        CacheAction::Remove { id } => {
            mgr.remove_cache(&id)?;
            mgr.save(&path)?;
            println!("Cache removed: {}", id);
        }
        CacheAction::Put { cache_id, key, value, ttl } => {
            mgr.put(&cache_id, &key, &value, ttl)?;
            mgr.save(&path)?;
            println!("Cached {}={}", key, value);
        }
        CacheAction::Get { cache_id, key } => {
            match mgr.get(&cache_id, &key)? {
                Some(v) => println!("{}", v),
                None => println!("(miss)"),
            }
            mgr.save(&path)?;
        }
        CacheAction::Invalidate { cache_id, key } => {
            mgr.invalidate(&cache_id, &key)?;
            mgr.save(&path)?;
            println!("Invalidated {}/{}", cache_id, key);
        }
        CacheAction::Clear { cache_id } => {
            mgr.invalidate_all(&cache_id)?;
            mgr.save(&path)?;
            println!("Cache cleared: {}", cache_id);
        }
        CacheAction::EvictExpired => {
            mgr.evict_expired();
            mgr.save(&path)?;
            println!("Expired entries evicted.");
        }
        CacheAction::Size { cache_id } => {
            let size = mgr.cache_size(&cache_id)?;
            println!("{} entries in {}", size, cache_id);
        }
        CacheAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Cache Stats".bold().cyan());
                println!("  Caches:    {}", stats.total_caches);
                println!("  Entries:   {}", stats.total_entries);
                println!("  Hits:      {}", stats.total_hits);
                println!("  Misses:    {}", stats.total_misses);
                println!("  Hit Rate:  {:.1}%", stats.hit_rate);
                println!("  Evictions: {}", stats.total_evictions);
                println!("  Size:      {} bytes", stats.total_size_bytes);
            });
        }
    }
    Ok(())
}

fn cmd_config_val(action: ConfigValAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::config_validator::ConfigValidator;
    let dir = crate::config::default_data_dir();
    let path = dir.join("config_validator.json");
    let mut mgr = ConfigValidator::load_or_default(&path);

    match action {
        ConfigValAction::Validate { schema_id, config_json } => {
            let config: std::collections::HashMap<String, String> = serde_json::from_str(&config_json)?;
            let result = mgr.validate(&schema_id, &config)?;
            crate::output::json_or(&result, || {
                if result.valid {
                    println!("{}", "Configuration VALID".bold().green());
                } else {
                    println!("{}", "Configuration INVALID".bold().red());
                    for e in &result.errors {
                        println!("  [ERROR] {} — {}", e.field, e.message);
                    }
                }
                for w in &result.warnings {
                    println!("  [WARN]  {} — {}", w.field, w.message);
                }
            });
        }
        ConfigValAction::Schemas => {
            let schemas: Vec<_> = mgr.schemas.values().collect();
            crate::output::json_or(&schemas, || {
                if schemas.is_empty() {
                    println!("No schemas registered.");
                } else {
                    println!("{}", "Config Schemas".bold().cyan());
                    for s in &schemas {
                        println!("  {} v{} — {} ({} fields)", s.id, s.version, s.name, s.fields.len());
                    }
                }
            });
        }
        ConfigValAction::Pending => {
            let pending = mgr.pending_migrations();
            crate::output::json_or(&pending, || {
                if pending.is_empty() {
                    println!("No pending migrations.");
                } else {
                    println!("{}", "Pending Migrations".bold().yellow());
                    for m in &pending {
                        println!("  {} — v{} → v{}: {}", m.id, m.from_version, m.to_version, m.description);
                    }
                }
            });
        }
        ConfigValAction::Applied => {
            let applied = mgr.applied_migrations();
            crate::output::json_or(&applied, || {
                if applied.is_empty() {
                    println!("No applied migrations.");
                } else {
                    println!("{}", "Applied Migrations".bold().green());
                    for m in &applied {
                        println!("  {} — v{} → v{} [{}]", m.id, m.from_version, m.to_version,
                            m.applied_at.as_deref().unwrap_or("?"));
                    }
                }
            });
        }
        ConfigValAction::Backup => {
            match mgr.latest_backup() {
                Some(b) => {
                    crate::output::json_or(b, || {
                        println!("{}", "Latest Backup".bold().cyan());
                        println!("  Version: {}", b.version);
                        println!("  Date:    {}", b.created_at);
                        println!("  Fields:  {}", b.data.len());
                    });
                }
                None => println!("No backups."),
            }
        }
        ConfigValAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Config Validator Stats".bold().cyan());
                println!("  Schemas:     {}", stats.total_schemas);
                println!("  Validations: {}", stats.total_validations);
                println!("  Migrations:  {} ({} applied)", stats.total_migrations, stats.applied_migrations);
                println!("  Rolled Back: {}", stats.rolled_back);
                println!("  Failed:      {}", stats.failed_validations);
                println!("  Backups:     {}", stats.backups);
            });
        }
    }
    Ok(())
}

fn cmd_tasks(action: TaskQueueAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::task_queue::{TaskQueue, QueueTask, TaskPriority2, TaskStatus3, TaskType2};
    let dir = crate::config::default_data_dir();
    let path = dir.join("task_queue.json");
    let mut queue = TaskQueue::load_or_default(&path);

    match action {
        TaskQueueAction::Enqueue { id, task_type, priority, max_retries } => {
            let tt = match task_type.to_lowercase().as_str() {
                "tx_submit" | "tx" => TaskType2::TxSubmit,
                "balance_refresh" | "balance" => TaskType2::BalanceRefresh,
                "energy_check" | "energy" => TaskType2::EnergyCheck,
                "backup" => TaskType2::Backup,
                "sync" => TaskType2::Sync,
                "notification" | "notify" => TaskType2::Notification,
                other => TaskType2::Custom(other.to_string()),
            };
            let tp = match priority.to_lowercase().as_str() {
                "critical" => TaskPriority2::Critical,
                "high" => TaskPriority2::High,
                "low" => TaskPriority2::Low,
                _ => TaskPriority2::Normal,
            };
            let task = QueueTask {
                id: id.clone(),
                task_type: tt,
                priority: tp,
                status: TaskStatus3::Queued,
                payload: std::collections::HashMap::new(),
                result: None,
                error: None,
                retry_count: 0,
                max_retries,
                created_at: chrono::Utc::now().to_rfc3339(),
                started_at: None,
                completed_at: None,
                progress_pct: 0.0,
            };
            queue.enqueue(task)?;
            queue.save(&path)?;
            println!("Task enqueued: {}", id);
        }
        TaskQueueAction::Dequeue => {
            match queue.dequeue() {
                Some(task) => {
                    queue.save(&path)?;
                    println!("Dequeued: {} ({:?}, {:?})", task.id, task.task_type, task.priority);
                }
                None => println!("Queue empty."),
            }
        }
        TaskQueueAction::Complete { id, result } => {
            queue.complete_task(&id, &result)?;
            queue.save(&path)?;
            println!("Task completed: {}", id);
        }
        TaskQueueAction::Fail { id, error } => {
            queue.fail_task(&id, &error)?;
            queue.save(&path)?;
            let status = queue.get_task(&id).map(|t| format!("{:?}", t.status)).unwrap_or("dead_letter".into());
            println!("Task failed: {} [{}]", id, status);
        }
        TaskQueueAction::Cancel { id } => {
            queue.cancel_task(&id)?;
            queue.save(&path)?;
            println!("Task cancelled: {}", id);
        }
        TaskQueueAction::Progress { id, pct } => {
            queue.update_progress(&id, pct)?;
            queue.save(&path)?;
            println!("Progress: {} = {:.1}%", id, pct);
        }
        TaskQueueAction::Retry { id } => {
            queue.retry_dead_letter(&id)?;
            queue.save(&path)?;
            println!("Task requeued from dead letter: {}", id);
        }
        TaskQueueAction::Running => {
            let running = queue.running_tasks();
            crate::output::json_or(&running, || {
                if running.is_empty() {
                    println!("No running tasks.");
                } else {
                    println!("{}", "Running Tasks".bold().cyan());
                    for t in &running {
                        println!("  {} — {:?} {:.1}%", t.id, t.task_type, t.progress_pct);
                    }
                }
            });
        }
        TaskQueueAction::DeadLetter => {
            let dl = queue.dead_letter_tasks();
            crate::output::json_or(&dl, || {
                if dl.is_empty() {
                    println!("No dead letter tasks.");
                } else {
                    println!("{}", "Dead Letter Queue".bold().red());
                    for d in &dl {
                        println!("  {} — {} (retries: {})", d.task.id, d.reason, d.task.retry_count);
                    }
                }
            });
        }
        TaskQueueAction::Purge => {
            let count = queue.purge_completed();
            queue.save(&path)?;
            println!("Purged {} completed tasks.", count);
        }
        TaskQueueAction::Depth => {
            let depth = queue.queue_depth();
            println!("Queue depth: {} tasks", depth);
        }
        TaskQueueAction::Stats => {
            let stats = queue.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Task Queue Stats".bold().cyan());
                println!("  Total:      {}", stats.total_tasks);
                println!("  Queued:     {}", stats.queued);
                println!("  Running:    {}", stats.running);
                println!("  Completed:  {}", stats.completed);
                println!("  Failed:     {}", stats.failed);
                println!("  Dead:       {}", stats.dead_letter);
                println!("  Cancelled:  {}", stats.cancelled);
                println!("  Retries:    {}", stats.total_retries);
            });
        }
    }
    Ok(())
}

fn cmd_changelog(action: ChangelogAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::changelog::{Changelog, ChangeEntry, ChangeType, ChangeScope};
    let dir = crate::config::default_data_dir();
    let path = dir.join("changelog.json");
    let mut cl = Changelog::load_or_default(&path);

    match action {
        ChangelogAction::Add { id, change_type, scope, description, author, breaking } => {
            let ct = match change_type.to_lowercase().as_str() {
                "changed" => ChangeType::Changed,
                "fixed" => ChangeType::Fixed,
                "removed" => ChangeType::Removed,
                "security" => ChangeType::Security,
                "deprecated" => ChangeType::Deprecated,
                "performance" => ChangeType::Performance,
                _ => ChangeType::Added,
            };
            let sc = match scope.to_lowercase().as_str() {
                "transaction" | "tx" => ChangeScope::Transaction,
                "energy" => ChangeScope::Energy,
                "staking" => ChangeScope::Staking,
                "defi" => ChangeScope::DeFi,
                "security" => ChangeScope::Security,
                "ui" => ChangeScope::UI,
                "internal" => ChangeScope::Internal,
                _ => ChangeScope::Wallet,
            };
            let entry = ChangeEntry {
                id: id.clone(),
                change_type: ct,
                scope: sc,
                description,
                author,
                timestamp: chrono::Utc::now().to_rfc3339(),
                version: None,
                breaking,
            };
            cl.add_entry(entry)?;
            cl.save(&path)?;
            println!("Changelog entry added: {}", id);
        }
        ChangelogAction::Remove { id } => {
            cl.remove_entry(&id)?;
            cl.save(&path)?;
            println!("Entry removed: {}", id);
        }
        ChangelogAction::Tag { version, name, entries, notes } => {
            let ids: Vec<String> = entries.split(',').filter(|s| !s.is_empty()).map(|s| s.trim().to_string()).collect();
            cl.tag_version(&version, &name, ids, notes)?;
            cl.save(&path)?;
            println!("Version tagged: {}", version);
        }
        ChangelogAction::Unversioned => {
            let entries = cl.unversioned_entries();
            crate::output::json_or(&entries, || {
                if entries.is_empty() {
                    println!("No unversioned entries.");
                } else {
                    println!("{}", "Unversioned Entries".bold().cyan());
                    for e in &entries {
                        println!("  {} [{:?}] {}", e.id, e.change_type, e.description);
                    }
                }
            });
        }
        ChangelogAction::Breaking => {
            let entries = cl.breaking_changes();
            crate::output::json_or(&entries, || {
                if entries.is_empty() {
                    println!("No breaking changes.");
                } else {
                    println!("{}", "Breaking Changes".bold().red());
                    for e in &entries {
                        println!("  {} — {}", e.id, e.description);
                    }
                }
            });
        }
        ChangelogAction::Markdown => {
            println!("{}", cl.generate_markdown());
        }
        ChangelogAction::Search { query } => {
            let results = cl.search(&query);
            println!("{} results for '{}'", results.len(), query);
            for e in &results {
                println!("  {} [{:?}] {}", e.id, e.change_type, e.description);
            }
        }
        ChangelogAction::Stats => {
            let stats = cl.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Changelog Stats".bold().cyan());
                println!("  Entries:   {}", stats.total_entries);
                println!("  Versions:  {}", stats.total_versions);
                println!("  Breaking:  {}", stats.breaking_changes);
                if let Some(v) = &stats.latest_version {
                    println!("  Latest:    {}", v);
                }
            });
        }
    }
    Ok(())
}

fn cmd_flags(action: FlagAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::feature_flags::{FeatureFlagManager, FeatureFlag, FlagStatus2, FlagCategory};
    let dir = crate::config::default_data_dir();
    let path = dir.join("feature_flags.json");
    let mut mgr = FeatureFlagManager::load_or_default(&path);

    match action {
        FlagAction::Register { id, name, category, desc } => {
            let cat = match category.to_lowercase().as_str() {
                "core" => FlagCategory::Core,
                "beta" => FlagCategory::Beta,
                "deprecated" => FlagCategory::Deprecated,
                _ => FlagCategory::Experimental,
            };
            let flag = FeatureFlag {
                id: id.clone(),
                name,
                description: desc,
                status: FlagStatus2::Disabled,
                category: cat,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                kill_switch: false,
                metadata: std::collections::HashMap::new(),
            };
            mgr.register_flag(flag)?;
            mgr.save(&path)?;
            println!("Flag registered: {}", id);
        }
        FlagAction::Remove { id } => {
            mgr.remove_flag(&id)?;
            mgr.save(&path)?;
            println!("Flag removed: {}", id);
        }
        FlagAction::Enable { id } => {
            mgr.enable_flag(&id)?;
            mgr.save(&path)?;
            println!("Flag enabled: {}", id);
        }
        FlagAction::Disable { id } => {
            mgr.disable_flag(&id)?;
            mgr.save(&path)?;
            println!("Flag disabled: {}", id);
        }
        FlagAction::Rollout { id, pct } => {
            mgr.set_rollout(&id, pct)?;
            mgr.save(&path)?;
            println!("Flag {} set to {}% rollout", id, pct);
        }
        FlagAction::Kill { id } => {
            mgr.kill_switch(&id)?;
            mgr.save(&path)?;
            println!("{}", format!("Kill switch activated: {}", id).bold().red());
        }
        FlagAction::Check { flag_id, user_id } => {
            let enabled = mgr.is_enabled(&flag_id, &user_id)?;
            mgr.save(&path)?;
            if enabled {
                println!("{}", "Feature ENABLED for this user.".bold().green());
            } else {
                println!("Feature disabled for this user.");
            }
        }
        FlagAction::Override { flag_id, user_id, enabled } => {
            mgr.add_override(&flag_id, &user_id, enabled)?;
            mgr.save(&path)?;
            println!("Override set: {}={} for {}", flag_id, enabled, user_id);
        }
        FlagAction::Enabled => {
            let flags = mgr.enabled_flags();
            crate::output::json_or(&flags, || {
                if flags.is_empty() {
                    println!("No enabled flags.");
                } else {
                    println!("{}", "Enabled Flags".bold().green());
                    for f in &flags {
                        println!("  {} — {} [{:?}]", f.id, f.name, f.category);
                    }
                }
            });
        }
        FlagAction::Killed => {
            let flags = mgr.killed_flags();
            crate::output::json_or(&flags, || {
                if flags.is_empty() {
                    println!("No killed flags.");
                } else {
                    println!("{}", "Killed Flags".bold().red());
                    for f in &flags {
                        println!("  {} — {}", f.id, f.name);
                    }
                }
            });
        }
        FlagAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Feature Flag Stats".bold().cyan());
                println!("  Total:    {}", stats.total_flags);
                println!("  Enabled:  {}", stats.enabled);
                println!("  Disabled: {}", stats.disabled);
                println!("  Rollout:  {}", stats.rollout);
                println!("  Kills:    {}", stats.kill_switches);
                println!("  Overrides:{}", stats.overrides);
                println!("  Evals:    {}", stats.evaluations);
            });
        }
    }
    Ok(())
}

fn cmd_telemetry(action: TelemetryAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::telemetry::{TelemetryManager, TelemetryLevel};
    let dir = crate::config::default_data_dir();
    let path = dir.join("telemetry.json");
    let mut mgr = TelemetryManager::load_or_default(&path);

    match action {
        TelemetryAction::OptIn { level } => {
            let lv = match level.to_lowercase().as_str() {
                "detailed" => TelemetryLevel::Detailed,
                "full" => TelemetryLevel::Full,
                _ => TelemetryLevel::Basic,
            };
            let label = format!("{:?}", lv);
            mgr.opt_in(lv);
            mgr.save(&path)?;
            println!("Telemetry opted in ({})", label);
        }
        TelemetryAction::OptOut => {
            mgr.opt_out();
            mgr.save(&path)?;
            println!("Telemetry opted out.");
        }
        TelemetryAction::Status => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Telemetry Status".bold().cyan());
                println!("  Enabled: {}", stats.enabled);
                println!("  Level:   {:?}", stats.level);
                println!("  Events:  {}", stats.total_events);
                println!("  Commands:{}", stats.unique_commands);
                println!("  Sessions:{}", stats.session_count);
            });
        }
        TelemetryAction::TopCommands { count } => {
            let top = mgr.top_commands(count);
            crate::output::json_or(&top, || {
                if top.is_empty() {
                    println!("No commands recorded.");
                } else {
                    println!("{}", "Top Commands".bold().cyan());
                    for p in &top {
                        println!("  {} — {} uses, avg {:.0}ms", p.command, p.count, p.avg_duration_ms);
                    }
                }
            });
        }
        TelemetryAction::Events { count } => {
            let events = mgr.recent_events(count);
            crate::output::json_or(&events, || {
                if events.is_empty() {
                    println!("No events.");
                } else {
                    println!("{}", "Recent Events".bold().cyan());
                    for e in &events {
                        println!("  [{:?}] {} — {}", e.category, e.name, e.timestamp);
                    }
                }
            });
        }
        TelemetryAction::Flush => {
            let flushed = mgr.flush();
            mgr.save(&path)?;
            println!("Flushed {} events.", flushed.len());
        }
        TelemetryAction::Stats => {
            let stats = mgr.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Telemetry Stats".bold().cyan());
                println!("  Enabled:    {}", stats.enabled);
                println!("  Level:      {:?}", stats.level);
                println!("  Events:     {}", stats.total_events);
                println!("  Commands:   {}", stats.unique_commands);
                println!("  Sessions:   {}", stats.session_count);
                println!("  Anonymized: {}", stats.anonymized_events);
            });
        }
    }
    Ok(())
}

fn cmd_api(action: ApiAction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::wallet_api::{WalletApi, ApiEndpoint, HttpMethod, ApiVersion, AuthType};
    let dir = crate::config::default_data_dir();
    let path = dir.join("wallet_api.json");
    let mut api = WalletApi::load_or_default(&path);

    match action {
        ApiAction::Register { id, path: ep_path, method, version, desc } => {
            let m = match method.to_lowercase().as_str() {
                "post" => HttpMethod::Post,
                "put" => HttpMethod::Put,
                "delete" => HttpMethod::Delete,
                _ => HttpMethod::Get,
            };
            let v = match version.to_lowercase().as_str() {
                "v2" => ApiVersion::V2,
                _ => ApiVersion::V1,
            };
            let endpoint = ApiEndpoint {
                id: id.clone(),
                path: ep_path,
                method: m,
                version: v,
                description: desc,
                auth_required: AuthType::None,
                rate_limit_per_min: None,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            api.register_endpoint(endpoint)?;
            api.save(&path)?;
            println!("Endpoint registered: {}", id);
        }
        ApiAction::Remove { id } => {
            api.remove_endpoint(&id)?;
            api.save(&path)?;
            println!("Endpoint removed: {}", id);
        }
        ApiAction::CreateKey { key, name, permissions } => {
            let perms: Vec<String> = permissions.split(',').map(|s| s.trim().to_string()).collect();
            api.create_api_key(&key, &name, perms, None)?;
            api.save(&path)?;
            println!("API key created: {}", name);
        }
        ApiAction::RevokeKey { key } => {
            api.revoke_api_key(&key)?;
            api.save(&path)?;
            println!("API key revoked.");
        }
        ApiAction::Keys => {
            let keys = api.active_api_keys();
            crate::output::json_or(&keys, || {
                if keys.is_empty() {
                    println!("No active API keys.");
                } else {
                    println!("{}", "Active API Keys".bold().cyan());
                    for k in &keys {
                        println!("  {} — {} perms={:?} reqs={}", k.key, k.name, k.permissions, k.request_count);
                    }
                }
            });
        }
        ApiAction::Endpoints => {
            let eps: Vec<_> = api.endpoints.values().collect();
            crate::output::json_or(&eps, || {
                if eps.is_empty() {
                    println!("No endpoints.");
                } else {
                    println!("{}", "API Endpoints".bold().cyan());
                    for e in &eps {
                        println!("  {} {:?} {} ({:?})", e.id, e.method, e.path, e.version);
                    }
                }
            });
        }
        ApiAction::Search { query } => {
            let results = api.search_endpoints(&query);
            println!("{} endpoints matching '{}'", results.len(), query);
            for e in &results {
                println!("  {} {:?} {}", e.id, e.method, e.path);
            }
        }
        ApiAction::Stats => {
            let stats = api.stats();
            crate::output::json_or(&stats, || {
                println!("{}", "Wallet API Stats".bold().cyan());
                println!("  Endpoints: {}", stats.total_endpoints);
                println!("  Requests:  {}", stats.total_requests);
                println!("  Keys:      {} ({} active)", stats.total_api_keys, stats.active_keys);
                println!("  Avg Resp:  {:.0}ms", stats.avg_response_ms);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Harness ─────────────────────────────────

fn cmd_harness(action: HarnessAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = dir.join("test_harness.json");
    let mut harness = crate::test_harness::TestHarness::load_or_default(&path);

    match action {
        HarnessAction::AddFixture { id, name, fixture_type } => {
            let ft = match fixture_type.to_lowercase().as_str() {
                "transaction" | "tx" => crate::test_harness::FixtureType::Transaction,
                "block" => crate::test_harness::FixtureType::Block,
                "token" => crate::test_harness::FixtureType::Token,
                "nft" => crate::test_harness::FixtureType::NFT,
                "contract" => crate::test_harness::FixtureType::Contract,
                "account" => crate::test_harness::FixtureType::Account,
                other => crate::test_harness::FixtureType::Custom(other.to_string()),
            };
            let fixture = crate::test_harness::TestFixture {
                id: id.clone(),
                fixture_type: ft,
                name,
                data: std::collections::HashMap::new(),
                created_at: chrono::Utc::now().to_rfc3339(),
                tags: vec![],
            };
            harness.add_fixture(fixture)?;
            harness.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"ok","id":&id}), || {
                println!("Fixture '{}' added.", id);
            });
        }
        HarnessAction::RemoveFixture { id } => {
            harness.fixtures.remove(&id);
            harness.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"removed","id":&id}), || {
                println!("Fixture '{}' removed.", id);
            });
        }
        HarnessAction::CreateMock { id, name, value } => {
            let mock = crate::test_harness::MockBuilder {
                id: id.clone(),
                name,
                behavior: crate::test_harness::MockBehavior::ReturnValue(value),
                call_count: 0,
                max_calls: None,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            harness.create_mock(mock)?;
            harness.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"ok","id":&id}), || {
                println!("Mock '{}' created.", id);
            });
        }
        HarnessAction::InvokeMock { id } => {
            let result = harness.invoke_mock(&id)?;
            harness.save(&path)?;
            crate::output::json_or(&serde_json::json!({"id":&id,"result":&result}), || {
                println!("Mock '{}' returned: {}", id, result);
            });
        }
        HarnessAction::Fixtures => {
            let list: Vec<_> = harness.fixtures.values().collect();
            crate::output::json_or(&list, || {
                if list.is_empty() {
                    println!("No fixtures.");
                } else {
                    for f in &list {
                        println!("  {} — {} ({:?})", f.id, f.name, f.fixture_type);
                    }
                }
            });
        }
        HarnessAction::Mocks => {
            let list: Vec<_> = harness.mocks.values().collect();
            crate::output::json_or(&list, || {
                if list.is_empty() {
                    println!("No mocks.");
                } else {
                    for m in &list {
                        println!("  {} — {} (calls: {})", m.id, m.name, m.call_count);
                    }
                }
            });
        }
        HarnessAction::Stats => {
            let stats = harness.stats();
            crate::output::json_or(&stats, || {
                println!("Test Harness Stats:");
                println!("  Fixtures:   {}", stats.total_fixtures);
                println!("  Mocks:      {}", stats.total_mocks);
                println!("  Cases:      {}", stats.total_cases);
                println!("  Suites:     {}", stats.total_suites);
                println!("  Pass rate:  {:.1}%", stats.pass_rate * 100.0);
                println!("  Executions: {}", stats.total_executions);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Fuzz ────────────────────────────────────

fn cmd_fuzz(action: FuzzAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = dir.join("fuzzer.json");
    let mut fuzzer = crate::fuzzer::Fuzzer::load_or_default(&path);

    match action {
        FuzzAction::AddTarget { id, name, desc } => {
            let target = crate::fuzzer::FuzzTarget {
                id: id.clone(),
                name,
                description: desc,
                input_types: vec![],
                invariants: vec![],
                runs: 0,
                failures: 0,
                last_run: None,
            };
            fuzzer.add_target(target)?;
            fuzzer.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"ok","id":&id}), || {
                println!("Fuzz target '{}' added.", id);
            });
        }
        FuzzAction::RemoveTarget { id } => {
            fuzzer.targets.remove(&id);
            fuzzer.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"removed","id":&id}), || {
                println!("Fuzz target '{}' removed.", id);
            });
        }
        FuzzAction::Campaign { target_id, runs } => {
            let campaign = fuzzer.start_campaign(&target_id, runs)?;
            let cid = campaign.id.clone();
            fuzzer.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"ok","campaign_id":&cid,"runs":runs}), || {
                println!("Campaign '{}' started: {} runs against '{}'.", cid, runs, target_id);
            });
        }
        FuzzAction::Failures => {
            let fails = fuzzer.failing_runs();
            crate::output::json_or(&fails, || {
                if fails.is_empty() {
                    println!("No failing runs.");
                } else {
                    for r in &fails {
                        println!("  {} — target: {} ({:?})", r.id, r.target_id, r.result);
                    }
                }
            });
        }
        FuzzAction::Recent { count } => {
            let recent: Vec<_> = fuzzer.runs.iter().rev().take(count).collect();
            crate::output::json_or(&recent, || {
                if recent.is_empty() {
                    println!("No runs yet.");
                } else {
                    for r in &recent {
                        println!("  {} — {:?} ({}ms)", r.id, r.result, r.duration_ms);
                    }
                }
            });
        }
        FuzzAction::Stats => {
            let stats = fuzzer.stats();
            crate::output::json_or(&stats, || {
                println!("Fuzzer Stats:");
                println!("  Targets:    {}", stats.total_targets);
                println!("  Runs:       {}", stats.total_runs);
                println!("  Failures:   {}", stats.total_failures);
                println!("  Campaigns:  {}", stats.total_campaigns);
                println!("  Fail rate:  {:.1}%", stats.failure_rate * 100.0);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Regression ──────────────────────────────

fn cmd_regression(action: RegressionAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = dir.join("regression_tracker.json");
    let mut tracker = crate::regression_tracker::RegressionTracker::load_or_default(&path);

    match action {
        RegressionAction::Report { id, title, severity, module, desc } => {
            let sev = match severity.to_lowercase().as_str() {
                "critical" => crate::regression_tracker::IssueSeverity2::Critical,
                "high" => crate::regression_tracker::IssueSeverity2::High,
                "low" => crate::regression_tracker::IssueSeverity2::Low,
                _ => crate::regression_tracker::IssueSeverity2::Medium,
            };
            let now = chrono::Utc::now().to_rfc3339();
            let issue = crate::regression_tracker::KnownIssue {
                id: id.clone(),
                title,
                description: desc,
                status: crate::regression_tracker::IssueStatus2::Open,
                severity: sev,
                module,
                first_seen: now.clone(),
                last_seen: now,
                occurrences: 1,
                fix_version: None,
            };
            tracker.report_issue(issue)?;
            tracker.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"ok","id":&id}), || {
                println!("Issue '{}' reported.", id);
            });
        }
        RegressionAction::Close { id, version } => {
            tracker.close_issue(&id, version)?;
            tracker.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"closed","id":&id}), || {
                println!("Issue '{}' closed.", id);
            });
        }
        RegressionAction::Reopen { id } => {
            tracker.reopen_issue(&id)?;
            tracker.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"reopened","id":&id}), || {
                println!("Issue '{}' reopened.", id);
            });
        }
        RegressionAction::Open => {
            let open: Vec<_> = tracker.issues.values()
                .filter(|i| i.status == crate::regression_tracker::IssueStatus2::Open)
                .collect();
            crate::output::json_or(&open, || {
                if open.is_empty() {
                    println!("No open issues.");
                } else {
                    for i in &open {
                        println!("  {} — {} [{:?}] ({})", i.id, i.title, i.severity, i.module);
                    }
                }
            });
        }
        RegressionAction::Regressed => {
            let regressed: Vec<_> = tracker.issues.values()
                .filter(|i| i.status == crate::regression_tracker::IssueStatus2::Regressed)
                .collect();
            crate::output::json_or(&regressed, || {
                if regressed.is_empty() {
                    println!("No regressed issues.");
                } else {
                    for i in &regressed {
                        println!("  {} — {} [{:?}]", i.id, i.title, i.severity);
                    }
                }
            });
        }
        RegressionAction::Search { query } => {
            let results = tracker.search_issues(&query);
            crate::output::json_or(&results, || {
                if results.is_empty() {
                    println!("No issues matching '{}'.", query);
                } else {
                    for i in &results {
                        println!("  {} — {} [{:?}]", i.id, i.title, i.status);
                    }
                }
            });
        }
        RegressionAction::Stats => {
            let stats = tracker.stats();
            crate::output::json_or(&stats, || {
                println!("Regression Tracker Stats:");
                println!("  Total:     {}", stats.total_issues);
                println!("  Open:      {}", stats.open);
                println!("  Fixed:     {}", stats.fixed);
                println!("  Regressed: {}", stats.regressed);
                println!("  Baselines: {}", stats.total_baselines);
                println!("  Diffs:     {}", stats.total_diffs);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Coverage ────────────────────────────────

fn cmd_coverage(action: CoverageAction) -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::config::default_data_dir();
    let path = dir.join("coverage_report.json");
    let mut tracker = crate::coverage_report::CoverageTracker::load_or_default(&path);

    match action {
        CoverageAction::Add { id, module_name, total_lines, covered_lines, total_functions, covered_functions, total_branches, covered_branches } => {
            let module = crate::coverage_report::ModuleCoverage {
                id: id.clone(),
                module_name,
                total_lines,
                covered_lines,
                total_functions,
                covered_functions,
                total_branches,
                covered_branches,
                uncovered_paths: vec![],
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            tracker.add_module(module)?;
            tracker.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"ok","id":&id}), || {
                println!("Module '{}' added.", id);
            });
        }
        CoverageAction::Update { id, covered_lines, covered_functions, covered_branches } => {
            tracker.update_module(&id, covered_lines, covered_functions, covered_branches)?;
            tracker.save(&path)?;
            crate::output::json_or(&serde_json::json!({"status":"updated","id":&id}), || {
                println!("Module '{}' updated.", id);
            });
        }
        CoverageAction::Report { name } => {
            let report = tracker.generate_report(&name);
            tracker.save(&path)?;
            crate::output::json_or(&report, || {
                println!("Coverage Report '{}':", report.name);
                println!("  Total coverage: {:.1}%", report.total_coverage);
                println!("  Modules:        {}", report.modules.len());
                println!("  Generated:      {}", report.generated_at);
            });
        }
        CoverageAction::Below { threshold } => {
            let below = tracker.modules_below_threshold(threshold);
            crate::output::json_or(&below, || {
                if below.is_empty() {
                    println!("All modules above {:.0}% threshold.", threshold);
                } else {
                    println!("Modules below {:.0}%:", threshold);
                    for m in &below {
                        println!("  {} — {:.1}%", m.module_name, m.line_coverage_pct());
                    }
                }
            });
        }
        CoverageAction::Overall => {
            let overall = tracker.overall_coverage();
            crate::output::json_or(&serde_json::json!({"overall_coverage":overall}), || {
                println!("Overall coverage: {:.1}%", overall);
            });
        }
        CoverageAction::Stats => {
            let stats = tracker.stats();
            crate::output::json_or(&stats, || {
                println!("Coverage Stats:");
                println!("  Modules:       {}", stats.total_modules);
                println!("  Avg line:      {:.1}%", stats.avg_line_coverage);
                println!("  Avg function:  {:.1}%", stats.avg_function_coverage);
                println!("  Avg branch:    {:.1}%", stats.avg_branch_coverage);
                println!("  Fully covered: {}", stats.fully_covered);
                println!("  Reports:       {}", stats.reports_generated);
            });
        }
    }
    Ok(())
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_parse_status() {
        let cli = Cli::parse_from(["wallet", "status"]);
        assert!(matches!(cli.command, Commands::Status));
    }

    #[test]
    fn test_parse_account_create() {
        let cli = Cli::parse_from(["wallet", "account", "create", "alice"]);
        match cli.command {
            Commands::Account { action: AccountAction::Create { name } } => {
                assert_eq!(name, "alice");
            }
            _ => panic!("expected Account Create"),
        }
    }

    #[test]
    fn test_parse_send() {
        let cli = Cli::parse_from(["wallet", "send", "0xabcd", "1000"]);
        match cli.command {
            Commands::Send { to, amount, wait } => {
                assert_eq!(to, "0xabcd");
                assert_eq!(amount, 1000);
                assert!(!wait);
            }
            _ => panic!("expected Send"),
        }
    }

    #[test]
    fn test_parse_energy_scan() {
        let cli = Cli::parse_from(["wallet", "energy", "scan"]);
        assert!(matches!(
            cli.command,
            Commands::Energy { action: EnergyAction::Scan }
        ));
    }

    #[test]
    fn test_parse_stake_forecast() {
        let cli = Cli::parse_from(["wallet", "stake", "forecast", "1", "50000", "100"]);
        match cli.command {
            Commands::Stake {
                action: StakeAction::Forecast { pool_id, amount, epochs },
            } => {
                assert_eq!(pool_id, 1);
                assert_eq!(amount, 50000);
                assert_eq!(epochs, 100);
            }
            _ => panic!("expected Stake Forecast"),
        }
    }

    #[test]
    fn test_parse_custom_node() {
        let cli = Cli::parse_from(["wallet", "--node", "http://custom:8080", "status"]);
        assert_eq!(cli.node, "http://custom:8080");
    }

    #[test]
    fn test_parse_seed_generate() {
        let cli = Cli::parse_from(["wallet", "seed", "generate"]);
        assert!(matches!(
            cli.command,
            Commands::Seed { action: SeedAction::Generate }
        ));
    }

    #[test]
    fn test_parse_contacts_add() {
        let cli = Cli::parse_from([
            "wallet", "contacts", "add", "alice", "0xabc", "--note", "friend",
        ]);
        match cli.command {
            Commands::Contacts {
                action: ContactAction::Add { name, address, note },
            } => {
                assert_eq!(name, "alice");
                assert_eq!(address, "0xabc");
                assert_eq!(note.as_deref(), Some("friend"));
            }
            _ => panic!("expected Contacts Add"),
        }
    }

    #[test]
    fn test_parse_history_list() {
        let cli = Cli::parse_from(["wallet", "history", "list", "--limit", "5"]);
        match cli.command {
            Commands::History {
                action: HistoryAction::List { limit },
            } => {
                assert_eq!(limit, 5);
            }
            _ => panic!("expected History List"),
        }
    }

    #[test]
    fn test_parse_gas_transfer() {
        let cli = Cli::parse_from(["wallet", "gas", "transfer"]);
        assert!(matches!(
            cli.command,
            Commands::Gas { action: GasAction::Transfer }
        ));
    }

    #[test]
    fn test_parse_gas_create_with_size() {
        let cli = Cli::parse_from(["wallet", "gas", "create", "--size", "500"]);
        match cli.command {
            Commands::Gas {
                action: GasAction::Create { size },
            } => {
                assert_eq!(size, 500);
            }
            _ => panic!("expected Gas Create"),
        }
    }

    #[test]
    fn test_parse_config_show() {
        let cli = Cli::parse_from(["wallet", "config", "show"]);
        assert!(matches!(
            cli.command,
            Commands::Config { action: ConfigAction::Show }
        ));
    }

    #[test]
    fn test_parse_config_set() {
        let cli = Cli::parse_from(["wallet", "config", "set", "node_url", "http://mainnet:3000"]);
        match cli.command {
            Commands::Config {
                action: ConfigAction::Set { key, value },
            } => {
                assert_eq!(key, "node_url");
                assert_eq!(value, "http://mainnet:3000");
            }
            _ => panic!("expected Config Set"),
        }
    }

    #[test]
    fn test_parse_offline_sign() {
        let cli = Cli::parse_from([
            "wallet", "offline", "sign", "0xabcd", "5000", "3", "-f", "my_tx.json",
        ]);
        match cli.command {
            Commands::Offline {
                action: OfflineAction::Sign { to, amount, nonce, file },
            } => {
                assert_eq!(to, "0xabcd");
                assert_eq!(amount, 5000);
                assert_eq!(nonce, 3);
                assert_eq!(file.to_str().unwrap(), "my_tx.json");
            }
            _ => panic!("expected Offline Sign"),
        }
    }

    #[test]
    fn test_parse_offline_broadcast() {
        let cli = Cli::parse_from(["wallet", "offline", "broadcast", "signed_tx.json"]);
        match cli.command {
            Commands::Offline {
                action: OfflineAction::Broadcast { file },
            } => {
                assert_eq!(file.to_str().unwrap(), "signed_tx.json");
            }
            _ => panic!("expected Offline Broadcast"),
        }
    }

    #[test]
    fn test_parse_offline_inspect() {
        let cli = Cli::parse_from(["wallet", "offline", "inspect", "tx.json"]);
        assert!(matches!(
            cli.command,
            Commands::Offline { action: OfflineAction::Inspect { .. } }
        ));
    }

    #[test]
    fn test_parse_energy_auto_refresh() {
        let cli = Cli::parse_from([
            "wallet", "energy", "auto-refresh", "--threshold", "15.0", "--interval", "30", "--once",
        ]);
        match cli.command {
            Commands::Energy {
                action: EnergyAction::AutoRefresh { threshold, interval, max_energy, once },
            } => {
                assert!((threshold - 15.0).abs() < f64::EPSILON);
                assert_eq!(interval, 30);
                assert_eq!(max_energy, 10000); // default
                assert!(once);
            }
            _ => panic!("expected Energy AutoRefresh"),
        }
    }

    #[test]
    fn test_parse_history_export() {
        let cli = Cli::parse_from(["wallet", "history", "export", "history.csv"]);
        match cli.command {
            Commands::History {
                action: HistoryAction::Export { file },
            } => {
                assert_eq!(file.to_str().unwrap(), "history.csv");
            }
            _ => panic!("expected History Export"),
        }
    }

    #[test]
    fn test_parse_offline_sign_refresh() {
        let cli = Cli::parse_from([
            "wallet", "offline", "sign-refresh", "0xobj123", "500", "-f", "refresh.json",
        ]);
        match cli.command {
            Commands::Offline {
                action: OfflineAction::SignRefresh { id, energy, file },
            } => {
                assert_eq!(id, "0xobj123");
                assert_eq!(energy, 500);
                assert_eq!(file.to_str().unwrap(), "refresh.json");
            }
            _ => panic!("expected Offline SignRefresh"),
        }
    }

    #[test]
    fn test_expand_path_home() {
        let expanded = expand_path("~/test/file.json");
        assert!(!expanded.starts_with("~/") || std::env::var("HOME").is_err());
    }

    #[test]
    fn test_expand_path_absolute() {
        let expanded = expand_path("/tmp/keystore.json");
        assert_eq!(expanded, "/tmp/keystore.json");
    }

    #[test]
    fn test_parse_json_flag() {
        let cli = Cli::parse_from(["wallet", "--json", "status"]);
        assert!(cli.json);
    }

    #[test]
    fn test_parse_no_json_flag() {
        let cli = Cli::parse_from(["wallet", "status"]);
        assert!(!cli.json);
    }

    #[test]
    fn test_parse_send_with_wait() {
        let cli = Cli::parse_from(["wallet", "send", "0xabcd", "1000", "--wait"]);
        match cli.command {
            Commands::Send { to, amount, wait } => {
                assert_eq!(to, "0xabcd");
                assert_eq!(amount, 1000);
                assert!(wait);
            }
            _ => panic!("expected Send"),
        }
    }

    #[test]
    fn test_parse_dashboard() {
        let cli = Cli::parse_from(["wallet", "dashboard"]);
        assert!(matches!(cli.command, Commands::Dashboard));
    }

    #[test]
    fn test_parse_watch() {
        let addr = format!("0x{}", "ab".repeat(32));
        let cli = Cli::parse_from(["wallet", "watch", &addr]);
        match cli.command {
            Commands::Watch { address } => {
                assert_eq!(address, addr);
            }
            _ => panic!("expected Watch"),
        }
    }

    #[test]
    fn test_parse_interactive() {
        let cli = Cli::parse_from(["wallet", "interactive"]);
        assert!(matches!(cli.command, Commands::Interactive));
    }

    #[test]
    fn test_parse_completions() {
        let cli = Cli::parse_from(["wallet", "completions", "bash"]);
        match cli.command {
            Commands::Completions { shell } => {
                assert_eq!(shell, "bash");
            }
            _ => panic!("expected Completions"),
        }
    }

    #[test]
    fn test_parse_json_with_command() {
        let cli = Cli::parse_from(["wallet", "--json", "account", "list"]);
        assert!(cli.json);
        assert!(matches!(
            cli.command,
            Commands::Account { action: AccountAction::List }
        ));
    }

    #[test]
    fn test_parse_simulate_send() {
        let cli = Cli::parse_from(["wallet", "simulate", "send", "0xabc", "1000"]);
        match cli.command {
            Commands::Simulate { action: SimulateAction::Send { to, amount } } => {
                assert_eq!(to, "0xabc");
                assert_eq!(amount, 1000);
            }
            _ => panic!("expected Simulate Send"),
        }
    }

    #[test]
    fn test_parse_simulate_refresh() {
        let addr = format!("0x{}", "ab".repeat(32));
        let cli = Cli::parse_from(["wallet", "simulate", "refresh", &addr, "500"]);
        match cli.command {
            Commands::Simulate { action: SimulateAction::Refresh { id, energy } } => {
                assert_eq!(id, addr);
                assert_eq!(energy, 500);
            }
            _ => panic!("expected Simulate Refresh"),
        }
    }

    #[test]
    fn test_parse_spending_show() {
        let cli = Cli::parse_from(["wallet", "spending", "show"]);
        assert!(matches!(cli.command, Commands::Spending { action: SpendingAction::Show }));
    }

    #[test]
    fn test_parse_spending_set_tx_limit() {
        let cli = Cli::parse_from(["wallet", "spending", "set-tx-limit", "10000"]);
        match cli.command {
            Commands::Spending { action: SpendingAction::SetTxLimit { amount } } => {
                assert_eq!(amount, 10000);
            }
            _ => panic!("expected SetTxLimit"),
        }
    }

    #[test]
    fn test_parse_spending_set_mode() {
        let cli = Cli::parse_from(["wallet", "spending", "set-mode", "enforce"]);
        match cli.command {
            Commands::Spending { action: SpendingAction::SetMode { mode } } => {
                assert_eq!(mode, "enforce");
            }
            _ => panic!("expected SetMode"),
        }
    }

    #[test]
    fn test_parse_spending_allow() {
        let cli = Cli::parse_from(["wallet", "spending", "allow", "0xfriend"]);
        match cli.command {
            Commands::Spending { action: SpendingAction::Allow { address } } => {
                assert_eq!(address, "0xfriend");
            }
            _ => panic!("expected Allow"),
        }
    }

    #[test]
    fn test_parse_spending_block() {
        let cli = Cli::parse_from(["wallet", "spending", "block", "0xenemy"]);
        match cli.command {
            Commands::Spending { action: SpendingAction::Block { address } } => {
                assert_eq!(address, "0xenemy");
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn test_parse_multisig_create_group() {
        let cli = Cli::parse_from(["wallet", "multisig", "create-group", "treasury", "0xa,0xb,0xc", "2"]);
        match cli.command {
            Commands::Multisig { action: MultisigAction::CreateGroup { name, members, threshold } } => {
                assert_eq!(name, "treasury");
                assert!(members.contains("0xa"));
                assert_eq!(threshold, 2);
            }
            _ => panic!("expected CreateGroup"),
        }
    }

    #[test]
    fn test_parse_multisig_groups() {
        let cli = Cli::parse_from(["wallet", "multisig", "groups"]);
        assert!(matches!(cli.command, Commands::Multisig { action: MultisigAction::Groups }));
    }

    #[test]
    fn test_parse_multisig_propose() {
        let cli = Cli::parse_from(["wallet", "multisig", "propose", "treasury", "0xdest", "5000", "--memo", "monthly"]);
        match cli.command {
            Commands::Multisig { action: MultisigAction::Propose { group, to, amount, memo } } => {
                assert_eq!(group, "treasury");
                assert_eq!(to, "0xdest");
                assert_eq!(amount, 5000);
                assert_eq!(memo.as_deref(), Some("monthly"));
            }
            _ => panic!("expected Propose"),
        }
    }

    #[test]
    fn test_parse_multisig_approve() {
        let cli = Cli::parse_from(["wallet", "multisig", "approve", "prop_123"]);
        match cli.command {
            Commands::Multisig { action: MultisigAction::Approve { id } } => {
                assert_eq!(id, "prop_123");
            }
            _ => panic!("expected Approve"),
        }
    }

    #[test]
    fn test_parse_hooks_list() {
        let cli = Cli::parse_from(["wallet", "hooks", "list"]);
        assert!(matches!(cli.command, Commands::Hooks { action: HooksAction::List }));
    }

    #[test]
    fn test_parse_hooks_add_shell() {
        let cli = Cli::parse_from(["wallet", "hooks", "add-shell", "notify", "post_send", "echo done", "--blocking"]);
        match cli.command {
            Commands::Hooks { action: HooksAction::AddShell { name, event, command, blocking } } => {
                assert_eq!(name, "notify");
                assert_eq!(event, "post_send");
                assert_eq!(command, "echo done");
                assert!(blocking);
            }
            _ => panic!("expected AddShell"),
        }
    }

    #[test]
    fn test_parse_hooks_add_log() {
        let cli = Cli::parse_from(["wallet", "hooks", "add-log", "logger", "on_error", "/tmp/err.log"]);
        match cli.command {
            Commands::Hooks { action: HooksAction::AddLog { name, event, file, format } } => {
                assert_eq!(name, "logger");
                assert_eq!(event, "on_error");
                assert_eq!(file, "/tmp/err.log");
                assert!(format.is_none());
            }
            _ => panic!("expected AddLog"),
        }
    }

    #[test]
    fn test_parse_hooks_remove() {
        let cli = Cli::parse_from(["wallet", "hooks", "remove", "old_hook"]);
        match cli.command {
            Commands::Hooks { action: HooksAction::Remove { name } } => {
                assert_eq!(name, "old_hook");
            }
            _ => panic!("expected Remove"),
        }
    }

    #[test]
    fn test_parse_hooks_enable() {
        let cli = Cli::parse_from(["wallet", "hooks", "enable", "my_hook"]);
        match cli.command {
            Commands::Hooks { action: HooksAction::Enable { name } } => {
                assert_eq!(name, "my_hook");
            }
            _ => panic!("expected Enable"),
        }
    }

    #[test]
    fn test_parse_hooks_disable() {
        let cli = Cli::parse_from(["wallet", "hooks", "disable", "my_hook"]);
        match cli.command {
            Commands::Hooks { action: HooksAction::Disable { name } } => {
                assert_eq!(name, "my_hook");
            }
            _ => panic!("expected Disable"),
        }
    }

    // ── Labels tests ──

    #[test]
    fn test_parse_labels_add() {
        let cli = Cli::parse_from(["wallet", "labels", "add", "0xabc", "Binance", "--category", "exchange", "--tags", "cex,hot"]);
        match cli.command {
            Commands::Labels { action: LabelsAction::Add { address, name, category, tags, .. } } => {
                assert_eq!(address, "0xabc");
                assert_eq!(name, "Binance");
                assert_eq!(category, "exchange");
                assert!(tags.unwrap().contains("cex"));
            }
            _ => panic!("expected Labels Add"),
        }
    }

    #[test]
    fn test_parse_labels_list() {
        let cli = Cli::parse_from(["wallet", "labels", "list"]);
        assert!(matches!(cli.command, Commands::Labels { action: LabelsAction::List }));
    }

    #[test]
    fn test_parse_labels_search() {
        let cli = Cli::parse_from(["wallet", "labels", "search", "binance"]);
        match cli.command {
            Commands::Labels { action: LabelsAction::Search { query } } => {
                assert_eq!(query, "binance");
            }
            _ => panic!("expected Labels Search"),
        }
    }

    #[test]
    fn test_parse_labels_annotate() {
        let cli = Cli::parse_from(["wallet", "labels", "annotate", "0xhash", "--note", "salary", "--tags", "income"]);
        match cli.command {
            Commands::Labels { action: LabelsAction::Annotate { tx_hash, note, tags, .. } } => {
                assert_eq!(tx_hash, "0xhash");
                assert_eq!(note.as_deref(), Some("salary"));
                assert!(tags.unwrap().contains("income"));
            }
            _ => panic!("expected Labels Annotate"),
        }
    }

    // ── Fees tests ──

    #[test]
    fn test_parse_fees_stats() {
        let cli = Cli::parse_from(["wallet", "fees", "stats"]);
        assert!(matches!(cli.command, Commands::Fees { action: FeesAction::Stats }));
    }

    #[test]
    fn test_parse_fees_timing() {
        let cli = Cli::parse_from(["wallet", "fees", "timing"]);
        assert!(matches!(cli.command, Commands::Fees { action: FeesAction::Timing }));
    }

    #[test]
    fn test_parse_fees_record() {
        let cli = Cli::parse_from(["wallet", "fees", "record"]);
        assert!(matches!(cli.command, Commands::Fees { action: FeesAction::Record }));
    }

    #[test]
    fn test_parse_fees_alert() {
        let cli = Cli::parse_from(["wallet", "fees", "alert", "cheap", "50"]);
        match cli.command {
            Commands::Fees { action: FeesAction::Alert { name, target } } => {
                assert_eq!(name, "cheap");
                assert_eq!(target, 50);
            }
            _ => panic!("expected Fees Alert"),
        }
    }

    // ── Hardware tests ──

    #[test]
    fn test_parse_hardware_list() {
        let cli = Cli::parse_from(["wallet", "hardware", "list"]);
        assert!(matches!(cli.command, Commands::Hardware { action: HardwareAction::List }));
    }

    #[test]
    fn test_parse_hardware_add_simulated() {
        let cli = Cli::parse_from(["wallet", "hardware", "add-simulated", "my-ledger"]);
        match cli.command {
            Commands::Hardware { action: HardwareAction::AddSimulated { name } } => {
                assert_eq!(name, "my-ledger");
            }
            _ => panic!("expected AddSimulated"),
        }
    }

    #[test]
    fn test_parse_hardware_info() {
        let cli = Cli::parse_from(["wallet", "hardware", "info", "sim_abc"]);
        match cli.command {
            Commands::Hardware { action: HardwareAction::Info { id } } => {
                assert_eq!(id, "sim_abc");
            }
            _ => panic!("expected Info"),
        }
    }

    // ── dApp tests ──

    #[test]
    fn test_parse_dapp_sessions() {
        let cli = Cli::parse_from(["wallet", "dapp", "sessions"]);
        assert!(matches!(cli.command, Commands::Dapp { action: DappAction::Sessions }));
    }

    #[test]
    fn test_parse_dapp_connect() {
        let cli = Cli::parse_from(["wallet", "dapp", "connect", "https://swap.io", "Swap", "view_account,request_sign", "--hours", "48"]);
        match cli.command {
            Commands::Dapp { action: DappAction::Connect { origin, name, permissions, hours } } => {
                assert_eq!(origin, "https://swap.io");
                assert_eq!(name, "Swap");
                assert!(permissions.contains("view_account"));
                assert_eq!(hours, 48);
            }
            _ => panic!("expected Connect"),
        }
    }

    #[test]
    fn test_parse_dapp_revoke() {
        let cli = Cli::parse_from(["wallet", "dapp", "revoke", "sess_123"]);
        match cli.command {
            Commands::Dapp { action: DappAction::Revoke { id } } => {
                assert_eq!(id, "sess_123");
            }
            _ => panic!("expected Revoke"),
        }
    }

    #[test]
    fn test_parse_dapp_revoke_origin() {
        let cli = Cli::parse_from(["wallet", "dapp", "revoke-origin", "https://bad.io"]);
        match cli.command {
            Commands::Dapp { action: DappAction::RevokeOrigin { origin } } => {
                assert_eq!(origin, "https://bad.io");
            }
            _ => panic!("expected RevokeOrigin"),
        }
    }

    // ── Notification tests ──

    #[test]
    fn test_parse_notifications_unread() {
        let cli = Cli::parse_from(["wallet", "notifications", "unread"]);
        assert!(matches!(cli.command, Commands::Notifications { action: NotificationsAction::Unread }));
    }

    #[test]
    fn test_parse_notifications_recent() {
        let cli = Cli::parse_from(["wallet", "notifications", "recent", "--limit", "5"]);
        match cli.command {
            Commands::Notifications { action: NotificationsAction::Recent { limit } } => {
                assert_eq!(limit, 5);
            }
            _ => panic!("expected Recent"),
        }
    }

    #[test]
    fn test_parse_notifications_filter() {
        let cli = Cli::parse_from(["wallet", "notifications", "filter", "energy_decay"]);
        match cli.command {
            Commands::Notifications { action: NotificationsAction::Filter { category } } => {
                assert_eq!(category, "energy_decay");
            }
            _ => panic!("expected Filter"),
        }
    }

    #[test]
    fn test_parse_notifications_count() {
        let cli = Cli::parse_from(["wallet", "notifications", "count"]);
        assert!(matches!(cli.command, Commands::Notifications { action: NotificationsAction::Count }));
    }

    // ── Session Keys tests ──

    #[test]
    fn test_parse_session_keys_list() {
        let cli = Cli::parse_from(["wallet", "session-keys", "list"]);
        assert!(matches!(cli.command, Commands::SessionKeys { action: SessionKeysAction::List }));
    }

    #[test]
    fn test_parse_session_keys_create() {
        let cli = Cli::parse_from(["wallet", "session-keys", "create", "my-dapp", "--max-per-tx", "5000", "--hours", "48"]);
        match cli.command {
            Commands::SessionKeys { action: SessionKeysAction::Create { label, max_per_tx, hours, .. } } => {
                assert_eq!(label, "my-dapp");
                assert_eq!(max_per_tx, 5000);
                assert_eq!(hours, 48);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn test_parse_session_keys_revoke() {
        let cli = Cli::parse_from(["wallet", "session-keys", "revoke", "sk_123"]);
        match cli.command {
            Commands::SessionKeys { action: SessionKeysAction::Revoke { id } } => {
                assert_eq!(id, "sk_123");
            }
            _ => panic!("expected Revoke"),
        }
    }

    #[test]
    fn test_parse_session_keys_setup_recovery() {
        let cli = Cli::parse_from(["wallet", "session-keys", "setup-recovery", "3", "--delay-hours", "72"]);
        match cli.command {
            Commands::SessionKeys { action: SessionKeysAction::SetupRecovery { threshold, delay_hours } } => {
                assert_eq!(threshold, 3);
                assert_eq!(delay_hours, 72);
            }
            _ => panic!("expected SetupRecovery"),
        }
    }

    // ── Bridge tests ──

    #[test]
    fn test_parse_bridge_list() {
        let cli = Cli::parse_from(["wallet", "bridge", "list"]);
        assert!(matches!(cli.command, Commands::Bridge { action: BridgeAction::List }));
    }

    #[test]
    fn test_parse_bridge_find() {
        let cli = Cli::parse_from(["wallet", "bridge", "find", "evaporchain", "ethereum"]);
        match cli.command {
            Commands::Bridge { action: BridgeAction::Find { source, dest } } => {
                assert_eq!(source, "evaporchain");
                assert_eq!(dest, "ethereum");
            }
            _ => panic!("expected Find"),
        }
    }

    #[test]
    fn test_parse_bridge_pending() {
        let cli = Cli::parse_from(["wallet", "bridge", "pending"]);
        assert!(matches!(cli.command, Commands::Bridge { action: BridgeAction::Pending }));
    }

    #[test]
    fn test_parse_bridge_transfer() {
        let cli = Cli::parse_from(["wallet", "bridge", "transfer", "br_abc", "EVAP", "1000", "0xsender", "0xrecipient"]);
        match cli.command {
            Commands::Bridge { action: BridgeAction::Transfer { bridge_id, token, amount, sender, recipient } } => {
                assert_eq!(bridge_id, "br_abc");
                assert_eq!(token, "EVAP");
                assert_eq!(amount, 1000);
                assert_eq!(sender, "0xsender");
                assert_eq!(recipient, "0xrecipient");
            }
            _ => panic!("expected Transfer"),
        }
    }

    // ── Lang tests ──

    #[test]
    fn test_parse_lang_show() {
        let cli = Cli::parse_from(["wallet", "lang", "show"]);
        assert!(matches!(cli.command, Commands::Lang { action: LangAction::Show }));
    }

    #[test]
    fn test_parse_lang_set() {
        let cli = Cli::parse_from(["wallet", "lang", "set", "es"]);
        match cli.command {
            Commands::Lang { action: LangAction::Set { locale } } => {
                assert_eq!(locale, "es");
            }
            _ => panic!("expected Set"),
        }
    }

    #[test]
    fn test_parse_lang_list() {
        let cli = Cli::parse_from(["wallet", "lang", "list"]);
        assert!(matches!(cli.command, Commands::Lang { action: LangAction::List }));
    }

    #[test]
    fn test_parse_lang_test() {
        let cli = Cli::parse_from(["wallet", "lang", "test", "success"]);
        match cli.command {
            Commands::Lang { action: LangAction::Test { key } } => {
                assert_eq!(key, "success");
            }
            _ => panic!("expected Test"),
        }
    }

    // ── Template tests ──

    #[test]
    fn test_parse_templates_list() {
        let cli = Cli::parse_from(["wallet", "templates", "list"]);
        assert!(matches!(cli.command, Commands::Templates { action: TemplatesAction::List }));
    }

    #[test]
    fn test_parse_templates_create_transfer() {
        let cli = Cli::parse_from(["wallet", "templates", "create-transfer", "rent", "0xlandlord", "5000", "--frequency", "monthly"]);
        match cli.command {
            Commands::Templates { action: TemplatesAction::CreateTransfer { name, to, amount, frequency } } => {
                assert_eq!(name, "rent");
                assert_eq!(to, "0xlandlord");
                assert_eq!(amount, 5000);
                assert_eq!(frequency, "monthly");
            }
            _ => panic!("expected CreateTransfer"),
        }
    }

    #[test]
    fn test_parse_templates_due() {
        let cli = Cli::parse_from(["wallet", "templates", "due"]);
        assert!(matches!(cli.command, Commands::Templates { action: TemplatesAction::Due }));
    }

    #[test]
    fn test_parse_templates_execute() {
        let cli = Cli::parse_from(["wallet", "templates", "execute", "rent"]);
        match cli.command {
            Commands::Templates { action: TemplatesAction::Execute { name } } => {
                assert_eq!(name, "rent");
            }
            _ => panic!("expected Execute"),
        }
    }

    // ── Analytics tests ──

    #[test]
    fn test_parse_analytics_summary() {
        let cli = Cli::parse_from(["wallet", "analytics", "summary", "month"]);
        match cli.command {
            Commands::Analytics { action: AnalyticsAction::Summary { period } } => {
                assert_eq!(period, "month");
            }
            _ => panic!("expected Summary"),
        }
    }

    #[test]
    fn test_parse_analytics_breakdown() {
        let cli = Cli::parse_from(["wallet", "analytics", "breakdown"]);
        assert!(matches!(cli.command, Commands::Analytics { action: AnalyticsAction::Breakdown { .. } }));
    }

    #[test]
    fn test_parse_analytics_trend() {
        let cli = Cli::parse_from(["wallet", "analytics", "trend", "week"]);
        match cli.command {
            Commands::Analytics { action: AnalyticsAction::Trend { period } } => {
                assert_eq!(period, "week");
            }
            _ => panic!("expected Trend"),
        }
    }

    #[test]
    fn test_parse_analytics_record() {
        let cli = Cli::parse_from(["wallet", "analytics", "record", "transfer_out", "1000", "49000", "--reference", "tx_abc"]);
        match cli.command {
            Commands::Analytics { action: AnalyticsAction::Record { event, amount, balance, reference } } => {
                assert_eq!(event, "transfer_out");
                assert_eq!(amount, 1000);
                assert_eq!(balance, 49000);
                assert_eq!(reference, "tx_abc");
            }
            _ => panic!("expected Record"),
        }
    }

    // ── Reputation tests ──

    #[test]
    fn test_parse_reputation_check() {
        let cli = Cli::parse_from(["wallet", "reputation", "check", "0xsuspect"]);
        match cli.command {
            Commands::Reputation { action: ReputationAction::Check { address } } => {
                assert_eq!(address, "0xsuspect");
            }
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn test_parse_reputation_flag() {
        let cli = Cli::parse_from(["wallet", "reputation", "flag", "0xbad", "scam", "--note", "reported"]);
        match cli.command {
            Commands::Reputation { action: ReputationAction::Flag { address, flag, note } } => {
                assert_eq!(address, "0xbad");
                assert_eq!(flag, "scam");
                assert_eq!(note.as_deref(), Some("reported"));
            }
            _ => panic!("expected Flag"),
        }
    }

    #[test]
    fn test_parse_reputation_verify() {
        let cli = Cli::parse_from(["wallet", "reputation", "verify", "0xgood", "--label", "My Exchange"]);
        match cli.command {
            Commands::Reputation { action: ReputationAction::Verify { address, label } } => {
                assert_eq!(address, "0xgood");
                assert_eq!(label.as_deref(), Some("My Exchange"));
            }
            _ => panic!("expected Verify"),
        }
    }

    #[test]
    fn test_parse_reputation_dangerous() {
        let cli = Cli::parse_from(["wallet", "reputation", "dangerous"]);
        assert!(matches!(cli.command, Commands::Reputation { action: ReputationAction::Dangerous }));
    }

    // ── Watchtower tests ──

    #[test]
    fn test_parse_watchtower_list() {
        let cli = Cli::parse_from(["wallet", "watchtower", "list"]);
        assert!(matches!(cli.command, Commands::Watchtower { action: WatchtowerAction::List }));
    }

    #[test]
    fn test_parse_watchtower_watch_balance() {
        let cli = Cli::parse_from(["wallet", "watchtower", "watch-balance", "low-bal", "0xaddr", "1000", "--interval", "30"]);
        match cli.command {
            Commands::Watchtower { action: WatchtowerAction::WatchBalance { name, address, threshold, interval } } => {
                assert_eq!(name, "low-bal");
                assert_eq!(address, "0xaddr");
                assert_eq!(threshold, 1000.0);
                assert_eq!(interval, 30);
            }
            _ => panic!("expected WatchBalance"),
        }
    }

    #[test]
    fn test_parse_watchtower_watch_energy() {
        let cli = Cli::parse_from(["wallet", "watchtower", "watch-energy", "nft-watch", "obj_42", "10", "--auto-refresh", "5000"]);
        match cli.command {
            Commands::Watchtower { action: WatchtowerAction::WatchEnergy { name, object_id, threshold, auto_refresh, .. } } => {
                assert_eq!(name, "nft-watch");
                assert_eq!(object_id, "obj_42");
                assert_eq!(threshold, 10.0);
                assert_eq!(auto_refresh, 5000);
            }
            _ => panic!("expected WatchEnergy"),
        }
    }

    #[test]
    fn test_parse_watchtower_status() {
        let cli = Cli::parse_from(["wallet", "watchtower", "status"]);
        assert!(matches!(cli.command, Commands::Watchtower { action: WatchtowerAction::Status }));
    }

    #[test]
    fn test_parse_watchtower_alerts() {
        let cli = Cli::parse_from(["wallet", "watchtower", "alerts", "--limit", "5"]);
        match cli.command {
            Commands::Watchtower { action: WatchtowerAction::Alerts { limit } } => {
                assert_eq!(limit, 5);
            }
            _ => panic!("expected Alerts"),
        }
    }

    // ── Audit tests ──

    #[test]
    fn test_parse_audit_recent() {
        let cli = Cli::parse_from(["wallet", "audit", "recent", "--limit", "10"]);
        match cli.command {
            Commands::Audit { action: AuditAction2::Recent { limit } } => assert_eq!(limit, 10),
            _ => panic!("expected Recent"),
        }
    }

    #[test]
    fn test_parse_audit_verify() {
        let cli = Cli::parse_from(["wallet", "audit", "verify"]);
        assert!(matches!(cli.command, Commands::Audit { action: AuditAction2::Verify }));
    }

    #[test]
    fn test_parse_audit_search() {
        let cli = Cli::parse_from(["wallet", "audit", "search", "transfer"]);
        match cli.command {
            Commands::Audit { action: AuditAction2::Search { query } } => assert_eq!(query, "transfer"),
            _ => panic!("expected Search"),
        }
    }

    // ── Tax tests ──

    #[test]
    fn test_parse_tax_acquire() {
        let cli = Cli::parse_from(["wallet", "tax", "acquire", "1000", "1.5", "--source", "purchase"]);
        match cli.command {
            Commands::Tax { action: TaxAction::Acquire { amount, cost, source, .. } } => {
                assert_eq!(amount, 1000);
                assert_eq!(cost, 1.5);
                assert_eq!(source, "purchase");
            }
            _ => panic!("expected Acquire"),
        }
    }

    #[test]
    fn test_parse_tax_dispose() {
        let cli = Cli::parse_from(["wallet", "tax", "dispose", "500", "3.0"]);
        match cli.command {
            Commands::Tax { action: TaxAction::Dispose { amount, proceeds, .. } } => {
                assert_eq!(amount, 500);
                assert_eq!(proceeds, 3.0);
            }
            _ => panic!("expected Dispose"),
        }
    }

    #[test]
    fn test_parse_tax_summary() {
        let cli = Cli::parse_from(["wallet", "tax", "summary", "2025"]);
        match cli.command {
            Commands::Tax { action: TaxAction::Summary { year } } => assert_eq!(year, 2025),
            _ => panic!("expected Summary"),
        }
    }

    #[test]
    fn test_parse_tax_lots() {
        let cli = Cli::parse_from(["wallet", "tax", "lots"]);
        assert!(matches!(cli.command, Commands::Tax { action: TaxAction::Lots }));
    }

    // ── Policy tests ──

    #[test]
    fn test_parse_policy_list() {
        let cli = Cli::parse_from(["wallet", "policy", "list"]);
        assert!(matches!(cli.command, Commands::Policy { action: PolicyAction::List }));
    }

    #[test]
    fn test_parse_policy_add_max() {
        let cli = Cli::parse_from(["wallet", "policy", "add-max-amount", "big-tx", "10000", "--enforcement", "warn"]);
        match cli.command {
            Commands::Policy { action: PolicyAction::AddMaxAmount { name, max, enforcement } } => {
                assert_eq!(name, "big-tx");
                assert_eq!(max, 10000);
                assert_eq!(enforcement, "warn");
            }
            _ => panic!("expected AddMaxAmount"),
        }
    }

    #[test]
    fn test_parse_policy_test() {
        let cli = Cli::parse_from(["wallet", "policy", "test", "0xrecipient", "5000"]);
        match cli.command {
            Commands::Policy { action: PolicyAction::Test { to, amount } } => {
                assert_eq!(to, "0xrecipient");
                assert_eq!(amount, 5000);
            }
            _ => panic!("expected Test"),
        }
    }

    #[test]
    fn test_parse_policy_add_timelock() {
        let cli = Cli::parse_from(["wallet", "policy", "add-timelock", "nighttime", "22", "6"]);
        match cli.command {
            Commands::Policy { action: PolicyAction::AddTimelock { name, deny_after, deny_before, .. } } => {
                assert_eq!(name, "nighttime");
                assert_eq!(deny_after, 22);
                assert_eq!(deny_before, 6);
            }
            _ => panic!("expected AddTimelock"),
        }
    }

    // ── Export tests ──

    #[test]
    fn test_parse_export_history() {
        let cli = Cli::parse_from(["wallet", "export", "history", "output.csv"]);
        match cli.command {
            Commands::Export { action: ExportAction::History { file } } => {
                assert_eq!(file.to_string_lossy(), "output.csv");
            }
            _ => panic!("expected History"),
        }
    }

    #[test]
    fn test_parse_export_dump() {
        let cli = Cli::parse_from(["wallet", "export", "dump", "state.json"]);
        match cli.command {
            Commands::Export { action: ExportAction::Dump { file } } => {
                assert_eq!(file.to_string_lossy(), "state.json");
            }
            _ => panic!("expected Dump"),
        }
    }

    #[test]
    fn test_parse_export_summary() {
        let cli = Cli::parse_from(["wallet", "export", "summary", "account.txt"]);
        match cli.command {
            Commands::Export { action: ExportAction::Summary { file } } => {
                assert_eq!(file.to_string_lossy(), "account.txt");
            }
            _ => panic!("expected Summary"),
        }
    }
}
