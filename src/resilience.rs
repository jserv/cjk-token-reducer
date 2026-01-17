//! Resilience patterns for fault-tolerant API calls
//!
//! Implements circuit breaker and rate limiting backpressure for Google Translate API.

use crate::config::ResilienceConfig;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation - requests pass through
    Closed,
    /// Circuit open - requests fail immediately
    Open,
    /// Testing if service recovered - allowing single request
    HalfOpen,
}

/// Thread-safe circuit breaker for API failure protection
///
/// Prevents cascading failures by failing fast when the API is unavailable.
/// Uses atomic operations for lock-free thread safety.
pub struct CircuitBreaker {
    /// Consecutive failure count
    failure_count: AtomicU32,
    /// Failure threshold before opening circuit
    threshold: u32,
    /// Timestamp when circuit was opened (0 = closed)
    opened_at: AtomicU64,
    /// Reset timeout in seconds
    reset_timeout_secs: u64,
    /// Total failures recorded (for stats)
    total_failures: AtomicU32,
    /// Total successful calls after circuit opened (for stats)
    recoveries: AtomicU32,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with configuration
    pub fn new(config: &ResilienceConfig) -> Self {
        Self {
            failure_count: AtomicU32::new(0),
            threshold: config.circuit_breaker_threshold,
            opened_at: AtomicU64::new(0),
            reset_timeout_secs: config.circuit_breaker_reset_secs,
            total_failures: AtomicU32::new(0),
            recoveries: AtomicU32::new(0),
        }
    }

    /// Create with explicit parameters (for testing)
    pub fn with_params(threshold: u32, reset_timeout_secs: u64) -> Self {
        Self {
            failure_count: AtomicU32::new(0),
            threshold,
            opened_at: AtomicU64::new(0),
            reset_timeout_secs,
            total_failures: AtomicU32::new(0),
            recoveries: AtomicU32::new(0),
        }
    }

    /// Get current circuit state
    pub fn state(&self) -> CircuitState {
        let opened_at = self.opened_at.load(Ordering::Acquire);
        if opened_at == 0 {
            return CircuitState::Closed;
        }

        let now = current_timestamp_secs();
        let elapsed = now.saturating_sub(opened_at);

        if elapsed >= self.reset_timeout_secs {
            CircuitState::HalfOpen
        } else {
            CircuitState::Open
        }
    }

    /// Check if request should be allowed through
    ///
    /// Race condition fix: In HalfOpen state, we must verify opened_at is non-zero
    /// before attempting CAS. If another thread called record_success() and set
    /// opened_at to 0 (Closed), we should allow the request (circuit is now closed).
    pub fn allow_request(&self) -> bool {
        loop {
            let opened_at = self.opened_at.load(Ordering::Acquire);

            // Circuit is closed - allow request
            if opened_at == 0 {
                return true;
            }

            let now = current_timestamp_secs();
            let elapsed = now.saturating_sub(opened_at);

            // Circuit is open (not yet timed out) - reject request
            if elapsed < self.reset_timeout_secs {
                return false;
            }

            // Circuit is half-open - try to claim the test slot
            // CAS: if opened_at unchanged, update to current time to prevent other threads
            match self.opened_at.compare_exchange_weak(
                opened_at,
                now,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true, // Successfully claimed test slot
                Err(_) => continue,   // Another thread modified state, retry
            }
        }
    }

    /// Record a successful call - resets failure count and closes circuit
    ///
    /// Uses CAS to atomically close the circuit, preventing race where another
    /// thread could increment failure_count and re-open immediately after success.
    pub fn record_success(&self) {
        // Try to close the circuit atomically - only if it's currently open
        let opened_at = self.opened_at.load(Ordering::Acquire);
        if opened_at != 0 {
            // Use CAS: only close if still open (another thread might have already closed it)
            if self
                .opened_at
                .compare_exchange(opened_at, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.recoveries.fetch_add(1, Ordering::Relaxed);
            }
        }
        // Always reset failure count on success
        self.failure_count.store(0, Ordering::Release);
    }

