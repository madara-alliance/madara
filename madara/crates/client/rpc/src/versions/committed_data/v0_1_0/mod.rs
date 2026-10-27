//! Dataset ingestion and signed transaction submission on the private Committed Data listener.
//! Imports are durable before transaction admission. Rejected transactions can leave unused datasets.
mod authorization;
#[cfg(test)]
mod tests;
mod types;

use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::ErrorObjectOwned;
use m_proc_macros::versioned_rpc;
use mp_convert::Felt;
use mp_gateway::committed_data::{CommittedDataUpdate, MAX_DATASETS_PER_TRANSACTION};
use mp_rpc::v0_10_2::BroadcastedTxn;
use mp_utils::service::MadaraServiceId;
use std::sync::{Arc, LazyLock};

const MAX_SIGNATURE_CHECKS: usize = 4;
static SIGNATURE_CHECKS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_SIGNATURE_CHECKS)));

#[derive(Debug, thiserror::Error)]
enum CommittedDataRpcError {
    #[error("Unauthorized committed-data import")]
    UnauthorizedImport,
    #[error("Signature verification busy; retry later")]
    SignatureVerificationBusy,
    #[error("Combined submission requires active local block production")]
    LocalProductionRequired,
    #[error("Expected 1..=4 committed-data datasets")]
    InvalidDatasetCount,
    #[error("Query transactions cannot be submitted")]
    QueryTransaction,
}

impl From<CommittedDataRpcError> for ErrorObjectOwned {
    fn from(error: CommittedDataRpcError) -> Self {
        let code = match &error {
            CommittedDataRpcError::UnauthorizedImport => -32001,
            CommittedDataRpcError::SignatureVerificationBusy => -32002,
            CommittedDataRpcError::LocalProductionRequired => -32003,
            CommittedDataRpcError::InvalidDatasetCount | CommittedDataRpcError::QueryTransaction => -32602,
        };
        Self::owned(code, error.to_string(), None::<()>)
    }
}

pub use authorization::{import_message_hash, update_message_hash};
use authorization::{validate_import_signatures, validate_signatures};
pub use types::{
    CommittedDataEntries, CommittedDataImport, CommittedDataImports, ImportAndSubmitResult, ImportSignature,
};

fn local_submission_ready(starknet: &Starknet) -> bool {
    starknet.committed_data_transaction_submission
        && starknet.ctx.service_status_actual(MadaraServiceId::BlockProduction).is_on()
        && !starknet.ctx.service_status(MadaraServiceId::L2Sync).is_on()
}

/// Private dataset API. Mount only on the separately enabled committed-data listener.
#[versioned_rpc("V0_1_0", "madara")]
pub trait CommittedDataRpcApi {
    /// Merge upserts into an explicit retained base and check the expected resulting root.
    /// A null base initializes an unused namespace or retries an already registered root.
    /// This method does not update the on-chain root.
    #[method(name = "updateCommittedDataSet")]
    async fn update_committed_data_set(
        &self,
        update: CommittedDataUpdate,
        root: Felt,
        entries: CommittedDataEntries,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()>;

    /// Imports and authenticates private data; requires available import capacity.
    /// Acknowledges only after durable storage. Does not publish or authorize its root on-chain.
    #[method(name = "importCommittedDataSet")]
    async fn import_committed_data_snapshot(
        &self,
        root: Felt,
        entries: CommittedDataEntries,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()>;

    /// Durably imports 1..=4 datasets, then submits any signed Starknet account transaction.
    /// A multicall is encoded inside one INVOKE transaction. Dataset imports are not rolled back
    /// if a later import or transaction fails. Returns the transaction hash after admission;
    /// callers poll normal transaction status/receipts for preconfirmed execution and finality.
    #[method(name = "importCommittedDataAndSubmitTransaction")]
    async fn import_committed_data_and_submit_transaction(
        &self,
        datasets: CommittedDataImports,
        transaction: BroadcastedTxn,
    ) -> RpcResult<ImportAndSubmitResult>;
}

#[async_trait]
impl CommittedDataRpcApiV0_1_0Server for Starknet {
    async fn update_committed_data_set(
        &self,
        update: CommittedDataUpdate,
        root: Felt,
        entries: CommittedDataEntries,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()> {
        let values = entries.into_inner();
        let allowed = Arc::clone(&self.committed_data_signers);
        let chain_id = Felt::try_from(&self.backend.chain_config().chain_id)
            .map_err(|_| ErrorObjectOwned::from(CommittedDataRpcError::UnauthorizedImport))?;
        let count = values.len() as u32; // CommittedDataEntries enforces the protocol bound.
        let signatures = signatures.unwrap_or_default();
        let permit = SIGNATURE_CHECKS
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorObjectOwned::from(CommittedDataRpcError::SignatureVerificationBusy))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            validate_signatures(&allowed, update_message_hash(chain_id, update, root, count), &signatures)
        })
        .await
        .map_err(|error| StarknetRpcApiError::from(anyhow::anyhow!(error)))??;
        self.backend.check_recovery_capacity(Some(update), root, values.len()).map_err(StarknetRpcApiError::from)?;
        self.backend.update_committed_data(update, root, values.clone()).await.map_err(StarknetRpcApiError::from)?;
        self.backend.journal_committed_data(Some(update), root, values).await.map_err(StarknetRpcApiError::from)?;
        Ok(())
    }

