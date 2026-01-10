//! Resource monitoring module for tracking orchestrator memory and CPU usage.
//!
//! This module provides a non-blocking background thread that monitors the current
//! process's resource usage and exports metrics every second.

use opentelemetry::KeyValue;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use sysinfo::{Pid, ProcessesToUpdate, System};
use tracing::{debug, error, info, warn};

use crate::utils::metrics::ORCHESTRATOR_METRICS;

/// Interval for resource monitoring in seconds
const MONITOR_INTERVAL_SECS: u64 = 1;

/// Bytes to gigabytes conversion factor
const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Handle for the resource monitor thread, allowing graceful shutdown.
pub struct ResourceMonitorHandle {
    shutdown_flag: Arc<AtomicBool>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl ResourceMonitorHandle {
    /// Signals the monitor thread to stop and waits for it to complete.
    pub fn shutdown(mut self) {
        info!("Shutting down resource monitor");
        self.shutdown_flag.store(true, Ordering::SeqCst);

        if let Some(handle) = self.thread_handle.take() {
            if let Err(e) = handle.join() {
                error!("Failed to join resource monitor thread: {:?}", e);
            }
        }

        info!("Resource monitor shutdown complete");
    }
}

/// Starts a non-blocking background thread that monitors memory and CPU usage.
///
/// The monitor tracks:
/// - Memory usage in GB
/// - CPU usage as a percentage
///
/// Metrics are logged and exported via OpenTelemetry every second.
///
/// # Returns
/// A `ResourceMonitorHandle` that can be used to gracefully shutdown the monitor.
pub fn start_resource_monitor() -> ResourceMonitorHandle {
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();

    let thread_handle = thread::spawn(move || {
        let pid = Pid::from_u32(std::process::id());
        let mut sys = System::new();

        info!("Resource monitor started for process {}", pid);

        // Initial refresh to get baseline
        sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

        while !shutdown_flag_clone.load(Ordering::SeqCst) {
            // Refresh process information for our specific process
            sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

            if let Some(process) = sys.process(pid) {
                // Get memory usage in GB
                let memory_bytes = process.memory();
                let memory_gb = memory_bytes as f64 / BYTES_TO_GB;

                // Get CPU usage as percentage
                let cpu_usage = process.cpu_usage() as f64;

                // Log the metrics
                debug!(
                    memory_gb = format!("{:.3}", memory_gb),
                    cpu_percent = format!("{:.2}", cpu_usage),
                    "Resource usage"
                );

                // Export metrics via OpenTelemetry
                let attributes = vec![KeyValue::new("process", "orchestrator")];

                ORCHESTRATOR_METRICS.process_memory_gb.record(memory_gb, &attributes);
                ORCHESTRATOR_METRICS.process_cpu_percent.record(cpu_usage, &attributes);
            } else {
                warn!("Could not find process information for PID {}", pid);
            }

            // Sleep for the monitoring interval
            thread::sleep(Duration::from_secs(MONITOR_INTERVAL_SECS));
        }

        info!("Resource monitor thread exiting");
    });

    ResourceMonitorHandle { shutdown_flag, thread_handle: Some(thread_handle) }
}
