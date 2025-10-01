use crate::{
    preconfirmed::PreconfirmedExecutedTransaction,
    prelude::*,
    rocksdb::{
        backup::BackupManager,
        column::{Column, ALL_COLUMNS},
        meta::StoredChainTipWithoutContent,
        metrics::DbMetrics,
        options::rocksdb_global_options,
        snapshots::Snapshots,
        update_global_trie::apply_to_global_trie,
    },
    storage::{
        ClassInfoWithBlockN, CompiledSierraWithBlockN, DevnetPredeployedKeys, EventFilter, MadaraStorageRead,
        MadaraStorageWrite, StorageChainTip, StorageTxIndex, StoredChainInfo,
    },
};
use bincode::Options;
use blockifier::bouncer::BouncerWeights;

use mp_block::{EventWithInfo, MadaraBlockInfo, TransactionWithReceipt};
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_state_update::StateDiff;
use mp_transactions::{validated::ValidatedTransaction, L1HandlerTransactionWithFee};
use rocksdb::{BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, FlushOptions, MultiThreaded, WriteOptions};
use std::{fmt, path::Path, sync::Arc};

mod backup;
mod blocks;
mod classes;
mod column;
mod events;
mod events_bloom_filter;
mod iter_pinned;
mod l1_to_l2_messages;
mod mempool;
mod meta;
mod metrics;
mod options;
mod rocksdb_snapshot;
mod snapshots;
mod state;

// TODO: remove this pub. this is temporary until get_storage_proof is properly abstracted.
pub mod trie;
// TODO: remove this pub. this is temporary until get_storage_proof is properly abstracted.
pub mod update_global_trie;

type WriteBatchWithTransaction = rocksdb::WriteBatchWithTransaction<false>;
type DB = DBWithThreadMode<MultiThreaded>;

pub use options::{RocksDBConfig, StatsLevel};

const DB_UPDATES_BATCH_SIZE: usize = 1024;

fn bincode_opts() -> impl bincode::Options {
    bincode::DefaultOptions::new()
}

fn serialize_to_smallvec<A: smallvec::Array<Item = u8>>(
    value: &impl serde::Serialize,
) -> Result<smallvec::SmallVec<A>, bincode::Error> {
    let mut opt = bincode_opts();
    let mut v = smallvec::SmallVec::with_capacity((&mut opt).serialized_size(value)? as usize);
    // this *doesn't* call serialized_size under the hood - we have to do it ourselves to match this optimisation that `serialize` also benefits.
    opt.serialize_into(&mut v, value)?;
    Ok(v)
}

fn serialize(value: &impl serde::Serialize) -> Result<Vec<u8>, bincode::Error> {
    bincode_opts().serialize(value) // this calls serialized_size under the hood to get the vec capacity beforehand
}

fn deserialize<T: serde::de::DeserializeOwned>(bytes: impl AsRef<[u8]>) -> Result<T, bincode::Error> {
    bincode_opts().deserialize(bytes.as_ref())
}

struct RocksDBStorageInner {
    db: DB,
    writeopts_no_wal: WriteOptions,
    config: RocksDBConfig,
}

impl Drop for RocksDBStorageInner {
    fn drop(&mut self) {
        tracing::debug!("⏳ Gracefully closing the database...");
        self.flush().expect("Error when flushing the database"); // flush :)
        self.db.cancel_all_background_work(/* wait */ true);
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

    /// This method also works for partially saved blocks. (that's important for mc-sync, which may create partial blocks past the chain tip.
    /// We also want to remove them!)
    fn remove_all_blocks_starting_from(&self, starting_from_block_n: u64) -> Result<()> {
        // Find the last block. We want to revert blocks in reverse order to make sure we can recover if the node
        // crashes at any point during the call of this function.

        tracing::debug!("Remove blocks starting_from_block_n={starting_from_block_n}");

        let mut last_block_n_exclusive = starting_from_block_n;
        while self.get_block_info(last_block_n_exclusive)?.is_some() {
            last_block_n_exclusive += 1;
        }

        tracing::debug!("Removing blocks range {starting_from_block_n}..{last_block_n_exclusive} in reverse order");

        // Reverse order
        for block_n in (starting_from_block_n..last_block_n_exclusive).rev() {
            let block_info = self.get_block_info(block_n)?.context("Block should be found")?;
            tracing::debug!("Remove block block_n={block_n}");

            let mut batch = WriteBatchWithTransaction::default();
            {
                if let Some(state_diff) = self.get_block_state_diff(block_n)? {
                    // State diff is in db.
                    self.classes_remove(state_diff.all_declared_classes(), &mut batch)?;
                    self.state_remove(block_n, &state_diff, &mut batch)?;
                }

                // This vec is empty if transactions for this block are not yet imported.
                let transactions: Vec<_> = self
                    .get_block_transactions(block_n, /* from_tx_index */ 0)
                    .take(block_info.tx_hashes.len())
                    .collect::<Result<_>>()?;

                self.events_remove_block(block_n, &mut batch)?;
                self.message_to_l2_remove_txns(
                    transactions.iter().filter_map(|v| v.transaction.as_l1_handler()).map(|tx| tx.nonce),
                    &mut batch,
                )?;

                self.blocks_remove_block(&block_info, &mut batch)?;
            }

            self.db
                .write(batch)
                .with_context(|| format!("Committing changes removing block_n={block_n} from database"))?;
        }

        Ok(())
    }
}

/// Implementation of [`MadaraStorageRead`] and [`MadaraStorageWrite`] interface using rocksdb.
#[derive(Debug, Clone)]
pub struct RocksDBStorage {
    inner: Arc<RocksDBStorageInner>,
    backup: BackupManager,
    snapshots: Arc<Snapshots>,
    metrics: DbMetrics,
}

impl RocksDBStorage {
    pub fn open(path: &Path, config: RocksDBConfig) -> Result<Self> {
        let opts = rocksdb_global_options(&config)?;
        tracing::debug!("Opening db at {:?}", path.display());
        let db = DB::open_cf_descriptors(
            &opts,
            path,
            ALL_COLUMNS.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name, col.rocksdb_options(&config))),
        )?;

