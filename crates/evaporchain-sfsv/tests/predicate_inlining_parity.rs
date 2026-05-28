//! Predicate-inlining parity check — closes gap #2 from
//! `research/SFSV_ARCHITECTURE.md` §10.2.
//!
//! EvaporScript has no contract-internal method dispatch (see
//! `evaporchain_evaporscript_grammar_gotchas.md`), so the predicate-
//! evaluation logic in `future_self_vault.es` is inlined in TWO
//! places:
//!
//!   - `fn try_payout()` — the state-mutating release entry point.
//!   - `fn predicate_satisfied() -> u64` — the read-only query.
//!
//! Both bodies are NOT byte-identical (one uses a `satisfied`
//! accumulator + `require`, the other uses early returns) but they
//! MUST be **semantically equivalent** with respect to the predicate
//! decision — otherwise the read-only query could report "released
//! now possible" while `try_payout` reverts (or vice versa), opening
//! a window for off-chain coordinators to misroute payouts.
//!
//! This file pins the semantic equivalence at the source level:
//!
//!   1. Both bodies must contain the EpochReached comparison
//!      `epoch >= self.release_epoch`.
//!   2. Both bodies must contain the EnergyDecaysBelow comparison
//!      `energy < self.threshold`.
//!   3. Each body must reference both predicate_type guards
//!      (`self.predicate_type == 0` and `self.predicate_type == 1`)
//!      so the dispatch is exhaustive.
//!   4. The two comparisons must appear in the same ORDER in each
//!      body (epoch check before energy check) — guards against a
//!      reviewer accidentally swapping order in one block but not
//!      the other.
//!
//! These checks are intentionally pattern-level, not bytecode-level.
//! A bytecode comparison would need the EvaporScript compiler at
//! test time (heavy dependency); pattern checks catch every drift
//! mode that has actually occurred in this repo's history.

use std::fs;
use std::path::PathBuf;

/// Relative path from the crate's `CARGO_MANIFEST_DIR` to the
/// canonical `.es` contract. If this file is renamed or moved, every
/// test in this module fails loudly — that is the desired behaviour,
/// since SFSV's contract location is doctrine-load-bearing.
fn contract_source() -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../contracts/evaporscript/future_self_vault.es");
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("could not read {}: {}", p.display(), e))
}

/// Extract the body of a top-level `fn <name>(...)` block from the
/// `.es` source. Returns everything between the opening `{` of the
/// function and its matching closing `}`. The extractor is whitespace-
/// and-comment tolerant: it finds the function signature by name and
/// then walks balanced braces. EvaporScript has C-style braces and
/// no nested function definitions, so brace counting suffices.
fn extract_fn_body(src: &str, fn_name: &str) -> String {
    // Find `fn <name>` token. Use `fn <name>(` to avoid matching
    // `fn predicate_satisfied` when looking for `fn predicate_`.
    let needle = format!("fn {fn_name}(");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("could not find `fn {fn_name}(` in contract source"));

    // Walk forward to the first `{` after the signature.
    let open = src[start..]
        .find('{')
        .unwrap_or_else(|| panic!("function `{fn_name}` has no opening brace"))
        + start;

    // Brace-count until balanced.
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut close = open;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    for i in open..bytes.len() {
        let c = bytes[i] as char;
        // Strip comments so braces inside `//` or `/* */` don't count.
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
    assert!(
        close > open,
        "function `{fn_name}` brace-walk did not close"
    );
    src[open + 1..close].to_string()
}

// =================================================================
// Predicate-inlining drift detection
// =================================================================

#[test]
fn try_payout_contains_epoch_reached_comparison() {
    let body = extract_fn_body(&contract_source(), "try_payout");
    assert!(
        body.contains("epoch >= self.release_epoch"),
        "try_payout is missing the EpochReached comparison \
         `epoch >= self.release_epoch`. Predicate evaluation must \
         remain byte-identical with predicate_satisfied — see \
         SFSV_ARCHITECTURE.md §10.2 gap #2 and the contract's own \
         line-comment at the top of try_payout."
    );
}

#[test]
fn try_payout_contains_energy_decays_below_comparison() {
    let body = extract_fn_body(&contract_source(), "try_payout");
    assert!(
        body.contains("energy < self.threshold"),
        "try_payout is missing the EnergyDecaysBelow comparison \
         `energy < self.threshold`. See SFSV_ARCHITECTURE.md §10.2 \
         gap #2."
    );
}

#[test]
fn predicate_satisfied_contains_epoch_reached_comparison() {
    let body = extract_fn_body(&contract_source(), "predicate_satisfied");
    assert!(
        body.contains("epoch >= self.release_epoch"),
        "predicate_satisfied is missing the EpochReached comparison. \
         Drift between try_payout and predicate_satisfied opens an \
         off-chain mis-routing window."
    );
}

