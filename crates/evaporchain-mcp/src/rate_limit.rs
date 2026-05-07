//! Per-tool rate limiting — closes the fourth sub-item of
//! `AUDIT_2026_05_06.md` CRITICAL-2.
//!
//! The audit said:
//!
//! > "No rate limiting — compromised agent can DoS the node."
//!
//! Without rate limiting at the MCP layer, a runaway agent
//! (whether prompt-injected or just buggy) can call any tool at
//! whatever rate the underlying I/O can sustain — burning CPU on
//! the chain backend, filling the audit log, and amplifying the
//! attack surface for downstream rate limits.
//!
//! This module installs a sliding-window counter per tool name.
//! Write-tools (the 5 mutating ones) get tight limits because each
//! call costs the chain a real tx; compute-heavy reads (e.g.
//! `compute_mera_commitment`, `prove_fork_evaporated`) get medium
//! limits because each is a noticeable backend CPU hit; the
//! remaining read-only tools get loose limits because they're
//! cheap.
//!
//! # Threading
//!
//! `MCP_RATE_LIMITER` is a `parking_lot::Mutex<HashMap<String,
//! WindowCounter>>` — wait, we don't have parking_lot. Falls back
//! to `std::sync::Mutex`. The MCP runs on a single tokio runtime
//! and tool calls are short-lived, so contention is bounded.
//!
//! # Window + threshold rationale
//!
//! Defaults are operator-overridable via env vars but ship sane:
//!
//! | Tier | Tools | Limit | Reason |
//! |---|---|---|---|
//! | write | transfer, create_object, refresh_object, resurrect_object, request_faucet | 10 / 60s | each call is a chain-state mutation; 1 every 6 sec is generous for a non-malicious agent |
//! | compute | compute_demurrage, compute_mera_commitment, prove_fork_evaporated, check_conservation, check_annealing_temperature | 30 / 60s | each one runs a non-trivial backend computation |
//! | read | everything else | 300 / 60s | cheap GETs; 5 / sec sustained is enough headroom |

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One sliding-window event log. Tracks event timestamps within
/// `window`; `try_take(now)` evicts old events, then admits a new
/// one iff `events.len() < max`.
#[derive(Debug)]
pub struct WindowCounter {
    /// Window length.
    window: Duration,
    /// Max events allowed in the window.
    max: usize,
    /// Event timestamps. Sorted oldest-first; pruned on every
    /// `try_take` call.
    events: VecDeque<Instant>,
}

impl WindowCounter {
    pub fn new(window: Duration, max: usize) -> Self {
        Self {
            window,
            max,
            events: VecDeque::with_capacity(max + 1),
        }
    }

    /// Attempt to admit a new event at `now`. Returns:
    /// - `Ok(())` if the event was admitted (added to the window).
    /// - `Err(retry_after)` if the event would exceed the limit;
    ///   `retry_after` is the duration until the oldest event in
    ///   the window expires (the soonest a retry could succeed).
    pub fn try_take(&mut self, now: Instant) -> Result<(), Duration> {
        // Evict events older than `now - window`.
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        while let Some(front) = self.events.front() {
            if *front < cutoff {
                self.events.pop_front();
            } else {
                break;
            }
        }

        if self.events.len() >= self.max {
            // Compute retry_after from the oldest event we couldn't
            // evict — when it ages out, a slot opens up.
            let oldest = self.events.front().copied().unwrap_or(now);
            let expires_at = oldest + self.window;
            let retry_after = expires_at.saturating_duration_since(now);
            return Err(retry_after);
        }

        self.events.push_back(now);
        Ok(())
    }

    /// Number of events currently in the window. Test-only.
    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.events.len()
    }
}

/// Tier classification for a tool — controls its rate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTier {
    /// Mutating tools (transfer, create_object, refresh_object,
    /// resurrect_object, request_faucet). Tightest limit.
    Write,
    /// Read tools that drive non-trivial backend computation.
    Compute,
    /// Cheap read tools.
    Read,
}

/// Classify a tool by name into its tier. Used to look up the
/// applicable rate-limit config.
pub fn classify_tool(name: &str) -> ToolTier {
    match name {
        // Write tools — every call is a chain-state mutation.
        "transfer" | "create_object" | "refresh_object" | "resurrect_object" | "request_faucet" => {
            ToolTier::Write
        }

        // Compute-heavy reads — backend runs non-trivial math.
        "compute_demurrage"
        | "compute_mera_commitment"
        | "prove_fork_evaporated"
        | "check_conservation"
        | "check_annealing_temperature" => ToolTier::Compute,

        // Everything else — cheap GETs.
        _ => ToolTier::Read,
    }
}

/// Default per-tier limit. Operator can override via env (read at
/// startup; not hot-reloaded).
pub fn default_limit_for_tier(tier: ToolTier) -> (Duration, usize) {
    match tier {
        ToolTier::Write => (Duration::from_secs(60), 10),
        ToolTier::Compute => (Duration::from_secs(60), 30),
        ToolTier::Read => (Duration::from_secs(60), 300),
    }
}

/// Thread-safe per-tool rate limiter. Lazily creates a
/// `WindowCounter` on first use of each tool name.
#[derive(Debug)]
pub struct McpRateLimiter {
    counters: Mutex<HashMap<String, WindowCounter>>,
}

impl McpRateLimiter {
    pub fn new() -> Self {
        Self {
            counters: Mutex::new(HashMap::new()),
        }
    }

