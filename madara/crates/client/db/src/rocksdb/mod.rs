use crate::{
    prelude::*,
    rocksdb::{
        column::{Column, ALL_COLUMNS},
        options::rocksdb_global_options,
        snapshots::Snapshots,
        update_global_trie::apply_to_global_trie,
    },
    storage::{
        ChainTip, ClassInfoWithBlockN, CompiledSierraWithBlockN, DevnetPredeployedKeys, EventFilter, MadaraStorageRead,
        MadaraStorageWrite, StoredChainInfo, TxIndex,
    },
    view::PreconfirmedBlockTransaction,
};
use mp_block::{EventWithInfo, MadaraBlockInfo, TransactionWithReceipt};
use mp_convert::Felt;
use mp_state_update::StateDiff;
use mp_transactions::{validated::ValidatedMempoolTx, L1HandlerTransactionWithFee};
use rocksdb::{BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, FlushOptions, MultiThreaded, WriteOptions};
use std::{fmt, path::Path, sync::Arc};

mod blocks;
mod classes;
mod column;
mod events;
mod events_bloom_filter;
mod iter_pinned;
mod l1_to_l2_messages;
mod mempool;
mod meta;
mod options;
mod rocksdb_snapshot;
mod snapshots;
mod state;
mod trie;
mod update_global_trie;

type WriteBatchWithTransaction = rocksdb::WriteBatchWithTransaction<false>;
type DB = DBWithThreadMode<MultiThreaded>;

pub use options::RocksDBConfig;

const DB_UPDATES_BATCH_SIZE: usize = 1024;

fn serialize_to_smallvec<A: smallvec::Array<Item = u8>>(
    value: &impl serde::Serialize,
) -> Result<smallvec::SmallVec<A>, bincode::Error> {
    let mut v = Default::default();
    bincode::serialize_into(&mut v, value)?;
    Ok(v)
}

struct RocksDBStorageInner {
    db: DB,
    writeopts_no_wal: WriteOptions,
    config: RocksDBConfig,
}

impl Drop for RocksDBStorageInner {
    fn drop(&mut self) {
        // tracing::info!("⏳ Gracefully closing the database...");
        self.flush().expect("Error when flushing the database"); // flush :)
        self.db.cancel_all_background_work(true);
    }
}

impl fmt::Debug for RocksDBStorageInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DBInner").field("config", &self.config).finish()
    }
}

impl RocksDBStorageInner {
    fn get_column(&self, col: Column) -> Arc<BoundColumnFamily<'_>> {
        let name = col.rocksdb_name;
        match self.db.cf_handle(name) {
            Some(column) => column,
            None => panic!("column {name} not initialized"),
        }
    }

    fn flush(&self) -> anyhow::Result<()> {
        tracing::debug!("doing a db flush");
        let mut opts = FlushOptions::default();
        opts.set_wait(true);
        // we have to collect twice here :/
        let columns = column::ALL_COLUMNS.iter().map(|e| self.get_column(e.clone())).collect::<Vec<_>>();
        let columns = columns.iter().collect::<Vec<_>>();

        self.db.flush_cfs_opt(&columns, &opts).context("Flushing database")?;

        Ok(())
    }
}

/// Implementation of [`MadaraStorageRead`] and [`MadaraStorageWrite`] interface using rocksdb.
#[derive(Debug, Clone)]
pub struct RocksDBStorage(Arc<RocksDBStorageInner>, Arc<Snapshots>);

impl RocksDBStorage {
    pub fn open(path: &Path, config: &RocksDBConfig) -> Result<Self> {
        let opts = rocksdb_global_options(config)?;
        tracing::debug!("Opening db at {:?}", path.display());
        let db = DB::open_cf_descriptors(
            &opts,
            path,
            ALL_COLUMNS.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name, col.rocksdb_options(config))),
        )?;

        let mut writeopts_no_wal = WriteOptions::new();
        writeopts_no_wal.disable_wal(true);
        let inner = Arc::new(RocksDBStorageInner { writeopts_no_wal, db, config: config.clone() });

        let head_block_n = inner.get_chain_tip()?.and_then(|c| match c {
            ChainTip::BlockN(block_n) => Some(block_n),
            ChainTip::Preconfirmed(pending_header) => pending_header.block_number.checked_sub(1),
        });

        let snapshot = Snapshots::new(inner.clone(), head_block_n, config.max_kept_snapshots, config.snapshot_interval);

        Ok(Self(inner, snapshot.into()))
    }
}

