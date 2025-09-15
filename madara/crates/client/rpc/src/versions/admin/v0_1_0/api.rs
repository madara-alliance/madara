use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_rpc::{
    admin::BroadcastedDeclareTxnV0, AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn,
    BroadcastedInvokeTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_utils::service::{MadaraServiceId, MadaraServiceStatus};
use serde::{Deserialize, Serialize};
use mp_block::header::{CustomHeader, GasPrices};
use mp_transactions::{L1HandlerTransactionResult, L1HandlerTransactionWithFee};

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

    /// Adds a L1 Handler message to the db for l1_txns_stream to pick
    /// Only works in block production mode.
    #[method(name = "addL1HandlerMessage")]
    async fn add_l1_handler_message(&self, l1_handler_message: L1HandlerTransactionWithFee) -> RpcResult<L1HandlerTransactionResult>;

    /// Use custom headers
    #[method(name = "setCustomBlockHeader")]
    async fn set_block_header(&self, custom_block_headers: CustomHeader) -> RpcResult<()>;
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
}
