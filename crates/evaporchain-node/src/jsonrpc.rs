//! JSON-RPC 2.0 endpoint for ethers.js / web3.py compatibility.
//!
//! All methods use the `evap_` namespace. Standard `net_*` methods are also
//! supported for tooling that probes the network layer.

use axum::{extract::State, Json};
use evaporchain_execution::ExecutionEngine;
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
    #[allow(dead_code)]
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
        Self {
            jsonrpc: "2.0",
            result: Some(result),
            error: None,
            id,
        }
    }

    fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
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
                results.push(JsonRpcResponse::err(Value::Null, -32600, "Invalid Request"));
            }
        }
        Json(serde_json::to_value(results).unwrap_or(Value::Null))
    } else if let Ok(r) = serde_json::from_value::<JsonRpcRequest>(req) {
        let resp = dispatch(&state, r);
        Json(serde_json::to_value(resp).unwrap_or(Value::Null))
    } else {
        Json(
            serde_json::to_value(JsonRpcResponse::err(Value::Null, -32700, "Parse error"))
                .unwrap_or(Value::Null),
        )
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
        "evap_getMMRProof" => rpc_get_mmr_proof(state, &req.params, req.id),
        "evap_getMMRRoot" => rpc_get_mmr_root(state, req.id),
        "evap_decayForgetProof" => rpc_decay_forget_proof(state, &req.params, req.id),
        "evap_getPntStatus" => rpc_get_pnt_status(state, req.id),
        "evap_getFrontierStatus" => rpc_get_frontier_status(state, req.id),
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
    let hash_hex = match params
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
    {
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
    let obj_hex = match params
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
    {
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

/// Pure formatter for MMR-proof responses. Extracted so the JSON
/// shape is unit-testable without a full ApiState (which carries
/// 30+ fields including RocksDB).
fn format_mmr_proof_response(
    proof: &evaporchain_crypto::accumulator::MMRProof,
) -> Value {
    serde_json::json!({
        "leaf_index": json_hex_u64(proof.leaf_index),
        "mmr_size": json_hex_u64(proof.mmr_size),
        "peak_index": proof.peak_index,
        "siblings": proof.siblings.iter()
            .map(|s| format!("0x{}", hex::encode(s)))
            .collect::<Vec<_>>(),
        "peak_hashes": proof.peak_hashes.iter()
            .map(|p| format!("0x{}", hex::encode(p)))
            .collect::<Vec<_>>(),
    })
}

/// Pure formatter for MMR-root responses.
fn format_mmr_root_response(root: [u8; 32], size: usize) -> Value {
    serde_json::json!({
        "root": format!("0x{}", hex::encode(root)),
        "size": json_hex_u64(size as u64),
    })
}

/// Parse the leaf_index hex param from position 0 of the JSON-RPC
/// params array.
fn parse_leaf_index_param(params: &Value) -> Result<u64, &'static str> {
    let s = params
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .ok_or("missing leaf_index")?;
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(stripped, 16).map_err(|_| "leaf_index must be hex u64")
}

/// `evap_getMMRProof(leaf_index_hex)` — return an inclusion proof for the
/// evaporation-nullifier MMR leaf at `leaf_index`. The proof bundle is
/// `{ leaf_index, mmr_size, siblings, peak_hashes, peak_index }`. Light
/// clients can verify a ghost-record's evaporation against the chain's
/// `mmr_root` without downloading the whole accumulator.
fn rpc_get_mmr_proof(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let leaf_index = match parse_leaf_index_param(params) {
        Ok(n) => n,
        Err(msg) => return JsonRpcResponse::invalid_params(id, msg),
    };
    let proof_opt = {
        let c = safe_lock(&state.consensus);
        c.executor.mmr_proof(leaf_index)
    };
    match proof_opt {
        Some(proof) => JsonRpcResponse::ok(id, format_mmr_proof_response(&proof)),
        None => JsonRpcResponse::ok(id, Value::Null),
    }
}

/// Pure computation behind `evap_decayForgetProof`. Extracted as a
/// free function so it can be exhaustively unit-tested without
/// constructing a full ApiState. Returns the JSON body that the RPC
/// handler ships back, or `Err(message)` on out-of-range inputs.
fn compute_decay_forget_response(
    record_id: [u8; 32],
    original_commitment: u64,
    activated_epoch: u64,
    query_epoch: u64,
    forget_threshold: u64,
    half_life: u64,
) -> Result<Value, &'static str> {
    if half_life == 0 {
        return Err("half_life must be > 0");
    }
    let lambda = evaporchain_energy_kernel::ChainLambda::new(
        evaporchain_energy_kernel::Lambda::from_epochs(half_life),
    );
    let proof = evaporchain_decay_forget::prove_forgotten(
        record_id,
        original_commitment,
        lambda,
        activated_epoch,
        query_epoch,
        forget_threshold,
    );
    Ok(serde_json::json!({
        "record_id": format!("0x{}", hex::encode(proof.record_id)),
        "original_commitment": json_hex_u64(proof.original_commitment),
        "activated_epoch": json_hex_u64(proof.activated_epoch),
        "forgotten_at_epoch": json_hex_u64(proof.forgotten_at_epoch),
        "forget_threshold": json_hex_u64(proof.forget_threshold),
        "decayed_commitment": json_hex_u64(proof.decayed_commitment),
        "witness": format!("0x{}", hex::encode(proof.witness)),
        "is_forgotten": proof.decayed_commitment <= proof.forget_threshold,
    }))
}

/// `evap_decayForgetProof([record_id_hex32, original_commitment_hex,
///                         activated_epoch_hex, query_epoch_hex,
///                         forget_threshold_hex, half_life_epochs_hex])`
///
/// Computational RPC for GDPR-Article-17-style "right to be forgotten"
/// attestations: returns a `DecayForgetProof` showing that a record's
/// recoverability commitment has decayed below `forget_threshold` at
/// `query_epoch`. Verified by anyone via `verify_forget_proof`.
///
/// **Production wiring** will consume `original_commitment` and
/// `activated_epoch` from a per-record commitment store (currently
/// driven by the caller; a chain-side store keyed on `record_id` is the
/// follow-up build).
fn rpc_decay_forget_proof(state: &ApiState, params: &Value, id: Value) -> JsonRpcResponse {
    let _ = state; // pure computation, no chain state read in this iteration
    let arr = match params.as_array() {
        Some(a) if a.len() == 6 => a,
        _ => {
            return JsonRpcResponse::invalid_params(
                id,
                "expected 6 hex params: [record_id, original_commitment, \
                 activated_epoch, query_epoch, forget_threshold, half_life]",
            )
        }
    };
    let record_hex = match arr[0].as_str() {
        Some(s) => s.strip_prefix("0x").unwrap_or(s),
        None => return JsonRpcResponse::invalid_params(id, "record_id must be hex string"),
    };
    let record_bytes = match hex::decode(record_hex) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        _ => return JsonRpcResponse::invalid_params(id, "record_id must be 32 bytes hex"),
    };
    let parse_u64 = |v: &Value| -> Option<u64> {
        v.as_str()
            .and_then(|s| u64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16).ok())
    };
    let original_commitment = match parse_u64(&arr[1]) {
        Some(n) => n,
        None => return JsonRpcResponse::invalid_params(id, "original_commitment must be hex u64"),
    };
    let activated_epoch = match parse_u64(&arr[2]) {
        Some(n) => n,
        None => return JsonRpcResponse::invalid_params(id, "activated_epoch must be hex u64"),
    };
    let query_epoch = match parse_u64(&arr[3]) {
        Some(n) => n,
        None => return JsonRpcResponse::invalid_params(id, "query_epoch must be hex u64"),
    };
    let forget_threshold = match parse_u64(&arr[4]) {
        Some(n) => n,
        None => return JsonRpcResponse::invalid_params(id, "forget_threshold must be hex u64"),
    };
    let half_life = match parse_u64(&arr[5]) {
        Some(n) if n > 0 => n,
        _ => {
            return JsonRpcResponse::invalid_params(
                id,
                "half_life must be hex u64 > 0",
            )
        }
    };
    match compute_decay_forget_response(
        record_bytes,
        original_commitment,
        activated_epoch,
        query_epoch,
        forget_threshold,
        half_life,
    ) {
        Ok(v) => JsonRpcResponse::ok(id, v),
        Err(msg) => JsonRpcResponse::invalid_params(id, msg),
    }
}

