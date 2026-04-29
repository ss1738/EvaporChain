//! 4 MCP Prompts — guided workflows for AI agents interacting with EvaporChain.

use serde_json::{json, Value};

use crate::protocol::Context;

/// Return the list of all 4 prompts.
pub fn list_prompts() -> Value {
    json!({
        "prompts": [
            {
                "name": "explore_chain",
                "description": "Explore the EvaporChain blockchain — check status, view objects, see what's evaporating, and understand the thermodynamic state decay in action.",
                "arguments": []
            },
            {
                "name": "create_and_watch",
                "description": "Create a new state object with custom energy and half-life, then monitor it as it decays toward evaporation. A hands-on demo of thermodynamic state.",
                "arguments": [
                    {
                        "name": "energy",
                        "description": "Initial energy for the object (e.g. 100)",
                        "required": false
                    },
                    {
                        "name": "half_life",
                        "description": "Epochs for energy to halve (e.g. 5)",
                        "required": false
                    }
                ]
            },
            {
                "name": "chain_health_report",
                "description": "Generate a comprehensive health report of the EvaporChain testnet — block production rate, object lifecycle metrics, evaporation trends, and network status.",
                "arguments": []
            },
            {
                "name": "viability_audit",
                "description": "Audit the chain's autopoietic viability — checks all three self-sustaining subsystems (Patronage, Sentinel, LLSA), the RG consensus phase, and fee controller drift. Outputs a structured viability verdict with recommendations.",
                "arguments": []
            }
        ]
    })
}

/// Get a prompt by name with its messages.
pub async fn get_prompt(ctx: &Context, params: &Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'name' parameter")?;

    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    match name {
        "explore_chain" => get_explore_chain(ctx).await,
        "create_and_watch" => get_create_and_watch(ctx, &args).await,
        "chain_health_report" => get_chain_health_report(ctx).await,
        "viability_audit" => get_viability_audit(ctx).await,
        _ => Err(format!("Unknown prompt: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_prompts_returns_4() {
        let prompts = list_prompts();
        let list = prompts["prompts"].as_array().unwrap();
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn test_all_prompts_have_required_fields() {
        let prompts = list_prompts();
        for p in prompts["prompts"].as_array().unwrap() {
            assert!(p["name"].is_string(), "prompt missing name");
            assert!(p["description"].is_string(), "prompt missing description");
            assert!(p["arguments"].is_array(), "prompt missing arguments");
        }
    }

    #[test]
    fn test_prompt_names() {
        let prompts = list_prompts();
        let names: Vec<&str> = prompts["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"explore_chain"));
        assert!(names.contains(&"create_and_watch"));
        assert!(names.contains(&"chain_health_report"));
        assert!(names.contains(&"viability_audit"));
    }

    #[test]
    fn test_create_and_watch_has_optional_args() {
        let prompts = list_prompts();
        let cw = prompts["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "create_and_watch")
            .unwrap();
        let args = cw["arguments"].as_array().unwrap();
        assert_eq!(args.len(), 2);
        for arg in args {
            assert_eq!(arg["required"], false);
        }
    }
}

async fn get_explore_chain(ctx: &Context) -> Result<Value, String> {
    let status = ctx.get_json("/api/status").await?;
    let objects = ctx.get_json("/api/objects").await?;
    let events = ctx.get_json("/api/events?limit=10").await?;

    let status_str = serde_json::to_string_pretty(&status).unwrap_or_default();
    let objects_str = serde_json::to_string_pretty(&objects).unwrap_or_default();
    let events_str = serde_json::to_string_pretty(&events).unwrap_or_default();

    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "I'm exploring the EvaporChain testnet — the first thermodynamic blockchain where state decays over time.\n\n\
                        Here's the current chain status:\n```json\n{status_str}\n```\n\n\
                        Active state objects (each with decaying energy):\n```json\n{objects_str}\n```\n\n\
                        Recent events:\n```json\n{events_str}\n```\n\n\
                        Please analyze this data and tell me:\n\
                        1. What's the overall health of the chain?\n\
                        2. Which objects are close to evaporating?\n\
                        3. What interesting patterns do you see in the events?\n\
                        4. Any objects that should be refreshed to prevent data loss?"
                    )
                }
            }
        ]
    }))
}

async fn get_create_and_watch(ctx: &Context, args: &Value) -> Result<Value, String> {
    let energy = args
        .get("energy")
        .and_then(|v| v.as_u64())
        .unwrap_or(100);
    let half_life = args
        .get("half_life")
        .and_then(|v| v.as_u64())
        .unwrap_or(5);

    let status = ctx.get_json("/api/status").await?;
    let current_epoch = status.get("epoch").and_then(|v| v.as_u64()).unwrap_or(0);

    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Let's create a state object on EvaporChain and watch it decay!\n\n\
                        The chain is currently at epoch {current_epoch}.\n\n\
                        Please use the `create_object` tool to create an object with:\n\
                        - creator: 1 (Alice)\n\
                        - object_id: pick an unused ID between 100-200\n\
                        - energy: {energy}\n\
                        - half_life: {half_life}\n\n\
                        After creating it, use `get_object` to check its initial state.\n\
                        Then wait a moment and check again to see the energy decay.\n\n\
                        With energy={energy} and half_life={half_life}:\n\
                        - After {half_life} epochs, energy will be ~{half}\n\
                        - After {} epochs, energy will be ~{quarter}\n\
                        - The object will evaporate when energy hits 0\n\n\
                        Explain what you observe about thermodynamic state decay.",
                        half_life * 2,
                        half = energy / 2,
                        quarter = energy / 4
                    )
                }
            }
        ]
    }))
}

