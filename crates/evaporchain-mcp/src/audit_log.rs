//! Audit log of every MCP tool invocation — closes the third part
//! of `AUDIT_2026_05_06.md` CRITICAL-2.
//!
//! The audit said:
//!
//! > "No logging of which agent called which tool — attack
//! >  forensics impossible."
//!
//! Every call into [`crate::tools::call_tool`] now emits a
//! structured `tracing::info!` event with:
//!
//! - The tool name (the only piece of "who/what" available — MCP
//!   itself runs over stdin, so there's no per-request agent
//!   identity to log).
//! - The arg shape (sorted field names, no values). Logging
//!   values would leak secrets / private data; logging shape is
//!   enough to reconstruct attack patterns post-hoc.
//! - The outcome (`ok` / `err: <message>`).
//! - A monotonic invocation counter so repeated invocations of
//!   the same tool with the same args are still distinguishable
//!   in log replay.
//!
//! This module's [`AuditEvent`] type is the pure formatted shape;
//! emission goes through `tracing::info!` so operator log-aggregators
//! (Prometheus exporter, Loki, Splunk, journald, etc.) can index
//! it like any other structured event.
//!
//! # Privacy invariants
//!
//! - Values are NEVER logged — only field names, sorted alphabetically.
//! - Errors are surfaced verbatim (the input validator's
//!   `ValidationError::Display` produces operator-readable but
//!   secret-free strings; backend errors are JSON shaped and may
//!   carry hex strings, but those are addresses/ids the agent
//!   already knew, not new disclosures).
//! - Successful tool calls log `outcome="ok"` only, NOT the
//!   response body. Operators correlate with backend logs by
//!   timestamp + counter.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

/// Process-wide monotonic invocation counter. Wraps after 2^64
/// invocations (~5×10^11 years at 1k-tools/sec, so practically
/// never).
static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Outcome of a tool invocation, as audit-logged.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum AuditOutcome {
    /// Tool ran end-to-end and returned a value.
    Ok,
    /// Tool failed (validation rejection, backend error, etc.). The
    /// `String` is the operator-readable error message — never a
    /// raw arg value.
    Err(String),
}

impl AuditOutcome {
    /// String form for log emission. `"ok"` or `"err: <message>"`.
    pub fn to_log_str(&self) -> String {
        match self {
            AuditOutcome::Ok => "ok".to_string(),
            AuditOutcome::Err(msg) => format!("err: {msg}"),
        }
    }
}

/// Pure formatted shape of one audit event. Built from the tool
/// name + agent args + outcome; no values from `args` are
/// preserved — only the sorted list of field names.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct AuditEvent {
    /// Process-monotonic invocation counter, assigned at build
    /// time. Cheap collisions across processes (each restart
    /// resets to 0); operator log-aggregators must combine with
    /// timestamp for cross-process uniqueness.
    pub seq: u64,
    /// Name of the MCP tool invoked.
    pub tool: String,
    /// Sorted list of arg field names. Empty if `args` is not an
    /// object or has no fields.
    pub arg_fields: Vec<String>,
    /// Outcome of the call.
    pub outcome: AuditOutcome,
}

impl AuditEvent {
    /// Build an audit event from the tool dispatch inputs +
    /// outcome. Increments the global invocation counter.
    pub fn build(tool: &str, args: &Value, outcome: AuditOutcome) -> Self {
        let seq = INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let arg_fields = args
            .as_object()
            .map(|m| {
                let mut keys: Vec<String> = m.keys().cloned().collect();
                keys.sort();
                keys
            })
            .unwrap_or_default();
        AuditEvent {
            seq,
            tool: tool.to_string(),
            arg_fields,
            outcome,
        }
    }

    /// Single-line log format for operator consumption. Pure
    /// function — emission via `tracing::info!` happens at the
    /// call site so structured-log subscribers can extract the
    /// fields independently.
    pub fn to_log_line(&self) -> String {
        format!(
            "mcp_audit seq={} tool={} args={:?} outcome={}",
            self.seq,
            self.tool,
            self.arg_fields,
            self.outcome.to_log_str()
        )
    }
}

