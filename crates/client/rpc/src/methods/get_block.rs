use jsonrpsee::core::RpcResult;
use mp_block::{BlockId, BlockTag};
use mp_felt::{Felt252Wrapper, FeltWrapper};
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use starknet_core::types::{
    BlockWithTxHashes, BlockWithTxs, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, PendingBlockWithTxHashes,
    PendingBlockWithTxs,
};

use crate::errors::StarknetRpcApiError;
use crate::utils::block::{
    l1_da_mode, l1_data_gas_price, l1_gas_price, new_root, parent_hash, sequencer_address, starknet_version, timestamp,
};
use crate::utils::helpers::status;
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
    let block_hash = Felt252Wrapper::from(block.block_hash().0).0;

    let block_number = block.block_n();
    let status = status(block_number);
    let parent_hash = parent_hash(&block);
    let new_root = new_root(&block);
    let timestamp = timestamp(&block);
    let sequencer_address = sequencer_address(&block);
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

    let block_hash = Felt252Wrapper::from(block.block_hash().0).0;
    let block_txs_hashes = block.tx_hashes().iter().map(FeltWrapper::into_field_element).collect::<Vec<_>>();

    let block_number = block.block_n();
    let status = status(block_number);
    let parent_hash = parent_hash(&block);
    let new_root = new_root(&block);
    let timestamp = timestamp(&block);
    let sequencer_address = sequencer_address(&block);
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
            let block_with_tx_hashes = BlockWithTxHashes {
                transactions: block_txs_hashes,
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

            Ok(MaybePendingBlockWithTxHashes::Block(block_with_tx_hashes))
        }
    }
}

// pub(crate) fn get_block_with_tx_hashes_pending(chain_id: Felt) ->
// RpcResult<MaybePendingBlockWithTxHashes> {
//     let starknet_block = get_pending_block()
//         .ok_or(Error::Custom("Failed to retrieve pending block, node not yet
// synchronized".to_string()))?;

//     let transactions = tx_hash_compute::<H>(&starknet_block, chain_id);
//     let parent_hash = parent_hash(&starknet_block);
//     let timestamp = timestamp(&starknet_block);
//     let sequencer_address = sequencer_address(&starknet_block);
//     let l1_gas_price = l1_gas_price(&starknet_block);
//     let l1_data_gas_price = l1_data_gas_price(&starknet_block);
//     let starknet_version = starknet_version(&starknet_block);
//     let l1_da_mode = l1_da_mode(&starknet_block);

//     let block_with_tx_hashes = PendingBlockWithTxHashes {
//         transactions,
//         parent_hash,
//         timestamp,
//         sequencer_address,
//         l1_gas_price,
//         l1_data_gas_price,
//         starknet_version,
//         l1_da_mode,
//     };

//     Ok(MaybePendingBlockWithTxHashes::PendingBlock(block_with_tx_hashes))
// }

// pub(crate) fn get_block_with_txs_finalized<BE, C, H>(
//     starknet: &Starknet<BE, C, H>,
//     substrate_block_hash: DHashT,
// ) -> RpcResult<MaybePendingBlockWithTxs>
// where
//     BE: Backend<DBlockT> + 'static,
//     C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
//     H: HasherT + Send + Sync + 'static,
// {
//     let starknet_block = get_block_by_block_hash(starknet.client.as_ref(),
// substrate_block_hash)?;

//     let block_hash = starknet_block.header().hash::<H>();
//     let block_txs_hashes =
// tx_hash_retrieve(starknet.get_block_transaction_hashes(block_hash.into())?);     let transactions
// = tx_conv(starknet_block.transactions(), block_txs_hashes);

//     let block_number = starknet_block.header().block_number;
//     let status = status(block_number);
//     let parent_hash = parent_hash(&starknet_block);
//     let new_root = new_root(&starknet_block);
//     let timestamp = timestamp(&starknet_block);
//     let sequencer_address = sequencer_address(&starknet_block);
//     let l1_gas_price = l1_gas_price(&starknet_block);
//     let l1_data_gas_price = l1_data_gas_price(&starknet_block);
//     let starknet_version = starknet_version(&starknet_block);
//     let l1_da_mode = l1_da_mode(&starknet_block);

//     let block_with_txs = BlockWithTxs {
//         status,
//         block_hash: block_hash.into(),
//         parent_hash,
//         block_number,
//         new_root,
//         timestamp,
//         sequencer_address,
//         transactions,
//         l1_gas_price,
//         l1_data_gas_price,
//         starknet_version,
//         l1_da_mode,
//     };

//     Ok(MaybePendingBlockWithTxs::Block(block_with_txs))
// }

// pub(crate) fn get_block_with_txs_pending<H>(chain_id: Felt) ->
// RpcResult<MaybePendingBlockWithTxs> where
//     H: HasherT + Send + Sync + 'static,
// {
//     let starknet_block = get_pending_block()
//         .ok_or(Error::Custom("Failed to retrieve pending block, node not yet
// synchronized".to_string()))?;

//     let tx_hashes = tx_hash_compute::<H>(&starknet_block, chain_id);
//     let transactions = tx_conv(starknet_block.transactions(), tx_hashes);

//     let parent_hash = parent_hash(&starknet_block);
//     let timestamp = timestamp(&starknet_block);
//     let sequencer_address = sequencer_address(&starknet_block);
//     let l1_gas_price = l1_gas_price(&starknet_block);
//     let l1_data_gas_price = l1_data_gas_price(&starknet_block);
//     let starknet_version = starknet_version(&starknet_block);
//     let l1_da_mode = l1_da_mode(&starknet_block);

//     let block_with_txs = PendingBlockWithTxs {
//         transactions,
//         parent_hash,
//         timestamp,
//         sequencer_address,
//         l1_gas_price,
//         l1_data_gas_price,
//         starknet_version,
//         l1_da_mode,
//     };

//     Ok(MaybePendingBlockWithTxs::PendingBlock(block_with_txs))
// }
