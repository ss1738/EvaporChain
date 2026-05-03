//! Smart contract system for EvaporChain testnet.
//!
//! Two layers:
//!   1. **Contract Templates** — 8 pre-built templates with thermodynamic
//!      decay: DecayingToken, MortalNFT, ThermodynamicEscrow,
//!      DecayingAuction, StakingPool, DAOVote, DecayingDAO,
//!      TemporalContract. (DecayingDAO + TemporalContract were added
//!      after the original "6 templates" comment was written.)
//!   2. **Rule Engine** — simple condition→action rules for custom behavior
//!
//! Every contract instance is itself a decaying state object: it has energy and
//! a half-life.  If nobody refreshes the contract, it evaporates.

use evaporchain_types::{energy_at_epoch, AccountAddress, Epoch};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

mod decaying_dao;
pub use decaying_dao::{DaoProposal, DaoProposalStatus, DecayingDaoState};

const MAX_CONTRACT_STATE_BYTES: usize = 1_048_576; // 1 MB per contract

// ═══════════════════════════════════════════════════════════════════════════
// Errors
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("contract not found: {0}")]
    NotFound(u64),
    #[error("contract evaporated: {0}")]
    Evaporated(u64),
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("insufficient funds: have {available}, need {required}")]
    InsufficientFunds { available: u64, required: u64 },
    #[error("rejected by rule: {0}")]
    RejectedByRule(String),
    #[error("state error: {0}")]
    StateError(String),
    #[error("deploy failed: {0}")]
    DeployFailed(String),
    #[error("contract storage quota exceeded: {size} bytes > {max} bytes")]
    StorageQuotaExceeded { size: usize, max: usize },
}

// ═══════════════════════════════════════════════════════════════════════════
// LAYER 1: Contract Templates
// ═══════════════════════════════════════════════════════════════════════════

/// The 8 pre-built contract templates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContractTemplate {
    DecayingToken,
    MortalNFT,
    ThermodynamicEscrow,
    DecayingAuction,
    StakingPool,
    DAOVote,
    /// Decay-native parameter governance: per-key bounds + vote-weight cap
    /// (`min(balance, stake)`) + quorum + timelock. Closes the governance
    /// unbounded-params + whale-pass + no-quorum gap.
    DecayingDAO,
    /// Temporal contract: evolves through time-based phases with energy-gated
    /// state transitions, scheduled callbacks, and thermodynamic governance.
    TemporalContract,
}

impl ContractTemplate {
    pub fn name(&self) -> &'static str {
        match self {
            Self::DecayingToken => "DecayingToken",
            Self::MortalNFT => "MortalNFT",
            Self::ThermodynamicEscrow => "ThermodynamicEscrow",
            Self::DecayingAuction => "DecayingAuction",
            Self::StakingPool => "StakingPool",
            Self::DAOVote => "DAOVote",
            Self::DecayingDAO => "DecayingDAO",
            Self::TemporalContract => "TemporalContract",
        }
    }
}

// ─────────────── Internal State Types ──────────────────────────────────

/// DecayingToken state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenState {
    pub name: String,
    pub symbol: String,
    pub balances: HashMap<String, u64>,
    pub decay_half_life: u64,
    pub owner: String,
    pub total_minted: u64,
    pub total_decayed: u64,
    pub last_tick_epoch: u64,
}

/// MortalNFT state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NftState {
    pub collection_name: String,
    pub max_supply: u64,
    pub tokens: HashMap<u64, NftInfo>,
    pub next_token_id: u64,
    pub last_tick_epoch: u64,
    /// Subscription-renewal economics: when > 0, calling `refresh` to
    /// extend an NFT's lifetime requires paying this amount of contract-
    /// internal balance to `renewal_recipient`. Backward-compatible —
    /// pre-existing collections deserialize with renewal_fee=0 and
    /// behave like the original gratis-refresh MortalNFT.
    #[serde(default)]
    pub renewal_fee: u64,
    /// Recipient of the renewal_fee. When empty, the contract creator
    /// is used (resolved at refresh time, not stored here).
    #[serde(default)]
    pub renewal_recipient: String,
    /// Per-account renewal-fee balance. Holders deposit funds here
    /// (e.g. via a top-level "deposit_renewal_fund" method, or by an
    /// external contract crediting them) and refresh debits from this.
    /// Keeps the renewal economy native to the NFT contract instead
    /// of requiring a side-channel to a token contract.
    #[serde(default)]
    pub renewal_balances: HashMap<String, u64>,
    /// EVR-721 Phase 2.2 (2026-05-03): grace period in epochs between
    /// energy=0 (Active→Grace) and full evaporation (Grace→Ghost).
    /// Spec default: 5 epochs (matches testnet). 0 collapses Grace
    /// to a no-op — tokens go straight from Active to Ghost on
    /// energy=0 — useful for tests that exercise the full state
    /// transition compactly.
    #[serde(default = "default_grace_period")]
    pub grace_period: u64,
    /// EVR-721 §"Ghost Record": tokens that have transitioned to
    /// Ghost are removed from `tokens` and recorded here so
    /// `resurrect()` can reconstruct an Active token without keeping
    /// every dead token in the active map. Resurrection (refresh on
    /// a Ghost) consumes the ghost record.
    #[serde(default)]
    pub ghost_records: HashMap<u64, GhostRecord>,
}

fn default_grace_period() -> u64 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NftInfo {
    pub owner: String,
    pub metadata_hash: String,
    pub energy: u64,
    pub half_life: u64,
    pub minted_epoch: u64,
    /// EVR-721 Phase 2.2 (2026-05-03): explicit lifecycle field. Old
    /// serialized state defaults to `Active` (the implicit prior
    /// behaviour). Templates still derive the live state from
    /// `current_energy + grace_period + grace_epoch` so the field is a
    /// cache; canonical transitions happen in `tick_nft`.
    #[serde(default)]
    pub state: NftLifecycleState,
    /// Set by `tick_nft` when the token first transitions to Grace.
    /// `None` while in Active. Used to compute the Grace→Ghost cutoff.
    #[serde(default)]
    pub grace_epoch: Option<u64>,
    /// Set by `tick_nft` when the token transitions to Ghost. The
    /// canonical evaporation epoch — also stored on the matching
    /// `GhostRecord` for proof generation.
    #[serde(default)]
    pub evaporated_epoch: Option<u64>,
    /// `Blake3(token_id : metadata_hash : evaporated_epoch)`. Cleared
    /// to `None` on resurrection (Ghost → Active via refresh).
    #[serde(default)]
    pub ghost_proof: Option<String>,
}

/// EVR-721 lifecycle state. Stored as a JSON-tagged enum so the spec's
/// state names appear directly in serialized output.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum NftLifecycleState {
    #[default]
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "grace")]
    Grace,
    #[serde(rename = "ghost")]
    Ghost,
}

/// Per-token record retained when a token evaporates. Allows the chain
/// to prove the token existed without keeping the full `NftInfo`
/// (e.g. metadata payload may be elsewhere by then). EVR-721 §"Ghost
/// Record".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GhostRecord {
    pub token_id: u64,
    pub owner: String,
    pub metadata_hash: String,
    pub evaporated_epoch: u64,
    /// `Blake3(token_id : metadata_hash : evaporated_epoch)`.
    pub ghost_proof: String,
}

/// ThermodynamicEscrow state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowState {
    pub sender: String,
    pub receiver: String,
    pub escrowed_amount: u64,
    pub release_epoch: u64,
    pub decay_after_epochs: u64,
    pub claimed: bool,
    pub refunded: bool,
    pub decayed: bool,
}

/// DecayingAuction state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuctionState {
    pub seller: String,
    pub item_description: String,
    pub min_bid: u64,
    pub duration_epochs: u64,
    pub reserve_price: u64,
    pub start_epoch: u64,
    pub bids: Vec<(String, u64)>,
    pub finalized: bool,
    pub winner: Option<String>,
}

/// StakingPool state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakingState {
    pub pool_name: String,
    pub reward_rate_per_epoch: u64,
    pub reward_decay_half_life: u64,
    pub stakes: HashMap<String, StakeInfo>,
    pub total_staked: u64,
    pub reward_pool: u64,
    pub last_tick_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeInfo {
    pub amount: u64,
    pub staked_epoch: u64,
    pub unclaimed_rewards: u64,
    pub last_claim_epoch: u64,
}

/// Temporal Contract state — a contract that evolves through time-based phases.
/// Each phase has its own behavior, duration, and energy requirements.
/// State transitions happen automatically on tick() when phase conditions are met.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalState {
    /// Contract name.
    pub name: String,
    /// Creator/owner address (hex string).
    pub owner: String,
    /// Ordered list of phases the contract transitions through.
    pub phases: Vec<Phase>,
    /// Index of the current active phase (0-based).
    pub current_phase: usize,
    /// Epoch when the current phase started.
    pub phase_start_epoch: u64,
    /// Whether the contract has completed all phases.
    pub completed: bool,
    /// Key-value store for phase-specific data.
    pub data: HashMap<String, serde_json::Value>,
    /// Scheduled callbacks: (trigger_epoch, callback_name, args).
    pub callbacks: Vec<ScheduledCallback>,
    /// History of phase transitions: (from_phase, to_phase, epoch, reason).
    pub transition_log: Vec<TransitionRecord>,
    /// Energy threshold below which the contract enters "low energy" behavior.
    pub low_energy_threshold: u64,
    /// Whether the contract is in low-energy mode.
    pub low_energy_mode: bool,
    /// Last tick epoch.
    pub last_tick_epoch: u64,
}

/// A phase in a temporal contract's lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    /// Phase name (e.g., "fundraising", "active", "cooldown", "settlement").
    pub name: String,
    /// Duration in epochs (0 = infinite / manual transition only).
    pub duration_epochs: u64,
    /// Minimum energy required to enter this phase.
    pub min_energy: u64,
    /// Whether this phase auto-advances on duration expiry.
    pub auto_advance: bool,
    /// Methods allowed during this phase (empty = all allowed).
    pub allowed_methods: Vec<String>,
    /// Energy cost per epoch while in this phase.
    pub energy_cost_per_epoch: u64,
}

/// A scheduled callback that fires at a specific epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledCallback {
    /// Epoch when this callback should fire.
    pub trigger_epoch: u64,
    /// Name/type of the callback.
    pub callback_name: String,
    /// Arguments for the callback (JSON).
    pub args: serde_json::Value,
    /// Whether this callback has already fired.
    pub fired: bool,
}

/// Record of a phase transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionRecord {
    pub from_phase: String,
    pub to_phase: String,
    pub epoch: u64,
    pub reason: String,
}

/// DAOVote state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaoState {
    pub title: String,
    pub description: String,
    pub options: Vec<String>,
    pub voting_period_epochs: u64,
    pub quorum_pct: u64,
    pub start_epoch: u64,
    pub votes: HashMap<String, (usize, u64)>,
    pub finalized: bool,
    pub result: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════
