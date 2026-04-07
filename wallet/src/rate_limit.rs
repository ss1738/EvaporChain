//! Token-bucket rate limiter for RPC calls.
//!
//! Prevents hammering the node by enforcing a maximum number of
//! requests per second. Uses a simple token-bucket algorithm that
//! refills tokens at a constant rate.

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

// ──────────────────────────── Rate Limiter ────────────────────────────────

/// Token-bucket rate limiter.
///
/// Call `acquire()` before each RPC request. It will sleep if the
/// bucket is empty, ensuring requests don't exceed the configured rate.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
}

#[derive(Debug)]
struct RateLimiterInner {
    /// Maximum tokens (burst capacity).
    max_tokens: f64,
    /// Current available tokens.
    tokens: f64,
    /// Tokens added per second.
    refill_rate: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// - `requests_per_second`: sustained request rate
    /// - `burst`: maximum burst size (requests that can fire instantly)
    pub fn new(requests_per_second: f64, burst: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                max_tokens: burst as f64,
                tokens: burst as f64,
                refill_rate: requests_per_second,
                last_refill: Instant::now(),
            })),
        }
    }

    /// Default limiter: 10 req/s with burst of 20.
    pub fn default_rpc() -> Self {
        Self::new(10.0, 20)
    }

    /// Unlimited — never throttles. For tests or local nodes.
    pub fn unlimited() -> Self {
        Self::new(f64::MAX, u32::MAX)
    }

    /// Acquire a token. Sleeps if the bucket is empty.
    pub async fn acquire(&self) {
        loop {
            let sleep_dur = {
                let mut inner = self.inner.lock().await;
                inner.refill();

                if inner.tokens >= 1.0 {
                    inner.tokens -= 1.0;
                    return;
                }

                // Calculate how long to wait for 1 token
                let deficit = 1.0 - inner.tokens;
                Duration::from_secs_f64(deficit / inner.refill_rate)
            };

            tokio::time::sleep(sleep_dur).await;
        }
    }

    /// Try to acquire without blocking. Returns true if a token was available.
    pub async fn try_acquire(&self) -> bool {
        let mut inner = self.inner.lock().await;
        inner.refill();
        if inner.tokens >= 1.0 {
            inner.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Current number of available tokens.
    pub async fn available(&self) -> f64 {
        let mut inner = self.inner.lock().await;
        inner.refill();
        inner.tokens
    }
}

impl RateLimiterInner {
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acquire_succeeds_within_burst() {
        let limiter = RateLimiter::new(10.0, 5);
        // Should be able to acquire 5 tokens without waiting
        for _ in 0..5 {
            limiter.acquire().await;
        }
    }

    #[tokio::test]
    async fn test_try_acquire_drains_bucket() {
        let limiter = RateLimiter::new(10.0, 3);
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);
        // Bucket should be empty now
        assert!(!limiter.try_acquire().await);
    }

    #[tokio::test]
    async fn test_tokens_refill_over_time() {
        let limiter = RateLimiter::new(100.0, 5); // 100/s = 1 token per 10ms
        // Drain the bucket
        for _ in 0..5 {
            limiter.try_acquire().await;
        }
        assert!(!limiter.try_acquire().await);

        // Wait for refill
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Should have refilled some tokens
        assert!(limiter.try_acquire().await);
    }

    #[tokio::test]
    async fn test_available_reflects_state() {
        let limiter = RateLimiter::new(10.0, 10);
        let avail = limiter.available().await;
        assert!((avail - 10.0).abs() < 0.5);

        limiter.acquire().await;
        let avail = limiter.available().await;
        assert!((avail - 9.0).abs() < 0.5);
    }

    #[tokio::test]
    async fn test_unlimited_never_blocks() {
        let limiter = RateLimiter::unlimited();
        for _ in 0..1000 {
            limiter.acquire().await;
        }
    }

    #[tokio::test]
    async fn test_default_rpc_settings() {
        let limiter = RateLimiter::default_rpc();
        let avail = limiter.available().await;
        assert!((avail - 20.0).abs() < 0.5); // burst=20
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let limiter = RateLimiter::new(10.0, 5);
        let limiter2 = limiter.clone();

        // Drain from one clone
        for _ in 0..5 {
            limiter.acquire().await;
        }

        // Other clone should see empty bucket
        assert!(!limiter2.try_acquire().await);
    }

    #[tokio::test]
    async fn test_acquire_blocks_then_succeeds() {
        let limiter = RateLimiter::new(100.0, 1); // 1 burst, 100/s refill
        limiter.acquire().await; // use the 1 burst token

        let start = Instant::now();
        limiter.acquire().await; // should block ~10ms
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(5)); // some wait happened
    }
}
