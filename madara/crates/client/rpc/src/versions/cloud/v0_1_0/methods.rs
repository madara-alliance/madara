/// T-062: Cloud RPC method implementation.
///
/// Implements `addInvokeTransaction` on the private cloud endpoint.
/// Converts the broadcasted invoke tx to starknet-api format, builds a
/// `ValidatedTransaction` and submits it directly to the mempool via
/// `SubmitValidatedTransaction`, bypassing full pre-validation.
use crate::errors::StarknetRpcApiError;
use crate::versions::cloud::v0_1_0::ParadoxCloudWriteRpcApiV0_1_0Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mc_submit_tx::{RejectedTransactionError, RejectedTransactionErrorKind};
use mp_convert::ToFelt;
use mp_rpc::v0_9_0::{AddInvokeTransactionResult, BroadcastedInvokeTxn, BroadcastedTxn};
use mp_transactions::{
    validated::{TxTimestamp, ValidatedTransaction},
    IntoStarknetApiExt,
};
use std::time::Instant;

#[async_trait]
impl ParadoxCloudWriteRpcApiV0_1_0Server for Starknet {
    async fn add_validated_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        let start = Instant::now();

        // Obtain validated transaction provider — only set when serving cloud endpoint.
        let provider = match self.add_validated_transaction_provider.as_ref() {
            Some(p) => p,
            None => {
                tracing::error!("Cloud RPC: add_validated_transaction_provider not configured");
                if let Some(m) = self.cloud_metrics.as_ref() {
                    m.on_rejected("internal_error", start.elapsed().as_secs_f64());
                }
                return Err(StarknetRpcApiError::InternalServerError.into());
            }
        };

        if let Some(m) = self.cloud_metrics.as_ref() {
            m.on_received();
        }

        // Reject query-only transactions.
        if invoke_transaction.is_query() {
            if let Some(m) = self.cloud_metrics.as_ref() {
                m.on_rejected("query_only", start.elapsed().as_secs_f64());
            }
            return Err(StarknetRpcApiError::from(RejectedTransactionError::new(
                RejectedTransactionErrorKind::InvalidTransactionVersion,
                "Cannot submit query-only transactions",
            ))
            .into());
        }

        let arrived_at = TxTimestamp::now();
        let chain_id = self.backend.chain_config().chain_id.to_felt();
        let protocol_version = self.backend.chain_config().latest_protocol_version;

        // Convert to starknet-api format.
        let (api_tx, class) =
            BroadcastedTxn::Invoke(invoke_transaction).into_starknet_api(chain_id, protocol_version).map_err(|e| {
                if let Some(m) = self.cloud_metrics.as_ref() {
                    m.on_rejected("conversion_failed", start.elapsed().as_secs_f64());
                }
                StarknetRpcApiError::from(e)
            })?;

        let tx_hash = api_tx.tx_hash().to_felt();

        // Build ValidatedTransaction — no stateful validation, use cloud_charge_fee flag.
        let validated = ValidatedTransaction::from_starknet_api(api_tx, arrived_at, class, self.cloud_charge_fee);

        // Submit to mempool.
        let mempool_start = Instant::now();
        provider.submit_validated_transaction(validated).await.map_err(|e| {
            let reason = classify_mempool_error(&e);
            let elapsed = start.elapsed().as_secs_f64();
            if let Some(m) = self.cloud_metrics.as_ref() {
                m.on_rejected(reason, elapsed);
            }
            StarknetRpcApiError::from(e)
        })?;

        let mempool_duration = mempool_start.elapsed().as_secs_f64();
        let total_duration = start.elapsed().as_secs_f64();
        if let Some(m) = self.cloud_metrics.as_ref() {
            m.on_accepted(total_duration, mempool_duration);
        }

        Ok(AddInvokeTransactionResult { transaction_hash: tx_hash })
    }
}

/// Classify a mempool submission error into a low-cardinality reason label.
fn classify_mempool_error(err: &mc_submit_tx::SubmitTransactionError) -> &'static str {
    use mc_submit_tx::{RejectedTransactionErrorKind as Kind, SubmitTransactionError as E};
    match err {
        E::Rejected(r) => match r.kind {
            Kind::InvalidTransactionNonce => "mempool_nonce_too_low",
            Kind::MempoolLimitReached => "mempool_full",
            _ => "mempool_other",
        },
        E::Internal(_) => "internal_error",
        E::Unsupported => "mempool_other",
    }
}
