use crate::{MadaraBackend, MadaraStorageError};
use mp_block::{header::PendingHeader, MadaraBlockInfo, MadaraPendingBlockInfo};
use std::sync::Arc;
use tokio::sync::{broadcast, watch};

pub type ClosedBlocksReceiver = broadcast::Receiver<Arc<MadaraBlockInfo>>;
pub type PendingBlockReceiver = watch::Receiver<Arc<MadaraPendingBlockInfo>>;

fn make_fake_pending_block(parent_block: Option<&MadaraBlockInfo>) -> Arc<MadaraPendingBlockInfo> {
    let Some(parent_block) = parent_block else {
        return Default::default(); // No genesis block, we have make it all up
    };
    MadaraPendingBlockInfo {
        header: PendingHeader {
            parent_block_hash: parent_block.block_hash,
            sequencer_address: parent_block.header.sequencer_address,
            block_timestamp: parent_block.header.block_timestamp, // Junk timestamp: unix epoch
            protocol_version: parent_block.header.protocol_version,
            l1_gas_price: parent_block.header.l1_gas_price.clone(),
            l1_da_mode: parent_block.header.l1_da_mode,
        },
        tx_hashes: vec![],
    }
    .into()
}

pub(crate) struct BlockWatch {
    closed_blocks: broadcast::Sender<Arc<MadaraBlockInfo>>,
    pending_block: watch::Sender<Arc<MadaraPendingBlockInfo>>,
}

impl BlockWatch {
    pub fn new() -> Self {
        Self {
            closed_blocks: broadcast::channel(100).0,
            pending_block: watch::channel(make_fake_pending_block(None)).0,
        }
    }

    pub fn init_initial_values(&self, db: &MadaraBackend) -> Result<(), MadaraStorageError> {
        let block = db.get_pending_block_info_from_db()?;
        self.update_pending(block.into());
        Ok(())
    }

    pub fn update_pending(&self, block: Arc<MadaraPendingBlockInfo>) {
        self.pending_block.send_replace(block);
    }

    pub fn clear_pending(&self, parent_block: Option<&MadaraBlockInfo>) {
        self.update_pending(make_fake_pending_block(parent_block));
    }

    pub fn on_new_block(&self, block: Arc<MadaraBlockInfo>) {
        let _no_listener_error = self.closed_blocks.send(Arc::clone(&block));
        self.update_pending(make_fake_pending_block(Some(&block)));
    }

    pub fn subscribe_closed_blocks(&self) -> ClosedBlocksReceiver {
        self.closed_blocks.subscribe()
    }
    pub fn subscribe_pending_block(&self) -> PendingBlockReceiver {
        self.pending_block.subscribe()
    }
    pub fn latest_pending_block(&self) -> Arc<MadaraPendingBlockInfo> {
        self.pending_block.borrow().clone()
    }
}

impl MadaraBackend {
    #[tracing::instrument(skip(self), fields(module = "MadaraBackendWatch"))]
    pub fn subscribe_closed_blocks(&self) -> ClosedBlocksReceiver {
        self.watch.subscribe_closed_blocks()
    }
    #[tracing::instrument(skip(self), fields(module = "MadaraBackendWatch"))]
    pub fn subscribe_pending_block(&self) -> PendingBlockReceiver {
        self.watch.subscribe_pending_block()
    }
    #[tracing::instrument(skip(self), fields(module = "MadaraBackendWatch"))]
    pub fn latest_pending_block(&self) -> Arc<MadaraPendingBlockInfo> {
        self.watch.latest_pending_block()
    }
}
