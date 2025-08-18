//! Madara database
//!
//! # Block storage
//!
//! Storing new blocks is the responsibility of the consumers of this crate. In the madara
//! node architecture, this means: the sync service mc-sync (when we are syncing new blocks), or
//! the block production mc-block-production task when we are producing blocks.
//! For the sake of the mc-db documentation, we will call this service the "block importer" downstream service.
//!
//! The block importer service has two ways of adding blocks to the database, namely:
//! - the easy [`MadaraBackend::add_full_block_with_classes`] function, which takes a full block, and does everything
//!   required to save it properly and increment the latest block number of the database.
//! - or, the more complicated lower level API that allows you to store partial blocks.
//!
//! Note that the validity of the block being stored is not checked for neither of those APIs.
//!
//! For the low-level API, there are a few responsibilities to follow:
//!
//! - The database can store partial blocks. Adding headers can be done using [`MadaraBackend::store_block_header`],
//!   transactions and receipts using [`MadaraBackend::store_transactions`], classes using [`MadaraBackend::class_db_store_block`],
//!   state diffs using [`MadaraBackend::store_state_diff`], events using [`MadaraBackend::store_events`]. Furthermore,
//!   [`MadaraBackend::apply_to_global_trie`] also needs to be called.
//! - Each of those functions can be called in parallel, however, [`MadaraBackend::apply_to_global_trie`] needs to be called
//!   sequentially. This is because we cannot support updating the global trie in an inter-block parallelism fashion. However,
//!   parallelism is still used inside of that function - intra-block parallelism.
//! - Each of these block parts has a [`chain_head::BlockNStatus`] associated inside of [`MadaraBackend::head_status`],
//!   which the block importer service can use however it wants. However, [`ChainHead::full_block`] is special,
//!   as it is updated by this crate.
//! - The block importer service needs to call [`MadaraBackend::on_block`] to mark a block as fully imported. This function
//!   will increment the [`ChainHead::full_block`] field, marking a new block. It will also record some metrics, flush the
//!   database if needed, and make may create db backups if the backend is configured to do so.
//!
//! In addition, readers of the database should use [`db_block_id::DbBlockId`] when querying blocks from the database.
//! This ensures that any partial block data beyond the current [`ChainHead::full_block`] will not be visible to, eg. the rpc
//! service. The block importer service can however bypass this restriction by using [`db_block_id::RawDbBlockId`] instead;
//! allowing it to see the partial data it has saved beyond the latest block marked as full.

use crate::preconfirmed::PreconfirmedBlock;
use crate::preconfirmed::PreconfirmedExecutedTransaction;
use crate::rocksdb::RocksDBConfig;
use crate::rocksdb::RocksDBStorage;
use crate::storage::MadaraStorage;
use crate::storage::MadaraStorageRead;
use crate::storage::StoredChainInfo;
use crate::storage::StoredChainTip;
use crate::sync_status::SyncStatusCell;
use crate::view::Anchor;
use crate::view::PreconfirmedBlockAnchor;
// use db_metrics::DbMetrics;
use itertools::Itertools;
use mp_block::commitments::BlockCommitments;
use mp_block::commitments::CommitmentComputationContext;
use mp_block::header::PreconfirmedHeader;
use mp_block::BlockHeaderWithSignatures;
use mp_block::FullBlock;
use mp_block::PendingFullBlock;
use mp_chain_config::ChainConfig;
use mp_class::ConvertedClass;
use mp_receipt::EventWithTransactionHash;
use mp_transactions::TransactionWithHash;
use prelude::*;
use std::path::Path;

// mod db_metrics;
mod db_version;
mod prelude;
mod rocksdb;
mod storage;
mod subscription;
mod sync_status;

pub mod preconfirmed;
pub mod view;
// pub mod tests;

/// Madara client database backend singleton.
#[derive(Debug)]
pub struct MadaraBackend<DB = RocksDBStorage> {
    db: DB,
    chain_config: Arc<ChainConfig>,
    // db_metrics: DbMetrics,
    config: MadaraBackendConfig,
    sync_status: SyncStatusCell,
    starting_block: Option<u64>,

    /// Current chain tip. Contains the current pre-confirmed block when one is present.
    chain_tip: tokio::sync::watch::Sender<Anchor>,

    /// Current finalized block on L1.
    latest_l1_confirmed: tokio::sync::watch::Sender<Option<u64>>,

    /// Keep the TempDir instance around so that the directory is not deleted until the MadaraBackend struct is dropped.
    #[cfg(any(test, feature = "testing"))]
    _temp_dir: Option<tempfile::TempDir>,
}

#[derive(Debug, Default)]
pub struct MadaraBackendConfig {
    pub flush_every_n_blocks: Option<u64>,
    /// When false, the preconfirmed block is never saved to database.
    pub save_preconfirmed: bool,
}

