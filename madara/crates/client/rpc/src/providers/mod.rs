pub mod forward_to_provider;
pub mod mempool;

use std::sync::Arc;

pub use forward_to_provider::*;
use mc_db::mempool_db::SerializedMempoolTx;
pub use mempool::*;

use jsonrpsee::core::{async_trait, RpcResult};
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::BroadcastedDeclareTransactionV0;
use mp_utils::service::{MadaraServiceId, ServiceContext};

use crate::utils::OptionExt;

#[async_trait]
pub trait AddTransactionProvider: Send + Sync {
    /// Madara specific.
    async fn add_declare_v0_transaction(
        &self,
        declare_v0_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<ClassAndTxnHash>;
    async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash>;

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash>;

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult>;

    /// Madara specific.
    async fn add_trusted_validated_transaction(
        &self,
        tx_hash: Felt,
        tx: SerializedMempoolTx,
        converted_class: Option<ConvertedClass>,
    ) -> RpcResult<()>;
}

/// A simple struct whose sole purpose is to toggle between a L2 sync and local
/// (sequencer) transaction provider depending on the state of the node as
/// specified by [ServiceContext].
///
/// - If we are relying on L2 sync, then all transactions are forwarded to the
///   sequencer.
///
/// - If we are relying on local block production, then transactions are also
///   executed locally.
///
/// This exists to accommodate warp updates, which require we toggle from L2
/// sync transaction forwarding to local transaction execution if we are
/// launching the sync on a local sequencer.
#[derive(Clone)]
pub struct AddTransactionProviderGroup {
    l2_sync: Arc<dyn AddTransactionProvider>,
    mempool: Arc<dyn AddTransactionProvider>,
    ctx: ServiceContext,
}

impl AddTransactionProviderGroup {
    pub const ERROR: &str =
        "Failed to retrieve add transaction provider, meaning neither l2 sync nor block production are running";

    pub fn new(
        l2_sync: Arc<dyn AddTransactionProvider>,
        mempool: Arc<dyn AddTransactionProvider>,
        ctx: ServiceContext,
    ) -> Self {
        Self { l2_sync, mempool, ctx }
    }

    fn provider(&self) -> Option<&Arc<dyn AddTransactionProvider>> {
        if self.ctx.service_status(MadaraServiceId::L2Sync).is_on() {
            Some(&self.l2_sync)
        } else if self.ctx.service_status(MadaraServiceId::BlockProduction).is_on() {
            Some(&self.mempool)
        } else {
            None
        }
    }
}

#[async_trait]
impl AddTransactionProvider for AddTransactionProviderGroup {
    async fn add_declare_v0_transaction(
        &self,
        declare_v0_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<ClassAndTxnHash> {
        self.provider()
            .ok_or_internal_server_error(Self::ERROR)?
            .add_declare_v0_transaction(declare_v0_transaction)
            .await
    }

    async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash> {
        self.provider().ok_or_internal_server_error(Self::ERROR)?.add_declare_transaction(declare_transaction).await
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash> {
        self.provider()
            .ok_or_internal_server_error(Self::ERROR)?
            .add_deploy_account_transaction(deploy_account_transaction)
            .await
    }

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        self.provider().ok_or_internal_server_error(Self::ERROR)?.add_invoke_transaction(invoke_transaction).await
    }

    async fn add_trusted_validated_transaction(
        &self,
        tx_hash: Felt,
        tx: SerializedMempoolTx,
        converted_class: Option<ConvertedClass>,
    ) -> RpcResult<()> {
        self.provider()
            .ok_or_internal_server_error(Self::ERROR)?
            .add_trusted_validated_transaction(tx_hash, tx, converted_class)
            .await
    }
}
