//! Madara telemetry service. This crate manages telemetry data collection and transmission
//! for Madara nodes, sending metrics and system information to configured endpoints.
//!
//! # Overview
//!
//! The Telemetry service collects and transmits operational data from Madara nodes to remote
//! telemetry endpoints. This enables monitoring of node health, performance metrics, and system
//! information. The service operates asynchronously, buffering events and transmitting them via
//! WebSocket connections without blocking node operation.
//!
//! The telemetry system is split into two main components: the [`TelemetryService`] which manages
//! WebSocket connections and event transmission, and the [`TelemetryHandle`] which provides a
//! lightweight interface for other parts of the system to send telemetry events.
//!
//! # Architecture
//!
//! The telemetry system consists of several key components working together:
//!
//! - [`TelemetryService`]: The main service that manages WebSocket connections to telemetry endpoints.
//! - [`TelemetryHandle`]: A cloneable handle for sending telemetry events from anywhere in the system.
//! - [`TelemetryEvent`]: Individual telemetry messages with associated verbosity levels.
//! - [`SysInfo`]: System information collector that gathers hardware and OS details.
//! - [`VerbosityLevel`]: Controls which events are sent to which endpoints based on their verbosity.
//!
//! # Telemetry Endpoints and Verbosity
//!
//! The service supports multiple telemetry endpoints, each configured with a verbosity level:
//!
//! - **Info level (0)**: Essential operational events like node startup, connection status
//! - **Debug level (1)**: Detailed diagnostic information for troubleshooting
//!
//! Endpoints only receive events at or below their configured verbosity level, allowing
//! different monitoring systems to receive different levels of detail.
//!
//! # Event Transmission Flow
//!
//! When a telemetry event is sent through the system:
//!
//! 1. **Event Creation**: Components create events via [`TelemetryHandle::send`] with a verbosity
//!    level and JSON message payload.
//! 2. **Event Buffering**: Events are sent through a broadcast channel to the telemetry service.
//! 3. **WebSocket Transmission**: The service processes events and sends them to all connected
//!    endpoints that match the event's verbosity level.
//! 4. **Error Handling**: Connection failures are logged but don't block the service.
//!
//! # System Information Collection
//!
//! The [`SysInfo`] component automatically collects system information including:
//!
//! - CPU information (brand, core count, architecture).
//! - Memory information (total RAM).
//! - Operating system details (distribution, kernel version).
//! - Target platform information (OS, architecture, environment).
//!
//! This information is transmitted during the initial connection event and provides context
//! for telemetry data analysis.
//!
//! # Service Lifecycle
//!
//! The telemetry service follows the standard Madara service pattern:
//!
//! 1. **Initialization**: Create the service with configured endpoints.
//! 2. **Connection**: Establish WebSocket connections to all configured endpoints.
//! 3. **Operation**: Continuously process telemetry events from the broadcast channel.
//! 4. **Transmission**: Send events to appropriate endpoints based on verbosity.
//! 5. **Shutdown**: Gracefully close connections when the service stops.
//!
//! # Connection Management
//!
//! WebSocket connections are established asynchronously during service startup. If an endpoint
//! is unreachable, the service logs a warning but continues operating with the remaining
//! connections. Failed connections are not automatically retried - the service assumes
//! telemetry endpoints are monitoring infrastructure that should be highly available.
//!
//! # Usage Example
//!
//! ```rust
//! // Create telemetry service with endpoints
//! let endpoints = vec![
//!     ("wss://telemetry.example.com".to_string(), 0), // Info level
//!     ("wss://debug.example.com".to_string(), 1),     // Debug level
//! ];
//! let telemetry = TelemetryService::new(endpoints)?;
//!
//! // Get a handle for sending events
//! let handle = telemetry.new_handle();
//!
//! // Send telemetry events
//! handle.send(VerbosityLevel::Info, serde_json::json!({
//!     "msg": "block.imported",
//!     "block_number": 12345,
//!     "block_hash": "0x..."
//! }));
//! ```
//!
//! # Error Handling
//!
//! The telemetry service is designed to be resilient to failures:
//!
//! - WebSocket connection failures are logged but don't crash the service.
//! - Malformed events are ignored rather than stopping transmission.
//! - The broadcast channel has a buffer to handle temporary transmission delays.
//! - Missing required fields (like "msg") generate warnings but don't block processing.
//!
//! # Performance Considerations
//!
//! The telemetry system is designed to have minimal impact on node performance:
//!
//! - Events are transmitted asynchronously via broadcast channels.
//! - WebSocket connections operate independently of core node operations.
//! - System information is collected once at startup rather than repeatedly.
//! - The broadcast channel is bounded to prevent unbounded memory growth.
//!
//! [`TelemetryService`]: TelemetryService
//! [`TelemetryHandle`]: TelemetryHandle
//! [`TelemetryEvent`]: TelemetryEvent
//! [`TelemetryHandle::send`]: TelemetryHandle::send
//! [`SysInfo`]: SysInfo
//! [`VerbosityLevel`]: VerbosityLevel
use std::time::SystemTime;

