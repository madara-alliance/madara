use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use starknet_core::types::{
    BlockHashAndNumber, BlockId, BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction,
    BroadcastedInvokeTransaction, BroadcastedTransaction, ContractClass, DeclareTransactionResult,
    DeployAccountTransactionResult, EventFilterWithPage, EventsPage, FeeEstimate, FunctionCall,
    InvokeTransactionResult, MaybePendingBlockWithReceipts, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs,
    MaybePendingStateUpdate, MsgFromL1, SimulatedTransaction, SimulationFlag, SimulationFlagForEstimateFee,
    SyncStatusType, Transaction, TransactionReceiptWithBlockInfo, TransactionStatus, TransactionTraceWithHash,
};
use starknet_types_core::felt::Felt;

use m_proc_macros::versioned_starknet_rpc;

// Starknet RPC API trait and types
//
// Starkware maintains [a description of the Starknet API](https://github.com/starkware-libs/starknet-specs/blob/master/api/starknet_api_openrpc.json)
// using the openRPC specification.
// This crate uses `jsonrpsee` to define such an API in Rust terms.

/// Starknet write rpc interface.
#[versioned_starknet_rpc("V0_7_1")]
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

#[versioned_starknet_rpc("V0_7_1")]
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
}

#[versioned_starknet_rpc("V0_7_1")]
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
