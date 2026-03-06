use blockifier::bouncer::BouncerWeights;
use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_block::header::CustomHeader;
use mp_convert::Felt;
use mp_rpc::admin::{BroadcastedDeclareTxnV0, ReplayBlockBoundary, ReplayBlockBoundaryStatus};
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{L1HandlerTransactionResult, L1HandlerTransactionWithFee};
use mp_utils::service::{MadaraServiceId, MadaraServiceStatus};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ServiceStatusInfo {
    pub service: MadaraServiceId,
    pub requested: MadaraServiceStatus,
    pub actual: MadaraServiceStatus,
}

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
    /// Submit a new class v0 declaration transaction, bypassing mempool and all validation.
    /// Only works in block production mode.
    #[method(name = "addDeclareV0Transaction")]
    async fn add_declare_v0_transaction(
        &self,
        declare_v0_transaction: BroadcastedDeclareTxnV0,
    ) -> RpcResult<ClassAndTxnHash>;

    /// Submit a declare transaction, bypassing mempool and all validation.
    /// Only works in block production mode.
    #[method(name = "bypassAddDeclareTransaction")]
    async fn bypass_add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTxn,
    ) -> RpcResult<ClassAndTxnHash>;

    /// Submit a deploy account transaction, bypassing mempool and all validation.
    /// Only works in block production mode.
    #[method(name = "bypassAddDeployAccountTransaction")]
    async fn bypass_add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash>;

    /// Submit an invoke transaction, bypassing mempool and all validation.
    /// Only works in block production mode.
    #[method(name = "bypassAddInvokeTransaction")]
    async fn bypass_add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult>;

    /// Force close a block.
    /// Only works in block production mode.
    #[method(name = "closeBlock")]
    async fn close_block(&self) -> RpcResult<()>;

    /// Revert the blockchain to a specific block hash, then shut down the node.
    ///
    /// This is the preferred reorg workflow for Madara because it coordinates
    /// an in-process stop of other services (so they ack as "actually down")
    /// before mutating the DB state, and then exits the process so Kubernetes
    /// (or another supervisor) can restart cleanly.
    ///
    #[method(name = "revertToAndShutdown")]
    async fn revert_to_and_shutdown(&self, block_hash: Felt) -> RpcResult<()>;

    /// Submit a L1 message into the bypass input stream
    #[method(name = "addL1HandlerMessage")]
    async fn add_l1_handler_message(
        &self,
        l1_handler_message: L1HandlerTransactionWithFee,
    ) -> RpcResult<L1HandlerTransactionResult>;

    /// Sets custom headers to be used for the upcoming block
    #[method(name = "setCustomBlockHeader")]
    async fn set_block_header(&self, custom_block_headers: CustomHeader) -> RpcResult<()>;

    /// Sets replay block boundary metadata for a specific block.
    ///
    /// This metadata is consumed by replay-aware batching/execution to avoid crossing block
    /// boundaries while transactions are sent asynchronously.
    #[method(name = "setReplayBoundary")]
    async fn set_replay_boundary(&self, replay_boundary: ReplayBlockBoundary) -> RpcResult<ReplayBlockBoundaryStatus>;

    /// Returns replay boundary status for a given block, if a boundary is configured.
    #[method(name = "getReplayBoundaryStatus")]
    async fn get_replay_boundary_status(&self, block_n: u64) -> RpcResult<Option<ReplayBlockBoundaryStatus>>;
}

#[versioned_rpc("V0_1_0", "madara")]
pub trait MadaraMempoolRpcApi {
    /// Enable or disable intake of mempool transactions in block production mode.
    ///
    /// - `true`: consume transactions from mempool as usual.
    /// - `false`: pause mempool intake while keeping bypass/L1 message paths active.
    #[method(name = "setMempoolIntake")]
    async fn set_mempool_intake(&self, enabled: bool) -> RpcResult<()>;
}

/// This is an admin method, so semver is different!
#[versioned_rpc("V0_1_0", "madara")]
pub trait MadaraReadRpcApi {
    /// Get the builtins  for the given block number
    #[method(name = "getBlockBuiltinWeights")]
    async fn get_block_builtin_weights(&self, block_number: u64) -> RpcResult<BouncerWeights>;
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
    async fn service(&self, service: Vec<MadaraServiceId>, status: ServiceRequest) -> RpcResult<MadaraServiceStatus>;

    /// Returns the requested and actual status of services.
    ///
    /// If the list is empty, returns the status of all externally controllable services.
    #[method(name = "serviceStatus")]
    async fn service_status(&self, service: Vec<MadaraServiceId>) -> RpcResult<Vec<ServiceStatusInfo>>;
}
