#[derive(Debug, Clone, Default)]
pub struct WorkerState {
    pub is_running: bool,
    pub last_execution: Option<chrono::DateTime<chrono::Utc>>,
    pub consecutive_failures: u32,
}

impl WorkerState {
    pub fn new() -> Self {
        Self { is_running: false, last_execution: None, consecutive_failures: 0 }
    }
    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn set_running(&mut self, is_running: bool) {
        self.is_running = is_running;
    }
    pub fn last_execution(&self) -> Option<&chrono::DateTime<chrono::Utc>> {
        self.last_execution.as_ref()
    }

    pub fn set_last_execution(&mut self, last_execution: chrono::DateTime<chrono::Utc>) {
        self.last_execution = Some(last_execution);
    }
}
