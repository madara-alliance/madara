use starknet::core::types::SyncStatusType;
use tokio::sync::mpsc as tmpsc;

use crate::radar::Radar;

pub struct App {
    pub should_quit: bool,
    pub data: Metrics,
    radar: Radar,
}

pub struct Metrics {
    pub block_number: Result<u64, String>,
    pub syncing: Result<SyncStatusType, String>,
    pub cpu_name: String,
    pub cpu_usage: Vec<f64>,
    pub memory_usage: Vec<u64>,
    pub total_memory: u64,
    pub disk_name: String,
    pub disk_size: u64,
    pub disk_usage: u64,
    pub available_storage: u64,
    pub rx_flow: Vec<f64>, // Mb/s
    pub tx_flow: Vec<f64>,
    pub l1_logs: Vec<Option<String>>,
    pub l2_logs: Vec<Option<String>>,
}

impl App {
    pub fn new(storage_path: &str, logs_rx: tmpsc::Receiver<String>) -> Result<Self, String> {
        let mut radar = Radar::new(storage_path, logs_rx)?;
        let total_memory = radar.get_total_system_memory();

        Ok(Self {
            should_quit: false,
            radar,
            data: Metrics {
                block_number: Ok(0),
                syncing: Ok(SyncStatusType::NotSyncing),
                cpu_name: "unknown".to_string(),
                cpu_usage: vec![0.; 100],
                memory_usage: vec![0; 100],
                total_memory,
                disk_name: "unknown".to_string(),
                disk_size: 0,
                disk_usage: 0,
                available_storage: 0,
                rx_flow: vec![0.; 100],
                tx_flow: vec![0.; 100],
                l1_logs: vec![None; 100],
                l2_logs: vec![None; 100],
            },
        })
    }
    pub async fn update_metrics(&mut self) {
        self.radar.snapshot();

        self.data.cpu_usage.rotate_left(1);
        if let Some(cpu_usage) = self.radar.get_cpu_usage() {
            self.data.cpu_usage[99] = cpu_usage;
        } else {
            self.data.cpu_usage[99] = self.data.cpu_usage[98];
        }

        self.data.memory_usage.rotate_left(1);
        self.data.memory_usage[99] = self.radar.get_memory_usage().unwrap_or(0);

        self.data.disk_size = self.radar.get_total_storage().unwrap_or(0);
        self.data.disk_usage = self.radar.get_storage_usage();
        self.data.available_storage = self.radar.get_available_storage().unwrap_or(0);

        self.data.rx_flow.rotate_left(1);
        self.data.tx_flow.rotate_left(1);
        let (rxf, txf) = self.radar.get_network_usage().unwrap_or((0., 0.));
        self.data.rx_flow[99] = (rxf * 1000.).round() / 1000.;
        self.data.tx_flow[99] = (txf * 1000.).round() / 1000.;

        let (l1_log, l2_log) = self.radar.get_logs();
        if l1_log.is_some() {
            self.data.l1_logs.rotate_left(1);
            self.data.l1_logs[99] = l1_log;
        }
        if l2_log.is_some() {
            self.data.l2_logs.rotate_left(1);
            self.data.l2_logs[99] = l2_log;
        }
    }
}
