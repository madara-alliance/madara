use crate::{versions::admin::v0_1_0::MadaraMempoolRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::admin::MempoolNonceFilter;
use mp_transactions::validated::ValidatedTransaction;

pub(super) fn matches_nonce_filter(transaction: &ValidatedTransaction, nonce_filter: MempoolNonceFilter) -> bool {
    if nonce_filter != MempoolNonceFilter::default() && transaction.sender_contract_address().is_none() {
        return false;
    }

    let nonce = transaction.transaction.nonce();
    nonce_filter.nonce_after.is_none_or(|lower| nonce > lower)
        && nonce_filter.nonce_before.is_none_or(|upper| nonce < upper)
}

#[async_trait]
impl MadaraMempoolRpcApiV0_1_0Server for Starknet {
    async fn set_mempool_intake(&self, enabled: bool) -> RpcResult<()> {
        if !self.rpc_unsafe_enabled {
            return Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
            }
            .into());
        }

        tracing::info!(target: "rpc::admin", enabled, "setMempoolIntake request received");
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .set_mempool_intake(enabled)
            .map_err(StarknetRpcApiError::from)?)
    }

    async fn flush_mempool(&self) -> RpcResult<()> {
        if !self.rpc_unsafe_enabled {
            return Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
            }
            .into());
        }

        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .flush_mempool()
            .map_err(StarknetRpcApiError::from)?)
    }

    async fn clear_saved_mempool(&self) -> RpcResult<()> {
        if !self.rpc_unsafe_enabled {
            return Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
            }
            .into());
        }

        tracing::info!(
            target: "rpc::admin",
            "clearSavedMempool request received; clearing persisted saved mempool rows only"
        );
        self.backend.clear_saved_mempool_transactions().map_err(StarknetRpcApiError::from)?;
        tracing::info!(target: "rpc::admin", "clearSavedMempool completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_utils::TestTransactionProvider, Starknet};
    use mc_db::{test_utils::validated_l1_handler, MadaraBackend};
    use mp_chain_config::ChainConfig;
    use mp_convert::Felt;
    use mp_utils::service::ServiceContext;
    use std::sync::Arc;

    fn make_rpc_with_backend(rpc_unsafe_enabled: bool) -> (Arc<MadaraBackend>, Starknet) {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let mut rpc = Starknet::new(
            backend.clone(),
            Arc::new(TestTransactionProvider),
            Default::default(),
            None,
            ServiceContext::default(),
        );
        rpc.set_rpc_unsafe_enabled(rpc_unsafe_enabled);
        (backend, rpc)
    }

    fn make_rpc(rpc_unsafe_enabled: bool) -> Starknet {
        make_rpc_with_backend(rpc_unsafe_enabled).1
    }

    #[tokio::test]
    async fn flush_mempool_requires_rpc_unsafe() {
        let rpc = make_rpc(false);

        let err = rpc.flush_mempool().await.expect_err("flush should require rpc unsafe");
        assert_ne!(err.code(), 0);
    }

    #[tokio::test]
    async fn flush_mempool_requires_block_production_handle() {
        let rpc = make_rpc(true);

        let err = rpc.flush_mempool().await.expect_err("flush should require block production");
        assert_ne!(err.code(), 0);
    }

    #[tokio::test]
    async fn clear_saved_mempool_requires_rpc_unsafe() {
        let rpc = make_rpc(false);

        let err = rpc.clear_saved_mempool().await.expect_err("clearSavedMempool should require rpc unsafe");
        assert_ne!(err.code(), 0);
    }

    #[tokio::test]
    async fn clear_saved_mempool_clears_persisted_rows_without_block_production_handle() {
        let (backend, rpc) = make_rpc_with_backend(true);
        let tx_1 = validated_l1_handler(Felt::from(11u64));
        let tx_2 = validated_l1_handler(Felt::from(12u64));

        backend.write_saved_mempool_transaction(&tx_1).expect("writing first saved mempool tx should succeed");
        backend.write_saved_mempool_transaction(&tx_2).expect("writing second saved mempool tx should succeed");
        assert_eq!(
            backend
                .get_saved_mempool_transactions()
                .collect::<Result<Vec<_>, _>>()
                .expect("reading saved mempool txs before clear"),
            vec![tx_1.clone(), tx_2.clone()]
        );

        rpc.clear_saved_mempool().await.expect("clearSavedMempool should succeed without block production handle");

        assert!(
            backend
                .get_saved_mempool_transactions()
                .collect::<Result<Vec<_>, _>>()
                .expect("reading saved mempool txs after clear")
                .is_empty(),
            "persisted saved mempool rows should be cleared"
        );
    }
}
