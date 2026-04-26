//! JSON-RPC 2.0 endpoint for ethers.js / web3.py compatibility.
//!
//! All methods use the `evap_` namespace. Standard `net_*` methods are also
//! supported for tooling that probes the network layer.

use axum::{extract::State, Json};
use evaporchain_state::db::StateDB;
use evaporchain_types::Transaction;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::api::ApiState;

fn safe_lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

// ──────────────────────────── Wire Types ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub id: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0", result: Some(result), error: None, id }
    }

    fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(JsonRpcError { code, message: message.into(), data: None }),
            id,
        }
    }

    fn method_not_found(id: Value, method: &str) -> Self {
        Self::err(id, -32601, format!("Method not found: {}", method))
    }

    fn invalid_params(id: Value, msg: impl Into<String>) -> Self {
        Self::err(id, -32602, msg)
    }
}

// ──────────────────────────── Handler ────────────────────────────────

pub async fn handle_jsonrpc(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<Value>,
) -> Json<Value> {
    if let Some(arr) = req.as_array() {
        let mut results = Vec::with_capacity(arr.len());
        for item in arr {
            if let Ok(r) = serde_json::from_value::<JsonRpcRequest>(item.clone()) {
                results.push(dispatch(&state, r));
            } else {
                results.push(JsonRpcResponse::err(
                    Value::Null,
                    -32600,
                    "Invalid Request",
                ));
            }
        }
        Json(serde_json::to_value(results).unwrap_or(Value::Null))
    } else if let Ok(r) = serde_json::from_value::<JsonRpcRequest>(req) {
        let resp = dispatch(&state, r);
        Json(serde_json::to_value(resp).unwrap_or(Value::Null))
    } else {
        Json(serde_json::to_value(JsonRpcResponse::err(
            Value::Null,
            -32700,
            "Parse error",
        )).unwrap_or(Value::Null))
    }
}

fn dispatch(state: &ApiState, req: JsonRpcRequest) -> JsonRpcResponse {
    match req.method.as_str() {
        "evap_chainId" => rpc_chain_id(state, req.id),
        "evap_blockNumber" => rpc_block_number(state, req.id),
        "evap_gasPrice" => rpc_gas_price(state, req.id),
        "evap_getBalance" => rpc_get_balance(state, &req.params, req.id),
        "evap_getTransactionCount" => rpc_get_tx_count(state, &req.params, req.id),
        "evap_getAccountInfo" => rpc_get_account_info(state, &req.params, req.id),
        "evap_getBlockByNumber" => rpc_get_block_by_number(state, &req.params, req.id),
        "evap_getTransactionReceipt" => rpc_get_tx_receipt(state, &req.params, req.id),
        "evap_sendRawTransaction" => rpc_send_raw_tx(state, &req.params, req.id),
        "evap_estimateGas" => rpc_estimate_gas(state, &req.params, req.id),
        "evap_getObject" => rpc_get_object(state, &req.params, req.id),
        "evap_mempoolSize" => rpc_mempool_size(state, req.id),
        "evap_getLogs" => rpc_get_logs(state, &req.params, req.id),
        "evap_getBlockLogs" => rpc_get_block_logs(state, &req.params, req.id),
        "evap_getFinalityStatus" => rpc_get_finality_status(state, &req.params, req.id),
        "evap_latestFinalizedBlock" => rpc_latest_finalized(state, req.id),
        "net_version" => rpc_net_version(state, req.id),
        "net_peerCount" => rpc_peer_count(state, req.id),
        "net_listening" => JsonRpcResponse::ok(req.id, Value::Bool(true)),
        _ => JsonRpcResponse::method_not_found(req.id, &req.method),
    }
}

// ──────────────────────────── Methods ────────────────────────────────

fn rpc_chain_id(state: &ApiState, id: Value) -> JsonRpcResponse {
    let chain_id = if let Some(ref tc) = state.tendermint {
        let c = safe_lock(tc);
        c.chain_id().to_string()
    } else {
        "evaporchain-devnet".to_string()
    };
    JsonRpcResponse::ok(id, Value::String(chain_id))
}

fn rpc_block_number(state: &ApiState, id: Value) -> JsonRpcResponse {
    let height = {
        let c = safe_lock(&state.consensus);
        c.block_number()
    };
    JsonRpcResponse::ok(id, json_hex_u64(height))
}

fn rpc_gas_price(state: &ApiState, id: Value) -> JsonRpcResponse {
    let base_fee = latest_base_fee(state);
    JsonRpcResponse::ok(id, json_hex_u64(base_fee))
}

