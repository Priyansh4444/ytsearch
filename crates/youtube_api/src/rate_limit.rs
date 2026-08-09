use crate::ApiError;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

const MAX_RATE_LIMIT_BUCKETS: usize = 50_000;

pub(crate) struct RateLimiter {
    buckets: HashMap<String, VecDeque<Instant>>,
    last_prune: Instant,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self {
            buckets: HashMap::new(),
            last_prune: Instant::now(),
        }
    }
}

impl RateLimiter {
    pub(crate) fn allow(
        &mut self,
        key: String,
        limit: usize,
        window: Duration,
    ) -> Result<(), ApiError> {
        self.allow_at(key, limit, window, Instant::now())
    }

    fn allow_at(
        &mut self,
        key: String,
        limit: usize,
        window: Duration,
        now: Instant,
    ) -> Result<(), ApiError> {
        if limit == 0 {
            return Ok(());
        }

        if now.duration_since(self.last_prune) >= window
            || (!self.buckets.contains_key(&key) && self.buckets.len() >= MAX_RATE_LIMIT_BUCKETS)
        {
            self.prune(now, window);
        }
        if !self.buckets.contains_key(&key) && self.buckets.len() >= MAX_RATE_LIMIT_BUCKETS {
            return Err(ApiError::RateLimited(window.as_secs().max(1)));
        }

        let bucket = self.buckets.entry(key).or_default();
        while bucket
            .front()
            .is_some_and(|instant| now.duration_since(*instant) >= window)
        {
            bucket.pop_front();
        }
        if bucket.len() >= limit {
            let oldest = bucket.front().copied().unwrap_or(now);
            let retry_after = window
                .saturating_sub(now.duration_since(oldest))
                .as_secs()
                .max(1);
            return Err(ApiError::RateLimited(retry_after));
        }
        bucket.push_back(now);
        Ok(())
    }

    fn prune(&mut self, now: Instant, window: Duration) {
        self.buckets.retain(|_, bucket| {
            while bucket
                .front()
                .is_some_and(|instant| now.duration_since(*instant) >= window)
            {
                bucket.pop_front();
            }
            !bucket.is_empty()
        });
        self.last_prune = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_requests_and_prunes_inactive_clients() {
        let start = Instant::now();
        let window = Duration::from_secs(60);
        let mut limiter = RateLimiter {
            buckets: HashMap::new(),
            last_prune: start,
        };

        limiter
            .allow_at("first".to_string(), 1, window, start)
            .expect("first request");
        assert!(
            limiter
                .allow_at("first".to_string(), 1, window, start)
                .is_err()
        );

        let after_window = start + window;
        limiter
            .allow_at("second".to_string(), 1, window, after_window)
            .expect("second client request");
        assert!(!limiter.buckets.contains_key("first"));
        assert!(limiter.buckets.contains_key("second"));
    }
}
