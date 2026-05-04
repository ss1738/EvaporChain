//! 6 MCP Prompts — guided workflows for AI agents interacting with EvaporChain.

use serde_json::{json, Value};

use crate::protocol::Context;

/// Return the list of all 6 prompts.
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
            },
            {
                "name": "oracle_data_analysis",
                "description": "Analyze all live oracle feeds — check for stale prices, outlier values, consensus confidence, and cross-feed correlations. Flags any feed that may require operator attention.",
                "arguments": [
                    {
                        "name": "staleness_threshold_secs",
                        "description": "How many seconds before a feed is considered stale (default: 300)",
                        "required": false
                    }
                ]
            },
            {
                "name": "consensus_phase_investigation",
                "description": "Deep-dive into the consensus substrate — RG Phase Map regime, WSBF renormalized λ_eff, Boltzmann proposer weight distribution, and self-annealing temperature. Diagnoses whether the chain is safe, live, or at risk of phase transition.",
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
        "oracle_data_analysis" => get_oracle_data_analysis(ctx, &args).await,
        "consensus_phase_investigation" => get_consensus_phase_investigation(ctx).await,
        _ => Err(format!("Unknown prompt: {name}")),
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn test_list_prompts_returns_6() {
        let prompts = list_prompts();
        let list = prompts["prompts"].as_array().unwrap();
        assert_eq!(list.len(), 6);
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
        assert!(names.contains(&"oracle_data_analysis"));
        assert!(names.contains(&"consensus_phase_investigation"));
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

    #[test]
    fn test_oracle_data_analysis_has_one_optional_arg() {
        let prompts = list_prompts();
        let oda = prompts["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "oracle_data_analysis")
            .unwrap();
        let args = oda["arguments"].as_array().unwrap();
        assert_eq!(args.len(), 1);
        assert_eq!(args[0]["name"], "staleness_threshold_secs");
        assert_eq!(args[0]["required"], false);
    }

    #[test]
    fn test_zero_arg_prompts_list_empty_arguments() {
        let prompts = list_prompts();
        let zero_arg = ["explore_chain", "chain_health_report", "viability_audit", "consensus_phase_investigation"];
        for name in zero_arg {
            let p = prompts["prompts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .unwrap_or_else(|| panic!("missing prompt {name}"));
            let args = p["arguments"].as_array().unwrap();
            assert_eq!(args.len(), 0, "prompt {name} should have no arguments");
        }
    }

    #[test]
    fn test_get_prompt_missing_name_errors() {
        // Synchronous dispatch path doesn't need a real Context — bad
        // params return Err before we ever touch the chain. Drive it
        // through a tiny tokio runtime so we can assert.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ctx = Context {
            node_url: "http://127.0.0.1:1".to_string(),
            client: reqwest::Client::new(),
        };
        let err = rt
            .block_on(get_prompt(&ctx, &json!({})))
            .expect_err("should error on missing name");
        assert!(err.contains("Missing 'name'"), "{err}");
    }

    #[test]
    fn test_get_prompt_unknown_name_errors() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ctx = Context {
            node_url: "http://127.0.0.1:1".to_string(),
            client: reqwest::Client::new(),
        };
        let err = rt
            .block_on(get_prompt(&ctx, &json!({"name": "nonexistent_prompt_xyz"})))
            .expect_err("should error on unknown name");
        assert!(err.starts_with("Unknown prompt:"), "{err}");
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
    let energy = args.get("energy").and_then(|v| v.as_u64()).unwrap_or(100);
    let half_life = args.get("half_life").and_then(|v| v.as_u64()).unwrap_or(5);

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
    let autopoietic = ctx
        .get_json("/api/autopoietic/health")
        .await
        .unwrap_or(json!({"error": "unavailable"}));
    let consensus_phase = ctx
        .get_json("/api/consensus/phase")
        .await
        .unwrap_or(json!({"error": "unavailable"}));
    let fee_status = ctx
        .get_json("/api/fee_controller/status")
        .await
        .unwrap_or(json!({"error": "unavailable"}));
    let epv_status = ctx
        .get_json("/api/epv/status")
        .await
        .unwrap_or(json!({"error": "unavailable"}));
    let sentinel = ctx
        .get_json("/api/sentinel/status")
        .await
        .unwrap_or(json!({"error": "unavailable"}));

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

async fn get_oracle_data_analysis(ctx: &Context, args: &Value) -> Result<Value, String> {
    let staleness = args
        .get("staleness_threshold_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(300);

    let oracle_status = ctx
        .get_json("/api/oracle/status")
        .await
        .unwrap_or(json!({"error": "unavailable"}));
    let status = ctx.get_json("/api/status").await.unwrap_or(json!({}));

    let oracle_str = serde_json::to_string_pretty(&oracle_status).unwrap_or_default();
    let status_str = serde_json::to_string_pretty(&status).unwrap_or_default();

    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Analyze the oracle feed data for the EvaporChain network.\n\n\
                        EvaporChain uses a multi-source oracle with signed reports and consensus confidence scoring.\n\
                        Stale threshold: {staleness}s\n\n\
                        ## Chain Status\n```json\n{status_str}\n```\n\n\
                        ## Oracle Feed Status\n```json\n{oracle_str}\n```\n\n\
                        Please analyze the oracle data and produce a report covering:\n\
                        1. **Feed Inventory**: How many feeds are active? What assets/sources?\n\
                        2. **Staleness Check**: Which feeds have not been updated within {staleness}s? Are any critically stale?\n\
                        3. **Confidence Scores**: Which feeds have low consensus confidence (< 0.7)? What might cause this?\n\
                        4. **Outlier Detection**: Are any feed values statistical outliers vs peer feeds for the same asset?\n\
                        5. **Cross-feed Correlation**: Do correlated assets (e.g. ETH/USD vs ETH/BTC) move consistently?\n\
                        6. **Operator Actions**: Which feeds need immediate attention? Recommend remediation steps.\n\
                        7. **Health Verdict**: Overall oracle subsystem health — OK | Degraded | Critical."
                    )
                }
            }
        ]
    }))
}

