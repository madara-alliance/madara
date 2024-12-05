use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_transactions::BroadcastedDeclareTransactionV0;
use mp_utils::service::MadaraServiceStatus;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::ClassAndTxnHash;

/// This is an admin method, so semver is different!
#[versioned_rpc("V0_1_0", "madara")]
pub trait MadaraWriteRpcApi {
    /// Submit a new class v0 declaration transaction
    #[method(name = "addDeclareV0Transaction")]
    async fn add_declare_v0_transaction(
        &self,
        declare_v0_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<ClassAndTxnHash<Felt>>;
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
    /// Disables l1 and l2 sync.
    ///
    /// This required the node to have been passed an l1 key on startup,
    /// otherwise only L2 sync will be affected.
    ///
    /// # Returns
    ///
    /// True if any of l1 or l2 sync were previously enabled.
    #[method(name = "syncDisable")]
    async fn service_sync_disable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Enables l1 and l2 sync.
    ///
    /// This required the node to have been passed an l1 key on startup,
    /// otherwise only L2 sync will be affected.
    ///
    /// # Returns
    ///
    /// True if any of l1 or l2 sync were previously enabled.
    #[method(name = "syncEnable")]
    async fn service_sync_enable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Restarts l1 and l2 sync, with a 5s grace period in between.
    ///
    /// This required the node to have been passed an l1 key on startup,
    /// otherwise only L2 sync will be affected.
    ///
    /// # Returns
    ///
    /// True if any of l1 or l2 sync were previously enabled.
    #[method(name = "syncRestart")]
    async fn service_sync_restart(&self) -> RpcResult<MadaraServiceStatus>;

    /// Disables block production.
    ///
    /// This requires the node to have been started as a sequencer. It is not
    /// possible to toggle block production on a full node.
    ///
    /// # Returns
    ///
    /// True if block production was previously enabled.
    #[method(name = "blockProductionDisable")]
    async fn service_block_production_disable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Enables block production.
    ///
    /// This requires the node to have been started as a sequencer. It is not
    /// possible to toggle block production on a full node.
    ///
    /// # Returns
    ///
    /// True if block production was previously enabled.
    #[method(name = "blockProductionEnable")]
    async fn service_block_production_enable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Restarts block production, with a 5s grace period in between.
    ///
    /// This requires the node to have been started as a sequencer. It is not
    /// possible to toggle block production on a full node.
    ///
    /// # Returns
    ///
    /// True if block production was previously enabled.
    #[method(name = "blockProductionRestart")]
    async fn service_block_production_restart(&self) -> RpcResult<MadaraServiceStatus>;

    /// Disables user-facing rpc services.
    ///
    /// # Returns
    ///
    /// True if user rpc was previously enabled.
    #[method(name = "rpcDisable")]
    async fn service_rpc_disable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Enables user-facing rpc services.
    ///
    /// # Returns
    ///
    /// True if user rpc was previously enabled.
    #[method(name = "rpcEnable")]
    async fn service_rpc_enable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Restarts user-facing rpc services, with a 5s grace period in between.
    ///
    /// # Returns
    ///
    /// True if user rpc was previously enabled.
    #[method(name = "rpcRestart")]
    async fn service_rpc_restart(&self) -> RpcResult<MadaraServiceStatus>;

    /// Disables the feeder gateway.
    ///
    /// # Returns
    ///
    /// True if the feeder gateway was previously enabled.
    #[method(name = "gatewayDisable")]
    async fn service_gateway_disable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Enables the feeder gateway.
    ///
    /// # Returns
    ///
    /// True if the feeder gateway was previously enabled.
    #[method(name = "gatewayEnable")]
    async fn service_gateway_enable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Restarts the feeder gateway, with a 5s grace period in between.
    ///
    /// # Returns
    ///
    /// True if the feeder gateway was previously enabled.
    #[method(name = "gatewayRestart")]
    async fn service_gateway_restart(&self) -> RpcResult<MadaraServiceStatus>;

    /// Disables node telemetry.
    ///
    /// # Returns
    ///
    /// True if telemetry was previously enabled.
    #[method(name = "telemetryDisable")]
    async fn service_telemetry_disable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Enables node telemetry.
    ///
    /// # Returns
    ///
    /// True if telemetry was previously enabled.
    #[method(name = "telemtryEnable")]
    async fn service_telemetry_enable(&self) -> RpcResult<MadaraServiceStatus>;

    /// Restarts node telemetry, with a 5s grace period in between.
    ///
    /// # Returns
    ///
    /// True if telemetry was previously enabled.
    #[method(name = "telemetryRestart")]
    async fn service_telemetry_restart(&self) -> RpcResult<MadaraServiceStatus>;
}
