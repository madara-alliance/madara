use std::sync::Arc;

use deoxys_runtime::opaque::{DBlockT, DHashT};
// Substrate
use parity_scale_codec::{Decode, Encode};
use rocksdb::WriteBatchWithTransaction;
use sp_runtime::traits::Block as BlockT;
use starknet_api::hash::StarkHash;

use crate::{Column, DbError, DB, DatabaseExt};

/// The mapping to write in db
#[derive(Debug)]
pub struct MappingCommitment<B: BlockT> {
    pub block_number: u64,
    pub block_hash: B::Hash,
    pub starknet_block_hash: StarkHash,
    pub starknet_transaction_hashes: Vec<StarkHash>,
}

/// Allow interaction with the mapping db
pub struct MappingDb {
    db: Arc<DB>,
    /// Whether more information should be cached in the database.
    cache_more_things: bool,
}

impl MappingDb {
    /// Creates a new instance of the mapping database.
    pub(crate) fn new(db: Arc<DB>, cache_more_things: bool) -> Self {
        Self { db, cache_more_things }
    }

    /// Check if the given block hash has already been processed
    pub fn is_synced(&self, block_hash: &DHashT) -> Result<bool, DbError> {
        let synced_mapping_col = self.db.get_column(Column::SyncedMapping);

        match self.db.get_cf(&synced_mapping_col, &block_hash.encode())? {
            Some(raw) => Ok(bool::decode(&mut &raw[..])?),
            None => Ok(false),
        }
    }

    /// Return the hash of the Substrate block wrapping the Starknet block with given hash
    ///
    /// Under some circumstances it can return multiples blocks hashes, meaning that the result has
    /// to be checked against the actual blockchain state in order to find the good one.
    pub fn block_hash(&self, starknet_block_hash: StarkHash) -> Result<Option<Vec<DHashT>>, DbError> {
        let block_mapping_col = self.db.get_column(Column::BlockMapping);

        match self.db.get_cf(&block_mapping_col, &starknet_block_hash.encode())? {
            Some(raw) => Ok(Some(Vec::<DHashT>::decode(&mut &raw[..])?)),
            None => Ok(None),
        }
    }

    /// Register that a Substrate block has been seen, without it containing a Starknet one
    pub fn write_none(&self, block_hash: DHashT) -> Result<(), DbError> {
        let synced_mapping_col = self.db.get_column(Column::SyncedMapping);

        self.db.put_cf(&synced_mapping_col, &block_hash.encode(), &true.encode())?;
        Ok(())
    }

    /// Register that a Substate block has been seen and map it to the Statknet block it contains
    pub fn write_hashes(&self, commitment: MappingCommitment<DBlockT>) -> Result<(), DbError> {
        let synced_mapping_col = self.db.get_column(Column::SyncedMapping);
        let block_mapping_col = self.db.get_column(Column::BlockMapping);
        let transaction_mapping_col = self.db.get_column(Column::TransactionMapping);
        let starknet_tx_hashes_col = self.db.get_column(Column::StarknetTransactionHashesCache);
        let starknet_block_hashes_col = self.db.get_column(Column::StarknetBlockHashesCache);

        let mut transaction: WriteBatchWithTransaction<true> = Default::default();

        let substrate_hashes = match self.block_hash(commitment.starknet_block_hash) {
            Ok(Some(mut data)) => {
                data.push(commitment.block_hash);
                // log::warn!(
                //     target: "fc-db",
                //     "Possible equivocation at starknet block hash {} {:?}",
                //     &commitment.starknet_block_hash,
                //     &data
                // );
                data
            }
            _ => vec![commitment.block_hash],
        };

        transaction.put_cf(&block_mapping_col, &commitment.starknet_block_hash.encode(), &substrate_hashes.encode());

        transaction.put_cf(&synced_mapping_col, &commitment.block_hash.encode(), &true.encode());

        for transaction_hash in commitment.starknet_transaction_hashes.iter() {
            transaction.put_cf(
                &transaction_mapping_col,
                &transaction_hash.encode(),
                &commitment.block_hash.encode(),
            );
        }

        if self.cache_more_things {
            transaction.put_cf(
                &starknet_tx_hashes_col,
                &commitment.starknet_block_hash.encode(),
                &commitment.starknet_transaction_hashes.encode(),
            );

            transaction.put_cf(
                &starknet_block_hashes_col,
                &commitment.block_number.encode(),
                &commitment.starknet_block_hash.encode(),
            );
        }

        self.db.write(transaction)?;

        Ok(())
    }

    /// Retrieves the substrate block hash
    /// associated with the given transaction hash, if any.
    ///
    /// # Arguments
    ///
    /// * `transaction_hash` - the transaction hash to search for. H256 is used here because it's a
    ///   native type of substrate, and we are sure it's SCALE encoding is optimized and will not
    ///   change.
    pub fn block_hash_from_transaction_hash(&self, transaction_hash: StarkHash) -> Result<Option<DHashT>, DbError> {
        let transaction_mapping_col = self.db.get_column(Column::TransactionMapping);

        match self.db.get_cf(&transaction_mapping_col, &transaction_hash.encode())? {
            Some(raw) => Ok(Some(<DHashT>::decode(&mut &raw[..])?)),
            None => Ok(None),
        }
    }

    /// Returns the list of transaction hashes for the given block hash.
    ///
    /// # Arguments
    ///
    /// * `starknet_hash` - the hash of the starknet block to search for.
    ///
    /// # Returns
    ///
    /// The list of transaction hashes.
    ///
    /// This function may return `None` for two separate reasons:
    ///
    /// - The cache is disabled.
    /// - The provided `starknet_hash` is not present in the cache.
    pub fn cached_transaction_hashes_from_block_hash(
        &self,
        starknet_block_hash: StarkHash,
    ) -> Result<Option<Vec<StarkHash>>, DbError> {
        let starknet_tx_hashes_col = self.db.get_column(Column::StarknetTransactionHashesCache);

        if !self.cache_more_things {
            // The cache is not enabled, no need to even touch the database.
            return Ok(None);
        }

        match self.db.get_cf(&starknet_tx_hashes_col, &starknet_block_hash.encode())? {
            Some(raw) => Ok(Some(Vec::<StarkHash>::decode(&mut &raw[..])?)),
            None => Ok(None),
        }
    }

    /// Returns the cached block hash of a given block number.
    ///
    /// # Arguments
    ///
    /// * `block_number` - the block number to search for.
    ///
    /// # Returns
    ///
    /// The block hash of a given block number.
    ///
    /// This function may return `None` for two separate reasons:
    ///
    /// - The cache is disabled.
    /// - The provided `starknet_hash` is not present in the cache.
    pub fn cached_block_hash_from_block_number(
        &self,
        starknet_block_number: u64,
    ) -> Result<Option<StarkHash>, DbError> {
        let starknet_block_hashes_col = self.db.get_column(Column::StarknetBlockHashesCache);

        if !self.cache_more_things {
            // The cache is not enabled, no need to even touch the database.
            return Ok(None);
        }

        match self.db.get_cf(&starknet_block_hashes_col, &starknet_block_number.encode())? {
            Some(raw) => Ok(Some(<StarkHash>::decode(&mut &raw[..])?)),
            None => Ok(None),
        }
    }
}
