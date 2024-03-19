//! Starknet RPC server API implementation
//!
//! It uses the madara client and backend in order to answer queries.

mod constants;
mod errors;
mod events;
mod madara_backend_client;
mod rpc_methods;
mod trace_api;
mod types;
mod utils;

use std::marker::PhantomData;
use std::sync::Arc;

use errors::StarknetRpcApiError;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::error::CallError;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
pub use mc_rpc_core::{Felt, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
use mc_storage::OverrideHandle;
use mc_sync::l2::get_highest_block_hash_and_number;
use mc_sync::utility::get_config;
use mp_contract::class::ContractClassWrapper;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use mp_transactions::UserTransaction;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use rpc_methods::get_state_update::{get_state_update_finalized, get_state_update_pending};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_network_sync::SyncingService;
use sc_transaction_pool::{ChainApi, Pool};
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_arithmetic::traits::UniqueSaturatedInto;
use sp_blockchain::HeaderBackend;
use sp_core::H256;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};
use starknet_api::block::BlockHash;
use starknet_api::hash::StarkHash;
use starknet_core::types::{
    BlockId, BlockTag, BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction,
    BroadcastedInvokeTransaction, BroadcastedTransaction, ContractClass, DeclareTransactionResult,
    DeployAccountTransactionResult, EventFilterWithPage, EventsPage, FeeEstimate, FieldElement,
    InvokeTransactionResult, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, MaybePendingStateUpdate,
    MaybePendingTransactionReceipt, MsgFromL1, StateDiff, SyncStatus, SyncStatusType, Transaction,
};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};

use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::rpc_methods::get_block::{
    get_block_with_tx_hashes_finalized, get_block_with_tx_hashes_pending, get_block_with_txs_finalized,
    get_block_with_txs_pending,
};
use crate::rpc_methods::get_transaction_receipt::{get_transaction_receipt_finalized, get_transaction_receipt_pending};
use crate::types::RpcEventFilter;

/// A Starknet RPC server for Madara
#[allow(dead_code)]
pub struct Starknet<A: ChainApi, B: BlockT, BE, G, C, P, H> {
    client: Arc<C>,
    backend: Arc<mc_db::Backend<B>>,
    overrides: Arc<OverrideHandle<B>>,
    #[allow(dead_code)]
    pool: Arc<P>,
    #[allow(dead_code)]
    graph: Arc<Pool<A>>,
    sync_service: Arc<SyncingService<B>>,
    starting_block: <<B>::Header as HeaderT>::Number,
    #[allow(dead_code)]
    genesis_provider: Arc<G>,
    _marker: PhantomData<(B, BE, H)>,
}

/// Constructor for A Starknet RPC server for Madara
/// # Arguments
// * `client` - The Madara client
// * `backend` - The Madara backend
// * `overrides` - The OverrideHandle
// * `sync_service` - The Substrate client sync service
// * `starting_block` - The starting block for the syncing
// * `hasher` - The hasher used by the runtime
//
// # Returns
// * `Self` - The actual Starknet struct
#[allow(clippy::too_many_arguments)]
impl<A: ChainApi, B: BlockT, BE, G, C, P, H> Starknet<A, B, BE, G, C, P, H> {
    pub fn new(
        client: Arc<C>,
        backend: Arc<mc_db::Backend<B>>,
        overrides: Arc<OverrideHandle<B>>,
        pool: Arc<P>,
        graph: Arc<Pool<A>>,
        sync_service: Arc<SyncingService<B>>,
        starting_block: <<B>::Header as HeaderT>::Number,
        genesis_provider: Arc<G>,
    ) -> Self {
        Self {
            client,
            backend,
            overrides,
            pool,
            graph,
            sync_service,
            starting_block,
            genesis_provider,
            _marker: PhantomData,
        }
    }
}

impl<A: ChainApi, B, BE, G, C, P, H> Starknet<A, B, BE, G, C, P, H>
where
    B: BlockT,
    C: HeaderBackend<B> + 'static,
{
    pub fn current_block_number(&self) -> RpcResult<u64> {
        Ok(UniqueSaturatedInto::<u64>::unique_saturated_into(self.client.info().best_number))
    }
}

impl<A: ChainApi, B, BE, G, C, P, H> Starknet<A, B, BE, G, C, P, H>
where
    B: BlockT,
    C: HeaderBackend<B> + 'static,
{
    pub fn current_spec_version(&self) -> RpcResult<String> {
        Ok("0.5.1".to_string())
    }
}

