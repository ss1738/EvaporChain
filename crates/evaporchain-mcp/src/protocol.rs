//! MCP JSON-RPC protocol handler.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{prompts, resources, tools};

/// Shared context for all handlers.
pub struct Context {
    pub node_url: String,
    pub client: reqwest::Client,
}

impl Context {
    pub fn api_url(&self, path: &str) -> String {
        format!("{}{}", self.node_url, path)
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, String> {
        self.client
            .get(self.api_url(path))
            .send()
            .await
            .map_err(|e| format!("HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("JSON parse error: {e}"))
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.client
            .post(self.api_url(path))
            .json(body)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("JSON parse error: {e}"))
    }
}

// ── JSON-RPC types ──

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
pub(crate) struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

fn ok_response(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

fn err_response(id: Value, code: i32, msg: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: msg.to_string(),
            data: None,
        }),
    }
}

/// Handle a single JSON-RPC message, return the response (or None for notifications).
pub async fn handle_message(ctx: &Context, raw: &str) -> Option<JsonRpcResponse> {
    let req: JsonRpcRequest = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            return Some(err_response(
                Value::Null,
                -32700,
                &format!("Parse error: {e}"),
            ));
        }
    };

    let _ = req.jsonrpc; // consumed for validation

    let id = req.id.clone().unwrap_or(Value::Null);
    let params = req.params.unwrap_or(Value::Null);

    let result = match req.method.as_str() {
        // ── Lifecycle ──
        "initialize" => handle_initialize(&params),
        "notifications/initialized" => return None,
        "ping" => Ok(json!({})),

        // ── Tools ──
        "tools/list" => Ok(tools::list_tools()),
        "tools/call" => tools::call_tool(ctx, &params).await,

        // ── Resources ──
        "resources/list" => Ok(resources::list_resources()),
        "resources/read" => resources::read_resource(ctx, &params).await,

        // ── Prompts ──
        "prompts/list" => Ok(prompts::list_prompts()),
        "prompts/get" => prompts::get_prompt(ctx, &params).await,

        _ => Err(format!("Method not found: {}", req.method)),
    };

    Some(match result {
        Ok(val) => ok_response(id, val),
        Err(msg) => err_response(id, -32603, &msg),
    })
}

fn handle_initialize(_params: &Value) -> Result<Value, String> {
    Ok(json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {},
            "resources": {},
            "prompts": {}
        },
        "serverInfo": {
            "name": "evaporchain-mcp",
            "version": "0.1.0",
            "description": "MCP server for EvaporChain — the first thermodynamic blockchain with native AI agent support"
        }
    }))
}
