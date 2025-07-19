use anyhow::{anyhow, Result};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::Notify;
use tracing::{error, info, warn};

#[cfg(unix)]
use signal::unix::{signal, SignalKind};

/// Signal types that can trigger shutdown
#[derive(Debug, Clone, Copy)]
pub enum ShutdownSignal {
    /// SIGTERM - Docker/Kubernetes graceful shutdown
    Terminate,
    /// SIGINT - Ctrl+C interactive shutdown
    Interrupt,
    /// SIGQUIT - Quit signal
    Quit,
    /// Internal - Application-triggered shutdown (e.g., worker errors)
    Internal,
}

impl std::fmt::Display for ShutdownSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShutdownSignal::Terminate => write!(f, "SIGTERM"),
            ShutdownSignal::Interrupt => write!(f, "SIGINT"),
            ShutdownSignal::Quit => write!(f, "SIGQUIT"),
            ShutdownSignal::Internal => write!(f, "INTERNAL"),
        }
    }
}

/// Signal handler for graceful shutdown
pub struct SignalHandler {
    shutdown_signal: Option<ShutdownSignal>,
    internal_shutdown_notify: Arc<Notify>,
}

impl SignalHandler {
    /// Create a new signal handler
    pub fn new() -> Self {
        Self { shutdown_signal: None, internal_shutdown_notify: Arc::new(Notify::new()) }
    }

    /// Get a handle to trigger internal shutdown
    pub fn get_shutdown_trigger(&self) -> Arc<Notify> {
        self.internal_shutdown_notify.clone()
    }

    /// Wait for any shutdown signal and return which one was received
    pub async fn wait_for_shutdown(&mut self) -> ShutdownSignal {
        let signal = self.wait_for_signal().await;
        self.shutdown_signal = Some(signal);
        info!("🛑 Received shutdown signal: {}", signal);
        signal
    }

    /// Get the signal that triggered shutdown (if any)
    pub fn shutdown_signal(&self) -> Option<ShutdownSignal> {
        self.shutdown_signal
    }

    #[cfg(unix)]
    async fn wait_for_signal(&self) -> ShutdownSignal {
        // Set up signal handlers for Unix systems
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to create SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("Failed to create SIGINT handler");
        let mut sigquit = signal(SignalKind::quit()).expect("Failed to create SIGQUIT handler");

        info!("📡 Signal handler initialized, listening for SIGTERM, SIGINT, SIGQUIT, and internal shutdown requests");

        tokio::select! {
            _ = sigterm.recv() => {
                info!("Docker/Kubernetes graceful shutdown initiated (SIGTERM)");
                ShutdownSignal::Terminate
            }
            _ = sigint.recv() => {
                info!("Interactive shutdown initiated (SIGINT/Ctrl+C)");
                ShutdownSignal::Interrupt
            }
            _ = sigquit.recv() => {
                warn!("Force quit signal received (SIGQUIT)");
                ShutdownSignal::Quit
            }
            _ = self.internal_shutdown_notify.notified() => {
                warn!("Internal application shutdown requested (worker error or system inconsistency)");
                ShutdownSignal::Internal
            }
        }
    }

    #[cfg(not(unix))]
    async fn wait_for_signal(&self) -> ShutdownSignal {
        // Note: Since we dont have the device to test this, This code might be blackbox for now
        info!("Signal handler initialized (Windows), listening for Ctrl+C and internal shutdown requests");

        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Interactive shutdown initiated (Ctrl+C)");
                ShutdownSignal::Interrupt
            }
            _ = self.internal_shutdown_notify.notified() => {
                warn!("Internal application shutdown requested (worker error or system inconsistency)");
                ShutdownSignal::Internal
            }
        }
    }

    /// Handle shutdown with timeout and fallback
    pub async fn handle_graceful_shutdown<F, Fut>(&self, shutdown_fn: F, timeout_secs: u64) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let signal = self.shutdown_signal.unwrap_or(ShutdownSignal::Interrupt);

        info!("Starting graceful shutdown (triggered by: {})", signal);
        info!("Shutdown timeout: {} seconds", timeout_secs);

        // Try graceful shutdown with timeout
        let shutdown_future = shutdown_fn();
        let timeout_duration = tokio::time::Duration::from_secs(timeout_secs);

        match tokio::time::timeout(timeout_duration, shutdown_future).await {
            Ok(Ok(())) => {
                info!("✅ Graceful shutdown completed successfully");
                Ok(())
            }
            Ok(Err(e)) => {
                error!("❌ Graceful shutdown failed: {}", e);
                Err(e)
            }
            Err(_) => {
                error!("⏰ Graceful shutdown timed out after {} seconds", timeout_secs);

                // Different behavior based on signal type
                match signal {
                    ShutdownSignal::Quit => {
                        warn!("💥 SIGQUIT received - forcing immediate exit");
                        std::process::exit(1);
                    }
                    _ => {
                        warn!("🔌 Shutdown timeout reached - this may leave some tasks incomplete");
                        Err(anyhow!("Shutdown timeout exceeded"))
                    }
                }
            }
        }
    }
}

impl Default for SignalHandler {
    fn default() -> Self {
        Self::new()
    }
}
