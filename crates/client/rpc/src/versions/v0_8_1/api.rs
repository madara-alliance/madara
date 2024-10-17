use m_proc_macros::versioned_starknet_rpc;

#[versioned_starknet_rpc("V0_8_0")]
pub trait StarknetWsApi {
    #[subscription(name = "subscribe_foo", unsubscribe = "unsubscribe_foo", item = String)]
    async fn foo(&self) -> jsonrpsee::core::SubscriptionResult;
}
