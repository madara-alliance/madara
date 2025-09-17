use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_rpc::v0_9_0::{
    BlockId, BroadcastedTxn, ContractStorageKeysItem, EventFilterWithPageRequest, EventsChunk, FeeEstimate, FunctionCall, GetStorageProofResult, MaybeDeprecatedContractClass, MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedBlockWithTxs, MaybePreConfirmedStateUpdate, MessageFeeEstimate, MsgFromL1, SimulationFlagForEstimateFee, StarknetGetBlockWithTxsAndReceiptsResult, TxnFinalityAndExecutionStatus, TxnReceiptWithBlockInfo, TxnWithHash
};
use starknet_types_core::felt::Felt;

#[versioned_rpc("V0_9_0", "starknet")]
pub trait StarknetReadRpcApi {
    #[method(name = "specVersion")]
    /// Get the Version of the StarkNet JSON-RPC Specification Being Used
    fn spec_version(&self) -> RpcResult<String>;

    /// Call a contract function at a given block id
    #[method(name = "call")]
    async fn call(&self, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<Felt>>;

    /// Get the number of transactions in a block given a block id
    #[method(name = "getBlockTransactionCount")]
    fn get_block_transaction_count(&self, block_id: BlockId) -> RpcResult<u128>;

    /// Estimate the fee associated with transaction
    #[method(name = "estimateFee")]
    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>>;

    /// Estimate the L2 fee of a message sent on L1
    #[method(name = "estimateMessageFee")]
    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<MessageFeeEstimate>;

    /// Get block information with full transactions and receipts given the block id
    #[method(name = "getBlockWithReceipts")]
    fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult>;

    /// Get block information with transaction hashes given the block id
    #[method(name = "getBlockWithTxHashes")]
    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedBlockWithTxHashes>;

    /// Get block information with full transactions given the block id
    #[method(name = "getBlockWithTxs")]
    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedBlockWithTxs>;

    /// Get the contract class at a given contract address for a given block id
    #[method(name = "getClassAt")]
    fn get_class_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<MaybeDeprecatedContractClass>;

    /// Get the contract class hash in the given block for the contract deployed at the given
    /// address
    #[method(name = "getClassHashAt")]
    fn get_class_hash_at(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt>;

    /// Get the contract class definition in the given block associated with the given hash
    #[method(name = "getClass")]
    fn get_class(&self, block_id: BlockId, class_hash: Felt) -> RpcResult<MaybeDeprecatedContractClass>;

    /// Returns all events matching the given filter
    #[method(name = "getEvents")]
    fn get_events(&self, filter: EventFilterWithPageRequest) -> RpcResult<EventsChunk>;

    /// Get the nonce associated with the given address at the given block
    #[method(name = "getNonce")]
    fn get_nonce(&self, block_id: BlockId, contract_address: Felt) -> RpcResult<Felt>;

    /// Get the value of the storage at the given address and key, at the given block id
    #[method(name = "getStorageAt")]
    fn get_storage_at(&self, contract_address: Felt, key: Felt, block_id: BlockId) -> RpcResult<Felt>;

    /// Get the details of a transaction by a given block id and index
    #[method(name = "getTransactionByBlockIdAndIndex")]
    fn get_transaction_by_block_id_and_index(&self, block_id: BlockId, index: u64) -> RpcResult<TxnWithHash>;

    /// Returns the information about a transaction by transaction hash.
    #[method(name = "getTransactionByHash")]
    fn get_transaction_by_hash(&self, transaction_hash: Felt) -> RpcResult<TxnWithHash>;

    /// Returns the receipt of a transaction by transaction hash.
    #[method(name = "getTransactionReceipt")]
    fn get_transaction_receipt(&self, transaction_hash: Felt) -> RpcResult<TxnReceiptWithBlockInfo>;

    /// Gets the Transaction Status, Including Mempool Status and Execution Details
    #[method(name = "getTransactionStatus")]
    async fn get_transaction_status(&self, transaction_hash: Felt) -> RpcResult<TxnFinalityAndExecutionStatus>;

    /// Get the information about the result of executing the requested block
    #[method(name = "getStateUpdate")]
    fn get_state_update(&self, block_id: BlockId) -> RpcResult<MaybePreConfirmedStateUpdate>;

    #[method(name = "getStorageProof")]
    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult>;
}

type SubscriptionItemPendingTxs = super::methods::ws::SubscriptionItem<mp_rpc::v0_9_0::TxnReceiptWithBlockInfo>;
type SubscriptionItemEvents = super::methods::ws::SubscriptionItem<mp_rpc::v0_9_0::EmittedEvent>;
type SubscriptionItemNewHeads = super::methods::ws::SubscriptionItem<mp_rpc::v0_9_0::BlockHeader>;
type SubscriptionItemTransactionStatus = super::methods::ws::SubscriptionItem<mp_rpc::v0_9_0::TxnStatus>;

#[versioned_rpc("V0_9_0", "starknet")]
pub trait StarknetWsRpcApi {
    #[subscription(name = "subscribeNewHeads", unsubscribe = "unsubscribeNewHeads", item = SubscriptionItemNewHeads, param_kind = map)]
    async fn subscribe_new_heads(&self, block: BlockId) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(
        name = "subscribeEvents",
        unsubscribe = "unsubscribeEvents",
        item = SubscriptionItemEvents,
        param_kind = map
    )]
    async fn subscribe_events(
        &self,
        from_address: Option<Felt>,
        keys: Option<Vec<Vec<Felt>>>,
        block: Option<BlockId>,
    ) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(
        name = "subscribeTransactionStatus",
        unsubscribe = "unsubscribeTransactionStatus",
        item = SubscriptionItemTransactionStatus,
        param_kind = map
    )]
    async fn subscribe_transaction_status(&self, transaction_hash: Felt) -> jsonrpsee::core::SubscriptionResult;

    #[subscription(
        name = "subscribePendingTransactions",
        unsubscribe = "unsubscribePendingTransactions",
        item = SubscriptionItemPendingTxs,
        param_kind = map
    )]
    async fn subscribe_pending_transactions(
        &self,
        transaction_details: bool,
        sender_address: Vec<starknet_types_core::felt::Felt>,
    ) -> jsonrpsee::core::SubscriptionResult;
    #[method(name = "unsubscribe")]
    async fn starknet_unsubscribe(&self, subscription_id: u64) -> RpcResult<bool>;
}