        let mut writeopts_no_wal = WriteOptions::new();
        writeopts_no_wal.disable_wal(true);
        let inner = Arc::new(RocksDBStorageInner { writeopts_no_wal, db, config: config.clone() });

        let head_block_n = inner.get_chain_tip_without_content()?.and_then(|c| match c {
            StoredChainTipWithoutContent::Confirmed(block_n) => Some(block_n),
            StoredChainTipWithoutContent::Preconfirmed(header) => header.block_number.checked_sub(1),
        });

        let snapshot = Snapshots::new(inner.clone(), head_block_n, config.max_kept_snapshots, config.snapshot_interval);

        Ok(Self {
            inner,
            snapshots: snapshot.into(),
            metrics: DbMetrics::register().context("Registering database metrics")?,
            backup: BackupManager::start_if_enabled(path, &config).context("Startup backup manager")?,
        })
    }
}

impl MadaraStorageRead for RocksDBStorage {
    // Blocks

    fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>> {
        self.inner
            .find_block_hash(block_hash)
            .with_context(|| format!("Finding block number for block_hash={block_hash:#x}"))
    }
    fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<StorageTxIndex>> {
        self.inner
            .find_transaction_hash(tx_hash)
            .with_context(|| format!("Finding transaction index for tx_hash={tx_hash:#x}"))
    }
    fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>> {
        self.inner.get_block_info(block_n).with_context(|| format!("Getting block info for block_n={block_n}"))
    }
    fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>> {
        self.inner
            .get_block_state_diff(block_n)
            .with_context(|| format!("Getting block state diff for block_n={block_n}"))
    }

