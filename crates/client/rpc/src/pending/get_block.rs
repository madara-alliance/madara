use jsonrpsee::core::error::Error;
use jsonrpsee::core::RpcResult;
use mc_genesis_data_provider::GenesisProvider;
use mc_rpc_core::Felt;
use mc_sync::l1::ETHEREUM_STATE_UPDATE;
use mc_sync::l2::get_pending_block;
use mp_block::BlockStatus;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_api::hash::StarkFelt;
use starknet_core::types::{BlockWithTxHashes, FieldElement, MaybePendingBlockWithTxHashes, PendingBlockWithTxHashes};

use crate::Starknet;

pub(crate) fn get_block_with_tx_hashes_l1<A, B, BE, G, C, P, H>(
    server: &Starknet<A, B, BE, G, C, P, H>,
    chain_id: Felt,
    substrate_block_hash: B::Hash,
) -> RpcResult<MaybePendingBlockWithTxHashes>
where
    A: ChainApi<Block = B> + 'static,
    B: BlockT,
    P: TransactionPool<Block = B> + 'static,
    BE: Backend<B> + 'static,
    C: HeaderBackend<B> + BlockBackend<B> + StorageProvider<B, BE> + 'static,
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B> + ConvertTransactionRuntimeApi<B>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = mc_rpc_core::utils::get_block_by_block_hash(server.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>();
    let transactions = if let Some(tx_hashes) = server.get_cached_transaction_hashes(block_hash.into()) {
        tx_hash_retrieve(tx_hashes)
    } else {
        tx_hash_compute::<H>(&starknet_block, chain_id)
    };

    let block_number = starknet_block.header().block_number;
    let status: BlockStatus = if block_number <= ETHEREUM_STATE_UPDATE.lock().unwrap().block_number {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };

    let parent_hash = Felt252Wrapper::from(starknet_block.header().parent_block_hash).into();
    let new_root = Felt252Wrapper::from(starknet_block.header().global_state_root).into();
    let timestamp = starknet_block.header().block_timestamp;
    let sequencer_address = Felt252Wrapper::from(starknet_block.header().sequencer_address).into();
    let l1_gas_price = starknet_block.header().l1_gas_price.into();
    let starknet_version =
        starknet_block.header().protocol_version.from_utf8().expect("starknet version should be a valid utf8 string");

    let block_with_tx_hashes = BlockWithTxHashes {
        transactions,
        status: status.into(),
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

pub(crate) fn get_block_with_tx_hashes_l2<H>(chain_id: Felt) -> RpcResult<MaybePendingBlockWithTxHashes>
where
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_pending_block()
        .ok_or(Error::Custom("Failed to retrieve pending block, node not yet synchronized".to_string()))?;

    let transactions = tx_hash_compute::<H>(&starknet_block, chain_id);
    let parent_hash = Felt252Wrapper::from(starknet_block.header().parent_block_hash).into();
    let timestamp = starknet_block.header().block_timestamp;
    let sequencer_address = Felt252Wrapper::from(starknet_block.header().sequencer_address).into();
    let l1_gas_price = starknet_block.header().l1_gas_price.into();
    let starknet_version =
        starknet_block.header().protocol_version.from_utf8().expect("starknet version should be a valid utf8 string");

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

fn tx_hash_retrieve(tx_hashes: Vec<StarkFelt>) -> Vec<FieldElement> {
    let mut v = Vec::with_capacity(tx_hashes.len());
    for tx_hash in tx_hashes {
        v.push(FieldElement::from(Felt252Wrapper::from(tx_hash)));
    }
    v
}

fn tx_hash_compute<H>(block: &mp_block::Block, chain_id: Felt) -> Vec<FieldElement>
where
    H: HasherT + Send + Sync + 'static,
{
    block
        .transactions_hashes::<H>(chain_id.0.into(), Some(block.header().block_number))
        .map(FieldElement::from)
        .collect()
}
