use std::sync::Arc;

use dp_block::{BlockId, BlockTag, DeoxysBlock, DeoxysBlockInfo, DeoxysBlockInner};
use starknet_api::block::BlockHash;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::PendingStateUpdate;

use crate::storage_handler::codec;
use crate::{Column, DatabaseExt, WriteBatchWithTransaction, DB};

#[derive(Debug, thiserror::Error)]
pub enum MappingDbError {
    #[error("rocksdb error: {0}")]
    RocksDBError(#[from] rocksdb::Error),
    #[error("db number format error")]
    NumberFormat,
    #[error("value codec error: {0}")]
    Codec(#[from] codec::Error),
    #[error("value codec error: {0}")]
    Bincode(#[from] bincode::Error),
}

type Result<T, E = MappingDbError> = std::result::Result<T, E>;

/// Allow interaction with the mapping db
#[derive(Debug)]
pub struct MappingDb {
    db: Arc<DB>,
}

#[derive(Debug, Clone)]
pub enum BlockStorageType {
    Pending,
    BlockN(u64),
}

const ROW_PENDING_INFO: &[u8] = b"pending_info";
const ROW_PENDING_STATE_UPDATE: &[u8] = b"pending_state_update";
const ROW_PENDING_INNER: &[u8] = b"pending";
const ROW_SYNC_TIP: &[u8] = b"sync_tip";
const ROW_L1_LAST_CONFIRMED_BLOCK: &[u8] = b"l1_last";

#[derive(Debug, Clone)]
pub struct TxStorageInfo {
    pub storage_type: BlockStorageType,
    pub tx_index: usize,
}

// TODO(error-handling): replace all of the else { return Ok(None) } with hard errors for
// inconsistent state.
impl MappingDb {
    /// Creates a new instance of the mapping database.
    pub(crate) fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    // DB read operations

    fn tx_hash_to_block_n(&self, tx_hash: &TransactionHash) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::TxHashToBlockN);
        let res = self.db.get_cf(&col, codec::Encode::encode(tx_hash)?)?;
        let Some(res) = res else { return Ok(None) };
        let block_n = codec::Decode::decode(&res)?;
        Ok(Some(block_n))
    }

    fn block_hash_to_block_n(&self, block_hash: &BlockHash) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockHashToBlockN);
        let res = self.db.get_cf(&col, codec::Encode::encode(block_hash)?)?;
        let Some(res) = res else { return Ok(None) };
        let block_n = codec::Decode::decode(&res)?;
        Ok(Some(block_n))
    }

    fn get_block_info_from_block_n(&self, block_n: u64) -> Result<Option<DeoxysBlockInfo>> {
        let col = self.db.get_column(Column::BlockNToBlockInfo);
        let res = self.db.get_cf(&col, codec::Encode::encode(&block_n)?)?;
        let Some(res) = res else { return Ok(None) };
        let block = bincode::deserialize(&res)?;
        Ok(Some(block))
    }

    fn get_block_inner_from_block_n(&self, block_n: u64) -> Result<Option<DeoxysBlockInner>> {
        let col = self.db.get_column(Column::BlockNToBlockInner);
        let res = self.db.get_cf(&col, codec::Encode::encode(&block_n)?)?;
        let Some(res) = res else { return Ok(None) };
        let block = bincode::deserialize(&res)?;
        Ok(Some(block))
    }

    fn get_latest_block_n(&self) -> Result<Option<u64>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_SYNC_TIP)? else { return Ok(None) };
        let res = codec::Decode::decode(&res)?;
        Ok(Some(res))
    }

    fn get_pending_block_info(&self) -> Result<Option<DeoxysBlockInfo>> {
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

    pub fn get_pending_block_state_update(&self) -> Result<Option<PendingStateUpdate>> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        let Some(res) = self.db.get_cf(&col, ROW_PENDING_STATE_UPDATE)? else { return Ok(None) };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    // DB write

    pub fn write_pending(
        &self,
        tx: &mut WriteBatchWithTransaction,
        block: &DeoxysBlock,
        state_update: &PendingStateUpdate,
    ) -> Result<()> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.put_cf(&col, ROW_PENDING_INFO, bincode::serialize(block.info())?);
        tx.put_cf(&col, ROW_PENDING_INNER, bincode::serialize(block.inner())?);
        tx.put_cf(&col, ROW_PENDING_STATE_UPDATE, bincode::serialize(&state_update)?);
        Ok(())
    }

    pub fn write_no_pending(&self, tx: &mut WriteBatchWithTransaction) -> Result<()> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.delete_cf(&col, ROW_PENDING_INFO);
        tx.delete_cf(&col, ROW_PENDING_INNER);
        tx.delete_cf(&col, ROW_PENDING_STATE_UPDATE);
        Ok(())
    }

    pub fn write_last_confirmed_block(&self, tx: &mut WriteBatchWithTransaction, l1_last: u64) -> Result<()> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        tx.put_cf(&col, ROW_L1_LAST_CONFIRMED_BLOCK, codec::Encode::encode(&l1_last)?);
        Ok(())
    }

    pub fn write_no_last_confirmed_block(&self, tx: &mut WriteBatchWithTransaction) -> Result<()> {
        self.write_last_confirmed_block(tx, 0)
    }

    pub fn write_new_block(&self, tx: &mut WriteBatchWithTransaction, block: &DeoxysBlock, reverted_txs: &[TransactionHash]) -> Result<()> {
        let tx_hash_to_block_n = self.db.get_column(Column::TxHashToBlockN);
        let block_hash_to_block_n = self.db.get_column(Column::BlockHashToBlockN);
        let block_n_to_block = self.db.get_column(Column::BlockNToBlockInfo);
        let block_n_to_block_inner = self.db.get_column(Column::BlockNToBlockInner);
        let reverted_txs_cl = self.db.get_column(Column::RevertedTxs);
        let meta = self.db.get_column(Column::BlockStorageMeta);

        let block_hash_encoded = codec::Encode::encode(block.block_hash())?;
        let block_n_encoded = codec::Encode::encode(&block.block_n())?;

        for hash in block.tx_hashes() {
            tx.put_cf(&tx_hash_to_block_n, codec::Encode::encode(hash)?, &block_n_encoded);
        }

        for hash in reverted_txs {
            tx.put_cf(&reverted_txs_cl, codec::Encode::encode(hash)?, []);
        }

        tx.put_cf(&block_hash_to_block_n, block_hash_encoded, &block_n_encoded);
        tx.put_cf(&block_n_to_block, &block_n_encoded, bincode::serialize(block.info())?);
        tx.put_cf(&block_n_to_block_inner, &block_n_encoded, bincode::serialize(block.inner())?);
        tx.put_cf(&meta, ROW_SYNC_TIP, block_n_encoded);

        self.write_no_pending(tx)
    }

    // Convenience functions

    fn id_to_storage_type(&self, id: &BlockId) -> Result<Option<BlockStorageType>> {
        match id {
            BlockId::Hash(felt) => {
                Ok(self.block_hash_to_block_n(&BlockHash::from(*felt))?.map(BlockStorageType::BlockN))
            }
            BlockId::Number(block_n) => Ok(Some(BlockStorageType::BlockN(*block_n))),
            BlockId::Tag(BlockTag::Latest) => Ok(self.get_latest_block_n()?.map(BlockStorageType::BlockN)),
            BlockId::Tag(BlockTag::Pending) => Ok(Some(BlockStorageType::Pending)),
        }
    }

    fn storage_to_info(&self, id: &BlockStorageType) -> Result<Option<DeoxysBlockInfo>> {
        match id {
            BlockStorageType::Pending => self.get_pending_block_info(),
            BlockStorageType::BlockN(block_n) => self.get_block_info_from_block_n(*block_n),
        }
    }

    fn storage_to_inner(&self, id: &BlockStorageType) -> Result<Option<DeoxysBlockInner>> {
        match id {
            BlockStorageType::Pending => self.get_pending_block_inner(),
            BlockStorageType::BlockN(block_n) => self.get_block_inner_from_block_n(*block_n),
        }
    }

    // BlockId

    pub fn get_block_n(&self, id: &BlockId) -> Result<Option<u64>> {
        let Some(ty) = self.id_to_storage_type(id)? else { return Ok(None) };
        match &ty {
            BlockStorageType::BlockN(block_id) => Ok(Some(*block_id)),
            BlockStorageType::Pending => Ok(self.storage_to_info(&ty)?.map(|e| e.block_n())),
        }
    }

    pub fn get_block_hash(&self, id: &BlockId) -> Result<Option<BlockHash>> {
        match id {
            BlockId::Hash(felt) => Ok(Some(BlockHash::from(*felt))),
            _ => Ok(self.get_block_info(id)?.map(|info| *info.block_hash())),
        }
    }

    pub fn get_block_info(&self, id: &BlockId) -> Result<Option<DeoxysBlockInfo>> {
        let Some(ty) = self.id_to_storage_type(id)? else { return Ok(None) };
        self.storage_to_info(&ty)
    }

    pub fn get_block_inner(&self, id: &BlockId) -> Result<Option<DeoxysBlockInner>> {
        let Some(ty) = self.id_to_storage_type(id)? else { return Ok(None) };
        self.storage_to_inner(&ty)
    }

    pub fn get_block(&self, id: &BlockId) -> Result<Option<DeoxysBlock>> {
        let Some(ty) = self.id_to_storage_type(id)? else { return Ok(None) };
        let Some(info) = self.storage_to_info(&ty)? else { return Ok(None) };
        let Some(inner) = self.storage_to_inner(&ty)? else { return Ok(None) };
        Ok(Some(DeoxysBlock::new(info, inner)))
    }

    // Tx hashes and tx status

    /// Returns the index of the tx.
    pub fn find_tx_hash_block_info(
        &self,
        tx_hash: &TransactionHash,
    ) -> Result<Option<(DeoxysBlockInfo, TxStorageInfo)>> {
        match self.tx_hash_to_block_n(tx_hash)? {
            Some(block_n) => {
                let Some(info) = self.get_block_info_from_block_n(block_n)? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes().iter().position(|a| a == tx_hash) else { return Ok(None) };
                Ok(Some((info, TxStorageInfo { storage_type: BlockStorageType::BlockN(block_n), tx_index })))
            }
            None => {
                let Some(info) = self.get_pending_block_info()? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes().iter().position(|a| a == tx_hash) else { return Ok(None) };
                Ok(Some((info, TxStorageInfo { storage_type: BlockStorageType::Pending, tx_index })))
            }
        }
    }

    /// Returns the index of the tx.
    pub fn find_tx_hash_block(&self, tx_hash: &TransactionHash) -> Result<Option<(DeoxysBlock, TxStorageInfo)>> {
        match self.tx_hash_to_block_n(tx_hash)? {
            Some(block_n) => {
                let Some(info) = self.get_pending_block_info()? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes().iter().position(|a| a == tx_hash) else { return Ok(None) };
                let Some(inner) = self.get_pending_block_inner()? else { return Ok(None) };
                Ok(Some((
                    DeoxysBlock::new(info, inner),
                    TxStorageInfo { storage_type: BlockStorageType::BlockN(block_n), tx_index },
                )))
            }
            None => {
                let Some(info) = self.get_pending_block_info()? else { return Ok(None) };
                let Some(tx_index) = info.tx_hashes().iter().position(|a| a == tx_hash) else { return Ok(None) };
                let Some(inner) = self.get_pending_block_inner()? else { return Ok(None) };
                Ok(Some((
                    DeoxysBlock::new(info, inner),
                    TxStorageInfo { storage_type: BlockStorageType::Pending, tx_index },
                )))
            }
        }
    }

    pub fn is_reverted_tx(&self, tx_hash: &TransactionHash) -> Result<bool> {
        let col = self.db.get_column(Column::RevertedTxs);
        Ok(self.db.get_cf(&col, codec::Encode::encode(tx_hash)?)?.is_some())
    }

    // pub fn get_tx_status(&self, tx_info: &TxStorageInfo) {

    // }

    // pub fn get_block_n_from_tx_hash(&self, tx_hash: &TransactionHash) -> Result<Option<u64>> {
    //     let Some(block_n) = self.tx_hash_to_block_n(tx_hash)? else { return Ok(None) };
    //     Ok(Some(block_n))
    // }

    // pub fn get_block_info_from_tx_hash(&self, tx_hash: &TransactionHash) ->
    // Result<Option<DeoxysBlockInfo>> {     let Some(block_n) = self.tx_hash_to_block_n(tx_hash)?
    // else { return Ok(None) };     let Some(block_info) =
    // self.get_block_info_from_block_n(block_n)? else { return Ok(None) };     Ok(Some(block_info))
    // }

    // pub fn get_block_inner_from_tx_hash(&self, tx_hash: &TransactionHash) ->
    // Result<Option<DeoxysBlockInner>> {     let Some(block_n) = self.tx_hash_to_block_n(tx_hash)?
    // else { return Ok(None) };     let Some(block_inner) =
    // self.get_block_inner_from_block_n(block_n)? else { return Ok(None) };
    //     Ok(Some(block_inner))
    // }

    // pub fn get_block_from_tx_hash(&self, tx_hash: &TransactionHash) -> Result<Option<DeoxysBlock>> {
    //     let Some(block_n) = self.tx_hash_to_block_n(tx_hash)? else { return Ok(None) };
    //     let Some(block_info) = self.get_block_info_from_block_n(block_n)? else { return Ok(None) };
    //     let Some(block_inner) = self.get_block_inner_from_block_n(block_n)? else { return Ok(None) };
    //     Ok(Some(DeoxysBlock::new(block_info, block_inner)))
    // }
}
