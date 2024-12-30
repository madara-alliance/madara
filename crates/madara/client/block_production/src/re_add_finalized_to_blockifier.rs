use std::time::{Duration, SystemTime};

use mc_db::{MadaraBackend, MadaraStorageError};
use mc_mempool::{MempoolProvider, MempoolTransaction};
use mp_block::{header::BlockTimestamp, BlockId, BlockTag, MadaraMaybePendingBlock};
use mp_transactions::{ToBlockifierError, TransactionWithHash};
use starknet_core::types::Felt;

#[derive(Debug, thiserror::Error)]
pub enum ReAddTxsToMempoolError {
    #[error(
        "Converting transaction with hash {tx_hash:#x}: Error when getting class with hash {class_hash:#x}: {err:#}"
    )]
    GettingConvertedClass { tx_hash: Felt, class_hash: Felt, err: MadaraStorageError },
    #[error("Converting transaction with hash {tx_hash:#x}: No class found for class with hash {class_hash:#x}")]
    NoClassFound { tx_hash: Felt, class_hash: Felt },

    #[error("Converting transaction with hash {tx_hash:#x}: Blockifier conversion error: {err:#}")]
    ToBlockifierError { tx_hash: Felt, err: ToBlockifierError },

    /// This error should never happen unless we are running on a platform where SystemTime cannot represent the timestamp we are making.
    #[error("Converting transaction with hash {tx_hash:#x}: Could not create arrived_at timestamp with block_timestamp={block_timestamp} and tx_index={tx_index}")]
    MakingArrivedAtTimestamp { tx_hash: Felt, block_timestamp: BlockTimestamp, tx_index: usize },
}

/// Take a block that was already executed and saved, extract the transactions and re-add them to the mempool.
/// This is useful to re-execute a pending block without losing any transaction when restarting block production,
/// but it it could also be useful to avoid dropping transactions when a reorg happens in the future.
/// Returns the number of transactions.
pub fn re_add_txs_to_mempool(
    block: MadaraMaybePendingBlock,
    mempool: &impl MempoolProvider,
    backend: &MadaraBackend,
) -> Result<usize, ReAddTxsToMempoolError> {
    let block_timestamp = block.info.block_timestamp();

    let txs_to_reexec: Vec<_> = block
        .inner
        .transactions
        .into_iter()
        .zip(block.info.tx_hashes())
        .enumerate()
        .map(|(tx_index, (tx, &tx_hash))| {
            let converted_class = if let Some(tx) = tx.as_declare() {
                let class_hash = *tx.class_hash();
                Some(
                    backend
                        .get_converted_class(&BlockId::Tag(BlockTag::Pending), &class_hash)
                        .map_err(|err| ReAddTxsToMempoolError::GettingConvertedClass { tx_hash, class_hash, err })?
                        .ok_or_else(|| ReAddTxsToMempoolError::NoClassFound { tx_hash, class_hash })?,
                )
            } else {
                None
            };

            let tx = TransactionWithHash::new(tx, tx_hash)
                .into_blockifier(converted_class.as_ref())
                .map_err(|err| ReAddTxsToMempoolError::ToBlockifierError { tx_hash, err })?;

            // HACK: we hack the order a little bit - this is because we don't have the arrived_at timestamp for the
            // transaction. This hack ensures these trasactions should have priority and should be reexecuted
            fn make_arrived_at(block_timestamp: BlockTimestamp, tx_index: usize) -> Option<SystemTime> {
                let duration =
                    Duration::from_secs(block_timestamp.0).checked_add(Duration::from_micros(tx_index as _))?;
                SystemTime::UNIX_EPOCH.checked_add(duration)
            }

            let arrived_at = make_arrived_at(block_timestamp, tx_index).ok_or_else(|| {
                ReAddTxsToMempoolError::MakingArrivedAtTimestamp { tx_hash, block_timestamp, tx_index }
            })?;
            Ok(MempoolTransaction { tx, arrived_at, converted_class })
        })
        .collect::<Result<_, _>>()?;

    let n = txs_to_reexec.len();

    mempool
        .insert_txs_no_validation(txs_to_reexec, /* force insertion */ true)
        .expect("Mempool force insertion should never fail");

    Ok(n)
}
