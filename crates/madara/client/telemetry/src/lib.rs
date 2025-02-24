use std::time::SystemTime;

use anyhow::Context;
use futures::SinkExt;
use mp_utils::service::{MadaraServiceId, Service, ServiceContext, ServiceId, ServiceIdProvider, ServiceRunner};
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

impl ServiceIdProvider for TelemetryService {
    #[inline(always)]
    fn id_provider(&self) -> impl ServiceId {
        MadaraServiceId::Telemetry
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
