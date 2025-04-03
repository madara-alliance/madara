use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Rolling average implementation.
pub struct ThroughputCounter {
    buckets: VecDeque<(Instant, u64)>,
    bucket_size: Duration,
    window_size: Duration,
    current_count: u64,
    current_bucket_start: Instant,
}
impl ThroughputCounter {
    pub fn new(window_size: Duration) -> Self {
        Self::new_at(window_size, Instant::now())
    }

    fn new_at(window_size: Duration, now: Instant) -> Self {
        Self {
            buckets: VecDeque::new(),
            bucket_size: window_size / 60,
            window_size,
            current_count: 0,
            current_bucket_start: now,
        }
    }

    pub fn increment(&mut self) {
        self.increment_at(Instant::now())
    }

    fn increment_at(&mut self, now: Instant) {
        if now.duration_since(self.current_bucket_start) >= self.bucket_size {
            // Clean-up expired buckets.
            while let Some((time, _)) = self.buckets.front() {
                if now.duration_since(*time) < self.window_size {
                    break;
                }
                self.buckets.pop_front();
            }

            // Make a new bucket.
            if self.current_count > 0 {
                self.buckets.push_back((self.current_bucket_start, self.current_count));
            }
            self.current_count = 0;
            self.current_bucket_start = now;
        }
        self.current_count += 1;
    }

    /// Returns ops/s
    pub fn get_throughput(&self) -> f64 {
        self.get_throughput_at(Instant::now())
    }

    pub fn get_throughput_at(&self, now: Instant) -> f64 {
        let total_ops = self
            .buckets
            .iter()
            .skip_while(|(time, _)| now.duration_since(*time) >= self.window_size)
            .map(|(_, count)| count)
            .sum::<u64>()
            + self.current_count;

        let window_duration =
            if self.buckets.front().is_some_and(|(time, _)| now.duration_since(*time) >= self.window_size) {
                self.window_size.as_secs_f64()
            } else if let Some((oldest_time, _)) = self.buckets.front() {
                now.duration_since(*oldest_time).as_secs_f64()
            } else {
                now.duration_since(self.current_bucket_start).as_secs_f64()
            };
        if window_duration > 0.0 {
            total_ops as f64 / window_duration
        } else {
            0.0
        }
    }
}
