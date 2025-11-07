//! Metrics for Cairo Native compilation and caching
//!
//! This module provides comprehensive metrics tracking for Cairo Native operations.
//! Metrics are exported via OpenTelemetry (OTEL) for integration with Prometheus/OTLP.
//!
//! # Metrics Categories
//!
//! ## Cache Metrics
//! - Memory cache hits/misses
//! - Disk cache hits
//! - Cache evictions
//! - Current cache size
//! - Cache operation latencies (memory lookup, disk lookup, conversions, evictions)
//! - Cache error counters (timeouts, thread disconnections, file errors)
//!
//! ## Compilation Metrics
//! - Compilations started/succeeded/failed/timeout
//! - Compilation times (via histogram)
//! - Compilation flow metrics (in-progress skips, blocking mode events)
//! - Blocking compilation conversion times
//!
//! ## Runtime Metrics
//! - VM fallbacks (when native compilation isn't available)
//!
//! # Usage
//!
//! Metrics are automatically recorded during compilation and cache operations.
//! Use `metrics()` to get the global metrics instance.
//!
//! ```rust
//! use mc_cairo_native::metrics::metrics;
//!
//! metrics().record_cache_hit_memory();
//! ```
//!
//! # OTEL Integration
//!
//! All metrics are exported via OpenTelemetry and can be scraped by Prometheus
//! or sent to an OTLP endpoint. Metrics are registered during `register()` which
//! should be called during application initialization (typically via `Analytics::setup()`).
//!
//! # Thread Safety
//!
//! OTEL instruments are thread-safe and can be called from any thread concurrently.

use mc_analytics::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
};
use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::time::Instant;

/// Test-only counters for verifying metrics in tests.
/// These are only compiled in test builds and provide readable counters
/// alongside the write-only OTEL metrics.
#[cfg(test)]
pub mod test_counters {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    /// Global mutex to serialize test execution and prevent interference between parallel tests.
    /// Tests should acquire this mutex at the start and reset counters before running.
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    /// Acquire the test mutex and reset all counters.
    /// Returns a guard that should be held for the duration of the test.
    /// When the guard is dropped, the mutex is released.
    ///
    /// # Example
    /// ```rust,no_run
    /// let _guard = test_counters::acquire_and_reset();
    /// // Test code here - metrics are isolated from other parallel tests
    /// ```
    pub fn acquire_and_reset() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        reset_all();
        guard
    }

    // Cache metrics
    pub static CACHE_HITS_MEMORY: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_HITS_DISK: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_MEMORY_MISS: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_DISK_MISS: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_EVICTIONS: AtomicU64 = AtomicU64::new(0);

    // Cache error counters
    pub static CACHE_MEMORY_TIMEOUT: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_MEMORY_THREAD_DISCONNECTED: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_DISK_LOAD_TIMEOUT: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_DISK_LOAD_ERROR: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_DISK_THREAD_DISCONNECTED: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_DISK_ERROR_FALLBACK: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_DISK_FILE_NOT_FOUND: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_DISK_FILE_EMPTY: AtomicU64 = AtomicU64::new(0);
    pub static CACHE_DISK_METADATA_ERROR: AtomicU64 = AtomicU64::new(0);

    // Compilation metrics
    pub static COMPILATIONS_STARTED: AtomicU64 = AtomicU64::new(0);
    pub static COMPILATIONS_SUCCEEDED: AtomicU64 = AtomicU64::new(0);
    pub static COMPILATIONS_FAILED: AtomicU64 = AtomicU64::new(0);
    pub static COMPILATIONS_TIMEOUT: AtomicU64 = AtomicU64::new(0);

    // Compilation flow metrics
    pub static COMPILATION_IN_PROGRESS_SKIP: AtomicU64 = AtomicU64::new(0);
    pub static COMPILATION_BLOCKING_MISS: AtomicU64 = AtomicU64::new(0);
    pub static COMPILATION_BLOCKING_COMPLETE: AtomicU64 = AtomicU64::new(0);

    // Runtime metrics
    pub static VM_FALLBACKS: AtomicU64 = AtomicU64::new(0);

    /// Reset all test counters to zero.
    /// Should be called at the start of each test to ensure clean state.
    pub fn reset_all() {
        CACHE_HITS_MEMORY.store(0, Ordering::Relaxed);
        CACHE_HITS_DISK.store(0, Ordering::Relaxed);
        CACHE_MEMORY_MISS.store(0, Ordering::Relaxed);
        CACHE_DISK_MISS.store(0, Ordering::Relaxed);
        CACHE_EVICTIONS.store(0, Ordering::Relaxed);

        CACHE_MEMORY_TIMEOUT.store(0, Ordering::Relaxed);
        CACHE_MEMORY_THREAD_DISCONNECTED.store(0, Ordering::Relaxed);
        CACHE_DISK_LOAD_TIMEOUT.store(0, Ordering::Relaxed);
        CACHE_DISK_LOAD_ERROR.store(0, Ordering::Relaxed);
        CACHE_DISK_THREAD_DISCONNECTED.store(0, Ordering::Relaxed);
        CACHE_DISK_ERROR_FALLBACK.store(0, Ordering::Relaxed);
        CACHE_DISK_FILE_NOT_FOUND.store(0, Ordering::Relaxed);
        CACHE_DISK_FILE_EMPTY.store(0, Ordering::Relaxed);
        CACHE_DISK_METADATA_ERROR.store(0, Ordering::Relaxed);

        COMPILATIONS_STARTED.store(0, Ordering::Relaxed);
        COMPILATIONS_SUCCEEDED.store(0, Ordering::Relaxed);
        COMPILATIONS_FAILED.store(0, Ordering::Relaxed);
        COMPILATIONS_TIMEOUT.store(0, Ordering::Relaxed);

        COMPILATION_IN_PROGRESS_SKIP.store(0, Ordering::Relaxed);
        COMPILATION_BLOCKING_MISS.store(0, Ordering::Relaxed);
        COMPILATION_BLOCKING_COMPLETE.store(0, Ordering::Relaxed);

        VM_FALLBACKS.store(0, Ordering::Relaxed);
    }
}

