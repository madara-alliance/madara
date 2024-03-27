//! A collection of node-specific RPC methods.
//! Substrate provides the `sc-rpc` crate, which defines the core RPC layer
//! used by Substrate nodes. This file extends those RPC definitions with
//! capabilities that are specific to this project's runtime configuration.

#![warn(missing_docs)]

mod starknet;
use std::sync::Arc;

use deoxys_runtime::{AccountId, Hash, Index, StarknetHasher};
use futures::channel::mpsc;
use jsonrpsee::RpcModule;
use mc_genesis_data_provider::GenesisProvider;
use mp_types::block::DBlockT;
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_consensus_manual_seal::rpc::EngineCommand;
pub use sc_rpc_api::DenyUnsafe;
use sc_transaction_pool::{ChainApi, Pool};
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_block_builder::BlockBuilder;
use sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};
pub use starknet::StarknetDeps;

/// Full client dependencies.
pub struct FullDeps<A: ChainApi, C, G: GenesisProvider, P> {
    /// The client instance to use.
    pub client: Arc<C>,
    /// Transaction pool instance.
    pub pool: Arc<P>,
    /// Extrinsic pool graph instance.
    pub graph: Arc<Pool<A>>,
    /// Whether to deny unsafe calls
    pub deny_unsafe: DenyUnsafe,
    /// Manual seal command sink
    pub command_sink: Option<mpsc::Sender<EngineCommand<Hash>>>,
    /// Starknet dependencies
    pub starknet: StarknetDeps<C, G, DBlockT>,
}

/// Instantiate all full RPC extensions.
pub fn create_full<A, C, G, P, BE>(
    deps: FullDeps<A, C, G, P>,
) -> Result<RpcModule<()>, Box<dyn std::error::Error + Send + Sync>>
where
    A: ChainApi<Block = DBlockT> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C: HeaderBackend<DBlockT>
        + BlockBackend<DBlockT>
        + HeaderMetadata<DBlockT, Error = BlockChainError>
        + StorageProvider<DBlockT, BE>
        + 'static,
    C: Send + Sync + 'static,
    C::Api: substrate_frame_rpc_system::AccountNonceApi<DBlockT, AccountId, Index>,
    C::Api: BlockBuilder<DBlockT>,
    C::Api: pallet_starknet_runtime_api::StarknetRuntimeApi<DBlockT>
        + pallet_starknet_runtime_api::ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
{
    use mc_rpc::{Starknet, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
    use sc_consensus_manual_seal::rpc::{ManualSeal, ManualSealApiServer};
    use substrate_frame_rpc_system::{System, SystemApiServer};

    let mut module = RpcModule::new(());
    let FullDeps { client, pool, deny_unsafe, starknet: starknet_params, command_sink, graph, .. } = deps;

    module.merge(System::new(client.clone(), pool.clone(), deny_unsafe).into_rpc())?;
    module.merge(StarknetReadRpcApiServer::into_rpc(Starknet::<_, _, _, _, _, StarknetHasher>::new(
        client.clone(),
        starknet_params.overrides.clone(),
        pool.clone(),
        graph.clone(),
        starknet_params.sync_service.clone(),
        starknet_params.starting_block,
        starknet_params.genesis_provider.clone(),
    )))?;
    module.merge(StarknetWriteRpcApiServer::into_rpc(Starknet::<_, _, _, _, _, StarknetHasher>::new(
        client.clone(),
        starknet_params.overrides.clone(),
        pool.clone(),
        graph.clone(),
        starknet_params.sync_service.clone(),
        starknet_params.starting_block,
        starknet_params.genesis_provider.clone(),
    )))?;
    module.merge(StarknetTraceRpcApiServer::into_rpc(Starknet::<_, _, _, _, _, StarknetHasher>::new(
        client,
        starknet_params.overrides,
        pool,
        graph,
        starknet_params.sync_service,
        starknet_params.starting_block,
        starknet_params.genesis_provider,
    )))?;

    if let Some(command_sink) = command_sink {
        module.merge(
            // We provide the rpc handler with the sending end of the channel to allow the rpc
            // send EngineCommands to the background block authorship task.
            ManualSeal::new(command_sink).into_rpc(),
        )?;
    }

    Ok(module)
}
