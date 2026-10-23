//! GCRA rate limiter (ADR-0018). One timestamp (TAT) per key.

use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::error::ApiError;

#[derive(Debug, Clone, Copy)]
pub struct Rate {
    pub limit: u32,
    pub period: Duration,
}

impl Rate {
    pub const fn new(limit: u32, period: Duration) -> Self {
        Self { limit, period }
    }
    fn emission(&self) -> Duration {
        self.period / self.limit.max(1)
    }
}

/// Bucket presets.
pub mod rates {
    use super::Rate;
    use std::time::Duration;
    pub const AUTH_IP: Rate = Rate::new(10, Duration::from_secs(60));
    pub const REGISTER_IP: Rate = Rate::new(5, Duration::from_secs(3600));
    pub const GLOBAL_USER: Rate = Rate::new(50, Duration::from_secs(1));
    pub const MESSAGE_CREATE: Rate = Rate::new(5, Duration::from_secs(5));
    /// Across all channels.
    pub const MESSAGE_CREATE_ALL: Rate = Rate::new(15, Duration::from_secs(5));
    /// Per account, counted on failed passwords only.
    pub const LOGIN_FAILURES: Rate = Rate::new(20, Duration::from_secs(900));
    pub const INVITE_PREVIEW: Rate = Rate::new(60, Duration::from_secs(60));
    /// Wrong password / 2FA code while already logged in (sensitive actions).
    pub const REAUTH_FAILURES: Rate = Rate::new(5, Duration::from_secs(900));
    pub const TYPING: Rate = Rate::new(10, Duration::from_secs(10));
    pub const INVITE_ACCEPT: Rate = Rate::new(10, Duration::from_secs(600));
    pub const WEBHOOK: Rate = Rate::new(30, Duration::from_secs(60));
    pub const MFA_TICKET: Rate = Rate::new(5, Duration::from_secs(300));
    /// Friend requests sent per user (anti-spam).
    pub const FRIEND_REQUEST: Rate = Rate::new(30, Duration::from_secs(3600));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub limit: u32,
    pub remaining: u32,
    pub reset_after: Duration,
}

#[derive(Default)]
pub struct RateLimiter {
    tat: DashMap<String, Instant>,
}

impl RateLimiter {
    pub fn check_at(&self, key: &str, rate: Rate, now: Instant) -> Result<Decision, Duration> {
        let t = rate.emission();
        let tau = rate.period.saturating_sub(t); // burst tolerance
        let mut entry = self.tat.entry(key.to_string()).or_insert(now);
        let tat = (*entry).max(now);
        let ahead = tat - now;
        if ahead > tau {
            return Err(ahead - tau);
        }
        let new_tat = tat + t;
        *entry = new_tat;
        let used = new_tat - now;
        let remaining = (rate.period.saturating_sub(used).as_nanos() / t.as_nanos().max(1)) as u32;
        Ok(Decision {
            limit: rate.limit,
            remaining,
            reset_after: used,
        })
    }

    /// Would a request be rejected right now? (does not consume)
    pub fn is_limited(&self, key: &str, rate: Rate) -> bool {
        let now = Instant::now();
        let tau = rate.period.saturating_sub(rate.emission());
        self.tat
            .get(key)
            .is_some_and(|tat| *tat > now && *tat - now > tau)
    }

    pub fn check(&self, key: &str, rate: Rate) -> Result<Decision, ApiError> {
        self.check_at(key, rate, Instant::now())
            .map_err(ApiError::RateLimited)
    }

    /// Drop expired keys; call periodically.
    pub fn sweep(&self) {
        let now = Instant::now();
        self.tat.retain(|_, v| *v > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_burst_then_limits() {
        let rl = RateLimiter::default();
        let r = Rate::new(5, Duration::from_secs(5));
        let now = Instant::now();
        for i in 0..5 {
            let d = rl.check_at("k", r, now).expect("within burst");
            assert_eq!(d.remaining, 4 - i);
        }
        let wait = rl.check_at("k", r, now).unwrap_err();
        assert!(wait <= Duration::from_secs(1) && wait > Duration::ZERO);
        // After one emission interval one more request is allowed.
        assert!(rl.check_at("k", r, now + Duration::from_secs(1)).is_ok());
        assert!(rl.check_at("k", r, now + Duration::from_secs(1)).is_err());
    }

    #[test]
    fn keys_are_independent() {
        let rl = RateLimiter::default();
        let r = Rate::new(1, Duration::from_secs(60));
        let now = Instant::now();
        assert!(rl.check_at("a", r, now).is_ok());
        assert!(rl.check_at("a", r, now).is_err());
        assert!(rl.check_at("b", r, now).is_ok());
    }

    #[test]
    fn sweep_removes_stale() {
        let rl = RateLimiter::default();
        let r = Rate::new(1000, Duration::from_millis(1));
        rl.check_at("x", r, Instant::now() - Duration::from_secs(10))
            .unwrap();
        rl.sweep();
        assert!(rl.tat.is_empty());
    }
}