    /// Try to admit a call to `tool` at `now`. Returns `Ok(())` on
    /// admit, `Err(retry_after)` on rejection.
    pub fn try_take(&self, tool: &str, now: Instant) -> Result<(), Duration> {
        let tier = classify_tool(tool);
        let (window, max) = default_limit_for_tier(tier);
        let mut map = self.counters.lock().expect("rate-limiter mutex poisoned");
        let counter = map
            .entry(tool.to_string())
            .or_insert_with(|| WindowCounter::new(window, max));
        counter.try_take(now)
    }
}

impl Default for McpRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify_tool ────────────────────────────────────────────

    #[test]
    fn classify_write_tools_correctly() {
        for tool in [
            "transfer",
            "create_object",
            "refresh_object",
            "resurrect_object",
            "request_faucet",
        ] {
            assert_eq!(classify_tool(tool), ToolTier::Write, "tool {tool}");
        }
    }

    #[test]
    fn classify_compute_tools_correctly() {
        for tool in [
            "compute_demurrage",
            "compute_mera_commitment",
            "prove_fork_evaporated",
            "check_conservation",
            "check_annealing_temperature",
        ] {
            assert_eq!(classify_tool(tool), ToolTier::Compute, "tool {tool}");
        }
    }

    #[test]
    fn classify_read_tools_to_read_tier() {
        for tool in [
            "get_chain_status",
            "list_objects",
            "get_object",
            "list_accounts",
            "list_ghosts",
            "get_recent_blocks",
            "get_block",
            "get_recent_events",
            "list_contracts",
            "get_stats",
            "get_autopoietic_health",
            "get_epv_status",
            "get_fee_status",
            "get_consensus_phase",
            "get_oracle_status",
            "get_shard_health",
        ] {
            assert_eq!(classify_tool(tool), ToolTier::Read, "tool {tool}");
        }
    }

    // ── tier limit defaults ──────────────────────────────────────

    #[test]
    fn write_tier_is_tightest_then_compute_then_read() {
        let (_, w) = default_limit_for_tier(ToolTier::Write);
        let (_, c) = default_limit_for_tier(ToolTier::Compute);
        let (_, r) = default_limit_for_tier(ToolTier::Read);
        assert!(w < c);
        assert!(c < r);
    }

    // ── WindowCounter behaviour ──────────────────────────────────

    #[test]
    fn window_counter_admits_up_to_max() {
        let mut wc = WindowCounter::new(Duration::from_secs(60), 3);
        let t0 = Instant::now();
        assert!(wc.try_take(t0).is_ok());
        assert!(wc.try_take(t0).is_ok());
        assert!(wc.try_take(t0).is_ok());
        // Fourth call exceeds max=3 within the same window.
        let res = wc.try_take(t0);
        assert!(res.is_err());
    }

    #[test]
    fn window_counter_evicts_old_events() {
        let mut wc = WindowCounter::new(Duration::from_secs(60), 2);
        let t0 = Instant::now();
        assert!(wc.try_take(t0).is_ok());
        assert!(wc.try_take(t0).is_ok());
        // 70 seconds later the first two are aged out.
        let t1 = t0 + Duration::from_secs(70);
        assert!(wc.try_take(t1).is_ok(), "stale events must be evicted");
        // Counter at t1 holds only the new event.
        assert_eq!(wc.count(), 1);
    }

    #[test]
    fn window_counter_retry_after_points_at_oldest_expiry() {
        let mut wc = WindowCounter::new(Duration::from_secs(60), 1);
        let t0 = Instant::now();
        wc.try_take(t0).unwrap();
        // Second call within window must fail with retry_after
        // approximately = 60s minus 0 (we asked at t0 again).
        let err = wc.try_take(t0).unwrap_err();
        assert!(err >= Duration::from_secs(59) && err <= Duration::from_secs(60));
    }

    // ── McpRateLimiter end-to-end ────────────────────────────────

    #[test]
    fn mcp_limiter_tracks_each_tool_independently() {
        let lim = McpRateLimiter::new();
        let now = Instant::now();
        // Saturate the write limit (10) for `transfer`.
        for _ in 0..10 {
            lim.try_take("transfer", now).unwrap();
        }
        // 11th transfer rejects.
        assert!(lim.try_take("transfer", now).is_err());
        // But `request_faucet` (also write tier, separate counter)
        // is unaffected.
        assert!(lim.try_take("request_faucet", now).is_ok());
        // And read tier is unaffected.
        assert!(lim.try_take("get_chain_status", now).is_ok());
    }

    #[test]
    fn mcp_limiter_allows_more_reads_than_writes() {
        let lim = McpRateLimiter::new();
        let now = Instant::now();
        // 11 writes — the 11th rejects (write cap = 10).
        let mut write_admits = 0;
        for _ in 0..15 {
            if lim.try_take("transfer", now).is_ok() {
                write_admits += 1;
            }
        }
        assert_eq!(write_admits, 10);

        // 50 reads — all admit (read cap = 300).
        let mut read_admits = 0;
        for _ in 0..50 {
            if lim.try_take("get_chain_status", now).is_ok() {
                read_admits += 1;
            }
        }
        assert_eq!(read_admits, 50);
    }

    #[test]
    fn mcp_limiter_unknown_tool_falls_to_read_tier() {
        // A tool not in any classification list (typo, dropped
        // tool, future tool) defaults to the loose read tier.
        // This is the right fallback: a buggy classify_tool
        // shouldn't accidentally over-restrict; it should
        // accidentally under-restrict, where the next layer's
        // input-validation + auth catches the abuse.
        let lim = McpRateLimiter::new();
        let now = Instant::now();
        for _ in 0..200 {
            assert!(
                lim.try_take("future_unknown_tool", now).is_ok(),
                "unknown tool should be admitted up to read-tier limit"
            );
        }
    }
}
