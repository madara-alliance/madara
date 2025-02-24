use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_transactions::BroadcastedDeclareTransactionV0;
use mp_utils::service::{MadaraServiceId, ServiceStatus};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use starknet_types_rpc::ClassAndTxnHash;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServiceRequest {
    Start,
    Stop,
    Restart,
}

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
    /// Sets the status of one or more services
    ///
    /// # Returns
    ///
    /// * 'on' if any service was active before being toggled, 'off' otherwise.
    #[method(name = "service")]
    async fn service(&self, service: Vec<MadaraServiceId>, status: ServiceRequest) -> RpcResult<Vec<ServiceStatus>>;
}