#[test]
fn predicate_satisfied_contains_energy_decays_below_comparison() {
    let body = extract_fn_body(&contract_source(), "predicate_satisfied");
    assert!(
        body.contains("energy < self.threshold"),
        "predicate_satisfied is missing the EnergyDecaysBelow \
         comparison."
    );
}

#[test]
fn both_inlined_blocks_dispatch_on_predicate_type_zero_and_one() {
    let src = contract_source();
    for fn_name in ["try_payout", "predicate_satisfied"] {
        let body = extract_fn_body(&src, fn_name);
        assert!(
            body.contains("self.predicate_type == 0"),
            "fn {fn_name} missing dispatch on predicate_type == 0"
        );
        assert!(
            body.contains("self.predicate_type == 1"),
            "fn {fn_name} missing dispatch on predicate_type == 1"
        );
    }
}

#[test]
fn comparison_order_matches_across_both_blocks() {
    // If one block tested energy first and the other tested epoch
    // first, the bodies could still match all individual-presence
    // tests above while diverging in their visited-branch order —
    // not a correctness bug for the current 2-variant enum, but a
    // landmine the day a 3rd predicate variant is added. Pin the
    // order now.
    let src = contract_source();
    for fn_name in ["try_payout", "predicate_satisfied"] {
        let body = extract_fn_body(&src, fn_name);
        let epoch_idx = body
            .find("epoch >= self.release_epoch")
            .expect("epoch comparison present (checked above)");
        let energy_idx = body
            .find("energy < self.threshold")
            .expect("energy comparison present (checked above)");
        assert!(
            epoch_idx < energy_idx,
            "fn {fn_name}: predicate-type-0 (epoch) check must precede \
             predicate-type-1 (energy) check — divergence in visit \
             order across the two blocks is a future-drift landmine."
        );
    }
}

#[test]
fn predicate_satisfied_guards_unsealed_state() {
    // try_payout enforces `require(self.sealed == true)` early; the
    // read-only predicate_satisfied must return 0 (not panic, not
    // throw) when the contract has not been sealed yet. Pin this so
    // a refactor doesn't accidentally remove the sealed check —
    // off-chain coordinators read predicate_satisfied to decide
    // whether to broadcast a try_payout; returning 1 on an unsealed
    // contract would cause spurious payout attempts.
    let body = extract_fn_body(&contract_source(), "predicate_satisfied");
    assert!(
        body.contains("self.sealed == false"),
        "predicate_satisfied must guard on `self.sealed == false` — \
         the read-only query is the off-chain coordinator's signal."
    );
}

#[test]
fn try_payout_guards_released_state_before_predicate_evaluation() {
    // §8.5 (Adversary E: Replay) testable invariant at the source
    // level: try_payout must check `self.released == false` BEFORE
    // it touches the predicate. A reversed order would not be a
    // correctness bug (the require would still fire), but a strict
    // reading of "predicate evaluation is pure" requires the gate
    // to run before any state-derived comparison.
    let body = extract_fn_body(&contract_source(), "try_payout");
    let released_idx = body
        .find("self.released == false")
        .expect("released guard present");
    let predicate_idx = body
        .find("self.predicate_type")
        .expect("predicate dispatch present");
    assert!(
        released_idx < predicate_idx,
        "try_payout must check `released == false` BEFORE dispatching \
         on predicate_type — see SFSV_ARCHITECTURE.md §8.5 replay \
         mitigation."
    );
}

// =================================================================
// Rust-side parity — the substrate-crate evaluate() must agree with
// the .es semantics
// =================================================================

#[test]
fn rust_predicate_evaluate_path_count_matches_contract() {
    // The contract dispatches on 2 predicate types (0 and 1). The
    // Rust enum `Predicate` must therefore expose exactly 2 variants
    // discriminable in the source. We pin the contract-side count by
    // checking the dispatch branches; the Rust-side count is pinned
    // by the predicate.rs unit-test suite's exhaustive `match`.
    let src = contract_source();
    let try_body = extract_fn_body(&src, "try_payout");
    let pred_body = extract_fn_body(&src, "predicate_satisfied");

    let try_dispatch_count = try_body.matches("self.predicate_type ==").count();
    let pred_dispatch_count = pred_body.matches("self.predicate_type ==").count();
    assert_eq!(
        try_dispatch_count, pred_dispatch_count,
        "try_payout has {try_dispatch_count} dispatch branches; \
         predicate_satisfied has {pred_dispatch_count}. A divergence \
         here means one block has been extended without the other."
    );
    assert_eq!(
        try_dispatch_count, 2,
        "expected exactly 2 predicate variants (EpochReached + \
         EnergyDecaysBelow); contract has {try_dispatch_count}. \
         If you're adding a 3rd variant, this test reminds you to \
         update BOTH blocks AND the Rust evaluate() match."
    );
}
