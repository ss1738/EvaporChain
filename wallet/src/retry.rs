//! Retry logic with exponential backoff for RPC calls.
//!
//! Wraps async functions with configurable retry behavior:
//! - Exponential backoff (base_delay × 2^attempt)
//! - Maximum retry count
//! - Jitter to prevent thundering herd
//! - Only retries on transient errors (network, timeout, 5xx)

use std::time::Duration;

// ──────────────────────────── Config ──────────────────────────────────

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Base delay between retries (doubled each attempt).
    pub base_delay: Duration,
    /// Maximum delay cap.
    pub max_delay: Duration,
    /// Add random jitter (0-50% of delay).
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
            jitter: true,
        }
    }
}

impl RetryConfig {
    /// No retries — fail immediately.
    pub fn no_retry() -> Self {
        Self {
            max_retries: 0,
            ..Default::default()
        }
    }

    /// Aggressive retry for critical operations.
    pub fn aggressive() -> Self {
        Self {
            max_retries: 5,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(30),
            jitter: true,
        }
    }

    /// Compute the delay for a given attempt (0-based).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base = self.base_delay.as_millis() as u64;
        let multiplier = 1u64 << attempt.min(10); // 2^attempt, capped
        let delay_ms = base.saturating_mul(multiplier);
        let capped = delay_ms.min(self.max_delay.as_millis() as u64);

        if self.jitter {
            // Add 0-50% jitter
            let jitter = (capped as f64 * rand::random::<f64>() * 0.5) as u64;
            Duration::from_millis(capped + jitter)
        } else {
            Duration::from_millis(capped)
        }
    }
}

// ──────────────────────────── Retry Function ──────────────────────────

/// Whether an error is transient and should be retried.
pub fn is_transient(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("timeout")
        || lower.contains("connection")
        || lower.contains("network")
        || lower.contains("timed out")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("temporarily unavailable")
        || lower.contains("retry")
}

/// Execute an async function with retry logic.
///
/// Only retries when `should_retry` returns true for the error.
pub async fn with_retry<F, Fut, T, E>(config: &RetryConfig, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_error: Option<E> = None;

    for attempt in 0..=config.max_retries {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let msg = e.to_string();
                if attempt < config.max_retries && is_transient(&msg) {
                    let delay = config.delay_for_attempt(attempt);
                    tokio::time::sleep(delay).await;
                    last_error = Some(e);
                } else {
                    return Err(e);
                }
            }
        }
    }

    Err(last_error.unwrap())
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay, Duration::from_millis(500));
    }

    #[test]
    fn test_no_retry() {
        let config = RetryConfig::no_retry();
        assert_eq!(config.max_retries, 0);
    }

    #[test]
    fn test_delay_exponential() {
        let config = RetryConfig {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            jitter: false,
            max_retries: 5,
        };

        assert_eq!(config.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(config.delay_for_attempt(3), Duration::from_millis(800));
    }

    #[test]
    fn test_delay_capped() {
        let config = RetryConfig {
            base_delay: Duration::from_millis(1000),
            max_delay: Duration::from_millis(5000),
            jitter: false,
            max_retries: 5,
        };

        // 1000 * 2^3 = 8000, but capped at 5000
        assert_eq!(config.delay_for_attempt(3), Duration::from_millis(5000));
    }

    #[test]
    fn test_delay_with_jitter() {
        let config = RetryConfig {
            base_delay: Duration::from_millis(1000),
            max_delay: Duration::from_secs(60),
            jitter: true,
            max_retries: 3,
        };

        let delay = config.delay_for_attempt(0);
        // Should be between 1000 and 1500 (1000 + 0-50% jitter)
        assert!(delay >= Duration::from_millis(1000));
        assert!(delay <= Duration::from_millis(1500));
    }

    #[test]
    fn test_is_transient() {
        assert!(is_transient("connection refused"));
        assert!(is_transient("request timed out"));
        assert!(is_transient("502 Bad Gateway"));
        assert!(is_transient("503 Service Temporarily Unavailable"));
        assert!(is_transient("network error"));

        assert!(!is_transient("invalid address"));
        assert!(!is_transient("insufficient balance"));
        assert!(!is_transient("nonce too low"));
    }

    #[tokio::test]
    async fn test_retry_success_first_try() {
        let config = RetryConfig::no_retry();
        let result: Result<i32, String> = with_retry(&config, || async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_non_transient_fails_immediately() {
        let config = RetryConfig::default();
        let attempt_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count = attempt_count.clone();

        let result: Result<i32, String> = with_retry(&config, || {
            let c = count.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err::<i32, String>("invalid address".to_string())
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempt_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // ─── Additional coverage tests (session 62) ───────────────────────────────

    #[test]
    fn test_aggressive_config() {
        let config = RetryConfig::aggressive();
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.base_delay, Duration::from_millis(200));
        assert_eq!(config.max_delay, Duration::from_secs(30));
        assert!(config.jitter);
    }

    #[tokio::test]
    async fn test_retry_transient_then_success_covers_sleep_path() {
        // Fail once with a transient error (covers lines 106-108), then succeed.
        let config = RetryConfig {
            max_retries: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
            jitter: false,
        };
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let a = attempts.clone();

        let result: Result<i32, String> = with_retry(&config, || {
            let c = a.clone();
            async move {
                let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    Err("connection refused".to_string())
                } else {
                    Ok(99)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn test_is_transient_covers_all_keywords() {
        assert!(is_transient("504 Gateway Timeout"));
        assert!(is_transient("temporarily unavailable"));
        assert!(is_transient("please retry later"));
        assert!(is_transient("timed out after 30s"));
        assert!(!is_transient("bad request"));
    }
}
