//! Listing-state parity check — closes the §5-A residual gap.
//!
//! The predicate path has `predicate_inlining_parity.rs` proving the
//! `.es` and Rust agree. The §5-A reconciliation added the on-chain
//! **listing state machine** (`list_for_sale` / `cancel_listing` /
//! `record_sale`) to the crate so the `.es` listing guards are
//! adversarially testable in Rust — but nothing pinned that the `.es`
//! (source of truth, EvaporChain invariant #2) and the Rust
//! `vault.rs` implementation enforce the SAME guard set. This file
//! does, at the source-pattern level (same philosophy as the
//! predicate parity test: pattern checks, not bytecode — they catch
//! every drift mode this repo has actually seen).
//!
//! The `.es` is canonical. If `vault.rs`'s listing guards drift from
//! these, the crate stops being a faithful `.es` model and the `.es`
//! header's "guarded by the adversarial tests in the SFSV crate"
//! claim becomes hollow for the listing path.
//!
//! Rust side is pinned by `vault.rs`'s unit tests
//! (`list_for_sale_guards`, `cancel_listing_guards`,
//! `record_sale_guards`, `release_clears_listing_and_blocks_market_ops`)
//! + the §8.6 listing adversaries in `adversarial.rs`. This file pins
//! the `.es` side and the cross-equivalence of the guard set.

use std::fs;
use std::path::PathBuf;

fn contract_source() -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../contracts/evaporscript/future_self_vault.es");
    fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("could not read {}: {}", p.display(), e))
}

/// Balanced-brace, comment-tolerant `fn <name>(...) { ... }` body
/// extractor. Identical algorithm to `predicate_inlining_parity.rs`
/// (kept in sync deliberately — both are `.es`-parity harnesses).
fn extract_fn_body(src: &str, fn_name: &str) -> String {
    let needle = format!("fn {fn_name}(");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("could not find `fn {fn_name}(` in contract source"));
    let open = src[start..]
        .find('{')
        .unwrap_or_else(|| panic!("function `{fn_name}` has no opening brace"))
        + start;
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut close = open;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    for i in open..bytes.len() {
        let c = bytes[i] as char;
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_block_comment {
            if c == '*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_block_comment = false;
            }
            continue;
        }
        if c == '/' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'/' => {
                    in_line_comment = true;
                    continue;
                }
                b'*' => {
                    in_block_comment = true;
                    continue;
                }
                _ => {}
            }
        }
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                close = i;
                break;
            }
        }
    }
    assert!(close > open, "function `{fn_name}` brace-walk did not close");
    src[open + 1..close].to_string()
}

fn assert_contains(body: &str, fn_name: &str, needle: &str) {
    assert!(
        body.contains(needle),
        "fn {fn_name} (.es) is missing guard `{needle}` — the Rust \
         vault.rs listing state machine enforces it; drift here breaks \
         §5-A `.es`↔crate parity (the `.es` is the source of truth)."
    );
}

// =================================================================
// list_for_sale — Rust vault.rs guards: NotLocked, NotCurrentHolder,
// AlreadyListed, BadListingBounds(ceiling<=floor), ZeroListingDuration
// =================================================================

#[test]
fn list_for_sale_guard_set_matches_rust() {
    let body = extract_fn_body(&contract_source(), "list_for_sale");
    assert_contains(&body, "list_for_sale", "self.sealed == true");
    assert_contains(&body, "list_for_sale", "self.released == false");
    assert_contains(&body, "list_for_sale", "caller == self.holder");
    assert_contains(&body, "list_for_sale", "self.listed == false");
    assert_contains(&body, "list_for_sale", "ceiling > floor");
    assert_contains(&body, "list_for_sale", "duration > 0");
}

// =================================================================
// cancel_listing — Rust guards: NotLocked, NotListed, NotCurrentHolder
// =================================================================

