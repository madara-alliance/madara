use jsonrpsee::core::error::Error;
use jsonrpsee::core::RpcResult;
use mc_sync::l2::get_pending_block;
use mp_hashers::HasherT;
use mp_types::block::{DBlockT, DHashT};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{
    BlockWithTxHashes, BlockWithTxs, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, PendingBlockWithTxHashes,
    PendingBlockWithTxs,
};

use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::utils::block::{
    l1_da_mode, l1_data_gas_price, l1_gas_price, new_root, parent_hash, sequencer_address, starknet_version, timestamp,
};
use crate::utils::helpers::{status, tx_conv, tx_hash_compute, tx_hash_retrieve};
use crate::{Felt, Starknet};

pub(crate) fn get_block_with_tx_hashes_finalized<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    substrate_block_hash: DHashT,
) -> RpcResult<MaybePendingBlockWithTxHashes>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>();
    let block_txs_hashes = tx_hash_retrieve(starknet.get_cached_transaction_hashes(block_hash.into())?);

    let block_number = starknet_block.header().block_number;
    let status = status(block_number);
    let parent_hash = parent_hash(&starknet_block);
    let new_root = new_root(&starknet_block);
    let timestamp = timestamp(&starknet_block);
    let sequencer_address = sequencer_address(&starknet_block);
    let l1_gas_price = l1_gas_price(&starknet_block);
    let l1_data_gas_price = l1_data_gas_price(&starknet_block);
    let starknet_version = starknet_version(&starknet_block);
    let l1_da_mode = l1_da_mode(&starknet_block);

    let block_with_tx_hashes = BlockWithTxHashes {
        transactions: block_txs_hashes,
        status,
        block_hash: block_hash.into(),
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

pub(crate) fn get_block_with_tx_hashes_pending<H>(chain_id: Felt) -> RpcResult<MaybePendingBlockWithTxHashes>
where
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_pending_block()
        .ok_or(Error::Custom("Failed to retrieve pending block, node not yet synchronized".to_string()))?;

    let transactions = tx_hash_compute::<H>(&starknet_block, chain_id);
    let parent_hash = parent_hash(&starknet_block);
    let timestamp = timestamp(&starknet_block);
    let sequencer_address = sequencer_address(&starknet_block);
    let l1_gas_price = l1_gas_price(&starknet_block);
    let l1_data_gas_price = l1_data_gas_price(&starknet_block);
    let starknet_version = starknet_version(&starknet_block);
    let l1_da_mode = l1_da_mode(&starknet_block);

    let block_with_tx_hashes = PendingBlockWithTxHashes {
        transactions,
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

pub(crate) fn get_block_with_txs_finalized<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    substrate_block_hash: DHashT,
) -> RpcResult<MaybePendingBlockWithTxs>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>();
    let block_txs_hashes = tx_hash_retrieve(starknet.get_cached_transaction_hashes(block_hash.into())?);
    let transactions = tx_conv(starknet_block.transactions(), block_txs_hashes);

    let block_number = starknet_block.header().block_number;
    let status = status(block_number);
    let parent_hash = parent_hash(&starknet_block);
    let new_root = new_root(&starknet_block);
    let timestamp = timestamp(&starknet_block);
    let sequencer_address = sequencer_address(&starknet_block);
    let l1_gas_price = l1_gas_price(&starknet_block);
    let l1_data_gas_price = l1_data_gas_price(&starknet_block);
    let starknet_version = starknet_version(&starknet_block);
    let l1_da_mode = l1_da_mode(&starknet_block);

    let block_with_txs = BlockWithTxs {
        status,
        block_hash: block_hash.into(),
        parent_hash,
        block_number,
        new_root,
        timestamp,
        sequencer_address,
        transactions,
        l1_gas_price,
        l1_data_gas_price,
        starknet_version,
        l1_da_mode,
    };

    Ok(MaybePendingBlockWithTxs::Block(block_with_txs))
}

pub(crate) fn get_block_with_txs_pending<H>(chain_id: Felt) -> RpcResult<MaybePendingBlockWithTxs>
where
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_pending_block()
        .ok_or(Error::Custom("Failed to retrieve pending block, node not yet synchronized".to_string()))?;

    let tx_hashes = tx_hash_compute::<H>(&starknet_block, chain_id);
    let transactions = tx_conv(starknet_block.transactions(), tx_hashes);

    let parent_hash = parent_hash(&starknet_block);
    let timestamp = timestamp(&starknet_block);
    let sequencer_address = sequencer_address(&starknet_block);
    let l1_gas_price = l1_gas_price(&starknet_block);
    let l1_data_gas_price = l1_data_gas_price(&starknet_block);
    let starknet_version = starknet_version(&starknet_block);
    let l1_da_mode = l1_da_mode(&starknet_block);

    let block_with_txs = PendingBlockWithTxs {
        transactions,
        parent_hash,
        timestamp,
        sequencer_address,
        l1_gas_price,
        l1_data_gas_price,
        starknet_version,
        l1_da_mode,
    };

    Ok(MaybePendingBlockWithTxs::PendingBlock(block_with_txs))
}
