use crate::{MadaraBackend, MadaraStorageError};
use mp_block::{header::PendingHeader, MadaraBlockInfo, MadaraPendingBlockInfo};
use std::sync::Arc;

pub type ClosedBlocksReceiver = tokio::sync::broadcast::Receiver<Arc<MadaraBlockInfo>>;
pub type PendingBlockReceiver = tokio::sync::watch::Receiver<Arc<PendingTransport>>;
pub type PendingTxsReceiver = tokio::sync::broadcast::Receiver<mp_block::TransactionWithReceipt>;
pub type LastBlockOnL1Receiver = tokio::sync::watch::Receiver<Option<u64>>;

fn make_fake_pending_transport(parent_block: Option<&MadaraBlockInfo>) -> Arc<PendingTransport> {
    let Some(parent_block) = parent_block else {
        return Default::default(); // No genesis block, we have to make it all up
    };
    let block = mp_block::PendingFullBlock {
        header: PendingHeader {
            parent_block_hash: parent_block.block_hash,
            parent_block_number: Some(parent_block.header.block_number),
            sequencer_address: parent_block.header.sequencer_address,
            block_timestamp: parent_block.header.block_timestamp, // Junk timestamp: unix epoch
            protocol_version: parent_block.header.protocol_version,
            l1_gas_price: parent_block.header.l1_gas_price.clone(),
            l1_da_mode: parent_block.header.l1_da_mode,
        },
        ..Default::default()
    };
    let class_updates = Default::default();
    let contract_updates = Default::default();
    let transport = PendingTransport { block, class_updates, contract_updates };

    Arc::new(transport)
}

pub(crate) struct BlockWatch {
    closed_blocks: tokio::sync::broadcast::Sender<Arc<MadaraBlockInfo>>,
    pending_block: tokio::sync::watch::Sender<Arc<PendingTransport>>,
    pending_txs: tokio::sync::broadcast::Sender<mp_block::TransactionWithReceipt>,
    last_block_on_l1: tokio::sync::watch::Sender<Option<u64>>,
}

#[derive(Clone, Default)]
pub struct PendingTransport {
    pub block: mp_block::PendingFullBlock,
    class_updates: crate::class_db::ClassDbBlockUpdatePending,
    contract_updates: crate::contract_db::ContractDbBlockUpdatePending,
}

impl PendingTransport {
    pub fn new(
        block: mp_block::PendingFullBlock,
        class_updates: crate::class_db::ClassDbBlockUpdatePending,
        contract_updates: crate::contract_db::ContractDbBlockUpdatePending,
    ) -> Self {
        Self { block, class_updates, contract_updates }
    }
}

impl BlockWatch {
    pub fn new() -> Self {
        Self {
            closed_blocks: tokio::sync::broadcast::channel(100).0,
            pending_block: tokio::sync::watch::channel(make_fake_pending_transport(None)).0,
            pending_txs: tokio::sync::broadcast::channel(100).0,
            last_block_on_l1: tokio::sync::watch::channel(None).0,
        }
    }

    pub fn init_initial_values(&self, db: &MadaraBackend) -> Result<(), MadaraStorageError> {
        let info = db.get_pending_block_info_from_db()?;
        let block = mp_block::PendingFullBlock { header: info.header, ..Default::default() };
        let transport = PendingTransport { block, ..Default::default() };
        let latest_block = db.get_l1_last_confirmed_block()?;

        self.pending_block.send_replace(Arc::new(transport));
        self.last_block_on_l1.send_replace(latest_block);

        Ok(())
    }

    pub fn pending_update(&self, transport: Arc<PendingTransport>) {
        self.pending_block.send_replace(transport);
    }

    pub fn pending_clear(&self, parent_block: Option<&MadaraBlockInfo>) {
        self.pending_update(make_fake_pending_transport(parent_block));
    }

    pub fn update_last_block_on_l1(&self, latest_block: u64) {
        self.last_block_on_l1.send_replace(Some(latest_block));
    }

    pub fn on_new_pending_tx(&self, tx: mp_block::TransactionWithReceipt) {
        let _no_listener_error = self.pending_txs.send(tx);
    }

    pub fn on_new_block(&self, block: Arc<MadaraBlockInfo>) {
        let _no_listener_error = self.closed_blocks.send(Arc::clone(&block));
        self.pending_update(make_fake_pending_transport(Some(&block)));
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
    pub fn subscribe_last_block_on_l1(&self) -> LastBlockOnL1Receiver {
        self.last_block_on_l1.subscribe()
    }
    pub fn pending_latest(&self) -> Arc<PendingTransport> {
        Arc::clone(&self.pending_block.borrow())
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
    pub fn subscribe_last_block_on_l1(&self) -> LastBlockOnL1Receiver {
        self.watch_blocks.subscribe_last_block_on_l1()
    }
    #[tracing::instrument(skip_all, fields(module = "MadaraBackendWatch"))]
    pub fn pending_update(&self, transport: Arc<PendingTransport>) {
        self.watch_blocks.pending_update(transport)
    }
    #[tracing::instrument(skip_all, fields(module = "MadaraBackendWatch"))]
    pub fn pending_latest(&self) -> Arc<PendingTransport> {
        self.watch_blocks.pending_latest()
    }
}
