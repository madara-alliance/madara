use m_proc_macros::versioned_rpc;

#[versioned_rpc("V0_8_0", "starknet")]
pub trait StarknetWsRpcApi {
    #[subscription(name = "subscribe_foo", unsubscribe = "unsubscribe_foo", item = String)]
    async fn foo(&self) -> jsonrpsee::core::SubscriptionResult;
}
