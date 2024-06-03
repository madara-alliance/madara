use anyhow::Result;
use mc_db::DeoxysBackend;
use mc_sync::l1::ETHEREUM_STATE_UPDATE;
use mp_block::DeoxysBlock;
use mp_hashers::HasherT;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use starknet_api::hash::{StarkFelt, StarkHash};
use starknet_api::transaction as stx;
use starknet_core::types::{BlockId, BlockStatus, BlockTag, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::Felt;

pub(crate) fn tx_hash_retrieve(tx_hashes: Vec<StarkFelt>) -> Vec<FieldElement> {
    // safe to unwrap because we know that the StarkFelt is a valid FieldElement
    tx_hashes.iter().map(|tx_hash| FieldElement::from_bytes_be(&tx_hash.0).unwrap()).collect()
}

pub(crate) fn tx_hash_compute<H>(block: &DeoxysBlock, chain_id: Felt) -> Vec<FieldElement>
where
    H: HasherT + Send + Sync + 'static,
{
    // safe to unwrap because we know that the StarkFelt is a valid FieldElement
    block
        .transactions_hashes::<H>(chain_id.0.into(), Some(block.header().block_number))
        .map(|tx_hash| FieldElement::from_bytes_be(&tx_hash.0.0).unwrap())
        .collect()
}

pub(crate) fn tx_conv(
    txs: &[stx::Transaction],
    tx_hashes: Vec<FieldElement>,
) -> Vec<starknet_core::types::Transaction> {
    txs.iter().zip(tx_hashes).map(|(tx, hash)| to_starknet_core_tx(tx.clone(), hash)).collect()
}

pub(crate) fn status(block_number: u64) -> BlockStatus {
    if block_number <= ETHEREUM_STATE_UPDATE.read().unwrap().block_number {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    }
}

/// Returns a list of all transaction hashes in the given block.
///
/// # Arguments
///
/// * `block_hash` - The hash of the block containing the transactions (starknet block).
pub fn txs_hashes_from_block_hash(block_hash: FieldElement) -> Result<Vec<StarkHash>, StarknetRpcApiError> {
    let block_hash = StarkFelt(block_hash.to_bytes_be());
    DeoxysBackend::mapping().transaction_hashes_from_block_hash(block_hash)?.ok_or(StarknetRpcApiError::BlockNotFound)
}

pub fn block_n_from_id(id: BlockId) -> Result<u64, StarknetRpcApiError> {
    let (latest_block_hash, latest_block_number) = DeoxysBackend::meta().get_latest_block_hash_and_number()?;
    match id {
        // Check if the block corresponding to the number is stored in the database
        BlockId::Number(number) => match DeoxysBackend::mapping().starknet_block_hash_from_block_number(number)? {
            Some(_) => Ok(number),
            None => Err(StarknetRpcApiError::BlockNotFound),
        },
        BlockId::Hash(block_hash) => {
            match DeoxysBackend::mapping().block_number_from_starknet_block_hash(StarkFelt(block_hash.to_bytes_be()))? {
                Some(block_number) => Ok(block_number),
                None if block_hash == latest_block_hash => Ok(latest_block_number),
                None => Err(StarknetRpcApiError::BlockNotFound),
            }
        }
        BlockId::Tag(BlockTag::Latest) => Ok(latest_block_number),
        BlockId::Tag(BlockTag::Pending) => Ok(latest_block_number + 1),
    }
}

pub fn block_hash_from_id(id: BlockId) -> Result<FieldElement, StarknetRpcApiError> {
    match id {
        BlockId::Number(n) => DeoxysBackend::mapping()
            .starknet_block_hash_from_block_number(n)?
            .map(|h| FieldElement::from_bytes_be(&h.0).unwrap())
            .ok_or(StarknetRpcApiError::BlockNotFound),
        BlockId::Hash(h) => Ok(h),
        BlockId::Tag(BlockTag::Latest) => Ok(DeoxysBackend::meta().get_latest_block_hash_and_number()?.0),
        BlockId::Tag(BlockTag::Pending) => Err(StarknetRpcApiError::BlockNotFound),
    }
}

pub fn block_hash_from_block_n(block_number: u64) -> Result<FieldElement, StarknetRpcApiError> {
    DeoxysBackend::mapping()
        .starknet_block_hash_from_block_number(block_number)?
        .map(|h| FieldElement::from_bytes_be(&h.0).unwrap())
        .ok_or(StarknetRpcApiError::BlockNotFound)
}
