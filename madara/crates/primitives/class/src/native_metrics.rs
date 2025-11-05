/// Metrics for Cairo Native compilation and caching
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct CompilationTimeStats {
    pub average_ms: u64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub total_count: u64,
}

#[derive(Debug)]
pub struct NativeMetrics {
    // Cache metrics
    pub cache_hits_memory: AtomicU64,
    pub cache_hits_disk: AtomicU64,
    pub cache_misses: AtomicU64,

    // Compilation metrics
    pub compilations_started: AtomicU64,
    pub compilations_succeeded: AtomicU64,
    pub compilations_failed: AtomicU64,
    pub compilations_timeout: AtomicU64,

    // Performance metrics
    pub total_compilation_time_ms: AtomicU64,
    pub min_compilation_time_ms: AtomicU64,
    pub max_compilation_time_ms: AtomicU64,

    // Runtime metrics
    pub current_cache_size: AtomicUsize,
    pub current_compilations: AtomicUsize,
    pub vm_fallbacks: AtomicU64,
    pub cache_evictions: AtomicU64,
}

impl NativeMetrics {
    pub const fn new() -> Self {
        Self {
            cache_hits_memory: AtomicU64::new(0),
            cache_hits_disk: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            compilations_started: AtomicU64::new(0),
            compilations_succeeded: AtomicU64::new(0),
            compilations_failed: AtomicU64::new(0),
            compilations_timeout: AtomicU64::new(0),
            total_compilation_time_ms: AtomicU64::new(0),
            min_compilation_time_ms: AtomicU64::new(u64::MAX),
            max_compilation_time_ms: AtomicU64::new(0),
            current_cache_size: AtomicUsize::new(0),
            current_compilations: AtomicUsize::new(0),
            vm_fallbacks: AtomicU64::new(0),
            cache_evictions: AtomicU64::new(0),
        }
    }

