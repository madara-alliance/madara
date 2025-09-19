use std::sync::Arc;

use crate::db_block_id::{DbBlockIdResolvable, RawDbBlockId};
use crate::events_bloom_filter::{EventBloomReader, EventBloomSearcher};
use crate::MadaraStorageError;
use crate::{Column, DatabaseExt, MadaraBackend, SyncStatus, WriteBatchWithTransaction};
use anyhow::Context;
use mp_block::event_with_info::{drain_block_events, event_match_filter, EventWithInfo};
use mp_block::header::{GasPrices, PendingHeader};
use mp_block::{
    BlockId, MadaraBlock, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo,
    MadaraPendingBlock, MadaraPendingBlockInfo,
};
use mp_state_update::StateDiff;
use rocksdb::{Direction, IteratorMode, WriteOptions};
use starknet_api::core::ChainId;
use starknet_types_core::felt::Felt;

type Result<T, E = MadaraStorageError> = std::result::Result<T, E>;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct ChainInfo {
    chain_id: ChainId,
    chain_name: String,
}

const ROW_CHAIN_INFO: &[u8] = b"chain_info";
const ROW_PENDING_INFO: &[u8] = b"pending_info";
const ROW_PENDING_STATE_UPDATE: &[u8] = b"pending_state_update";
const ROW_PENDING_INNER: &[u8] = b"pending";
const ROW_L1_LAST_CONFIRMED_BLOCK: &[u8] = b"l1_last";
const ROW_SYNC_TIP: &[u8] = b"sync_tip";

#[derive(Debug, PartialEq, Eq)]
pub struct TxIndex(pub u64);

// TODO(error-handling): some of the else { return Ok(None) } should be replaced with hard errors for
// inconsistent state.
impl MadaraBackend {
    /// This function checks a that the program was started on a db of the wrong chain (ie. main vs
    /// sepolia) and returns an error if it does.
    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub(crate) fn check_configuration(&self) -> anyhow::Result<()> {
        let expected = &self.chain_config;
        let col = self.db.get_column(Column::BlockStorageMeta);
        if let Some(res) = self.db.get_pinned_cf(&col, ROW_CHAIN_INFO)? {
            let res: ChainInfo = bincode::deserialize(res.as_ref())?;

            if res.chain_id != expected.chain_id {
                anyhow::bail!(
                    "The database has been created on the network \"{}\" (chain id `{}`), \
                            but the node is configured for network \"{}\" (chain id `{}`).",
                    res.chain_name,
                    res.chain_id,
                    expected.chain_name,
                    expected.chain_id
                )
            }
        } else {
            let chain_info = ChainInfo { chain_id: expected.chain_id.clone(), chain_name: expected.chain_name.clone() };
            self.db
                .put_cf(&col, ROW_CHAIN_INFO, bincode::serialize(&chain_info)?)
                .context("Writing chain info to db")?;
        }

        Ok(())
    }

    // DB read operations

