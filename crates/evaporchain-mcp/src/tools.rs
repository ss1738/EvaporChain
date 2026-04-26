//! 15 MCP Tools — actions AI agents can take on the EvaporChain blockchain.

use serde_json::{json, Value};

use crate::protocol::Context;

/// Return the list of all 15 tools.
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
            let data = ctx
                .get_json(&format!("/api/blocks?limit={limit}"))
                .await?;
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
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(30);
            let data = ctx
                .get_json(&format!("/api/events?limit={limit}"))
                .await?;
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
    fn test_list_tools_returns_15_tools() {
        let tools = list_tools();
        let tool_list = tools["tools"].as_array().unwrap();
        assert_eq!(tool_list.len(), 15);
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
        let no_required = ["get_chain_status", "list_objects", "list_accounts",
                          "list_ghosts", "get_recent_blocks", "get_recent_events",
                          "list_contracts", "get_stats"];
        for tool in tools["tools"].as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            if no_required.contains(&name) {
                let required = tool["inputSchema"]["required"].as_array().unwrap();
                // These may have optional params but no required ones
                // (get_recent_blocks has optional "limit")
                assert!(
                    required.is_empty() || name == "get_recent_blocks" || name == "get_recent_events",
                    "{name} should have no required params"
                );
            }
        }
    }

    #[test]
    fn test_write_tools_have_required_params() {
        let tools = list_tools();
        let write_tools = ["transfer", "create_object", "refresh_object",
                          "resurrect_object", "request_faucet"];
        for tool in tools["tools"].as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            if write_tools.contains(&name) {
                let required = tool["inputSchema"]["required"].as_array().unwrap();
                assert!(
                    !required.is_empty(),
                    "{name} should have required params"
                );
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
