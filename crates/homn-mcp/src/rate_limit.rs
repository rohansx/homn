//! Per-session rate limiting for the MCP server (T079).
//!
//! [`docs/technical/mcp-server.md`](../../../docs/technical/mcp-server.md) and the spec
//! (FR-038 / US5) cap an agent at **100 MCP calls per 60-second window**. An agent that
//! introspects policy is welcome to do so, but it shouldn't be able to hammer the audit
//! DB or the policy engine in a tight loop.
//!
//! The limiter is backed by a [`tokio::sync::Semaphore`]: it starts with `max_per_window`
//! permits, each [`check`](RateLimiter::check) consumes one, and a permit is returned to the
//! pool exactly `window` later. The result is a rolling window — at most `max_per_window`
//! successful calls in any `window`-length span.
//!
//! Over the stdio transport Claude Code spawns one `homn mcp stdio` process per session, so
//! a single limiter on the server *is* per-session. When the HTTP transport lands (T071) the
//! server keys one [`RateLimiter`] per session id.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

/// Default quota: 100 calls per window. Matches `docs/technical/mcp-server.md`.
pub const DEFAULT_MAX_PER_WINDOW: u32 = 100;

/// Default rolling window the quota applies over.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);

/// Returned by [`RateLimiter::check`] when the caller has exhausted its quota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimited {
    /// The rolling window the limit applies over.
    pub window: Duration,
    /// How many calls are allowed per window.
    pub max_per_window: u32,
}

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "rate limit exceeded: at most {} MCP calls per {} s — retry shortly",
            self.max_per_window,
            self.window.as_secs()
        )
    }
}

impl std::error::Error for RateLimited {}

/// A rolling-window rate limiter backed by a [`tokio::sync::Semaphore`].
///
/// Cheap to clone — clones share the same permit pool, so one [`RateLimiter`] guards one
/// logical session no matter how many times the MCP server value is cloned per request.
#[derive(Clone)]
pub struct RateLimiter {
    sem: Arc<Semaphore>,
    window: Duration,
    max_per_window: u32,
}

impl RateLimiter {
    /// Build a limiter allowing `max_per_window` calls per rolling `window`.
    pub fn new(max_per_window: u32, window: Duration) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(max_per_window as usize)),
            window,
            max_per_window,
        }
    }

    /// Build a limiter with the production defaults (100 calls / 60 s).
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_MAX_PER_WINDOW, DEFAULT_WINDOW)
    }

    /// Consume one permit for a single MCP call.
    ///
    /// Returns `Ok(())` if the caller is under quota, or `Err(RateLimited)` if the window is
    /// saturated. On success a Tokio task is spawned that returns the permit to the pool after
    /// `window` — so this **must** be called from within a Tokio runtime.
    pub fn check(&self) -> Result<(), RateLimited> {
        match Arc::clone(&self.sem).try_acquire_owned() {
            Ok(permit) => {
                // Don't hold the permit for the call's duration — hold it for the *window*.
                // `forget` takes it out of the pool; the spawned task adds one back later.
                permit.forget();
                let sem = Arc::clone(&self.sem);
                let window = self.window;
                tokio::spawn(async move {
                    tokio::time::sleep(window).await;
                    sem.add_permits(1);
                });
                Ok(())
            }
            Err(_) => Err(RateLimited {
                window: self.window,
                max_per_window: self.max_per_window,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_up_to_limit_then_rejects() {
        // T072: 101 calls inside the window — the 101st must be rate-limited.
        let limiter = RateLimiter::new(100, Duration::from_secs(60));
        for i in 0..100 {
            assert!(
                limiter.check().is_ok(),
                "call {i} (of the first 100) should pass"
            );
        }
        let over = limiter.check();
        assert!(over.is_err(), "the 101st call must be rate-limited");
        assert_eq!(over.unwrap_err().max_per_window, 100);
    }

    #[tokio::test]
    async fn permits_return_after_the_window() {
        // Once the window elapses, the freed permit lets a fresh call through.
        let limiter = RateLimiter::new(1, Duration::from_millis(50));
        assert!(
            limiter.check().is_ok(),
            "first call consumes the only permit"
        );
        assert!(
            limiter.check().is_err(),
            "second call inside the window is rejected"
        );

        tokio::time::sleep(Duration::from_millis(140)).await;

        assert!(
            limiter.check().is_ok(),
            "after the window the permit is back and a call is allowed"
        );
    }

    #[tokio::test]
    async fn rate_limited_error_renders_a_useful_message() {
        let err = RateLimited {
            window: Duration::from_secs(60),
            max_per_window: 100,
        };
        let msg = err.to_string();
        assert!(msg.contains("100"), "message names the quota: {msg}");
        assert!(msg.contains("60"), "message names the window: {msg}");
    }
}
