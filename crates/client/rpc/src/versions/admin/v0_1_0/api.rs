use m_proc_macros::versioned_rpc;
use mp_transactions::BroadcastedDeclareTransactionV0;

use jsonrpsee::core::RpcResult;
use starknet_core::types::DeclareTransactionResult;

/// This is an admin method, so semver is different!
#[versioned_rpc("V0_1_0", "madara")]
pub trait MadaraWriteRpcApi {
    /// Submit a new class v0 declaration transaction
    #[method(name = "addDeclareV0Transaction")]
    async fn add_declare_v0_transaction(
        &self,
        declare_transaction_v0: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<DeclareTransactionResult>;
}

#[versioned_rpc("V0_1_0", "madara")]
pub trait MadaraStatusRpcApi {
    /// Can be used to check node availability and network latency
    ///
    /// # Returns
    ///
    /// * Ping time in unix time.
    #[method(name = "ping")]
    async fn ping(&self) -> RpcResult<u64>;

    /// Stops the node by gracefully shutting down each of its services.
    ///
    /// # Returns
    ///
    /// * Time of shutdown in unix time.
    #[method(name = "shutdown")]
    async fn shutdown(&self) -> RpcResult<u64>;

    /// Periodically sends a signal that the node is alive.
    ///
    /// # Sends
    ///
    /// * Current time in unix time
    #[subscription(name = "pulse", unsubscribe = "unsubscribe", item = u64)]
    async fn pulse(&self) -> jsonrpsee::core::SubscriptionResult;
}

#[versioned_rpc("V0_1_0", "madara")]
pub trait MadaraServicesRpcApi {
    /// Disables user-facing rpc services.
    ///
    /// This only works if user rpc has been enabled on startup, otherwise this
    /// does nothing.
    ///
    /// # Returns
    ///
    /// True if user rpc was previously enabled.
    #[method(name = "rpcDisable")]
    async fn service_rpc_disable(&self) -> RpcResult<bool>;

    /// Enables user-facing rpc services.
    ///
    /// This only works if user rpc has been enabled on startup, otherwise this
    /// does nothing.
    ///
    /// # Returns
    ///
    /// True if user rpc was previously enabled.
    #[method(name = "rpcEnable")]
    async fn service_rpc_enable(&self) -> RpcResult<bool>;

    /// Restarts user-facing rpc services, with a 5s grace period in between.
    ///
    /// This only works if user rpc has been enabled on startup, otherwise this
    /// does nothing.
    ///
    /// # Returns
    ///
    /// True if user rpc was previously enabled.
    #[method(name = "rpcRestart")]
    async fn service_rpc_restart(&self) -> RpcResult<bool>;

    /// Disables l1 and l2 sync services.
    ///
    /// This only works if sync services have been enabled on startup, otherwise
    /// this does nothing.
    ///
    /// # Returns
    ///
    /// True if any of l1 or l2 sync was previously enabled.
    #[method(name = "syncDisable")]
    async fn service_sync_disable(&self) -> RpcResult<bool>;

    /// Enables l1 and l2 sync services.
    ///
    /// This only works if sync services have been enabled on startup, otherwise
    /// this does nothing.
    ///
    /// # Returns
    ///
    /// True if any of l1 or l2 sync was previously enabled.
    #[method(name = "syncEnable")]
    async fn service_sync_enable(&self) -> RpcResult<bool>;

    /// Disables l1 and l2 sync services, with a 5s grace period in between.
    ///
    /// This only works if sync services have been enabled on startup, otherwise
    /// this does nothing.
    ///
    /// # Returns
    ///
    /// True if l1 or l2 sync was previously enabled.
    #[method(name = "syncRestart")]
    async fn service_sync_restart(&self) -> RpcResult<bool>;
}
