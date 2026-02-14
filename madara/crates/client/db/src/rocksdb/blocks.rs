use crate::{
    prelude::*,
    rocksdb::{
        events::EVENTS_BLOOM_COLUMN,
        events_bloom_filter::EventBloomWriter,
        iter_pinned::DBIterator,
        l1_to_l2_messages::{L1_TO_L2_PENDING_MESSAGE_BY_NONCE, L1_TO_L2_TXN_HASH_BY_NONCE},
        Column, RocksDBStorageInner, WriteBatchWithTransaction,
    },
    storage::{ConfirmStagedBlockResult, StorageTxIndex},
};
use blockifier::bouncer::BouncerWeights;
use itertools::{Either, Itertools};
use mp_block::{
    commitments::{BlockCommitments, CommitmentComputationContext},
    BlockHeaderWithSignatures, FullBlockWithoutCommitments, MadaraBlockInfo, TransactionWithReceipt,
};
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
};
use rocksdb::{IteratorMode, ReadOptions};
use starknet_types_core::felt::Felt as StarkFelt;
use std::iter;

// TODO (mohit, 14/12/2024): Remove this struct once the v8→v9 migration from
// tmp/0.14.1-with-migration is merged. After that, all databases will have migrated_compiled_classes.
/// Old StateDiff format without migrated_compiled_classes (for v8 database compatibility)
#[derive(serde::Deserialize)]
struct StateDiffV8 {
    storage_diffs: Vec<ContractStorageDiffItem>,
    old_declared_contracts: Vec<StarkFelt>,
    declared_classes: Vec<DeclaredClassItem>,
    deployed_contracts: Vec<DeployedContractItem>,
    replaced_classes: Vec<ReplacedClassItem>,
    nonces: Vec<NonceUpdate>,
}

/// <block_hash 32 bytes> => bincode(block_n)
pub const BLOCK_HASH_TO_BLOCK_N_COLUMN: Column = Column::new("block_hash_to_block_n").set_point_lookup();
/// <tx_hash 32 bytes> => bincode(block_n and tx_index)
pub const TX_HASH_TO_INDEX_COLUMN: Column = Column::new("tx_hash_to_index").set_point_lookup();
/// <block_n 4 bytes> => block_info
pub const BLOCK_INFO_COLUMN: Column = Column::new("block_info").set_point_lookup().use_blocks_mem_budget();
/// <block_n 4 bytes> => bincode(state diff)
pub const BLOCK_STATE_DIFF_COLUMN: Column = Column::new("block_state_diff").set_point_lookup();

/// prefix [<block_n 4 bytes>] | <tx_index 2 bytes> => bincode(tx and receipt)
pub const BLOCK_TRANSACTIONS_COLUMN: Column =
    Column::new("block_transactions").with_prefix_extractor_len(size_of::<u32>()).use_blocks_mem_budget();

/// prefix [<block_n 4 bytes>] => bincode(bouncer_weights)
pub const BLOCK_BOUNCER_WEIGHT_COLUMN: Column =
    Column::new("block_bouncer_weight").with_prefix_extractor_len(size_of::<u32>()).use_blocks_mem_budget();

const TRANSACTIONS_KEY_LEN: usize = size_of::<u32>() + size_of::<u16>();
fn make_transaction_column_key(block_n: u32, tx_index: u16) -> [u8; TRANSACTIONS_KEY_LEN] {
    let mut key = [0u8; TRANSACTIONS_KEY_LEN];
    key[..4].copy_from_slice(&block_n.to_be_bytes());
    key[4..].copy_from_slice(&tx_index.to_be_bytes());
    key
}

fn merge_transactions_with_events(
    transactions: &[TransactionWithReceipt],
    events: &[EventWithTransactionHash],
) -> Result<Vec<TransactionWithReceipt>> {
    let mut out = Vec::with_capacity(transactions.len());
    let mut events = events.iter().peekable();

    for tx in transactions {
        let mut tx = tx.clone();
        let transaction_hash = *tx.receipt.transaction_hash();
        tx.receipt.events_mut().clear();
        tx.receipt.events_mut().extend(
            events
                .peeking_take_while(|event| event.transaction_hash == transaction_hash)
                .map(|event| event.event.clone()),
        );
        out.push(tx);
    }

    ensure!(
        events.next().is_none(),
        "Found event(s) with a transaction hash that is not present in persisted transactions"
    );

    Ok(out)
}

