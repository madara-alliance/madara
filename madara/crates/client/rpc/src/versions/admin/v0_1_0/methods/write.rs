use super::mempool::matches_nonce_filter;
use crate::{versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use anyhow::Context;
use jsonrpsee::core::{async_trait, RpcResult};
use mc_db::MadaraStorageRead;
use mc_submit_tx::{SubmitL1HandlerTransaction, SubmitTransaction};
use mp_block::header::CustomHeader;
use mp_convert::Felt;
use mp_rpc::admin::{BroadcastedDeclareTxnV0, FlushMempoolTxnsParams, FlushMempoolTxnsResult, MempoolNonceFilter};
use mp_rpc::v0_10_2::BroadcastedInvokeTxn;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{validated::ValidatedTransaction, L1HandlerTransactionResult, L1HandlerTransactionWithFee};
use mp_utils::service::{MadaraServiceId, MadaraServiceStatus, SERVICE_GRACE_PERIOD};
use std::time::Duration;
use tokio::time::Instant;

const REVERT_STOP_WAIT_EXTRA: Duration = Duration::from_secs(5);
const REVERT_STOP_LOG_INTERVAL: Duration = Duration::from_secs(1);
const REVERT_STOP_POLL_INTERVAL: Duration = Duration::from_millis(200);
const REVERT_SHUTDOWN_DELAY: Duration = Duration::from_millis(100);

enum FlushMode {
    All { nonce_filter: MempoolNonceFilter },
    ContractAddress { contract_address: Felt, nonce_filter: MempoolNonceFilter },
    TransactionHashes { transaction_hashes: Vec<Felt>, nonce_filter: MempoolNonceFilter },
}

fn invalid_flush_params(message: &'static str) -> jsonrpsee::types::ErrorObjectOwned {
    jsonrpsee::types::ErrorObject::owned(jsonrpsee::types::ErrorCode::InvalidParams.code(), message, Some(()))
}

impl FlushMode {
    fn from_params(params: FlushMempoolTxnsParams) -> RpcResult<Self> {
        let nonce_filter = params.nonce_filter;
        let using_all = params.all;
        let using_contract_address = params.contract_address.is_some();
        let using_transaction_hashes = params.transaction_hashes.as_ref().is_some_and(|hashes| !hashes.is_empty());
        let using_nonce_filter = nonce_filter.nonce_after.is_some() || nonce_filter.nonce_before.is_some();

        let selected_filters = [using_all, using_contract_address, using_transaction_hashes]
            .into_iter()
            .filter(|selected| *selected)
            .count();
        if selected_filters > 1 {
            return Err(invalid_flush_params(
                "Provide at most one base flush filter: all, contract_address, or transaction_hashes",
            ));
        }

        if using_all {
            return Ok(Self::All { nonce_filter });
        }

        if let Some(contract_address) = params.contract_address {
            return Ok(Self::ContractAddress { contract_address, nonce_filter });
        }

        if using_transaction_hashes {
            return Ok(Self::TransactionHashes {
                transaction_hashes: params.transaction_hashes.unwrap_or_default(),
                nonce_filter,
            });
        }

        if using_nonce_filter {
            return Err(invalid_flush_params(
                "Nonce filters only narrow an explicit base flush filter: all, contract_address, or transaction_hashes",
            ));
        }

        Err(invalid_flush_params(
            "Provide at least one base flush filter: all, contract_address, or transaction_hashes",
        ))
    }
    fn matches(&self, transaction: &ValidatedTransaction) -> bool {
        match self {
            FlushMode::All { nonce_filter } => matches_nonce_filter(transaction, *nonce_filter),
            FlushMode::ContractAddress { contract_address, nonce_filter } => {
                transaction.sender_contract_address() == Some(*contract_address)
                    && matches_nonce_filter(transaction, *nonce_filter)
            }
            FlushMode::TransactionHashes { transaction_hashes, nonce_filter } => {
                transaction_hashes.contains(&transaction.hash) && matches_nonce_filter(transaction, *nonce_filter)
            }
        }
    }
}

fn schedule_global_cancel(ctx: mp_utils::service::ServiceContext) {
    tokio::spawn(async move {
        tokio::time::sleep(REVERT_SHUTDOWN_DELAY).await;
        ctx.cancel_global();
    });
}

// Only include services controlled by ServiceMonitor.
fn services_to_stop_for_revert() -> [MadaraServiceId; 6] {
    [
        MadaraServiceId::L1Sync,
        MadaraServiceId::L2Sync,
        MadaraServiceId::BlockProduction,
        MadaraServiceId::RpcUser,
        MadaraServiceId::Gateway,
        MadaraServiceId::Mempool,
    ]
}

#[async_trait]
impl MadaraWriteRpcApiV0_1_0Server for Starknet {
    /// Submit a new class v0 declaration transaction, bypassing mempool and all validation.
    /// Only works in block production mode.
    async fn add_declare_v0_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTxnV0,
    ) -> RpcResult<ClassAndTxnHash> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .submit_declare_v0_transaction(declare_transaction)
            .await
            .map_err(StarknetRpcApiError::from)?)
    }

    /// Submit a declare transaction, bypassing mempool and all validation.
    /// Only works in block production mode.
    async fn bypass_add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTxn,
    ) -> RpcResult<ClassAndTxnHash> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .submit_declare_transaction(declare_transaction)
            .await
            .map_err(StarknetRpcApiError::from)?)
    }

    /// Submit a deploy account transaction, bypassing mempool and all validation.
    /// Only works in block production mode.
    async fn bypass_add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .submit_deploy_account_transaction(deploy_account_transaction)
            .await
            .map_err(StarknetRpcApiError::from)?)
    }

    /// Submit an invoke transaction, bypassing mempool and all validation.
    /// Only works in block production mode.
    async fn bypass_add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .submit_invoke_transaction(invoke_transaction)
            .await
            .map_err(StarknetRpcApiError::from)?)
    }

    /// Force close a block.
    /// Only works in block production mode.
    async fn close_block(&self) -> RpcResult<()> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .close_block()
            .await
            .context("Force-closing block")
            .map_err(StarknetRpcApiError::from)?)
    }

    /// Force revert chain to a previous block by hash.
    /// Only available when unsafe RPC methods are enabled.
    /// Coordinated revert: stop all other services, wait for ack, revert DB, then exit.
    async fn revert_to_and_shutdown(&self, block_hash: Felt) -> RpcResult<()> {
        // Check if unsafe RPC methods are enabled
        if !self.rpc_unsafe_enabled {
            return Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
            }
            .into());
        }

        // Validate revert target and snap sync constraints early (before shutdown).
        let target_block_n = self
            .backend
            .db
            .find_block_hash(&block_hash)
            .context("Failed to find block number for revert target")
            .map_err(StarknetRpcApiError::from)?
            .ok_or_else(|| StarknetRpcApiError::ErrUnexpectedError {
                error: format!("Block with hash {:#x} not found", block_hash).into(),
            })?;

        if let Some(snap_sync_latest_block) = self
            .backend
            .get_snap_sync_latest_block()
            .context("Failed to check snap sync status")
            .map_err(StarknetRpcApiError::from)?
        {
            if target_block_n < snap_sync_latest_block {
                return Err(StarknetRpcApiError::ErrUnexpectedError {
                    error: format!(
                        "Cannot revert to block {} because snap sync was used up to block {}. Trie data is only available from block {} onwards.",
                        target_block_n, snap_sync_latest_block, snap_sync_latest_block
                    )
                    .into(),
                }
                .into());
            }
        }

        // 1) Initiate shutdown of all services except Admin RPC.
        let stop_svcs = services_to_stop_for_revert();

        tracing::info!(
            target: "rpc::admin",
            "revertToAndShutdown: requesting shutdown for services (excluding rpc_admin): {:?}",
            stop_svcs.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );

        for svc in &stop_svcs {
            let prev = self.ctx.service_remove(*svc);
            tracing::info!(
                target: "rpc::admin",
                "revertToAndShutdown: shutdown requested for service={} (was_requested={})",
                svc,
                prev
            );
        }

        // 2) Wait until all services are *actually* down.
        let timeout = SERVICE_GRACE_PERIOD + REVERT_STOP_WAIT_EXTRA;
        let deadline = Instant::now() + timeout;
        let mut last_log = Instant::now();
        let log_interval = REVERT_STOP_LOG_INTERVAL;

        loop {
            let mut still_up: Vec<MadaraServiceId> = Vec::new();
            for svc in &stop_svcs {
                if self.ctx.service_status_actual(*svc) == MadaraServiceStatus::On {
                    still_up.push(*svc);
                }
            }

            if still_up.is_empty() {
                break;
            }

            if Instant::now() >= deadline {
                tracing::error!(
                    target: "rpc::admin",
                    "revertToAndShutdown: timed out waiting for services to stop; still_up={:?}",
                    still_up.iter().map(|s| s.to_string()).collect::<Vec<_>>()
                );
                schedule_global_cancel(self.ctx.clone());
                return Err(StarknetRpcApiError::ErrUnexpectedError {
                    error: format!(
                        "Timed out waiting for services to stop (timeout {:?}). Still up: {:?}",
                        timeout,
                        still_up.iter().map(|s| s.to_string()).collect::<Vec<_>>()
                    )
                    .into(),
                }
                .into());
            }

            if Instant::now().duration_since(last_log) >= log_interval {
                tracing::info!(
                    target: "rpc::admin",
                    "revertToAndShutdown: waiting for services to stop... still_up={:?}",
                    still_up.iter().map(|s| s.to_string()).collect::<Vec<_>>()
                );
                last_log = Instant::now();
            }

            tokio::time::sleep(REVERT_STOP_POLL_INTERVAL).await;
        }

        tracing::info!(target: "rpc::admin", "revertToAndShutdown: all non-admin services are down; proceeding with revert");

        // 3) Revert DB state, then refresh backend chain tip broadcast.
        tracing::info!(
            target: "rpc::admin",
            "revertToAndShutdown: reverting chain to block_hash={:#x} (block_number={})",
            block_hash,
            target_block_n
        );
        self.backend.revert_to(&block_hash).map_err(StarknetRpcApiError::from)?;

        tracing::info!(target: "rpc::admin", "revertToAndShutdown: revert complete; triggering node shutdown");

        // Shut down the process after responding, so the client gets an ACK.
        schedule_global_cancel(self.ctx.clone());

        Ok(())
    }

    async fn add_l1_handler_message(
        &self,
        l1_handler_message: L1HandlerTransactionWithFee,
    ) -> RpcResult<L1HandlerTransactionResult> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .ok_or(StarknetRpcApiError::UnimplementedMethod)?
            .submit_l1_handler_transaction(l1_handler_message)
            .await
            .map_err(StarknetRpcApiError::from)?)
    }

    async fn set_block_header(&self, custom_block_headers: CustomHeader) -> RpcResult<()> {
        // Check if unsafe RPC methods are enabled
        if !self.rpc_unsafe_enabled {
            return Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
            }
            .into());
        }

        self.backend.set_custom_header(custom_block_headers).map_err(StarknetRpcApiError::from)?;

        Ok(())
    }

    async fn flush_mempool_txns(&self, params: FlushMempoolTxnsParams) -> RpcResult<FlushMempoolTxnsResult> {
        if !self.rpc_unsafe_enabled {
            return Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
            }
            .into());
        }

        let mempool = self.mempool.as_ref().ok_or(StarknetRpcApiError::UnimplementedMethod)?;
        let flush_mode = FlushMode::from_params(params)?;
        let removed_transactions = match flush_mode {
            FlushMode::TransactionHashes { transaction_hashes, nonce_filter } => {
                if nonce_filter == MempoolNonceFilter::default() {
                    mempool.flush_transactions_by_hashes(transaction_hashes).await
                } else {
                    mempool
                        .flush_transactions_matching(|tx| {
                            transaction_hashes.contains(&tx.hash) && matches_nonce_filter(tx, nonce_filter)
                        })
                        .await
                }
            }
            flush_mode => mempool.flush_transactions_matching(|tx| flush_mode.matches(tx)).await,
        };

        Ok(FlushMempoolTxnsResult {
            removed_transaction_hashes: removed_transactions.into_iter().map(|tx| tx.hash).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::services_to_stop_for_revert;
    use crate::{
        test_utils::TestTransactionProvider, versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server, Starknet,
    };
    use mc_db::{
        test_utils::{add_test_block, l1_handler_tx_with_receipt},
        MadaraBackend,
    };
    use mc_mempool::{Mempool, MempoolConfig};
    use mp_block::header::{CustomHeader, GasPrices};
    use mp_chain_config::ChainConfig;
    use mp_convert::Felt;
    use mp_rpc::admin::{FlushMempoolTxnsParams, MempoolNonceFilter};
    use mp_transactions::{
        validated::{TxTimestamp, ValidatedTransaction},
        InvokeTransaction, InvokeTransactionV1, L1HandlerTransaction, Transaction,
    };
    use mp_utils::service::{MadaraServiceMask, MadaraServiceStatus, ServiceContext};
    use std::sync::Arc;
    use std::time::Duration;

    fn make_starknet(backend: Arc<MadaraBackend>, ctx: ServiceContext) -> Starknet {
        let provider = Arc::new(TestTransactionProvider);
        let mut rpc = Starknet::new(backend, Arc::clone(&provider) as _, provider, Default::default(), None, ctx);
        rpc.set_rpc_unsafe_enabled(true);
        rpc
    }

    fn make_starknet_with_mempool_and_unsafe(rpc_unsafe_enabled: bool) -> (Arc<Mempool>, Starknet) {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
        let mut rpc = make_starknet(backend, ServiceContext::new_for_testing());
        rpc.set_rpc_unsafe_enabled(rpc_unsafe_enabled);
        rpc.set_mempool(mempool.clone());
        (mempool, rpc)
    }

    fn make_starknet_with_mempool() -> (Arc<Mempool>, Starknet) {
        make_starknet_with_mempool_and_unsafe(true)
    }

    fn invoke_v1_tx(sender: Felt, nonce: Felt, hash: Felt, arrived_at: u64) -> ValidatedTransaction {
        ValidatedTransaction {
            transaction: Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
                sender_address: sender,
                calldata: vec![Felt::from(10_u64)].into(),
                max_fee: Felt::from(1_u64),
                signature: vec![].into(),
                nonce,
            })),
            paid_fee_on_l1: None,
            contract_address: sender,
            arrived_at: TxTimestamp(arrived_at),
            declared_class: None,
            hash,
            charge_fee: true,
        }
    }

    fn l1_handler_tx(contract_address: Felt, hash: Felt, arrived_at: u64) -> ValidatedTransaction {
        ValidatedTransaction {
            transaction: Transaction::L1Handler(L1HandlerTransaction {
                version: Felt::ZERO,
                nonce: 0,
                contract_address,
                entry_point_selector: Felt::from(123_u64),
                calldata: vec![Felt::from(10_u64)].into(),
            }),
            paid_fee_on_l1: Some(1),
            contract_address,
            arrived_at: TxTimestamp(arrived_at),
            declared_class: None,
            hash,
            charge_fee: true,
        }
    }

    #[tokio::test]
    async fn revert_waits_for_actual_service_shutdown_before_reverting_and_cancels_node() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

        let block_0_hash = add_test_block(&backend, 0, vec![]);
        add_test_block(&backend, 1, vec![]);

        let requested = Arc::new(MadaraServiceMask::default());
        for svc in services_to_stop_for_revert() {
            requested.activate(svc);
        }
        let actual = Arc::new(MadaraServiceMask::default());
        actual.activate(mp_utils::service::MadaraServiceId::L2Sync);

        let ctx = ServiceContext::new_with_services(Arc::clone(&requested)).with_services_actual(Arc::clone(&actual));
        let rpc = make_starknet(backend.clone(), ctx.clone());

        let mut cancel_wait_ctx = ctx.clone();
        let wait_cancelled = tokio::spawn(async move { cancel_wait_ctx.cancelled().await });

        let rpc_task = tokio::spawn(async move { rpc.revert_to_and_shutdown(block_0_hash).await });

        // While one service is still reported as "actually up", revert must not proceed.
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(backend.latest_confirmed_block_n(), Some(1));
        for svc in services_to_stop_for_revert() {
            assert_eq!(ctx.service_status_requested(svc), MadaraServiceStatus::Off);
        }

        actual.deactivate(mp_utils::service::MadaraServiceId::L2Sync);

        rpc_task.await.expect("rpc task should complete").expect("revert should succeed");
        assert_eq!(backend.latest_confirmed_block_n(), Some(0));

        tokio::time::timeout(Duration::from_secs(2), wait_cancelled)
            .await
            .expect("node cancellation should be triggered")
            .expect("cancel waiter task should not panic");
    }

    #[tokio::test]
    async fn revert_fails_when_source_mapping_missing() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

        let block_0_hash = add_test_block(&backend, 0, vec![]);
        let reverted_nonce = 7u64;
        add_test_block(&backend, 1, vec![l1_handler_tx_with_receipt(reverted_nonce, Felt::from(700u64))]);

        let rpc = make_starknet(backend.clone(), ServiceContext::default());

        // Missing nonce->l1_block metadata should fail fast without mutating the chain.
        let err = rpc
            .revert_to_and_shutdown(block_0_hash)
            .await
            .expect_err("revert should fail when source metadata is missing");
        assert_ne!(err.code(), 0);
        assert_eq!(backend.latest_confirmed_block_n(), Some(1));
        assert!(backend.get_l1_handler_txn_hash_by_nonce(reverted_nonce).expect("DB read should succeed").is_some());
    }

    #[tokio::test]
    async fn set_block_header_updates_fake_preconfirmed_view() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        add_test_block(&backend, 0, vec![]);

        let rpc = make_starknet(backend.clone(), ServiceContext::default());
        let custom_header = CustomHeader {
            block_n: 1,
            timestamp: 1_234_567_890,
            gas_prices: GasPrices {
                eth_l1_gas_price: 11,
                strk_l1_gas_price: 12,
                eth_l1_data_gas_price: 21,
                strk_l1_data_gas_price: 22,
                eth_l2_gas_price: 31,
                strk_l2_gas_price: 32,
            },
            expected_block_hash: Felt::from(0x1234_u64),
        };

        rpc.set_block_header(custom_header.clone()).await.expect("set block header should succeed");

        let preconfirmed =
            backend.block_view_on_preconfirmed_or_fake().expect("fake preconfirmed block should always be available");

        assert_eq!(preconfirmed.block_number(), custom_header.block_n);
        assert_eq!(preconfirmed.header().block_timestamp.0, custom_header.timestamp);
        assert_eq!(preconfirmed.header().gas_prices, custom_header.gas_prices);
    }

    #[tokio::test]
    async fn flush_mempool_txns_all_removes_everything() {
        let (mempool, rpc) = make_starknet_with_mempool();
        let base = TxTimestamp::now().0;
        let tx1 = invoke_v1_tx(Felt::from(11_u64), Felt::ZERO, Felt::from(101_u64), base);
        let tx2 = invoke_v1_tx(Felt::from(22_u64), Felt::ZERO, Felt::from(202_u64), base + 1_000);

        mempool.accept_tx(tx1.clone()).await.unwrap();
        mempool.accept_tx(tx2.clone()).await.unwrap();

        let result = rpc.flush_mempool_txns(FlushMempoolTxnsParams { all: true, ..Default::default() }).await.unwrap();

        assert_eq!(result.removed_transaction_hashes, vec![tx1.hash, tx2.hash]);
        assert!(mempool.is_empty().await);
    }

    #[tokio::test]
    async fn flush_mempool_txns_requires_unsafe_rpc() {
        let (mempool, rpc) = make_starknet_with_mempool_and_unsafe(false);
        let tx = invoke_v1_tx(Felt::from(11_u64), Felt::ZERO, Felt::from(101_u64), TxTimestamp::now().0);
        mempool.accept_tx(tx.clone()).await.unwrap();

        let err = rpc.flush_mempool_txns(FlushMempoolTxnsParams { all: true, ..Default::default() }).await.unwrap_err();

        assert_eq!(err.code(), 63);
        assert_eq!(err.message(), "An unexpected error occurred");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(err.data().expect("error data should be present").get())
                .expect("error data should be valid JSON"),
            serde_json::json!("This method requires the --rpc-unsafe flag to be enabled")
        );
        assert_eq!(
            mempool
                .snapshot_transaction_hashes_matching(0, usize::MAX, false, |_| true)
                .await
                .into_iter()
                .map(|tx| tx.transaction_hash)
                .collect::<Vec<_>>(),
            vec![tx.hash]
        );
    }

    #[tokio::test]
    async fn flush_mempool_txns_by_sender_contract_address_filters_sender_side_only() {
        let (mempool, rpc) = make_starknet_with_mempool();
        let base = TxTimestamp::now().0;
        let sender_match = invoke_v1_tx(Felt::from(77_u64), Felt::ZERO, Felt::from(701_u64), base);
        let to_match_only = l1_handler_tx(Felt::from(99_u64), Felt::from(702_u64), base + 1_000);
        let untouched = invoke_v1_tx(Felt::from(88_u64), Felt::ZERO, Felt::from(703_u64), base + 2_000);

        mempool.accept_tx(sender_match.clone()).await.unwrap();
        mempool.accept_tx(to_match_only.clone()).await.unwrap();
        mempool.accept_tx(untouched.clone()).await.unwrap();

        let result = rpc
            .flush_mempool_txns(FlushMempoolTxnsParams {
                contract_address: Some(Felt::from(77_u64)),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(result.removed_transaction_hashes, vec![sender_match.hash]);
        let remaining = mempool.snapshot_transactions_matching(0, usize::MAX, false, |_| true).await;
        assert_eq!(
            remaining.into_iter().map(|tx| tx.transaction.hash).collect::<Vec<_>>(),
            vec![to_match_only.hash, untouched.hash]
        );
    }

    #[tokio::test]
    async fn flush_mempool_txns_by_explicit_hashes_removes_only_requested_transactions() {
        let (mempool, rpc) = make_starknet_with_mempool();
        let base = TxTimestamp::now().0;
        let tx1 = invoke_v1_tx(Felt::from(11_u64), Felt::ZERO, Felt::from(901_u64), base);
        let tx2 = invoke_v1_tx(Felt::from(22_u64), Felt::ZERO, Felt::from(902_u64), base + 1_000);
        let tx3 = invoke_v1_tx(Felt::from(33_u64), Felt::ZERO, Felt::from(903_u64), base + 2_000);

        mempool.accept_tx(tx1.clone()).await.unwrap();
        mempool.accept_tx(tx2.clone()).await.unwrap();
        mempool.accept_tx(tx3.clone()).await.unwrap();

        let result = rpc
            .flush_mempool_txns(FlushMempoolTxnsParams {
                transaction_hashes: Some(vec![tx2.hash, Felt::from(999_u64)]),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(result.removed_transaction_hashes, vec![tx2.hash]);
        let remaining = mempool.snapshot_transactions_matching(0, usize::MAX, false, |_| true).await;
        assert_eq!(remaining.into_iter().map(|tx| tx.transaction.hash).collect::<Vec<_>>(), vec![tx1.hash, tx3.hash]);
    }

    #[tokio::test]
    async fn flush_mempool_txns_can_filter_by_nonce_range_when_all_is_explicit() {
        let (mempool, rpc) = make_starknet_with_mempool();
        let base = TxTimestamp::now().0;
        let tx1 = invoke_v1_tx(Felt::from(11_u64), Felt::ZERO, Felt::from(1001_u64), base);
        let tx2 = invoke_v1_tx(Felt::from(22_u64), Felt::from(2_u64), Felt::from(1002_u64), base + 1_000);
        let tx3 = invoke_v1_tx(Felt::from(33_u64), Felt::from(4_u64), Felt::from(1003_u64), base + 2_000);

        mempool.accept_tx(tx1.clone()).await.unwrap();
        mempool.accept_tx(tx2.clone()).await.unwrap();
        mempool.accept_tx(tx3.clone()).await.unwrap();

        let result = rpc
            .flush_mempool_txns(FlushMempoolTxnsParams {
                all: true,
                nonce_filter: MempoolNonceFilter {
                    nonce_after: Some(Felt::from(1_u64)),
                    nonce_before: Some(Felt::from(4_u64)),
                },
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(result.removed_transaction_hashes, vec![tx2.hash]);
        let remaining = mempool.snapshot_transactions_matching(0, usize::MAX, false, |_| true).await;
        assert_eq!(remaining.into_iter().map(|tx| tx.transaction.hash).collect::<Vec<_>>(), vec![tx1.hash, tx3.hash]);
    }

    #[tokio::test]
    async fn flush_mempool_txns_rejects_nonce_only_requests_without_base_filter() {
        let (_, rpc) = make_starknet_with_mempool();

        let err = rpc
            .flush_mempool_txns(FlushMempoolTxnsParams {
                nonce_filter: MempoolNonceFilter {
                    nonce_after: Some(Felt::from(1_u64)),
                    nonce_before: Some(Felt::from(4_u64)),
                },
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert_eq!(err.code(), jsonrpsee::types::ErrorCode::InvalidParams.code());
        assert_eq!(
            err.message(),
            "Nonce filters only narrow an explicit base flush filter: all, contract_address, or transaction_hashes"
        );
    }
}
