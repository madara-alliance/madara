use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_block::BlockId;
use starknet_types_core::felt::Felt;

pub(crate) type NewHead = starknet_types_rpc::BlockHeader<Felt>;

#[versioned_rpc("V0_8_0", "starknet")]
pub trait StarknetWsRpcApi {
    #[subscription(name = "subscribeNewHeads", unsubscribe = "unsubscribe", item = NewHead, param_kind = map)]
    async fn subscribe_new_heads(&self, block_id: BlockId) -> jsonrpsee::core::SubscriptionResult;
}

#[versioned_rpc("V0_8_0", "starknet")]
pub trait StarknetReadRpcApi {
    #[method(name = "specVersion")]
    fn spec_version(&self) -> RpcResult<String>;

    #[method(name = "getCompiledCasm")]
    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value>;
}
