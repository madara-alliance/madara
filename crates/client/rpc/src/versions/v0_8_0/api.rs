use m_proc_macros::versioned_rpc;
use starknet_types_core::felt::Felt;

pub(crate) type NewHead = starknet_types_rpc::BlockHeader<Felt>;

#[versioned_rpc("V0_8_0", "starknet")]
pub trait StarknetWsRpcApi {
    #[subscription(name = "subscribeNewHeads", unsubscribe = "unsubscribe", item = NewHead, param_kind = map)]
    async fn subscribe_new_heads(&self, block_id: starknet_core::types::BlockId)
        -> jsonrpsee::core::SubscriptionResult;
}
