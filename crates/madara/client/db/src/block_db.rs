use crate::db_block_id::{DbBlockId, DbBlockIdResolvable};
use crate::{Column, DatabaseExt, MadaraBackend, WriteBatchWithTransaction};
use crate::{MadaraStorageError, DB};
use anyhow::Context;
use blockifier::bouncer::BouncerWeights;
use mp_block::header::{GasPrices, PendingHeader};
use mp_block::{
    BlockId, BlockTag, MadaraBlock, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock,
    MadaraMaybePendingBlockInfo, MadaraPendingBlock, MadaraPendingBlockInfo, VisitedSegments,
};
use mp_rpc::EmittedEvent;
use mp_state_update::StateDiff;
use rocksdb::WriteOptions;
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
const ROW_PENDING_SEGMENTS: &[u8] = b"pending_segments";
const ROW_PENDING_BOUNCER_WEIGHTS: &[u8] = b"pending_bouncer_weights";
const ROW_PENDING_INNER: &[u8] = b"pending";
const ROW_SYNC_TIP: &[u8] = b"sync_tip";
const ROW_L1_LAST_CONFIRMED_BLOCK: &[u8] = b"l1_last";

#[tracing::instrument(skip(db), fields(module = "BlockDB"))]
pub fn get_latest_block_n(db: &DB) -> Result<Option<u64>> {
    let col = db.get_column(Column::BlockStorageMeta);
    let Some(res) = db.get_cf(&col, ROW_SYNC_TIP)? else { return Ok(None) };
    let res = bincode::deserialize(&res)?;
    Ok(Some(res))
}

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

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    fn tx_hash_to_block_n(&self, tx_hash: &Felt) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::TxHashToBlockN);
        let res = self.db.get_cf(&col, bincode::serialize(tx_hash)?)?;
        let Some(res) = res else { return Ok(None) };
        let block_n = bincode::deserialize(&res)?;
        Ok(Some(block_n))
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    fn block_hash_to_block_n(&self, block_hash: &Felt) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockHashToBlockN);
        let res = self.db.get_cf(&col, bincode::serialize(block_hash)?)?;
        let Some(res) = res else { return Ok(None) };
        let block_n = bincode::deserialize(&res)?;
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
        let res = self.db.get_cf(&col, bincode::serialize(&block_n)?)?;
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
        get_latest_block_n(&self.db)
    }

    // Pending block quirk: We should act as if there is always a pending block in db, to match
    //  juno and pathfinder's handling of pending blocks.

    fn get_pending_block_info(&self) -> Result<MadaraPendingBlockInfo> {
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
                        l1_gas_price: GasPrices {
                            eth_l1_gas_price: 1,
                            strk_l1_gas_price: 1,
                            eth_l1_data_gas_price: 1,
                            strk_l1_data_gas_price: 1,
                        },
                        l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
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
                    l1_gas_price: latest_block_info.header.l1_gas_price.clone(),
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
    pub fn get_pending_block_segments(&self) -> Result<Option<VisitedSegments>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_SEGMENTS)? else {
            // See pending block quirk
            return Ok(None);
        };
        let res = Some(bincode::deserialize(&res)?);
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn get_pending_block_bouncer_weights(&self) -> Result<Option<BouncerWeights>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_BOUNCER_WEIGHTS)? else {
            // See pending block quirk
            return Ok(None);
        };
        let res = Some(bincode::deserialize(&res)?);
        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn get_l1_last_confirmed_block(&self) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_L1_LAST_CONFIRMED_BLOCK)? else { return Ok(None) };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    // DB write

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub(crate) fn block_db_store_pending(
        &self,
        block: &MadaraPendingBlock,
        state_update: &StateDiff,
        visited_segments: Option<VisitedSegments>,
        bouncer_weights: Option<BouncerWeights>,
    ) -> Result<()> {
        let mut tx = WriteBatchWithTransaction::default();
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.put_cf(&col, ROW_PENDING_INFO, bincode::serialize(&block.info)?);
        tx.put_cf(&col, ROW_PENDING_INNER, bincode::serialize(&block.inner)?);
        tx.put_cf(&col, ROW_PENDING_STATE_UPDATE, bincode::serialize(&state_update)?);
        if let Some(visited_segments) = visited_segments {
            tx.put_cf(&col, ROW_PENDING_SEGMENTS, bincode::serialize(&visited_segments)?);
        }
        if let Some(bouncer_weights) = bouncer_weights {
            tx.put_cf(&col, ROW_PENDING_BOUNCER_WEIGHTS, bincode::serialize(&bouncer_weights)?);
        }
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);
        self.db.write_opt(tx, &writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub(crate) fn block_db_clear_pending(&self) -> Result<()> {
        let mut tx = WriteBatchWithTransaction::default();
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.delete_cf(&col, ROW_PENDING_INFO);
        tx.delete_cf(&col, ROW_PENDING_INNER);
        tx.delete_cf(&col, ROW_PENDING_STATE_UPDATE);
        tx.delete_cf(&col, ROW_PENDING_SEGMENTS);
        tx.delete_cf(&col, ROW_PENDING_BOUNCER_WEIGHTS);
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);
        self.db.write_opt(tx, &writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn write_last_confirmed_block(&self, l1_last: u64) -> Result<()> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let mut writeopts = WriteOptions::default(); // todo move that in db
        writeopts.disable_wal(true);
        self.db.put_cf_opt(&col, ROW_L1_LAST_CONFIRMED_BLOCK, bincode::serialize(&l1_last)?, &writeopts)?;
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

        tx.put_cf(&block_n_to_block, &block_n_encoded, bincode::serialize(&block.info)?);
        tx.put_cf(&block_hash_to_block_n, block_hash_encoded, &block_n_encoded);
        tx.put_cf(&block_n_to_block_inner, &block_n_encoded, bincode::serialize(&block.inner)?);
        tx.put_cf(&block_n_to_state_diff, &block_n_encoded, bincode::serialize(state_diff)?);
        tx.put_cf(&meta, ROW_SYNC_TIP, block_n_encoded);

        // susbcribers
        if self.sender_block_info.receiver_count() > 0 {
            if let Err(e) = self.sender_block_info.send(block.info.clone()) {
                tracing::debug!("Failed to send block info to subscribers: {e}");
            }
        }
        if self.sender_event.receiver_count() > 0 {
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
                .for_each(|(transaction_hash, event)| {
                    if let Err(e) = self.sender_event.publish(EmittedEvent {
                        event: event.clone().into(),
                        block_hash: Some(block_hash),
                        block_number: Some(block_number),
                        transaction_hash,
                    }) {
                        tracing::debug!("Failed to send event to subscribers: {e}");
                    }
                });
        }

        // clear pending
        tx.delete_cf(&meta, ROW_PENDING_INFO);
        tx.delete_cf(&meta, ROW_PENDING_INNER);
        tx.delete_cf(&meta, ROW_PENDING_STATE_UPDATE);

        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);
        self.db.write_opt(tx, &writeopts)?;
        Ok(())
    }

    // Convenience functions

    pub(crate) fn id_to_storage_type(&self, id: &BlockId) -> Result<Option<DbBlockId>> {
        match id {
            BlockId::Hash(hash) => Ok(self.block_hash_to_block_n(hash)?.map(DbBlockId::Number)),
            BlockId::Number(block_n) => Ok(Some(DbBlockId::Number(*block_n))),
            BlockId::Tag(BlockTag::Latest) => Ok(self.get_latest_block_n()?.map(DbBlockId::Number)),
            BlockId::Tag(BlockTag::Pending) => Ok(Some(DbBlockId::Pending)),
        }
    }

    fn storage_to_info(&self, id: &DbBlockId) -> Result<Option<MadaraMaybePendingBlockInfo>> {
        match id {
            DbBlockId::Pending => Ok(Some(MadaraMaybePendingBlockInfo::Pending(self.get_pending_block_info()?))),
            DbBlockId::Number(block_n) => {
                Ok(self.get_block_info_from_block_n(*block_n)?.map(MadaraMaybePendingBlockInfo::NotPending))
            }
        }
    }

    fn storage_to_inner(&self, id: &DbBlockId) -> Result<Option<MadaraBlockInner>> {
        match id {
            DbBlockId::Pending => Ok(Some(self.get_pending_block_inner()?)),
            DbBlockId::Number(block_n) => self.get_block_inner_from_block_n(*block_n),
        }
    }

    // BlockId

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block_n(&self, id: &impl DbBlockIdResolvable) -> Result<Option<u64>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match &ty {
            DbBlockId::Number(block_id) => Ok(Some(*block_id)),
            DbBlockId::Pending => Ok(None),
        }
    }

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block_hash(&self, id: &impl DbBlockIdResolvable) -> Result<Option<Felt>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match &ty {
            // TODO: fast path if id is already a block hash..
            DbBlockId::Number(block_n) => Ok(self.get_block_info_from_block_n(*block_n)?.map(|b| b.block_hash)),
            DbBlockId::Pending => Ok(None),
        }
    }

    #[tracing::instrument(skip(self, id), fields(module = "BlockDB"))]
    pub fn get_block_state_diff(&self, id: &impl DbBlockIdResolvable) -> Result<Option<StateDiff>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match ty {
            DbBlockId::Pending => Ok(Some(self.get_pending_block_state_update()?)),
            DbBlockId::Number(block_n) => self.get_state_update(block_n),
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

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn subscribe_block_info(&self) -> tokio::sync::broadcast::Receiver<mp_block::MadaraBlockInfo> {
        self.sender_block_info.subscribe()
    }

    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn subscribe_events(&self, from_address: Option<Felt>) -> tokio::sync::broadcast::Receiver<EmittedEvent> {
        self.sender_event.subscribe(from_address)
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
                let info = self.get_pending_block_info()?;
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
                let info = self.get_pending_block_info()?;
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                let inner = self.get_pending_block_inner()?;
                Ok(Some((MadaraMaybePendingBlock { info: info.into(), inner }, TxIndex(tx_index as _))))
            }
        }
    }
}