    // Cache operations
    pub fn record_cache_hit_memory(&self) {
        self.cache_hits_memory.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_hit_disk(&self) {
        self.cache_hits_disk.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_eviction(&self) {
        self.cache_evictions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_cache_size(&self, size: usize) {
        self.current_cache_size.store(size, Ordering::Relaxed);
    }

    // Compilation operations
    pub fn record_compilation_start(&self) {
        self.compilations_started.fetch_add(1, Ordering::Relaxed);
        self.current_compilations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_compilation_end(&self, duration_ms: u64, success: bool, timeout: bool) {
        self.current_compilations.fetch_sub(1, Ordering::Relaxed);

        if timeout {
            self.compilations_timeout.fetch_add(1, Ordering::Relaxed);
            return;
        }

        if success {
            self.compilations_succeeded.fetch_add(1, Ordering::Relaxed);
            self.total_compilation_time_ms.fetch_add(duration_ms, Ordering::Relaxed);

            // Update min
            let mut current_min = self.min_compilation_time_ms.load(Ordering::Relaxed);
            while duration_ms < current_min {
                match self.min_compilation_time_ms.compare_exchange_weak(
                    current_min,
                    duration_ms,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(x) => current_min = x,
                }
            }

            // Update max
            let mut current_max = self.max_compilation_time_ms.load(Ordering::Relaxed);
            while duration_ms > current_max {
                match self.max_compilation_time_ms.compare_exchange_weak(
                    current_max,
                    duration_ms,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(x) => current_max = x,
                }
            }
        } else {
            self.compilations_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_vm_fallback(&self) {
        self.vm_fallbacks.fetch_add(1, Ordering::Relaxed);
    }

    // Getters
    pub fn get_cache_hits_memory(&self) -> u64 {
        self.cache_hits_memory.load(Ordering::Relaxed)
    }

    pub fn get_cache_hits_disk(&self) -> u64 {
        self.cache_hits_disk.load(Ordering::Relaxed)
    }

    pub fn get_cache_misses(&self) -> u64 {
        self.cache_misses.load(Ordering::Relaxed)
    }

    pub fn get_total_requests(&self) -> u64 {
        self.get_cache_hits_memory() + self.get_cache_hits_disk() + self.get_cache_misses()
    }

    pub fn get_cache_hit_rate(&self) -> f64 {
        let total = self.get_total_requests();
        if total == 0 {
            return 0.0;
        }
        let hits = self.get_cache_hits_memory() + self.get_cache_hits_disk();
        (hits as f64 / total as f64) * 100.0
    }

    pub fn get_compilations_started(&self) -> u64 {
        self.compilations_started.load(Ordering::Relaxed)
    }

    pub fn get_compilations_succeeded(&self) -> u64 {
        self.compilations_succeeded.load(Ordering::Relaxed)
    }

    pub fn get_compilations_failed(&self) -> u64 {
        self.compilations_failed.load(Ordering::Relaxed)
    }

    pub fn get_compilations_timeout(&self) -> u64 {
        self.compilations_timeout.load(Ordering::Relaxed)
    }

    pub fn get_compilation_success_rate(&self) -> f64 {
        let total = self.get_compilations_started();
        if total == 0 {
            return 0.0;
        }
        let succeeded = self.get_compilations_succeeded();
        (succeeded as f64 / total as f64) * 100.0
    }

    pub fn get_average_compilation_time_ms(&self) -> u64 {
        let succeeded = self.get_compilations_succeeded();
        if succeeded == 0 {
            return 0;
        }
        let total_time = self.total_compilation_time_ms.load(Ordering::Relaxed);
        total_time / succeeded
    }

    pub fn get_min_compilation_time_ms(&self) -> u64 {
        let min = self.min_compilation_time_ms.load(Ordering::Relaxed);
        if min == u64::MAX {
            0
        } else {
            min
        }
    }

    pub fn get_max_compilation_time_ms(&self) -> u64 {
        self.max_compilation_time_ms.load(Ordering::Relaxed)
    }

    pub fn get_current_cache_size(&self) -> usize {
        self.current_cache_size.load(Ordering::Relaxed)
    }

    pub fn get_current_compilations(&self) -> usize {
        self.current_compilations.load(Ordering::Relaxed)
    }

    pub fn get_vm_fallbacks(&self) -> u64 {
        self.vm_fallbacks.load(Ordering::Relaxed)
    }

    pub fn get_cache_evictions(&self) -> u64 {
        self.cache_evictions.load(Ordering::Relaxed)
    }

    /// Get a summary of all metrics as a formatted string
    pub fn summary(&self) -> String {
        format!(
            "Cairo Native Metrics:\n\
             Cache:\n\
             - Memory hits: {}\n\
             - Disk hits: {}\n\
             - Misses: {}\n\
             - Hit rate: {:.2}%\n\
             - Current size: {}\n\
             - Evictions: {}\n\
             Compilation:\n\
             - Started: {}\n\
             - Succeeded: {}\n\
             - Failed: {}\n\
             - Timeout: {}\n\
             - Success rate: {:.2}%\n\
             - Avg time: {}ms\n\
             - Min time: {}ms\n\
             - Max time: {}ms\n\
             - Currently compiling: {}\n\
             Runtime:\n\
             - VM fallbacks: {}",
            self.get_cache_hits_memory(),
            self.get_cache_hits_disk(),
            self.get_cache_misses(),
            self.get_cache_hit_rate(),
            self.get_current_cache_size(),
            self.get_cache_evictions(),
            self.get_compilations_started(),
            self.get_compilations_succeeded(),
            self.get_compilations_failed(),
            self.get_compilations_timeout(),
            self.get_compilation_success_rate(),
            self.get_average_compilation_time_ms(),
            self.get_min_compilation_time_ms(),
            self.get_max_compilation_time_ms(),
            self.get_current_compilations(),
            self.get_vm_fallbacks(),
        )
    }

    /// Log metrics summary periodically
    pub fn log_summary(&self) {
        let total_compilations = self.get_compilations_started();
        let avg_time = self.get_average_compilation_time_ms();
        let min_time = self.get_min_compilation_time_ms();
        let max_time = self.get_max_compilation_time_ms();

        if total_compilations > 0 {
            tracing::info!(
                "📊 [Cairo Native Metrics] Cache: {:.1}% hit rate ({} classes) | Compilations: {}/{} succeeded ({:.1}%) | Time: avg={}ms min={}ms max={}ms | VM fallbacks: {}",
                self.get_cache_hit_rate(),
                self.get_current_cache_size(),
                self.get_compilations_succeeded(),
                total_compilations,
                self.get_compilation_success_rate(),
                avg_time,
                min_time,
                max_time,
                self.get_vm_fallbacks(),
            );
        } else {
            tracing::info!(
                "📊 [Cairo Native Metrics] Cache: {:.1}% hit rate ({} classes) | No compilations yet | VM fallbacks: {}",
                self.get_cache_hit_rate(),
                self.get_current_cache_size(),
                self.get_vm_fallbacks(),
            );
        }
    }

    /// Get compilation time statistics
    pub fn get_compilation_time_stats(&self) -> CompilationTimeStats {
        CompilationTimeStats {
            average_ms: self.get_average_compilation_time_ms(),
            min_ms: self.get_min_compilation_time_ms(),
            max_ms: self.get_max_compilation_time_ms(),
            total_count: self.get_compilations_succeeded(),
        }
    }
}

impl Default for NativeMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Global metrics instance
static METRICS: std::sync::LazyLock<NativeMetrics> = std::sync::LazyLock::new(NativeMetrics::new);

/// Get the global metrics instance
pub fn metrics() -> &'static NativeMetrics {
    &METRICS
}

/// Helper to time compilation
pub struct CompilationTimer {
    start: Instant,
}

impl CompilationTimer {
    pub fn new() -> Self {
        metrics().record_compilation_start();
        Self { start: Instant::now() }
    }

    pub fn finish(self, success: bool, timeout: bool) {
        let duration_ms = self.start.elapsed().as_millis() as u64;
        metrics().record_compilation_end(duration_ms, success, timeout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_initialization() {
        let metrics = NativeMetrics::new();
        assert_eq!(metrics.get_cache_hits_memory(), 0);
        assert_eq!(metrics.get_cache_hits_disk(), 0);
        assert_eq!(metrics.get_cache_misses(), 0);
        assert_eq!(metrics.get_compilations_started(), 0);
    }

    #[test]
    fn test_cache_operations() {
        let metrics = NativeMetrics::new();

        metrics.record_cache_hit_memory();
        metrics.record_cache_hit_memory();
        metrics.record_cache_hit_disk();
        metrics.record_cache_miss();

        assert_eq!(metrics.get_cache_hits_memory(), 2);
        assert_eq!(metrics.get_cache_hits_disk(), 1);
        assert_eq!(metrics.get_cache_misses(), 1);
        assert_eq!(metrics.get_total_requests(), 4);
        assert_eq!(metrics.get_cache_hit_rate(), 75.0);
    }

    #[test]
    fn test_compilation_metrics() {
        let metrics = NativeMetrics::new();

        metrics.record_compilation_start();
        metrics.record_compilation_end(100, true, false);

        metrics.record_compilation_start();
        metrics.record_compilation_end(200, true, false);

        metrics.record_compilation_start();
        metrics.record_compilation_end(0, false, false);

        assert_eq!(metrics.get_compilations_succeeded(), 2);
        assert_eq!(metrics.get_compilations_failed(), 1);
        assert_eq!(metrics.get_average_compilation_time_ms(), 150);
        assert_eq!(metrics.get_min_compilation_time_ms(), 100);
        assert_eq!(metrics.get_max_compilation_time_ms(), 200);
    }

    #[test]
    fn test_cache_size_tracking() {
        let metrics = NativeMetrics::new();

        metrics.set_cache_size(42);
        assert_eq!(metrics.get_current_cache_size(), 42);

        metrics.set_cache_size(100);
        assert_eq!(metrics.get_current_cache_size(), 100);
    }

    #[test]
    fn test_vm_fallbacks() {
        let metrics = NativeMetrics::new();

        metrics.record_vm_fallback();
        metrics.record_vm_fallback();
        metrics.record_vm_fallback();

        assert_eq!(metrics.get_vm_fallbacks(), 3);
    }

    #[test]
    fn test_summary_format() {
        let metrics = NativeMetrics::new();
        metrics.record_cache_hit_memory();
        metrics.record_cache_miss();

        let summary = metrics.summary();
        assert!(summary.contains("Memory hits: 1"));
        assert!(summary.contains("Misses: 1"));
    }
}
