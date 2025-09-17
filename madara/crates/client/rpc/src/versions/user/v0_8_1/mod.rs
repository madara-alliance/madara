use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_rpc::v0_8_1::{
    BlockId, BroadcastedTxn, FeeEstimate, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, MsgFromL1,
    SimulationFlagForEstimateFee, StarknetGetBlockWithTxsAndReceiptsResult,
};
use starknet_types_core::felt::Felt;

type SubscriptionItemPendingTxs = methods::ws::SubscriptionItem<mp_rpc::v0_8_1::PendingTxnInfo>;
type SubscriptionItemEvents = methods::ws::SubscriptionItem<mp_rpc::v0_8_1::EmittedEvent>;
type SubscriptionItemNewHeads = methods::ws::SubscriptionItem<mp_rpc::v0_8_1::BlockHeader>;
type SubscriptionItemTransactionStatus = methods::ws::SubscriptionItem<mp_rpc::v0_8_1::NewTxnStatus>;

pub mod methods;

#[versioned_rpc("V0_8_1", "starknet")]
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

#[versioned_rpc("V0_8_1", "starknet")]
pub trait StarknetReadRpcApi {
    #[method(name = "specVersion")]
    fn spec_version(&self) -> RpcResult<String>;

    #[method(name = "getCompiledCasm", and_versions = ["V0_9_0"])]
    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value>;

    #[method(name = "getStorageProof")]
    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<mp_rpc::v0_8_1::ContractStorageKeysItem>>,
    ) -> RpcResult<mp_rpc::v0_8_1::GetStorageProofResult>;

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
    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate>;

    /// Get block information with full transactions and receipts given the block id
    #[method(name = "getBlockWithReceipts")]
    fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult>;

    /// Get block information with transaction hashes given the block id
    #[method(name = "getBlockWithTxHashes")]
    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes>;

    /// Get block information with full transactions given the block id
    #[method(name = "getBlockWithTxs")]
    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs>;
}
