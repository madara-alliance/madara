use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner, WriteBatchWithTransaction},
    storage::StorageTxIndex,
};
use blockifier::bouncer::BouncerWeights;
use itertools::{Either, Itertools};
use mp_block::{BlockHeaderWithSignatures, MadaraBlockInfo, TransactionWithReceipt};
use mp_convert::Felt;
use mp_receipt::TransactionReceipt;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
};
use mp_transactions::{
    DataAvailabilityMode, DeclareTransaction, DeployAccountTransaction, DeployTransaction, InvokeTransaction,
    InvokeTransactionV0, InvokeTransactionV1, L1HandlerTransaction, ResourceBoundsMapping, Transaction,
};
use rocksdb::{IteratorMode, ReadOptions};
use starknet_types_core::felt::Felt as StarkFelt;
use std::{iter, sync::Arc};

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

#[derive(serde::Deserialize, serde::Serialize)]
struct LegacyTransactionWithReceipt {
    transaction: LegacyTransaction,
    receipt: TransactionReceipt,
}

#[derive(serde::Deserialize, serde::Serialize)]
enum LegacyTransaction {
    Invoke(LegacyInvokeTransaction),
    L1Handler(L1HandlerTransaction),
    Declare(DeclareTransaction),
    Deploy(DeployTransaction),
    DeployAccount(DeployAccountTransaction),
}

#[derive(serde::Deserialize, serde::Serialize)]
enum LegacyInvokeTransaction {
    V0(InvokeTransactionV0),
    V1(InvokeTransactionV1),
    V3(LegacyInvokeTransactionV3),
}

#[derive(Debug, Clone, Hash, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
struct LegacyInvokeTransactionV3 {
    sender_address: Felt,
    calldata: Arc<Vec<Felt>>,
    signature: Arc<Vec<Felt>>,
    nonce: Felt,
    resource_bounds: ResourceBoundsMapping,
    tip: u64,
    paymaster_data: Vec<Felt>,
    account_deployment_data: Vec<Felt>,
    nonce_data_availability_mode: DataAvailabilityMode,
    fee_data_availability_mode: DataAvailabilityMode,
}

impl From<LegacyTransactionWithReceipt> for TransactionWithReceipt {
    fn from(value: LegacyTransactionWithReceipt) -> Self {
        Self { transaction: value.transaction.into(), receipt: value.receipt }
    }
}

impl From<LegacyTransaction> for Transaction {
    fn from(value: LegacyTransaction) -> Self {
        match value {
            LegacyTransaction::Invoke(tx) => Self::Invoke(tx.into()),
            LegacyTransaction::L1Handler(tx) => Self::L1Handler(tx),
            LegacyTransaction::Declare(tx) => Self::Declare(tx),
            LegacyTransaction::Deploy(tx) => Self::Deploy(tx),
            LegacyTransaction::DeployAccount(tx) => Self::DeployAccount(tx),
        }
    }
}

impl From<LegacyInvokeTransaction> for InvokeTransaction {
    fn from(value: LegacyInvokeTransaction) -> Self {
        match value {
            LegacyInvokeTransaction::V0(tx) => Self::V0(tx),
            LegacyInvokeTransaction::V1(tx) => Self::V1(tx),
            LegacyInvokeTransaction::V3(tx) => Self::V3(tx.into()),
        }
    }
}

impl From<LegacyInvokeTransactionV3> for mp_transactions::InvokeTransactionV3 {
    fn from(value: LegacyInvokeTransactionV3) -> Self {
        Self {
            sender_address: value.sender_address,
            calldata: value.calldata,
            signature: value.signature,
            nonce: value.nonce,
            resource_bounds: value.resource_bounds,
            tip: value.tip,
            paymaster_data: value.paymaster_data,
            account_deployment_data: value.account_deployment_data,
            nonce_data_availability_mode: value.nonce_data_availability_mode,
            fee_data_availability_mode: value.fee_data_availability_mode,
            proof_facts: None,
        }
    }
}