impl<D: MadaraStorage> MadaraBackend<D> {
    pub fn chain_config(&self) -> &Arc<ChainConfig> {
        &self.chain_config
    }

    fn new_and_init(db: D, chain_config: Arc<ChainConfig>, config: MadaraBackendConfig) -> Result<Self> {
        let mut backend = Self {
            db,
            // db_metrics: DbMetrics::register().context("Registering db metrics")?,
            chain_config,
            config,
            starting_block: None,
            sync_status: SyncStatusCell::default(),
            #[cfg(any(test, feature = "testing"))]
            _temp_dir: None,
            chain_tip: tokio::sync::watch::Sender::new(Default::default()),
            latest_l1_confirmed: tokio::sync::watch::Sender::new(Default::default()),
        };
        backend.init().context("Initializing madara backend")?;
        Ok(backend)
    }

    fn init(&mut self) -> Result<()> {
        // Check chain configuration
        if let Some(res) = self.db.get_stored_chain_info()? {
            if res.chain_id != self.chain_config.chain_id {
                bail!(
                    "The database has been created on the network \"{}\" (chain id `{}`), \
                            but the node is configured for network \"{}\" (chain id `{}`).",
                    res.chain_name,
                    res.chain_id,
                    self.chain_config.chain_name,
                    self.chain_config.chain_id
                )
            }
        } else {
            self.db.write_chain_info(&StoredChainInfo {
                chain_id: self.chain_config.chain_id.clone(),
                chain_name: self.chain_config.chain_name.clone(),
            })?;
        }

        // Init chain_tip and set starting block
        let chain_tip = match self.db.get_chain_tip()? {
            Some(StoredChainTip::BlockN(block_n)) => Anchor::new_on_confirmed(Some(block_n)),
            Some(StoredChainTip::Preconfirmed(header)) => {
                // Load the preconfirmed block from db.
                let preconfirmed_block = Arc::new(PreconfirmedBlock::new(header));
                self.db.get_preconfirmed_content().process_results(|iter| preconfirmed_block.append(iter, []))?;
                Anchor::new_on_preconfirmed(preconfirmed_block)
            }
            // Pre-genesis state.
            None => Anchor::new_on_confirmed(None),
        };
        self.starting_block = chain_tip.block_n();
        self.chain_tip.send_replace(chain_tip);

        // Init L1 head
        self.latest_l1_confirmed.send_replace(self.db.get_confirmed_on_l1_tip()?);

        Ok(())
    }

    // TODO: ensure exclusive access?
    pub fn write_access(self: &Arc<Self>) -> MadaraBackendWriter<D> {
        MadaraBackendWriter { inner: self.clone() }
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    pub fn chain_tip(&self) -> Anchor {
        self.chain_tip.borrow().clone()
    }
    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        self.chain_tip.borrow().latest_confirmed_block_n()
    }
    pub fn latest_block_n(&self) -> Option<u64> {
        self.chain_tip.borrow().block_n()
    }
    pub fn has_preconfirmed_block(&self) -> bool {
        self.chain_tip.borrow().is_preconfirmed()
    }
    pub fn latest_l1_confirmed_block_n(&self) -> Option<u64> {
        self.latest_l1_confirmed.borrow().clone()
    }

    fn preconfirmed_block(&self) -> Option<Arc<PreconfirmedBlock>> {
        self.chain_tip.borrow().preconfirmed().map(|p| p.block.clone())
    }
}

impl MadaraBackend<RocksDBStorage> {
    #[cfg(any(test, feature = "testing"))]
    pub fn open_for_testing(chain_config: Arc<ChainConfig>) -> Arc<Self> {
        let temp_dir = tempfile::TempDir::with_prefix("madara-test").unwrap();
        let db = RocksDBStorage::open(temp_dir.as_ref(), Default::default()).unwrap();
        let mut backend = Self::new_and_init(db, chain_config, Default::default()).unwrap();
        backend._temp_dir = Some(temp_dir);
        Arc::new(backend)
    }

    /// Open the db.
    pub fn open_rocksdb(
        base_path: &Path,
        chain_config: Arc<ChainConfig>,
        config: MadaraBackendConfig,
        rocksdb_config: RocksDBConfig,
    ) -> Result<Arc<Self>> {
        // check if the db version is compatible with the current binary
        tracing::debug!("checking db version");
        if let Some(db_version) = db_version::check_db_version(&base_path).context("Checking database version")? {
            tracing::debug!("version of existing db is {db_version}");
        }
        let db_path = base_path.join("db");
        let db = RocksDBStorage::open(&db_path, rocksdb_config).context("Opening rocksdb storage")?;
        Ok(Arc::new(Self::new_and_init(db, chain_config, config)?))
    }
}

