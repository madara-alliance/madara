use async_trait::async_trait;
use mc_submit_tx::{SubmitTransaction, SubmitTransactionError, SubmitValidatedTransaction};
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_7_1::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::validated::ValidatedMempoolTx;
use mp_utils::service::{MadaraServiceId, ServiceContext};
use starknet_core::types::Felt;
use std::sync::Arc;

pub const ERROR: &str =
    "Failed to retrieve add transaction provider, meaning neither l2 sync nor block production are running";

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
pub struct SubmitTransactionSwitch {
    redirect_to_gateway: Arc<dyn SubmitTransaction>,
    mempool_with_validator: Arc<dyn SubmitTransaction>,
    ctx: ServiceContext,
}

impl SubmitTransactionSwitch {
    fn provider(&self) -> Result<&Arc<dyn SubmitTransaction>, SubmitTransactionError> {
        if self.ctx.service_status(MadaraServiceId::L2Sync).is_on() {
            Ok(&self.redirect_to_gateway)
        } else if self.ctx.service_status(MadaraServiceId::BlockProduction).is_on() {
            Ok(&self.mempool_with_validator)
        } else {
            Err(SubmitTransactionError::Internal(anyhow::anyhow!(ERROR)))
        }
    }
}

#[async_trait]
impl SubmitTransaction for SubmitTransactionSwitch {
    async fn submit_declare_v0_transaction(
        &self,
        tx: BroadcastedDeclareTxnV0,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        self.provider()?.submit_declare_v0_transaction(tx).await
    }
    async fn submit_declare_transaction(
        &self,
        tx: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        self.provider()?.submit_declare_transaction(tx).await
    }
    async fn submit_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
        self.provider()?.submit_deploy_account_transaction(tx).await
    }
    async fn submit_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
        self.provider()?.submit_invoke_transaction(tx).await
    }

    async fn received_transaction(&self, hash: Felt) -> Option<bool> {
        match self.provider().ok() {
            Some(provider) => provider.received_transaction(hash).await,
            None => None,
        }
    }

    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<Felt>> {
        match self.provider().ok() {
            Some(provider) => provider.subscribe_new_transactions().await,
            None => None,
        }
    }
}

#[derive(Clone)]
pub struct SubmitValidatedTransactionSwitch {
    redirect_to_gateway: Arc<dyn SubmitValidatedTransaction>,
    mempool: Arc<dyn SubmitValidatedTransaction>,
    ctx: ServiceContext,
}

impl SubmitValidatedTransactionSwitch {
    fn validated_provider(&self) -> Result<&Arc<dyn SubmitValidatedTransaction>, SubmitTransactionError> {
        if self.ctx.service_status(MadaraServiceId::L2Sync).is_on() {
            Ok(&self.redirect_to_gateway)
        } else if self.ctx.service_status(MadaraServiceId::BlockProduction).is_on() {
            Ok(&self.mempool)
        } else {
            Err(SubmitTransactionError::Internal(anyhow::anyhow!(ERROR)))
        }
    }
}

#[async_trait]
impl SubmitValidatedTransaction for SubmitValidatedTransactionSwitch {
    async fn submit_validated_transaction(&self, tx: ValidatedMempoolTx) -> Result<(), SubmitTransactionError> {
        self.validated_provider()?.submit_validated_transaction(tx).await
    }

    async fn received_transaction(&self, hash: Felt) -> Option<bool> {
        match self.validated_provider().ok() {
            Some(provider) => provider.received_transaction(hash).await,
            None => None,
        }
    }

    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<Felt>> {
        match self.validated_provider().ok() {
            Some(provider) => provider.subscribe_new_transactions().await,
            None => None,
        }
    }
}

/// TODO: remove this when we have another way to get the service statuses.
#[derive(Clone)]
pub struct MakeSubmitTransactionSwitch {
    redirect_to_gateway: Arc<dyn SubmitTransaction>,
    mempool_with_validator: Arc<dyn SubmitTransaction>,
}

impl MakeSubmitTransactionSwitch {
    pub fn new(
        redirect_to_gateway: Arc<dyn SubmitTransaction>,
        mempool_with_validator: Arc<dyn SubmitTransaction>,
    ) -> Self {
        Self { redirect_to_gateway, mempool_with_validator }
    }

    pub fn make(&self, ctx: ServiceContext) -> SubmitTransactionSwitch {
        SubmitTransactionSwitch {
            redirect_to_gateway: self.redirect_to_gateway.clone(),
            mempool_with_validator: self.mempool_with_validator.clone(),
            ctx,
        }
    }
}

#[derive(Clone)]
pub struct MakeSubmitValidatedTransactionSwitch {
    redirect_to_gateway: Arc<dyn SubmitValidatedTransaction>,
    mempool: Arc<dyn SubmitValidatedTransaction>,
}

impl MakeSubmitValidatedTransactionSwitch {
    pub fn new(
        redirect_to_gateway: Arc<dyn SubmitValidatedTransaction>,
        mempool: Arc<dyn SubmitValidatedTransaction>,
    ) -> Self {
        Self { redirect_to_gateway, mempool }
    }

    pub fn make(&self, ctx: ServiceContext) -> SubmitValidatedTransactionSwitch {
        SubmitValidatedTransactionSwitch {
            redirect_to_gateway: self.redirect_to_gateway.clone(),
            mempool: self.mempool.clone(),
            ctx,
        }
    }
}