use anyhow::Context;
use futures::SinkExt;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceContext, ServiceId, ServiceRunner};
use reqwest_websocket::{Message, RequestBuilderExt, WebSocket};

mod sysinfo;
pub use sysinfo::*;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum VerbosityLevel {
    Info = 0,
    Debug = 1,
}

#[derive(Debug, Clone)]
pub struct TelemetryEvent {
    verbosity: VerbosityLevel,
    message: serde_json::Value,
}

#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct TelemetryHandle(tokio::sync::broadcast::Sender<TelemetryEvent>);

impl TelemetryHandle {
    pub fn send(&self, verbosity: VerbosityLevel, message: serde_json::Value) {
        if message.get("msg").is_none() {
            tracing::warn!("Telemetry messages should have a message type");
        }
        let _ = self.0.send(TelemetryEvent { verbosity, message });
    }
}
pub struct TelemetryService {
    telemetry_endpoints: Vec<(String, u8)>,
    telemetry_handle: TelemetryHandle,
}

impl TelemetryService {
    pub fn new(telemetry_endpoints: Vec<(String, u8)>) -> anyhow::Result<Self> {
        let telemetry_handle = TelemetryHandle(tokio::sync::broadcast::channel(1024).0);
        Ok(Self { telemetry_endpoints, telemetry_handle })
    }

    pub fn new_handle(&self) -> TelemetryHandle {
        self.telemetry_handle.clone()
    }

    pub fn send_connected(&self, name: &str, version: &str, network_id: &str, sys_info: &SysInfo) {
        let startup_time = SystemTime::UNIX_EPOCH.elapsed().map(|dur| dur.as_millis()).unwrap_or(0).to_string();

        let msg = serde_json::json!({
            "chain": "Starknet",
            "authority": false,
            "config": "",
            "genesis_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "implementation": "Madara Node",
            "msg": "system.connected",
            "name": name,
            "network_id": network_id,
            "startup_time": startup_time,
            "sysinfo": {
              "core_count": sys_info.core_count,
              "cpu": sys_info.cpu,
              // "is_virtual_machine": false,
              "linux_distro": sys_info.linux_distro,
              "linux_kernel": sys_info.linux_kernel,
              "memory": sys_info.memory,
            },
            "target_os": TARGET_OS,
            "target_arch": TARGET_ARCH,
            "target_env": TARGET_ENV,
            "version": version,
        });

        self.telemetry_handle.send(VerbosityLevel::Info, msg)
    }
}

#[async_trait::async_trait]
impl Service for TelemetryService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let rx = self.telemetry_handle.0.subscribe();
        let clients = start_clients(&self.telemetry_endpoints).await;

        runner.service_loop(move |ctx| start_telemetry(rx, ctx, clients));

        anyhow::Ok(())
    }
}

impl ServiceId for TelemetryService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::Telemetry.svc_id()
    }
}

async fn start_clients(telemetry_endpoints: &[(String, u8)]) -> Vec<Option<(WebSocket, u8, String)>> {
    let client = &reqwest::Client::default();
    futures::future::join_all(telemetry_endpoints.iter().map(|(endpoint, pr)| async move {
        let websocket = match client.get(endpoint).upgrade().send().await {
            Ok(ws) => ws,
            Err(err) => {
                tracing::warn!("Failed to connect to telemetry endpoint '{endpoint}': {err:?}");
                return None;
            }
        };
        let websocket = match websocket.into_websocket().await {
            Ok(ws) => ws,
            Err(err) => {
                tracing::warn!("Failed to connect websocket to telemetry endpoint '{endpoint}': {err:?}");
                return None;
            }
        };
        Some((websocket, *pr, endpoint.clone()))
    }))
    .await
}

async fn start_telemetry(
    mut rx: tokio::sync::broadcast::Receiver<TelemetryEvent>,
    mut ctx: ServiceContext,
    mut clients: Vec<Option<(WebSocket, u8, String)>>,
) -> anyhow::Result<()> {
    while let Some(Ok(event)) = ctx.run_until_cancelled(rx.recv()).await {
        tracing::debug!(
            "Sending telemetry event '{}'.",
            event.message.get("msg").and_then(|e| e.as_str()).unwrap_or("<unknown>")
        );

        let ts = chrono::Local::now().to_rfc3339();
        let msg = serde_json::json!({ "id": 1, "ts": ts, "payload": event.message });
        let msg = &serde_json::to_string(&msg).context("serializing telemetry message to string")?;

        futures::future::join_all(clients.iter_mut().map(|client| async move {
            if let Some((websocket, verbosity, endpoint)) = client {
                if *verbosity >= event.verbosity as u8 {
                    tracing::trace!("Sending telemetry to '{endpoint}'");
                    if let Err(err) = websocket.send(Message::Text(msg.clone())).await {
                        tracing::warn!("Failed to send telemetry to endpoint '{endpoint}': {err:#}");
                    }
                }
            }
        }))
        .await;
    }

    anyhow::Ok(())
}
