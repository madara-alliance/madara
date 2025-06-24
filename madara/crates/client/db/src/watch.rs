use crate::{MadaraBackend, MadaraStorageError};
use mp_block::{header::PendingHeader, MadaraBlockInfo};

pub type ClosedBlocksReceiver = tokio::sync::broadcast::Receiver<std::sync::Arc<MadaraBlockInfo>>;
pub type PendingBlockReceiver = tokio::sync::watch::Receiver<std::sync::Arc<PendingBlockTransport>>;
pub type PendingTxsReceiver = tokio::sync::broadcast::Receiver<mp_block::TransactionWithReceipt>;
pub type LastConfirmedBlockReceived = tokio::sync::watch::Receiver<Option<u64>>;

fn make_fake_pending_block(parent_block: Option<&MadaraBlockInfo>) -> std::sync::Arc<PendingBlockTransport> {
    parent_block
        .cloned()
        .map(|parent_block| {
            std::sync::Arc::new(PendingBlockTransport {
                block: mp_block::PendingFullBlock {
                    header: PendingHeader {
                        parent_block_hash: parent_block.block_hash,
                        block_number: parent_block.header.block_number + 1,
                        sequencer_address: parent_block.header.sequencer_address,
                        block_timestamp: parent_block.header.block_timestamp, // Junk timestamp: unix epoch
                        protocol_version: parent_block.header.protocol_version,
                        l1_gas_price: parent_block.header.l1_gas_price,
                        l1_da_mode: parent_block.header.l1_da_mode,
                    },
                    state_diff: Default::default(),
                    transactions: Default::default(),
                    events: Default::default(),
                },
                ..Default::default()
            })
        })
        .unwrap_or_default()
}

pub(crate) struct BlockWatch {
    closed_blocks: tokio::sync::broadcast::Sender<std::sync::Arc<MadaraBlockInfo>>,
    pending_block: tokio::sync::watch::Sender<std::sync::Arc<PendingBlockTransport>>,
    pending_txs: tokio::sync::broadcast::Sender<mp_block::TransactionWithReceipt>,
    last_confirmed_block: tokio::sync::watch::Sender<Option<u64>>,
}

#[derive(Debug, Default)]
pub struct PendingBlockTransport {
    pub block: mp_block::PendingFullBlock,
    pub contracts: crate::contract_db::ContractUpdates,
    pub classes: crate::class_db::ClassUpdates,
}

impl BlockWatch {
    pub fn new() -> Self {
        Self {
            closed_blocks: tokio::sync::broadcast::channel(100).0,
            pending_block: tokio::sync::watch::channel(make_fake_pending_block(None)).0,
            pending_txs: tokio::sync::broadcast::channel(100).0,
            last_confirmed_block: tokio::sync::watch::channel(None).0,
        }
    }

    pub fn init_initial_values(&self, db: &MadaraBackend) -> Result<(), MadaraStorageError> {
        db.get_l1_last_confirmed_block().map(|block| {
            self.last_confirmed_block.send_replace(block);
        })
    }

    pub fn update_pending(&self, block: std::sync::Arc<PendingBlockTransport>) {
        self.pending_block.send_replace(block);
    }

    pub fn update_last_confirmed_block(&self, latest_block: u64) {
        self.last_confirmed_block.send_replace(Some(latest_block));
    }

    pub fn pending_clear(&self, parent_block: Option<&MadaraBlockInfo>) {
        self.update_pending(make_fake_pending_block(parent_block));
    }

    pub fn on_new_pending_tx(&self, tx: mp_block::TransactionWithReceipt) {
        let _no_listener_error = self.pending_txs.send(tx);
    }

    pub fn on_new_block(&self, block: std::sync::Arc<MadaraBlockInfo>) {
        let _no_listener_error = self.closed_blocks.send(std::sync::Arc::clone(&block));
        self.update_pending(make_fake_pending_block(Some(&block)));
    }

    pub fn subscribe_closed_blocks(&self) -> ClosedBlocksReceiver {
        self.closed_blocks.subscribe()
    }
    pub fn subscribe_pending_txs(&self) -> PendingTxsReceiver {
        self.pending_txs.subscribe()
    }
    pub fn subscribe_pending_block(&self) -> PendingBlockReceiver {
        self.pending_block.subscribe()
    }
    pub fn subscribe_last_confirmed_block(&self) -> LastConfirmedBlockReceived {
        self.last_confirmed_block.subscribe()
    }
    pub fn latest_pending_block(&self) -> std::sync::Arc<PendingBlockTransport> {
        std::sync::Arc::clone(&self.pending_block.borrow())
    }
}

impl MadaraBackend {
    #[tracing::instrument(skip_all, fields(module = "MadaraBackendWatch"))]
    pub fn subscribe_closed_blocks(&self) -> ClosedBlocksReceiver {
        self.watch_blocks.subscribe_closed_blocks()
    }
    #[tracing::instrument(skip_all, fields(module = "MadaraBackendWatch"))]
    pub fn on_new_pending_tx(&self, tx: mp_block::TransactionWithReceipt) {
        self.watch_blocks.on_new_pending_tx(tx);
    }
    #[tracing::instrument(skip_all, fields(module = "MadaraBackendWatch"))]
    pub fn subscribe_pending_txs(&self) -> PendingTxsReceiver {
        self.watch_blocks.subscribe_pending_txs()
    }
    #[tracing::instrument(skip_all, fields(module = "MadaraBackendWatch"))]
    pub fn subscribe_pending_block(&self) -> PendingBlockReceiver {
        self.watch_blocks.subscribe_pending_block()
    }
    #[tracing::instrument(skip_all, fields(module = "MadaraBackendWatch"))]
    pub fn subscribe_last_confirmed_block(&self) -> LastConfirmedBlockReceived {
        self.watch_blocks.subscribe_last_confirmed_block()
    }
    #[tracing::instrument(skip_all, fields(module = "MadaraBackendWatch"))]
    pub fn pending_latest(&self) -> std::sync::Arc<PendingBlockTransport> {
        self.watch_blocks.latest_pending_block()
    }
}
