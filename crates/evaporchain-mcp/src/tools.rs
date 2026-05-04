//! 26 MCP Tools — actions AI agents can take on the EvaporChain blockchain.

use serde_json::{json, Value};

use crate::protocol::Context;

/// Return the list of all 26 tools.
pub fn list_tools() -> Value {
    json!({
        "tools": [
            // ── Read tools ──
            {
                "name": "get_chain_status",
                "description": "Get the current status of the EvaporChain blockchain — block height, epoch, active objects, ghosts, peer count, uptime, and state root.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "list_objects",
                "description": "List all active state objects on the chain. Each object has energy that decays over time — when energy hits zero, the object evaporates into a ghost.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "get_object",
                "description": "Get detailed info about a specific state object by its hex ID — including current energy, decay percentage, half-life, state (Active/Grace/Ghost), and owner.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Hex ID of the object (e.g. '0a' or '0x0a')"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "list_accounts",
                "description": "List all accounts on the chain with their names, addresses, balances, and nonces.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "list_ghosts",
                "description": "List all evaporated (ghost) objects — state that has decayed to zero energy. Only their cryptographic nullifier proof remains.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "get_recent_blocks",
                "description": "Get recent blocks from the chain. Each block shows transactions, evaporations, grace transitions, and state root changes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Number of recent blocks to return (default: 20, max: 50)"
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "get_block",
                "description": "Get a specific block by its block number.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "number": {
                            "type": "integer",
                            "description": "Block number"
                        }
                    },
                    "required": ["number"]
                }
            },
            {
                "name": "get_recent_events",
                "description": "Get recent chain events — evaporations, creations, grace transitions, refreshes, resurrections, and transfers.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Number of recent events (default: 30)"
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "list_contracts",
                "description": "List all deployed smart contracts — both template contracts and EvaporScript contracts. Shows template type, creator, energy, half-life, and status.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "get_stats",
                "description": "Get aggregate chain statistics — total objects created, evaporated, resurrected, refreshed, and average object lifetime in epochs.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            // ── Write tools ──
            {
                "name": "transfer",
                "description": "Transfer EVAP tokens from one account to another. Accounts are identified by address number (1=Alice, 2=Bob, 3=Charlie).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "integer", "description": "Sender address (e.g. 1 for Alice)" },
                        "to": { "type": "integer", "description": "Recipient address (e.g. 2 for Bob)" },
                        "amount": { "type": "integer", "description": "Amount of EVAP to transfer" },
                        "nonce": { "type": "integer", "description": "Transaction nonce (default: 0)" }
                    },
                    "required": ["from", "to", "amount"]
                }
            },
            {
                "name": "create_object",
                "description": "Create a new state object with thermodynamic energy. The object will decay over time based on its half-life — when energy reaches zero, it evaporates into a ghost.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "creator": { "type": "integer", "description": "Creator address (e.g. 1 for Alice)" },
                        "object_id": { "type": "integer", "description": "Unique object ID (1-255)" },
                        "energy": { "type": "integer", "description": "Initial energy budget" },
                        "half_life": { "type": "integer", "description": "Epochs for energy to halve" }
                    },
                    "required": ["creator", "object_id", "energy", "half_life"]
                }
            },
            {
                "name": "refresh_object",
                "description": "Deposit energy into an existing state object to extend its lifetime and prevent evaporation.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "object_id": { "type": "integer", "description": "Object ID to refresh" },
                        "energy_deposit": { "type": "integer", "description": "Energy to add" }
                    },
                    "required": ["object_id", "energy_deposit"]
                }
            },
            {
                "name": "resurrect_object",
                "description": "Resurrect an evaporated ghost object by providing a new energy deposit. The object returns to active state with fresh energy.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "object_id": { "type": "integer", "description": "Ghost object ID to resurrect" },
                        "energy_deposit": { "type": "integer", "description": "Energy to deposit for resurrection" }
                    },
                    "required": ["object_id", "energy_deposit"]
                }
            },
            {
                "name": "request_faucet",
                "description": "Request free testnet EVAP tokens from the faucet. Rate limited to once per address per hour. Gives 10,000 EVAP.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "address": { "type": "integer", "description": "Account address (0-255)" }
                    },
                    "required": ["address"]
                }
            },
            // ── Substrate thermodynamic tools (new) ──
            {
                "name": "get_autopoietic_health",
                "description": "Check the chain's autopoietic viability (Maturana-Varela 1980). Reports whether the three self-sustaining subsystems are healthy: Patronage (self-funding), Sentinel (self-maintenance), LLSA (self-boundary). Status: Viable | Stressed | Inviable.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "compute_demurrage",
                "description": "Calculate how much demurrage an idle account balance owes. Native EvaporChain demurrage: piecewise log rate lambda_base_ppm × log2(balance/threshold) ppm/epoch. Sink goes to the refresh pool.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "balance": { "type": "integer", "description": "Account balance in energy units" },
                        "last_touched_epoch": { "type": "integer", "description": "Epoch when account was last active" },
                        "current_epoch": { "type": "integer", "description": "Current chain epoch" },
                        "lambda_base_ppm": { "type": "integer", "description": "Rate coefficient (ppm per epoch per log-doubling). Default genesis: 1" },
                        "threshold": { "type": "integer", "description": "Energy threshold below which demurrage is zero. Default genesis: 1024" }
                    },
                    "required": ["balance", "last_touched_epoch", "current_epoch"]
                }
            },
            {
                "name": "get_epv_status",
                "description": "Get the Evaporative Protocol Versioning status — which protocol versions are currently live and their energy budgets. Versions whose energy decays to zero are automatically pruned.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "get_fee_status",
                "description": "Get the current fee controller state — base fee, target gas, EIP-1559-style drift direction (Rising/Falling/Stable), and EMA of recent gas usage.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "get_consensus_phase",
                "description": "Get the current RG Phase Map consensus regime: LivenessStable | SafetyStable | Frozen | Chaotic. Also returns the last WSBF EffectiveParams (renormalized λ_eff, energy density, entropy_mb).",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "check_annealing_temperature",
                "description": "Compute the self-annealing effective temperature at a given epoch. T(epoch) = λ × 2^(−epoch/λ). Approaches zero as the validator set crystallises — fully crystallised = no degrading moves accepted (Kirkpatrick-Gelatt-Vecchi 1983).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "lambda_half_life": { "type": "integer", "description": "Chain λ half-life in epochs" },
                        "epoch": { "type": "integer", "description": "Epoch to evaluate temperature at" }
                    },
                    "required": ["lambda_half_life", "epoch"]
                }
            },
            {
                "name": "compute_mera_commitment",
                "description": "Build a MERA tensor-network state commitment from a list of account energy values. Returns the blake3 root hash and per-layer hashes. The root goes in the block header as the chain's λ-parameterised state commitment (Vidal 2007).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "energies": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Account energy values (physical layer)"
                        },
                        "lambda_half_life": { "type": "integer", "description": "Chain λ half-life in epochs (default: 4096)" },
                        "base_half_life": { "type": "integer", "description": "τ₀ base half-life for MERA layer 0 (default: 100)" }
                    },
                    "required": ["energies"]
                }
            },
            {
                "name": "prove_fork_evaporated",
                "description": "Prove that a competing fork has λ-decayed below the evaporation threshold. Returns an EvaporatedForkCert with blake3 witness binding. If is_evaporated=true, the fork is safely ignorable by light clients.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "fork_root_hex": { "type": "string", "description": "64-hex blake3 hash of the competing fork root" },
                        "blocks": {
                            "type": "array",
                            "description": "Block energy entries: [{seed_energy, observed_epoch}]",
                            "items": { "type": "object" }
                        },
                        "evaluated_at_epoch": { "type": "integer", "description": "Epoch at which to evaluate decay" },
                        "threshold": { "type": "integer", "description": "Energy threshold below which fork is evaporated" },
                        "lambda_epochs": { "type": "integer", "description": "Chain λ half-life in epochs" }
                    },
                    "required": ["fork_root_hex", "blocks", "evaluated_at_epoch", "threshold", "lambda_epochs"]
                }
            },
            {
                "name": "get_oracle_status",
                "description": "Get the status of the on-chain oracle bridge — whether it is active, how many feed keys are tracked, how many BLS quorum rounds are open, and the current oracle state root. The oracle is a decay-aware data feed: stale entries λ-evaporate automatically.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "get_shard_health",
                "description": "Get per-shard health metrics: liveness_ratio (live_objects / total_objects), total_energy, is_dead, and which shards are candidates for compaction. Useful for identifying shards that are degrading and need operator attention.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "check_conservation",
                "description": "Audit whether a hypothetical block transition (before → after EnergyAccumulator) satisfies the §1.2 conservation invariant: total energy must be non-increasing, and any drop must be ≤ what the global λ-decay allows over the elapsed epochs. Returns {valid, before_total, after_total} or {valid:false, error}.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "before_accounts":     { "type": "integer", "description": "Accounts compartment energy before block" },
                        "before_stake":        { "type": "integer", "description": "Stake compartment energy before block" },
                        "before_refresh_pool": { "type": "integer", "description": "RefreshPool compartment energy before block" },
                        "before_slashed_pool": { "type": "integer", "description": "SlashedPool compartment energy before block" },
                        "after_accounts":      { "type": "integer", "description": "Accounts compartment energy after block" },
                        "after_stake":         { "type": "integer", "description": "Stake compartment energy after block" },
                        "after_refresh_pool":  { "type": "integer", "description": "RefreshPool compartment energy after block" },
                        "after_slashed_pool":  { "type": "integer", "description": "SlashedPool compartment energy after block" },
                        "epochs_elapsed":      { "type": "integer", "description": "Number of epochs that elapsed during the block" },
                        "half_life_epochs":    { "type": "integer", "description": "Chain λ half-life in epochs (default 4096)" }
                    },
                    "required": ["before_accounts","before_stake","before_refresh_pool","before_slashed_pool","after_accounts","after_stake","after_refresh_pool","after_slashed_pool","epochs_elapsed"]
                }
            }
        ]
    })
}