/// `evap_getMMRRoot()` — return the current evaporation-nullifier MMR
/// root + leaf count. Companion to `evap_getMMRProof`; clients verify
/// proofs against this root.
fn rpc_get_mmr_root(state: &ApiState, id: Value) -> JsonRpcResponse {
    let (root, size) = {
        let c = safe_lock(&state.consensus);
        (c.executor.mmr_root(), c.executor.mmr_size())
    };
    JsonRpcResponse::ok(id, format_mmr_root_response(root, size))
}

/// Pure formatter for PNT-status responses. `current_phase` rotates
/// every block by `tick_pnt_phase` (commit 05a36b1); `live_count` is
/// the number of nullifiers retained in the live window. Operators
/// compare growth curves between this bounded count and the unbounded
/// canonical nullifier set before flipping the future hard-fork that
/// makes PNT authoritative.
fn format_pnt_status_response(
    current_phase: u64,
    live_count: usize,
    window_depth: usize,
    last_phase_epoch: Option<u64>,
    phase_interval_epochs: u64,
) -> Value {
    serde_json::json!({
        "current_phase": json_hex_u64(current_phase),
        "live_count": json_hex_u64(live_count as u64),
        "window_depth": window_depth,
        "last_phase_epoch": last_phase_epoch.map(json_hex_u64).unwrap_or(Value::Null),
        "phase_interval_epochs": json_hex_u64(phase_interval_epochs),
    })
}

