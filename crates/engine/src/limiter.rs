//! Global token bucket shared by every worker.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) struct Limiter(Mutex<Bucket>);

struct Bucket {
    /// Bytes per second; `None` is unlimited.
    rate: Option<u64>,
    /// Goes negative when a chunk is bigger than what is saved up; callers sleep off the debt.
    tokens: f64,
    last: Instant,
}

impl Bucket {
    /// Adds tokens for the elapsed time, at most a quarter second worth of burst.
    fn refill(&mut self) -> Option<f64> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        let rate = self.rate? as f64;
        self.tokens = (self.tokens + rate * elapsed).min(rate / 4.0);
        Some(rate)
    }

    fn debt(&mut self) -> Option<Duration> {
        match self.refill() {
            Some(rate) if self.tokens < 0.0 => Some(Duration::from_secs_f64(-self.tokens / rate)),
            _ => None,
        }
    }
}

impl Limiter {
    pub(crate) fn new(rate: Option<u64>) -> Self {
        Self(Mutex::new(Bucket {
            rate,
            tokens: 0.0,
            last: Instant::now(),
        }))
    }

    pub(crate) fn rate(&self) -> Option<u64> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).rate
    }

    pub(crate) fn set_rate(&self, rate: Option<u64>) {
        let mut b = self.0.lock().unwrap_or_else(|e| e.into_inner());
        b.refill();
        b.rate = rate;
        b.tokens = b.tokens.min(0.0);
    }

    /// Waits until `n` bytes fit under the current rate. Re-checks every 250 ms so rate changes apply live.
    pub(crate) async fn take(&self, n: usize) {
        let mut wait = {
            let mut b = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if b.refill().is_none() {
                return;
            }
            b.tokens -= n as f64;
            b.debt()
        };
        while let Some(d) = wait {
            tokio::time::sleep(d.min(Duration::from_millis(250))).await;
            let mut b = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if b.rate.is_none() {
                b.tokens = 0.0;
            }
            wait = b.debt();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn caps_rate() {
        let limiter = Limiter::new(Some(1_000_000));
        let started = Instant::now();
        for _ in 0..6 {
            limiter.take(100_000).await;
        }
        let secs = started.elapsed().as_secs_f64();
        assert!((0.5..1.0).contains(&secs), "600 kB at 1 MB/s took {secs}s");

        limiter.set_rate(None);
        let started = Instant::now();
        limiter.take(10_000_000).await;
        assert!(started.elapsed() < Duration::from_millis(50));
    }
}