    // #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    fn tx_hash_to_block_n(&self, tx_hash: &Felt) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::TxHashToBlockN);
        let res = self.db.get_cf(&col, bincode::serialize(tx_hash)?)?;
        let Some(res) = res else { return Ok(None) };
        let block_n = bincode::deserialize(&res)?;
        // If the block_n is partial (past the latest_full_block_n), we not return it.
        if self.head_status.latest_full_block_n().is_none_or(|n| n < block_n) {
            return Ok(None);
        }
        Ok(Some(block_n))
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub(crate) fn block_hash_to_block_n(&self, block_hash: &Felt) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockHashToBlockN);
        let res = self.db.get_cf(&col, bincode::serialize(block_hash)?)?;
        let Some(res) = res else { return Ok(None) };
        let block_n = bincode::deserialize(&res)?;
        // If the block_n is partial (past the latest_full_block_n), we not return it.
        if self.head_status.latest_full_block_n().is_none_or(|n| n < block_n) {
            return Ok(None);
        }
        Ok(Some(block_n))
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    fn get_state_update(&self, block_n: u64) -> Result<Option<StateDiff>> {
        let col = self.db.get_column(Column::BlockNToStateDiff);
        let res = self.db.get_cf(&col, bincode::serialize(&block_n)?)?;
        let Some(res) = res else { return Ok(None) };
        let block = bincode::deserialize(&res)?;
        Ok(Some(block))
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    fn get_block_info_from_block_n(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>> {
        let col = self.db.get_column(Column::BlockNToBlockInfo);
        let res = self.db.get_cf(&col, block_n.to_be_bytes())?;
        let Some(res) = res else { return Ok(None) };
        let block = bincode::deserialize(&res)?;
        Ok(Some(block))
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    fn get_block_inner_from_block_n(&self, block_n: u64) -> Result<Option<MadaraBlockInner>> {
        let col = self.db.get_column(Column::BlockNToBlockInner);
        let res = self.db.get_cf(&col, bincode::serialize(&block_n)?)?;
        let Some(res) = res else { return Ok(None) };
        let block = bincode::deserialize(&res)?;
        Ok(Some(block))
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn get_latest_block_n(&self) -> Result<Option<u64>> {
        Ok(self.head_status().latest_full_block_n())
    }

    // Pending block quirk: We should act as if there is always a pending block in db, to match
    //  juno and pathfinder's handling of pending blocks.

    pub(crate) fn get_pending_block_info_from_db(&self) -> Result<MadaraPendingBlockInfo> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_INFO)? else {
            // See pending block quirk

            let Some(latest_block_id) = self.get_latest_block_n()? else {
                // Second quirk: if there is not even a genesis block in db, make up the gas prices and everything else
                return Ok(MadaraPendingBlockInfo {
                    header: PendingHeader {
                        parent_block_hash: Felt::ZERO,
                        // Sequencer address is ZERO for chains where we don't produce blocks. This means that trying to simulate/trace a transaction on Pending when
                        // genesis has not been loaded yet will return an error. That probably fine because the ERC20 fee contracts are not even deployed yet - it
                        // will error somewhere else anyway.
                        sequencer_address: **self.chain_config().sequencer_address,
                        block_timestamp: Default::default(), // Junk timestamp: unix epoch
                        protocol_version: self.chain_config.latest_protocol_version,
                        gas_prices: GasPrices {
                            eth_l1_gas_price: 1,
                            strk_l1_gas_price: 1,
                            eth_l1_data_gas_price: 1,
                            strk_l1_data_gas_price: 1,
                            eth_l2_gas_price: 1,
                            strk_l2_gas_price: 1,
                        },
                        l1_da_mode: self.chain_config.l1_da_mode,
                    },
                    tx_hashes: vec![],
                });
            };

            let latest_block_info =
                self.get_block_info_from_block_n(latest_block_id)?.ok_or(MadaraStorageError::MissingChainInfo)?;

            return Ok(MadaraPendingBlockInfo {
                header: PendingHeader {
                    parent_block_hash: latest_block_info.block_hash,
                    sequencer_address: latest_block_info.header.sequencer_address,
                    block_timestamp: latest_block_info.header.block_timestamp,
                    protocol_version: latest_block_info.header.protocol_version,
                    gas_prices: latest_block_info.header.gas_prices,
                    l1_da_mode: latest_block_info.header.l1_da_mode,
                },
                tx_hashes: vec![],
            });
        };
        let res = bincode::deserialize(&res)?;
        Ok(res)
    }

    fn get_pending_block_inner(&self) -> Result<MadaraBlockInner> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_INNER)? else {
            // See pending block quirk
            return Ok(MadaraBlockInner::default());
        };
        let res = bincode::deserialize(&res)?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    /// Returns true if the database has a pending block. Note getting a pending block will always succeed, because if there is
    /// no pending block we will return a virtual one which has no in it transaction.
    /// This function returns `false` in the case where getting a pending block will return a virtual one.
    pub fn has_pending_block(&self) -> Result<bool> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        Ok(self.db.get_cf(&col, ROW_PENDING_STATE_UPDATE)?.is_some())
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn get_pending_block_state_update(&self) -> Result<StateDiff> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_STATE_UPDATE)? else {
            // See pending block quirk
            return Ok(StateDiff::default());
        };
        let res = bincode::deserialize(&res)?;
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn get_l1_last_confirmed_block(&self) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_L1_LAST_CONFIRMED_BLOCK)? else { return Ok(None) };
        let res = bincode::deserialize(&res)?;
        tracing::debug!("GET LAST CONFIRMED l1: {res}");
        Ok(Some(res))
    }

    // DB write

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub(crate) fn block_db_store_pending(&self, block: &MadaraPendingBlock, state_update: &StateDiff) -> Result<()> {
        let mut tx = WriteBatchWithTransaction::default();
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.put_cf(&col, ROW_PENDING_INFO, bincode::serialize(&block.info)?);
        tx.put_cf(&col, ROW_PENDING_INNER, bincode::serialize(&block.inner)?);
        tx.put_cf(&col, ROW_PENDING_STATE_UPDATE, bincode::serialize(&state_update)?);
        self.db.write_opt(tx, &self.writeopts_no_wal)?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub(crate) fn block_db_clear_pending(&self) -> Result<()> {
        let mut tx = WriteBatchWithTransaction::default();
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.delete_cf(&col, ROW_PENDING_INFO);
        tx.delete_cf(&col, ROW_PENDING_INNER);
        tx.delete_cf(&col, ROW_PENDING_STATE_UPDATE);
        self.db.write_opt(tx, &self.writeopts_no_wal)?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn write_last_confirmed_block(&self, l1_last: u64) -> Result<()> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        tracing::debug!("WRITE LAST CONFIRMED l1: {l1_last}");
        self.db.put_cf(&col, ROW_L1_LAST_CONFIRMED_BLOCK, bincode::serialize(&l1_last)?)?;
        self.watch_blocks.update_last_block_on_l1(l1_last);
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn clear_last_confirmed_block(&self) -> Result<()> {
        self.write_last_confirmed_block(0)
    }

    /// Also clears pending block
    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub(crate) fn block_db_store_block(&self, block: &MadaraBlock, state_diff: &StateDiff) -> Result<()> {
        let mut tx = WriteBatchWithTransaction::default();

        let tx_hash_to_block_n = self.db.get_column(Column::TxHashToBlockN);
        let block_hash_to_block_n = self.db.get_column(Column::BlockHashToBlockN);
        let block_n_to_block = self.db.get_column(Column::BlockNToBlockInfo);
        let block_n_to_block_inner = self.db.get_column(Column::BlockNToBlockInner);
        let block_n_to_state_diff = self.db.get_column(Column::BlockNToStateDiff);
        let meta = self.db.get_column(Column::BlockStorageMeta);

        let block_hash_encoded = bincode::serialize(&block.info.block_hash)?;
        let block_n_encoded = bincode::serialize(&block.info.header.block_number)?;

        for hash in &block.info.tx_hashes {
            tx.put_cf(&tx_hash_to_block_n, bincode::serialize(hash)?, &block_n_encoded);
        }

        tx.put_cf(&block_n_to_block, block.info.header.block_number.to_be_bytes(), bincode::serialize(&block.info)?);
        tx.put_cf(&block_hash_to_block_n, block_hash_encoded, &block_n_encoded);
        tx.put_cf(&block_n_to_block_inner, &block_n_encoded, bincode::serialize(&block.inner)?);
        tx.put_cf(&block_n_to_state_diff, &block_n_encoded, bincode::serialize(state_diff)?);

        // susbcribers
        self.watch_blocks.on_new_block(block.info.clone().into());

        if self.watch_events.receiver_count() > 0 {
            let block_number = block.info.header.block_number;
            let block_hash = block.info.block_hash;

            block
                .inner
                .receipts
                .iter()
                .flat_map(|receipt| {
                    let tx_hash = receipt.transaction_hash();
                    receipt.events().iter().map(move |event| (tx_hash, event))
                })
                .enumerate()
                .for_each(|(event_index, (transaction_hash, event))| {
                    if let Err(e) = self.watch_events.publish(EventWithInfo {
                        event: event.clone(),
                        block_hash: Some(block_hash),
                        block_number: Some(block_number),
                        transaction_hash,
                        event_index_in_block: event_index,
                    }) {
                        tracing::debug!("Failed to send event to subscribers: {e}");
                    }
                });
        }

        // clear pending
        tx.delete_cf(&meta, ROW_PENDING_INFO);
        tx.delete_cf(&meta, ROW_PENDING_INNER);
        tx.delete_cf(&meta, ROW_PENDING_STATE_UPDATE);

        self.db.write_opt(tx, &self.writeopts_no_wal)?;
        Ok(())
    }


    /// Reverts the tip of the chain back to the given block.
    ///
    /// In addition, this removes all historical data (chain state, transactions, state diffs,
    /// etc.) from the database. `ROW_SYNC_TIP` is set to the new tip.
    ///
    /// Does not clear pending info; caller should do this if needed.
    ///
    /// Returns a Vec of `(block_number, state_diff)` where the Vec is in reverse order (the first
    /// element is the current tip of the chain and the last is `revert_to`).
    pub(crate) fn block_db_revert(&self, revert_to: u64) -> Result<Vec<(u64, StateDiff)>> {
        let mut tx = WriteBatchWithTransaction::default();

        let tx_hash_to_block_n = self.db.get_column(Column::TxHashToBlockN);
        let block_hash_to_block_n = self.db.get_column(Column::BlockHashToBlockN);
        let block_n_to_block = self.db.get_column(Column::BlockNToBlockInfo);
        let block_n_to_block_inner = self.db.get_column(Column::BlockNToBlockInner);
        let block_n_to_state_diff = self.db.get_column(Column::BlockNToStateDiff);
        let meta = self.db.get_column(Column::BlockStorageMeta);

        let latest_block_n = self.get_latest_block_n()?.unwrap(); // TODO: unwrap
        let mut state_diffs = Vec::with_capacity((latest_block_n - revert_to) as usize);
        for block_n in (revert_to + 1..=latest_block_n).rev() {
            let block_n_encoded = bincode::serialize(&block_n)?;

            let res = self.db.get_cf(&block_n_to_block, &block_n_encoded)?;
            let block_info: MadaraBlockInfo = match res {
                Some(data) => bincode::deserialize(&data)?,
                None => {
                    tracing::warn!("Block {} not found during revert, skipping", block_n);
                    continue;
                }
            };

            // clear all txns from this block
            for txn_hash in block_info.tx_hashes {
                let txn_hash_encoded = bincode::serialize(&txn_hash)?;
                tx.delete_cf(&tx_hash_to_block_n, &txn_hash_encoded);
            }

            let block_hash_encoded = bincode::serialize(&block_info.block_hash)?;

            // get state diff for this block before removing it
            let state_diff_serialized = self.db.get_cf(&block_n_to_state_diff, block_n_encoded.clone())?;
            if let Some(state_diff_data) = state_diff_serialized {
                match bincode::deserialize::<StateDiff>(&state_diff_data) {
                    Ok(state_diff) => state_diffs.push((block_n, state_diff)),
                    Err(e) => tracing::warn!("Failed to deserialize state diff for block {}: {}", block_n, e),
                }
            } else {
                tracing::warn!("Block {} has no StateDiff during revert, skipping", block_n);
            }

            tx.delete_cf(&block_n_to_block, &block_n_encoded);
            tx.delete_cf(&block_hash_to_block_n, &block_hash_encoded);
            tx.delete_cf(&block_n_to_block_inner, &block_n_encoded);
            tx.delete_cf(&block_n_to_state_diff, &block_n_encoded);
        }

        let latest_block_n_encoded = bincode::serialize(&revert_to)?;
        tx.put_cf(&meta, ROW_SYNC_TIP, latest_block_n_encoded);

        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);
        self.db.write_opt(tx, &writeopts)?;

        Ok(state_diffs)
    }

    // Convenience functions

    fn storage_to_info(&self, id: &RawDbBlockId) -> Result<Option<MadaraMaybePendingBlockInfo>> {
        match id {
            RawDbBlockId::Pending => {
                Ok(Some(MadaraMaybePendingBlockInfo::Pending(Arc::unwrap_or_clone(self.latest_pending_block()))))
            }
            RawDbBlockId::Number(block_n) => {
                Ok(self.get_block_info_from_block_n(*block_n)?.map(MadaraMaybePendingBlockInfo::NotPending))
            }
        }
    }

    fn storage_to_inner(&self, id: &RawDbBlockId) -> Result<Option<MadaraBlockInner>> {
        match id {
            RawDbBlockId::Pending => Ok(Some(self.get_pending_block_inner()?)),
            RawDbBlockId::Number(block_n) => self.get_block_inner_from_block_n(*block_n),
        }
    }

    // BlockId

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block_n(&self, id: &impl DbBlockIdResolvable) -> Result<Option<u64>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match &ty {
            RawDbBlockId::Number(block_id) => Ok(Some(*block_id)),
            RawDbBlockId::Pending => Ok(None),
        }
    }

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block_hash(&self, id: &impl DbBlockIdResolvable) -> Result<Option<Felt>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match &ty {
            // TODO: fast path if id is already a block hash..
            RawDbBlockId::Number(block_n) => Ok(self.get_block_info_from_block_n(*block_n)?.map(|b| b.block_hash)),
            RawDbBlockId::Pending => Ok(None),
        }
    }

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block_state_diff(&self, id: &impl DbBlockIdResolvable) -> Result<Option<StateDiff>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match ty {
            RawDbBlockId::Pending => Ok(Some(self.get_pending_block_state_update()?)),
            RawDbBlockId::Number(block_n) => self.get_state_update(block_n),
        }
    }

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn contains_block(&self, id: &impl DbBlockIdResolvable) -> Result<bool> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(false) };
        // todo: optimize this
        Ok(self.storage_to_info(&ty)?.is_some())
    }

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block_info(&self, id: &impl DbBlockIdResolvable) -> Result<Option<MadaraMaybePendingBlockInfo>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        self.storage_to_info(&ty)
    }

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block_inner(&self, id: &impl DbBlockIdResolvable) -> Result<Option<MadaraBlockInner>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        self.storage_to_inner(&ty)
    }

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block(&self, id: &impl DbBlockIdResolvable) -> Result<Option<MadaraMaybePendingBlock>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        let Some(info) = self.storage_to_info(&ty)? else { return Ok(None) };
        let Some(inner) = self.storage_to_inner(&ty)? else { return Ok(None) };
        Ok(Some(MadaraMaybePendingBlock { info, inner }))
    }

    // Tx hashes and tx status

    /// Returns the index of the tx.
    #[tracing::instrument(skip(self, tx_hash), fields(module = "BlockDB"))]
    pub fn find_tx_hash_block_info(&self, tx_hash: &Felt) -> Result<Option<(MadaraMaybePendingBlockInfo, TxIndex)>> {
        match self.tx_hash_to_block_n(tx_hash)? {
            Some(block_n) => {
                let Some(info) = self.get_block_info_from_block_n(block_n)? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                Ok(Some((info.into(), TxIndex(tx_index as _))))
            }
            None => {
                let info = Arc::unwrap_or_clone(self.latest_pending_block());
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                Ok(Some((info.into(), TxIndex(tx_index as _))))
            }
        }
    }

    /// Returns the index of the tx.
    #[tracing::instrument(skip(self, tx_hash), fields(module = "BlockDB"))]
    pub fn find_tx_hash_block(&self, tx_hash: &Felt) -> Result<Option<(MadaraMaybePendingBlock, TxIndex)>> {
        match self.tx_hash_to_block_n(tx_hash)? {
            Some(block_n) => {
                let Some(info) = self.get_block_info_from_block_n(block_n)? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                let Some(inner) = self.get_block_inner_from_block_n(block_n)? else { return Ok(None) };
                Ok(Some((MadaraMaybePendingBlock { info: info.into(), inner }, TxIndex(tx_index as _))))
            }
            None => {
                let info = Arc::unwrap_or_clone(self.latest_pending_block());
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                let inner = self.get_pending_block_inner()?;
                Ok(Some((MadaraMaybePendingBlock { info: info.into(), inner }, TxIndex(tx_index as _))))
            }
        }
    }

    /// Retrieves an iterator over event bloom filters starting from the specified block.
    ///
    /// This method returns an iterator that yields (block_number, bloom_filter) pairs,
    /// allowing for efficient filtering of potential blocks containing matching events.
    /// Only blocks containing events will have bloom filters, which is why we return
    /// the block number with each filter - this allows us to identify gaps in the sequence
    /// where blocks had no events.
    ///
    /// Note: The caller should consume this iterator quickly to avoid pinning RocksDB
    /// resources for an extended period.
    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    fn get_event_filter_stream(
        &self,
        block_n: u64,
    ) -> Result<impl Iterator<Item = Result<(u64, EventBloomReader)>> + '_> {
        let col = self.db.get_column(Column::EventBloom);
        let key = bincode::serialize(&block_n)?;
        let iter_mode = IteratorMode::From(&key, Direction::Forward);
        let iter = self.db.iterator_cf(&col, iter_mode);

        Ok(iter.map(|kvs| {
            kvs.map_err(MadaraStorageError::from).and_then(|(key, value)| {
                let stored_block_n: u64 = bincode::deserialize(&key).map_err(MadaraStorageError::from)?;
                let bloom = bincode::deserialize(&value).map_err(MadaraStorageError::from)?;
                Ok((stored_block_n, bloom))
            })
        }))
    }

    /// Retrieves events that match the specified filter criteria within a block range.
    ///
    /// This implementation uses a two-phase filtering approach:
    /// 1. First use bloom filters to quickly identify blocks that *might* contain matching events
    /// 2. Then retrieve and process only those candidate blocks
    ///
    /// The method processes blocks incrementally to avoid keeping RocksDB iterators open for too long.
    ///
    /// ### Returns
    /// - A vector of events that match the filter criteria, up to `max_events` in size.
    /// - The returned events are collected across multiple blocks within the specified range.
    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn get_filtered_events(
        &self,
        start_block: u64,
        start_event_index: usize,
        end_block: u64,
        from_address: Option<&Felt>,
        keys_pattern: Option<&[Vec<Felt>]>,
        max_events: usize,
    ) -> Result<Vec<EventWithInfo>> {
        let key_filter = EventBloomSearcher::new(from_address, keys_pattern);

        let mut events_infos = Vec::new();

        let mut current_block = start_block;

        'event_block_research: while current_block <= end_block && events_infos.len() < max_events {
            'bloom_research: {
                // Scope the filter stream iterator to ensure it's dropped promptly
                let filter_event_stream = self.get_event_filter_stream(current_block)?;

                for filter_block in filter_event_stream {
                    let (block_n, bloom_filter) = filter_block?;

                    // Stop if we've gone beyond the requested range
                    if block_n > end_block {
                        break 'event_block_research;
                    }

                    // Use the bloom filter to quickly check if the block might contain relevant events.
                    // - This avoids unnecessary block retrieval if no matching events exist.
                    if key_filter.search(&bloom_filter) {
                        current_block = block_n;
                        break 'bloom_research;
                    }
                }
                // If no bloom filter was found, there's no more blocks whith events to process in DB.
                break 'event_block_research;
            } // RocksDB iterator is dropped here

            // Retrieve the full block data since we now suspect it contains relevant events.
            let block =
                self.get_block(&BlockId::Number(current_block))?.ok_or(MadaraStorageError::InconsistentStorage(
                    format!("Bloom filter found but block not found for block {current_block}").into(),
                ))?;

            // Determine starting event index based on whether we're continuing from a previous query
            let skip_events = if current_block == start_block { start_event_index } else { 0 };

            // Extract matching events from the block
            let mut iter = drain_block_events(block)
                .enumerate()
                .skip(skip_events)
                .filter(|(_, event)| event_match_filter(&event.event, from_address, keys_pattern));

            // Take exactly enough events to fill the requested chunk size.
            events_infos.extend(iter.by_ref().take(max_events - events_infos.len()).map(|(_, event)| event));

            current_block = current_block
                .checked_add(1)
                .ok_or(MadaraStorageError::InconsistentStorage("Block number overflow".into()))?;
        }

        Ok(events_infos)
    }

    pub fn get_starting_block(&self) -> Option<u64> {
        self.starting_block
    }

    pub fn set_starting_block(&mut self, starting_block: Option<u64>) {
        self.starting_block = starting_block;
    }

    pub async fn get_sync_status(&self) -> SyncStatus {
        self.sync_status.get().await
    }

    pub async fn set_sync_status(&self, sync_status: SyncStatus) {
        self.sync_status.set(sync_status).await;
    }
}
