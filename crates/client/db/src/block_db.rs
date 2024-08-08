use anyhow::Context;
use dp_block::{
    BlockId, BlockTag, DeoxysBlock, DeoxysBlockInfo, DeoxysBlockInner, DeoxysMaybePendingBlock,
    DeoxysMaybePendingBlockInfo, DeoxysPendingBlock, DeoxysPendingBlockInfo,
};
use dp_state_update::StateDiff;
use rocksdb::WriteOptions;
use starknet_api::core::ChainId;
use starknet_types_core::felt::Felt;

use crate::db_block_id::{DbBlockId, DbBlockIdResolvable};
use crate::DeoxysStorageError;
use crate::{Column, DatabaseExt, DeoxysBackend, WriteBatchWithTransaction};

type Result<T, E = DeoxysStorageError> = std::result::Result<T, E>;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct ChainInfo {
    chain_id: ChainId,
    chain_name: String,
}

const ROW_CHAIN_INFO: &[u8] = b"chain_info";
const ROW_PENDING_INFO: &[u8] = b"pending_info";
const ROW_PENDING_STATE_UPDATE: &[u8] = b"pending_state_update";
const ROW_PENDING_INNER: &[u8] = b"pending";
const ROW_SYNC_TIP: &[u8] = b"sync_tip";
const ROW_L1_LAST_CONFIRMED_BLOCK: &[u8] = b"l1_last";

#[derive(Debug, PartialEq, Eq)]
pub struct TxIndex(pub u64);

// TODO(error-handling): some of the else { return Ok(None) } should be replaced with hard errors for
// inconsistent state.
impl DeoxysBackend {
    /// This function checks a that the program was started on a db of the wrong chain (ie. main vs
    /// sepolia) and returns an error if it does.
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