/// `evap_getPntStatus()` — current Phasing Nullifier Tree state for
/// operator monitoring. PNT runs in shadow alongside the canonical
/// (unbounded) nullifier set; `live_count` is the bounded count.
fn rpc_get_pnt_status(state: &ApiState, id: Value) -> JsonRpcResponse {
    let (current_phase, live_count, window_depth, last_epoch, interval) = {
        let c = safe_lock(&state.consensus);
        let pe = &c.executor.privacy_executor;
        (
            pe.pnt.current_phase,
            pe.pnt.live_count(),
            pe.pnt.window_depth,
            pe.pnt_last_phase_epoch(),
            pe.pnt_phase_interval_epochs(),
        )
    };
    JsonRpcResponse::ok(
        id,
        format_pnt_status_response(
            current_phase,
            live_count,
            window_depth,
            last_epoch,
            interval,
        ),
    )
}

/// Pure formatter for FrontierState status. Surfaces anchor count,
/// PoHA temperature distribution, energy-verkle health snapshot, and
/// the lazy-state-cache size — substrate state operators need but
/// can't see otherwise.
#[allow(clippy::too_many_arguments)]
fn format_frontier_status_response(
    anchor_count: usize,
    poha_active: usize,
    poha_ghosts: usize,
    poha_hot: u32,
    poha_warm: u32,
    poha_cold: u32,
    poha_evaporated: u32,
    trie_active_leaves: u32,
    trie_compressed_leaves: u32,
    trie_total_nodes: u32,
    trie_compressions: u64,
    trie_decompressions: u64,
    lazy_snapshot_count: usize,
) -> Value {
    serde_json::json!({
        "anchor_count": json_hex_u64(anchor_count as u64),
        "poha": {
            "active": json_hex_u64(poha_active as u64),
            "ghosts": json_hex_u64(poha_ghosts as u64),
            "temperature": {
                "hot": json_hex_u64(poha_hot as u64),
                "warm": json_hex_u64(poha_warm as u64),
                "cold": json_hex_u64(poha_cold as u64),
                "evaporated": json_hex_u64(poha_evaporated as u64),
            },
        },
        "energy_trie": {
            "active_leaves": json_hex_u64(trie_active_leaves as u64),
            "compressed_leaves": json_hex_u64(trie_compressed_leaves as u64),
            "total_nodes": json_hex_u64(trie_total_nodes as u64),
            "compressions": json_hex_u64(trie_compressions),
            "decompressions": json_hex_u64(trie_decompressions),
        },
        "lazy_snapshot_count": json_hex_u64(lazy_snapshot_count as u64),
    })
}