    fn get_block_bouncer_weights(&self, block_n: u64) -> Result<Option<BouncerWeights>> {
        self.inner
            .get_block_bouncer_weight(block_n)
            .with_context(|| format!("Getting block bouncer weights for block_n={block_n}"))
    }
    fn get_transaction(&self, block_n: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        self.inner
            .get_transaction(block_n, tx_index)
            .with_context(|| format!("Getting block transaction for block_n={block_n} tx_index={tx_index}"))
    }
    fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt>> + '_ {
        self.inner.get_block_transactions(block_n, from_tx_index).map(move |e| {
            e.with_context(|| format!("Getting block transactions for block_n={block_n} from_tx_index={from_tx_index}"))
        })
    }

    // State

    fn get_storage_at(&self, block_n: u64, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>> {
        self.inner.get_storage_at(block_n, contract_address, key).with_context(|| {
            format!("Getting storage value for block_n={block_n} contract_address={contract_address:#x} key={key:#x}")
        })
    }
    fn get_contract_nonce_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.inner
            .get_contract_nonce_at(block_n, contract_address)
            .with_context(|| format!("Getting nonce for block_n={block_n} contract_address={contract_address:#x}"))
    }
    fn get_contract_class_hash_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.inner
            .get_contract_class_hash_at(block_n, contract_address)
            .with_context(|| format!("Getting class_hash for block_n={block_n} contract_address={contract_address:#x}"))
    }
    fn is_contract_deployed_at(&self, block_n: u64, contract_address: &Felt) -> Result<bool> {
        self.inner.is_contract_deployed_at(block_n, contract_address).with_context(|| {
            format!("Checking if contract is deployed for block_n={block_n} contract_address={contract_address:#x}")
        })
    }

    // Classes

    fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>> {
        self.inner.get_class(class_hash).with_context(|| format!("Getting class info for class_hash={class_hash:#x}"))
    }
    fn get_class_compiled(&self, compiled_class_hash: &Felt) -> Result<Option<CompiledSierraWithBlockN>> {
        self.inner
            .get_class_compiled(compiled_class_hash)
            .with_context(|| format!("Getting class compiled for compiled_class_hash={compiled_class_hash:#x}"))
    }

    // Events

    fn get_events(&self, filter: EventFilter) -> Result<Vec<EventWithInfo>> {
        self.inner.get_filtered_events(filter.clone()).with_context(|| format!("Getting events for filter={filter:?}"))
    }

    // Meta

    fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        self.inner.get_devnet_predeployed_keys().context("Getting devnet predeployed contracts keys")
    }
    fn get_chain_tip(&self) -> Result<StorageChainTip> {
        self.inner.get_chain_tip().context("Getting chain tip from db")
    }
    fn get_confirmed_on_l1_tip(&self) -> Result<Option<u64>> {
        self.inner.get_confirmed_on_l1_tip().context("Getting confirmed block on l1 tip")
    }
    fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>> {
        self.inner.get_l1_messaging_sync_tip().context("Getting l1 messaging sync tip")
    }
    fn get_stored_chain_info(&self) -> Result<Option<StoredChainInfo>> {
        self.inner.get_stored_chain_info().context("Getting stored chain info from db")
    }
    fn get_latest_applied_trie_update(&self) -> Result<Option<u64>> {
        self.inner.get_latest_applied_trie_update().context("Getting latest applied trie update info from db")
    }

    // L1 to L2 messages

    fn get_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.inner
            .get_pending_message_to_l2(core_contract_nonce)
            .with_context(|| format!("Getting pending message to l2 with nonce={core_contract_nonce}"))
    }
    fn get_next_pending_message_to_l2(&self, start_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.inner
            .get_next_pending_message_to_l2(start_nonce)
            .with_context(|| format!("Getting next pending message to l2 with start_nonce={start_nonce}"))
    }
    fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        self.inner
            .get_l1_handler_txn_hash_by_nonce(core_contract_nonce)
            .with_context(|| format!("Getting next pending message to l2 with nonce={core_contract_nonce}"))
    }

    // Mempool

    fn get_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_ {
        self.inner.get_mempool_transactions().map(|res| res.context("Getting mempool transactions"))
    }
}

impl MadaraStorageWrite for RocksDBStorage {
    fn write_header(&self, header: mp_block::BlockHeaderWithSignatures) -> Result<()> {
        tracing::debug!("Writing header {}", header.header.block_number);
        let block_n = header.header.block_number;
        self.inner
            .blocks_store_block_header(header)
            .with_context(|| format!("Storing block_header for block_n={block_n}"))
    }

    fn write_transactions(&self, block_n: u64, txs: &[TransactionWithReceipt]) -> Result<()> {
        tracing::debug!("Writing transactions {block_n} {txs:?}");
        // Save l1 core contract nonce to tx mapping.
        self.inner
            .messages_to_l2_write_trasactions(
                txs.iter().filter_map(|v| v.transaction.as_l1_handler().zip(v.receipt.as_l1_handler())),
            )
            .with_context(|| format!("Updating L1 state when storing transactions for block_n={block_n}"))?;

        self.inner
            .blocks_store_transactions(block_n, txs)
            .with_context(|| format!("Storing transactions for block_n={block_n}"))
    }

    fn write_state_diff(&self, block_n: u64, value: &StateDiff) -> Result<()> {
        tracing::debug!("Writing state diff {block_n}");
        self.inner
            .blocks_store_state_diff(block_n, value)
            .with_context(|| format!("Storing state diff for block_n={block_n}"))?;
        self.inner
            .state_apply_state_diff(block_n, value)
            .with_context(|| format!("Applying state from state diff for block_n={block_n}"))
    }

    fn write_bouncer_weights(&self, block_n: u64, value: &BouncerWeights) -> Result<()> {
        tracing::debug!("Writing bouncer weights for block_n={block_n}");
        self.inner
            .blocks_store_bouncer_weights(block_n, value)
            .with_context(|| format!("Storing bouncer weights for block_n={block_n}"))
    }

    fn write_events(&self, block_n: u64, events: &[mp_receipt::EventWithTransactionHash]) -> Result<()> {
        tracing::debug!("Writing events {block_n}");
        self.inner
            .blocks_store_events_to_receipts(block_n, events)
            .with_context(|| format!("Storing events to receipts for block_n={block_n}"))?;
        self.inner
            .store_events_bloom(block_n, events)
            .with_context(|| format!("Storing events bloom filter for block_n={block_n}"))
    }

    fn write_classes(&self, block_n: u64, converted_classes: &[ConvertedClass]) -> Result<()> {
        tracing::debug!("Writing classes {block_n}");
        self.inner.store_classes(block_n, converted_classes)
    }