/// Execute a tool call.
pub async fn call_tool(ctx: &Context, params: &Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing tool name")?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = match name {
        "get_chain_status" => {
            let data = ctx.get_json("/api/status").await?;
            format_text_result(&data)
        }
        "list_objects" => {
            let data = ctx.get_json("/api/objects").await?;
            format_text_result(&data)
        }
        "get_object" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'id' argument")?;
            let clean = id.strip_prefix("0x").unwrap_or(id);
            let data = ctx.get_json(&format!("/api/object/{clean}")).await?;
            format_text_result(&data)
        }
        "list_accounts" => {
            let data = ctx.get_json("/api/accounts").await?;
            format_text_result(&data)
        }
        "list_ghosts" => {
            let data = ctx.get_json("/api/ghosts").await?;
            format_text_result(&data)
        }
        "get_recent_blocks" => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(20)
                .min(50);
            let data = ctx.get_json(&format!("/api/blocks?limit={limit}")).await?;
            format_text_result(&data)
        }
        "get_block" => {
            let num = args
                .get("number")
                .and_then(|v| v.as_u64())
                .ok_or("Missing 'number' argument")?;
            let data = ctx.get_json(&format!("/api/block/{num}")).await?;
            format_text_result(&data)
        }
        "get_recent_events" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30);
            let data = ctx.get_json(&format!("/api/events?limit={limit}")).await?;
            format_text_result(&data)
        }
        "list_contracts" => {
            let data = ctx.get_json("/api/contracts").await?;
            format_text_result(&data)
        }
        "get_stats" => {
            let data = ctx.get_json("/api/stats/summary").await?;
            format_text_result(&data)
        }
        "transfer" => {
            let body = json!({
                "from": args.get("from").ok_or("Missing 'from'")?,
                "to": args.get("to").ok_or("Missing 'to'")?,
                "amount": args.get("amount").ok_or("Missing 'amount'")?,
                "nonce": args.get("nonce").unwrap_or(&json!(0))
            });
            let data = ctx.post_json("/api/tx/transfer", &body).await?;
            format_text_result(&data)
        }
        "create_object" => {
            let body = json!({
                "creator": args.get("creator").ok_or("Missing 'creator'")?,
                "object_id": args.get("object_id").ok_or("Missing 'object_id'")?,
                "energy": args.get("energy").ok_or("Missing 'energy'")?,
                "half_life": args.get("half_life").ok_or("Missing 'half_life'")?,
            });
            let data = ctx.post_json("/api/tx/create-object", &body).await?;
            format_text_result(&data)
        }
        "refresh_object" => {
            let body = json!({
                "object_id": args.get("object_id").ok_or("Missing 'object_id'")?,
                "energy_deposit": args.get("energy_deposit").ok_or("Missing 'energy_deposit'")?,
            });
            let data = ctx.post_json("/api/tx/refresh", &body).await?;
            format_text_result(&data)
        }
        "resurrect_object" => {
            let body = json!({
                "object_id": args.get("object_id").ok_or("Missing 'object_id'")?,
                "energy_deposit": args.get("energy_deposit").ok_or("Missing 'energy_deposit'")?,
            });
            let data = ctx.post_json("/api/tx/resurrect", &body).await?;
            format_text_result(&data)
        }
        "request_faucet" => {
            let body = json!({
                "address": args.get("address").ok_or("Missing 'address'")?,
            });
            let data = ctx.post_json("/api/faucet", &body).await?;
            format_text_result(&data)
        }
        "get_autopoietic_health" => {
            let data = ctx.get_json("/api/autopoietic/health").await?;
            format_text_result(&data)
        }
        "compute_demurrage" => {
            let body = json!({
                "balance": args.get("balance").ok_or("Missing 'balance'")?,
                "last_touched_epoch": args.get("last_touched_epoch").ok_or("Missing 'last_touched_epoch'")?,
                "current_epoch": args.get("current_epoch").ok_or("Missing 'current_epoch'")?,
                "lambda_base_ppm": args.get("lambda_base_ppm").unwrap_or(&json!(1)),
                "threshold": args.get("threshold").unwrap_or(&json!(1024)),
            });
            let data = ctx.post_json("/api/demurrage/owed", &body).await?;
            format_text_result(&data)
        }
        "get_epv_status" => {
            let data = ctx.get_json("/api/epv/status").await?;
            format_text_result(&data)
        }
        "get_fee_status" => {
            let data = ctx.get_json("/api/fee_controller/status").await?;
            format_text_result(&data)
        }
        "get_consensus_phase" => {
            let data = ctx.get_json("/api/consensus/phase").await?;
            format_text_result(&data)
        }
        "check_annealing_temperature" => {
            let body = json!({
                "lambda_half_life": args.get("lambda_half_life").ok_or("Missing 'lambda_half_life'")?,
                "beta_mb": 1000u64,
                "epoch": args.get("epoch").ok_or("Missing 'epoch'")?,
            });
            let data = ctx.post_json("/api/annealing/temperature", &body).await?;
            format_text_result(&data)
        }
        "compute_mera_commitment" => {
            let body = json!({
                "energies": args.get("energies").ok_or("Missing 'energies'")?,
                "lambda_half_life": args.get("lambda_half_life").unwrap_or(&json!(4096)),
                "base_half_life": args.get("base_half_life").unwrap_or(&json!(100)),
            });
            let data = ctx.post_json("/api/mera/commit", &body).await?;
            format_text_result(&data)
        }
        "prove_fork_evaporated" => {
            let body = json!({
                "fork_root_hex": args.get("fork_root_hex").ok_or("Missing 'fork_root_hex'")?,
                "blocks": args.get("blocks").ok_or("Missing 'blocks'")?,
                "evaluated_at_epoch": args.get("evaluated_at_epoch").ok_or("Missing 'evaluated_at_epoch'")?,
                "threshold": args.get("threshold").ok_or("Missing 'threshold'")?,
                "lambda_epochs": args.get("lambda_epochs").ok_or("Missing 'lambda_epochs'")?,
            });
            let data = ctx.post_json("/api/fork_cert/prove", &body).await?;
            format_text_result(&data)
        }
        "get_oracle_status" => {
            let data = ctx.get_json("/api/oracle/status").await?;
            format_text_result(&data)
        }
        "get_shard_health" => {
            let data = ctx.get_json("/api/shards/health").await?;
            format_text_result(&data)
        }
        "check_conservation" => {
            let body = json!({
                "before": {
                    "accounts":    args.get("before_accounts").and_then(|v| v.as_u64()).unwrap_or(0),
                    "stake":       args.get("before_stake").and_then(|v| v.as_u64()).unwrap_or(0),
                    "refresh_pool": args.get("before_refresh_pool").and_then(|v| v.as_u64()).unwrap_or(0),
                    "slashed_pool": args.get("before_slashed_pool").and_then(|v| v.as_u64()).unwrap_or(0),
                },
                "after": {
                    "accounts":    args.get("after_accounts").and_then(|v| v.as_u64()).unwrap_or(0),
                    "stake":       args.get("after_stake").and_then(|v| v.as_u64()).unwrap_or(0),
                    "refresh_pool": args.get("after_refresh_pool").and_then(|v| v.as_u64()).unwrap_or(0),
                    "slashed_pool": args.get("after_slashed_pool").and_then(|v| v.as_u64()).unwrap_or(0),
                },
                "epochs_elapsed": args.get("epochs_elapsed").and_then(|v| v.as_u64()).unwrap_or(1),
                "half_life_epochs": args.get("half_life_epochs").and_then(|v| v.as_u64()).unwrap_or(4096),
            });
            let data = ctx
                .post_json("/api/energy_kernel/conservation_check", &body)
                .await?;
            format_text_result(&data)
        }
        _ => Err(format!("Unknown tool: {name}"))?,
    };

    Ok(result)
}