fn events_from_transactions(transactions: &[TransactionWithReceipt]) -> Vec<EventWithTransactionHash> {
    transactions
        .iter()
        .flat_map(|tx| {
            let transaction_hash = *tx.receipt.transaction_hash();
            tx.receipt.events().iter().cloned().map(move |event| EventWithTransactionHash { transaction_hash, event })
        })
        .collect()
}

impl RocksDBStorageInner {
    #[tracing::instrument(skip(self))]
    pub(super) fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>> {
        let Some(res) =
            self.db.get_pinned_cf(&self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN), block_hash.to_bytes_be())?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize::<u32>(&res)?.into()))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<StorageTxIndex>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(TX_HASH_TO_INDEX_COLUMN), tx_hash.to_bytes_be())? else {
            return Ok(None);
        };
        let res = super::deserialize::<(u32, u16)>(&res)?;
        Ok(Some(StorageTxIndex { block_number: res.0.into(), transaction_index: res.1.into() }))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>> {
        let Some(block_n) = u32::try_from(block_n).ok() else { return Ok(None) }; // Every OOB block_n returns not found.
        let Some(res) = self.db.get_pinned_cf(&self.get_column(BLOCK_INFO_COLUMN), block_n.to_be_bytes())? else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    // TODO (mohit, 14/12/2024): Remove the fallback logic once the v8→v9 migration from
    // tmp/0.14.1-with-migration is merged. After that, all databases will have migrated_compiled_classes.
    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>> {
        let Some(block_n) = u32::try_from(block_n).ok() else { return Ok(None) }; // Every OOB block_n returns not found.
        let Some(res) = self.db.get_pinned_cf(&self.get_column(BLOCK_STATE_DIFF_COLUMN), block_n.to_be_bytes())? else {
            return Ok(None);
        };

        // Try deserializing as the new format first (with migrated_compiled_classes)
        if let Ok(state_diff) = super::deserialize::<StateDiff>(&res) {
            return Ok(Some(state_diff));
        }

        // Fallback: try deserializing as the old v8 format (without migrated_compiled_classes)
        let old: StateDiffV8 = super::deserialize(&res)?;
        Ok(Some(StateDiff {
            storage_diffs: old.storage_diffs,
            old_declared_contracts: old.old_declared_contracts,
            declared_classes: old.declared_classes,
            deployed_contracts: old.deployed_contracts,
            replaced_classes: old.replaced_classes,
            nonces: old.nonces,
            migrated_compiled_classes: Vec::new(),
        }))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_bouncer_weight(&self, block_n: u64) -> Result<Option<BouncerWeights>> {
        let Some(block_n) = u32::try_from(block_n).ok() else { return Ok(None) }; // Every OOB block_n returns not found.
        let Some(res) = self.db.get_pinned_cf(&self.get_column(BLOCK_BOUNCER_WEIGHT_COLUMN), block_n.to_be_bytes())?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_transaction(&self, block_n: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        let Some((block_n, tx_index)) = Option::zip(u32::try_from(block_n).ok(), u16::try_from(tx_index).ok()) else {
            return Ok(None); // If it overflows u32, it means it is not found. (we can't store block_n > u32::MAX / tx_index > u16::MAX)
        };
        let Some(res) = self.db.get_pinned_cf(
            &self.get_column(BLOCK_TRANSACTIONS_COLUMN),
            make_transaction_column_key(block_n, tx_index),
        )?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt>> + '_ {
        let Some((block_n, from_tx_index)) =
            Option::zip(u32::try_from(block_n).ok(), u16::try_from(from_tx_index).ok())
        else {
            return Either::Left(iter::empty()); // If it overflows u32, it means it is not found. (we can't store block_n > u32::MAX / tx_index > u16::MAX)
        };
        let from = make_transaction_column_key(block_n, from_tx_index);

        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        let iter = DBIterator::new_cf(
            &self.db,
            &self.get_column(BLOCK_TRANSACTIONS_COLUMN),
            options,
            IteratorMode::From(&from, rocksdb::Direction::Forward),
        )
        .into_iter_values(|bytes| super::deserialize::<TransactionWithReceipt>(bytes))
        .map(|res| Ok(res??));

        Either::Right(iter)
    }

    #[tracing::instrument(skip(self, header))]
    pub(super) fn blocks_store_block_header(&self, header: BlockHeaderWithSignatures) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let block_n = u32::try_from(header.header.block_number).context("Converting block_n to u32")?;

        let block_hash_to_block_n_col = self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN);
        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);

        let info = MadaraBlockInfo {
            header: header.header,
            block_hash: header.block_hash,
            tx_hashes: vec![],
            total_l2_gas_used: 0,
        };

        batch.put_cf(&block_info_col, block_n.to_be_bytes(), super::serialize(&info)?);
        batch.put_cf(
            &block_hash_to_block_n_col,
            header.block_hash.to_bytes_be(),
            &super::serialize_to_smallvec::<[u8; 16]>(&block_n)?,
        );

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, value))]
    pub(super) fn blocks_store_transactions(&self, block_number: u64, value: &[TransactionWithReceipt]) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();

        tracing::debug!(
            "Write {block_number} => {:?}",
            value.iter().map(|v| v.receipt.transaction_hash()).collect::<Vec<_>>()
        );

        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);
        let block_txs_col = self.get_column(BLOCK_TRANSACTIONS_COLUMN);
        let tx_hash_to_index_col = self.get_column(TX_HASH_TO_INDEX_COLUMN);

        let block_n_u32 = u32::try_from(block_number).context("Converting block_n to u32")?;

        for (tx_index, transaction) in value.iter().enumerate() {
            let tx_index_u16 = u16::try_from(tx_index).context("Converting tx_index to u16")?;
            batch.put_cf(
                &tx_hash_to_index_col,
                transaction.receipt.transaction_hash().to_bytes_be(),
                super::serialize_to_smallvec::<[u8; 16]>(&(block_n_u32, tx_index_u16))?,
            );
            batch.put_cf(
                &block_txs_col,
                make_transaction_column_key(block_n_u32, tx_index_u16),
                super::serialize(transaction)?,
            );
        }

        // Update block info tx hashes
        // Also update total_l2_gas_used.
        let mut block_info: MadaraBlockInfo = super::deserialize(
            &self.db.get_pinned_cf(&block_info_col, block_n_u32.to_be_bytes())?.context("Block info not found")?,
        )?;
        block_info.total_l2_gas_used = value.iter().map(|tx| tx.receipt.l2_gas_used()).sum();
        block_info.tx_hashes =
            value.iter().map(|tx_with_receipt| *tx_with_receipt.receipt.transaction_hash()).collect();
        batch.put_cf(&block_info_col, block_n_u32.to_be_bytes(), super::serialize(&block_info)?);

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, value))]
    pub(super) fn blocks_store_state_diff(&self, block_number: u64, value: &StateDiff) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let block_n_u32 = u32::try_from(block_number).context("Converting block_n to u32")?;

        let block_n_to_state_diff = self.get_column(BLOCK_STATE_DIFF_COLUMN);
        batch.put_cf(&block_n_to_state_diff, block_n_u32.to_be_bytes(), &super::serialize(value)?);
        self.db.write_opt(batch, &self.writeopts)?;

        Ok(())
    }

    #[tracing::instrument(skip(self, value))]
    pub(super) fn blocks_store_bouncer_weights(&self, block_number: u64, value: &BouncerWeights) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let block_n_u32 = u32::try_from(block_number).context("Converting block_n to u32")?;

        let block_n_to_bouncer_weights = self.get_column(BLOCK_BOUNCER_WEIGHT_COLUMN);
        batch.put_cf(&block_n_to_bouncer_weights, block_n_u32.to_be_bytes(), &super::serialize(value)?);
        self.db.write_opt(batch, &self.writeopts)?;

        Ok(())
    }

    #[tracing::instrument(skip(self, value))]
    pub(super) fn blocks_store_events_to_receipts(
        &self,
        block_n: u64,
        value: &[mp_receipt::EventWithTransactionHash],
    ) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();

        let mut events = value.iter().peekable();
        let block_txs_col = self.get_column(BLOCK_TRANSACTIONS_COLUMN);

        for (tx_index, transaction) in self.get_block_transactions(block_n, /* from_tx_index */ 0).enumerate() {
            let mut transaction = transaction.with_context(|| format!("Parsing transaction {tx_index}"))?;
            let transaction_hash = *transaction.receipt.transaction_hash();

            transaction.receipt.events_mut().clear();
            transaction.receipt.events_mut().extend(
                events.peeking_take_while(|tx| tx.transaction_hash == transaction_hash).map(|tx| tx.event.clone()),
            );

            let block_n = u32::try_from(block_n).context("Converting block_n to u32")?;
            let tx_index = u16::try_from(tx_index).context("Converting tx_index to u16")?;
            batch.put_cf(
                &block_txs_col,
                make_transaction_column_key(block_n, tx_index),
                super::serialize(&transaction)?,
            );
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, block, converted_classes, bouncer_weights))]
    pub(super) fn write_block_data_without_header(
        &self,
        block: &FullBlockWithoutCommitments,
        converted_classes: &[ConvertedClass],
        bouncer_weights: Option<&BouncerWeights>,
    ) -> Result<()> {
        let block_n = block.header.block_number;
        ensure!(
            self.get_staged_block_header(block_n)?.is_none(),
            "Block #{block_n} is already staged and cannot be written twice"
        );
        ensure!(self.get_block_info(block_n)?.is_none(), "Block #{block_n} is already confirmed and cannot be staged");

        let block_n_u32 = u32::try_from(block_n).context("Converting block_n to u32")?;
        let transactions = merge_transactions_with_events(&block.transactions, &block.events)?;

        let block_transactions_col = self.get_column(BLOCK_TRANSACTIONS_COLUMN);
        let tx_hash_to_index_col = self.get_column(TX_HASH_TO_INDEX_COLUMN);
        let block_state_diff_col = self.get_column(BLOCK_STATE_DIFF_COLUMN);
        let block_bouncer_weight_col = self.get_column(BLOCK_BOUNCER_WEIGHT_COLUMN);
        let events_bloom_col = self.get_column(EVENTS_BLOOM_COLUMN);

        let mut batch = WriteBatchWithTransaction::default();

        for (tx_index, transaction) in transactions.iter().enumerate() {
            let tx_index_u16 = u16::try_from(tx_index).context("Converting tx_index to u16")?;
            batch.put_cf(
                &tx_hash_to_index_col,
                transaction.receipt.transaction_hash().to_bytes_be(),
                super::serialize_to_smallvec::<[u8; 16]>(&(block_n_u32, tx_index_u16))?,
            );
            batch.put_cf(
                &block_transactions_col,
                make_transaction_column_key(block_n_u32, tx_index_u16),
                super::serialize(transaction)?,
            );
        }

        batch.put_cf(&block_state_diff_col, block_n_u32.to_be_bytes(), super::serialize(&block.state_diff)?);

        if let Some(weights) = bouncer_weights {
            batch.put_cf(&block_bouncer_weight_col, block_n_u32.to_be_bytes(), super::serialize(weights)?);
        }

        if !block.events.is_empty() {
            let writer = EventBloomWriter::from_events(block.events.iter().map(|event| &event.event));
            batch.put_cf(&events_bloom_col, block_n_u32.to_be_bytes(), super::serialize(&writer)?);
        }

        self.store_classes_in_batch(block_n, converted_classes, &mut batch)?;
        self.put_staged_block_header_in_batch(block_n, &block.header, &mut batch)?;
        self.db.write_opt(batch, &self.writeopts)?;

        // Apply state and class-index side effects so reads by block number remain consistent with
        // existing partial-import behavior.
        self.state_apply_state_diff(block_n, &block.state_diff)?;
        if !block.state_diff.migrated_compiled_classes.is_empty() {
            let migrations = block
                .state_diff
                .migrated_compiled_classes
                .iter()
                .map(|m| (m.class_hash, m.compiled_class_hash))
                .collect::<Vec<_>>();
            self.update_class_v2_hashes(migrations)?;
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn confirm_staged_block_with_precomputed_root(
        &self,
        block_n: u64,
        precomputed_state_root: Felt,
        chain_id: Felt,
        pre_v0_13_2_hash_override: bool,
    ) -> Result<ConfirmStagedBlockResult> {
        let staged_header = self
            .get_staged_block_header(block_n)?
            .with_context(|| format!("Block #{block_n} is not staged and cannot be confirmed"))?;

        ensure!(
            staged_header.block_number == block_n,
            "Staged header block number mismatch (expected #{block_n}, found #{})",
            staged_header.block_number
        );

        let transactions = self.get_block_transactions(block_n, 0).collect::<Result<Vec<_>>>()?;
        let state_diff = self
            .get_block_state_diff(block_n)?
            .with_context(|| format!("Missing persisted state diff for staged block #{block_n}"))?;
        let events = events_from_transactions(&transactions);

        let parent_block_hash = if block_n == 0 {
            Felt::ZERO
        } else {
            self.get_block_info(block_n - 1)?
                .with_context(|| format!("Parent block #{} not found for staged block #{block_n}", block_n - 1))?
                .block_hash
        };

        let commitments = BlockCommitments::compute(
            &CommitmentComputationContext { protocol_version: staged_header.protocol_version, chain_id },
            &transactions,
            &state_diff,
            &events,
        );
        let header =
            staged_header.into_confirmed_header(parent_block_hash, commitments.clone(), precomputed_state_root);
        let block_hash = header.compute_hash(chain_id, pre_v0_13_2_hash_override);

        let block_n_u32 = u32::try_from(block_n).context("Converting block_n to u32")?;
        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);
        let block_hash_to_block_n_col = self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN);
        let pending_l1_to_l2_col = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        let on_l2_col = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);

        let tx_hashes = transactions.iter().map(|tx| *tx.receipt.transaction_hash()).collect::<Vec<_>>();
        let total_l2_gas_used = transactions.iter().map(|tx| tx.receipt.l2_gas_used()).sum();
        let block_info = MadaraBlockInfo { header, block_hash, tx_hashes, total_l2_gas_used };

        let mut batch = WriteBatchWithTransaction::default();
        batch.put_cf(&block_info_col, block_n_u32.to_be_bytes(), super::serialize(&block_info)?);
        batch.put_cf(
            &block_hash_to_block_n_col,
            block_hash.to_bytes_be(),
            super::serialize_to_smallvec::<[u8; 16]>(&block_n_u32)?,
        );
        for tx in &transactions {
            if let (Some(l1_handler), Some(l1_handler_receipt)) =
                (tx.transaction.as_l1_handler(), tx.receipt.as_l1_handler())
            {
                let nonce_key = l1_handler.nonce.to_be_bytes();
                batch.delete_cf(&pending_l1_to_l2_col, nonce_key);
                batch.put_cf(&on_l2_col, nonce_key, l1_handler_receipt.transaction_hash.to_bytes_be());
            }
        }
        self.set_confirmed_chain_tip_and_clear_staged_marker_in_batch(block_n, block_n, &mut batch)?;

        self.db.write_opt(batch, &self.writeopts)?;

        Ok(ConfirmStagedBlockResult { commitments, block_hash, parent_block_hash })
    }

    #[tracing::instrument(skip(self, batch))]
    pub(super) fn blocks_remove_block(
        &self,
        block_info: &MadaraBlockInfo,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let block_hash_to_block_n_col = self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN);
        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);
        let block_txs_col = self.get_column(BLOCK_TRANSACTIONS_COLUMN);
        let tx_hash_to_index_col = self.get_column(TX_HASH_TO_INDEX_COLUMN);
        let block_n_to_state_diff = self.get_column(BLOCK_STATE_DIFF_COLUMN);

        let block_n_u32 = u32::try_from(block_info.header.block_number).context("Converting block_n to u32")?;

        // Delete transactions
        for (tx_index, tx_hash) in block_info.tx_hashes.iter().enumerate() {
            let tx_index_u16 = u16::try_from(tx_index).context("Converting block_n to u32")?;
            batch.delete_cf(&block_txs_col, make_transaction_column_key(block_n_u32, tx_index_u16));

            // Tx hash to index entry
            batch.delete_cf(&tx_hash_to_index_col, tx_hash.to_bytes_be());
        }

        // Delete header
        batch.delete_cf(&block_info_col, block_n_u32.to_be_bytes());
        // Block hash to block n entry
        batch.delete_cf(&block_hash_to_block_n_col, block_info.block_hash.to_bytes_be());

        // Delete state diff
        batch.delete_cf(&block_n_to_state_diff, block_n_u32.to_be_bytes());

        Ok(())
    }

    /// Reverts the tip of the chain back to the given block.
    ///
    /// In addition, this removes all historical data (chain state, transactions, state diffs,
    /// etc.) from the database for blocks after `revert_to_block_n`.
    ///
    /// Returns a Vec of `(block_number, state_diff)` where the Vec is in reverse order (the first
    /// element is the current tip of the chain and the last are `revert_to_block_n + 1`).
    #[tracing::instrument(skip(self))]
    pub(super) fn block_db_revert(
        &self,
        revert_to_block_n: u64,
        current_tip_block_n: u64,
    ) -> Result<Vec<(u64, StateDiff)>> {
        tracing::info!("📦 REORG [block_db_revert]: Starting, target block_n={}", revert_to_block_n);

        let block_hash_to_block_n_col = self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN);
        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);
        let block_txs_col = self.get_column(BLOCK_TRANSACTIONS_COLUMN);
        let tx_hash_to_index_col = self.get_column(TX_HASH_TO_INDEX_COLUMN);
        let block_n_to_state_diff = self.get_column(BLOCK_STATE_DIFF_COLUMN);

        let latest_block_n = current_tip_block_n;

        tracing::info!(
            "📦 REORG [block_db_revert]: Found latest block_n={}, will remove {} blocks",
            latest_block_n,
            latest_block_n - revert_to_block_n
        );

        let mut state_diffs = Vec::with_capacity((latest_block_n - revert_to_block_n) as usize);

        for block_n in (revert_to_block_n + 1..=latest_block_n).rev() {
            tracing::debug!("📦 REORG [block_db_revert]: Processing block_n={}", block_n);

            let block_n_u32 = u32::try_from(block_n).context("Converting block_n to u32")?;
            let mut batch = WriteBatchWithTransaction::default();

            let block_info =
                self.get_block_info(block_n)?.with_context(|| format!("Block info not found for block_n={block_n}"))?;

            tracing::debug!(
                "📦 REORG [block_db_revert]: Block {} hash={:#x} has {} transactions",
                block_n,
                block_info.block_hash,
                block_info.tx_hashes.len()
            );

            if let Some(state_diff) = self.get_block_state_diff(block_n)? {
                tracing::debug!(
                    "📦 REORG [block_db_revert]: Block {} has state diff with {} deployed contracts, {} storage diffs, {} declared classes",
                    block_n,
                    state_diff.deployed_contracts.len(),
                    state_diff.storage_diffs.len(),
                    state_diff.declared_classes.len()
                );
                state_diffs.push((block_n, state_diff.clone()));
                batch.delete_cf(&block_n_to_state_diff, block_n_u32.to_be_bytes());
            }

            // Get transactions for this block to handle L1 handler removal
            let transactions: Vec<_> =
                self.get_block_transactions(block_n, 0).take(block_info.tx_hashes.len()).collect::<Result<_>>()?;

            // Remove events for this block
            self.events_remove_block(block_n, &mut batch)?;

            let l1_handler_count = transactions.iter().filter(|v| v.transaction.as_l1_handler().is_some()).count();
            if l1_handler_count > 0 {
                tracing::debug!(
                    "📦 REORG [block_db_revert]: Removing {} L1->L2 messages from block {}",
                    l1_handler_count,
                    block_n
                );
            }

            // TODO: No sure how to implement the same for the L2 Network
            // self.message_to_l2_remove_txns(
            //     transactions.iter().filter_map(|v| v.transaction.as_l1_handler()).map(|tx| tx.nonce),
            //     &mut batch,
            // )?;

            tracing::debug!(
                "📦 REORG [block_db_revert]: Removing {} transactions from block {}",
                block_info.tx_hashes.len(),
                block_n
            );
            for (tx_index, tx_hash) in block_info.tx_hashes.iter().enumerate() {
                let tx_index_u16 = u16::try_from(tx_index).context("Converting tx_index to u16")?;
                batch.delete_cf(&block_txs_col, make_transaction_column_key(block_n_u32, tx_index_u16));
                batch.delete_cf(&tx_hash_to_index_col, tx_hash.to_bytes_be());
            }
            batch.delete_cf(&block_info_col, block_n_u32.to_be_bytes());
            batch.delete_cf(&block_hash_to_block_n_col, block_info.block_hash.to_bytes_be());

            self.db.write_opt(batch, &self.writeopts)?;
            tracing::debug!("📦 REORG [block_db_revert]: Block {} successfully removed from database", block_n);
        }

        tracing::info!(
            "✅ REORG [block_db_revert]: Completed, removed {} blocks and collected {} state diffs",
            latest_block_n - revert_to_block_n,
            state_diffs.len()
        );

        Ok(state_diffs)
    }
}
