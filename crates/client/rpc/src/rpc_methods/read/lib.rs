use std::marker::PhantomData;
use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::error::CallError;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
pub use mc_rpc_core::{Felt, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
use mc_storage::OverrideHandle;
use mc_sync::utility::get_config;
use mp_contract::class::ContractClassWrapper;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use mp_transactions::{TransactionStatus, UserTransaction};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
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
use sp_runtime::DispatchError;
use starknet_api::block::BlockHash;
use starknet_api::hash::StarkHash;
use starknet_api::transaction::Calldata;
use starknet_core::types::{
    BlockHashAndNumber, BlockId, BlockTag, BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction,
    BroadcastedInvokeTransaction, BroadcastedTransaction, ContractClass, DeclareTransactionResult,
    DeployAccountTransactionResult, EventFilterWithPage, EventsPage, FeeEstimate, FieldElement, FunctionCall,
    InvokeTransactionResult, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, MaybePendingStateUpdate,
    MaybePendingTransactionReceipt, MsgFromL1, StateDiff, SyncStatus, SyncStatusType, Transaction,
    TransactionExecutionStatus, TransactionFinalityStatus,
};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};

use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::rpc_methods::get_block::{
    get_block_with_tx_hashes_finalized, get_block_with_tx_hashes_pending, get_block_with_txs_finalized,
    get_block_with_txs_pending,
};
use crate::rpc_methods::get_transaction_receipt::{get_transaction_receipt_finalized, get_transaction_receipt_pending};
use crate::types::RpcEventFilter;
use crate::Starknet;
use super::block_hash_and_number::{self, block_hash_and_number};
use super::call::*;
use super::chain_id::*;
use super::get_block_transaction_count::*;
use super::estimate_fee::*;
use super::estimate_message_fee::*;
use super::get_block_with_tx_hashes::*;
use super::get_block_with_txs::*;
use super::get_class_at::*;
use super::get_class_hash_at::*;

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
    fn block_number(&self) -> RpcResult<u64> {
        self.current_block_number()
    }

    fn spec_version(&self) -> RpcResult<String> {
        self.current_spec_version()
    }

    fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumber> {
        block_hash_and_number(self)
    }

    fn call(&self, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<String>> {
        call(self, request, block_id)
    }

    fn chain_id(&self) -> RpcResult<Felt> {
        chain_id(self)
    }

    fn get_block_transaction_count(&self, block_id: BlockId) -> RpcResult<u128> {
        get_block_transaction_count(self, block_id)
    }

    async fn estimate_fee(&self, request: Vec<BroadcastedTransaction>, block_id: BlockId) -> RpcResult<Vec<FeeEstimate>> {
        estimate_fee(self, request, block_id).await
    }

    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate> {
        estimate_message_fee(self, message, block_id).await
    }

    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes> {
        get_block_with_tx_hashes(self, block_id)
    }

    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs> {
        get_block_with_txs(self, block_id)
    }

    fn get_class_at(&self, block_id: BlockId, contract_address: FieldElement) -> RpcResult<ContractClass> {
        get_class_at(self, block_id, contract_address)
    }

    fn get_class_hash_at(&self, block_id: BlockId, contract_address: FieldElement) -> RpcResult<Felt> {
        get_class_hash_at(self, block_id, contract_address)
    }
}