impl MadaraStorageRead for RocksDBStorage {
    // Blocks

    fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>> {
        self.0
            .find_block_hash(block_hash)
            .with_context(|| format!("Finding block number for block_hash={block_hash:#x}"))
    }
    fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<TxIndex>> {
        self.0
            .find_transaction_hash(tx_hash)
            .with_context(|| format!("Finding transaction index for tx_hash={tx_hash:#x}"))
    }
    fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>> {
        self.0.get_block_info(block_n).with_context(|| format!("Getting block info for block_n={block_n}"))
    }
    fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>> {
        self.0.get_block_state_diff(block_n).with_context(|| format!("Getting block state diff for block_n={block_n}"))
    }
    fn get_transaction(&self, block_n: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        self.0
            .get_transaction(block_n, tx_index)
            .with_context(|| format!("Getting block transaction for block_n={block_n} tx_index={tx_index}"))
    }
    fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt>> + '_ {
        self.0.get_block_transactions(block_n, from_tx_index).map(move |e| {
            e.with_context(|| format!("Getting block transactions for block_n={block_n} from_tx_index={from_tx_index}"))
        })
    }

    // State

    fn get_storage_at(&self, block_n: u64, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>> {
        self.0.get_storage_at(block_n, contract_address, key).with_context(|| {
            format!("Getting storage value for block_n={block_n} contract_address={contract_address:#x} key={key:#x}")
        })
    }
    fn get_contract_nonce_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.0
            .get_contract_nonce_at(block_n, contract_address)
            .with_context(|| format!("Getting nonce for block_n={block_n} contract_address={contract_address:#x}"))
    }
    fn get_contract_class_hash_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.0
            .get_contract_class_hash_at(block_n, contract_address)
            .with_context(|| format!("Getting class_hash for block_n={block_n} contract_address={contract_address:#x}"))
    }
    fn is_contract_deployed_at(&self, block_n: u64, contract_address: &Felt) -> Result<bool> {
        self.0.is_contract_deployed_at(block_n, contract_address).with_context(|| {
            format!("Checking if contract is deployed for block_n={block_n} contract_address={contract_address:#x}")
        })
    }

    // Classes

    fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>> {
        self.0.get_class(class_hash).with_context(|| format!("Getting class info for class_hash={class_hash:#x}"))
    }
    fn get_class_compiled(&self, compiled_class_hash: &Felt) -> Result<Option<CompiledSierraWithBlockN>> {
        self.0
            .get_class_compiled(compiled_class_hash)
            .with_context(|| format!("Getting class compiled for compiled_class_hash={compiled_class_hash:#x}"))
    }

    // Events

    fn get_events(&self, filter: EventFilter) -> Result<Vec<EventWithInfo>> {
        self.0.get_filtered_events(filter.clone()).with_context(|| format!("Getting events for filter={filter:?}"))
    }

    // Meta

    fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        self.0.get_devnet_predeployed_keys().context("Getting devnet predeployed contracts keys")
    }
    fn get_chain_tip(&self) -> Result<Option<ChainTip>> {
        self.0.get_chain_tip().context("Getting chain tip from db")
    }
    fn get_preconfirmed_content(&self) -> impl Iterator<Item = Result<PreconfirmedBlockTransaction>> + '_ {
        self.0.get_preconfirmed_content().map(|res| res.context("Getting preconfirmed block content from db"))
    }
    fn get_confirmed_on_l1_tip(&self) -> Result<Option<u64>> {
        self.0.get_confirmed_on_l1_tip().context("Getting confirmed block on l1 tip")
    }
    fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>> {
        self.0.get_l1_messaging_sync_tip().context("Getting l1 messaging sync tip")
    }
    fn get_stored_chain_info(&self) -> Result<Option<StoredChainInfo>> {
        self.0.get_stored_chain_info().context("Getting stored chain info from db")
    }

    // L1 to L2 messages

    fn get_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.0
            .get_pending_message_to_l2(core_contract_nonce)
            .with_context(|| format!("Getting pending message to l2 with nonce={core_contract_nonce}"))
    }
    fn get_next_pending_message_to_l2(&self, start_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.0
            .get_next_pending_message_to_l2(start_nonce)
            .with_context(|| format!("Getting next pending message to l2 with start_nonce={start_nonce}"))
    }
    fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        self.0
            .get_l1_handler_txn_hash_by_nonce(core_contract_nonce)
            .with_context(|| format!("Getting next pending message to l2 with nonce={core_contract_nonce}"))
    }

    // Mempool

    fn get_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedMempoolTx>> + '_ {
        self.0.get_mempool_transactions().map(|res| res.context("Getting mempool transactions"))
    }
}