/// Reset the invocation counter. **Test-only** — production should
/// never call this; the counter is process-monotonic by design.
#[cfg(test)]
pub fn reset_counter_for_test() {
    INVOCATION_COUNTER.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fresh_counter() {
        reset_counter_for_test();
    }

    #[test]
    fn audit_event_captures_tool_name() {
        fresh_counter();
        let args = json!({});
        let ev = AuditEvent::build("transfer", &args, AuditOutcome::Ok);
        assert_eq!(ev.tool, "transfer");
        assert_eq!(ev.outcome, AuditOutcome::Ok);
    }

    #[test]
    fn audit_event_records_arg_field_names_sorted() {
        fresh_counter();
        let args = json!({ "to": "x", "from": "y", "amount": 1 });
        let ev = AuditEvent::build("transfer", &args, AuditOutcome::Ok);
        assert_eq!(ev.arg_fields, vec!["amount", "from", "to"]);
    }

    #[test]
    fn audit_event_does_not_capture_arg_values() {
        // Privacy invariant: values are never recorded. The event
        // must contain only field names; if a future refactor
        // accidentally captures values, this test catches it.
        fresh_counter();
        let secret = "0xdeadbeef00000000000000000000000000000000000000000000000000000000";
        let args = json!({ "from": secret, "amount": 999_999 });
        let ev = AuditEvent::build("transfer", &args, AuditOutcome::Ok);
        let line = ev.to_log_line();
        assert!(
            !line.contains(secret),
            "audit log line must NOT contain hex arg values; got: {line}"
        );
        // The amount value as a substring search: 999_999 written
        // as a json! literal becomes "999999" with no separators;
        // we check that 6-digit sequence specifically (3-digit
        // checks would false-positive on incidental seq counter
        // values under parallel test execution).
        assert!(
            !line.contains("999999"),
            "audit log line must NOT contain numeric arg values; got: {line}"
        );
    }

    #[test]
    fn audit_event_handles_non_object_args() {
        fresh_counter();
        // E.g. agent sent {"arguments": "raw-string"} or similar.
        // arg_fields is empty.
        let args = json!("a-string");
        let ev = AuditEvent::build("get_chain_status", &args, AuditOutcome::Ok);
        assert!(ev.arg_fields.is_empty());
    }

    #[test]
    fn audit_event_handles_missing_args_field() {
        fresh_counter();
        let args = json!({});
        let ev = AuditEvent::build("get_chain_status", &args, AuditOutcome::Ok);
        assert!(ev.arg_fields.is_empty());
    }

    #[test]
    fn audit_event_seq_monotone() {
        // Test in parallel-safe form: assert relative monotonicity
        // within this test's three consecutive builds, NOT specific
        // counter values. Other tests share the global counter so
        // exact starting values are non-deterministic under parallel
        // test execution.
        let args = json!({});
        let a = AuditEvent::build("transfer", &args, AuditOutcome::Ok);
        let b = AuditEvent::build("transfer", &args, AuditOutcome::Ok);
        let c = AuditEvent::build("get_chain_status", &args, AuditOutcome::Ok);
        assert!(b.seq > a.seq, "seq must increase: {} -> {}", a.seq, b.seq);
        assert!(c.seq > b.seq, "seq must increase: {} -> {}", b.seq, c.seq);
    }

    #[test]
    fn audit_outcome_err_is_logged_with_message() {
        let outcome = AuditOutcome::Err("Field 'amount' = 0 is out of range [1, ...]".into());
        let s = outcome.to_log_str();
        assert!(s.starts_with("err: "));
        assert!(s.contains("amount"));
    }

    #[test]
    fn audit_outcome_ok_is_logged_as_ok() {
        let outcome = AuditOutcome::Ok;
        assert_eq!(outcome.to_log_str(), "ok");
    }

    #[test]
    fn audit_log_line_is_grep_friendly() {
        let args = json!({ "from": "0x...", "to": "0x...", "amount": 100 });
        let ev = AuditEvent::build("transfer", &args, AuditOutcome::Ok);
        let line = ev.to_log_line();
        // Operator-grep contract — these substrings must appear
        // and operators rely on them for forensic queries. The
        // exact seq value depends on parallel-test ordering;
        // assert "seq=" appears (with some integer) rather than
        // a specific value.
        assert!(line.contains("mcp_audit"));
        assert!(
            line.contains(&format!("seq={}", ev.seq)),
            "line must include seq={} for this event; got: {line}",
            ev.seq
        );
        assert!(line.contains("tool=transfer"));
        assert!(line.contains("outcome=ok"));
    }
}
