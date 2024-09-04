use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Context;
use futures::SinkExt;
use mp_utils::channel_wait_or_graceful_shutdown;
use mp_utils::service::Service;
use reqwest_websocket::{Message, RequestBuilderExt};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

mod sysinfo;
pub use sysinfo::*;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum VerbosityLevel {
    Info = 0,
    Debug = 1,
}

#[derive(Debug)]
pub struct TelemetryEvent {
    verbosity: VerbosityLevel,
    message: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct TelemetryHandle(Option<Arc<mpsc::Sender<TelemetryEvent>>>);

impl TelemetryHandle {
    pub fn send(&self, verbosity: VerbosityLevel, message: serde_json::Value) {
        if message.get("msg").is_none() {
            log::warn!("Telemetry messages should have a message type");
        }
        if let Some(tx) = &self.0 {
            // drop the message if the channel if full.
            let _ = tx.try_send(TelemetryEvent { verbosity, message });
        }
    }
}
pub struct TelemetryService {
    no_telemetry: bool,
    telemetry_endpoints: Vec<(String, u8)>,
    telemetry_handle: TelemetryHandle,
    start_state: Option<mpsc::Receiver<TelemetryEvent>>,
}

impl TelemetryService {
    pub fn new(no_telemetry: bool, telemetry_endpoints: Vec<(String, u8)>) -> anyhow::Result<Self> {
        let (telemetry_handle, start_state) = if no_telemetry {
            (TelemetryHandle(None), None)
        } else {
            let (tx, rx) = mpsc::channel(1024);
            (TelemetryHandle(Some(Arc::new(tx))), Some(rx))
        };
        Ok(Self { no_telemetry, telemetry_endpoints, telemetry_handle, start_state })
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
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if self.no_telemetry {
            return Ok(());
        }

        let telemetry_endpoints = self.telemetry_endpoints.clone();
        let mut rx = self.start_state.take().context("the service has already been started")?;

        join_set.spawn(async move {
            let client = &reqwest::Client::default();
            let mut clients = futures::future::join_all(telemetry_endpoints.iter().map(|(endpoint, pr)| async move {
                let websocket = match client.get(endpoint).upgrade().send().await {
                    Ok(ws) => ws,
                    Err(err) => {
                        log::warn!("Could not connect to telemetry endpoint '{endpoint}': {err:?}");
                        return None;
                    }
                };
                let websocket = match websocket.into_websocket().await {
                    Ok(ws) => ws,
                    Err(err) => {
                        log::warn!("Could not connect websocket to telemetry endpoint '{endpoint}': {err:?}");
                        return None;
                    }
                };
                Some((websocket, *pr, endpoint.clone()))
            }))
            .await;

            let rx = &mut rx;

            while let Some(event) = channel_wait_or_graceful_shutdown(rx.recv()).await {
                log::debug!(
                    "Sending telemetry event '{}'.",
                    event.message.get("msg").and_then(|e| e.as_str()).unwrap_or("<unknown>")
                );
                let ts = chrono::Local::now().to_rfc3339();
                let msg = serde_json::json!({ "id": 1, "ts": ts, "payload": event.message });
                let msg = &serde_json::to_string(&msg).context("serializing telemetry message to string")?;

                futures::future::join_all(clients.iter_mut().map(|client| async move {
                    if let Some((websocket, verbosity, endpoint)) = client {
                        if *verbosity >= event.verbosity as u8 {
                            log::trace!("send telemetry to '{endpoint}'");
                            match websocket.send(Message::Text(msg.clone())).await {
                                Ok(_) => {}
                                Err(err) => {
                                    log::warn!("Could not connect send telemetry to endpoint '{endpoint}': {err:#}");
                                }
                            }
                        }
                    }
                }))
                .await;
            }

            Ok(())
        });

        Ok(())
    }
}
