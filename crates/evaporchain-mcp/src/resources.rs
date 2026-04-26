//! 7 MCP Resources — live blockchain data AI agents can read.

use serde_json::{json, Value};

use crate::protocol::Context;

/// Return the list of all 7 resources.
pub fn list_resources() -> Value {
    json!({
        "resources": [
            {
                "uri": "evaporchain://status",
                "name": "Chain Status",
                "description": "Live blockchain status — block height, epoch, object counts, peer count, state root, uptime",
                "mimeType": "application/json"
            },
            {
                "uri": "evaporchain://objects",
                "name": "Active Objects",
                "description": "All active state objects with their energy levels, decay percentages, half-lives, and owners",
                "mimeType": "application/json"
            },
            {
                "uri": "evaporchain://ghosts",
                "name": "Ghost Objects",
                "description": "All evaporated objects — state that has decayed to zero. Only nullifier proofs remain.",
                "mimeType": "application/json"
            },
            {
                "uri": "evaporchain://accounts",
                "name": "Accounts",
                "description": "All accounts with balances and nonces",
                "mimeType": "application/json"
            },
            {
                "uri": "evaporchain://blocks",
                "name": "Recent Blocks",
                "description": "Last 50 blocks with transactions, evaporations, and state transitions",
                "mimeType": "application/json"
            },
            {
                "uri": "evaporchain://events",
                "name": "Live Events",
                "description": "Recent chain events — evaporations, creations, grace transitions, transfers",
                "mimeType": "application/json"
            },
            {
                "uri": "evaporchain://stats",
                "name": "Chain Statistics",
                "description": "Aggregate statistics — total created, evaporated, resurrected, refreshed, average lifetime",
                "mimeType": "application/json"
            }
        ]
    })
}

/// Read a resource by URI.
pub async fn read_resource(ctx: &Context, params: &Value) -> Result<Value, String> {
    let uri = params
        .get("uri")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'uri' parameter")?;

    let (api_path, resource_name) = match uri {
        "evaporchain://status" => ("/api/status", "Chain Status"),
        "evaporchain://objects" => ("/api/objects", "Active Objects"),
        "evaporchain://ghosts" => ("/api/ghosts", "Ghost Objects"),
        "evaporchain://accounts" => ("/api/accounts", "Accounts"),
        "evaporchain://blocks" => ("/api/blocks?limit=50", "Recent Blocks"),
        "evaporchain://events" => ("/api/events?limit=50", "Live Events"),
        "evaporchain://stats" => ("/api/stats/summary", "Chain Statistics"),
        _ => return Err(format!("Unknown resource URI: {uri}")),
    };

    let data = ctx.get_json(api_path).await?;
    let text = serde_json::to_string_pretty(&data)
        .unwrap_or_else(|_| data.to_string());

    Ok(json!({
        "contents": [{
            "uri": uri,
            "name": resource_name,
            "mimeType": "application/json",
            "text": text
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_resources_returns_7() {
        let resources = list_resources();
        let list = resources["resources"].as_array().unwrap();
        assert_eq!(list.len(), 7);
    }

    #[test]
    fn test_all_resources_have_required_fields() {
        let resources = list_resources();
        for r in resources["resources"].as_array().unwrap() {
            assert!(r["uri"].is_string(), "resource missing uri");
            assert!(r["name"].is_string(), "resource missing name");
            assert!(r["description"].is_string(), "resource missing description");
            assert_eq!(r["mimeType"], "application/json");
        }
    }

    #[test]
    fn test_resource_uris_use_evaporchain_scheme() {
        let resources = list_resources();
        for r in resources["resources"].as_array().unwrap() {
            let uri = r["uri"].as_str().unwrap();
            assert!(
                uri.starts_with("evaporchain://"),
                "URI should use evaporchain:// scheme: {uri}"
            );
        }
    }

    #[test]
    fn test_resource_uris_unique() {
        let resources = list_resources();
        let uris: Vec<&str> = resources["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect();
        let mut deduped = uris.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(uris.len(), deduped.len(), "duplicate resource URIs");
    }
}
