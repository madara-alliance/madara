use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_block::BlockId;
use mp_rpc::v0_8_1::{L1TxnHash, TxnHashWithStatus};
use starknet_types_core::felt::Felt;

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

    #[method(name = "getMessagesStatus")]
    async fn get_messages_status(&self, tx_hash_l1: L1TxnHash) -> RpcResult<Vec<TxnHashWithStatus>>;
}