/// Metrics for Cairo Native compilation and caching operations.
///
/// Uses OpenTelemetry instruments for recording metrics (exported to Prometheus/OTLP).
#[derive(Debug)]
pub struct NativeMetrics {
    // Cache metrics
    cache_hits_memory_counter: Counter<u64>,
    cache_hits_disk_counter: Counter<u64>,
    cache_memory_miss_counter: Counter<u64>,
    cache_disk_miss_counter: Counter<u64>,
    cache_evictions_counter: Counter<u64>,
    cache_size_gauge: Gauge<u64>,

    // Cache error counters
    cache_memory_timeout_counter: Counter<u64>,
    cache_memory_thread_disconnected_counter: Counter<u64>,
    cache_disk_load_timeout_counter: Counter<u64>,
    cache_disk_load_error_counter: Counter<u64>,
    cache_disk_thread_disconnected_counter: Counter<u64>,
    cache_disk_error_fallback_counter: Counter<u64>,
    cache_disk_file_not_found_counter: Counter<u64>,
    cache_disk_file_empty_counter: Counter<u64>,
    cache_disk_metadata_error_counter: Counter<u64>,

    // Cache latency histograms
    cache_lookup_time_memory_histogram: Histogram<f64>,
    cache_lookup_time_disk_histogram: Histogram<f64>,
    cache_conversion_time_histogram: Histogram<f64>,
    cache_eviction_time_histogram: Histogram<f64>,
    cache_disk_load_time_histogram: Histogram<f64>,
    cache_disk_blockifier_convert_time_histogram: Histogram<f64>,
    cache_disk_runnable_convert_time_histogram: Histogram<f64>,

    // Compilation metrics
    compilations_started_counter: Counter<u64>,
    compilations_succeeded_counter: Counter<u64>,
    compilations_failed_counter: Counter<u64>,
    compilations_timeout_counter: Counter<u64>,
    compilation_time_histogram: Histogram<f64>,
    current_compilations_gauge: Gauge<u64>,

    // Compilation flow metrics
    compilation_in_progress_skip_counter: Counter<u64>,
    compilation_blocking_miss_counter: Counter<u64>,
    compilation_blocking_complete_counter: Counter<u64>,
    compilation_blocking_conversion_time_histogram: Histogram<f64>,

    // Runtime metrics
    vm_fallbacks_counter: Counter<u64>,
}