/// `evap_getFrontierStatus()` — anchor count, PoHA temperature
/// distribution, energy-Verkle trie health, lazy-state-cache size.
/// Returns a "frontier_state_disabled" body when no FrontierState
/// is wired (i.e. the operator didn't construct one in main.rs).
fn rpc_get_frontier_status(state: &ApiState, id: Value) -> JsonRpcResponse {
    let Some(ref fs_arc) = state.frontier_state else {
        return JsonRpcResponse::ok(
            id,
            serde_json::json!({ "frontier_state_disabled": true }),
        );
    };
    let fs = safe_lock(fs_arc);
    let dist = fs.poha.temperature_distribution();
    let health = fs.energy_trie.health();
    JsonRpcResponse::ok(
        id,
        format_frontier_status_response(
            fs.anchors.anchor_count(),
            fs.poha.active_count(),
            fs.poha.ghost_count(),
            dist.hot,
            dist.warm,
            dist.cold,
            dist.evaporated,
            health.active_leaves,
            health.compressed_leaves,
            health.total_nodes,
            health.compressions,
            health.decompressions,
            fs.lazy_cache.snapshot_count(),
        ),
    )
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
    let height = match params
        .as_array()
        .and_then(|a| a.first())
        .and_then(parse_hex_u64)
    {
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
    let from_block = filter.get("fromBlock").and_then(parse_hex_u64);
    let to_block = filter.get("toBlock").and_then(parse_hex_u64);
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
    let block_num = match params
        .as_array()
        .and_then(|a| a.first())
        .and_then(parse_hex_u64)
    {
        Some(n) => n,
        None => return JsonRpcResponse::invalid_params(id, "missing block number"),
    };
    let limit = params
        .as_array()
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

fn parse_address_param(
    params: &Value,
    idx: usize,
) -> Result<[u8; 32], fn(Value) -> JsonRpcResponse> {
    let arr = params.as_array();
    let hex_str = arr
        .and_then(|a| a.get(idx))
        .and_then(|v| v.as_str())
        .ok_or(invalid_params_fn as fn(Value) -> JsonRpcResponse)?;
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes =
        hex::decode(hex_str).map_err(|_| invalid_params_fn as fn(Value) -> JsonRpcResponse)?;
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
        let hashes: Vec<String> = block
            .transactions
            .iter()
            .map(|tx| {
                let h = blake3::hash(&serde_json::to_vec(tx).unwrap_or_default());
                format!("0x{}", hex::encode(h.as_bytes()))
            })
            .collect();
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

    // ── parse_address_param edge cases ──

    #[test]
    fn test_parse_address_invalid_hex_chars() {
        let params = serde_json::json!(["0xZZ"]);
        assert!(parse_address_param(&params, 0).is_err());
    }

    #[test]
    fn test_parse_address_too_long() {
        let too_long = hex::encode([0xCDu8; 33]);
        let params = serde_json::json!([format!("0x{}", too_long)]);
        assert!(parse_address_param(&params, 0).is_err());
    }

    // ── parse_hex_u64 edge cases ──

    #[test]
    fn test_parse_hex_u64_empty_string_is_none() {
        let v = Value::String(String::new());
        assert_eq!(parse_hex_u64(&v), None);
    }

    #[test]
    fn test_parse_hex_u64_overflow_is_none() {
        let v = Value::String("0x10000000000000000".into()); // 2^64
        assert_eq!(parse_hex_u64(&v), None);
    }

    // ── event_log_to_json field lock ──

    #[test]
    fn test_event_log_to_json_field_lock() {
        let log = crate::persistence::ContractEventLog {
            contract_id: 7,
            block_number: 42,
            log_index: 1,
            epoch: 4,
            timestamp: 1000,
            tx_hash: "feedface".into(),
            event_name: "Burn".into(),
            topics: vec!["t0".into(), "t1".into()],
            data: vec!["d0".into()],
        };
        let v = event_log_to_json(&log);
        assert_eq!(v["contractId"], 7);
        assert_eq!(v["blockNumber"], "0x2a");
        assert_eq!(v["logIndex"], 1);
        assert_eq!(v["epoch"], "0x4");
        assert_eq!(v["timestamp"], "0x3e8");
        assert_eq!(v["transactionHash"], "0xfeedface");
        assert_eq!(v["eventName"], "Burn");
        assert_eq!(v["topics"], serde_json::json!(["t0", "t1"]));
        assert_eq!(v["data"], serde_json::json!(["d0"]));
    }

    // ── JsonRpcResponse serialization shape ──

    #[test]
    fn test_response_serializes_with_jsonrpc_2_0() {
        let resp = JsonRpcResponse::ok(Value::Number(99.into()), Value::String("ok".into()));
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 99);
        assert_eq!(v["result"], "ok");
        assert!(v.get("error").is_none(), "error should be skipped on ok");
    }

    #[test]
    fn test_error_response_skips_result_field() {
        let resp = JsonRpcResponse::method_not_found(Value::Number(1.into()), "evap_unknown");
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert!(v.get("result").is_none(), "result should be skipped on err");
        assert_eq!(v["error"]["code"], -32601);
        assert!(v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("evap_unknown"));
    }

    #[test]
    fn test_block_to_json_no_txs() {
        let block = evaporchain_types::Block {
            number: 42,
            epoch: 5,
            state_root: [0xAA; 32],
            parent_hash: [0xBB; 32],
            timestamp: 1000,
            transactions: vec![],
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
        };
        let json = block_to_json(&block, false);
        assert_eq!(json["number"], "0x2a");
        assert_eq!(json["epoch"], "0x5");
        assert_eq!(json["txCount"], 0);
        assert!(json["stateRoot"].as_str().unwrap().starts_with("0x"));
    }

    // ── PNT + Frontier status formatters ──

    #[test]
    fn test_format_pnt_status_basic() {
        let v = format_pnt_status_response(3, 42, 5, Some(100), 100);
        assert_eq!(v["current_phase"], "0x3");
        assert_eq!(v["live_count"], "0x2a");
        assert_eq!(v["window_depth"], 5);
        assert_eq!(v["last_phase_epoch"], "0x64");
        assert_eq!(v["phase_interval_epochs"], "0x64");
    }

    #[test]
    fn test_format_pnt_status_no_advance_yet() {
        // Fresh executor: last_phase_epoch is None → JSON null.
        let v = format_pnt_status_response(0, 0, 5, None, 100);
        assert_eq!(v["current_phase"], "0x0");
        assert_eq!(v["live_count"], "0x0");
        assert_eq!(v["last_phase_epoch"], serde_json::Value::Null);
    }

    #[test]
    fn test_format_frontier_status_basic() {
        let v = format_frontier_status_response(
            7, 12, 3, 4, 5, 2, 1, 100, 8, 116, 5, 1, 9,
        );
        assert_eq!(v["anchor_count"], "0x7");
        assert_eq!(v["poha"]["active"], "0xc"); // 12
        assert_eq!(v["poha"]["ghosts"], "0x3");
        let temp = &v["poha"]["temperature"];
        assert_eq!(temp["hot"], "0x4");
        assert_eq!(temp["warm"], "0x5");
        assert_eq!(temp["cold"], "0x2");
        assert_eq!(temp["evaporated"], "0x1");
        let trie = &v["energy_trie"];
        assert_eq!(trie["active_leaves"], "0x64"); // 100
        assert_eq!(trie["compressed_leaves"], "0x8");
        assert_eq!(trie["total_nodes"], "0x74"); // 116
        assert_eq!(trie["compressions"], "0x5");
        assert_eq!(trie["decompressions"], "0x1");
        assert_eq!(v["lazy_snapshot_count"], "0x9");
    }

    #[test]
    fn test_format_frontier_status_zero_distribution() {
        // An empty/quiescent FrontierState — every count is 0.
        let v = format_frontier_status_response(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
        assert_eq!(v["anchor_count"], "0x0");
        assert_eq!(v["poha"]["temperature"]["hot"], "0x0");
        assert_eq!(v["energy_trie"]["compressions"], "0x0");
    }

    // ── compute_decay_forget_response (evap_decayForgetProof core) ──

    // ── MMR RPC helpers ──

    #[test]
    fn test_format_mmr_root_response_shape() {
        let v = format_mmr_root_response([0xABu8; 32], 17);
        assert_eq!(
            v["root"].as_str().unwrap(),
            format!("0x{}", "ab".repeat(32))
        );
        assert_eq!(v["size"], "0x11"); // 17 in hex
    }

    #[test]
    fn test_format_mmr_proof_response_shape() {
        use evaporchain_crypto::accumulator::MMRProof;
        let proof = MMRProof {
            leaf_index: 0x42,
            mmr_size: 0x100,
            siblings: vec![[0x11u8; 32], [0x22u8; 32]],
            peak_hashes: vec![[0x33u8; 32]],
            peak_index: 1,
        };
        let v = format_mmr_proof_response(&proof);
        assert_eq!(v["leaf_index"], "0x42");
        assert_eq!(v["mmr_size"], "0x100");
        assert_eq!(v["peak_index"], 1);
        let siblings = v["siblings"].as_array().unwrap();
        assert_eq!(siblings.len(), 2);
        assert_eq!(
            siblings[0].as_str().unwrap(),
            format!("0x{}", "11".repeat(32))
        );
        let peaks = v["peak_hashes"].as_array().unwrap();
        assert_eq!(peaks.len(), 1);
        assert_eq!(
            peaks[0].as_str().unwrap(),
            format!("0x{}", "33".repeat(32))
        );
    }

    #[test]
    fn test_format_mmr_proof_response_empty_arrays() {
        // A leaf at the only peak has no siblings and no other peaks.
        use evaporchain_crypto::accumulator::MMRProof;
        let proof = MMRProof {
            leaf_index: 0,
            mmr_size: 1,
            siblings: vec![],
            peak_hashes: vec![],
            peak_index: 0,
        };
        let v = format_mmr_proof_response(&proof);
        assert_eq!(v["siblings"].as_array().unwrap().len(), 0);
        assert_eq!(v["peak_hashes"].as_array().unwrap().len(), 0);
        assert_eq!(v["peak_index"], 0);
    }

    #[test]
    fn test_parse_leaf_index_param_with_prefix() {
        let params = serde_json::json!(["0x2a"]);
        assert_eq!(parse_leaf_index_param(&params).unwrap(), 42);
    }

    #[test]
    fn test_parse_leaf_index_param_without_prefix() {
        let params = serde_json::json!(["ff"]);
        assert_eq!(parse_leaf_index_param(&params).unwrap(), 255);
    }

    #[test]
    fn test_parse_leaf_index_param_missing() {
        let params = serde_json::json!([]);
        assert!(parse_leaf_index_param(&params).is_err());
    }

    #[test]
    fn test_parse_leaf_index_param_invalid_hex() {
        let params = serde_json::json!(["0xZZ"]);
        assert!(parse_leaf_index_param(&params).is_err());
    }

    #[test]
    fn test_parse_leaf_index_param_non_string() {
        let params = serde_json::json!([42]);
        assert!(parse_leaf_index_param(&params).is_err());
    }

    #[test]
    fn test_decay_forget_zero_half_life_rejected() {
        let r = compute_decay_forget_response([1u8; 32], 1_000, 0, 100, 10, 0);
        assert!(r.is_err(), "half_life=0 must be rejected");
    }

    #[test]
    fn test_decay_forget_carries_input_fields() {
        // record_id, original_commitment, activated_epoch, query_epoch,
        // forget_threshold, half_life
        let v = compute_decay_forget_response([0xAA; 32], 1_000, 5, 105, 10, 100)
            .expect("valid inputs must succeed");
        assert_eq!(
            v["record_id"].as_str().unwrap(),
            format!("0x{}", hex::encode([0xAA; 32]))
        );
        assert_eq!(v["original_commitment"], "0x3e8"); // 1000
        assert_eq!(v["activated_epoch"], "0x5");
        assert_eq!(v["forgotten_at_epoch"], "0x69"); // 105
        assert_eq!(v["forget_threshold"], "0xa"); // 10
    }

    #[test]
    fn test_decay_forget_late_query_under_threshold() {
        // 10 half-lives of decay → energy effectively 0; below threshold=10.
        let v = compute_decay_forget_response([0u8; 32], 1_000, 0, 1_000, 10, 100)
            .expect("valid inputs must succeed");
        assert_eq!(v["is_forgotten"], serde_json::Value::Bool(true));
    }

    #[test]
    fn test_decay_forget_early_query_over_threshold() {
        // No elapsed time → decayed_commitment == original > 10.
        let v = compute_decay_forget_response([0u8; 32], 1_000, 50, 50, 10, 100)
            .expect("valid inputs must succeed");
        assert_eq!(v["is_forgotten"], serde_json::Value::Bool(false));
    }

    #[test]
    fn test_decay_forget_witness_is_32_byte_hex() {
        let v = compute_decay_forget_response([7u8; 32], 1_000, 0, 100, 10, 100)
            .expect("valid inputs must succeed");
        let w = v["witness"].as_str().unwrap();
        assert!(w.starts_with("0x"));
        assert_eq!(w.len(), 2 + 64, "witness must be 0x + 32-byte hex");
    }

    #[test]
    fn test_decay_forget_deterministic() {
        // Same inputs must yield identical witnesses.
        let v1 = compute_decay_forget_response([0xCC; 32], 1_000, 1, 50, 10, 100).unwrap();
        let v2 = compute_decay_forget_response([0xCC; 32], 1_000, 1, 50, 10, 100).unwrap();
        assert_eq!(v1["witness"], v2["witness"]);
        assert_eq!(v1["decayed_commitment"], v2["decayed_commitment"]);
    }
}
