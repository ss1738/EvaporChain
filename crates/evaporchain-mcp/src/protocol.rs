//! MCP JSON-RPC protocol handler.
//!
//! ## Security posture (CRITICAL-2 partial fix, 2026-05-06)
//!
//! Per `AUDIT_2026_05_06.md` CRITICAL-2, this MCP server was previously
//! a zero-auth attack surface for AI agents:
//!   - hardcoded backend URL `http://37.27.1.1:8080` with no token
//!   - 26 tools forwarding agent JSON to chain backend without auth
//!   - any prompt-injected agent could mutate chain state at will
//!
//! Mitigations now in place:
//!   1. Optional [`Context::api_token`] populated from
//!      `EVAPORCHAIN_MCP_API_TOKEN` env var. When set, every outgoing
//!      HTTP request adds an `Authorization: Bearer <token>` header.
//!   2. `EVAPORCHAIN_MCP_REQUIRE_AUTH=true` env var causes the MCP
//!      binary to refuse startup if the token is unset (fail-fast for
//!      production deployments where unauth-by-default is unacceptable).
//!   3. Critical write-tools (`transfer`, `mint`, `burn`, `*_apply`)
//!      log a stderr warning per call so operators can audit usage.
//!
//! Still TODO (deferred to a follow-up commit):
//!   - per-tool input validation (range checks on amounts, address
//!     format, schema enforcement)
//!   - per-tool rate limiting
//!   - audit log of every tool invocation with caller identification
//!   - backend-side enforcement of the bearer token (currently
//!     advisory; the node API does not yet check the header)

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{prompts, resources, tools};

/// Shared context for all handlers.
pub struct Context {
    pub node_url: String,
    pub client: reqwest::Client,
    /// Optional bearer token sent with every backend request.
    /// Loaded from `EVAPORCHAIN_MCP_API_TOKEN` env var in `main.rs`.
    /// `None` = no auth header (legacy behaviour, dev-only).
    pub api_token: Option<String>,
}

impl Context {
    pub fn api_url(&self, path: &str) -> String {
        format!("{}{}", self.node_url, path)
    }

    /// Build a `RequestBuilder` with the `Authorization: Bearer <token>`
    /// header attached if a token is configured. All outgoing HTTP
    /// requests must go through this helper.
    fn authed(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_token {
            Some(t) => builder.bearer_auth(t),
            None => builder,
        }
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, String> {
        self.authed(self.client.get(self.api_url(path)))
            .send()
            .await
            .map_err(|e| format!("HTTP error: {e}"))?
            .json::<Value>()
            .await
            .map_err(|e| format!("JSON parse error: {e}"))
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.authed(self.client.post(self.api_url(path)).json(body))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> Context {
        Context {
            node_url: "http://127.0.0.1:9999".into(),
            client: reqwest::Client::new(),
            api_token: None,
        }
    }

    fn test_ctx_with_token(token: &str) -> Context {
        Context {
            node_url: "http://127.0.0.1:9999".into(),
            client: reqwest::Client::new(),
            api_token: Some(token.into()),
        }
    }

    /// CRITICAL-2 hardening — Context.authed() must inject Authorization
    /// header when api_token is set, and not when it's None. This test
    /// exercises the builder shape rather than an end-to-end request
    /// (no live backend in unit tests).
    #[tokio::test]
    async fn test_authed_no_token_omits_header() {
        let ctx = test_ctx();
        let req = ctx
            .authed(ctx.client.get(ctx.api_url("/api/status")))
            .build()
            .unwrap();
        assert!(
            req.headers().get(reqwest::header::AUTHORIZATION).is_none(),
            "no token configured -> no Authorization header"
        );
    }

    #[tokio::test]
    async fn test_authed_with_token_adds_bearer_header() {
        let ctx = test_ctx_with_token("test-token-12345");
        let req = ctx
            .authed(ctx.client.get(ctx.api_url("/api/status")))
            .build()
            .unwrap();
        let auth = req
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .expect("token configured -> Authorization header present");
        assert_eq!(auth.to_str().unwrap(), "Bearer test-token-12345");
    }

    #[tokio::test]
    async fn test_initialize() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = handle_message(&ctx, msg).await.unwrap();
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["capabilities"]["resources"].is_object());
        assert!(result["capabilities"]["prompts"].is_object());
        assert_eq!(result["serverInfo"]["name"], "evaporchain-mcp");
    }

    #[tokio::test]
    async fn test_ping() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#;
        let resp = handle_message(&ctx, msg).await.unwrap();
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_tools_list() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#;
        let resp = handle_message(&ctx, msg).await.unwrap();
        let result = resp.result.unwrap();
        // Lower bound rather than exact count so adding new tools doesn't
        // immediately break this test. Original tools surface had 15 entries.
        assert!(result["tools"].as_array().unwrap().len() >= 15);
    }

    #[tokio::test]
    async fn test_resources_list() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#;
        let resp = handle_message(&ctx, msg).await.unwrap();
        let result = resp.result.unwrap();
        assert!(result["resources"].as_array().unwrap().len() >= 7);
    }

    #[tokio::test]
    async fn test_prompts_list() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","id":5,"method":"prompts/list"}"#;
        let resp = handle_message(&ctx, msg).await.unwrap();
        let result = resp.result.unwrap();
        assert!(result["prompts"].as_array().unwrap().len() >= 3);
    }

    #[tokio::test]
    async fn test_unknown_method_returns_error() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","id":6,"method":"nonexistent/method"}"#;
        let resp = handle_message(&ctx, msg).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32603);
    }

    #[tokio::test]
    async fn test_invalid_json_returns_parse_error() {
        let ctx = test_ctx();
        let resp = handle_message(&ctx, "not json").await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32700);
    }

    #[tokio::test]
    async fn test_notification_returns_none() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let resp = handle_message(&ctx, msg).await;
        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn test_response_has_correct_id() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","id":42,"method":"ping"}"#;
        let resp = handle_message(&ctx, msg).await.unwrap();
        assert_eq!(resp.id, json!(42));
    }

    #[tokio::test]
    async fn test_null_id_handling() {
        let ctx = test_ctx();
        let msg = r#"{"jsonrpc":"2.0","method":"ping"}"#;
        let resp = handle_message(&ctx, msg).await.unwrap();
        assert_eq!(resp.id, Value::Null);
    }
}