impl NativeMetrics {
    /// Register and initialize OTEL metrics.
    ///
    /// This should be called during application initialization, typically via `Analytics::setup()`.
    /// If not called, metrics will still work but won't be exported via OTEL.
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.cairo_native.opentelemetry")
                .with_attributes([KeyValue::new("crate", "cairo_native")])
                .build(),
        );

        let cache_hits_memory_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_hits_memory".to_string(),
            "Number of memory cache hits for Cairo Native compiled classes".to_string(),
            "hit".to_string(),
        );
        let cache_hits_disk_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_hits_disk".to_string(),
            "Number of disk cache hits for Cairo Native compiled classes".to_string(),
            "hit".to_string(),
        );
        let cache_memory_miss_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_memory_miss".to_string(),
            "Number of memory cache misses for Cairo Native compiled classes".to_string(),
            "miss".to_string(),
        );
        let cache_disk_miss_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_disk_miss".to_string(),
            "Number of disk cache misses for Cairo Native compiled classes".to_string(),
            "miss".to_string(),
        );
        let cache_evictions_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_evictions".to_string(),
            "Number of cache evictions for Cairo Native compiled classes".to_string(),
            "eviction".to_string(),
        );
        let cache_size_gauge = register_gauge_metric_instrument(
            &meter,
            "cairo_native_cache_size".to_string(),
            "Current number of classes in the Cairo Native memory cache".to_string(),
            "class".to_string(),
        );

        // Cache error counters
        let cache_memory_timeout_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_memory_timeout".to_string(),
            "Number of memory cache lookup timeouts".to_string(),
            "timeout".to_string(),
        );
        let cache_memory_thread_disconnected_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_memory_thread_disconnected".to_string(),
            "Number of memory cache thread disconnections".to_string(),
            "disconnection".to_string(),
        );
        let cache_disk_load_timeout_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_disk_load_timeout".to_string(),
            "Number of disk cache load timeouts".to_string(),
            "timeout".to_string(),
        );
        let cache_disk_load_error_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_disk_load_error".to_string(),
            "Number of disk cache load errors".to_string(),
            "error".to_string(),
        );
        let cache_disk_thread_disconnected_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_disk_thread_disconnected".to_string(),
            "Number of disk cache load thread disconnections".to_string(),
            "disconnection".to_string(),
        );
        let cache_disk_error_fallback_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_disk_error_fallback".to_string(),
            "Number of disk cache errors causing fallback to compilation".to_string(),
            "fallback".to_string(),
        );
        let cache_disk_file_not_found_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_disk_file_not_found".to_string(),
            "Number of disk cache file not found occurrences".to_string(),
            "miss".to_string(),
        );
        let cache_disk_file_empty_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_disk_file_empty".to_string(),
            "Number of empty or corrupted disk cache files encountered".to_string(),
            "error".to_string(),
        );
        let cache_disk_metadata_error_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_cache_disk_metadata_error".to_string(),
            "Number of disk cache file metadata errors".to_string(),
            "error".to_string(),
        );

        // Cache latency histograms
        let cache_lookup_time_memory_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_cache_lookup_time_memory".to_string(),
            "Time taken for memory cache lookups".to_string(),
            "ms".to_string(),
        );
        let cache_lookup_time_disk_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_cache_lookup_time_disk".to_string(),
            "Time taken for disk cache lookups".to_string(),
            "ms".to_string(),
        );
        let cache_conversion_time_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_cache_conversion_time".to_string(),
            "Time taken to convert NativeCompiledClass to RunnableCompiledClass".to_string(),
            "ms".to_string(),
        );
        let cache_eviction_time_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_cache_eviction_time".to_string(),
            "Time taken to evict cache entries".to_string(),
            "ms".to_string(),
        );
        let cache_disk_load_time_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_cache_disk_load_time".to_string(),
            "Time taken to load executor from disk cache".to_string(),
            "ms".to_string(),
        );
        let cache_disk_blockifier_convert_time_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_cache_disk_blockifier_convert_time".to_string(),
            "Time taken for blockifier conversion when loading from disk cache".to_string(),
            "ms".to_string(),
        );
        let cache_disk_runnable_convert_time_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_cache_disk_runnable_convert_time".to_string(),
            "Time taken for RunnableCompiledClass conversion when loading from disk cache".to_string(),
            "ms".to_string(),
        );

        let compilations_started_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_compilations_started".to_string(),
            "Number of Cairo Native compilations started".to_string(),
            "compilation".to_string(),
        );
        let compilations_succeeded_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_compilations_succeeded".to_string(),
            "Number of successful Cairo Native compilations".to_string(),
            "compilation".to_string(),
        );
        let compilations_failed_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_compilations_failed".to_string(),
            "Number of failed Cairo Native compilations".to_string(),
            "compilation".to_string(),
        );
        let compilations_timeout_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_compilations_timeout".to_string(),
            "Number of Cairo Native compilations that timed out".to_string(),
            "compilation".to_string(),
        );
        let compilation_time_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_compilation_time".to_string(),
            "Time taken to compile Cairo Native classes".to_string(),
            "ms".to_string(),
        );
        let current_compilations_gauge = register_gauge_metric_instrument(
            &meter,
            "cairo_native_current_compilations".to_string(),
            "Current number of active Cairo Native compilations".to_string(),
            "compilation".to_string(),
        );

        let vm_fallbacks_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_vm_fallbacks".to_string(),
            "Number of times Cairo VM fallback was used instead of native execution".to_string(),
            "fallback".to_string(),
        );

        // Compilation flow metrics
        let compilation_in_progress_skip_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_compilation_in_progress_skip".to_string(),
            "Number of times disk cache lookup was skipped due to compilation in progress".to_string(),
            "skip".to_string(),
        );
        let compilation_blocking_miss_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_compilation_blocking_miss".to_string(),
            "Number of cache misses in blocking compilation mode".to_string(),
            "miss".to_string(),
        );
        let compilation_blocking_complete_counter = register_counter_metric_instrument(
            &meter,
            "cairo_native_compilation_blocking_complete".to_string(),
            "Number of blocking compilations completed".to_string(),
            "compilation".to_string(),
        );
        let compilation_blocking_conversion_time_histogram = register_histogram_metric_instrument(
            &meter,
            "cairo_native_compilation_blocking_conversion_time".to_string(),
            "Time taken to convert to RunnableCompiledClass after blocking compilation".to_string(),
            "ms".to_string(),
        );

        Self {
            cache_hits_memory_counter,
            cache_hits_disk_counter,
            cache_memory_miss_counter,
            cache_disk_miss_counter,
            cache_evictions_counter,
            cache_size_gauge,
            cache_memory_timeout_counter,
            cache_memory_thread_disconnected_counter,
            cache_disk_load_timeout_counter,
            cache_disk_load_error_counter,
            cache_disk_thread_disconnected_counter,
            cache_disk_error_fallback_counter,
            cache_disk_file_not_found_counter,
            cache_disk_file_empty_counter,
            cache_disk_metadata_error_counter,
            cache_lookup_time_memory_histogram,
            cache_lookup_time_disk_histogram,
            cache_conversion_time_histogram,
            cache_eviction_time_histogram,
            cache_disk_load_time_histogram,
            cache_disk_blockifier_convert_time_histogram,
            cache_disk_runnable_convert_time_histogram,
            compilations_started_counter,
            compilations_succeeded_counter,
            compilations_failed_counter,
            compilations_timeout_counter,
            compilation_time_histogram,
            current_compilations_gauge,
            compilation_in_progress_skip_counter,
            compilation_blocking_miss_counter,
            compilation_blocking_complete_counter,
            compilation_blocking_conversion_time_histogram,
            vm_fallbacks_counter,
        }
    }

    // Cache operations
    pub fn record_cache_hit_memory(&self) {
        self.cache_hits_memory_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_HITS_MEMORY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_hit_disk(&self) {
        self.cache_hits_disk_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_HITS_DISK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_memory_miss(&self) {
        self.cache_memory_miss_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_MEMORY_MISS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_disk_miss(&self) {
        self.cache_disk_miss_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_DISK_MISS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_eviction(&self) {
        self.cache_evictions_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_EVICTIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_cache_size(&self, size: usize) {
        self.cache_size_gauge.record(size as u64, &[]);
    }

    // Cache error recording
    pub fn record_cache_memory_timeout(&self) {
        self.cache_memory_timeout_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_MEMORY_TIMEOUT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_memory_thread_disconnected(&self) {
        self.cache_memory_thread_disconnected_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_MEMORY_THREAD_DISCONNECTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_disk_load_timeout(&self) {
        self.cache_disk_load_timeout_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_DISK_LOAD_TIMEOUT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_disk_load_error(&self) {
        self.cache_disk_load_error_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_DISK_LOAD_ERROR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_disk_thread_disconnected(&self) {
        self.cache_disk_thread_disconnected_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_DISK_THREAD_DISCONNECTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_disk_error_fallback(&self) {
        self.cache_disk_error_fallback_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_DISK_ERROR_FALLBACK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_disk_file_not_found(&self) {
        self.cache_disk_file_not_found_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_DISK_FILE_NOT_FOUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_disk_file_empty(&self) {
        self.cache_disk_file_empty_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_DISK_FILE_EMPTY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_cache_disk_metadata_error(&self) {
        self.cache_disk_metadata_error_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::CACHE_DISK_METADATA_ERROR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    // Cache latency recording
    pub fn record_cache_lookup_time_memory(&self, duration_ms: u64) {
        self.cache_lookup_time_memory_histogram.record(duration_ms as f64, &[]);
    }

    pub fn record_cache_lookup_time_disk(&self, duration_ms: u64) {
        self.cache_lookup_time_disk_histogram.record(duration_ms as f64, &[]);
    }

    pub fn record_cache_conversion_time(&self, duration_ms: u64) {
        self.cache_conversion_time_histogram.record(duration_ms as f64, &[]);
    }

    pub fn record_cache_eviction_time(&self, duration_ms: u64) {
        self.cache_eviction_time_histogram.record(duration_ms as f64, &[]);
    }

    pub fn record_cache_disk_load_time(&self, duration_ms: u64) {
        self.cache_disk_load_time_histogram.record(duration_ms as f64, &[]);
    }

    pub fn record_cache_disk_blockifier_convert_time(&self, duration_ms: u64) {
        self.cache_disk_blockifier_convert_time_histogram.record(duration_ms as f64, &[]);
    }

    pub fn record_cache_disk_runnable_convert_time(&self, duration_ms: u64) {
        self.cache_disk_runnable_convert_time_histogram.record(duration_ms as f64, &[]);
    }

    // Compilation operations
    pub fn record_compilation_start(&self) {
        self.compilations_started_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::COMPILATIONS_STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Update current compilations gauge by querying COMPILATION_IN_PROGRESS directly
        // This avoids maintaining separate atomic state
        self.update_current_compilations_gauge();
    }

    pub fn record_compilation_end(&self, duration_ms: u64, success: bool, timeout: bool) {
        // Update current compilations gauge before decrementing
        self.update_current_compilations_gauge();

        if timeout {
            self.compilations_timeout_counter.add(1, &[]);
            #[cfg(test)]
            test_counters::COMPILATIONS_TIMEOUT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }

        if success {
            self.compilations_succeeded_counter.add(1, &[]);
            #[cfg(test)]
            test_counters::COMPILATIONS_SUCCEEDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Record compilation time in histogram (in milliseconds as f64)
            self.compilation_time_histogram.record(duration_ms as f64, &[]);
        } else {
            self.compilations_failed_counter.add(1, &[]);
            #[cfg(test)]
            test_counters::COMPILATIONS_FAILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn record_vm_fallback(&self) {
        self.vm_fallbacks_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::VM_FALLBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    // Compilation flow recording
    pub fn record_compilation_in_progress_skip(&self) {
        self.compilation_in_progress_skip_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::COMPILATION_IN_PROGRESS_SKIP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_compilation_blocking_miss(&self) {
        self.compilation_blocking_miss_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::COMPILATION_BLOCKING_MISS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_compilation_blocking_complete(&self) {
        self.compilation_blocking_complete_counter.add(1, &[]);
        #[cfg(test)]
        test_counters::COMPILATION_BLOCKING_COMPLETE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_compilation_blocking_conversion_time(&self, duration_ms: u64) {
        self.compilation_blocking_conversion_time_histogram.record(duration_ms as f64, &[]);
    }

    /// Update the current compilations gauge by querying the compilation map.
    fn update_current_compilations_gauge(&self) {
        // Query the actual compilation map to get current count
        // This avoids maintaining separate atomic state
        let current = crate::compilation::get_current_compilations_count();
        self.current_compilations_gauge.record(current as u64, &[]);
    }
}

/// Global metrics instance (initialized lazily on first access).
/// If `register()` hasn't been called, metrics will still work but won't be exported via OTEL.
static METRICS: std::sync::LazyLock<NativeMetrics> = std::sync::LazyLock::new(|| {
    // Try to register OTEL metrics, but if OTEL isn't initialized yet, create a fallback
    // In practice, register() should be called explicitly during Analytics::setup()
    NativeMetrics::register()
});

/// Get the global metrics instance.
///
/// Returns a thread-safe reference to the global metrics singleton.
/// Safe to call from any thread.
///
/// **Note**: For OTEL export to work, `register()` should be called during application
/// initialization (typically via `Analytics::setup()`). If not called, metrics will
/// still work but won't be exported.
pub fn metrics() -> &'static NativeMetrics {
    &METRICS
}

/// Helper to time compilation operations.
///
/// Automatically records compilation start and end times in metrics.
/// Usage:
///
/// ```rust
/// use mc_cairo_native::metrics::CompilationTimer;
///
/// let timer = CompilationTimer::new();
/// // ... compilation happens ...
/// let success = true;
/// let timeout = false;
/// timer.finish(success, timeout);
/// ```
pub struct CompilationTimer {
    start: Instant,
}

impl Default for CompilationTimer {
    fn default() -> Self {
        Self::new()
    }
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
        let metrics = NativeMetrics::register();
        // Just verify it can be created
        let _ = metrics;
    }

    #[test]
    fn test_cache_operations() {
        let metrics = NativeMetrics::register();

        // Test that recording doesn't panic
        metrics.record_cache_hit_memory();
        metrics.record_cache_hit_memory();
        metrics.record_cache_hit_disk();
        metrics.record_cache_eviction();
    }

    #[test]
    fn test_cache_error_metrics() {
        let metrics = NativeMetrics::register();

        // Test cache error counters
        metrics.record_cache_memory_timeout();
        metrics.record_cache_memory_thread_disconnected();
        metrics.record_cache_disk_load_timeout();
        metrics.record_cache_disk_load_error();
        metrics.record_cache_disk_thread_disconnected();
        metrics.record_cache_disk_error_fallback();
        metrics.record_cache_disk_file_not_found();
        metrics.record_cache_disk_file_empty();
        metrics.record_cache_disk_metadata_error();
    }

    #[test]
    fn test_cache_latency_metrics() {
        let metrics = NativeMetrics::register();

        // Test cache latency histograms
        metrics.record_cache_lookup_time_memory(10);
        metrics.record_cache_lookup_time_disk(50);
        metrics.record_cache_conversion_time(5);
        metrics.record_cache_eviction_time(2);
        metrics.record_cache_disk_load_time(30);
        metrics.record_cache_disk_blockifier_convert_time(15);
        metrics.record_cache_disk_runnable_convert_time(8);
    }

    #[test]
    fn test_compilation_flow_metrics() {
        let metrics = NativeMetrics::register();

        // Test compilation flow metrics
        metrics.record_compilation_in_progress_skip();
        metrics.record_compilation_blocking_miss();
        metrics.record_compilation_blocking_complete();
        metrics.record_compilation_blocking_conversion_time(20);
    }

    #[test]
    fn test_compilation_metrics() {
        let metrics = NativeMetrics::register();

        // Test that recording doesn't panic
        metrics.record_compilation_start();
        metrics.record_compilation_end(100, true, false);

        metrics.record_compilation_start();
        metrics.record_compilation_end(200, true, false);

        metrics.record_compilation_start();
        metrics.record_compilation_end(0, false, false);

        metrics.record_compilation_start();
        metrics.record_compilation_end(0, false, true);
    }

    #[test]
    fn test_cache_size_tracking() {
        let metrics = NativeMetrics::register();

        // Test that setting cache size doesn't panic
        metrics.set_cache_size(42);
        metrics.set_cache_size(100);
    }

    #[test]
    fn test_vm_fallbacks() {
        let metrics = NativeMetrics::register();

        // Test that recording doesn't panic
        metrics.record_vm_fallback();
        metrics.record_vm_fallback();
        metrics.record_vm_fallback();
    }

    #[test]
    fn test_global_metrics_singleton() {
        // Test that the global metrics singleton works correctly
        let metrics = metrics();

        // Test that recording doesn't panic
        metrics.record_cache_hit_memory();
        metrics.record_cache_hit_memory();
        metrics.record_cache_hit_disk();
        metrics.record_vm_fallback();
        metrics.record_cache_eviction();

        // Test compilation metrics
        metrics.record_compilation_start();
        metrics.record_compilation_end(100, true, false);
        metrics.record_compilation_start();
        metrics.record_compilation_end(200, true, false);
        metrics.record_compilation_start();
        metrics.record_compilation_end(0, false, false);
        metrics.record_compilation_start();
        metrics.record_compilation_end(0, false, true);

        // Test cache size
        metrics.set_cache_size(42);
    }
}
