use blockifier::bouncer::BouncerWeights;
use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_block::header::CustomHeader;
use mp_convert::Felt;
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_10_2::BroadcastedInvokeTxn;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, ClassAndTxnHash, ContractAndTxnHash,
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReplayBlockRequest {
    pub custom_header: CustomHeader,
    pub transactions: Vec<ReplayBlockTransaction>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReplayBlockResult {
    pub block_number: u64,
    pub block_hash: Felt,
    pub transaction_hashes: Vec<Felt>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayBlockTransaction {
    Invoke { transaction_hash: Felt, invoke_transaction: BroadcastedInvokeTxn },
    DeclareV0 { transaction_hash: Felt, declare_transaction: BroadcastedDeclareTxnV0 },
    Declare { transaction_hash: Felt, declare_transaction: BroadcastedDeclareTxn },
    DeployAccount { transaction_hash: Felt, deploy_account_transaction: BroadcastedDeployAccountTxn },
    L1Handler { transaction_hash: Felt, l1_handler_message: L1HandlerTransactionWithFee },
}

impl ReplayBlockTransaction {
    pub fn expected_tx_hash(&self) -> Felt {
        match self {
            Self::Invoke { transaction_hash, .. }
            | Self::DeclareV0 { transaction_hash, .. }
            | Self::Declare { transaction_hash, .. }
            | Self::DeployAccount { transaction_hash, .. }
            | Self::L1Handler { transaction_hash, .. } => *transaction_hash,
        }
    }
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

    /// Replays a full Starknet block inside Madara and only returns after the block is confirmed.
    ///
    /// The replay request stages the custom header, submits the ordered transactions, waits until all
    /// expected transaction hashes are present in the current preconfirmed block, force-closes the block,
    /// and then waits until the confirmed block is written to the database.
    #[method(name = "replayBlock")]
    async fn replay_block(&self, replay_block_request: ReplayBlockRequest) -> RpcResult<ReplayBlockResult>;
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
    /// Process-global services such as telemetry are intentionally excluded.
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