    /// Record a failed call - may open circuit
    ///
    /// Only sets opened_at when transitioning from closed to open state.
    /// This prevents extending the open window on repeated failures.
    pub fn record_failure(&self) {
        self.total_failures.fetch_add(1, Ordering::Relaxed);
        let failures = self.failure_count.fetch_add(1, Ordering::AcqRel) + 1;

        if failures >= self.threshold {
            // Only open if currently closed (opened_at == 0)
            // This prevents extending the open window on repeated failures
            self.opened_at
                .compare_exchange(
                    0,
                    current_timestamp_secs(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .ok(); // Ignore result - if already open, that's fine
        }
    }

    /// Get statistics for monitoring
    pub fn stats(&self) -> CircuitBreakerStats {
        CircuitBreakerStats {
            state: self.state(),
            failure_count: self.failure_count.load(Ordering::Acquire),
            threshold: self.threshold,
            total_failures: self.total_failures.load(Ordering::Acquire),
            recoveries: self.recoveries.load(Ordering::Acquire),
        }
    }

    /// Reset circuit breaker to closed state (for testing/admin)
    pub fn reset(&self) {
        self.failure_count.store(0, Ordering::Release);
        self.opened_at.store(0, Ordering::Release);
    }
}

/// Statistics about circuit breaker state
#[derive(Debug, Clone)]
pub struct CircuitBreakerStats {
    pub state: CircuitState,
    pub failure_count: u32,
    pub threshold: u32,
    pub total_failures: u32,
    pub recoveries: u32,
}

impl std::fmt::Display for CircuitBreakerStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Circuit: {:?} ({}/{} failures, {} total, {} recoveries)",
            self.state, self.failure_count, self.threshold, self.total_failures, self.recoveries
        )
    }
}

/// Rate limiter with backpressure for 429 responses
///
/// Implements adaptive rate limiting based on API responses.
/// Uses reservation-based scheduling to prevent thundering herd.
///
/// Thundering herd fix: Instead of tracking last_request (which causes multiple
/// threads to read the same value, sleep together, and wake together), we track
/// next_allowed timestamp. Each thread atomically advances this to reserve its slot,
/// ensuring requests are properly spaced even under concurrent load.
pub struct RateLimiter {
    /// Minimum delay between requests in milliseconds
    min_delay_ms: AtomicU64,
    /// Next allowed request timestamp (reservation-based)
    next_allowed_ms: AtomicU64,
    /// Backoff multiplier when rate limited
    backoff_multiplier: f64,
    /// Maximum delay cap in milliseconds
    max_delay_ms: u64,
    /// Count of rate limit hits
    rate_limit_hits: AtomicU32,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new() -> Self {
        Self {
            min_delay_ms: AtomicU64::new(0), // Start with no delay
            next_allowed_ms: AtomicU64::new(0),
            backoff_multiplier: 2.0,
            max_delay_ms: 30_000, // 30 second max delay
            rate_limit_hits: AtomicU32::new(0),
        }
    }

    /// Wait if needed before making a request
    ///
    /// Uses atomic reservation to prevent thundering herd:
    /// Each caller reserves a time slot by advancing next_allowed_ms,
    /// then waits until their reserved slot arrives.
    pub async fn wait_if_needed(&self) {
        let min_delay = self.min_delay_ms.load(Ordering::Acquire);
        if min_delay == 0 {
            return;
        }

        let now = current_timestamp_ms();

        // Atomically reserve next slot: advance next_allowed by min_delay
        // fetch_update ensures each thread gets a unique reservation
        let my_slot = loop {
            let current_next = self.next_allowed_ms.load(Ordering::Acquire);
            // My slot is either now (if we're past next_allowed) or next_allowed
            let effective_next = current_next.max(now);
            let new_next = effective_next + min_delay;

            match self.next_allowed_ms.compare_exchange_weak(
                current_next,
                new_next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break effective_next,
                Err(_) => continue, // Another thread updated, retry
            }
        };

        // Wait until our reserved slot
        let wait_time = my_slot.saturating_sub(now);
        if wait_time > 0 {
            tokio::time::sleep(Duration::from_millis(wait_time)).await;
        }
    }