fn deserialize_transaction_with_receipt(bytes: &[u8]) -> Result<TransactionWithReceipt> {
    match super::deserialize::<TransactionWithReceipt>(bytes) {
        Ok(transaction) => Ok(transaction),
        Err(current_err) => {
            let legacy: LegacyTransactionWithReceipt = super::deserialize(bytes).map_err(|legacy_err| {
                anyhow::anyhow!(
                    "deserializing TransactionWithReceipt failed with current schema ({current_err}) and legacy invoke-v3 fallback ({legacy_err})"
                )
            })?;
            Ok(legacy.into())
        }
    }
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

impl RocksDBStorageInner {
    /// Collect all L1 handler nonces for blocks in `(revert_to_block_n, current_tip_block_n]`.
    #[tracing::instrument(skip(self))]
    pub(super) fn collect_reverted_l1_handler_nonces(
        &self,
        revert_to_block_n: u64,
        current_tip_block_n: u64,
    ) -> Result<Vec<u64>> {
        let mut nonces = Vec::new();

        for block_n in (revert_to_block_n + 1..=current_tip_block_n).rev() {
            let block_info =
                self.get_block_info(block_n)?.with_context(|| format!("Block info not found for block_n={block_n}"))?;
            let transactions: Vec<_> =
                self.get_block_transactions(block_n, 0).take(block_info.tx_hashes.len()).collect::<Result<_>>()?;
            nonces.extend(transactions.iter().filter_map(|v| v.transaction.as_l1_handler().map(|tx| tx.nonce)));
        }

        Ok(nonces)
    }

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
        Ok(Some(deserialize_transaction_with_receipt(&res)?))
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
        .into_iter_values(deserialize_transaction_with_receipt)
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

            // Remove events for this block
            self.events_remove_block(block_n, &mut batch)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use mp_receipt::{ExecutionResources, ExecutionResult, FeePayment, GasVector, InvokeTransactionReceipt, PriceUnit};

    #[test]
    fn deserialize_legacy_invoke_v3_transaction_without_proof_facts() {
        let legacy = LegacyTransactionWithReceipt {
            transaction: LegacyTransaction::Invoke(LegacyInvokeTransaction::V3(LegacyInvokeTransactionV3 {
                sender_address: Felt::from_hex_unchecked("0x123"),
                calldata: Arc::new(vec![Felt::from_hex_unchecked("0x1"), Felt::from_hex_unchecked("0x2")]),
                signature: Arc::new(vec![Felt::from_hex_unchecked("0x3")]),
                nonce: Felt::from_hex_unchecked("0x4"),
                resource_bounds: ResourceBoundsMapping {
                    l1_gas: mp_transactions::ResourceBounds { max_amount: 5, max_price_per_unit: 6 },
                    l2_gas: mp_transactions::ResourceBounds { max_amount: 7, max_price_per_unit: 8 },
                    l1_data_gas: Some(mp_transactions::ResourceBounds { max_amount: 9, max_price_per_unit: 10 }),
                },
                tip: 11,
                paymaster_data: vec![Felt::from_hex_unchecked("0xc")],
                account_deployment_data: vec![Felt::from_hex_unchecked("0xd")],
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                fee_data_availability_mode: DataAvailabilityMode::L2,
            })),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: Felt::from_hex_unchecked("0xbeef"),
                actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x1234"), unit: PriceUnit::Fri },
                messages_sent: vec![],
                events: vec![],
                execution_resources: ExecutionResources {
                    steps: 1,
                    memory_holes: 2,
                    range_check_builtin_applications: 3,
                    pedersen_builtin_applications: 4,
                    poseidon_builtin_applications: 5,
                    ec_op_builtin_applications: 6,
                    ecdsa_builtin_applications: 7,
                    bitwise_builtin_applications: 8,
                    keccak_builtin_applications: 9,
                    segment_arena_builtin: 10,
                    data_availability: GasVector { l1_gas: 11, l1_data_gas: 12, l2_gas: 13 },
                    total_gas_consumed: GasVector { l1_gas: 14, l1_data_gas: 15, l2_gas: 16 },
                },
                execution_result: ExecutionResult::Succeeded,
            }),
        };

        let encoded = super::super::serialize(&legacy).expect("legacy tx should serialize");
        let decoded = deserialize_transaction_with_receipt(&encoded).expect("legacy tx should deserialize");

        match decoded.transaction {
            Transaction::Invoke(InvokeTransaction::V3(tx)) => {
                assert_eq!(tx.sender_address, Felt::from_hex_unchecked("0x123"));
                assert_eq!(tx.calldata.as_ref(), &vec![Felt::from_hex_unchecked("0x1"), Felt::from_hex_unchecked("0x2")]);
                assert_eq!(tx.signature.as_ref(), &vec![Felt::from_hex_unchecked("0x3")]);
                assert_eq!(tx.proof_facts, None);
            }
            other => panic!("expected invoke v3 transaction, got {other:?}"),
        }

        assert_eq!(*decoded.receipt.transaction_hash(), Felt::from_hex_unchecked("0xbeef"));
        assert_eq!(decoded.receipt.actual_fee().unit, PriceUnit::Fri);
    }
}