/// Structure holding exclusive access to write the blocks and the tip of the chain.
pub struct MadaraBackendWriter<D: MadaraStorage> {
    inner: Arc<MadaraBackend<D>>,
}

impl<D: MadaraStorage> MadaraBackendWriter<D> {
    fn replace_chain_tip(&self, new_tip: Anchor) -> Result<()> {
        let current_tip = self.inner.chain_tip.borrow().clone();

        // Write to db if needed.
        let saved_tip = if self.inner.config.save_preconfirmed {
            new_tip.clone()
        } else {
            new_tip.latest_confirmed() // We save the anchor to the parent block of the preconfirmed instead.
        };
        if current_tip != saved_tip {
            self.inner.db.replace_chain_tip(&saved_tip)?;
        }

        // Write to the backend. This also sends the notification to subscribers :)
        self.inner.chain_tip.send_replace(new_tip.clone());

        Ok(())
    }

    /// Append transactions to the current preconfirmed block. Returns an error if there is no preconfirmed block.
    /// Replaces all candidate transactions with the content of `replace_candidates`.
    pub fn append_to_preconfirmed(
        &self,
        executed: &[PreconfirmedExecutedTransaction],
        replace_candidates: impl IntoIterator<Item = TransactionWithHash>,
    ) -> Result<()> {
        let block = self.inner.preconfirmed_block().context("There is no current preconfirmed block")?;

        if self.inner.config.save_preconfirmed {
            let start_tx_index = block.content.borrow().n_executed();
            // We don't save candidate transactions.
            self.inner.db.append_preconfirmed_content(start_tx_index as u64, executed)?;
        }

        block.append(executed.iter().cloned(), replace_candidates);

        Ok(())
    }

    /// Does nothing when the backend has no preconfirmed block.
    pub fn clear_preconfirmed(&self) -> Result<()> {
        self.replace_chain_tip(self.inner.chain_tip.borrow().latest_confirmed())
    }

    /// Deletes the current preconfirmed block if present.
    pub fn new_preconfirmed(&self, header: PreconfirmedHeader) -> Result<()> {
        self.replace_chain_tip(Anchor::new_on_preconfirmed(Arc::new(PreconfirmedBlock::new(header))))
    }

    /// Close the current preconfirmed block. Returns an error if there is no preconfirmed block.
    pub fn close_preconfirmed(&self, pre_v0_13_2_hash_override: bool) -> Result<()> {
        let block = self.inner.preconfirmed_block().context("There is no current preconfirmed block")?;

        // Converting the block.

        let header = block.header.clone();
        let block_n = header.block_number;

        // We don't care about the candidate transactions.
        let executed_transactions: Vec<_> = block.content.borrow().executed_transactions().cloned().collect();

        let state_diff = self
            .inner
            .preconfirmed_view_on_anchor(PreconfirmedBlockAnchor::new(block))
            .get_normalized_state_diff()
            .context("Creating normalized state diff")?;
        let transactions: Vec<_> = executed_transactions.into_iter().map(|tx| tx.transaction.clone()).collect();
        let events: Vec<_> = transactions
            .iter()
            .flat_map(|tx| {
                tx.receipt
                    .events()
                    .iter()
                    .cloned()
                    .map(|event| EventWithTransactionHash { transaction_hash: *tx.receipt.transaction_hash(), event })
            })
            .collect();

        let commitments = BlockCommitments::compute(
            &CommitmentComputationContext {
                protocol_version: self.inner.chain_config.latest_protocol_version,
                chain_id: self.inner.chain_config.chain_id.to_felt(),
            },
            &transactions,
            &state_diff,
            &events,
        );

        // Global state root and block hash.

        let global_state_root = self.inner.db.apply_to_global_trie(block_n, [&state_diff])?;

        let header = header.into_confirmed_header(commitments, global_state_root);
        let block_hash = header.compute_hash(self.inner.chain_config.chain_id.to_felt(), pre_v0_13_2_hash_override);

        // Save the block.

        self.inner.db.write_header(BlockHeaderWithSignatures { header, block_hash, consensus_signatures: vec![] })?;
        self.inner.db.write_transactions(block_n, &transactions)?;
        self.inner.db.write_state_diff(block_n, &state_diff)?;
        self.inner.db.write_events(block_n, &events)?;

        // Advance chain tip.

        self.replace_chain_tip(Anchor::new_on_confirmed(Some(block_n)))?;

        Ok(())
    }

    // /// Returns the total storage size
    // pub fn update_metrics(&self) -> u64 {
    //     self.db_metrics.update(&self.db)
    // }

    pub fn add_full_block_with_classes(&self, block: PendingFullBlock, classes: &[ConvertedClass]) -> Result<()> {
        

        Ok(())
    }
}
