//! Node poller — scans `/api/scripts` for SFSV vault contracts with
//! active listings, and translates the on-chain state into off-chain
//! `Vault` + `Auction` objects for the auctioneer.
//!
//! Identification heuristic: any script whose `methods` include both
//! `list_for_sale` and `record_sale` is treated as an SFSV vault.
//! Production deployments should enforce a contract identity hash
//! (e.g., BLAKE3 of the canonical `.es` bytecode) — V1 uses the
//! method-name heuristic to keep the coordinator standalone.

use evaporchain_sddc::auction::{Auction, AuctionId};
use evaporchain_sfsv::{
    predicate::Predicate,
    vault::Vault,
};
use tracing::{debug, warn};

use crate::node::{NodeClient, ScriptSummary};

/// Build a synthetic `Vault` + `Auction` from the on-chain state JSON.
/// Returns `None` if the listing fields are absent (vault not listed).
pub fn parse_listing(
    contract_id: u64,
    state: &serde_json::Value,
) -> Option<(Vault, Auction)> {
    // Pull the nested `.state` object (EvaporScript state map).
    let s = state.get("state")?;

    // All numeric fields come out as JSON numbers.
    let listed = s.get("listed").and_then(|v| v.as_bool()).unwrap_or(false);
    if !listed {
        return None;
    }

    let ceiling = s.get("list_ceiling").and_then(|v| v.as_u64()).unwrap_or(0);
    let floor = s.get("list_floor").and_then(|v| v.as_u64()).unwrap_or(0);
    let opened_at = s.get("list_opened_at").and_then(|v| v.as_u64()).unwrap_or(0);
    let duration = s.get("list_duration").and_then(|v| v.as_u64()).unwrap_or(0);
    let deposit = s.get("deposit").and_then(|v| v.as_u64()).unwrap_or(1);
    let release_epoch = s.get("release_epoch").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);

    if ceiling <= floor || duration == 0 {
        warn!(contract_id, "on-chain listing has invalid bounds; skipping");
        return None;
    }

    // Synthetic creator and future_self — not critical for off-chain clearing.
    let creator = [0u8; 32];
    let future_self_arr = parse_addr(s.get("future_self_addr"));

    let vault_id: [u8; 32] = {
        let mut id = [0u8; 32];
        id[..8].copy_from_slice(&contract_id.to_le_bytes());
        id
    };

    let vault = Vault::create(
        vault_id,
        creator,
        future_self_arr,
        deposit.max(1),
        Predicate::EpochReached {
            release_epoch,
        },
        0,
    )
    .ok()?;

    // Re-apply the listing so the vault is in the Listed state (required
    // for `record_sale` to succeed via the on-chain guard).
    let mut vault = vault;
    vault
        .list_for_sale(future_self_arr, ceiling, floor, opened_at, duration)
        .ok()?;

    // Mirror listing into an SDDC Auction. `lot_lambda` ≔ 0 (coordinator
    // V1 does not interpret the patience axis from the `.es` state; it
    // accepts all lambda-tolerant bids). A real deployment reads the
    // `lot_lambda` field stored by `set_terms`.
    let auction_id: AuctionId = vault_id;
    let auction = Auction::open(auction_id, ceiling, floor, 0, opened_at, duration).ok()?;

    debug!(
        contract_id,
        ceiling,
        floor,
        opened_at,
        duration,
        "discovered active listing"
    );
    Some((vault, auction))
}

/// Returns the list of SFSV vault contract IDs that have active listings.
pub async fn discover_listings(client: &NodeClient) -> Vec<(u64, Vault, Auction)> {
    let scripts = match client.list_scripts().await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "failed to poll /api/scripts");
            return vec![];
        }
    };

    let vault_ids: Vec<u64> = scripts
        .into_iter()
        .filter(is_sfsv_vault)
        .map(|s| s.id)
        .collect();

    let mut out = Vec::new();
    for id in vault_ids {
        let detail = match client.get_script(id).await {
            Ok(d) => d,
            Err(e) => {
                warn!(contract_id = id, error = %e, "failed to fetch script detail");
                continue;
            }
        };
        if let Some((vault, auction)) = parse_listing(id, &detail) {
            out.push((id, vault, auction));
        }
    }
    out
}

fn is_sfsv_vault(s: &ScriptSummary) -> bool {
    let methods: std::collections::HashSet<&str> =
        s.methods.iter().map(String::as_str).collect();
    methods.contains("list_for_sale") && methods.contains("record_sale")
}

fn parse_addr(v: Option<&serde_json::Value>) -> [u8; 32] {
    v.and_then(|v| v.as_array())
        .map(|arr| {
            let mut addr = [0u8; 32];
            for (i, byte) in arr.iter().take(32).enumerate() {
                addr[i] = byte.as_u64().unwrap_or(0) as u8;
            }
            addr
        })
        .unwrap_or([0u8; 32])
}