    async fn import_committed_data_snapshot(
        &self,
        root: Felt,
        entries: CommittedDataEntries,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()> {
        let values = entries.into_inner();
        let allowed = Arc::clone(&self.committed_data_signers);
        let chain_id = self.backend.chain_config().chain_id.clone();
        let count = values.len();
        let signatures = signatures.unwrap_or_default();
        let permit = SIGNATURE_CHECKS
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorObjectOwned::from(CommittedDataRpcError::SignatureVerificationBusy))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            validate_import_signatures(&allowed, &chain_id, root, count, &signatures)
        })
        .await
        .map_err(|e| StarknetRpcApiError::from(anyhow::anyhow!(e)))??;
        self.backend.check_recovery_capacity(None, root, values.len()).map_err(StarknetRpcApiError::from)?;
        self.backend.import_committed_data_snapshot(root, values.clone()).await.map_err(StarknetRpcApiError::from)?;
        self.backend.journal_committed_data(None, root, values).await.map_err(StarknetRpcApiError::from)?;
        Ok(())
    }

    async fn import_committed_data_and_submit_transaction(
        &self,
        datasets: CommittedDataImports,
        transaction: BroadcastedTxn,
    ) -> RpcResult<ImportAndSubmitResult> {
        if !local_submission_ready(self) {
            return Err(CommittedDataRpcError::LocalProductionRequired.into());
        }
        if !(1..=MAX_DATASETS_PER_TRANSACTION).contains(&datasets.0.len()) {
            return Err(CommittedDataRpcError::InvalidDatasetCount.into());
        }
        // The transaction is deliberately submitted through the same validated path as public RPC.
        // In particular, do not accept a query-only transaction as a publication request.
        if transaction.is_query() {
            return Err(CommittedDataRpcError::QueryTransaction.into());
        }
        let mut dependencies = Vec::new();
        for dataset in datasets.0 {
            match dataset.update {
                Some(update) => {
                    self.update_committed_data_set(update, dataset.root, dataset.entries, Some(dataset.signatures))
                        .await?
                }
                None => {
                    self.import_committed_data_snapshot(dataset.root, dataset.entries, Some(dataset.signatures)).await?
                }
            }
            dependencies.push(
                self.backend.recovery_reference(dataset.update, dataset.root).map_err(StarknetRpcApiError::from)?,
            );
        }
        // Catch a sequencer that entered catch-up while the (possibly large) imports were running.
        if !local_submission_ready(self) {
            return Err(CommittedDataRpcError::LocalProductionRequired.into());
        }
        let transaction_hash = self
            .transaction_submitter
            .submit_committed_data_transaction(transaction, dependencies)
            .await
            .map_err(StarknetRpcApiError::from)?;
        Ok(ImportAndSubmitResult { transaction_hash })
    }
}
