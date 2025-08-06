use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_block::BlockId;
use starknet_types_core::felt::Felt;

use mp_rpc::v0_8_1::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    BroadcastedTxn, ClassAndTxnHash, ContractAndTxnHash, FeeEstimate, MaybePendingBlockWithTxHashes,
    MaybePendingBlockWithTxs, MsgFromL1, SimulateTransactionsResult, SimulationFlag, SimulationFlagForEstimateFee,
    StarknetGetBlockWithTxsAndReceiptsResult,
};

type SubscriptionItemPendingTxs = super::methods::ws::SubscriptionItem<mp_rpc::v0_8_1::PendingTxnInfo>;
type SubscriptionItemEvents = super::methods::ws::SubscriptionItem<mp_rpc::v0_8_1::EmittedEvent>;
type SubscriptionItemNewHeads = super::methods::ws::SubscriptionItem<mp_rpc::v0_8_1::BlockHeader>;
type SubscriptionItemTransactionStatus = super::methods::ws::SubscriptionItem<mp_rpc::v0_8_1::NewTxnStatus>;

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

// Only V3 transactions are supported in this version.
#[versioned_rpc("V0_8_1", "starknet")]
pub trait StarknetWriteRpcApi {
    #[method(name = "addInvokeTransaction")]
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult>;

    #[method(name = "addDeployAccountTransaction")]
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash>;

    #[method(name = "addDeclareTransaction")]
    async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash>;
}

#[versioned_rpc("V0_8_1", "starknet")]
pub trait StarknetReadRpcApi {
    #[method(name = "specVersion")]
    fn spec_version(&self) -> RpcResult<String>;

    #[method(name = "getCompiledCasm")]
    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value>;

    #[method(name = "getStorageProof")]
    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<mp_rpc::v0_8_1::ContractStorageKeysItem>>,
    ) -> RpcResult<mp_rpc::v0_8_1::GetStorageProofResult>;

    #[method(name = "estimateFee")]
    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>>;

    #[method(name = "estimateMessageFee")]
    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate>;

    #[method(name = "getBlockWithReceipts")]
    fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult>;

    #[method(name = "getBlockWithTxHashes")]
    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes>;

    #[method(name = "getBlockWithTxs")]
    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs>;
}

#[versioned_rpc("V0_8_1", "starknet")]
pub trait StarknetTraceRpcApi {
    /// Returns the execution trace of a transaction by simulating it in the runtime.
    #[method(name = "simulateTransactions")]
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulateTransactionsResult>>;
}