    fn replace_chain_tip(&self, chain_tip: &StorageChainTip) -> Result<()> {
        tracing::debug!("Replace chain tip {chain_tip:?}");
        self.inner.replace_chain_tip(chain_tip).context("Replacing chain tip in db")
    }

    fn append_preconfirmed_content(&self, start_tx_index: u64, txs: &[PreconfirmedExecutedTransaction]) -> Result<()> {
        tracing::debug!("Append preconfirmed content start_tx_index={start_tx_index}, new_txs={}", txs.len());
        self.inner.append_preconfirmed_content(start_tx_index, txs).context("Appending to preconfirmed content to db")
    }

    fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        tracing::debug!(
            "Write l1 handler tx hash by nonce core_contract_nonce={core_contract_nonce}, txn_hash={txn_hash:#x}"
        );
        self.inner.write_l1_handler_txn_hash_by_nonce(core_contract_nonce, txn_hash).with_context(|| {
            format!("Writing l1 handler txn hash by nonce nonce={core_contract_nonce} txn_hash={txn_hash:#x}")
        })
    }
    fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        tracing::debug!("Write pending message to l2 nonce={}", msg.tx.nonce);
        let nonce = msg.tx.nonce;
        self.inner
            .write_pending_message_to_l2(msg)
            .with_context(|| format!("Writing pending message to l2 nonce={nonce}"))
    }
    fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        tracing::debug!("Remove pending message to l2 nonce={core_contract_nonce}");
        self.inner
            .remove_pending_message_to_l2(core_contract_nonce)
            .with_context(|| format!("Removing pending message to l2 nonce={core_contract_nonce}"))
    }

    fn write_chain_info(&self, info: &StoredChainInfo) -> Result<()> {
        tracing::debug!("Write chain info");
        self.inner.write_chain_info(info)
    }
    fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()> {
        tracing::debug!("Write devnet keys");
        self.inner.write_devnet_predeployed_keys(devnet_keys).context("Writing devnet predeployed keys to db")
    }
    fn write_l1_messaging_sync_tip(&self, block_n: Option<u64>) -> Result<()> {
        tracing::debug!("Write l1 messaging tip block_n={block_n:?}");
        self.inner.write_l1_messaging_sync_tip(block_n).context("Writing l1 messaging sync tip")
    }
    fn write_confirmed_on_l1_tip(&self, block_n: Option<u64>) -> Result<()> {
        tracing::debug!("Write confirmed on l1 tip block_n={block_n:?}");
        self.inner.write_confirmed_on_l1_tip(block_n).context("Writing confirmed on l1 tip")
    }
    fn write_latest_applied_trie_update(&self, block_n: &Option<u64>) -> Result<()> {
        tracing::debug!("Write latest applied trie update block_n={block_n:?}");
        self.inner.write_latest_applied_trie_update(block_n).context("Writing latest applied trie update block_n")
    }

    fn remove_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()> {
        tracing::debug!("Remove mempool transactions");
        self.inner.remove_mempool_transactions(tx_hashes).context("Removing mempool transactions from db")
    }
    fn write_mempool_transaction(&self, tx: &ValidatedTransaction) -> Result<()> {
        let tx_hash = tx.hash;
        tracing::debug!("Writing mempool transaction from db for tx_hash={tx_hash:#x}");
        self.inner
            .write_mempool_transaction(tx)
            .with_context(|| format!("Writing mempool transaction from db for tx_hash={tx_hash:#x}"))
    }

    fn apply_to_global_trie<'a>(
        &self,
        start_block_n: u64,
        state_diffs: impl IntoIterator<Item = &'a StateDiff>,
    ) -> Result<Felt> {
        tracing::debug!("Applying state diff to global trie start_block_n={start_block_n}");
        apply_to_global_trie(self, start_block_n, state_diffs).context("Applying state diff to global trie")
    }

    fn flush(&self) -> Result<()> {
        tracing::debug!("Flushing");
        self.inner.flush().context("Flushing RocksDB database")?;
        self.backup.backup_if_enabled(&self.inner).context("Backing up RocksDB database")
    }

    fn on_new_confirmed_head(&self, block_n: u64) -> Result<()> {
        tracing::debug!("on_new_confirmed_head block_n={block_n}");
        self.snapshots.set_new_head(block_n);
        self.metrics.update(self);
        Ok(())
    }

    fn remove_all_blocks_starting_from(&self, starting_from_block_n: u64) -> Result<()> {
        tracing::debug!("remove_all_blocks_starting_from starting_from_block_n={starting_from_block_n}");
        self.inner
            .remove_all_blocks_starting_from(starting_from_block_n)
            .with_context(|| format!("Removing all blocks in range [{starting_from_block_n}..] from database"))
    }
}