// LAYER 2: Rule Engine
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuleTrigger {
    OnTransfer,
    OnMint,
    OnBurn,
    OnRefresh,
    OnTick,
    OnDeploy,
    OnCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ComparisonOp {
    Gt,
    Lt,
    Eq,
    Gte,
    Lte,
    Neq,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuleCondition {
    Always,
    If {
        field: String,
        op: ComparisonOp,
        value: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuleAction {
    CostEnergy(u64),
    Reject,
    EmitEvent(String),
    BurnAmount(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub trigger: RuleTrigger,
    pub condition: RuleCondition,
    pub action: RuleAction,
}

// ═══════════════════════════════════════════════════════════════════════════
// Contract Instance & Engine
// ═══════════════════════════════════════════════════════════════════════════

/// A deployed contract instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractInstance {
    pub id: u64,
    pub template: ContractTemplate,
    pub state: serde_json::Value,
    pub rules: Vec<Rule>,
    pub creator: AccountAddress,
    pub created_epoch: u64,
    pub energy: u64,
    pub half_life: u64,
    pub last_refreshed: u64,
    pub evaporated: bool,
    /// Bytes credited to the creator's `storage_bytes` at deploy time.
    /// The execution-layer evaporation credit-back reads this to debit the
    /// exact amount (replaces an earlier JSON-serialize approximation).
    /// `#[serde(default)]` keeps legacy contracts deserializable; their
    /// field defaults to 0 → credit-back is a no-op (saturating_sub).
    #[serde(default)]
    pub storage_bytes_charged: u64,
}

impl ContractInstance {
    /// Current energy at the given epoch.
    pub fn energy_at(&self, epoch: Epoch) -> u64 {
        if self.evaporated {
            return 0;
        }
        let elapsed = epoch.saturating_sub(self.last_refreshed);
        energy_at_epoch(self.energy, self.half_life, elapsed)
    }
}

/// Result of calling a contract method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallResult {
    pub success: bool,
    pub return_value: serde_json::Value,
    pub events: Vec<String>,
    pub energy_cost: u64,
    pub rules_triggered: Vec<String>,
}

/// Result of ticking all contracts for one epoch.
#[derive(Debug, Clone, Default)]
pub struct TickResult {
    pub contracts_ticked: usize,
    pub contracts_evaporated: Vec<u64>,
    pub events: Vec<String>,
}

/// The contract engine managing all deployed contracts.
#[derive(Debug, Clone)]
pub struct ContractEngine {
    contracts: HashMap<u64, ContractInstance>,
    next_id: u64,
}

impl ContractEngine {
    pub fn new() -> Self {
        Self {
            contracts: HashMap::new(),
            next_id: 1,
        }
    }

    /// Return references to all deployed contract instances.
    pub fn all_contracts(&self) -> Vec<&ContractInstance> {
        self.contracts.values().collect()
    }

    /// Re-insert a previously persisted contract, adjusting next_id to avoid collisions.
    pub fn restore_contract(&mut self, instance: ContractInstance) {
        if instance.id >= self.next_id {
            self.next_id = instance.id + 1;
        }
        self.contracts.insert(instance.id, instance);
    }

    /// Deploy a new contract. Returns the contract ID.
    #[allow(clippy::too_many_arguments)]
    pub fn deploy(
        &mut self,
        template: ContractTemplate,
        params: serde_json::Value,
        rules: Vec<Rule>,
        creator: AccountAddress,
        energy: u64,
        half_life: u64,
        current_epoch: Epoch,
    ) -> Result<u64, ContractError> {
        // Compute the storage charge as the JSON length of the parsed
        // params. Close to (but not always exactly) `tx.init_args.len()`
        // — whitespace and key-ordering differences may differ by a few
        // bytes. Saturating arithmetic in the credit-back path absorbs
        // any drift.
        let storage_bytes_charged = serde_json::to_string(&params)
            .map(|s| s.len() as u64)
            .unwrap_or(0);

        let state = self.initialize_state(&template, &params, current_epoch)?;
        let id = self.next_id;
        self.next_id += 1;

        let instance = ContractInstance {
            id,
            template,
            state,
            rules,
            creator,
            created_epoch: current_epoch,
            energy,
            half_life,
            last_refreshed: current_epoch,
            evaporated: false,
            storage_bytes_charged,
        };

        self.contracts.insert(id, instance);
        Ok(id)
    }

    /// Call a method on a contract.
    pub fn call(
        &mut self,
        contract_id: u64,
        method: &str,
        args: &serde_json::Value,
        caller: &AccountAddress,
        current_epoch: Epoch,
    ) -> Result<CallResult, ContractError> {
        // Check contract exists and is alive.
        let contract = self
            .contracts
            .get(&contract_id)
            .ok_or(ContractError::NotFound(contract_id))?;
        if contract.evaporated {
            return Err(ContractError::Evaporated(contract_id));
        }
        if contract.energy_at(current_epoch) == 0 {
            return Err(ContractError::Evaporated(contract_id));
        }

        // Determine trigger for rule evaluation.
        let trigger = method_to_trigger(method);
        let mut events = Vec::new();
        let mut rules_triggered = Vec::new();
        let mut energy_cost = 0u64;

        // Evaluate rules BEFORE execution.
        let rules = contract.rules.clone();
        for rule in &rules {
            if rule.trigger != trigger && rule.trigger != RuleTrigger::OnCall {
                continue;
            }
            if evaluate_condition(&rule.condition, args, &contract.state) {
                match &rule.action {
                    RuleAction::Reject => {
                        return Err(ContractError::RejectedByRule(format!(
                            "Rule rejected call to {method}"
                        )));
                    }
                    RuleAction::CostEnergy(cost) => {
                        energy_cost += cost;
                        rules_triggered.push(format!("CostEnergy({cost}) on {method}"));
                    }
                    RuleAction::EmitEvent(msg) => {
                        events.push(msg.clone());
                        rules_triggered.push(format!("EmitEvent({msg})"));
                    }
                    RuleAction::BurnAmount(pct) => {
                        rules_triggered.push(format!("BurnAmount({pct}%)"));
                    }
                }
            }
        }

        // Execute the method (snapshot state for rollback on quota violation).
        let contract = self
            .contracts
            .get_mut(&contract_id)
            .ok_or(ContractError::NotFound(contract_id))?;
        let creator = contract.creator;
        let state_snapshot = contract.state.clone();
        let return_value = execute_method(
            &contract.template,
            &mut contract.state,
            method,
            args,
            caller,
            &creator,
            current_epoch,
        )
        .inspect_err(|_| {
            contract.state = state_snapshot.clone();
        })?;

        // Enforce per-contract storage quota
        let state_size = serde_json::to_vec(&contract.state)
            .map(|v| v.len())
            .unwrap_or(0);
        if state_size > MAX_CONTRACT_STATE_BYTES {
            contract.state = state_snapshot;
            return Err(ContractError::StorageQuotaExceeded {
                size: state_size,
                max: MAX_CONTRACT_STATE_BYTES,
            });
        }

        // Deduct energy cost from contract.
        if energy_cost > 0 {
            let current = contract.energy_at(current_epoch);
            if current < energy_cost {
                return Err(ContractError::InsufficientFunds {
                    available: current,
                    required: energy_cost,
                });
            }
            // Approximate: reduce stored energy.
            contract.energy = contract.energy.saturating_sub(energy_cost);
        }

        Ok(CallResult {
            success: true,
            return_value,
            events,
            energy_cost,
            rules_triggered,
        })
    }

    /// Tick all contracts: decay contract energy, run template-specific ticks.
    pub fn tick(&mut self, current_epoch: Epoch) -> TickResult {
        let mut result = TickResult::default();
        let ids: Vec<u64> = self.contracts.keys().copied().collect();

        for id in ids {
            let contract = match self.contracts.get_mut(&id) {
                Some(c) if !c.evaporated => c,
                _ => continue,
            };

            result.contracts_ticked += 1;

            // Check if the contract itself has died.
            if contract.energy_at(current_epoch) == 0 {
                contract.evaporated = true;
                result.contracts_evaporated.push(id);
                result.events.push(format!(
                    "Contract {} ({}) evaporated at epoch {}",
                    id,
                    contract.template.name(),
                    current_epoch
                ));
                continue;
            }

            // Run template-specific tick logic.
            let tick_events = tick_template(&contract.template, &mut contract.state, current_epoch);
            result.events.extend(tick_events);

            // Evaluate OnTick rules.
            let rules = contract.rules.clone();
            for rule in &rules {
                if rule.trigger != RuleTrigger::OnTick {
                    continue;
                }
                if evaluate_condition(&rule.condition, &serde_json::Value::Null, &contract.state) {
                    if let RuleAction::EmitEvent(msg) = &rule.action {
                        result.events.push(msg.clone());
                    }
                }
            }
        }

        result
    }

    /// Get a contract by ID.
    pub fn get(&self, id: u64) -> Option<&ContractInstance> {
        self.contracts.get(&id)
    }

    /// List all contracts.
    pub fn list(&self) -> Vec<&ContractInstance> {
        self.contracts.values().collect()
    }

    /// Get contract state as JSON.
    pub fn get_state(&self, id: u64) -> Option<&serde_json::Value> {
        self.contracts.get(&id).map(|c| &c.state)
    }

    /// Number of contracts.
    pub fn len(&self) -> usize {
        self.contracts.len()
    }

    /// Whether there are no contracts.
    pub fn is_empty(&self) -> bool {
        self.contracts.is_empty()
    }

    /// Refresh a contract's energy.
    pub fn refresh_contract(
        &mut self,
        id: u64,
        energy_deposit: u64,
        epoch: Epoch,
    ) -> Result<(), ContractError> {
        let contract = self
            .contracts
            .get_mut(&id)
            .ok_or(ContractError::NotFound(id))?;
        if contract.evaporated {
            return Err(ContractError::Evaporated(id));
        }
        contract.energy = contract.energy_at(epoch) + energy_deposit;
        contract.last_refreshed = epoch;
        Ok(())
    }

    // ─── Internal ──────────────────────────────────────────────────────

    fn initialize_state(
        &self,
        template: &ContractTemplate,
        params: &serde_json::Value,
        epoch: Epoch,
    ) -> Result<serde_json::Value, ContractError> {
        match template {
            ContractTemplate::DecayingToken => {
                let name = get_str(params, "name")?;
                let symbol = get_str(params, "symbol")?;
                let total_supply = get_u64(params, "total_supply")?;
                let decay_half_life = get_u64(params, "decay_half_life")?;
                // Canonicalize the owner string at the API boundary so the
                // balances-map key matches `hex::encode(caller)` exactly.
                // Phase 3.1 (2026-05-03): now actually canonicalized
                // (the prior comment claimed it was, but the
                // canonicalize_address_hex helper was #[allow(dead_code)]
                // and never invoked).
                let owner = canonicalize_address_hex(&get_str(params, "owner")?)?;

                let mut balances = HashMap::new();
                balances.insert(owner.clone(), total_supply);

                let state = TokenState {
                    name,
                    symbol,
                    balances,
                    decay_half_life,
                    owner,
                    total_minted: total_supply,
                    total_decayed: 0,
                    last_tick_epoch: epoch,
                };
                Ok(serde_json::to_value(state).unwrap())
            }

            ContractTemplate::MortalNFT => {
                let collection_name = get_str(params, "collection_name")?;
                let max_supply = get_u64(params, "max_supply")?;
                // Optional subscription-renewal economics. Defaults
                // (renewal_fee=0) preserve the original gratis-refresh
                // behaviour. When > 0, refresh debits the caller's
                // pre-deposited balance and credits renewal_recipient
                // (or the contract creator if recipient is empty).
                let renewal_fee = get_u64(params, "renewal_fee").unwrap_or(0);
                // Phase 3.1: canonicalize renewal_recipient if supplied.
                // Empty string is the sentinel "creator pays / receives"
                // and stays empty; non-empty must be a valid address.
                let renewal_recipient_raw = params
                    .get("renewal_recipient")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let renewal_recipient = if renewal_recipient_raw.is_empty() {
                    String::new()
                } else {
                    canonicalize_address_hex(renewal_recipient_raw)?
                };
                // EVR-721 Phase 2.2: deployer-configurable grace period.
                // Default 5 epochs matches the spec / testnet number.
                let grace_period =
                    get_u64(params, "grace_period").unwrap_or_else(|_| default_grace_period());

                let state = NftState {
                    collection_name,
                    max_supply,
                    tokens: HashMap::new(),
                    next_token_id: 1,
                    last_tick_epoch: epoch,
                    renewal_fee,
                    renewal_recipient,
                    renewal_balances: HashMap::new(),
                    grace_period,
                    ghost_records: HashMap::new(),
                };
                Ok(serde_json::to_value(state).unwrap())
            }

            ContractTemplate::ThermodynamicEscrow => {
                // Phase 3.1 (2026-05-03): canonicalize sender/receiver
                // at init so stored fields match `hex::encode(caller)`
                // exactly. Comment claimed canonicalization since the
                // earlier sweep, but the helper was never invoked
                // until now.
                let sender = canonicalize_address_hex(&get_str(params, "sender")?)?;
                let receiver = canonicalize_address_hex(&get_str(params, "receiver")?)?;
                let amount = get_u64(params, "amount")?;
                let release_epoch = get_u64(params, "release_epoch")?;
                let decay_after_epochs = get_u64(params, "decay_after_epochs")?;

                let state = EscrowState {
                    sender,
                    receiver,
                    escrowed_amount: amount,
                    release_epoch,
                    decay_after_epochs,
                    claimed: false,
                    refunded: false,
                    decayed: false,
                };
                Ok(serde_json::to_value(state).unwrap())
            }

            ContractTemplate::DecayingAuction => {
                // Phase 3.1 (2026-05-03): canonicalize seller at init.
                let seller = canonicalize_address_hex(&get_str(params, "seller")?)?;
                let item_description = get_str(params, "item_description")?;
                let min_bid = get_u64(params, "min_bid")?;
                let duration_epochs = get_u64(params, "duration_epochs")?;
                let reserve_price = get_u64(params, "reserve_price")?;

                let state = AuctionState {
                    seller,
                    item_description,
                    min_bid,
                    duration_epochs,
                    reserve_price,
                    start_epoch: epoch,
                    bids: Vec::new(),
                    finalized: false,
                    winner: None,
                };
                Ok(serde_json::to_value(state).unwrap())
            }

            ContractTemplate::StakingPool => {
                let pool_name = get_str(params, "pool_name")?;
                let reward_rate = get_u64(params, "reward_rate_per_epoch")?;
                let reward_decay_hl = get_u64(params, "reward_decay_half_life")?;

                let state = StakingState {
                    pool_name,
                    reward_rate_per_epoch: reward_rate,
                    reward_decay_half_life: reward_decay_hl,
                    stakes: HashMap::new(),
                    total_staked: 0,
                    reward_pool: 0,
                    last_tick_epoch: epoch,
                };
                Ok(serde_json::to_value(state).unwrap())
            }

            ContractTemplate::DAOVote => {
                let title = get_str(params, "title")?;
                let description = get_str(params, "description")?;
                let options: Vec<String> = params
                    .get("options")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .ok_or_else(|| ContractError::InvalidParams("missing options".into()))?;
                let voting_period = get_u64(params, "voting_period_epochs")?;
                let quorum_pct = get_u64(params, "quorum_pct")?;

                let state = DaoState {
                    title,
                    description,
                    options,
                    voting_period_epochs: voting_period,
                    quorum_pct,
                    start_epoch: epoch,
                    votes: HashMap::new(),
                    finalized: false,
                    result: None,
                };
                Ok(serde_json::to_value(state).unwrap())
            }

            ContractTemplate::DecayingDAO => decaying_dao::init(params, epoch),

            ContractTemplate::TemporalContract => {
                let name = get_str(params, "name")?;
                // Phase 3.1 (2026-05-03): canonicalize at the API
                // boundary so caller_hex (always lowercase from
                // hex::encode) matches ts.owner regardless of how the
                // deployer encoded the param. See
                // audit/end_to_end_audit_2026_04_27.md §5.
                let owner = canonicalize_address_hex(&get_str(params, "owner")?)?;
                let low_energy_threshold = params
                    .get("low_energy_threshold")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100);

                // Parse phases from JSON array
                let phases: Vec<Phase> = params
                    .get("phases")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .ok_or_else(|| {
                        ContractError::InvalidParams("missing or invalid 'phases' array".into())
                    })?;

                if phases.is_empty() {
                    return Err(ContractError::InvalidParams(
                        "temporal contract must have at least one phase".into(),
                    ));
                }

                // Parse initial scheduled callbacks (optional)
                let callbacks: Vec<ScheduledCallback> = params
                    .get("callbacks")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                let state = TemporalState {
                    name,
                    owner,
                    phases,
                    current_phase: 0,
                    phase_start_epoch: epoch,
                    completed: false,
                    data: HashMap::new(),
                    callbacks,
                    transition_log: Vec::new(),
                    low_energy_threshold,
                    low_energy_mode: false,
                    last_tick_epoch: epoch,
                };
                Ok(serde_json::to_value(state).unwrap())
            }
        }
    }
}

impl Default for ContractEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Template Method Execution
// ═══════════════════════════════════════════════════════════════════════════

fn execute_method(
    template: &ContractTemplate,
    state: &mut serde_json::Value,
    method: &str,
    args: &serde_json::Value,
    caller: &AccountAddress,
    creator: &AccountAddress,
    current_epoch: Epoch,
) -> Result<serde_json::Value, ContractError> {
    match template {
        ContractTemplate::DecayingToken => exec_token(state, method, args, caller, creator),
        ContractTemplate::MortalNFT => {
            exec_nft(state, method, args, caller, creator, current_epoch)
        }
        ContractTemplate::ThermodynamicEscrow => {
            exec_escrow(state, method, args, caller, current_epoch)
        }
        ContractTemplate::DecayingAuction => {
            exec_auction(state, method, args, caller, current_epoch)
        }
        ContractTemplate::StakingPool => {
            exec_staking(state, method, args, caller, creator, current_epoch)
        }
        ContractTemplate::DAOVote => exec_dao(state, method, args, caller, current_epoch),
        ContractTemplate::DecayingDAO => {
            decaying_dao::exec(state, method, args, caller, current_epoch)
        }
        ContractTemplate::TemporalContract => {
            exec_temporal(state, method, args, caller, creator, current_epoch)
        }
    }
}

// ─────────────── DecayingToken ─────────────────────────────────────────

fn exec_token(
    state: &mut serde_json::Value,
    method: &str,
    args: &serde_json::Value,
    caller: &AccountAddress,
    creator: &AccountAddress,
) -> Result<serde_json::Value, ContractError> {
    let mut ts: TokenState = serde_json::from_value(state.clone())
        .map_err(|e| ContractError::StateError(e.to_string()))?;

    let result = match method {
        "mint" => {
            if caller != creator {
                return Err(ContractError::PermissionDenied(
                    "only owner can mint".into(),
                ));
            }
            let to = canonicalize_address_hex(&get_str(args, "to")?)?;
            let amount = get_u64(args, "amount")?;
            let bal = ts.balances.entry(to).or_insert(0);
            *bal = bal
                .checked_add(amount)
                .ok_or_else(|| ContractError::StateError("balance overflow".into()))?;
            ts.total_minted = ts
                .total_minted
                .checked_add(amount)
                .ok_or_else(|| ContractError::StateError("total_minted overflow".into()))?;
            serde_json::json!({ "minted": amount })
        }
        "transfer" => {
            let from = canonicalize_address_hex(&get_str(args, "from")?)?;
            let to = canonicalize_address_hex(&get_str(args, "to")?)?;
            let amount = get_u64(args, "amount")?;
            // EVR-20 spec auth: caller must equal `from` (token
            // holder is the only one who can move their balance).
            // The previous creator-override was a violation of
            // ERC-20 parity — it let the deployer move any holder's
            // tokens. Reconciled to spec 2026-05-03.
            let caller_hex = hex::encode(caller);
            if !caller_hex.eq_ignore_ascii_case(&from) {
                return Err(ContractError::PermissionDenied(
                    "caller must be the sender (caller == from) to transfer".into(),
                ));
            }
            let _ = creator; // no creator privilege on transfer per EVR-20
            let from_bal = ts.balances.get(&from).copied().unwrap_or(0);
            if from_bal < amount {
                return Err(ContractError::InsufficientFunds {
                    available: from_bal,
                    required: amount,
                });
            }
            let to_bal = ts.balances.get(&to).copied().unwrap_or(0);
            let new_to_bal = to_bal
                .checked_add(amount)
                .ok_or_else(|| ContractError::StateError("balance overflow".into()))?;
            *ts.balances.entry(from).or_insert(0) -= amount;
            *ts.balances.entry(to).or_insert(0) = new_to_bal;
            serde_json::json!({ "transferred": amount })
        }
        "balance_of" => {
            let addr = canonicalize_address_hex(&get_str(args, "addr")?)?;
            let bal = ts.balances.get(&addr).copied().unwrap_or(0);
            serde_json::json!({ "balance": bal })
        }
        "total_supply" => {
            let total: u64 = ts.balances.values().sum();
            serde_json::json!({ "total_supply": total })
        }
        "burn" => {
            // EVR-20 spec auth: token holder burns their own tokens.
            // Code historically required `caller == creator` which
            // contradicts ERC-20 parity (holders couldn't burn).
            // Reconciled to spec 2026-05-03: caller must equal `from`.
            // The contract creator retains an admin path only via
            // burning their own held balance (no special privilege).
            let from = canonicalize_address_hex(&get_str(args, "from")?)?;
            let caller_hex = hex::encode(caller);
            if !caller_hex.eq_ignore_ascii_case(&from) {
                return Err(ContractError::PermissionDenied(
                    "caller must be the holder to burn (caller == from)".into(),
                ));
            }
            let amount = get_u64(args, "amount")?;
            let bal = ts.balances.get(&from).copied().unwrap_or(0);
            if bal < amount {
                return Err(ContractError::InsufficientFunds {
                    available: bal,
                    required: amount,
                });
            }
            *ts.balances.entry(from).or_insert(0) -= amount;
            serde_json::json!({ "burned": amount })
        }
        "refresh_balance" => {
            // EVR-20 spec auth: any caller can refresh (incentivised
            // keeper op, similar to ERC-20's pattern of letting anyone
            // trigger lazy state-maintenance ops). The previous
            // creator-only check made the token un-maintainable
            // without the deployer online. The amount of `energy` to
            // restore is bounded by the per-balance refresh cost
            // upstream — this method just applies the credit.
            let addr = canonicalize_address_hex(&get_str(args, "addr")?)?;
            let energy = get_u64(args, "energy")?;
            *ts.balances.entry(addr).or_insert(0) += energy;
            // `caller` is intentionally not consulted — see comment.
            let _ = (caller, creator);
            serde_json::json!({ "refreshed": energy })
        }
        _ => return Err(ContractError::UnknownMethod(method.into())),
    };

    *state = serde_json::to_value(ts).unwrap();
    Ok(result)
}

// ─────────────── MortalNFT ─────────────────────────────────────────────

fn exec_nft(
    state: &mut serde_json::Value,
    method: &str,
    args: &serde_json::Value,
    caller: &AccountAddress,
    creator: &AccountAddress,
    current_epoch: Epoch,
) -> Result<serde_json::Value, ContractError> {
    let mut ns: NftState = serde_json::from_value(state.clone())
        .map_err(|e| ContractError::StateError(e.to_string()))?;

    let caller_hex = hex::encode(caller);

    let result = match method {
        "mint" => {
            // Only the contract creator can mint new NFTs.
            if caller != creator {
                return Err(ContractError::PermissionDenied(
                    "only contract creator can mint NFTs".into(),
                ));
            }
            // Phase 3.1 (2026-05-03): canonicalize the recipient
            // address so the stored NFT owner matches
            // `hex::encode(caller)` exactly for that recipient's
            // future transfer/refresh/burn calls.
            let to = canonicalize_address_hex(&get_str(args, "to")?)?;
            let metadata_hash = get_str(args, "metadata_hash")?;
            let energy = get_u64(args, "energy")?;
            let half_life = get_u64(args, "half_life")?;

            if ns.max_supply > 0 && ns.tokens.len() as u64 >= ns.max_supply {
                return Err(ContractError::StateError("max supply reached".into()));
            }

            let token_id = ns.next_token_id;
            ns.next_token_id += 1;
            ns.tokens.insert(
                token_id,
                NftInfo {
                    owner: to,
                    metadata_hash,
                    energy,
                    half_life,
                    minted_epoch: current_epoch,
                    state: NftLifecycleState::Active,
                    grace_epoch: None,
                    evaporated_epoch: None,
                    ghost_proof: None,
                },
            );
            // EVR-721 §"Required Events": `Mint` event.
            serde_json::json!({ "token_id": token_id, "event": "Mint" })
        }
        "transfer" => {
            let token_id = get_u64(args, "token_id")?;
            // Phase 3.1 (2026-05-03): canonicalize the recipient so
            // the new owner string matches `hex::encode(caller)` for
            // that recipient's future calls.
            let to = canonicalize_address_hex(&get_str(args, "to")?)?;
            let nft = ns
                .tokens
                .get_mut(&token_id)
                .ok_or_else(|| ContractError::StateError(format!("NFT {token_id} not found")))?;
            // EVR-721 §"States" table: only Active tokens are
            // transferable. Grace/Ghost tokens are read-only or
            // proof-only respectively.
            if nft.state != NftLifecycleState::Active {
                return Err(ContractError::PermissionDenied(format!(
                    "NFT {token_id} is in state {:?}; only Active tokens are transferable",
                    nft.state
                )));
            }
            // Only the NFT owner or contract creator can transfer it.
            if !nft.owner.eq_ignore_ascii_case(&caller_hex) && caller != creator {
                return Err(ContractError::PermissionDenied(format!(
                    "caller does not own NFT {token_id}"
                )));
            }
            let from = nft.owner.clone();
            nft.owner = to.clone();
            // EVR-721 §"Required Events": `Transfer` event.
            serde_json::json!({
                "transferred": token_id,
                "event": "Transfer",
                "from": from,
                "to": to,
            })
        }
        "owner_of" => {
            let token_id = get_u64(args, "token_id")?;
            let nft = ns
                .tokens
                .get(&token_id)
                .ok_or_else(|| ContractError::StateError(format!("NFT {token_id} not found")))?;
            serde_json::json!({ "owner": nft.owner })
        }
        "state_of" => {
            // EVR-721 §"Queries (Read)" `state_of`: returns the
            // current lifecycle state. Looks up the active map first,
            // then falls back to ghost_records for evaporated tokens.
            let token_id = get_u64(args, "token_id")?;
            if let Some(nft) = ns.tokens.get(&token_id) {
                serde_json::json!({ "state": nft.state })
            } else if ns.ghost_records.contains_key(&token_id) {
                serde_json::json!({ "state": NftLifecycleState::Ghost })
            } else {
                return Err(ContractError::StateError(format!(
                    "NFT {token_id} not found"
                )));
            }
        }
        "ghost_proof" => {
            // EVR-721 §"Queries (Read)" `ghost_proof`: only available
            // for tokens currently in Ghost state.
            let token_id = get_u64(args, "token_id")?;
            let gr = ns.ghost_records.get(&token_id).ok_or_else(|| {
                ContractError::StateError(format!(
                    "no ghost record for NFT {token_id} (token is not in Ghost state)"
                ))
            })?;
            serde_json::json!({
                "token_id": gr.token_id,
                "owner": gr.owner,
                "metadata_hash": gr.metadata_hash,
                "evaporated_epoch": gr.evaporated_epoch,
                "ghost_proof": gr.ghost_proof,
            })
        }
        "token_info" => {
            let token_id = get_u64(args, "token_id")?;
            let nft = ns
                .tokens
                .get(&token_id)
                .ok_or_else(|| ContractError::StateError(format!("NFT {token_id} not found")))?;
            let current_energy =
                energy_at_epoch(nft.energy, nft.half_life, current_epoch - nft.minted_epoch);
            serde_json::json!({
                "owner": nft.owner,
                "metadata_hash": nft.metadata_hash,
                "energy": nft.energy,
                "current_energy": current_energy,
                "half_life": nft.half_life,
                "minted_epoch": nft.minted_epoch
            })
        }
        "refresh" => {
            let token_id = get_u64(args, "token_id")?;
            let energy = get_u64(args, "energy")?;
            let renewal_fee = ns.renewal_fee;
            let recipient_key = if ns.renewal_recipient.is_empty() {
                hex::encode(creator)
            } else {
                ns.renewal_recipient.clone()
            };

            // EVR-721 §"Refresh": handles Active (extend), Grace
            // (extend + clear grace), and Ghost (resurrect from
            // ghost_records). The auth model is "any signature"
            // per the spec for refresh, but this template's
            // `renewal_fee` gate restricts who can pay — the
            // existing economic gate is preserved here.
            //
            // Resurrection path: if `token_id` is missing from
            // `tokens` but present in `ghost_records`, restore an
            // NftInfo from the ghost record, give it `energy` as
            // a fresh start, and clear grace/ghost fields. The
            // original owner stays the owner — Ghost-state
            // tokens cannot be reassigned by resurrection.
            let resurrect_path = !ns.tokens.contains_key(&token_id)
                && ns.ghost_records.contains_key(&token_id);

            if resurrect_path {
                let gr = ns
                    .ghost_records
                    .remove(&token_id)
                    .expect("checked above");
                // Auth on Ghost refresh: still require caller to be
                // the original owner OR the contract creator.
                if !gr.owner.eq_ignore_ascii_case(&caller_hex) && caller != creator {
                    // Restore the ghost record on auth fail; refresh
                    // didn't happen.
                    ns.ghost_records.insert(token_id, gr);
                    return Err(ContractError::PermissionDenied(format!(
                        "caller is not the original owner of ghost NFT {token_id}"
                    )));
                }
                // Renewal-fee gate also applies on resurrection.
                if renewal_fee > 0 && caller != creator {
                    let bal = ns.renewal_balances.get(&caller_hex).copied().unwrap_or(0);
                    if bal < renewal_fee {
                        ns.ghost_records.insert(token_id, gr);
                        return Err(ContractError::StateError(format!(
                            "resurrect requires {renewal_fee} units in renewal balance \
                             (caller has {bal}); call deposit_renewal first"
                        )));
                    }
                    ns.renewal_balances
                        .insert(caller_hex.clone(), bal - renewal_fee);
                    let r = ns
                        .renewal_balances
                        .get(&recipient_key)
                        .copied()
                        .unwrap_or(0);
                    ns.renewal_balances
                        .insert(recipient_key, r.saturating_add(renewal_fee));
                }
                // Use the half_life of the original ghost (we don't
                // store it on the GhostRecord, so default to the
                // collection's last-known shape via re-mint values).
                // Spec: resurrected token starts fresh — energy =
                // energy_deposit, minted_epoch = current_epoch.
                let resurrected = NftInfo {
                    owner: gr.owner.clone(),
                    metadata_hash: gr.metadata_hash.clone(),
                    energy,
                    half_life: 1, // half-life isn't recorded on the ghost; caller may re-mint with desired half-life if they want a longer-lived token
                    minted_epoch: current_epoch,
                    state: NftLifecycleState::Active,
                    grace_epoch: None,
                    evaporated_epoch: None,
                    ghost_proof: None,
                };
                ns.tokens.insert(token_id, resurrected);
                // EVR-721 §"Required Events": `Resurrected` event.
                let new_energy = ns.tokens.get(&token_id).map(|n| n.energy).unwrap_or(0);
                let result = serde_json::json!({
                    "new_energy": new_energy,
                    "event": "Resurrected",
                    "token_id": token_id,
                });
                *state = serde_json::to_value(ns).unwrap();
                return Ok(result);
            }

            let nft = ns
                .tokens
                .get_mut(&token_id)
                .ok_or_else(|| ContractError::StateError(format!("NFT {token_id} not found")))?;
            // Active or Grace: caller must be owner OR creator.
            // Ghost is handled by the resurrect_path above.
            if !nft.owner.eq_ignore_ascii_case(&caller_hex) && caller != creator {
                return Err(ContractError::PermissionDenied(format!(
                    "caller does not own NFT {token_id}"
                )));
            }
            // Subscription gate: when renewal_fee > 0, debit the caller's
            // pre-deposited renewal balance and credit the renewal_recipient.
            // Free for the original creator (they own the contract).
            if renewal_fee > 0 && caller != creator {
                let bal = ns.renewal_balances.get(&caller_hex).copied().unwrap_or(0);
                if bal < renewal_fee {
                    return Err(ContractError::StateError(format!(
                        "refresh requires {renewal_fee} units in renewal balance \
                         (caller has {bal}); call deposit_renewal first"
                    )));
                }
                ns.renewal_balances
                    .insert(caller_hex.clone(), bal - renewal_fee);
                let r = ns
                    .renewal_balances
                    .get(&recipient_key)
                    .copied()
                    .unwrap_or(0);
                ns.renewal_balances
                    .insert(recipient_key, r.saturating_add(renewal_fee));
                // Re-borrow nft after the second mutable borrow above.
                let nft = ns.tokens.get_mut(&token_id).expect("checked above");
                let current =
                    energy_at_epoch(nft.energy, nft.half_life, current_epoch - nft.minted_epoch);
                nft.energy = current + energy;
                nft.minted_epoch = current_epoch;
                // Refresh always returns the token to Active and
                // clears any grace marker — the energy deposit
                // resets the lifecycle clock.
                nft.state = NftLifecycleState::Active;
                nft.grace_epoch = None;
            } else {
                let current =
                    energy_at_epoch(nft.energy, nft.half_life, current_epoch - nft.minted_epoch);
                nft.energy = current + energy;
                nft.minted_epoch = current_epoch;
                nft.state = NftLifecycleState::Active;
                nft.grace_epoch = None;
            }
            // EVR-721 §"Required Events": `Refresh` event.
            serde_json::json!({
                "new_energy": ns.tokens.get(&token_id).map(|n| n.energy).unwrap_or(0),
                "event": "Refresh",
                "token_id": token_id,
                "energy_added": energy,
            })
        }
        "deposit_renewal" => {
            // Holders pre-fund their renewal balance. amount is taken on
            // trust (the off-chain payment rail credits it). For an
            // on-chain integration, a paired Token contract would invoke
            // this method on the NFT contract during a transfer.
            let amount = get_u64(args, "amount")?;
            let bal = ns.renewal_balances.get(&caller_hex).copied().unwrap_or(0);
            ns.renewal_balances
                .insert(caller_hex.clone(), bal.saturating_add(amount));
            serde_json::json!({ "balance": bal.saturating_add(amount) })
        }
        "renewal_balance" => {
            let addr = canonicalize_address_hex(&get_str(args, "addr")?)?;
            let bal = ns.renewal_balances.get(&addr).copied().unwrap_or(0);
            serde_json::json!({ "balance": bal })
        }
        "burn" => {
            let token_id = get_u64(args, "token_id")?;
            let nft = ns
                .tokens
                .get(&token_id)
                .ok_or_else(|| ContractError::StateError(format!("NFT {token_id} not found")))?;
            // Only the NFT owner or contract creator can burn.
            if !nft.owner.eq_ignore_ascii_case(&caller_hex) && caller != creator {
                return Err(ContractError::PermissionDenied(format!(
                    "caller cannot burn NFT {token_id}"
                )));
            }
            ns.tokens.remove(&token_id);
            serde_json::json!({ "burned": token_id })
        }
        _ => return Err(ContractError::UnknownMethod(method.into())),
    };

    *state = serde_json::to_value(ns).unwrap();
    Ok(result)
}

// ─────────────── ThermodynamicEscrow ───────────────────────────────────

fn exec_escrow(
    state: &mut serde_json::Value,
    method: &str,
    _args: &serde_json::Value,
    caller: &AccountAddress,
    current_epoch: Epoch,
) -> Result<serde_json::Value, ContractError> {
    let mut es: EscrowState = serde_json::from_value(state.clone())
        .map_err(|e| ContractError::StateError(e.to_string()))?;

    let caller_hex = hex::encode(caller);
    let result = match method {
        "claim" => {
            if es.claimed || es.refunded || es.decayed {
                return Err(ContractError::StateError("escrow already settled".into()));
            }
            if !caller_hex.eq_ignore_ascii_case(&es.receiver) {
                return Err(ContractError::PermissionDenied(
                    "only receiver can claim".into(),
                ));
            }
            if current_epoch < es.release_epoch {
                return Err(ContractError::StateError("not yet released".into()));
            }
            es.claimed = true;
            serde_json::json!({ "claimed": es.escrowed_amount })
        }
        "refund" => {
            if es.claimed || es.refunded || es.decayed {
                return Err(ContractError::StateError("escrow already settled".into()));
            }
            if !caller_hex.eq_ignore_ascii_case(&es.sender) {
                return Err(ContractError::PermissionDenied(
                    "only sender can refund".into(),
                ));
            }
            if current_epoch < es.release_epoch + es.decay_after_epochs {
                return Err(ContractError::StateError("decay period not elapsed".into()));
            }
            es.refunded = true;
            serde_json::json!({ "refunded": es.escrowed_amount })
        }
        "status" => {
            let status = if es.claimed {
                "claimed"
            } else if es.refunded {
                "refunded"
            } else if es.decayed {
                "decayed"
            } else if current_epoch >= es.release_epoch + es.decay_after_epochs {
                "expired"
            } else if current_epoch >= es.release_epoch {
                "claimable"
            } else {
                "locked"
            };
            serde_json::json!({
                "status": status,
                "amount": es.escrowed_amount,
                "release_epoch": es.release_epoch
            })
        }
        _ => return Err(ContractError::UnknownMethod(method.into())),
    };

    *state = serde_json::to_value(es).unwrap();
    Ok(result)
}

// ─────────────── DecayingAuction ───────────────────────────────────────

fn exec_auction(
    state: &mut serde_json::Value,
    method: &str,
    args: &serde_json::Value,
    caller: &AccountAddress,
    current_epoch: Epoch,
) -> Result<serde_json::Value, ContractError> {
    let caller_hex = hex::encode(caller);
    let mut aus: AuctionState = serde_json::from_value(state.clone())
        .map_err(|e| ContractError::StateError(e.to_string()))?;

    let result = match method {
        "bid" => {
            if aus.finalized {
                return Err(ContractError::StateError("auction finalized".into()));
            }
            if current_epoch >= aus.start_epoch + aus.duration_epochs {
                return Err(ContractError::StateError("auction ended".into()));
            }
            let bidder = canonicalize_address_hex(&get_str(args, "bidder")?)?;
            if !caller_hex.eq_ignore_ascii_case(&bidder) {
                return Err(ContractError::PermissionDenied(
                    "caller must be the bidder".into(),
                ));
            }
            let amount = get_u64(args, "amount")?;
            if amount < aus.min_bid {
                return Err(ContractError::StateError(format!(
                    "bid {} below minimum {}",
                    amount, aus.min_bid
                )));
            }
            let highest = aus.bids.last().map(|(_, a)| *a).unwrap_or(0);
            if amount <= highest {
                return Err(ContractError::StateError(
                    "bid must exceed current highest".into(),
                ));
            }
            aus.bids.push((bidder, amount));
            serde_json::json!({ "bid_accepted": amount })
        }
        "finalize" => {
            if aus.finalized {
                return Err(ContractError::StateError("already finalized".into()));
            }
            let ended = current_epoch >= aus.start_epoch + aus.duration_epochs;
            if !ended && !caller_hex.eq_ignore_ascii_case(&aus.seller) {
                return Err(ContractError::PermissionDenied(
                    "only seller can finalize before auction ends".into(),
                ));
            }
            aus.finalized = true;
            if let Some((winner, amount)) = aus.bids.last() {
                if *amount >= aus.reserve_price {
                    aus.winner = Some(winner.clone());
                    serde_json::json!({ "winner": winner, "price": amount })
                } else {
                    serde_json::json!({ "result": "reserve not met" })
                }
            } else {
                serde_json::json!({ "result": "no bids" })
            }
        }
        "highest_bid" => {
            let highest = aus
                .bids
                .last()
                .map(|(b, a)| serde_json::json!({ "bidder": b, "amount": a }));
            serde_json::json!({ "highest": highest })
        }
        "status" => {
            let ended = current_epoch >= aus.start_epoch + aus.duration_epochs;
            serde_json::json!({
                "finalized": aus.finalized,
                "ended": ended,
                "bid_count": aus.bids.len(),
                "winner": aus.winner
            })
        }
        "time_remaining" => {
            let end = aus.start_epoch + aus.duration_epochs;
            let remaining = end.saturating_sub(current_epoch);
            serde_json::json!({ "epochs_remaining": remaining })
        }
        _ => return Err(ContractError::UnknownMethod(method.into())),
    };

    *state = serde_json::to_value(aus).unwrap();
    Ok(result)
}

// ─────────────── StakingPool ──────────────────────────────────────────

fn exec_staking(
    state: &mut serde_json::Value,
    method: &str,
    args: &serde_json::Value,
    caller: &AccountAddress,
    creator: &AccountAddress,
    current_epoch: Epoch,
) -> Result<serde_json::Value, ContractError> {
    let mut ss: StakingState = serde_json::from_value(state.clone())
        .map_err(|e| ContractError::StateError(e.to_string()))?;

    let caller_hex = hex::encode(caller);

    let result = match method {
        "stake" => {
            let staker = canonicalize_address_hex(&get_str(args, "staker")?)?;
            // Caller must be the staker themselves, or the contract creator.
            if !caller_hex.eq_ignore_ascii_case(&staker) && caller != creator {
                return Err(ContractError::PermissionDenied(
                    "caller must be the staker or contract owner".into(),
                ));
            }
            let amount = get_u64(args, "amount")?;
            ss.total_staked += amount;
            let entry = ss.stakes.entry(staker).or_insert(StakeInfo {
                amount: 0,
                staked_epoch: current_epoch,
                unclaimed_rewards: 0,
                last_claim_epoch: current_epoch,
            });
            entry.amount += amount;
            serde_json::json!({ "staked": amount })
        }
        "unstake" => {
            let staker = canonicalize_address_hex(&get_str(args, "staker")?)?;
            // Caller must be the staker themselves, or the contract creator.
            if !caller_hex.eq_ignore_ascii_case(&staker) && caller != creator {
                return Err(ContractError::PermissionDenied(
                    "caller must be the staker or contract owner to unstake".into(),
                ));
            }
            let info = ss
                .stakes
                .remove(&staker)
                .ok_or_else(|| ContractError::StateError("not staked".into()))?;
            ss.total_staked = ss.total_staked.saturating_sub(info.amount);
            serde_json::json!({ "unstaked": info.amount, "unclaimed_rewards": info.unclaimed_rewards })
        }
        "claim_rewards" => {
            let staker = canonicalize_address_hex(&get_str(args, "staker")?)?;
            // Caller must be the staker themselves, or the contract creator.
            if !caller_hex.eq_ignore_ascii_case(&staker) && caller != creator {
                return Err(ContractError::PermissionDenied(
                    "caller must be the staker or contract owner to claim rewards".into(),
                ));
            }
            let info = ss
                .stakes
                .get_mut(&staker)
                .ok_or_else(|| ContractError::StateError("not staked".into()))?;
            let rewards = info.unclaimed_rewards;
            info.unclaimed_rewards = 0;
            info.last_claim_epoch = current_epoch;
            serde_json::json!({ "claimed": rewards })
        }
        "pool_info" => {
            serde_json::json!({
                "pool_name": ss.pool_name,
                "total_staked": ss.total_staked,
                "reward_pool": ss.reward_pool,
                "staker_count": ss.stakes.len()
            })
        }
        "pending_rewards" => {
            let staker = canonicalize_address_hex(&get_str(args, "staker")?)?;
            let info = ss
                .stakes
                .get(&staker)
                .ok_or_else(|| ContractError::StateError("not staked".into()))?;
            serde_json::json!({ "pending": info.unclaimed_rewards })
        }
        _ => return Err(ContractError::UnknownMethod(method.into())),
    };

    *state = serde_json::to_value(ss).unwrap();
    Ok(result)
}

// ─────────────── DAOVote ──────────────────────────────────────────────

fn exec_dao(
    state: &mut serde_json::Value,
    method: &str,
    args: &serde_json::Value,
    caller: &AccountAddress,
    current_epoch: Epoch,
) -> Result<serde_json::Value, ContractError> {
    let caller_hex = hex::encode(caller);
    let mut ds: DaoState = serde_json::from_value(state.clone())
        .map_err(|e| ContractError::StateError(e.to_string()))?;

    let result = match method {
        "vote" => {
            if ds.finalized {
                return Err(ContractError::StateError("voting finalized".into()));
            }
            if current_epoch >= ds.start_epoch + ds.voting_period_epochs {
                return Err(ContractError::StateError("voting period ended".into()));
            }
            let voter = canonicalize_address_hex(&get_str(args, "voter")?)?;
            if !caller_hex.eq_ignore_ascii_case(&voter) {
                return Err(ContractError::PermissionDenied(
                    "caller must be the voter".into(),
                ));
            }
            let option_idx = get_u64(args, "option_idx")? as usize;
            let weight = get_u64(args, "weight")?;
            if option_idx >= ds.options.len() {
                return Err(ContractError::StateError("invalid option".into()));
            }
            ds.votes.insert(voter, (option_idx, weight));
            serde_json::json!({ "voted": ds.options[option_idx] })
        }
        "results" => {
            let mut tallies = vec![0u64; ds.options.len()];
            for (idx, weight) in ds.votes.values() {
                if *idx < tallies.len() {
                    tallies[*idx] += weight;
                }
            }
            let results: Vec<_> = ds
                .options
                .iter()
                .zip(tallies.iter())
                .map(|(opt, count)| serde_json::json!({ "option": opt, "votes": count }))
                .collect();
            serde_json::json!({ "results": results, "total_voters": ds.votes.len() })
        }
        "is_finalized" => {
            serde_json::json!({ "finalized": ds.finalized, "result": ds.result })
        }
        "quorum_reached" => {
            let total_weight: u64 = ds.votes.values().map(|(_, w)| w).sum();
            let total_voters = ds.votes.len() as u64;
            // Simple quorum: percentage of voters
            let reached = if total_voters > 0 {
                total_voters * 100 >= ds.quorum_pct
            } else {
                false
            };
            serde_json::json!({ "reached": reached, "total_weight": total_weight })
        }
        "time_remaining" => {
            let end = ds.start_epoch + ds.voting_period_epochs;
            let remaining = end.saturating_sub(current_epoch);
            serde_json::json!({ "epochs_remaining": remaining })
        }
        _ => return Err(ContractError::UnknownMethod(method.into())),
    };

    *state = serde_json::to_value(ds).unwrap();
    Ok(result)
}

// ═══════════════════════════════════════════════════════════════════════════
// Template-specific tick logic
// ═══════════════════════════════════════════════════════════════════════════

fn tick_template(
    template: &ContractTemplate,
    state: &mut serde_json::Value,
    current_epoch: Epoch,
) -> Vec<String> {
    match template {
        ContractTemplate::DecayingToken => tick_token(state, current_epoch),
        ContractTemplate::MortalNFT => tick_nft(state, current_epoch),
        ContractTemplate::ThermodynamicEscrow => tick_escrow(state, current_epoch),
        ContractTemplate::DecayingAuction => tick_auction(state, current_epoch),
        ContractTemplate::StakingPool => tick_staking(state, current_epoch),
        ContractTemplate::DAOVote => tick_dao(state, current_epoch),
        ContractTemplate::DecayingDAO => decaying_dao::tick(state, current_epoch),
        ContractTemplate::TemporalContract => tick_temporal(state, current_epoch),
    }
}

fn tick_token(state: &mut serde_json::Value, current_epoch: Epoch) -> Vec<String> {
    let mut ts: TokenState = match serde_json::from_value(state.clone()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut events = Vec::new();

    if current_epoch <= ts.last_tick_epoch {
        return events;
    }

    let epochs_elapsed = current_epoch - ts.last_tick_epoch;
    let mut total_decayed_this_tick = 0u64;

    let keys: Vec<String> = ts.balances.keys().cloned().collect();
    for key in keys {
        let bal = ts.balances.get(&key).copied().unwrap_or(0);
        if bal == 0 {
            continue;
        }
        let new_bal = energy_at_epoch(bal, ts.decay_half_life, epochs_elapsed);
        let decay = bal - new_bal;
        total_decayed_this_tick += decay;
        if new_bal == 0 {
            ts.balances.remove(&key);
            events.push(format!("Token balance of {} swept to zero", key));
        } else {
            ts.balances.insert(key, new_bal);
        }
    }

    ts.total_decayed += total_decayed_this_tick;
    ts.last_tick_epoch = current_epoch;
    *state = serde_json::to_value(ts).unwrap();

    if total_decayed_this_tick > 0 {
        events.push(format!(
            "DecayingToken: {} total decayed this epoch",
            total_decayed_this_tick
        ));
    }
    events
}

fn tick_nft(state: &mut serde_json::Value, current_epoch: Epoch) -> Vec<String> {
    let mut ns: NftState = match serde_json::from_value(state.clone()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut events = Vec::new();

    // EVR-721 Phase 2.2: full Active → Grace → Ghost state machine.
    // Two passes per tick:
    //   Pass A: Active tokens whose computed energy is 0 transition
    //           to Grace, recording `grace_epoch = current_epoch`.
    //           Emits `GraceEntered`.
    //   Pass B: Grace tokens whose `grace_epoch + grace_period <=
    //           current_epoch` transition to Ghost. The token is
    //           moved out of `tokens` into a `GhostRecord`. Emits
    //           `Evaporated`.
    let grace_period = ns.grace_period;
    let mut to_grace: Vec<u64> = Vec::new();
    let mut to_ghost: Vec<u64> = Vec::new();

    for (&id, info) in ns.tokens.iter() {
        let current = energy_at_epoch(
            info.energy,
            info.half_life,
            current_epoch.saturating_sub(info.minted_epoch),
        );
        match info.state {
            NftLifecycleState::Active if current == 0 => {
                to_grace.push(id);
            }
            NftLifecycleState::Grace => {
                let cutoff = info.grace_epoch.unwrap_or(current_epoch).saturating_add(grace_period);
                if current_epoch >= cutoff {
                    to_ghost.push(id);
                }
            }
            // grace_period == 0: skip Grace and go straight to Ghost.
            NftLifecycleState::Active if grace_period == 0 && current == 0 => {
                to_ghost.push(id);
            }
            _ => {}
        }
    }

    for id in to_grace {
        if let Some(nft) = ns.tokens.get_mut(&id) {
            nft.state = NftLifecycleState::Grace;
            nft.grace_epoch = Some(current_epoch);
            events.push(format!(
                "GraceEntered: token={} epoch={}",
                id, current_epoch
            ));
        }
    }

    for id in to_ghost {
        if let Some(nft) = ns.tokens.remove(&id) {
            // Compute the canonical ghost proof:
            // Blake3(token_id : metadata_hash : evaporated_epoch).
            let mut hasher = blake3::Hasher::new();
            hasher.update(&id.to_le_bytes());
            hasher.update(b":");
            hasher.update(nft.metadata_hash.as_bytes());
            hasher.update(b":");
            hasher.update(&current_epoch.to_le_bytes());
            let ghost_proof = hex::encode(hasher.finalize().as_bytes());
            ns.ghost_records.insert(
                id,
                GhostRecord {
                    token_id: id,
                    owner: nft.owner.clone(),
                    metadata_hash: nft.metadata_hash.clone(),
                    evaporated_epoch: current_epoch,
                    ghost_proof: ghost_proof.clone(),
                },
            );
            events.push(format!(
                "Evaporated: token={} epoch={} ghost_proof={}",
                id, current_epoch, &ghost_proof[..16]
            ));
        }
    }

    ns.last_tick_epoch = current_epoch;
    *state = serde_json::to_value(ns).unwrap();
    events
}

fn tick_escrow(state: &mut serde_json::Value, current_epoch: Epoch) -> Vec<String> {
    let mut es: EscrowState = match serde_json::from_value(state.clone()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut events = Vec::new();

    if !es.claimed
        && !es.refunded
        && !es.decayed
        && current_epoch >= es.release_epoch + es.decay_after_epochs
    {
        es.decayed = true;
        events.push(format!(
            "ThermodynamicEscrow: {} evaporated (unclaimed)",
            es.escrowed_amount
        ));
    }

    *state = serde_json::to_value(es).unwrap();
    events
}

fn tick_auction(state: &mut serde_json::Value, current_epoch: Epoch) -> Vec<String> {
    let mut aus: AuctionState = match serde_json::from_value(state.clone()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut events = Vec::new();

    if !aus.finalized && current_epoch >= aus.start_epoch + aus.duration_epochs {
        aus.finalized = true;
        if let Some((winner, amount)) = aus.bids.last() {
            if *amount >= aus.reserve_price {
                aus.winner = Some(winner.clone());
                events.push(format!(
                    "DecayingAuction auto-finalized: winner={}, price={}",
                    winner, amount
                ));
            } else {
                events.push("DecayingAuction auto-finalized: reserve not met".into());
            }
        } else {
            events.push("DecayingAuction evaporated: no bids".into());
        }
    }

    *state = serde_json::to_value(aus).unwrap();
    events
}

fn tick_staking(state: &mut serde_json::Value, current_epoch: Epoch) -> Vec<String> {
    let mut ss: StakingState = match serde_json::from_value(state.clone()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut events = Vec::new();

    if current_epoch <= ss.last_tick_epoch {
        return events;
    }

    let epochs_elapsed = current_epoch - ss.last_tick_epoch;

    // Accumulate rewards for each staker.
    for (staker, info) in ss.stakes.iter_mut() {
        if info.amount == 0 {
            continue;
        }
        // New rewards = rate * epochs * (stake / total_staked)
        #[allow(unknown_lints, clippy::manual_checked_ops)]
        let new_rewards = if ss.total_staked > 0 {
            ss.reward_rate_per_epoch * epochs_elapsed * info.amount / ss.total_staked
        } else {
            0
        };
        // Add new rewards, then decay unclaimed portion.
        info.unclaimed_rewards += new_rewards;
        let unclaimed_elapsed = current_epoch.saturating_sub(info.last_claim_epoch);
        if unclaimed_elapsed > 0 && info.unclaimed_rewards > 0 {
            let decayed = energy_at_epoch(
                info.unclaimed_rewards,
                ss.reward_decay_half_life,
                unclaimed_elapsed,
            );
            let lost = info.unclaimed_rewards - decayed;
            if lost > 0 {
                events.push(format!(
                    "StakingPool: {} unclaimed rewards decayed for {}",
                    lost, staker
                ));
            }
            info.unclaimed_rewards = decayed;
        }
    }

    ss.last_tick_epoch = current_epoch;
    *state = serde_json::to_value(ss).unwrap();
    events
}

fn tick_dao(state: &mut serde_json::Value, current_epoch: Epoch) -> Vec<String> {
    let mut ds: DaoState = match serde_json::from_value(state.clone()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut events = Vec::new();

    let voting_end = ds.start_epoch + ds.voting_period_epochs;

    if !ds.finalized && current_epoch >= voting_end {
        ds.finalized = true;
        // Determine winner.
        let mut tallies = vec![0u64; ds.options.len()];
        for (idx, weight) in ds.votes.values() {
            if *idx < tallies.len() {
                tallies[*idx] += weight;
            }
        }
        if let Some((best_idx, _)) = tallies.iter().enumerate().max_by_key(|(_, &v)| v) {
            if tallies[best_idx] > 0 {
                ds.result = Some(ds.options[best_idx].clone());
                events.push(format!(
                    "DAOVote auto-finalized: result={}",
                    ds.options[best_idx]
                ));
            } else {
                events.push("DAOVote auto-finalized: no votes".into());
            }
        }
    }

    // Evaporate proposals older than 2x voting period.
    if current_epoch >= ds.start_epoch + 2 * ds.voting_period_epochs {
        events.push("DAOVote proposal evaporated (past 2x voting period)".into());
    }

    *state = serde_json::to_value(ds).unwrap();
    events
}

// ═══════════════════════════════════════════════════════════════════════════
// Rule Engine Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn method_to_trigger(method: &str) -> RuleTrigger {
    match method {
        "transfer" => RuleTrigger::OnTransfer,
        "mint" => RuleTrigger::OnMint,
        "burn" => RuleTrigger::OnBurn,
        "refresh" | "refresh_balance" => RuleTrigger::OnRefresh,
        _ => RuleTrigger::OnCall,
    }
}

fn evaluate_condition(
    condition: &RuleCondition,
    args: &serde_json::Value,
    _state: &serde_json::Value,
) -> bool {
    match condition {
        RuleCondition::Always => true,
        RuleCondition::If { field, op, value } => {
            // Try to extract the field value from args.
            let field_val = args.get(field).and_then(|v| v.as_u64()).unwrap_or(0);

            match op {
                ComparisonOp::Gt => field_val > *value,
                ComparisonOp::Lt => field_val < *value,
                ComparisonOp::Eq => field_val == *value,
                ComparisonOp::Gte => field_val >= *value,
                ComparisonOp::Lte => field_val <= *value,
                ComparisonOp::Neq => field_val != *value,
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Param Helpers
// ═══════════════════════════════════════════════════════════════════════════

// ─────────────── TemporalContract ──────────────────────────────────────

fn exec_temporal(
    state: &mut serde_json::Value,
    method: &str,
    args: &serde_json::Value,
    caller: &AccountAddress,
    creator: &AccountAddress,
    current_epoch: Epoch,
) -> Result<serde_json::Value, ContractError> {
    let mut ts: TemporalState = serde_json::from_value(state.clone())
        .map_err(|e| ContractError::StateError(e.to_string()))?;

    if ts.completed {
        return Err(ContractError::StateError(
            "temporal contract has completed all phases".into(),
        ));
    }

    // Check if the method is allowed in the current phase
    let current = &ts.phases[ts.current_phase];
    if !current.allowed_methods.is_empty() && !current.allowed_methods.contains(&method.to_string())
    {
        return Err(ContractError::PermissionDenied(format!(
            "method '{}' not allowed in phase '{}'",
            method, current.name
        )));
    }

    let caller_hex = hex::encode(caller);

    // Privileged methods require owner or creator authority.
    //
    // Both sides should be canonical lowercase post-2026-04-27 (the deploy-
    // time canonicalize_address_hex enforces it for new TemporalContract
    // deployments). The case-insensitive compare here is defense-in-depth
    // for any pre-fix instances that stored a non-canonical owner string —
    // it ensures legitimate owners aren't locked out by case mismatch.
    let is_privileged = matches!(method, "advance_phase" | "set_data" | "schedule_callback");
    let owner_match = ts.owner.eq_ignore_ascii_case(&caller_hex);
    if is_privileged && !owner_match && caller != creator {
        return Err(ContractError::PermissionDenied(format!(
            "only the owner can call '{}'",
            method,
        )));
    }

    let result = match method {
        // Query current phase info
        "get_phase" => {
            let phase = &ts.phases[ts.current_phase];
            let elapsed = current_epoch.saturating_sub(ts.phase_start_epoch);
            let remaining = if phase.duration_epochs > 0 {
                phase.duration_epochs.saturating_sub(elapsed)
            } else {
                0
            };
            serde_json::json!({
                "phase_index": ts.current_phase,
                "phase_name": phase.name,
                "elapsed_epochs": elapsed,
                "remaining_epochs": remaining,
                "auto_advance": phase.auto_advance,
                "low_energy_mode": ts.low_energy_mode,
                "completed": ts.completed,
            })
        }

        // Manually advance to the next phase (owner only or auto-advance)
        "advance_phase" => {
            if ts.current_phase + 1 >= ts.phases.len() {
                ts.completed = true;
                ts.transition_log.push(TransitionRecord {
                    from_phase: ts.phases[ts.current_phase].name.clone(),
                    to_phase: "COMPLETED".into(),
                    epoch: current_epoch,
                    reason: "manual advance — final phase".into(),
                });
                *state = serde_json::to_value(&ts).unwrap();
                return Ok(serde_json::json!({ "status": "completed", "phase": "COMPLETED" }));
            }

            let next_phase = &ts.phases[ts.current_phase + 1];
            // Check energy requirement for next phase
            if next_phase.min_energy > 0 {
                // Caller must provide energy info via args or we check contract energy
                // For now, min_energy is informational — checked at tick time
            }

            let from_name = ts.phases[ts.current_phase].name.clone();
            ts.current_phase += 1;
            ts.phase_start_epoch = current_epoch;
            let to_name = ts.phases[ts.current_phase].name.clone();

            ts.transition_log.push(TransitionRecord {
                from_phase: from_name,
                to_phase: to_name.clone(),
                epoch: current_epoch,
                reason: "manual advance".into(),
            });

            *state = serde_json::to_value(&ts).unwrap();
            return Ok(serde_json::json!({
                "status": "advanced",
                "new_phase": to_name,
                "phase_index": ts.current_phase,
            }));
        }

        // Set a key-value pair in the contract data store
        "set_data" => {
            let key = get_str(args, "key")?;
            let value = args
                .get("value")
                .cloned()
                .ok_or_else(|| ContractError::InvalidParams("missing 'value'".into()))?;
            ts.data.insert(key.clone(), value.clone());
            serde_json::json!({ "set": key, "value": value })
        }

        // Get a key from the data store
        "get_data" => {
            let key = get_str(args, "key")?;
            let value = ts
                .data
                .get(&key)
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            serde_json::json!({ "key": key, "value": value })
        }

        // Schedule a callback for a future epoch
        "schedule_callback" => {
            let trigger_epoch = get_u64(args, "trigger_epoch")?;
            let callback_name = get_str(args, "callback_name")?;
            let callback_args = args.get("args").cloned().unwrap_or(serde_json::Value::Null);

            if trigger_epoch <= current_epoch {
                return Err(ContractError::InvalidParams(
                    "trigger_epoch must be in the future".into(),
                ));
            }

            ts.callbacks.push(ScheduledCallback {
                trigger_epoch,
                callback_name: callback_name.clone(),
                args: callback_args,
                fired: false,
            });

            serde_json::json!({
                "scheduled": callback_name,
                "trigger_epoch": trigger_epoch,
            })
        }

        // Get the full transition history
        "get_history" => {
            serde_json::json!({
                "transitions": ts.transition_log,
                "current_phase": ts.phases[ts.current_phase].name,
                "total_phases": ts.phases.len(),
            })
        }

        // Get all pending callbacks
        "get_callbacks" => {
            let pending: Vec<&ScheduledCallback> =
                ts.callbacks.iter().filter(|c| !c.fired).collect();
            serde_json::to_value(pending).unwrap_or(serde_json::Value::Null)
        }

        other => {
            return Err(ContractError::UnknownMethod(other.to_string()));
        }
    };

    *state = serde_json::to_value(&ts).unwrap();
    Ok(result)
}

fn tick_temporal(state: &mut serde_json::Value, current_epoch: Epoch) -> Vec<String> {
    let mut ts: TemporalState = match serde_json::from_value(state.clone()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    if ts.completed || current_epoch <= ts.last_tick_epoch {
        return vec![];
    }

    let mut events = Vec::new();

    // 1. Check phase duration — auto-advance if expired
    if !ts.completed {
        let phase = &ts.phases[ts.current_phase];
        if phase.auto_advance && phase.duration_epochs > 0 {
            let elapsed = current_epoch.saturating_sub(ts.phase_start_epoch);
            if elapsed >= phase.duration_epochs {
                let from_name = phase.name.clone();

                if ts.current_phase + 1 >= ts.phases.len() {
                    ts.completed = true;
                    ts.transition_log.push(TransitionRecord {
                        from_phase: from_name.clone(),
                        to_phase: "COMPLETED".into(),
                        epoch: current_epoch,
                        reason: format!("auto-advance: duration {} expired", phase.duration_epochs),
                    });
                    events.push(format!(
                        "Temporal '{}': completed (phase '{}' expired at epoch {})",
                        ts.name, from_name, current_epoch
                    ));
                } else {
                    ts.current_phase += 1;
                    ts.phase_start_epoch = current_epoch;
                    let to_name = ts.phases[ts.current_phase].name.clone();
                    ts.transition_log.push(TransitionRecord {
                        from_phase: from_name.clone(),
                        to_phase: to_name.clone(),
                        epoch: current_epoch,
                        reason: format!("auto-advance: duration {} expired", phase.duration_epochs),
                    });
                    events.push(format!(
                        "Temporal '{}': phase '{}' → '{}' at epoch {}",
                        ts.name, from_name, to_name, current_epoch
                    ));
                }
            }
        }
    }

    // 2. Fire scheduled callbacks
    for callback in ts.callbacks.iter_mut() {
        if !callback.fired && current_epoch >= callback.trigger_epoch {
            callback.fired = true;
            events.push(format!(
                "Temporal '{}': callback '{}' fired at epoch {}",
                ts.name, callback.callback_name, current_epoch
            ));

            // Execute callback effects based on name
            match callback.callback_name.as_str() {
                "advance_phase" => {
                    if !ts.completed && ts.current_phase + 1 < ts.phases.len() {
                        let from_name = ts.phases[ts.current_phase].name.clone();
                        ts.current_phase += 1;
                        ts.phase_start_epoch = current_epoch;
                        let to_name = ts.phases[ts.current_phase].name.clone();
                        ts.transition_log.push(TransitionRecord {
                            from_phase: from_name,
                            to_phase: to_name.clone(),
                            epoch: current_epoch,
                            reason: "scheduled callback".into(),
                        });
                        events.push(format!(
                            "Temporal '{}': callback advanced to phase '{}'",
                            ts.name, to_name
                        ));
                    }
                }
                "set_data" => {
                    if let (Some(key), Some(value)) = (
                        callback.args.get("key").and_then(|v| v.as_str()),
                        callback.args.get("value"),
                    ) {
                        ts.data.insert(key.to_string(), value.clone());
                        events.push(format!(
                            "Temporal '{}': callback set data[{}]",
                            ts.name, key
                        ));
                    }
                }
                "complete" => {
                    if !ts.completed {
                        let from_name = ts.phases[ts.current_phase].name.clone();
                        ts.completed = true;
                        ts.transition_log.push(TransitionRecord {
                            from_phase: from_name,
                            to_phase: "COMPLETED".into(),
                            epoch: current_epoch,
                            reason: "scheduled completion callback".into(),
                        });
                    }
                }
                _ => {
                    // Custom callback — just log it
                    events.push(format!(
                        "Temporal '{}': custom callback '{}' args={}",
                        ts.name, callback.callback_name, callback.args
                    ));
                }
            }
        }
    }

    // 3. Check low-energy mode
    // Note: energy is checked on the ContractInstance level, but we track it here
    // as a state flag for phase-specific behavior
    // (The actual energy value is on ContractInstance, not in TemporalState)

    ts.last_tick_epoch = current_epoch;
    *state = serde_json::to_value(ts).unwrap();
    events
}

fn get_str(v: &serde_json::Value, key: &str) -> Result<String, ContractError> {
    v.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ContractError::InvalidParams(format!("missing string field: {key}")))
}

fn get_u64(v: &serde_json::Value, key: &str) -> Result<u64, ContractError> {
    v.get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ContractError::InvalidParams(format!("missing u64 field: {key}")))
}

/// Canonicalize a hex address string for stored-state comparisons.
///
/// Closes the contract-privilege type-canonicalization gap from
/// audit/end_to_end_audit_2026_04_27.md §5: previously, `caller_hex !=
/// ts.owner` could fail to match a legitimate owner whose deployer-supplied
/// owner string was uppercase or carried a `0x` prefix. After this, all
/// stored owner strings are canonical lowercase, no prefix, exactly 64 hex
/// chars — matching the output of `hex::encode(caller)`.
///
/// Accepts inputs with or without a leading `0x` / `0X` prefix. Rejects
/// anything that isn't a valid 32-byte hex address.
///
/// Phase 3.1 (2026-05-03): wired into every address-storing site
/// across all 8 templates. Previously this helper existed but was
/// dead-code; storage paths accepted raw strings (uppercase /
/// 0x-prefixed / friendly names) and the comparison sites did
/// `caller_hex.eq_ignore_ascii_case(&stored)` which masked the
/// canonicalization gap as long as the stored form happened to be
/// 64-char lowercase. The dead-code allowance is removed; any
/// caller that constructs a state field from a JSON string MUST
/// route through this helper.
fn canonicalize_address_hex(s: &str) -> Result<String, ContractError> {
    let trimmed = s.trim();
    let no_prefix = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if no_prefix.len() != 64 {
        return Err(ContractError::InvalidParams(format!(
            "address must be 64 hex chars (got {})",
            no_prefix.len()
        )));
    }
    if !no_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ContractError::InvalidParams(
            "address contains non-hex characters".into(),
        ));
    }
    Ok(no_prefix.to_ascii_lowercase())
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    /// Hex-encode `addr(b)` for use as a balances-map key.
    /// Phase 2.1 (2026-05-03) reconciled the `DecayingToken`
    /// auth model to match the EVR-20 spec, which keys balances by
    /// hex-encoded address. The friendly-string ("alice"/"bob")
    /// convention used in earlier tests collided with
    /// `caller_hex.eq_ignore_ascii_case(&from)` checks once the
    /// privileged-op gate was fixed; tests now use `addr_hex`.
    fn addr_hex(b: u8) -> String {
        hex::encode(addr(b))
    }

    fn engine() -> ContractEngine {
        ContractEngine::new()
    }

    // ─── canonicalize_address_hex ─────────────────────────────────────────

    #[test]
    fn test_canonicalize_address_hex_basic_forms() {
        let canonical = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let upper = canonical.to_ascii_uppercase();
        let prefixed_lower = format!("0x{}", canonical);
        let prefixed_upper_x = format!("0X{}", upper);
        let mixed = "AbCdEf0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

        assert_eq!(canonicalize_address_hex(canonical).unwrap(), canonical);
        assert_eq!(canonicalize_address_hex(&upper).unwrap(), canonical);
        assert_eq!(
            canonicalize_address_hex(&prefixed_lower).unwrap(),
            canonical
        );
        assert_eq!(
            canonicalize_address_hex(&prefixed_upper_x).unwrap(),
            canonical
        );
        assert_eq!(canonicalize_address_hex(mixed).unwrap(), canonical);

        // Whitespace tolerated via trim
        let padded = format!("  {}  ", canonical);
        assert_eq!(canonicalize_address_hex(&padded).unwrap(), canonical);
    }

    #[test]
    fn test_canonicalize_address_hex_rejects_wrong_length() {
        assert!(canonicalize_address_hex("").is_err());
        assert!(canonicalize_address_hex("abc").is_err());
        // 63 hex chars
        assert!(canonicalize_address_hex(&"a".repeat(63)).is_err());
        // 65 hex chars
        assert!(canonicalize_address_hex(&"a".repeat(65)).is_err());
    }

    #[test]
    fn test_canonicalize_address_hex_rejects_non_hex() {
        let bad = "g".to_string() + &"a".repeat(63);
        assert!(canonicalize_address_hex(&bad).is_err());
        // 0x prefix consumed, but non-hex inside still rejected
        let bad_prefixed = format!("0x{}", bad);
        assert!(canonicalize_address_hex(&bad_prefixed).is_err());
    }

    #[test]
    fn test_canonicalize_address_hex_idempotent() {
        let s = "deadbeef".repeat(8);
        let once = canonicalize_address_hex(&s).unwrap();
        let twice = canonicalize_address_hex(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn test_canonicalize_address_matches_hex_encode_output() {
        // Round-trip: hex::encode of a 32-byte address always produces a
        // canonical string. Canonicalizing it must be a no-op.
        let bytes = [0xCAu8; 32];
        let encoded = hex::encode(bytes);
        assert_eq!(canonicalize_address_hex(&encoded).unwrap(), encoded);
        // And the deployer-supplied uppercase variant canonicalizes to it
        let upper = encoded.to_ascii_uppercase();
        assert_eq!(canonicalize_address_hex(&upper).unwrap(), encoded);
    }

    // ─── Deploy Tests ──────────────────────────────────────────────

    #[test]
    fn test_deploy_decaying_token() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "TestCoin",
                    "symbol": "TC",
                    "total_supply": 1_000_000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                vec![],
                addr(1),
                1000,
                50,
                0,
            )
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(eng.len(), 1);
        let c = eng.get(id).unwrap();
        assert_eq!(c.template, ContractTemplate::DecayingToken);
    }

    #[test]
    fn test_deploy_mortal_nft() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::MortalNFT,
                serde_json::json!({
                    "collection_name": "Mortal Apes",
                    "max_supply": 100
                }),
                vec![],
                addr(1),
                1000,
                50,
                0,
            )
            .unwrap();
        assert!(eng.get(id).is_some());
    }

    #[test]
    fn test_deploy_thermodynamic_escrow() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::ThermodynamicEscrow,
                serde_json::json!({
                    "sender": hex::encode(addr(1)),
                    "receiver": hex::encode(addr(2)),
                    "amount": 5000,
                    "release_epoch": 100,
                    "decay_after_epochs": 50
                }),
                vec![],
                addr(1),
                1000,
                50,
                0,
            )
            .unwrap();
        assert!(eng.get(id).is_some());
    }

    #[test]
    fn test_deploy_decaying_auction() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::DecayingAuction,
                serde_json::json!({
                    "seller": addr_hex(1),
                    "item_description": "Rare sword",
                    "min_bid": 100,
                    "duration_epochs": 50,
                    "reserve_price": 500
                }),
                vec![],
                addr(1),
                1000,
                50,
                0,
            )
            .unwrap();
        assert!(eng.get(id).is_some());
    }

    #[test]
    fn test_deploy_staking_pool() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::StakingPool,
                serde_json::json!({
                    "pool_name": "MainPool",
                    "reward_rate_per_epoch": 100,
                    "reward_decay_half_life": 50
                }),
                vec![],
                addr(1),
                1000,
                50,
                0,
            )
            .unwrap();
        assert!(eng.get(id).is_some());
    }

    #[test]
    fn test_deploy_dao_vote() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::DAOVote,
                serde_json::json!({
                    "title": "Proposal #1",
                    "description": "Increase block size",
                    "options": ["Yes", "No", "Abstain"],
                    "voting_period_epochs": 100,
                    "quorum_pct": 50
                }),
                vec![],
                addr(1),
                1000,
                50,
                0,
            )
            .unwrap();
        assert!(eng.get(id).is_some());
    }

    // ─── DecayingToken Tests ───────────────────────────────────────

    fn deploy_token(eng: &mut ContractEngine) -> u64 {
        // Holder = addr(2) ("alice"); creator/deployer = addr(1).
        // EVR-20 keys balances by hex address (see addr_hex doc).
        eng.deploy(
            ContractTemplate::DecayingToken,
            serde_json::json!({
                "name": "TestCoin",
                "symbol": "TC",
                "total_supply": 10000,
                "decay_half_life": 10,
                "owner": addr_hex(2),
            }),
            vec![],
            addr(1),
            1000,
            100,
            0,
        )
        .unwrap()
    }

    #[test]
    fn test_token_mint_and_balance() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        let r = eng
            .call(
                id,
                "balance_of",
                &serde_json::json!({"addr": addr_hex(2)}),
                &addr(1),
                0,
            )
            .unwrap();
        assert_eq!(r.return_value["balance"], 10000);

        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": addr_hex(3), "amount": 500}),
            &addr(1),
            0,
        )
        .unwrap();
        let r = eng
            .call(
                id,
                "balance_of",
                &serde_json::json!({"addr": addr_hex(3)}),
                &addr(1),
                0,
            )
            .unwrap();
        assert_eq!(r.return_value["balance"], 500);
    }

    #[test]
    fn test_token_transfer() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // Holder = addr(2), recipient = addr(3). Caller MUST equal
        // `from` per EVR-20 spec.
        eng.call(
            id,
            "transfer",
            &serde_json::json!({"from": addr_hex(2), "to": addr_hex(3), "amount": 3000}),
            &addr(2),
            0,
        )
        .unwrap();

        let r = eng
            .call(
                id,
                "balance_of",
                &serde_json::json!({"addr": addr_hex(2)}),
                &addr(1),
                0,
            )
            .unwrap();
        assert_eq!(r.return_value["balance"], 7000);

        let r = eng
            .call(
                id,
                "balance_of",
                &serde_json::json!({"addr": addr_hex(3)}),
                &addr(1),
                0,
            )
            .unwrap();
        assert_eq!(r.return_value["balance"], 3000);
    }

    #[test]
    fn test_token_decay_over_epochs() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // Alice has 10000 with half_life=10. After 10 epochs, should be ~5000.
        eng.tick(10);

        let r = eng
            .call(
                id,
                "balance_of",
                &serde_json::json!({"addr": addr_hex(2)}),
                &addr(1),
                10,
            )
            .unwrap();
        let bal = r.return_value["balance"].as_u64().unwrap();
        assert_eq!(bal, 5000, "After one half-life, balance should halve");
    }

    #[test]
    fn test_token_balance_hits_zero() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // After many half-lives, balance should be negligible.
        // Note: integer division causes decay to plateau at a small value rather than exactly 0.
        for epoch in 1..=200 {
            eng.tick(epoch);
        }

        let r = eng
            .call(
                id,
                "balance_of",
                &serde_json::json!({"addr": addr_hex(2)}),
                &addr(1),
                200,
            )
            .unwrap();
        let bal = r.return_value["balance"].as_u64().unwrap();
        assert!(
            bal < 20,
            "After 20 half-lives, balance should be negligible, got {}",
            bal
        );
    }

    #[test]
    fn test_token_burn() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // EVR-20 spec: caller must be the holder. Holder = addr(2).
        eng.call(
            id,
            "burn",
            &serde_json::json!({"from": addr_hex(2), "amount": 2000}),
            &addr(2),
            0,
        )
        .unwrap();

        let r = eng
            .call(
                id,
                "balance_of",
                &serde_json::json!({"addr": addr_hex(2)}),
                &addr(1),
                0,
            )
            .unwrap();
        assert_eq!(r.return_value["balance"], 8000);
    }

    // ─── MortalNFT Tests ──────────────────────────────────────────

    fn deploy_nft(eng: &mut ContractEngine) -> u64 {
        eng.deploy(
            ContractTemplate::MortalNFT,
            serde_json::json!({
                "collection_name": "MortalApes",
                "max_supply": 10
            }),
            vec![],
            addr(1),
            5000,
            100,
            0,
        )
        .unwrap()
    }

    fn deploy_subscription_nft(eng: &mut ContractEngine, fee: u64) -> u64 {
        eng.deploy(
            ContractTemplate::MortalNFT,
            serde_json::json!({
                "collection_name": "SubscribeApes",
                "max_supply": 10,
                "renewal_fee": fee,
            }),
            vec![],
            addr(1), // creator
            5000,
            100,
            0,
        )
        .unwrap()
    }

    #[test]
    fn test_subscription_nft_refresh_requires_payment() {
        let mut eng = engine();
        let id = deploy_subscription_nft(&mut eng, 100);
        // Mint to alice (caller=creator addr(1))
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "alice", "metadata_hash": "abc", "energy": 50, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();
        // Alice tries to refresh without depositing — must fail with
        // "renewal balance" error.
        let alice_addr = {
            let mut a = [0u8; 32];
            a[0] = 0xAA;
            a
        };
        // First, set the NFT owner to alice's hex so the owner check passes.
        eng.call(
            id,
            "transfer",
            &serde_json::json!({"token_id": 1, "to": hex::encode(alice_addr)}),
            &addr(1),
            0,
        )
        .unwrap();
        let err = eng
            .call(
                id,
                "refresh",
                &serde_json::json!({"token_id": 1, "energy": 50}),
                &alice_addr,
                10,
            )
            .unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("renewal balance"),
            "expected fee gate, got: {msg}"
        );
    }

    #[test]
    fn test_subscription_nft_refresh_with_deposit_succeeds() {
        let mut eng = engine();
        let id = deploy_subscription_nft(&mut eng, 100);
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "alice", "metadata_hash": "abc", "energy": 50, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();
        let alice_addr = {
            let mut a = [0u8; 32];
            a[0] = 0xAA;
            a
        };
        eng.call(
            id,
            "transfer",
            &serde_json::json!({"token_id": 1, "to": hex::encode(alice_addr)}),
            &addr(1),
            0,
        )
        .unwrap();
        // Alice deposits 200, then refreshes (debits 100) — succeeds.
        eng.call(
            id,
            "deposit_renewal",
            &serde_json::json!({"amount": 200}),
            &alice_addr,
            10,
        )
        .unwrap();
        let r = eng
            .call(
                id,
                "refresh",
                &serde_json::json!({"token_id": 1, "energy": 50}),
                &alice_addr,
                10,
            )
            .unwrap();
        // Energy should be > 0 after a successful refresh.
        assert!(r.return_value["new_energy"].as_u64().unwrap() > 0);
        // Alice's balance dropped to 100, creator received 100.
        let r = eng
            .call(
                id,
                "renewal_balance",
                &serde_json::json!({"addr": hex::encode(alice_addr)}),
                &addr(1),
                10,
            )
            .unwrap();
        assert_eq!(r.return_value["balance"], 100);
        let r = eng
            .call(
                id,
                "renewal_balance",
                &serde_json::json!({"addr": hex::encode(addr(1))}),
                &addr(1),
                10,
            )
            .unwrap();
        assert_eq!(r.return_value["balance"], 100);
    }

    #[test]
    fn test_subscription_nft_creator_refresh_is_free() {
        let mut eng = engine();
        let id = deploy_subscription_nft(&mut eng, 100);
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "alice", "metadata_hash": "abc", "energy": 50, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();
        // Creator (addr(1)) refreshes without paying — should succeed.
        let r = eng
            .call(
                id,
                "refresh",
                &serde_json::json!({"token_id": 1, "energy": 100}),
                &addr(1),
                0,
            )
            .unwrap();
        assert!(r.return_value["new_energy"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_nft_mint_and_owner() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);

        let r = eng
            .call(
                id,
                "mint",
                &serde_json::json!({"to": "alice", "metadata_hash": "abc123", "energy": 100, "half_life": 5}),
                &addr(1),
                0,
            )
            .unwrap();
        let token_id = r.return_value["token_id"].as_u64().unwrap();
        assert_eq!(token_id, 1);

        let r = eng
            .call(
                id,
                "owner_of",
                &serde_json::json!({"token_id": 1}),
                &addr(1),
                0,
            )
            .unwrap();
        assert_eq!(r.return_value["owner"], "alice");
    }

    #[test]
    fn test_nft_transfer() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "alice", "metadata_hash": "abc", "energy": 100, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();

        eng.call(
            id,
            "transfer",
            &serde_json::json!({"token_id": 1, "to": "bob"}),
            &addr(1),
            0,
        )
        .unwrap();

        let r = eng
            .call(
                id,
                "owner_of",
                &serde_json::json!({"token_id": 1}),
                &addr(1),
                0,
            )
            .unwrap();
        assert_eq!(r.return_value["owner"], "bob");
    }

    #[test]
    fn test_nft_dies_after_lifespan() {
        // EVR-721 Phase 2.2: with default grace_period=5, the token
        // enters Grace at energy=0 then evaporates to Ghost only
        // after 5 more epochs.
        let mut eng = engine();
        let id = deploy_nft(&mut eng);
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "alice", "metadata_hash": "abc", "energy": 4, "half_life": 1}),
            &addr(1),
            0,
        )
        .unwrap();

        // Each `eng.tick(N)` is a single tick at epoch N, not a
        // sweep through all prior epochs. We need TWO ticks to drive
        // the full state machine:
        //   tick(3): energy reaches 0 → Active → Grace, with
        //            grace_epoch=3.
        //   tick(8): grace cutoff = 3 + grace_period(5) = 8 → Grace
        //            → Ghost.
        eng.tick(3);
        eng.tick(8);

        let r = eng.call(
            id,
            "owner_of",
            &serde_json::json!({"token_id": 1}),
            &addr(1),
            8,
        );
        assert!(r.is_err(), "owner_of should fail post-evaporation");

        let r = eng
            .call(
                id,
                "ghost_proof",
                &serde_json::json!({"token_id": 1}),
                &addr(1),
                8,
            )
            .unwrap();
        assert_eq!(r.return_value["token_id"], 1);
        assert_eq!(r.return_value["evaporated_epoch"], 8);

        let r = eng
            .call(
                id,
                "state_of",
                &serde_json::json!({"token_id": 1}),
                &addr(1),
                8,
            )
            .unwrap();
        assert_eq!(r.return_value["state"], "ghost");
    }

    #[test]
    fn test_nft_grace_state_at_energy_zero() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "alice", "metadata_hash": "abc", "energy": 4, "half_life": 1}),
            &addr(1),
            0,
        )
        .unwrap();
        eng.tick(3);

        let r = eng
            .call(
                id,
                "state_of",
                &serde_json::json!({"token_id": 1}),
                &addr(1),
                3,
            )
            .unwrap();
        assert_eq!(r.return_value["state"], "grace");

        let r = eng.call(
            id,
            "owner_of",
            &serde_json::json!({"token_id": 1}),
            &addr(1),
            3,
        );
        assert!(r.is_ok());
    }

    #[test]
    fn test_nft_transfer_rejected_in_grace_state() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "alice", "metadata_hash": "abc", "energy": 4, "half_life": 1}),
            &addr(1),
            0,
        )
        .unwrap();
        eng.tick(3);

        let r = eng.call(
            id,
            "transfer",
            &serde_json::json!({"token_id": 1, "to": "bob"}),
            &addr(1),
            3,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_nft_resurrect_from_ghost() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "alice", "metadata_hash": "abc", "energy": 4, "half_life": 1}),
            &addr(1),
            0,
        )
        .unwrap();
        // Two ticks: epoch 3 (Active→Grace), epoch 8 (Grace→Ghost).
        eng.tick(3);
        eng.tick(8);

        let r = eng
            .call(
                id,
                "refresh",
                &serde_json::json!({"token_id": 1, "energy": 100}),
                &addr(1),
                8,
            )
            .unwrap();
        assert_eq!(r.return_value["event"], "Resurrected");

        let r = eng
            .call(
                id,
                "state_of",
                &serde_json::json!({"token_id": 1}),
                &addr(1),
                8,
            )
            .unwrap();
        assert_eq!(r.return_value["state"], "active");

        let r = eng.call(
            id,
            "ghost_proof",
            &serde_json::json!({"token_id": 1}),
            &addr(1),
            8,
        );
        assert!(r.is_err());
    }

    // ─── ThermodynamicEscrow Tests ─────────────────────────────────

    fn deploy_escrow(eng: &mut ContractEngine) -> u64 {
        eng.deploy(
            ContractTemplate::ThermodynamicEscrow,
            serde_json::json!({
                "sender": hex::encode(addr(1)),
                "receiver": hex::encode(addr(2)),
                "amount": 5000,
                "release_epoch": 10,
                "decay_after_epochs": 5
            }),
            vec![],
            addr(1),
            10000,
            100,
            0,
        )
        .unwrap()
    }

    #[test]
    fn test_escrow_claim_after_release() {
        let mut eng = engine();
        let id = deploy_escrow(&mut eng);

        let r = eng
            .call(id, "claim", &serde_json::json!({}), &addr(2), 10)
            .unwrap();
        assert_eq!(r.return_value["claimed"], 5000);
    }

    #[test]
    fn test_escrow_claim_before_release_fails() {
        let mut eng = engine();
        let id = deploy_escrow(&mut eng);

        let r = eng.call(id, "claim", &serde_json::json!({}), &addr(2), 5);
        assert!(r.is_err());
    }

    #[test]
    fn test_escrow_refund_after_decay() {
        let mut eng = engine();
        let id = deploy_escrow(&mut eng);

        // Refund available after release_epoch + decay_after_epochs = 15
        let r = eng
            .call(id, "refund", &serde_json::json!({}), &addr(1), 15)
            .unwrap();
        assert_eq!(r.return_value["refunded"], 5000);
    }

    #[test]
    fn test_escrow_unclaimed_evaporates() {
        let mut eng = engine();
        let id = deploy_escrow(&mut eng);

        // Tick past decay period.
        eng.tick(16);

        let r = eng
            .call(id, "status", &serde_json::json!({}), &addr(1), 16)
            .unwrap();
        assert_eq!(r.return_value["status"], "decayed");
        let state = eng.get_state(id).unwrap();
        let es: EscrowState = serde_json::from_value(state.clone()).unwrap();
        assert!(es.decayed);
    }

    // ─── DecayingAuction Tests ─────────────────────────────────────

    fn deploy_auction(eng: &mut ContractEngine) -> u64 {
        eng.deploy(
            ContractTemplate::DecayingAuction,
            serde_json::json!({
                "seller": addr_hex(1),
                "item_description": "Rare Sword",
                "min_bid": 100,
                "duration_epochs": 10,
                "reserve_price": 500
            }),
            vec![],
            addr(1),
            5000,
            100,
            0,
        )
        .unwrap()
    }

    #[test]
    fn test_auction_bid_and_outbid() {
        let mut eng = engine();
        let id = deploy_auction(&mut eng);

        let bidder2 = hex::encode(addr(2));
        let bidder3 = hex::encode(addr(3));
        eng.call(
            id,
            "bid",
            &serde_json::json!({"bidder": bidder2, "amount": 200}),
            &addr(2),
            1,
        )
        .unwrap();
        eng.call(
            id,
            "bid",
            &serde_json::json!({"bidder": bidder3, "amount": 600}),
            &addr(3),
            2,
        )
        .unwrap();

        let r = eng
            .call(id, "highest_bid", &serde_json::json!({}), &addr(1), 2)
            .unwrap();
        assert_eq!(r.return_value["highest"]["amount"], 600);
    }

    #[test]
    fn test_auction_auto_finalize() {
        let mut eng = engine();
        let id = deploy_auction(&mut eng);

        let bidder = hex::encode(addr(2));
        eng.call(
            id,
            "bid",
            &serde_json::json!({"bidder": bidder, "amount": 700}),
            &addr(2),
            1,
        )
        .unwrap();

        // Tick past duration.
        let result = eng.tick(11);
        assert!(!result.events.is_empty());

        let state = eng.get_state(id).unwrap();
        let aus: AuctionState = serde_json::from_value(state.clone()).unwrap();
        assert!(aus.finalized);
        assert_eq!(aus.winner, Some(bidder));
    }

    #[test]
    fn test_auction_no_bids_evaporates() {
        let mut eng = engine();
        let _id = deploy_auction(&mut eng);

        let result = eng.tick(11);
        assert!(result.events.iter().any(|e| e.contains("no bids")));
    }

    // ─── StakingPool Tests ─────────────────────────────────────────

    fn deploy_staking(eng: &mut ContractEngine) -> u64 {
        eng.deploy(
            ContractTemplate::StakingPool,
            serde_json::json!({
                "pool_name": "MainPool",
                "reward_rate_per_epoch": 1000,
                "reward_decay_half_life": 10
            }),
            vec![],
            addr(1),
            50000,
            100,
            0,
        )
        .unwrap()
    }

    #[test]
    fn test_staking_stake_and_rewards() {
        let mut eng = engine();
        let id = deploy_staking(&mut eng);

        eng.call(
            id,
            "stake",
            &serde_json::json!({"staker": "alice", "amount": 1000}),
            &addr(1),
            0,
        )
        .unwrap();

        // Tick 5 epochs — rewards accumulate.
        eng.tick(5);

        let r = eng
            .call(
                id,
                "pending_rewards",
                &serde_json::json!({"staker": "alice"}),
                &addr(1),
                5,
            )
            .unwrap();
        let pending = r.return_value["pending"].as_u64().unwrap();
        assert!(pending > 0, "Should have accumulated rewards");
    }

    #[test]
    fn test_staking_unclaimed_rewards_decay() {
        let mut eng = engine();
        let id = deploy_staking(&mut eng);

        eng.call(
            id,
            "stake",
            &serde_json::json!({"staker": "alice", "amount": 1000}),
            &addr(1),
            0,
        )
        .unwrap();

        // Tick to accumulate rewards.
        eng.tick(5);
        let r1 = eng
            .call(
                id,
                "pending_rewards",
                &serde_json::json!({"staker": "alice"}),
                &addr(1),
                5,
            )
            .unwrap();
        let pending_at_5 = r1.return_value["pending"].as_u64().unwrap();

        // Tick more — unclaimed rewards should decay.
        eng.tick(15);
        let r2 = eng
            .call(
                id,
                "pending_rewards",
                &serde_json::json!({"staker": "alice"}),
                &addr(1),
                15,
            )
            .unwrap();
        let pending_at_15 = r2.return_value["pending"].as_u64().unwrap();

        // Rewards at 15 should be LESS than we'd expect without decay
        // (even though new rewards accumulated, the old ones decayed).
        assert!(
            pending_at_15 > 0,
            "Should still have some rewards: {}",
            pending_at_15
        );
        // The decay effect should be visible: without decay, rewards would be 3x what they were at 5.
        // With decay (hl=10), the early rewards have halved.
        assert!(
            pending_at_15 < pending_at_5 * 4,
            "Decay should reduce total: {} vs {}*4={}",
            pending_at_15,
            pending_at_5,
            pending_at_5 * 4
        );
    }

    #[test]
    fn test_staking_claim_rewards() {
        let mut eng = engine();
        let id = deploy_staking(&mut eng);

        eng.call(
            id,
            "stake",
            &serde_json::json!({"staker": "alice", "amount": 1000}),
            &addr(1),
            0,
        )
        .unwrap();

        eng.tick(5);

        let r = eng
            .call(
                id,
                "claim_rewards",
                &serde_json::json!({"staker": "alice"}),
                &addr(1),
                5,
            )
            .unwrap();
        let claimed = r.return_value["claimed"].as_u64().unwrap();
        assert!(claimed > 0);

        // After claiming, pending should be 0.
        let r2 = eng
            .call(
                id,
                "pending_rewards",
                &serde_json::json!({"staker": "alice"}),
                &addr(1),
                5,
            )
            .unwrap();
        assert_eq!(r2.return_value["pending"], 0);
    }

    // ─── DAOVote Tests ─────────────────────────────────────────────

    fn deploy_dao(eng: &mut ContractEngine) -> u64 {
        eng.deploy(
            ContractTemplate::DAOVote,
            serde_json::json!({
                "title": "Proposal #1",
                "description": "Increase block size",
                "options": ["Yes", "No"],
                "voting_period_epochs": 10,
                "quorum_pct": 1
            }),
            vec![],
            addr(1),
            5000,
            100,
            0,
        )
        .unwrap()
    }

    #[test]
    fn test_dao_vote_and_results() {
        let mut eng = engine();
        let id = deploy_dao(&mut eng);

        let voter1 = hex::encode(addr(1));
        let voter2 = hex::encode(addr(2));
        eng.call(
            id,
            "vote",
            &serde_json::json!({"voter": voter1, "option_idx": 0, "weight": 100}),
            &addr(1),
            1,
        )
        .unwrap();
        eng.call(
            id,
            "vote",
            &serde_json::json!({"voter": voter2, "option_idx": 1, "weight": 50}),
            &addr(2),
            2,
        )
        .unwrap();

        let r = eng
            .call(id, "results", &serde_json::json!({}), &addr(1), 3)
            .unwrap();
        let results = r.return_value["results"].as_array().unwrap();
        assert_eq!(results[0]["votes"], 100); // Yes
        assert_eq!(results[1]["votes"], 50); // No
    }

    #[test]
    fn test_dao_auto_finalize() {
        let mut eng = engine();
        let id = deploy_dao(&mut eng);

        let voter = hex::encode(addr(1));
        eng.call(
            id,
            "vote",
            &serde_json::json!({"voter": voter, "option_idx": 0, "weight": 100}),
            &addr(1),
            1,
        )
        .unwrap();

        eng.tick(11);

        let state = eng.get_state(id).unwrap();
        let ds: DaoState = serde_json::from_value(state.clone()).unwrap();
        assert!(ds.finalized);
        assert_eq!(ds.result, Some("Yes".into()));
    }

    #[test]
    fn test_dao_evaporation() {
        let mut eng = engine();
        let _id = deploy_dao(&mut eng);

        // Tick past 2x voting period (20 epochs).
        let result = eng.tick(21);
        assert!(result.events.iter().any(|e| e.contains("evaporated")));
    }

    // ─── Rule Engine Tests ─────────────────────────────────────────

    #[test]
    fn test_rule_on_transfer_cost_energy() {
        let mut eng = engine();
        let rules = vec![Rule {
            trigger: RuleTrigger::OnTransfer,
            condition: RuleCondition::If {
                field: "amount".into(),
                op: ComparisonOp::Gt,
                value: 5000,
            },
            action: RuleAction::CostEnergy(100),
        }];

        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "RuleCoin",
                    "symbol": "RC",
                    "total_supply": 100000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                rules,
                addr(1),
                10000,
                100,
                0,
            )
            .unwrap();

        // Small transfer: no rule triggered.
        let r = eng
            .call(
                id,
                "transfer",
                &serde_json::json!({"from": addr_hex(1), "to": addr_hex(2), "amount": 1000}),
                &addr(1),
                0,
            )
            .unwrap();
        assert!(r.rules_triggered.is_empty());
        assert_eq!(r.energy_cost, 0);

        // Large transfer: rule triggers.
        let r = eng
            .call(
                id,
                "transfer",
                &serde_json::json!({"from": addr_hex(1), "to": addr_hex(2), "amount": 10000}),
                &addr(1),
                0,
            )
            .unwrap();
        assert!(!r.rules_triggered.is_empty());
        assert_eq!(r.energy_cost, 100);
    }

    #[test]
    fn test_rule_reject_action() {
        let mut eng = engine();
        let rules = vec![Rule {
            trigger: RuleTrigger::OnMint,
            condition: RuleCondition::Always,
            action: RuleAction::Reject,
        }];

        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "NoMintCoin",
                    "symbol": "NM",
                    "total_supply": 1000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                rules,
                addr(1),
                10000,
                100,
                0,
            )
            .unwrap();

        // Minting should be rejected.
        let r = eng.call(
            id,
            "mint",
            &serde_json::json!({"to": addr_hex(2), "amount": 500}),
            &addr(1),
            0,
        );
        assert!(r.is_err());
        match r.unwrap_err() {
            ContractError::RejectedByRule(_) => {}
            e => panic!("Expected RejectedByRule, got {:?}", e),
        }
    }

    #[test]
    fn test_rule_emit_event() {
        let mut eng = engine();
        let rules = vec![Rule {
            trigger: RuleTrigger::OnTransfer,
            condition: RuleCondition::Always,
            action: RuleAction::EmitEvent("transfer_happened".into()),
        }];

        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "EventCoin",
                    "symbol": "EC",
                    "total_supply": 10000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                rules,
                addr(1),
                10000,
                100,
                0,
            )
            .unwrap();

        let r = eng
            .call(
                id,
                "transfer",
                &serde_json::json!({"from": addr_hex(1), "to": addr_hex(2), "amount": 100}),
                &addr(1),
                0,
            )
            .unwrap();
        assert!(r.events.contains(&"transfer_happened".to_string()));
    }

    // ─── Contract Lifecycle Tests ──────────────────────────────────

    #[test]
    fn test_contract_evaporates_when_energy_zero() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "DyingCoin",
                    "symbol": "DC",
                    "total_supply": 1000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                vec![],
                addr(1),
                4, // Very low energy
                1, // Very fast decay (hl=1)
                0,
            )
            .unwrap();

        // After enough epochs, contract energy → 0.
        let result = eng.tick(5);
        assert!(
            result.contracts_evaporated.contains(&id),
            "Contract should evaporate"
        );
    }

    #[test]
    fn test_call_to_evaporated_contract_fails() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "DyingCoin",
                    "symbol": "DC",
                    "total_supply": 1000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                vec![],
                addr(1),
                4,
                1,
                0,
            )
            .unwrap();

        eng.tick(5);

        let r = eng.call(
            id,
            "balance_of",
            &serde_json::json!({"addr": addr_hex(1)}),
            &addr(1),
            5,
        );
        assert!(r.is_err());
        match r.unwrap_err() {
            ContractError::Evaporated(_) => {}
            e => panic!("Expected Evaporated, got {:?}", e),
        }
    }

    #[test]
    fn test_list_contracts() {
        let mut eng = engine();
        deploy_token(&mut eng);
        deploy_nft(&mut eng);
        deploy_auction(&mut eng);
        assert_eq!(eng.list().len(), 3);
    }

    #[test]
    fn test_contract_not_found() {
        let mut eng = engine();
        let r = eng.call(999, "anything", &serde_json::json!({}), &addr(1), 0);
        assert!(matches!(r, Err(ContractError::NotFound(999))));
    }

    #[test]
    fn test_unknown_method() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);
        let r = eng.call(id, "nonexistent", &serde_json::json!({}), &addr(1), 0);
        assert!(matches!(r, Err(ContractError::UnknownMethod(_))));
    }

    #[test]
    fn test_full_lifecycle_deploy_use_decay_evaporate() {
        let mut eng = engine();

        // Deploy with low energy.
        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "MortalCoin",
                    "symbol": "MC",
                    "total_supply": 10000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                vec![],
                addr(1),
                8, // Low energy
                2, // Fast decay
                0,
            )
            .unwrap();

        // Use it.
        eng.call(
            id,
            "transfer",
            &serde_json::json!({"from": addr_hex(1), "to": addr_hex(2), "amount": 1000}),
            &addr(1),
            0,
        )
        .unwrap();

        // Verify it works.
        let r = eng
            .call(
                id,
                "balance_of",
                &serde_json::json!({"addr": addr_hex(2)}),
                &addr(1),
                0,
            )
            .unwrap();
        assert_eq!(r.return_value["balance"], 1000);

        // Let it decay.
        let result = eng.tick(10);
        assert!(
            result.contracts_evaporated.contains(&id),
            "Contract should have evaporated by epoch 10"
        );

        // Can't use anymore.
        let r = eng.call(
            id,
            "balance_of",
            &serde_json::json!({"addr": addr_hex(2)}),
            &addr(1),
            10,
        );
        assert!(r.is_err());
    }

    #[test]
    fn test_refresh_contract_energy() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "RefreshCoin",
                    "symbol": "RF",
                    "total_supply": 1000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                vec![],
                addr(1),
                100,
                10,
                0,
            )
            .unwrap();

        // Refresh at epoch 5.
        eng.refresh_contract(id, 500, 5).unwrap();
        let c = eng.get(id).unwrap();
        assert!(
            c.energy_at(5) > 100,
            "Energy should have increased after refresh"
        );
    }

    #[test]
    fn test_deploy_with_custom_rules_and_verify() {
        let mut eng = engine();
        let rules = vec![
            Rule {
                trigger: RuleTrigger::OnTransfer,
                condition: RuleCondition::If {
                    field: "amount".into(),
                    op: ComparisonOp::Gt,
                    value: 10000,
                },
                action: RuleAction::CostEnergy(100),
            },
            Rule {
                trigger: RuleTrigger::OnBurn,
                condition: RuleCondition::Always,
                action: RuleAction::EmitEvent("burn_event".into()),
            },
        ];

        let id = eng
            .deploy(
                ContractTemplate::DecayingToken,
                serde_json::json!({
                    "name": "RuleCoin",
                    "symbol": "RC",
                    "total_supply": 100000,
                    "decay_half_life": 100,
                    "owner": addr_hex(1),
                }),
                rules,
                addr(1),
                10000,
                100,
                0,
            )
            .unwrap();

        // Burn triggers EmitEvent rule.
        let r = eng
            .call(
                id,
                "burn",
                &serde_json::json!({"from": addr_hex(1), "amount": 100}),
                &addr(1),
                0,
            )
            .unwrap();
        assert!(r.events.contains(&"burn_event".to_string()));
    }

    #[test]
    fn test_token_total_supply() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        let r = eng
            .call(id, "total_supply", &serde_json::json!({}), &addr(1), 0)
            .unwrap();
        assert_eq!(r.return_value["total_supply"], 10000);
    }

    #[test]
    fn test_token_insufficient_balance() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // Caller must be the holder per EVR-20; the holder requesting
        // more than their balance gets InsufficientFunds (not
        // PermissionDenied).
        let r = eng.call(
            id,
            "transfer",
            &serde_json::json!({"from": addr_hex(2), "to": addr_hex(3), "amount": 999999}),
            &addr(2),
            0,
        );
        assert!(matches!(r, Err(ContractError::InsufficientFunds { .. })));
    }

    // ─── Access Control Tests (C-11 fix) ──────────────────────────

    #[test]
    fn test_token_mint_rejected_for_non_owner() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // addr(2) is NOT the creator (addr(1)), so mint should fail.
        let r = eng.call(
            id,
            "mint",
            &serde_json::json!({"to": addr_hex(4), "amount": 1000000}),
            &addr(2),
            0,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_token_burn_rejected_for_non_holder() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // EVR-20 spec: only the holder can burn their own tokens.
        // addr(3) tries to burn addr(2)'s ("alice's") balance — must
        // fail. The contract creator (addr(1)) likewise has no
        // privilege to burn other holders' tokens.
        let r = eng.call(
            id,
            "burn",
            &serde_json::json!({"from": addr_hex(2), "amount": 100}),
            &addr(3),
            0,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));

        let r = eng.call(
            id,
            "burn",
            &serde_json::json!({"from": addr_hex(2), "amount": 100}),
            &addr(1), // creator — no privilege per EVR-20
            0,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_token_transfer_rejected_for_non_sender() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // addr(3) is not the holder, so transferring addr(2)'s
        // tokens must fail.
        let r = eng.call(
            id,
            "transfer",
            &serde_json::json!({"from": addr_hex(2), "to": addr_hex(4), "amount": 100}),
            &addr(3),
            0,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_token_transfer_rejected_for_creator_when_not_holder() {
        let mut eng = engine();
        let id = deploy_token(&mut eng);

        // EVR-20 spec: creator has NO override on transfer. Even
        // though addr(1) deployed the contract, transferring
        // someone else's balance must fail. (Reconciled 2026-05-03
        // from the previous over-permissive `caller == creator`
        // bypass that contradicted ERC-20 parity.)
        let r = eng.call(
            id,
            "transfer",
            &serde_json::json!({"from": addr_hex(2), "to": addr_hex(3), "amount": 100}),
            &addr(1),
            0,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_nft_mint_rejected_for_non_creator() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);

        // addr(2) is NOT the creator, so minting should fail.
        let r = eng.call(
            id,
            "mint",
            &serde_json::json!({"to": "eve", "metadata_hash": "evil", "energy": 100, "half_life": 5}),
            &addr(2),
            0,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_nft_transfer_rejected_for_non_owner() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);

        // Creator mints to "alice".
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": hex::encode(addr(5)), "metadata_hash": "abc", "energy": 100, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();

        // addr(3) is neither the NFT owner (addr(5)) nor the creator (addr(1)).
        let r = eng.call(
            id,
            "transfer",
            &serde_json::json!({"token_id": 1, "to": "mallory"}),
            &addr(3),
            0,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_nft_transfer_allowed_by_nft_owner() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);

        // Creator mints to addr(5).
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": hex::encode(addr(5)), "metadata_hash": "abc", "energy": 100, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();

        // addr(5) IS the NFT owner, so transfer succeeds.
        let r = eng.call(
            id,
            "transfer",
            &serde_json::json!({"token_id": 1, "to": "bob"}),
            &addr(5),
            0,
        );
        assert!(r.is_ok());
    }

    #[test]
    fn test_nft_burn_rejected_for_non_owner() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);

        // Creator mints to addr(5).
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": hex::encode(addr(5)), "metadata_hash": "abc", "energy": 100, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();

        // addr(3) is neither the NFT owner (addr(5)) nor the creator (addr(1)).
        let r = eng.call(id, "burn", &serde_json::json!({"token_id": 1}), &addr(3), 0);
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_nft_burn_allowed_by_nft_owner() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);

        // Creator mints to addr(5).
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": hex::encode(addr(5)), "metadata_hash": "abc", "energy": 100, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();

        // addr(5) owns the NFT, so burn succeeds.
        let r = eng.call(id, "burn", &serde_json::json!({"token_id": 1}), &addr(5), 0);
        assert!(r.is_ok());
    }

    #[test]
    fn test_nft_burn_allowed_by_creator() {
        let mut eng = engine();
        let id = deploy_nft(&mut eng);

        // Creator mints to addr(5).
        eng.call(
            id,
            "mint",
            &serde_json::json!({"to": hex::encode(addr(5)), "metadata_hash": "abc", "energy": 100, "half_life": 5}),
            &addr(1),
            0,
        )
        .unwrap();

        // addr(1) is the creator, so burn succeeds even though they don't own the NFT.
        let r = eng.call(id, "burn", &serde_json::json!({"token_id": 1}), &addr(1), 0);
        assert!(r.is_ok());
    }

    #[test]
    fn test_staking_unstake_rejected_for_non_staker() {
        let mut eng = engine();
        let id = deploy_staking(&mut eng);

        // Creator stakes for "alice".
        eng.call(
            id,
            "stake",
            &serde_json::json!({"staker": "alice", "amount": 1000}),
            &addr(1),
            0,
        )
        .unwrap();

        // addr(3) is neither the staker ("alice") nor the creator (addr(1)).
        let r = eng.call(
            id,
            "unstake",
            &serde_json::json!({"staker": "alice"}),
            &addr(3),
            5,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_staking_claim_rewards_rejected_for_non_staker() {
        let mut eng = engine();
        let id = deploy_staking(&mut eng);

        eng.call(
            id,
            "stake",
            &serde_json::json!({"staker": "alice", "amount": 1000}),
            &addr(1),
            0,
        )
        .unwrap();

        eng.tick(5);

        // addr(3) is neither the staker nor the creator.
        let r = eng.call(
            id,
            "claim_rewards",
            &serde_json::json!({"staker": "alice"}),
            &addr(3),
            5,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_temporal_advance_phase_rejected_for_non_owner() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::TemporalContract,
                serde_json::json!({
                    "name": "TestTemporal",
                    "owner": hex::encode(addr(1)),
                    "phases": [
                        {"name": "phase1", "duration_epochs": 10, "min_energy": 0, "auto_advance": false, "allowed_methods": [], "energy_cost_per_epoch": 0},
                        {"name": "phase2", "duration_epochs": 10, "min_energy": 0, "auto_advance": false, "allowed_methods": [], "energy_cost_per_epoch": 0}
                    ]
                }),
                vec![],
                addr(1),
                5000,
                100,
                0,
            )
            .unwrap();

        // addr(3) is neither the owner (hex of addr(1)) nor the creator (addr(1)).
        let r = eng.call(id, "advance_phase", &serde_json::json!({}), &addr(3), 5);
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_temporal_set_data_rejected_for_non_owner() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::TemporalContract,
                serde_json::json!({
                    "name": "TestTemporal",
                    "owner": hex::encode(addr(1)),
                    "phases": [
                        {"name": "phase1", "duration_epochs": 10, "min_energy": 0, "auto_advance": false, "allowed_methods": [], "energy_cost_per_epoch": 0}
                    ]
                }),
                vec![],
                addr(1),
                5000,
                100,
                0,
            )
            .unwrap();

        // addr(3) is not authorized.
        let r = eng.call(
            id,
            "set_data",
            &serde_json::json!({"key": "evil", "value": "hack"}),
            &addr(3),
            0,
        );
        assert!(matches!(r, Err(ContractError::PermissionDenied(_))));
    }

    #[test]
    fn test_temporal_read_methods_allowed_for_anyone() {
        let mut eng = engine();
        let id = eng
            .deploy(
                ContractTemplate::TemporalContract,
                serde_json::json!({
                    "name": "TestTemporal",
                    "owner": hex::encode(addr(1)),
                    "phases": [
                        {"name": "phase1", "duration_epochs": 10, "min_energy": 0, "auto_advance": false, "allowed_methods": [], "energy_cost_per_epoch": 0}
                    ]
                }),
                vec![],
                addr(1),
                5000,
                100,
                0,
            )
            .unwrap();

        // Read-only methods should work for any caller.
        let r = eng.call(id, "get_phase", &serde_json::json!({}), &addr(3), 0);
        assert!(r.is_ok());

        let r = eng.call(id, "get_history", &serde_json::json!({}), &addr(3), 0);
        assert!(r.is_ok());

        let r = eng.call(id, "get_callbacks", &serde_json::json!({}), &addr(3), 0);
        assert!(r.is_ok());
    }
}