fn rpc_get_balance(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let addr = match parse_address_param(params, 0) {
        Ok(a) => a,
        Err(e) => return e(id),
    };
    let db = safe_lock(&state.db);
    let balance = db.get_account(&addr).map(|a| a.balance).unwrap_or(0);
    JsonRpcResponse::ok(id, json_hex_u64(balance))
}

fn rpc_get_tx_count(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let addr = match parse_address_param(params, 0) {
        Ok(a) => a,
        Err(e) => return e(id),
    };
    let db = safe_lock(&state.db);
    let nonce = db.get_account(&addr).map(|a| a.nonce).unwrap_or(0);
    JsonRpcResponse::ok(id, json_hex_u64(nonce))
}

fn rpc_get_account_info(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let addr = match parse_address_param(params, 0) {
        Ok(a) => a,
        Err(e) => return e(id),
    };
    let db = safe_lock(&state.db);
    match db.get_account(&addr) {
        Some(acct) => {
            let obj = serde_json::json!({
                "address": format!("0x{}", hex::encode(acct.address)),
                "balance": json_hex_u64(acct.balance),
                "nonce": json_hex_u64(acct.nonce),
                "storage_deposit": json_hex_u64(acct.storage_deposit),
                "storage_bytes": json_hex_u64(acct.storage_bytes),
            });
            JsonRpcResponse::ok(id, obj)
        }
        None => JsonRpcResponse::ok(id, Value::Null),
    }
}

fn rpc_get_block_by_number(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let arr = match params.as_array() {
        Some(a) => a,
        None => return JsonRpcResponse::invalid_params(id, "expected array params"),
    };
    let block_num = match arr.first() {
        Some(Value::String(s)) if s == "latest" => {
            let c = safe_lock(&state.consensus);
            c.block_number()
        }
        Some(v) => match parse_hex_u64(v) {
            Some(n) => n,
            None => return JsonRpcResponse::invalid_params(id, "invalid block number"),
        },
        None => return JsonRpcResponse::invalid_params(id, "missing block number"),
    };
    let full_txs = arr.get(1).and_then(|v| v.as_bool()).unwrap_or(false);

    if let Some(ref cs) = state.chain_store {
        if let Some(block) = cs.load_full_block(block_num) {
            return JsonRpcResponse::ok(id, block_to_json(&block, full_txs));
        }
    }

    let history = safe_lock(&state.block_history);
    if let Some(record) = history.iter().find(|b| b.number == block_num) {
        let obj = serde_json::json!({
            "number": json_hex_u64(record.number),
            "epoch": json_hex_u64(record.epoch),
            "hash": &record.state_root,
            "parentHash": record.parent_hash,
            "stateRoot": record.state_root,
            "txCount": record.tx_count,
            "timestamp": json_hex_u64(record.timestamp),
            "gasUsed": json_hex_u64(record.gas_used),
            "baseFee": json_hex_u64(record.base_fee),
        });
        return JsonRpcResponse::ok(id, obj);
    }

    JsonRpcResponse::ok(id, Value::Null)
}

fn rpc_get_tx_receipt(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let hash_hex = match params.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()) {
        Some(h) => h.strip_prefix("0x").unwrap_or(h),
        None => return JsonRpcResponse::invalid_params(id, "missing tx hash"),
    };
    if let Some(ref cs) = state.chain_store {
        if let Some(receipt) = cs.get_tx_receipt(hash_hex) {
            let obj = serde_json::json!({
                "transactionHash": format!("0x{}", receipt.tx_hash),
                "blockNumber": json_hex_u64(receipt.block_number),
                "transactionIndex": json_hex_u64(receipt.tx_index as u64),
                "epoch": json_hex_u64(receipt.epoch),
                "type": receipt.tx_type,
                "from": receipt.from,
                "to": receipt.to,
                "status": if receipt.status == "confirmed" { "0x1" } else { "0x0" },
                "gasUsed": json_hex_u64(receipt.gas_used),
                "revertReason": receipt.revert_reason,
                "logCount": receipt.log_count,
            });
            return JsonRpcResponse::ok(id, obj);
        }
    }
    JsonRpcResponse::ok(id, Value::Null)
}

