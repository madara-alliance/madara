use mp_convert::Felt;

#[derive(Debug, Default, Clone)]
pub enum SyncStatus {
    #[default]
    NotRunning,
    Running {
        highest_block_n: u64,
        highest_block_hash: Felt,
    },
}

#[derive(Debug, Default)]
pub(super) struct SyncStatusCell(std::sync::RwLock<SyncStatus>);
impl SyncStatusCell {
    fn set(&self, sync_status: SyncStatus) {
        *self.0.write().expect("Poisoned lock") = sync_status;
    }
    fn get(&self) -> SyncStatus {
        self.0.read().expect("Poisoned lock").clone()
    }
}