impl<A: ChainApi, B, BE, G, C, P, H> Starknet<A, B, BE, G, C, P, H>
where
    B: BlockT,
    C: HeaderBackend<B> + 'static,
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B>,
    H: HasherT + Send + Sync + 'static,
{
    pub fn current_block_hash(&self) -> Result<H256, StarknetRpcApiError> {
        let substrate_block_hash = self.client.info().best_hash;

        let starknet_block = match get_block_by_block_hash(self.client.as_ref(), substrate_block_hash) {
            Ok(block) => block,
            Err(_) => return Err(StarknetRpcApiError::BlockNotFound),
        };
        Ok(starknet_block.header().hash::<H>().into())
    }

    /// Returns the substrate block hash corresponding to the given Starknet block id
    fn substrate_block_hash_from_starknet_block(&self, block_id: BlockId) -> Result<B::Hash, StarknetRpcApiError> {
        match block_id {
            BlockId::Hash(h) => {
                madara_backend_client::load_hash(self.client.as_ref(), &self.backend, Felt252Wrapper::from(h).into())
                    .map_err(|e| {
                        error!("Failed to load Starknet block hash for Substrate block with hash '{h}': {e}");
                        StarknetRpcApiError::BlockNotFound
                    })?
            }
            BlockId::Number(n) => self
                .client
                .hash(UniqueSaturatedInto::unique_saturated_into(n))
                .map_err(|_| StarknetRpcApiError::BlockNotFound)?,
            BlockId::Tag(_) => Some(self.client.info().best_hash),
        }
        .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    /// Helper function to get the substrate block number from a Starknet block id
    ///
    /// # Arguments
    ///
    /// * `block_id` - The Starknet block id
    ///
    /// # Returns
    ///
    /// * `u64` - The substrate block number
    fn substrate_block_number_from_starknet_block(&self, block_id: BlockId) -> Result<u64, StarknetRpcApiError> {
        // Short circuit on block number
        if let BlockId::Number(x) = block_id {
            return Ok(x);
        }

        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id)?;

        let starknet_block = match get_block_by_block_hash(self.client.as_ref(), substrate_block_hash) {
            Ok(block) => block,
            Err(_) => return Err(StarknetRpcApiError::BlockNotFound),
        };

        Ok(starknet_block.header().block_number)
    }

    /// Returns a list of all transaction hashes in the given block.
    ///
    /// # Arguments
    ///
    /// * `block_hash` - The hash of the block containing the transactions (starknet block).
    fn get_cached_transaction_hashes(&self, block_hash: StarkHash) -> Option<Vec<StarkHash>> {
        self.backend.mapping().cached_transaction_hashes_from_block_hash(block_hash).unwrap_or_else(|err| {
            error!("Failed to read from cache: {err}");
            None
        })
    }

    /// Returns the state diff for the given block.
    ///
    /// # Arguments
    ///
    /// * `starknet_block_hash` - The hash of the block containing the state diff (starknet block).
    fn get_state_diff(&self, starknet_block_hash: &BlockHash) -> Result<StateDiff, StarknetRpcApiError> {
        let state_diff = self.backend.da().state_diff(starknet_block_hash).map_err(|e| {
            error!("Failed to retrieve state diff from cache for block with hash {}: {e}", starknet_block_hash);
            StarknetRpcApiError::InternalServerError
        })?;

        let rpc_state_diff = to_rpc_state_diff(state_diff);

        Ok(rpc_state_diff)
    }
}

