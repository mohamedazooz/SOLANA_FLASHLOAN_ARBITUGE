//! Fixed-window per-client rate limiting.
//!
//! A per-key counter over a fixed window (60 s by default). Keys are client
//! IPs (from `ConnectInfo`, `X-Forwarded-For`, or a shared global bucket when
//! neither is available, e.g. unit tests). The bucket map is opportunistically
//! pruned once it exceeds [`MAX_TRACKED_KEYS`] so adversarial source-IP
//! rotation cannot grow memory without bound.

use dashmap::DashMap;
use std::time::{Duration, Instant};

/// Upper bound on concurrently tracked keys before expired windows are pruned.
const MAX_TRACKED_KEYS: usize = 10_000;

struct WindowCount {
    window_start: Instant,
    count: u32,
}

/// Fixed-window rate limiter. `max_per_window == 0` disables enforcement.
pub struct RateLimiter {
    max_per_window: u32,
    window: Duration,
    buckets: DashMap<String, WindowCount>,
}

impl RateLimiter {
    /// Per-minute limiter (the standard operational unit).
    pub fn new(max_per_window: u32) -> Self {
        Self::with_window(max_per_window, Duration::from_secs(60))
    }

    /// Explicit-window constructor (exposed for deterministic tests).
    pub fn with_window(max_per_window: u32, window: Duration) -> Self {
        Self {
            max_per_window,
            window,
            buckets: DashMap::new(),
        }
    }

    pub fn disabled() -> Self {
        Self::new(0)
    }

    pub fn is_enabled(&self) -> bool {
        self.max_per_window > 0
    }

    /// Returns `true` when the request is allowed under the configured limit.
    pub fn check(&self, key: &str) -> bool {
        if self.max_per_window == 0 {
            return true;
        }
        if self.buckets.len() > MAX_TRACKED_KEYS {
            self.prune();
        }
        let now = Instant::now();
        match self.buckets.entry(key.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(mut occupied) => {
                let wc = occupied.get_mut();
                if now.duration_since(wc.window_start) >= self.window {
                    wc.window_start = now;
                    wc.count = 1;
                    true
                } else if wc.count < self.max_per_window {
                    wc.count += 1;
                    true
                } else {
                    false
                }
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                vacant.insert(WindowCount {
                    window_start: now,
                    count: 1,
                });
                true
            }
        }
    }

    /// Drops buckets whose window has expired.
    pub fn prune(&self) {
        let now = Instant::now();
        self.buckets
            .retain(|_, wc| now.duration_since(wc.window_start) < self.window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_limiter_allows_everything() {
        let limiter = RateLimiter::disabled();
        for _ in 0..1000 {
            assert!(limiter.check("1.2.3.4"));
        }
    }

    #[test]
    fn enforces_limit_within_window() {
        let limiter = RateLimiter::with_window(3, Duration::from_secs(60));
        assert!(limiter.check("10.0.0.1"));
        assert!(limiter.check("10.0.0.1"));
        assert!(limiter.check("10.0.0.1"));
        assert!(!limiter.check("10.0.0.1"));
    }

    #[test]
    fn keys_are_independent() {
        let limiter = RateLimiter::with_window(1, Duration::from_secs(60));
        assert!(limiter.check("10.0.0.1"));
        assert!(!limiter.check("10.0.0.1"));
        assert!(limiter.check("10.0.0.2"));
    }

    #[test]
    fn window_reset_restores_capacity() {
        let limiter = RateLimiter::with_window(1, Duration::from_millis(20));
        assert!(limiter.check("10.0.0.5"));
        assert!(!limiter.check("10.0.0.5"));
        std::thread::sleep(Duration::from_millis(30));
        assert!(limiter.check("10.0.0.5"));
    }

    #[test]
    fn prune_drops_expired_windows_only() {
        let limiter = RateLimiter::with_window(1, Duration::from_millis(20));
        assert!(limiter.check("10.0.0.7"));
        std::thread::sleep(Duration::from_millis(30));
        assert!(limiter.check("10.0.0.8"));
        limiter.prune();
        assert!(!limiter.buckets.contains_key("10.0.0.7"));
        assert!(limiter.buckets.contains_key("10.0.0.8"));
    }
}