impl MadaraStorageWrite for RocksDBStorage {
    fn write_header(&self, header: mp_block::BlockHeaderWithSignatures) -> Result<()> {
        let block_n = header.header.block_number;
        self.0.blocks_store_block_header(header).with_context(|| format!("Storing block_header for block_n={block_n}"))
    }

    fn write_transactions(&self, block_n: u64, txs: &[TransactionWithReceipt]) -> Result<()> {
        // Save l1 core contract nonce to tx mapping.
        self.0
            .messages_to_l2_write_trasactions(
                txs.iter().filter_map(|v| v.transaction.as_l1_handler().zip(v.receipt.as_l1_handler())),
            )
            .with_context(|| format!("Updating L1 state when storing transactions for block_n={block_n}"))?;

        self.0
            .blocks_store_transactions(block_n, txs)
            .context("Storing transactions")
            .with_context(|| format!("Storing transactions for block_n={block_n}"))
    }

    fn write_state_diff(&self, block_n: u64, value: &StateDiff) -> Result<()> {
        self.0
            .blocks_store_state_diff(block_n, value)
            .with_context(|| format!("Storing state diff for block_n={block_n}"))?;
        self.0
            .state_apply_state_diff(block_n, value)
            .with_context(|| format!("Applying state from state diff for block_n={block_n}"))
    }

    fn write_events(&self, block_n: u64, events: &[mp_receipt::EventWithTransactionHash]) -> Result<()> {
        self.0
            .blocks_store_events_to_receipts(block_n, events)
            .with_context(|| format!("Storing events to receipts for block_n={block_n}"))?;
        self.0
            .store_events_bloom(block_n, events)
            .with_context(|| format!("Storing events bloom filter for block_n={block_n}"))
    }

    fn write_chain_tip(&self, chain_tip: &ChainTip) -> Result<()> {
        self.0.write_chain_tip(chain_tip).context("Writing chain tip to db")
    }

    fn append_preconfirmed_content(&self, start_tx_index: u64, txs: &[PreconfirmedBlockTransaction]) -> Result<()> {
        self.0.append_preconfirmed_content(start_tx_index, txs).context("Appending to preconfirmed content to db")
    }

    fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        self.0.write_l1_handler_txn_hash_by_nonce(core_contract_nonce, txn_hash).with_context(|| {
            format!("Writing l1 handler txn hash by nonce nonce={core_contract_nonce} txn_hash={txn_hash:#x}")
        })
    }
    fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        let nonce = msg.tx.nonce;
        self.0.write_pending_message_to_l2(msg).with_context(|| format!("Writing pending message to l2 nonce={nonce}"))
    }
    fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        self.0
            .remove_pending_message_to_l2(core_contract_nonce)
            .with_context(|| format!("Removing pending message to l2 nonce={core_contract_nonce}"))
    }

    fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()> {
        self.0.write_devnet_predeployed_keys(devnet_keys).context("Writing devnet predeployed keys to db")
    }
    fn write_l1_messaging_sync_tip(&self, block_n: u64) -> Result<()> {
        self.0.write_l1_messaging_sync_tip(block_n).context("Writing l1 messaging sync tip")
    }
    fn write_confirmed_on_l1_tip(&self, block_n: u64) -> Result<()> {
        self.0.write_confirmed_on_l1_tip(block_n).context("Writing confirmed on l1 tip")
    }

    fn remove_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()> {
        self.0.remove_mempool_transactions(tx_hashes).context("Removing mempool transactions from db")
    }
    fn write_mempool_transaction(&self, tx: &ValidatedMempoolTx) -> Result<()> {
        let tx_hash = tx.tx_hash;
        self.0
            .write_mempool_transaction(tx)
            .with_context(|| format!("Writing mempool transaction from db for tx_hash={tx_hash:#x}"))
    }

    fn apply_to_global_trie<'a>(
        &self,
        start_block_n: u64,
        state_diffs: impl IntoIterator<Item = &'a StateDiff>,
    ) -> Result<Felt> {
        apply_to_global_trie(self, start_block_n, state_diffs).context("Applying state diff to global trie")
    }

    fn flush(&self) -> Result<()> {
        self.0.flush().context("Flushing RocksDB database")
    }
}