    /// Record successful request - gradually reduce delay
    ///
    /// Uses CAS to prevent race where concurrent record_rate_limit() increases
    /// the delay, but a stale record_success() would overwrite with old reduced value.
    pub fn record_success(&self) {
        loop {
            let current = self.min_delay_ms.load(Ordering::Acquire);
            if current == 0 {
                return;
            }
            // Reduce delay by 25% on success, minimum 0
            let new_delay = (current as f64 * 0.75) as u64;
            match self.min_delay_ms.compare_exchange_weak(
                current,
                new_delay,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(_) => continue, // Value changed, retry with fresh value
            }
        }
    }

    /// Handle rate limit (429) response
    ///
    /// If `retry_after` header is provided, use it. Otherwise, apply exponential backoff.
    pub fn record_rate_limit(&self, retry_after_secs: Option<u64>) {
        self.rate_limit_hits.fetch_add(1, Ordering::Relaxed);

        let new_delay = if let Some(secs) = retry_after_secs {
            // Use Retry-After header value
            (secs * 1000).min(self.max_delay_ms)
        } else {
            // Exponential backoff
            let current = self.min_delay_ms.load(Ordering::Acquire).max(100);
            ((current as f64 * self.backoff_multiplier) as u64).min(self.max_delay_ms)
        };

        self.min_delay_ms.store(new_delay, Ordering::Release);
    }

    /// Get current delay in milliseconds
    pub fn current_delay_ms(&self) -> u64 {
        self.min_delay_ms.load(Ordering::Acquire)
    }

    /// Get rate limit hit count
    pub fn rate_limit_hits(&self) -> u32 {
        self.rate_limit_hits.load(Ordering::Acquire)
    }

    /// Reset rate limiter state
    pub fn reset(&self) {
        self.min_delay_ms.store(0, Ordering::Release);
        self.next_allowed_ms.store(0, Ordering::Release);
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current timestamp in seconds (for circuit breaker)
fn current_timestamp_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Get current timestamp in milliseconds (for rate limiter)
fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::with_params(3, 60);
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn test_circuit_breaker_opens_on_threshold() {
        let cb = CircuitBreaker::with_params(3, 60);

        // Record failures up to threshold
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn test_circuit_breaker_success_resets() {
        let cb = CircuitBreaker::with_params(3, 60);

        cb.record_failure();
        cb.record_failure();
        cb.record_success(); // Should reset
        assert_eq!(cb.state(), CircuitState::Closed);

        // Need 3 more failures to open again
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_stats() {
        let cb = CircuitBreaker::with_params(5, 60);
        cb.record_failure();
        cb.record_failure();

        let stats = cb.stats();
        assert_eq!(stats.failure_count, 2);
        assert_eq!(stats.threshold, 5);
        assert_eq!(stats.total_failures, 2);
        assert_eq!(stats.state, CircuitState::Closed);
    }

    #[test]
    fn test_rate_limiter_starts_with_no_delay() {
        let rl = RateLimiter::new();
        assert_eq!(rl.current_delay_ms(), 0);
    }

    #[test]
    fn test_rate_limiter_backoff() {
        let rl = RateLimiter::new();

        // First rate limit - should set initial delay
        rl.record_rate_limit(None);
        assert!(rl.current_delay_ms() >= 100);

        // Second rate limit - should double
        let first_delay = rl.current_delay_ms();
        rl.record_rate_limit(None);
        assert!(rl.current_delay_ms() > first_delay);
    }

    #[test]
    fn test_rate_limiter_retry_after() {
        let rl = RateLimiter::new();

        // With explicit Retry-After
        rl.record_rate_limit(Some(5));
        assert_eq!(rl.current_delay_ms(), 5000);
    }

    #[test]
    fn test_rate_limiter_success_reduces_delay() {
        let rl = RateLimiter::new();

        rl.record_rate_limit(Some(10)); // 10 second delay
        assert_eq!(rl.current_delay_ms(), 10000);

        rl.record_success();
        assert!(rl.current_delay_ms() < 10000); // Reduced
    }

    #[test]
    fn test_rate_limiter_max_delay() {
        let rl = RateLimiter::new();

        // Even with large Retry-After, should cap at max
        rl.record_rate_limit(Some(60)); // 60 seconds
        assert!(rl.current_delay_ms() <= 30000); // Capped at 30s
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let cb = CircuitBreaker::with_params(2, 60);
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }
}
