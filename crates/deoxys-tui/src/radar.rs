use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{self, Duration, SystemTime};

use starknet::core::types::SyncStatusType;
use starknet::providers::jsonrpc::{self, HttpTransport};
use starknet::providers::{Provider, Url};
use sysinfo::{Disks, Networks, Pid, ProcessRefreshKind, System};

pub struct Radar {
    _rpc_client: jsonrpc::JsonRpcClient<HttpTransport>,
    network: Networks,
    system: System,
    disks: Disks,
    storage_directory: String,
    pid: u32,
    last_network_refresh_time: SystemTime,
    cpu_antenna: mpsc::Receiver<f64>,
    logs_antenna: tokio::sync::mpsc::Receiver<String>,
}

impl Radar {
    pub fn new(target_storage_directory: &str, logs_rx: tokio::sync::mpsc::Receiver<String>) -> Result<Self, String> {
        let url = Url::parse("localhost:9944").map_err(|_| "Error: Not a Valid URL for RPC endpoint")?;
        let rpc_provider = jsonrpc::JsonRpcClient::new(HttpTransport::new(url));
        let sys = System::new_all();
        let disks = Disks::new();
        let mut network: Networks = Networks::new_with_refreshed_list();
        network.refresh();
        let (cpu_tx, cpu_rx) = mpsc::channel::<f64>();

        thread::spawn(move || {
            let tx = cpu_tx.clone();
            let mut system = System::new_all();
            let cpus_number = system.cpus().len();
            let pid = std::process::id();

            loop {
                system.refresh_processes_specifics(ProcessRefreshKind::new().with_cpu());
                std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
                system.refresh_processes_specifics(ProcessRefreshKind::new().with_cpu());
                let process = system.process(Pid::from_u32(pid)).unwrap();
                let usage = process.cpu_usage() as f64 / cpus_number as f64;
                if tx.send(usage).is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            logs_antenna: logs_rx,
            cpu_antenna: cpu_rx,
            _rpc_client: rpc_provider,
            disks,
            storage_directory: target_storage_directory.to_string(),
            system: sys,
            pid: std::process::id(),
            network,
            last_network_refresh_time: time::SystemTime::now(),
        })
    }
    pub async fn _get_block_number(&self) -> Result<u64, String> {
        self._rpc_client.block_number().await.map_err(|err| format!("Error: {:?}", err))
    }
    pub async fn _get_syncing(&self) -> Result<SyncStatusType, String> {
        self._rpc_client.syncing().await.map_err(|err| format!("Error: {:?}", err))
    }
    pub fn snapshot(&mut self) {
        self.network.refresh();
        self.last_network_refresh_time = time::SystemTime::now();
        self.system.refresh_processes_specifics(ProcessRefreshKind::new().with_memory());
        self.disks.refresh_list();
    }
    pub fn get_cpu_usage(&mut self) -> Option<f64> {
        match self.cpu_antenna.recv_timeout(Duration::from_millis(10)) {
            Ok(usage) => Some(usage),
            _ => None,
        }
    }
    pub fn get_memory_usage(&mut self) -> Option<u64> {
        let process = self.system.process(Pid::from_u32(self.pid)).unwrap();
        Some(process.memory())
    }
    pub fn get_total_system_memory(&mut self) -> u64 {
        self.system.refresh_memory();
        self.system.total_memory()
    }
    pub fn get_total_storage(&mut self) -> Option<u64> {
        self.disks.list().first().map(|disk| disk.total_space())
    }
    pub fn get_storage_usage(&mut self) -> u64 {
        let path = Path::new(&self.storage_directory);
        du::get_size(path).unwrap_or(0)
    }
    pub fn get_available_storage(&mut self) -> Option<u64> {
        self.disks.list().first().map(|elm| elm.available_space())
    }
    pub fn get_network_usage(&mut self) -> Option<(f64, f64)> {
        // Returns the data (rx, tx) rate in Mb/s
        self.network.refresh();
        let dt = self.last_network_refresh_time.elapsed().unwrap().as_millis() as f64 / 1000.;
        let received: u64 = self.network.into_iter().map(|(_, elm)| elm.received()).sum();
        let sent: u64 = self.network.into_iter().map(|(_, elm)| elm.transmitted()).sum();
        Some(((received as f64 / dt) / 1000000., (sent as f64 / dt) / 1000000.))
    }
    pub fn get_logs(&mut self) -> (Option<String>, Option<String>) {
        if let Ok(raw) = self.logs_antenna.try_recv() {
            if raw.starts_with('🔃') { (Some(raw), None) } else { (None, Some(raw)) }
        } else {
            (None, None)
        }
    }
}