#[test]
fn cancel_listing_guard_set_matches_rust() {
    let body = extract_fn_body(&contract_source(), "cancel_listing");
    assert_contains(&body, "cancel_listing", "self.sealed == true");
    assert_contains(&body, "cancel_listing", "self.listed == true");
    assert_contains(&body, "cancel_listing", "caller == self.holder");
}

// =================================================================
// record_sale — Rust guards: NotLocked, NotListed, ListingExpired
// (epoch_now > opened_at + duration). No caller restriction — the
// listing state itself guards it (matches vault.rs::record_sale).
// =================================================================

#[test]
fn record_sale_guard_set_matches_rust() {
    let body = extract_fn_body(&contract_source(), "record_sale");
    assert_contains(&body, "record_sale", "self.sealed == true");
    assert_contains(&body, "record_sale", "self.released == false");
    assert_contains(&body, "record_sale", "self.listed == true");
}

#[test]
fn record_sale_expiry_formula_matches_rust() {
    // `.es`:  if epoch > self.list_opened_at + self.list_duration { expired }
    // Rust :  epoch_now > listing.opened_at.saturating_add(listing.duration)
    // Same boundary semantics: epoch == opened_at+duration is NOT expired
    // (strict `>`). Pin the `.es` formula so the boundary can't silently
    // drift to `>=` on one side only.
    let body = extract_fn_body(&contract_source(), "record_sale");
    assert!(
        body.contains("epoch > self.list_opened_at + self.list_duration"),
        "record_sale (.es) expiry must be strictly \
         `epoch > self.list_opened_at + self.list_duration` to match \
         Rust `epoch_now > opened_at + duration` (boundary epoch == \
         opened+duration is NOT expired on both sides). Drift to `>=` \
         on one side opens a one-epoch settlement window mismatch."
    );
    assert!(
        body.contains("listing has expired"),
        "record_sale (.es) must reject the expired case (the \
         `listing has expired` require) — Rust returns ListingExpired."
    );
}

#[test]
fn record_sale_has_no_caller_restriction() {
    // Doctrine: record_sale is permissionless — the listing state
    // (listed + not-expired + not-released) is the guard, exactly as
    // Rust vault.rs::record_sale takes no `caller`. If a future edit
    // added `caller == self.holder` here, an off-chain SDDC clear by
    // the winner could no longer be recorded — pin its absence.
    let body = extract_fn_body(&contract_source(), "record_sale");
    assert!(
        !body.contains("caller == self.holder"),
        "record_sale (.es) must NOT restrict caller — the listing \
         state guards it (parity with vault.rs::record_sale which \
         takes no caller arg). A caller check here breaks SDDC \
         winner-records-sale."
    );
}

// =================================================================
// Cross-cutting: record_sale clears the listing (parity with
// vault.rs setting `self.listing = None` + holder := winner).
// =================================================================

#[test]
fn record_sale_transfers_holder_and_clears_listing() {
    let body = extract_fn_body(&contract_source(), "record_sale");
    assert!(
        body.contains("self.holder = winner_addr"),
        "record_sale (.es) must set holder := winner (parity with \
         vault.rs status → Locked{{winner}})."
    );
    assert!(
        body.contains("self.listed = false"),
        "record_sale (.es) must clear `listed` (parity with \
         vault.rs setting listing = None on a recorded sale)."
    );
}

// =================================================================
// State-field parity: every `.es` listing state field the Rust
// `Listing` struct mirrors must exist in the `.es` state block.
// =================================================================

#[test]
fn es_declares_all_listing_state_fields() {
    let src = contract_source();
    for field in [
        "listed",
        "list_ceiling",
        "list_floor",
        "list_opened_at",
        "list_duration",
    ] {
        assert!(
            src.contains(field),
            "`.es` state is missing listing field `{field}` — the Rust \
             `Listing {{ ceiling, floor, opened_at, duration }}` + the \
             `listed`⇔`listing.is_some()` mapping require all of them."
        );
    }
}
