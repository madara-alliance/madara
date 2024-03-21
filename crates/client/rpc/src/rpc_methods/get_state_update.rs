use jsonrpsee::core::error::Error;
use jsonrpsee::core::RpcResult;
use log::error;
use madara_runtime::opaque::{Block, BlockHash};
use mc_genesis_data_provider::GenesisProvider;
use mc_rpc_core::utils::get_block_by_block_hash;
use mc_sync::l2::get_pending_state_update;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::block::BlockHash as APIBlockHash;
use starknet_core::types::{FieldElement, MaybePendingStateUpdate, StateDiff, StateUpdate};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

pub(crate) fn get_state_update_finalized<A, BE, G, C, P, H>(
    server: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: BlockHash,
) -> RpcResult<MaybePendingStateUpdate>
where
    A: ChainApi<Block = Block> + 'static,
    P: TransactionPool<Block = Block> + 'static,
    BE: Backend<Block> + 'static,
    C: HeaderBackend<Block> + BlockBackend<Block> + StorageProvider<Block, BE> + 'static,
    C: ProvideRuntimeApi<Block>,
    C::Api: StarknetRuntimeApi<Block> + ConvertTransactionRuntimeApi<Block>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_block_by_block_hash(server.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>().into();

    let new_root = Felt252Wrapper::from(starknet_block.header().global_state_root).into();

    let old_root = if starknet_block.header().block_number > 0 {
        Felt252Wrapper::from(server.backend.temporary_global_state_root_getter()).into()
    } else {
        FieldElement::default()
    };

    let state_diff = state_diff(&starknet_block, server)?;

    Ok(MaybePendingStateUpdate::Update(StateUpdate { block_hash, old_root, new_root, state_diff }))
}

pub(crate) fn get_state_update_pending() -> RpcResult<MaybePendingStateUpdate> {
    match get_pending_state_update() {
        Some(state_update) => Ok(MaybePendingStateUpdate::PendingUpdate(state_update)),
        None => Err(Error::Custom("Failed to retrieve pending state update, node not yet synchronized".to_string())),
    }
}

fn state_diff<A, BE, G, C, P, H>(block: &mp_block::Block, server: &Starknet<A, BE, G, C, P, H>) -> RpcResult<StateDiff>
where
    A: ChainApi<Block = Block> + 'static,
    P: TransactionPool<Block = Block> + 'static,
    BE: Backend<Block> + 'static,
    C: HeaderBackend<Block> + BlockBackend<Block> + StorageProvider<Block, BE> + 'static,
    C: ProvideRuntimeApi<Block>,
    C::Api: StarknetRuntimeApi<Block> + ConvertTransactionRuntimeApi<Block>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block_hash = APIBlockHash(block.header().hash::<H>().into());

    Ok(server.get_state_diff(&starknet_block_hash).map_err(|e| {
        error!("Failed to get state diff. Starknet block hash: {starknet_block_hash}, error: {e}");
        StarknetRpcApiError::InternalServerError
    })?)
}
