use jsonrpsee::core::error::Error;
use jsonrpsee::core::RpcResult;
use mc_genesis_data_provider::GenesisProvider;
use mc_sync::l2::get_pending_block;
use mp_hashers::HasherT;
use mp_types::block::{DBlockT, DHashT};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{
    BlockWithTxHashes, BlockWithTxs, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, PendingBlockWithTxHashes,
    PendingBlockWithTxs,
};

use crate::utils::{
    get_block_by_block_hash, l1_gas_price, new_root, parent_hash, sequencer_address, starknet_version, status,
    timestamp, tx_conv, tx_hash_compute, tx_hash_retrieve,
};
use crate::{Felt, Starknet};

pub(crate) fn get_block_with_tx_hashes_finalized<A, BE, G, C, P, H>(
    server: &Starknet<A, BE, G, C, P, H>,
    chain_id: Felt,
    substrate_block_hash: DHashT,
) -> RpcResult<MaybePendingBlockWithTxHashes>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_block_by_block_hash(server.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>();
    let transactions = if let Some(tx_hashes) = server.get_cached_transaction_hashes(block_hash.into()) {
        tx_hash_retrieve(tx_hashes)
    } else {
        tx_hash_compute::<H>(&starknet_block, chain_id)
    };

    let block_number = starknet_block.header().block_number;
    let status = status(block_number);
    let parent_hash = parent_hash(&starknet_block);
    let new_root = new_root(&starknet_block);
    let timestamp = timestamp(&starknet_block);
    let sequencer_address = sequencer_address(&starknet_block);
    let l1_gas_price = l1_gas_price(&starknet_block);
    let starknet_version = starknet_version(&starknet_block);

    let block_with_tx_hashes = BlockWithTxHashes {
        transactions,
        status,
        block_hash: block_hash.into(),
        parent_hash,
        block_number,
        new_root,
        timestamp,
        sequencer_address,
        l1_gas_price,
        starknet_version,
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
    let starknet_version = starknet_version(&starknet_block);

    let block_with_tx_hashes = PendingBlockWithTxHashes {
        transactions,
        parent_hash,
        timestamp,
        sequencer_address,
        l1_gas_price,
        starknet_version,
    };

    Ok(MaybePendingBlockWithTxHashes::PendingBlock(block_with_tx_hashes))
}

pub(crate) fn get_block_with_txs_finalized<A, BE, G, C, P, H>(
    server: &Starknet<A, BE, G, C, P, H>,
    chain_id: Felt,
    substrate_block_hash: DHashT,
) -> RpcResult<MaybePendingBlockWithTxs>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_block_by_block_hash(server.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>();
    let tx_hashes = if let Some(tx_hashes) = server.get_cached_transaction_hashes(block_hash.into()) {
        tx_hash_retrieve(tx_hashes)
    } else {
        tx_hash_compute::<H>(&starknet_block, chain_id)
    };
    let transactions = tx_conv(starknet_block.transactions(), tx_hashes);

    let block_number = starknet_block.header().block_number;
    let status = status(block_number);
    let parent_hash = parent_hash(&starknet_block);
    let new_root = new_root(&starknet_block);
    let timestamp = timestamp(&starknet_block);
    let sequencer_address = sequencer_address(&starknet_block);
    let l1_gas_price = l1_gas_price(&starknet_block);
    let starknet_version = starknet_version(&starknet_block);

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
        starknet_version,
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
    let starknet_version = starknet_version(&starknet_block);

    let block_with_txs =
        PendingBlockWithTxs { transactions, parent_hash, timestamp, sequencer_address, l1_gas_price, starknet_version };

    Ok(MaybePendingBlockWithTxs::PendingBlock(block_with_txs))
}
