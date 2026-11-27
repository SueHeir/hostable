//! Abuse limits: per-phone message rate and per-IP failed joins.
//!
//! Both take the current time as an argument so tests can run them without
//! sleeping.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Messages a second one phone may send.
pub const PHONE_MESSAGES_PER_SECOND: u32 = 60;
/// Wrong codes or tokens one IP may try per [`FAILED_JOIN_WINDOW`].
pub const MAX_FAILED_JOINS: u32 = 10;
pub const FAILED_JOIN_WINDOW: Duration = Duration::from_secs(60);

/// A token bucket: `rate` tokens a second, holding at most `rate`.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    rate: f64,
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(per_second: u32, now: Instant) -> Self {
        Self {
            rate: per_second as f64,
            tokens: per_second as f64,
            last: now,
        }
    }

    /// Whether one more message may go through now.
    pub fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.rate);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Counts failed joins per IP in fixed windows.
#[derive(Debug, Default)]
pub struct JoinGuard {
    failures: HashMap<IpAddr, (Instant, u32)>,
}

impl JoinGuard {
    /// Whether this IP may try to join now.
    pub fn may_try(&mut self, ip: IpAddr, now: Instant) -> bool {
        match self.failures.get(&ip) {
            Some((start, _)) if now.saturating_duration_since(*start) >= FAILED_JOIN_WINDOW => {
                self.failures.remove(&ip);
                true
            }
            Some((_, n)) => *n < MAX_FAILED_JOINS,
            None => true,
        }
    }

    pub fn failed(&mut self, ip: IpAddr, now: Instant) {
        let entry = self.failures.entry(ip).or_insert((now, 0));
        if now.saturating_duration_since(entry.0) >= FAILED_JOIN_WINDOW {
            *entry = (now, 0);
        }
        entry.1 += 1;
    }

    /// Forget old windows so the map can't grow without bound.
    pub fn sweep(&mut self, now: Instant) {
        self.failures
            .retain(|_, (start, _)| now.saturating_duration_since(*start) < FAILED_JOIN_WINDOW);
    }

    pub fn tracked(&self) -> usize {
        self.failures.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixty_a_second_then_refills() {
        let t0 = Instant::now();
        let mut rl = RateLimiter::new(PHONE_MESSAGES_PER_SECOND, t0);
        let allowed = (0..100).filter(|_| rl.allow(t0)).count();
        assert_eq!(allowed, 60);
        assert!(!rl.allow(t0 + Duration::from_millis(10)));
        // A sixtieth of a second buys one more message.
        assert!(rl.allow(t0 + Duration::from_millis(30)));
        // A full second refills the bucket, but never above 60.
        let t1 = t0 + Duration::from_secs(5);
        assert_eq!((0..100).filter(|_| rl.allow(t1)).count(), 60);
    }

    #[test]
    fn steady_traffic_under_the_limit_is_never_dropped() {
        let t0 = Instant::now();
        let mut rl = RateLimiter::new(60, t0);
        for i in 0..600u64 {
            assert!(rl.allow(t0 + Duration::from_millis(i * 20)), "message {i}");
        }
    }

    #[test]
    fn ten_wrong_codes_lock_an_ip_out_for_a_minute() {
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let other: IpAddr = "203.0.113.8".parse().unwrap();
        let t0 = Instant::now();
        let mut guard = JoinGuard::default();
        for _ in 0..MAX_FAILED_JOINS {
            assert!(guard.may_try(ip, t0));
            guard.failed(ip, t0);
        }
        assert!(!guard.may_try(ip, t0 + Duration::from_secs(30)));
        assert!(guard.may_try(other, t0));
        assert!(guard.may_try(ip, t0 + FAILED_JOIN_WINDOW));
        guard.failed(other, t0);
        guard.sweep(t0 + FAILED_JOIN_WINDOW);
        assert_eq!(guard.tracked(), 0);
    }
}