#[async_trait]
impl<A, B, BE, G, C, P, H> StarknetWriteRpcApiServer for Starknet<A, B, BE, G, C, P, H>
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
    /// Submit a new declare transaction to be added to the chain
    ///
    /// # Arguments
    ///
    /// * `declare_transaction` - the declare transaction to be added to the chain
    ///
    /// # Returns
    ///
    /// * `declare_transaction_result` - the result of the declare transaction
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        let config = get_config().map_err(|e| {
            error!("Failed to get config: {e}");
            StarknetRpcApiError::InternalServerError
        })?;
        let sequencer = SequencerGatewayProvider::new(config.feeder_gateway, config.gateway, config.chain_id);

        let sequencer_response = match sequencer.add_declare_transaction(declare_transaction).await {
            Ok(response) => response,
            Err(ProviderError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => {
                error!("Failed to add invoke transaction to sequencer: {e}");
                return Err(StarknetRpcApiError::InternalServerError.into());
            }
        };

        Ok(sequencer_response)
    }

    /// Add an Invoke Transaction to invoke a contract function
    ///
    /// # Arguments
    ///
    /// * `invoke tx` - <https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/#invoke_transaction>
    ///
    /// # Returns
    ///
    /// * `transaction_hash` - transaction hash corresponding to the invocation
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        let config = get_config().map_err(|e| {
            error!("Failed to get config: {e}");
            StarknetRpcApiError::InternalServerError
        })?;
        let sequencer = SequencerGatewayProvider::new(config.feeder_gateway, config.gateway, config.chain_id);

        let sequencer_response = match sequencer.add_invoke_transaction(invoke_transaction).await {
            Ok(response) => response,
            Err(ProviderError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => {
                error!("Failed to add invoke transaction to sequencer: {e}");
                return Err(StarknetRpcApiError::InternalServerError.into());
            }
        };

        Ok(sequencer_response)
    }

    /// Add an Deploy Account Transaction
    ///
    /// # Arguments
    ///
    /// * `deploy account transaction` - <https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/#deploy_account_transaction>
    ///
    /// # Returns
    ///
    /// * `transaction_hash` - transaction hash corresponding to the invocation
    /// * `contract_address` - address of the deployed contract account
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        let config = get_config().map_err(|e| {
            error!("Failed to get config: {e}");
            StarknetRpcApiError::InternalServerError
        })?;
        let sequencer = SequencerGatewayProvider::new(config.feeder_gateway, config.gateway, config.chain_id);

        let sequencer_response = match sequencer.add_deploy_account_transaction(deploy_account_transaction).await {
            Ok(response) => response,
            Err(ProviderError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => {
                error!("Failed to add invoke transaction to sequencer: {e}");
                return Err(StarknetRpcApiError::InternalServerError.into());
            }
        };

        Ok(sequencer_response)
    }
}