fn format_text_result(data: &Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools_returns_26_tools() {
        let tools = list_tools();
        let tool_list = tools["tools"].as_array().unwrap();
        assert_eq!(tool_list.len(), 26);
    }

    #[test]
    fn test_all_tools_have_required_fields() {
        let tools = list_tools();
        for tool in tools["tools"].as_array().unwrap() {
            assert!(tool["name"].is_string(), "tool missing name");
            assert!(tool["description"].is_string(), "tool missing description");
            assert!(tool["inputSchema"].is_object(), "tool missing inputSchema");
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn test_tool_names_unique() {
        let tools = list_tools();
        let names: Vec<&str> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len(), "duplicate tool names");
    }

    #[test]
    fn test_read_tools_have_no_required_params_except_getters() {
        let tools = list_tools();
        let no_required = [
            "get_chain_status",
            "list_objects",
            "list_accounts",
            "list_ghosts",
            "get_recent_blocks",
            "get_recent_events",
            "list_contracts",
            "get_stats",
        ];
        for tool in tools["tools"].as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            if no_required.contains(&name) {
                let required = tool["inputSchema"]["required"].as_array().unwrap();
                // These may have optional params but no required ones
                // (get_recent_blocks has optional "limit")
                assert!(
                    required.is_empty()
                        || name == "get_recent_blocks"
                        || name == "get_recent_events",
                    "{name} should have no required params"
                );
            }
        }
    }

    #[test]
    fn test_write_tools_have_required_params() {
        let tools = list_tools();
        let write_tools = [
            "transfer",
            "create_object",
            "refresh_object",
            "resurrect_object",
            "request_faucet",
        ];
        for tool in tools["tools"].as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            if write_tools.contains(&name) {
                let required = tool["inputSchema"]["required"].as_array().unwrap();
                assert!(!required.is_empty(), "{name} should have required params");
            }
        }
    }

    #[test]
    fn test_get_object_requires_id() {
        let tools = list_tools();
        let get_obj = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "get_object")
            .unwrap();
        let required = get_obj["inputSchema"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("id")));
    }

    #[test]
    fn test_transfer_requires_from_to_amount() {
        let tools = list_tools();
        let transfer = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "transfer")
            .unwrap();
        let required = transfer["inputSchema"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("from")));
        assert!(required.contains(&json!("to")));
        assert!(required.contains(&json!("amount")));
    }

    #[test]
    fn test_format_text_result() {
        let data = json!({"block_height": 42});
        let result = format_text_result(&data);
        assert_eq!(result["content"][0]["type"], "text");
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("42"));
    }
}