async fn get_viability_audit(ctx: &Context) -> Result<Value, String> {
    let autopoietic = ctx.get_json("/api/autopoietic/health").await.unwrap_or(json!({"error": "unavailable"}));
    let consensus_phase = ctx.get_json("/api/consensus/phase").await.unwrap_or(json!({"error": "unavailable"}));
    let fee_status = ctx.get_json("/api/fee_controller/status").await.unwrap_or(json!({"error": "unavailable"}));
    let epv_status = ctx.get_json("/api/epv/status").await.unwrap_or(json!({"error": "unavailable"}));
    let sentinel = ctx.get_json("/api/sentinel/status").await.unwrap_or(json!({"error": "unavailable"}));

    let ap_str = serde_json::to_string_pretty(&autopoietic).unwrap_or_default();
    let cp_str = serde_json::to_string_pretty(&consensus_phase).unwrap_or_default();
    let fee_str = serde_json::to_string_pretty(&fee_status).unwrap_or_default();
    let epv_str = serde_json::to_string_pretty(&epv_status).unwrap_or_default();
    let sent_str = serde_json::to_string_pretty(&sentinel).unwrap_or_default();

    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Audit the autopoietic viability of the EvaporChain network.\n\n\
                        EvaporChain is a thermodynamic blockchain that can legitimately die — \
                        it has a viability condition: all three self-sustaining subsystems must remain functional.\n\n\
                        ## Autopoietic Health (Maturana-Varela 1980)\n```json\n{ap_str}\n```\n\n\
                        ## Consensus Phase (RG Phase Map)\n```json\n{cp_str}\n```\n\n\
                        ## Fee Controller State\n```json\n{fee_str}\n```\n\n\
                        ## Protocol Version Set (EPV)\n```json\n{epv_str}\n```\n\n\
                        ## Sentinel Status\n```json\n{sent_str}\n```\n\n\
                        Please produce a structured viability audit covering:\n\
                        1. **Overall Verdict**: Viable | Stressed | Inviable — and what that means\n\
                        2. **Patronage subsystem**: Is the chain self-funding? Any covenant shortfalls?\n\
                        3. **Sentinel subsystem**: Is autonomic governance active? Last vote staleness?\n\
                        4. **LLSA subsystem**: Is the upgrade gate functional?\n\
                        5. **Consensus regime**: What phase is the chain in? Is it safe and live?\n\
                        6. **Fee trajectory**: Is the fee controller drifting toward extremes?\n\
                        7. **Protocol evolution**: Are old versions being pruned? Any version cliff?\n\
                        8. **Immediate recommendations**: What must be fixed in the next N epochs to prevent Inviable status?"
                    )
                }
            }
        ]
    }))
}

async fn get_chain_health_report(ctx: &Context) -> Result<Value, String> {
    let status = ctx.get_json("/api/status").await?;
    let stats = ctx.get_json("/api/stats/summary").await?;
    let objects = ctx.get_json("/api/objects").await?;
    let ghosts = ctx.get_json("/api/ghosts").await?;
    let events = ctx.get_json("/api/events?limit=50").await?;

    let status_str = serde_json::to_string_pretty(&status).unwrap_or_default();
    let stats_str = serde_json::to_string_pretty(&stats).unwrap_or_default();
    let objects_str = serde_json::to_string_pretty(&objects).unwrap_or_default();
    let ghosts_str = serde_json::to_string_pretty(&ghosts).unwrap_or_default();
    let events_str = serde_json::to_string_pretty(&events).unwrap_or_default();

    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Generate a comprehensive health report for the EvaporChain testnet.\n\n\
                        ## Chain Status\n```json\n{status_str}\n```\n\n\
                        ## Aggregate Statistics\n```json\n{stats_str}\n```\n\n\
                        ## Active Objects\n```json\n{objects_str}\n```\n\n\
                        ## Ghost Objects (Evaporated)\n```json\n{ghosts_str}\n```\n\n\
                        ## Recent Events (last 50)\n```json\n{events_str}\n```\n\n\
                        Please produce a structured health report covering:\n\
                        1. **Block Production**: Rate, consistency, any gaps\n\
                        2. **State Lifecycle**: Active → Grace → Ghost transition rate\n\
                        3. **Evaporation Metrics**: How many objects evaporated, average lifetime\n\
                        4. **Energy Distribution**: Which objects have the most/least energy\n\
                        5. **Network Health**: Peer count, uptime, status\n\
                        6. **Recommendations**: Objects to refresh, potential issues"
                    )
                }
            }
        ]
    }))
}