async fn get_consensus_phase_investigation(ctx: &Context) -> Result<Value, String> {
    let consensus_phase = ctx
        .get_json("/api/consensus/phase")
        .await
        .unwrap_or(json!({"error": "unavailable"}));
    let autopoietic = ctx
        .get_json("/api/autopoietic/health")
        .await
        .unwrap_or(json!({"error": "unavailable"}));
    let validators = ctx
        .get_json("/api/validators")
        .await
        .unwrap_or(json!({"error": "unavailable"}));
    let status = ctx.get_json("/api/status").await.unwrap_or(json!({}));

    let cp_str = serde_json::to_string_pretty(&consensus_phase).unwrap_or_default();
    let ap_str = serde_json::to_string_pretty(&autopoietic).unwrap_or_default();
    let val_str = serde_json::to_string_pretty(&validators).unwrap_or_default();
    let status_str = serde_json::to_string_pretty(&status).unwrap_or_default();

    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "Investigate the consensus substrate of the EvaporChain network.\n\n\
                        EvaporChain consensus is built on three interacting layers:\n\
                        - **WSBF** (Weighted Scale-free Block Flow): renormalizes λ_eff from recent block history\n\
                        - **RG Phase Map**: classifies the regime as Chaotic | Frozen | SafetyStable | LivenessStable\n\
                        - **Boltzmann Stake**: weights validators by stake × activity via a thermodynamic scoring function\n\
                        - **Self-Annealing**: gradually tightens validator selection as the chain matures\n\n\
                        ## Chain Status\n```json\n{status_str}\n```\n\n\
                        ## Consensus Phase (WSBF + RG)\n```json\n{cp_str}\n```\n\n\
                        ## Autopoietic Health\n```json\n{ap_str}\n```\n\n\
                        ## Validator Set\n```json\n{val_str}\n```\n\n\
                        Please produce a consensus investigation report covering:\n\
                        1. **Current Phase**: What is the RG regime? Is it stable or at risk of transition?\n\
                        2. **λ_eff Analysis**: What does the renormalized decay parameter tell us about chain health?\n\
                        3. **Energy Density & Entropy**: Are block energy levels trending up or down?\n\
                        4. **Adversary Estimate**: Based on fork history and voting patterns, what is the inferred adversary fraction?\n\
                        5. **Validator Distribution**: Is stake well-distributed (Nakamoto coefficient)? Any dangerous concentration?\n\
                        6. **Phase Transition Risk**: How close is the chain to crossing into Frozen or Chaotic?\n\
                        7. **Annealing Temperature**: Is the chain in early (high-T, exploratory) or mature (low-T, conservative) regime?\n\
                        8. **Recommendations**: Any validator set or governance actions needed to stabilize the phase?"
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