fn rpc_send_raw_tx(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let tx_json = match params.as_array().and_then(|a| a.first()) {
        Some(v) => v,
        None => return JsonRpcResponse::invalid_params(id, "missing transaction"),
    };
    let tx: Transaction = match serde_json::from_value(tx_json.clone()) {
        Ok(t) => t,
        Err(e) => return JsonRpcResponse::invalid_params(id, format!("invalid tx: {}", e)),
    };
    let hash = hex::encode(blake3::hash(&serde_json::to_vec(&tx).unwrap_or_default()).as_bytes());
    state.submit_tx(tx);
    JsonRpcResponse::ok(id, Value::String(format!("0x{}", hash)))
}

fn rpc_estimate_gas(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let tx_json = match params.as_array().and_then(|a| a.first()) {
        Some(v) => v,
        None => return JsonRpcResponse::invalid_params(id, "missing transaction"),
    };
    let tx: Transaction = match serde_json::from_value(tx_json.clone()) {
        Ok(t) => t,
        Err(e) => return JsonRpcResponse::invalid_params(id, format!("invalid tx: {}", e)),
    };
    let gas = crate::api::estimate_tx_gas_pub(&tx);
    let base_fee = latest_base_fee(state);
    let obj = serde_json::json!({
        "gas": json_hex_u64(gas),
        "baseFee": json_hex_u64(base_fee),
        "totalFee": json_hex_u64(gas * base_fee),
    });
    JsonRpcResponse::ok(id, obj)
}

fn rpc_get_object(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let obj_hex = match params.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()) {
        Some(h) => h.strip_prefix("0x").unwrap_or(h),
        None => return JsonRpcResponse::invalid_params(id, "missing object ID"),
    };
    let bytes = match hex::decode(obj_hex) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return JsonRpcResponse::invalid_params(id, "invalid object ID (need 32 bytes hex)"),
    };
    let db = safe_lock(&state.db);
    match db.get_object(&bytes) {
        Some(obj) => {
            let obj_json = serde_json::json!({
                "id": format!("0x{}", hex::encode(obj.id)),
                "owner": format!("0x{}", hex::encode(obj.owner)),
                "energy": obj.energy,
                "half_life": obj.half_life,
                "created_at": obj.created_at,
                "data_len": obj.data.len(),
            });
            JsonRpcResponse::ok(id, obj_json)
        }
        None => JsonRpcResponse::ok(id, Value::Null),
    }
}

fn rpc_mempool_size(state: &ApiState, id: Value) -> JsonRpcResponse {
    let size = if let Some(ref tc) = state.tendermint {
        let c = safe_lock(tc);
        c.mempool.len()
    } else {
        let c = safe_lock(&state.consensus);
        c.mempool.len()
    };
    JsonRpcResponse::ok(id, json_hex_u64(size as u64))
}

fn rpc_net_version(_state: &ApiState, id: Value) -> JsonRpcResponse {
    JsonRpcResponse::ok(id, Value::String("1".to_string()))
}

fn rpc_peer_count(state: &ApiState, id: Value) -> JsonRpcResponse {
    let count = state.peer_count.load(std::sync::atomic::Ordering::Relaxed);
    JsonRpcResponse::ok(id, json_hex_u64(count as u64))
}

fn rpc_get_finality_status(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let height = match params.as_array().and_then(|a| a.first()).and_then(|v| parse_hex_u64(v)) {
        Some(h) => h,
        None => return JsonRpcResponse::invalid_params(id, "missing block height"),
    };
    let ft = safe_lock(&state.finality_tracker);
    let status = ft.finality_status(height);
    let obj = match status {
        evaporchain_consensus::finality::FinalityStatus::Finalized { confirmations } => {
            let record = ft.get_record(height);
            serde_json::json!({
                "status": "finalized",
                "confirmations": confirmations,
                "signerCount": record.map(|r| r.signer_count).unwrap_or(0),
                "participationRate": record.map(|r| r.participation_rate()).unwrap_or(0.0),
            })
        }
        evaporchain_consensus::finality::FinalityStatus::Pending => {
            serde_json::json!({ "status": "pending" })
        }
        evaporchain_consensus::finality::FinalityStatus::Unknown => {
            serde_json::json!({ "status": "unknown" })
        }
    };
    JsonRpcResponse::ok(id, obj)
}

fn rpc_latest_finalized(state: &ApiState, id: Value) -> JsonRpcResponse {
    let ft = safe_lock(&state.finality_tracker);
    let height = ft.latest_finalized_height();
    let obj = serde_json::json!({
        "latestFinalizedBlock": json_hex_u64(height),
        "totalFinalized": ft.total_finalized(),
    });
    JsonRpcResponse::ok(id, obj)
}