#[async_trait]
#[allow(unused_variables)]
impl<A, B, BE, G, C, P, H> StarknetReadRpcApiServer for Starknet<A, B, BE, G, C, P, H>
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
    /// Get the Contract Class Definition at a Given Address in a Specific Block
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The identifier of the block. This can be the hash of the block, its number
    ///   (height), or a specific block tag.
    /// * `contract_address` - The address of the contract whose class definition will be returned.
    ///
    /// ### Returns
    ///
    /// * `contract_class` - The contract class definition. This may be either a standard contract
    ///   class or a deprecated contract class, depending on the contract's status and the
    ///   blockchain's version.
    ///
    /// ### Errors
    ///
    /// This method may return the following errors:
    /// * `BLOCK_NOT_FOUND` - If the specified block does not exist in the blockchain.
    /// * `CONTRACT_NOT_FOUND` - If the specified contract address does not exist.
    fn get_class_at(&self, block_id: BlockId, contract_address: FieldElement) -> RpcResult<ContractClass> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        let contract_address_wrapped = Felt252Wrapper(contract_address).into();

        let contract_class = self
            .overrides
            .for_block_hash(self.client.as_ref(), substrate_block_hash)
            .contract_class_by_address(substrate_block_hash, contract_address_wrapped)
            .ok_or_else(|| {
                error!("Failed to retrieve contract class at '{contract_address}'");
                StarknetRpcApiError::ContractNotFound
            })?;

        // Blockifier classes do not store ABI, has to be retrieved separately
        let contract_abi = self
            .overrides
            .for_block_hash(self.client.as_ref(), substrate_block_hash)
            .contract_abi_by_address(substrate_block_hash, contract_address_wrapped)
            .ok_or_else(|| {
                error!("Failed to retrieve contract ABI at '{contract_address}'");
                StarknetRpcApiError::ContractNotFound
            })?;

        // converting from stored Blockifier class to rpc class
        Ok(ContractClassWrapper { contract: contract_class, abi: contract_abi }.try_into().map_err(|e| {
            log::error!("Failed to convert contract class at address '{contract_address}' to RPC contract class: {e}");
            StarknetRpcApiError::InternalServerError
        })?)
    }

    /// Get the contract class hash in the given block for the contract deployed at the given
    /// address
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag
    /// * `contract_address` - The address of the contract whose class hash will be returned
    ///
    /// ### Returns
    ///
    /// * `class_hash` - The class hash of the given contract
    fn get_class_hash_at(&self, block_id: BlockId, contract_address: FieldElement) -> RpcResult<Felt> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        let contract_address = Felt252Wrapper(contract_address).into();
        let class_hash = self
            .overrides
            .for_block_hash(self.client.as_ref(), substrate_block_hash)
            .contract_class_hash_by_address(substrate_block_hash, contract_address)
            .ok_or_else(|| {
                error!("Failed to retrieve contract class hash at '{contract_address:?}'");
                StarknetRpcApiError::ContractNotFound
            })?;

        Ok(Felt(Felt252Wrapper::from(class_hash).into()))
    }

    /// Returns an object about the sync status, or false if the node is not synching
    ///
    /// ### Arguments
    ///
    /// This function does not take any arguments.
    ///
    /// ### Returns
    ///
    /// * `Syncing` - An Enum that can either be a `mc_rpc_core::SyncStatus` struct representing the
    ///   sync status, or a `Boolean` (`false`) indicating that the node is not currently
    ///   synchronizing.
    ///
    /// This is an asynchronous function due to its reliance on `sync_service.best_seen_block()`,
    /// which potentially involves network communication and processing to determine the best block
    /// seen by the sync service.
    async fn syncing(&self) -> RpcResult<SyncStatusType> {
        // obtain best seen (highest) block number
        match self.sync_service.best_seen_block().await {
            Ok(best_seen_block) => {
                let best_number = self.client.info().best_number;
                let highest_number = best_seen_block.unwrap_or(best_number);

                // get a starknet block from the starting substrate block number
                let starting_block = madara_backend_client::starknet_block_from_substrate_hash(
                    self.client.as_ref(),
                    self.starting_block,
                );

                // get a starknet block from the current substrate block number
                let current_block =
                    madara_backend_client::starknet_block_from_substrate_hash(self.client.as_ref(), best_number);

                // get a starknet block from the highest substrate block number
                let highest_block =
                    madara_backend_client::starknet_block_from_substrate_hash(self.client.as_ref(), highest_number);

                if starting_block.is_ok() && current_block.is_ok() && highest_block.is_ok() {
                    // Convert block numbers and hashes to the respective type required by the `syncing` endpoint.
                    let starting_block_num = UniqueSaturatedInto::<u64>::unique_saturated_into(self.starting_block);
                    let starting_block_hash = starting_block?.header().hash::<H>().0;

                    let current_block_num = UniqueSaturatedInto::<u64>::unique_saturated_into(best_number);
                    let current_block_hash = current_block?.header().hash::<H>().0;

                    // Get the highest block number and hash from the global variable update in l2 sync()
                    let (highest_block_hash, highest_block_num) = get_highest_block_hash_and_number();

                    // Build the `SyncStatus` struct with the respective syn information
                    Ok(SyncStatusType::Syncing(SyncStatus {
                        starting_block_num,
                        starting_block_hash,
                        current_block_num,
                        current_block_hash,
                        highest_block_num,
                        highest_block_hash,
                    }))
                } else {
                    // If there was an error when getting a starknet block, then we return `false`,
                    // as per the endpoint specification
                    log::error!("Failed to load Starknet block");
                    Ok(SyncStatusType::NotSyncing)
                }
            }
            Err(_) => {
                // If there was an error when getting a starknet block, then we return `false`,
                // as per the endpoint specification
                log::error!("`SyncingEngine` shut down");
                Ok(SyncStatusType::NotSyncing)
            }
        }
    }

    /// Get the contract class definition in the given block associated with the given hash.
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag.
    /// * `class_hash` - The hash of the requested contract class.
    ///
    /// ### Returns
    ///
    /// Returns the contract class definition if found. In case of an error, returns a
    /// `StarknetRpcApiError` indicating either `BlockNotFound` or `ClassHashNotFound`.
    fn get_class(&self, block_id: BlockId, class_hash: FieldElement) -> RpcResult<ContractClass> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        let class_hash = Felt252Wrapper(class_hash).into();

        let contract_class = self
            .overrides
            .for_block_hash(self.client.as_ref(), substrate_block_hash)
            .contract_class_by_class_hash(substrate_block_hash, class_hash)
            .ok_or_else(|| {
                error!("Failed to retrieve contract class from hash '{class_hash}'");
                StarknetRpcApiError::ClassHashNotFound
            })?;

        // Blockifier classes do not store ABI, has to be retrieved separately
        let contract_abi = self
            .overrides
            .for_block_hash(self.client.as_ref(), substrate_block_hash)
            .contract_abi_by_class_hash(substrate_block_hash, class_hash)
            .ok_or_else(|| {
                error!("Failed to retrieve contract ABI from hash '{class_hash}'");
                StarknetRpcApiError::ClassHashNotFound
            })?;

        // converting from stored Blockifier class to rpc class
        Ok(ContractClassWrapper { contract: contract_class, abi: contract_abi }.try_into().map_err(|e| {
            error!("Failed to convert contract class from hash '{class_hash}' to RPC contract class: {e}");
            StarknetRpcApiError::InternalServerError
        })?)
    }

    /// Get block information with transaction hashes given the block id.
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag.
    ///
    /// ### Returns
    ///
    /// Returns block information with transaction hashes. This includes either a confirmed block or
    /// a pending block with transaction hashes, depending on the state of the requested block.
    /// In case the block is not found, returns a `StarknetRpcApiError` with `BlockNotFound`.
    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes> {
        let chain_id = self.chain_id()?;
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        match block_id {
            BlockId::Tag(BlockTag::Pending) => get_block_with_tx_hashes_pending::<H>(chain_id),
            _ => get_block_with_tx_hashes_finalized(self, chain_id, substrate_block_hash),
        }
    }

    /// Get the nonce associated with the given address in the given block.
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag. This parameter specifies the block in which the nonce is to be checked.
    /// * `contract_address` - The address of the contract whose nonce we're seeking. This is the
    ///   unique identifier of the contract in the Starknet network.
    ///
    /// ### Returns
    ///
    /// Returns the contract's nonce at the requested state. The nonce is returned as a
    /// `FieldElement`, representing the current state of the contract in terms of transactions
    /// count or other contract-specific operations. In case of errors, such as
    /// `BLOCK_NOT_FOUND` or `CONTRACT_NOT_FOUND`, returns a `StarknetRpcApiError` indicating the
    /// specific issue.
    fn get_nonce(&self, block_id: BlockId, contract_address: FieldElement) -> RpcResult<Felt> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        let contract_address = Felt252Wrapper(contract_address).into();

        let nonce = self
            .overrides
            .for_block_hash(self.client.as_ref(), substrate_block_hash)
            .nonce(substrate_block_hash, contract_address)
            .ok_or_else(|| {
                error!("Failed to get nonce at '{contract_address:?}'");
                StarknetRpcApiError::ContractNotFound
            })?;

        Ok(Felt(Felt252Wrapper::from(nonce).into()))
    }

    /// Return the currently configured chain id.
    ///
    /// This function provides the chain id for the network that the node is connected to. The chain
    /// id is a unique identifier that distinguishes between different networks, such as mainnet or
    /// testnet.
    ///
    /// ### Arguments
    ///
    /// This function does not take any arguments.
    ///
    /// ### Returns
    ///
    /// Returns the chain id this node is connected to. The chain id is returned as a specific type,
    /// defined by the Starknet protocol, indicating the particular network.
    fn chain_id(&self) -> RpcResult<Felt> {
        let best_block_hash = self.client.info().best_hash;
        let chain_id = get_config()
            .map_err(|e| {
                error!("Failed to get config: {e}");
                StarknetRpcApiError::InternalServerError
            })?
            .chain_id;

        Ok(Felt(chain_id))
    }

    /// Estimate the fee associated with transaction
    ///
    /// # Arguments
    ///
    /// * `request` - starknet transaction request
    /// * `block_id` - hash of the requested block, number (height), or tag
    ///
    /// # Returns
    ///
    /// * `fee_estimate` - fee estimate in gwei
    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTransaction>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;
        let best_block_hash = self.client.info().best_hash;
        let chain_id = Felt252Wrapper(self.chain_id()?.0);

        let transactions =
            request.into_iter().map(|tx| tx.try_into()).collect::<Result<Vec<UserTransaction>, _>>().map_err(|e| {
                error!("Failed to convert BroadcastedTransaction to UserTransaction: {e}");
                StarknetRpcApiError::InternalServerError
            })?;

        let fee_estimates = self
            .client
            .runtime_api()
            .estimate_fee(substrate_block_hash, transactions)
            .map_err(|e| {
                error!("Request parameters error: {e}");
                StarknetRpcApiError::InternalServerError
            })?
            .map_err(|e| {
                error!("Failed to call function: {:#?}", e);
                StarknetRpcApiError::ContractError
            })?;

        let estimates = fee_estimates
            .into_iter()
			// FIXME: https://github.com/keep-starknet-strange/madara/issues/329
            .map(|x| FeeEstimate { gas_price: 10, gas_consumed: x.1, overall_fee: x.0 })
            .collect();

        Ok(estimates)
    }

    /// Estimate the L2 fee of a message sent on L1
    ///
    /// # Arguments
    ///
    /// * `message` - the message to estimate
    /// * `block_id` - hash, number (height), or tag of the requested block
    ///
    /// # Returns
    ///
    /// * `FeeEstimate` - the fee estimation (gas consumed, gas price, overall fee, unit)
    ///
    /// # Errors
    ///
    /// BlockNotFound : If the specified block does not exist.
    /// ContractNotFound : If the specified contract address does not exist.
    /// ContractError : If there is an error with the contract.
    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;
        let chain_id = Felt252Wrapper(self.chain_id()?.0);

        let message = message.try_into().map_err(|e| {
            error!("Failed to convert MsgFromL1 to UserTransaction: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

        let fee_estimate = self
            .client
            .runtime_api()
            .estimate_message_fee(substrate_block_hash, message)
            .map_err(|e| {
                error!("Runtime api error: {e}");
                StarknetRpcApiError::InternalServerError
            })?
            .map_err(|e| {
                error!("function execution failed: {:#?}", e);
                StarknetRpcApiError::ContractError
            })?;

        let estimate = FeeEstimate {
            gas_price: fee_estimate.0.try_into().map_err(|_| StarknetRpcApiError::InternalServerError)?,
            gas_consumed: fee_estimate.2,
            overall_fee: fee_estimate.1,
        };

        Ok(estimate)
    }

    /// Get the details of a transaction by a given block id and index.
    ///
    /// This function fetches the details of a specific transaction in the StarkNet network by
    /// identifying it through its block and position (index) within that block. If no transaction
    /// is found at the specified index, null is returned.
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag. This parameter is used to specify the block in which the transaction is
    ///   located.
    /// * `index` - An integer representing the index in the block where the transaction is expected
    ///   to be found. The index starts from 0 and increases sequentially for each transaction in
    ///   the block.
    ///
    /// ### Returns
    ///
    /// Returns the details of the transaction if found, including the transaction hash. The
    /// transaction details are returned as a type conforming to the StarkNet protocol. In case of
    /// errors like `BLOCK_NOT_FOUND` or `INVALID_TXN_INDEX`, returns a `StarknetRpcApiError`
    /// indicating the specific issue.
    fn get_transaction_by_block_id_and_index(&self, block_id: BlockId, index: u64) -> RpcResult<Transaction> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        let starknet_block = get_block_by_block_hash(self.client.as_ref(), substrate_block_hash)?;

        let transaction =
            starknet_block.transactions().get(index as usize).ok_or(StarknetRpcApiError::InvalidTxnIndex)?;
        let chain_id = self.chain_id()?;

        let opt_cached_transaction_hashes =
            self.get_cached_transaction_hashes(starknet_block.header().hash::<H>().into());

        let transaction_hash = if let Some(cached_tx_hashes) = opt_cached_transaction_hashes {
            cached_tx_hashes.get(index as usize).map(|&fe| FieldElement::from(Felt252Wrapper::from(fe))).ok_or(
                CallError::Failed(anyhow::anyhow!(
                    "Number of cached tx hashes does not match the number of transactions in block with id {:?}",
                    block_id
                )),
            )?
        } else {
            transaction.compute_hash::<H>(chain_id.0.into(), false, Some(starknet_block.header().block_number)).0
        };

        Ok(to_starknet_core_tx(transaction.clone(), transaction_hash))
    }

    /// Get block information with full transactions given the block id.
    ///
    /// This function retrieves detailed information about a specific block in the StarkNet network,
    /// including all transactions contained within that block. The block is identified using its
    /// unique block id, which can be the block's hash, its number (height), or a block tag.
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag. This parameter is used to specify the block from which to retrieve
    ///   information and transactions.
    ///
    /// ### Returns
    ///
    /// Returns detailed block information along with full transactions. Depending on the state of
    /// the block, this can include either a confirmed block or a pending block with its
    /// transactions. In case the specified block is not found, returns a `StarknetRpcApiError` with
    /// `BlockNotFound`.
    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs> {
        let chain_id = self.chain_id()?;
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("Block not found: '{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        match block_id {
            BlockId::Tag(BlockTag::Pending) => get_block_with_txs_pending::<H>(chain_id),
            _ => get_block_with_txs_finalized(self, chain_id, substrate_block_hash),
        }
    }

    /// Get the information about the result of executing the requested block.
    ///
    /// This function fetches details about the state update resulting from executing a specific
    /// block in the StarkNet network. The block is identified using its unique block id, which can
    /// be the block's hash, its number (height), or a block tag.
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag. This parameter specifies the block for which the state update information
    ///   is required.
    ///
    /// ### Returns
    ///
    /// Returns information about the state update of the requested block, including any changes to
    /// the state of the network as a result of the block's execution. This can include a confirmed
    /// state update or a pending state update. If the block is not found, returns a
    /// `StarknetRpcApiError` with `BlockNotFound`.
    fn get_state_update(&self, block_id: BlockId) -> RpcResult<MaybePendingStateUpdate> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        match block_id {
            BlockId::Tag(BlockTag::Pending) => get_state_update_pending(),
            _ => get_state_update_finalized(self, substrate_block_hash),
        }
    }

    /// Returns all events matching the given filter.
    ///
    /// This function retrieves all event objects that match the conditions specified in the
    /// provided event filter. The filter can include various criteria such as contract addresses,
    /// event types, and block ranges. The function supports pagination through the result page
    /// request schema.
    ///
    /// ### Arguments
    ///
    /// * `filter` - The conditions used to filter the returned events. The filter is a combination
    ///   of an event filter and a result page request, allowing for precise control over which
    ///   events are returned and in what quantity.
    ///
    /// ### Returns
    ///
    /// Returns a chunk of event objects that match the filter criteria, encapsulated in an
    /// `EventsChunk` type. The chunk includes details about the events, such as their data, the
    /// block in which they occurred, and the transaction that triggered them. In case of
    /// errors, such as `PAGE_SIZE_TOO_BIG`, `INVALID_CONTINUATION_TOKEN`, `BLOCK_NOT_FOUND`, or
    /// `TOO_MANY_KEYS_IN_FILTER`, returns a `StarknetRpcApiError` indicating the specific issue.
    async fn get_events(&self, filter: EventFilterWithPage) -> RpcResult<EventsPage> {
        let from_address = filter.event_filter.address.map(Felt252Wrapper::from);
        let keys = filter.event_filter.keys.unwrap_or_default();
        let chunk_size = filter.result_page_request.chunk_size;

        if keys.len() > MAX_EVENTS_KEYS {
            return Err(StarknetRpcApiError::TooManyKeysInFilter.into());
        }
        if chunk_size > MAX_EVENTS_CHUNK_SIZE as u64 {
            return Err(StarknetRpcApiError::PageSizeTooBig.into());
        }

        // Get the substrate block numbers for the requested range
        let latest_block =
            self.substrate_block_number_from_starknet_block(BlockId::Tag(BlockTag::Latest)).map_err(|e| {
                error!("'{e}'");
                StarknetRpcApiError::BlockNotFound
            })?;
        let from_block = self
            .substrate_block_number_from_starknet_block(filter.event_filter.from_block.unwrap_or(BlockId::Number(0)))
            .map_err(|e| {
                error!("'{e}'");
                StarknetRpcApiError::BlockNotFound
            })?;
        let to_block = self
            .substrate_block_number_from_starknet_block(
                filter.event_filter.to_block.unwrap_or(BlockId::Tag(BlockTag::Latest)),
            )
            .map_err(|e| {
                error!("'{e}'");
                StarknetRpcApiError::BlockNotFound
            })?;

        let continuation_token = match filter.result_page_request.continuation_token {
            Some(token) => types::ContinuationToken::parse(token).map_err(|e| {
                error!("Failed to parse continuation token: {:?}", e);
                StarknetRpcApiError::InvalidContinuationToken
            })?,
            None => types::ContinuationToken { block_n: from_block, event_n: 0 },
        };

        // Verify that the requested range is valid
        if from_block > to_block {
            return Ok(EventsPage { events: vec![], continuation_token: None });
        }

        let to_block = if latest_block > to_block { to_block } else { latest_block };
        let filter = RpcEventFilter { from_block, to_block, from_address, keys, chunk_size, continuation_token };

        self.filter_events(filter)
    }

    /// Get the details and status of a submitted transaction.
    ///
    /// This function retrieves the detailed information and status of a transaction identified by
    /// its hash. The transaction hash uniquely identifies a specific transaction that has been
    /// submitted to the StarkNet network.
    ///
    /// ### Arguments
    ///
    /// * `transaction_hash` - The hash of the requested transaction. This parameter specifies the
    ///   transaction for which details and status are requested.
    ///
    /// ### Returns
    ///
    /// Returns information about the requested transaction, including its status, sender,
    /// recipient, and other transaction details. The information is encapsulated in a `Transaction`
    /// type, which is a combination of the `TXN` schema and additional properties, such as the
    /// `transaction_hash`. In case the specified transaction hash is not found, returns a
    /// `StarknetRpcApiError` with `TXN_HASH_NOT_FOUND`.
    ///
    /// ### Errors
    ///
    /// The function may return one of the following errors if encountered:
    /// - `PAGE_SIZE_TOO_BIG` if the requested page size exceeds the allowed limit.
    /// - `INVALID_CONTINUATION_TOKEN` if the provided continuation token is invalid or expired.
    /// - `BLOCK_NOT_FOUND` if the specified block is not found.
    /// - `TOO_MANY_KEYS_IN_FILTER` if there are too many keys in the filter, which may exceed the
    ///   system's capacity.
    fn get_transaction_by_hash(&self, transaction_hash: FieldElement) -> RpcResult<Transaction> {
        let substrate_block_hash_from_db = self
            .backend
            .mapping()
            .block_hash_from_transaction_hash(Felt252Wrapper::from(transaction_hash).into())
            .map_err(|e| {
                error!("Failed to get transaction's substrate block hash from mapping_db: {e}");
                StarknetRpcApiError::TxnHashNotFound
            })?;

        let substrate_block_hash = match substrate_block_hash_from_db {
            Some(block_hash) => block_hash,
            None => return Err(StarknetRpcApiError::TxnHashNotFound.into()),
        };

        let starknet_block = get_block_by_block_hash(self.client.as_ref(), substrate_block_hash)?;

        let chain_id = self.chain_id()?.0.into();

        let find_tx =
            if let Some(tx_hashes) = self.get_cached_transaction_hashes(starknet_block.header().hash::<H>().into()) {
                tx_hashes
                    .into_iter()
                    .zip(starknet_block.transactions())
                    .find(|(tx_hash, _)| *tx_hash == Felt252Wrapper(transaction_hash).into())
                    .map(|(_, tx)| to_starknet_core_tx(tx.clone(), transaction_hash))
            } else {
                starknet_block
                    .transactions()
                    .iter()
                    .find(|tx| {
                        tx.compute_hash::<H>(chain_id, false, Some(starknet_block.header().block_number)).0
                            == transaction_hash
                    })
                    .map(|tx| to_starknet_core_tx(tx.clone(), transaction_hash))
            };

        find_tx.ok_or(StarknetRpcApiError::TxnHashNotFound.into())
    }

    /// Get the transaction receipt by the transaction hash.
    ///
    /// This function retrieves the transaction receipt for a specific transaction identified by its
    /// hash. The transaction receipt includes information about the execution status of the
    /// transaction, events generated during its execution, and other relevant details.
    ///
    /// ### Arguments
    ///
    /// * `transaction_hash` - The hash of the requested transaction. This parameter specifies the
    ///   transaction for which the receipt is requested.
    ///
    /// ### Returns
    ///
    /// Returns a transaction receipt, which can be one of two types:
    /// - `TransactionReceipt` if the transaction has been processed and has a receipt.
    /// - `PendingTransactionReceipt` if the transaction is pending and the receipt is not yet
    ///   available.
    ///
    /// ### Errors
    ///
    /// The function may return a `TXN_HASH_NOT_FOUND` error if the specified transaction hash is
    /// not found.
    async fn get_transaction_receipt(
        &self,
        transaction_hash: FieldElement,
    ) -> RpcResult<MaybePendingTransactionReceipt> {
        let substrate_block_hash = self
            .backend
            .mapping()
            .block_hash_from_transaction_hash(Felt252Wrapper::from(transaction_hash).into())
            .map_err(|e| {
                log::error!("Failed to retrieve substrate block hash: {e}");
                StarknetRpcApiError::InternalServerError
            })?;

        let chain_id = self.chain_id()?;

        match substrate_block_hash {
            Some(substrate_block_hash) => {
                get_transaction_receipt_finalized(self, chain_id, substrate_block_hash, transaction_hash)
            }
            None => {
                let substrate_block_hash =
                    self.substrate_block_hash_from_starknet_block(BlockId::Tag(BlockTag::Latest)).map_err(|e| {
                        error!("'{e}'");
                        StarknetRpcApiError::BlockNotFound
                    })?;

                get_transaction_receipt_pending(self, chain_id, substrate_block_hash, transaction_hash)
            }
        }
    }
}
