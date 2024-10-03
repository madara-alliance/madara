use super::v0_7_1::StarknetReadRpcApiV0_7_1Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::proc_macros::rpc;
use m_proc_macros::versioned_starknet_rpc;
use serde::Serialize;
use starknet_core::types::{
    BlockHashAndNumber, BlockId, BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction,
    BroadcastedInvokeTransaction, BroadcastedTransaction, ContractClass, DeclareTransactionResult,
    DeployAccountTransactionResult, EventFilterWithPage, EventsPage, FeeEstimate, FunctionCall,
    InvokeTransactionResult, MaybePendingBlockWithReceipts, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs,
    MaybePendingStateUpdate, MsgFromL1, SimulatedTransaction, SimulationFlag, SimulationFlagForEstimateFee,
    SyncStatusType, Transaction, TransactionReceiptWithBlockInfo, TransactionStatus, TransactionTraceWithHash,
};
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

mod get_storage_proof;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProofNode {
    Binary { left: Felt, right: Felt },
    Edge { child: Felt, path: Felt, length: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GetStorageProofResult {
    pub classes_proof: HashMap<Felt, ProofNode>,
    pub contracts_proof: HashMap<Felt, ProofNode>,
    pub contracts_storage_proofs: HashMap<Felt, HashMap<Felt, ProofNode>>,
}

// Starknet RPC API trait and types
//
// Starkware maintains [a description of the Starknet API](https://github.com/starkware-libs/starknet-specs/blob/master/api/starknet_api_openrpc.json)
// using the openRPC specification.
// This crate uses `jsonrpsee` to define such an API in Rust terms.

/// Starknet write rpc interface.
#[versioned_starknet_rpc("V0_8_0")]
pub trait StarknetWriteRpcApi {
    /// Submit a new transaction to be added to the chain
    #[method(name = "addInvokeTransaction")]
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult>;

    /// Submit a new class declaration transaction
    #[method(name = "addDeployAccountTransaction")]
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult>;

    /// Submit a new deploy account transaction
    #[method(name = "addDeclareTransaction")]
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult>;
}

#[versioned_starknet_rpc("V0_8_0")]
pub trait StarknetReadRpcApi {
    /// Get the Version of the StarkNet JSON-RPC Specification Being Used
    #[method(name = "specVersion")]
    fn spec_version(&self) -> RpcResult<String>;

    /// Get the most recent accepted block number
    #[method(name = "blockNumber")]
    fn block_number(&self) -> RpcResult<u64>;

    // Get the most recent accepted block hash and number
    #[method(name = "blockHashAndNumber")]
    fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumber>;

    /// Call a contract function at a given block id
    #[method(name = "call")]
    fn call(&self, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<Felt>>;

    /// Get the chain id
    #[method(name = "chainId")]
    fn chain_id(&self) -> RpcResult<Felt>;

    /// Get the number of transactions in a block given a block id
    #[method(name = "getBlockTransactionCount")]
    fn get_block_transaction_count(&self, block_id: BlockId) -> RpcResult<u128>;

    /// Estimate the fee associated with transaction
    #[method(name = "estimateFee")]
    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTransaction>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>>;

    /// Estimate the L2 fee of a message sent on L1
    #[method(name = "estimateMessageFee")]
    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate>;

    /// Get block information with full transactions and receipts given the block id
    #[method(name = "getBlockWithReceipts")]
    async fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithReceipts>;

    /// Get block information with transaction hashes given the block id
    #[method(name = "getBlockWithTxHashes")]
    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes>;

    /// Get block information with full transactions given the block id
    #[method(name = "getBlockWithTxs")]
    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs>;

    /// Get the contract class at a given contract address for a given block id
    #[method(name = "getClassAt")]
    fn get_class_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<ContractClass>;

    /// Get the contract class hash in the given block for the contract deployed at the given
    /// address
    #[method(name = "getClassHashAt")]
    fn get_class_hash_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt>;

    /// Get the contract class definition in the given block associated with the given hash
    #[method(name = "getClass")]
    fn get_class(&self, block_id: BlockId, class_hash: Felt) -> RpcResult<ContractClass>;

    /// Returns all events matching the given filter
    #[method(name = "getEvents")]
    async fn get_events(&self, filter: EventFilterWithPage) -> RpcResult<EventsPage>;

    /// Get the nonce associated with the given address at the given block
    #[method(name = "getNonce")]
    fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt>;

    /// Get the value of the storage at the given address and key, at the given block id
    #[method(name = "getStorageAt")]
    fn get_storage_at(&self, contract_address: Felt, key: Felt, block_id: BlockId) -> RpcResult<Felt>;

    /// Get the details of a transaction by a given block id and index
    #[method(name = "getTransactionByBlockIdAndIndex")]
    fn get_transaction_by_block_id_and_index(&self, block_id: BlockId, index: u64) -> RpcResult<Transaction>;

    /// Returns the information about a transaction by transaction hash.
    #[method(name = "getTransactionByHash")]
    fn get_transaction_by_hash(&self, transaction_hash: Felt) -> RpcResult<Transaction>;

    /// Returns the receipt of a transaction by transaction hash.
    #[method(name = "getTransactionReceipt")]
    async fn get_transaction_receipt(&self, transaction_hash: Felt) -> RpcResult<TransactionReceiptWithBlockInfo>;

    /// Gets the Transaction Status, Including Mempool Status and Execution Details
    #[method(name = "getTransactionStatus")]
    fn get_transaction_status(&self, transaction_hash: Felt) -> RpcResult<TransactionStatus>;

    /// Get an object about the sync status, or false if the node is not syncing
    #[method(name = "syncing")]
    async fn syncing(&self) -> RpcResult<SyncStatusType>;

    /// Get the information about the result of executing the requested block
    #[method(name = "getStateUpdate")]
    fn get_state_update(&self, block_id: BlockId) -> RpcResult<MaybePendingStateUpdate>;

    /// Get merkle paths in one of the state tries: global state, classes, individual contract
    #[method(name = "getStorageProof")]
    fn get_storage_proof(
        &self,
        block_id: Option<BlockId>,
        class_hashes: Vec<Felt>,
        contract_addresses: Vec<Felt>,
        contracts_storage_keys: HashMap<Felt, Vec<Felt>>,
    ) -> RpcResult<GetStorageProofResult>;
}

#[versioned_starknet_rpc("V0_8_0")]
pub trait StarknetTraceRpcApi {
    /// Returns the execution trace of a transaction by simulating it in the runtime.
    #[method(name = "simulateTransactions")]
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTransaction>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulatedTransaction>>;

    #[method(name = "traceBlockTransactions")]
    /// Returns the execution traces of all transactions included in the given block
    async fn trace_block_transactions(&self, block_id: BlockId) -> RpcResult<Vec<TransactionTraceWithHash>>;

    #[method(name = "traceTransaction")]
    /// Returns the execution trace of a transaction
    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TransactionTraceWithHash>;
}

#[async_trait]
impl StarknetReadRpcApiV0_8_0Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok("0.8.0".into())
    }
    fn block_number(&self) -> RpcResult<u64> {
        StarknetReadRpcApiV0_7_1Server::block_number(self)
    }
    fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumber> {
        StarknetReadRpcApiV0_7_1Server::block_hash_and_number(self)
    }
    fn call(&self, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<Felt>> {
        StarknetReadRpcApiV0_7_1Server::call(self, request, block_id)
    }
    fn chain_id(&self) -> RpcResult<Felt> {
        StarknetReadRpcApiV0_7_1Server::chain_id(self)
    }
    fn get_block_transaction_count(&self, block_id: BlockId) -> RpcResult<u128> {
        StarknetReadRpcApiV0_7_1Server::get_block_transaction_count(self, block_id)
    }
    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTransaction>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>> {
        StarknetReadRpcApiV0_7_1Server::estimate_fee(self, request, simulation_flags, block_id).await
    }
    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate> {
        StarknetReadRpcApiV0_7_1Server::estimate_message_fee(self, message, block_id).await
    }
    async fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithReceipts> {
        StarknetReadRpcApiV0_7_1Server::get_block_with_receipts(self, block_id).await
    }
    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes> {
        StarknetReadRpcApiV0_7_1Server::get_block_with_tx_hashes(self, block_id)
    }
    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs> {
        StarknetReadRpcApiV0_7_1Server::get_block_with_txs(self, block_id)
    }
    fn get_class_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<ContractClass> {
        StarknetReadRpcApiV0_7_1Server::get_class_at(self, block_id, contract_address)
    }
    fn get_class_hash_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        StarknetReadRpcApiV0_7_1Server::get_class_hash_at(self, block_id, contract_address)
    }
    fn get_class(&self, block_id: BlockId, class_hash: Felt) -> RpcResult<ContractClass> {
        StarknetReadRpcApiV0_7_1Server::get_class(self, block_id, class_hash)
    }
    async fn get_events(&self, filter: EventFilterWithPage) -> RpcResult<EventsPage> {
        StarknetReadRpcApiV0_7_1Server::get_events(self, filter).await
    }
    fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt> {
        StarknetReadRpcApiV0_7_1Server::get_nonce(self, block_id, contract_address)
    }
    fn get_storage_at(&self, contract_address: Felt, key: Felt, block_id: BlockId) -> RpcResult<Felt> {
        StarknetReadRpcApiV0_7_1Server::get_storage_at(self, contract_address, key, block_id)
    }
    fn get_transaction_by_block_id_and_index(&self, block_id: BlockId, index: u64) -> RpcResult<Transaction> {
        StarknetReadRpcApiV0_7_1Server::get_transaction_by_block_id_and_index(self, block_id, index)
    }
    fn get_transaction_by_hash(&self, transaction_hash: Felt) -> RpcResult<Transaction> {
        StarknetReadRpcApiV0_7_1Server::get_transaction_by_hash(self, transaction_hash)
    }
    async fn get_transaction_receipt(&self, transaction_hash: Felt) -> RpcResult<TransactionReceiptWithBlockInfo> {
        StarknetReadRpcApiV0_7_1Server::get_transaction_receipt(self, transaction_hash).await
    }
    fn get_transaction_status(&self, transaction_hash: Felt) -> RpcResult<TransactionStatus> {
        StarknetReadRpcApiV0_7_1Server::get_transaction_status(self, transaction_hash)
    }
    async fn syncing(&self) -> RpcResult<SyncStatusType> {
        StarknetReadRpcApiV0_7_1Server::syncing(self).await
    }
    fn get_state_update(&self, block_id: BlockId) -> RpcResult<MaybePendingStateUpdate> {
        StarknetReadRpcApiV0_7_1Server::get_state_update(self, block_id)
    }

    fn get_storage_proof(
        &self,
        block_id: Option<BlockId>,
        class_hashes: Vec<Felt>,
        contract_addresses: Vec<Felt>,
        contracts_storage_keys: HashMap<Felt, Vec<Felt>>,
    ) -> RpcResult<GetStorageProofResult> {
        get_storage_proof::get_storage_proof(self, block_id, class_hashes, contract_addresses, contracts_storage_keys)
    }
}

#[async_trait]
impl StarknetWriteRpcApiV0_8_0Server for Starknet {
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        StarknetWriteRpcApiV0_8_0Server::add_declare_transaction(self, declare_transaction).await
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        StarknetWriteRpcApiV0_8_0Server::add_deploy_account_transaction(self, deploy_account_transaction).await
    }
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        StarknetWriteRpcApiV0_8_0Server::add_invoke_transaction(self, invoke_transaction).await
    }
}
#[async_trait]
impl StarknetTraceRpcApiV0_8_0Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTransaction>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulatedTransaction>> {
        StarknetTraceRpcApiV0_8_0Server::simulate_transactions(self, block_id, transactions, simulation_flags).await
    }

    async fn trace_block_transactions(&self, block_id: BlockId) -> RpcResult<Vec<TransactionTraceWithHash>> {
        StarknetTraceRpcApiV0_8_0Server::trace_block_transactions(self, block_id).await
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TransactionTraceWithHash> {
        StarknetTraceRpcApiV0_8_0Server::trace_transaction(self, transaction_hash).await
    }
}