fn rpc_get_logs(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let filter = match params.as_array().and_then(|a| a.first()) {
        Some(v) => v,
        None => return JsonRpcResponse::invalid_params(id, "missing filter object"),
    };
    let contract_id = match filter.get("contractId").and_then(|v| v.as_u64()) {
        Some(c) => c,
        None => return JsonRpcResponse::invalid_params(id, "contractId (u64) required"),
    };
    let event_name = filter.get("eventName").and_then(|v| v.as_str());
    let from_block = filter.get("fromBlock").and_then(|v| parse_hex_u64(v));
    let to_block = filter.get("toBlock").and_then(|v| parse_hex_u64(v));
    let limit = filter.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
    let limit = limit.min(1000);

    let cs = match &state.chain_store {
        Some(cs) => cs,
        None => return JsonRpcResponse::ok(id, Value::Array(vec![])),
    };
    let logs = cs.get_contract_events(contract_id, event_name, from_block, to_block, limit);
    let arr: Vec<Value> = logs.iter().map(event_log_to_json).collect();
    JsonRpcResponse::ok(id, Value::Array(arr))
}

fn rpc_get_block_logs(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let block_num = match params.as_array().and_then(|a| a.first()).and_then(|v| parse_hex_u64(v)) {
        Some(n) => n,
        None => return JsonRpcResponse::invalid_params(id, "missing block number"),
    };
    let limit = params.as_array()
        .and_then(|a| a.get(1))
        .and_then(|v| v.as_u64())
        .unwrap_or(500) as usize;
    let limit = limit.min(5000);

    let cs = match &state.chain_store {
        Some(cs) => cs,
        None => return JsonRpcResponse::ok(id, Value::Array(vec![])),
    };
    let logs = cs.get_block_events(block_num, limit);
    let arr: Vec<Value> = logs.iter().map(event_log_to_json).collect();
    JsonRpcResponse::ok(id, Value::Array(arr))
}

fn event_log_to_json(log: &crate::persistence::ContractEventLog) -> Value {
    serde_json::json!({
        "contractId": log.contract_id,
        "blockNumber": json_hex_u64(log.block_number),
        "logIndex": log.log_index,
        "epoch": json_hex_u64(log.epoch),
        "timestamp": json_hex_u64(log.timestamp),
        "transactionHash": format!("0x{}", log.tx_hash),
        "eventName": log.event_name,
        "topics": log.topics,
        "data": log.data,
    })
}

// ──────────────────────────── Helpers ────────────────────────────────

fn json_hex_u64(val: u64) -> Value {
    Value::String(format!("0x{:x}", val))
}

fn parse_hex_u64(v: &Value) -> Option<u64> {
    let s = v.as_str()?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16).ok()
}

fn parse_address_param(params: &Value, idx: usize) -> Result<[u8; 32], fn(Value) -> JsonRpcResponse> {
    let arr = params.as_array();
    let hex_str = arr
        .and_then(|a| a.get(idx))
        .and_then(|v| v.as_str())
        .ok_or(invalid_params_fn as fn(Value) -> JsonRpcResponse)?;
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(hex_str).map_err(|_| invalid_params_fn as fn(Value) -> JsonRpcResponse)?;
    if bytes.len() != 32 {
        return Err(invalid_params_fn);
    }
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&bytes);
    Ok(addr)
}

fn invalid_params_fn(id: Value) -> JsonRpcResponse {
    JsonRpcResponse::invalid_params(id, "invalid address (need 32 bytes hex)")
}

fn latest_base_fee(state: &ApiState) -> u64 {
    let history = safe_lock(&state.block_history);
    history.back().map(|b| b.base_fee).unwrap_or(1)
}

