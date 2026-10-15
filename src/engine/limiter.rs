//! Global bandwidth limiter shared by every connection.
//!
//! A token bucket that may go into debt: a reader takes the bytes it just received and, if the
//! bucket is negative, sleeps until the debt is repaid. Sleeping readers stop pulling from their
//! sockets, so TCP flow control slows the servers down. Fair enough across connections and free
//! when no limit is set.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::time::Instant;

pub struct RateLimiter {
    /// Bytes per second, 0 = unlimited.
    rate: AtomicU64,
    bucket: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    refreshed: Instant,
}

impl RateLimiter {
    pub fn new(rate: Option<u64>) -> Self {
        Self {
            rate: AtomicU64::new(rate.unwrap_or(0)),
            bucket: Mutex::new(Bucket { tokens: 0.0, refreshed: Instant::now() }),
        }
    }

    pub fn set_rate(&self, rate: Option<u64>) {
        self.rate.store(rate.unwrap_or(0), Ordering::Relaxed);
        let mut bucket = self.bucket.lock().expect("limiter poisoned");
        bucket.tokens = 0.0;
        bucket.refreshed = Instant::now();
    }

    pub fn rate(&self) -> Option<u64> {
        Some(self.rate.load(Ordering::Relaxed)).filter(|r| *r > 0)
    }

    /// Accounts for `bytes` just received, sleeping if the limit is exceeded.
    pub async fn consume(&self, bytes: usize) {
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return;
        }
        let wait = {
            let mut bucket = self.bucket.lock().expect("limiter poisoned");
            let now = Instant::now();
            // Allow a burst of a quarter second, so the limit is smooth but not jerky.
            let burst = (rate as f64 / 4.0).max(64.0 * 1024.0);
            let elapsed = now.duration_since(bucket.refreshed).as_secs_f64();
            bucket.tokens = (bucket.tokens + elapsed * rate as f64).min(burst) - bytes as f64;
            bucket.refreshed = now;
            if bucket.tokens >= 0.0 { None } else { Some(Duration::from_secs_f64(-bucket.tokens / rate as f64)) }
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn enforces_rate() {
        let limiter = RateLimiter::new(Some(1024 * 1024));
        let start = tokio::time::Instant::now();
        for _ in 0..8 {
            limiter.consume(512 * 1024).await;
        }
        // 4 MiB at 1 MiB/s, starting from an empty bucket.
        let elapsed = start.elapsed().as_secs_f64();
        assert!((3.9..4.1).contains(&elapsed), "elapsed {elapsed}");
    }

    #[tokio::test]
    async fn unlimited_never_waits() {
        let limiter = RateLimiter::new(None);
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            limiter.consume(1 << 20).await;
        }
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