    fn tx_hash_to_block_n(&self, tx_hash: &Felt) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::TxHashToBlockN);
        let res = self.db.get_cf(&col, bincode::serialize(tx_hash)?)?;
        let Some(res) = res else { return Ok(None) };
        let block_n = bincode::deserialize(&res)?;
        Ok(Some(block_n))
    }

    fn block_hash_to_block_n(&self, block_hash: &Felt) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockHashToBlockN);
        let res = self.db.get_cf(&col, bincode::serialize(block_hash)?)?;
        let Some(res) = res else { return Ok(None) };
        let block_n = bincode::deserialize(&res)?;
        Ok(Some(block_n))
    }

    fn get_state_update(&self, block_n: u64) -> Result<Option<StateDiff>> {
        let col = self.db.get_column(Column::BlockNToStateDiff);
        let res = self.db.get_cf(&col, bincode::serialize(&block_n)?)?;
        let Some(res) = res else { return Ok(None) };
        let block = bincode::deserialize(&res)?;
        Ok(Some(block))
    }

    fn get_block_info_from_block_n(&self, block_n: u64) -> Result<Option<DeoxysBlockInfo>> {
        let col = self.db.get_column(Column::BlockNToBlockInfo);
        let res = self.db.get_cf(&col, bincode::serialize(&block_n)?)?;
        let Some(res) = res else { return Ok(None) };
        let block = bincode::deserialize(&res)?;
        Ok(Some(block))
    }

    fn get_block_inner_from_block_n(&self, block_n: u64) -> Result<Option<DeoxysBlockInner>> {
        let col = self.db.get_column(Column::BlockNToBlockInner);
        let res = self.db.get_cf(&col, bincode::serialize(&block_n)?)?;
        let Some(res) = res else { return Ok(None) };
        let block = bincode::deserialize(&res)?;
        Ok(Some(block))
    }

    pub fn get_latest_block_n(&self) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_SYNC_TIP)? else { return Ok(None) };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    fn get_pending_block_info(&self) -> Result<Option<DeoxysPendingBlockInfo>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_INFO)? else { return Ok(None) };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    fn get_pending_block_inner(&self) -> Result<Option<DeoxysBlockInner>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_INNER)? else { return Ok(None) };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    pub fn get_l1_last_confirmed_block(&self) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_L1_LAST_CONFIRMED_BLOCK)? else { return Ok(None) };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    pub fn get_pending_block_state_update(&self) -> Result<Option<StateDiff>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_STATE_UPDATE)? else { return Ok(None) };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    // DB write

    pub(crate) fn block_db_store_pending(&self, block: &DeoxysPendingBlock, state_update: &StateDiff) -> Result<()> {
        let mut tx = WriteBatchWithTransaction::default();
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.put_cf(&col, ROW_PENDING_INFO, bincode::serialize(&block.info)?);
        tx.put_cf(&col, ROW_PENDING_INNER, bincode::serialize(&block.inner)?);
        tx.put_cf(&col, ROW_PENDING_STATE_UPDATE, bincode::serialize(&state_update)?);
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);
        self.db.write_opt(tx, &writeopts)?;
        Ok(())
    }

    pub(crate) fn block_db_clear_pending(&self) -> Result<()> {
        let mut tx = WriteBatchWithTransaction::default();
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.delete_cf(&col, ROW_PENDING_INFO);
        tx.delete_cf(&col, ROW_PENDING_INNER);
        tx.delete_cf(&col, ROW_PENDING_STATE_UPDATE);
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);
        self.db.write_opt(tx, &writeopts)?;
        Ok(())
    }

    pub fn write_last_confirmed_block(&self, l1_last: u64) -> Result<()> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let mut writeopts = WriteOptions::default(); // todo move that in db
        writeopts.disable_wal(true);
        self.db.put_cf_opt(&col, ROW_L1_LAST_CONFIRMED_BLOCK, bincode::serialize(&l1_last)?, &writeopts)?;
        Ok(())
    }

    pub fn clear_last_confirmed_block(&self) -> Result<()> {
        self.write_last_confirmed_block(0)
    }

    /// Also clears pending block
    pub(crate) fn block_db_store_block(&self, block: &DeoxysBlock, state_diff: &StateDiff) -> Result<()> {
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

        tx.put_cf(&block_hash_to_block_n, block_hash_encoded, &block_n_encoded);
        tx.put_cf(&block_n_to_block, &block_n_encoded, bincode::serialize(&block.info)?);
        tx.put_cf(&block_n_to_block_inner, &block_n_encoded, bincode::serialize(&block.inner)?);
        tx.put_cf(&block_n_to_state_diff, &block_n_encoded, bincode::serialize(state_diff)?);
        tx.put_cf(&meta, ROW_SYNC_TIP, block_n_encoded);

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
            BlockId::Hash(hash) => Ok(self.block_hash_to_block_n(hash)?.map(DbBlockId::BlockN)),
            BlockId::Number(block_n) => Ok(Some(DbBlockId::BlockN(*block_n))),
            BlockId::Tag(BlockTag::Latest) => Ok(self.get_latest_block_n()?.map(DbBlockId::BlockN)),
            BlockId::Tag(BlockTag::Pending) => Ok(Some(DbBlockId::Pending)),
        }
    }

    fn storage_to_info(&self, id: &DbBlockId) -> Result<Option<DeoxysMaybePendingBlockInfo>> {
        match id {
            DbBlockId::Pending => Ok(self.get_pending_block_info()?.map(DeoxysMaybePendingBlockInfo::Pending)),
            DbBlockId::BlockN(block_n) => {
                Ok(self.get_block_info_from_block_n(*block_n)?.map(DeoxysMaybePendingBlockInfo::NotPending))
            }
        }
    }

    fn storage_to_inner(&self, id: &DbBlockId) -> Result<Option<DeoxysBlockInner>> {
        match id {
            DbBlockId::Pending => self.get_pending_block_inner(),
            DbBlockId::BlockN(block_n) => self.get_block_inner_from_block_n(*block_n),
        }
    }

    // BlockId

    pub fn get_block_n(&self, id: &impl DbBlockIdResolvable) -> Result<Option<u64>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match &ty {
            DbBlockId::BlockN(block_id) => Ok(Some(*block_id)),
            DbBlockId::Pending => Ok(None),
        }
    }

    pub fn get_block_hash(&self, id: &impl DbBlockIdResolvable) -> Result<Option<Felt>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match &ty {
            // TODO: fast path if id is already a block hash..
            DbBlockId::BlockN(block_n) => Ok(self.get_block_info_from_block_n(*block_n)?.map(|b| b.block_hash)),
            DbBlockId::Pending => Ok(None),
        }
    }

    pub fn get_block_state_diff(&self, id: &impl DbBlockIdResolvable) -> Result<Option<StateDiff>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        match ty {
            DbBlockId::Pending => self.get_pending_block_state_update(),
            DbBlockId::BlockN(block_n) => self.get_state_update(block_n),
        }
    }

    pub fn get_block_info(&self, id: &impl DbBlockIdResolvable) -> Result<Option<DeoxysMaybePendingBlockInfo>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        self.storage_to_info(&ty)
    }

    pub fn get_block_inner(&self, id: &impl DbBlockIdResolvable) -> Result<Option<DeoxysBlockInner>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        self.storage_to_inner(&ty)
    }

    pub fn get_block(&self, id: &impl DbBlockIdResolvable) -> Result<Option<DeoxysMaybePendingBlock>> {
        let Some(ty) = id.resolve_db_block_id(self)? else { return Ok(None) };
        let Some(info) = self.storage_to_info(&ty)? else { return Ok(None) };
        let Some(inner) = self.storage_to_inner(&ty)? else { return Ok(None) };
        Ok(Some(DeoxysMaybePendingBlock { info, inner }))
    }

    // Tx hashes and tx status

    /// Returns the index of the tx.
    pub fn find_tx_hash_block_info(&self, tx_hash: &Felt) -> Result<Option<(DeoxysMaybePendingBlockInfo, TxIndex)>> {
        match self.tx_hash_to_block_n(tx_hash)? {
            Some(block_n) => {
                let Some(info) = self.get_block_info_from_block_n(block_n)? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                Ok(Some((info.into(), TxIndex(tx_index as _))))
            }
            None => {
                let Some(info) = self.get_pending_block_info()? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                Ok(Some((info.into(), TxIndex(tx_index as _))))
            }
        }
    }

    /// Returns the index of the tx.
    pub fn find_tx_hash_block(&self, tx_hash: &Felt) -> Result<Option<(DeoxysMaybePendingBlock, TxIndex)>> {
        match self.tx_hash_to_block_n(tx_hash)? {
            Some(block_n) => {
                let Some(info) = self.get_block_info_from_block_n(block_n)? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                let Some(inner) = self.get_block_inner_from_block_n(block_n)? else { return Ok(None) };
                Ok(Some((DeoxysMaybePendingBlock { info: info.into(), inner }, TxIndex(tx_index as _))))
            }
            None => {
                let Some(info) = self.get_pending_block_info()? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes.iter().position(|a| a == tx_hash) else { return Ok(None) };
                let Some(inner) = self.get_pending_block_inner()? else { return Ok(None) };
                Ok(Some((DeoxysMaybePendingBlock { info: info.into(), inner }, TxIndex(tx_index as _))))
            }
        }
    }
}