fn block_to_json(block: &evaporchain_types::Block, full_txs: bool) -> Value {
    let tx_list = if full_txs {
        serde_json::to_value(&block.transactions).unwrap_or(Value::Array(vec![]))
    } else {
        let hashes: Vec<String> = block.transactions.iter().map(|tx| {
            let h = blake3::hash(&serde_json::to_vec(tx).unwrap_or_default());
            format!("0x{}", hex::encode(h.as_bytes()))
        }).collect();
        serde_json::to_value(hashes).unwrap_or(Value::Array(vec![]))
    };
    serde_json::json!({
        "number": json_hex_u64(block.number),
        "epoch": json_hex_u64(block.epoch),
        "hash": format!("0x{}", hex::encode(block.state_root)),
        "parentHash": format!("0x{}", hex::encode(block.parent_hash)),
        "stateRoot": format!("0x{}", hex::encode(block.state_root)),
        "timestamp": json_hex_u64(block.timestamp),
        "txCount": block.transactions.len(),
        "transactions": tx_list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── json_hex_u64 ──

    #[test]
    fn test_json_hex_u64_zero() {
        assert_eq!(json_hex_u64(0), Value::String("0x0".into()));
    }

    #[test]
    fn test_json_hex_u64_small() {
        assert_eq!(json_hex_u64(255), Value::String("0xff".into()));
    }

    #[test]
    fn test_json_hex_u64_large() {
        assert_eq!(json_hex_u64(1_000_000), Value::String("0xf4240".into()));
    }

    #[test]
    fn test_json_hex_u64_max() {
        let result = json_hex_u64(u64::MAX);
        assert_eq!(result, Value::String("0xffffffffffffffff".into()));
    }

    // ── parse_hex_u64 ──

    #[test]
    fn test_parse_hex_u64_with_prefix() {
        let v = Value::String("0xff".into());
        assert_eq!(parse_hex_u64(&v), Some(255));
    }

    #[test]
    fn test_parse_hex_u64_without_prefix() {
        let v = Value::String("ff".into());
        assert_eq!(parse_hex_u64(&v), Some(255));
    }

    #[test]
    fn test_parse_hex_u64_zero() {
        let v = Value::String("0x0".into());
        assert_eq!(parse_hex_u64(&v), Some(0));
    }

    #[test]
    fn test_parse_hex_u64_invalid() {
        let v = Value::String("0xGG".into());
        assert_eq!(parse_hex_u64(&v), None);
    }

    #[test]
    fn test_parse_hex_u64_non_string() {
        let v = Value::Number(serde_json::Number::from(42));
        assert_eq!(parse_hex_u64(&v), None);
    }

    // ── parse_address_param ──

    #[test]
    fn test_parse_address_valid() {
        let addr_hex = hex::encode([0xABu8; 32]);
        let params = serde_json::json!([format!("0x{}", addr_hex)]);
        let result = parse_address_param(&params, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), [0xAB; 32]);
    }

    #[test]
    fn test_parse_address_without_prefix() {
        let addr_hex = hex::encode([0x01u8; 32]);
        let params = serde_json::json!([addr_hex]);
        assert!(parse_address_param(&params, 0).is_ok());
    }

    #[test]
    fn test_parse_address_wrong_length() {
        let params = serde_json::json!(["0xdeadbeef"]);
        assert!(parse_address_param(&params, 0).is_err());
    }

    #[test]
    fn test_parse_address_missing_param() {
        let params = serde_json::json!([]);
        assert!(parse_address_param(&params, 0).is_err());
    }

    #[test]
    fn test_parse_address_not_array() {
        let params = serde_json::json!({"addr": "0x00"});
        assert!(parse_address_param(&params, 0).is_err());
    }

    // ── JsonRpcResponse constructors ──

    #[test]
    fn test_response_ok_format() {
        let resp = JsonRpcResponse::ok(Value::Number(1.into()), Value::Bool(true));
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        assert_eq!(resp.id, Value::Number(1.into()));
    }

    #[test]
    fn test_response_err_format() {
        let resp = JsonRpcResponse::err(Value::Number(2.into()), -32600, "bad request");
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "bad request");
    }

    #[test]
    fn test_method_not_found() {
        let resp = JsonRpcResponse::method_not_found(Value::Null, "eth_foo");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("eth_foo"));
    }

    #[test]
    fn test_invalid_params() {
        let resp = JsonRpcResponse::invalid_params(Value::Null, "missing field");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("missing field"));
    }

    // ── roundtrip ──

    #[test]
    fn test_hex_roundtrip() {
        for val in [0, 1, 255, 65535, u64::MAX] {
            let hex_val = json_hex_u64(val);
            let parsed = parse_hex_u64(&hex_val);
            assert_eq!(parsed, Some(val), "roundtrip failed for {val}");
        }
    }

    // ── block_to_json ──

    #[test]
    fn test_block_to_json_no_txs() {
        let block = evaporchain_types::Block {
            number: 42,
            epoch: 5,
            state_root: [0xAA; 32],
            parent_hash: [0xBB; 32],
            timestamp: 1000,
            transactions: vec![],
            vrf_output: None,
            producer_id: None,
            commit_certificate: None,
        };
        let json = block_to_json(&block, false);
        assert_eq!(json["number"], "0x2a");
        assert_eq!(json["epoch"], "0x5");
        assert_eq!(json["txCount"], 0);
        assert!(json["stateRoot"].as_str().unwrap().starts_with("0x"));
    }
}
