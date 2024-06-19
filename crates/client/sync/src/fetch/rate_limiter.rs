use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};

#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<RateLimiterState>>,
    notify: Arc<Notify>,
}

#[derive(Debug)]
struct RateLimiterState {
    rate_limited: bool,
    until: Option<Instant>,
}

impl RateLimiter {
    const MAX_WAIT_DURATION: Duration = Duration::from_secs(60);

    pub fn new() -> Self {
        RateLimiter {
            state: Arc::new(Mutex::new(RateLimiterState { rate_limited: false, until: None })),
            notify: Arc::new(Notify::new()),
        }
    }

    async fn wait_if_rate_limited(&self) {
        let mut state = self.state.lock().await;

        while state.rate_limited {
            let wait_duration = state.until.unwrap().saturating_duration_since(Instant::now());
            if wait_duration.is_zero() {
                state.rate_limited = false;
                state.until = None;
                break;
            }
            drop(state); // Release the lock before awaiting
                         // Wait to be notified or until the duration is over (50x the wait duration)
            tokio::time::timeout(Self::MAX_WAIT_DURATION, self.notify.notified()).await.ok();
            state = self.state.lock().await;
        }
    }

    async fn set_rate_limit(&self, duration: Duration) {
        let mut state = self.state.lock().await;
        state.rate_limited = true;
        state.until = Some(Instant::now() + duration);
        self.notify.notify_one();
    }

    async fn clear_rate_limit(&self) {
        let mut state = self.state.lock().await;
        state.rate_limited = false;
        state.until = None;
        self.notify.notify_waiters();
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct RateLimitGuard {
    rate_limiter: RateLimiter,
    active: bool,
}

impl RateLimitGuard {
    pub fn new(rate_limiter: RateLimiter) -> Self {
        RateLimitGuard { rate_limiter, active: false }
    }

    pub async fn wait_if_rate_limited(&self) {
        self.rate_limiter.wait_if_rate_limited().await;
    }

    pub async fn activate_rate_limit(&mut self, duration: Duration) {
        self.active = true;
        self.rate_limiter.set_rate_limit(duration).await;
    }
}

impl Drop for RateLimitGuard {
    fn drop(&mut self) {
        if self.active {
            let rate_limiter = self.rate_limiter.clone();
            tokio::spawn(async move {
                rate_limiter.clear_rate_limit().await;
            });
        }
    }
}
