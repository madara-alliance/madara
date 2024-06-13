use dp_block::{BlockId, BlockTag};
use dp_convert::felt_wrapper::FeltWrapper;
use dp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use jsonrpsee::core::RpcResult;
use starknet_core::types::{
    BlockStatus, BlockWithTxHashes, BlockWithTxs, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs,
    PendingBlockWithTxHashes, PendingBlockWithTxs,
};

use crate::errors::StarknetRpcApiError;
use crate::utils::block::{l1_da_mode, l1_data_gas_price, l1_gas_price, starknet_version};
use crate::utils::ResultExt;
use crate::Starknet;

pub(crate) fn get_block_with_txs(starknet: &Starknet, block_id: &BlockId) -> RpcResult<MaybePendingBlockWithTxs> {
    let block = starknet
        .block_storage()
        .get_block(block_id)
        .or_internal_server_error("Error getting block from db")?
        .ok_or(StarknetRpcApiError::BlockNotFound)?;

    let transactions = block
        .transactions()
        .iter()
        .zip(block.tx_hashes())
        .map(|(tx, tx_hash)| to_starknet_core_tx(tx, tx_hash.into_field_element()))
        .collect();

    let parent_hash = block.header().parent_block_hash.into_field_element();
    let new_root = block.header().global_state_root.into_field_element();
    let timestamp = block.header().block_timestamp;
    let sequencer_address = block.header().sequencer_address.into_field_element();
    let l1_gas_price = l1_gas_price(&block);
    let l1_data_gas_price = l1_data_gas_price(&block);
    let starknet_version = starknet_version(&block);
    let l1_da_mode = l1_da_mode(&block);

    match block_id {
        BlockId::Tag(BlockTag::Pending) => {
            let block_with_tx_hashes = PendingBlockWithTxs {
                transactions,
                parent_hash,
                timestamp,
                sequencer_address,
                l1_gas_price,
                l1_data_gas_price,
                starknet_version,
                l1_da_mode,
            };

            Ok(MaybePendingBlockWithTxs::PendingBlock(block_with_tx_hashes))
        }
        _ => {
            let block_number = block.block_n();
            let status = if block_number <= starknet.get_l1_last_confirmed_block()? {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };
            let block_hash = block.block_hash().into_field_element();
            let block_with_tx_hashes = BlockWithTxs {
                transactions,
                status,
                block_hash,
                parent_hash,
                block_number,
                new_root,
                timestamp,
                sequencer_address,
                l1_gas_price,
                l1_data_gas_price,
                starknet_version,
                l1_da_mode,
            };

            Ok(MaybePendingBlockWithTxs::Block(block_with_tx_hashes))
        }
    }
}

pub(crate) fn get_block_with_tx_hashes(
    starknet: &Starknet,
    block_id: &BlockId,
) -> RpcResult<MaybePendingBlockWithTxHashes> {
    let block = starknet
        .block_storage()
        .get_block(block_id)
        .or_internal_server_error("Error getting block from db")?
        .ok_or(StarknetRpcApiError::BlockNotFound)?;

    let block_hash_as_field = block.block_hash().into_field_element();
    let block_txs_hashes = block.tx_hashes().iter().map(FeltWrapper::into_field_element).collect::<Vec<_>>();

    let parent_hash = block.header().parent_block_hash.into_field_element();
    let new_root = block.header().global_state_root.into_field_element();
    let timestamp = block.header().block_timestamp;
    let sequencer_address = block.header().sequencer_address.into_field_element();
    let l1_gas_price = l1_gas_price(&block);
    let l1_data_gas_price = l1_data_gas_price(&block);
    let starknet_version = starknet_version(&block);
    let l1_da_mode = l1_da_mode(&block);

    match block_id {
        BlockId::Tag(BlockTag::Pending) => {
            let block_with_tx_hashes = PendingBlockWithTxHashes {
                transactions: block_txs_hashes,
                parent_hash,
                timestamp,
                sequencer_address,
                l1_gas_price,
                l1_data_gas_price,
                starknet_version,
                l1_da_mode,
            };

            Ok(MaybePendingBlockWithTxHashes::PendingBlock(block_with_tx_hashes))
        }
        _ => {
            let block_number = block.block_n();
            let status = if block_number <= starknet.get_l1_last_confirmed_block()? {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };
            let block_with_tx_hashes = BlockWithTxHashes {
                transactions: block_txs_hashes,
                status,
                block_hash: block_hash_as_field,
                parent_hash,
                block_number,
                new_root,
                timestamp,
                sequencer_address,
                l1_gas_price,
                l1_data_gas_price,
                starknet_version,
                l1_da_mode,
            };

            Ok(MaybePendingBlockWithTxHashes::Block(block_with_tx_hashes))
        }
    }
}
