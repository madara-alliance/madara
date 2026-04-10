use crate::{versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use anyhow::Context;
use jsonrpsee::core::{async_trait, RpcResult};
use mc_db::MadaraStorageRead;
use mc_submit_tx::{SubmitL1HandlerTransaction, SubmitTransaction};
use mp_block::header::CustomHeader;
use mp_convert::Felt;
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_10_2::BroadcastedInvokeTxn;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{L1HandlerTransactionResult, L1HandlerTransactionWithFee};
use mp_utils::service::{MadaraServiceId, MadaraServiceStatus, SERVICE_GRACE_PERIOD};
use std::time::Duration;
use tokio::time::Instant;

const REVERT_STOP_WAIT_EXTRA: Duration = Duration::from_secs(5);
const REVERT_STOP_LOG_INTERVAL: Duration = Duration::from_secs(1);
const REVERT_STOP_POLL_INTERVAL: Duration = Duration::from_millis(200);
const REVERT_SHUTDOWN_DELAY: Duration = Duration::from_millis(100);

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

        let fresh_chain_tip = self
            .backend
            .db
            .get_chain_tip()
            .context("Failed to get chain tip after revert")
            .map_err(StarknetRpcApiError::from)?;
        let backend_chain_tip = mc_db::ChainTip::from_storage(fresh_chain_tip);
        self.backend.chain_tip.send_replace(backend_chain_tip);

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

        if let Some(preconfirmed) = self.backend.block_view_on_preconfirmed() {
            // Replay may resend the same header after a restart to restore the in-memory
            // expected hash, but once PRE_CONFIRMED exists we must not mutate the values
            // that were already used to create the execution context.
            if preconfirmed.block_number() == custom_block_headers.block_n
                && (preconfirmed.header().block_timestamp.0 != custom_block_headers.timestamp
                    || preconfirmed.header().gas_prices != custom_block_headers.gas_prices)
            {
                return Err(StarknetRpcApiError::ErrUnexpectedError {
                    error: format!(
                        "Cannot change custom header for block {} after PRE_CONFIRMED has started",
                        custom_block_headers.block_n
                    )
                    .into(),
                }
                .into());
            }
        }

        self.backend.set_custom_header(custom_block_headers);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::services_to_stop_for_revert;
    use crate::{
        test_utils::TestTransactionProvider, versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server, Starknet,
    };
    use mc_db::{
        preconfirmed::PreconfirmedBlock,
        test_utils::{add_test_block, l1_handler_tx_with_receipt},
        MadaraBackend,
    };
    use mp_block::header::{CustomHeader, GasPrices, PreconfirmedHeader};
    use mp_chain_config::ChainConfig;
    use mp_convert::Felt;
    use mp_utils::rayon::global_spawn_rayon_task;
    use mp_utils::service::{MadaraServiceMask, MadaraServiceStatus, ServiceContext};
    use std::sync::Arc;
    use std::time::Duration;

    fn make_starknet(backend: Arc<MadaraBackend>, ctx: ServiceContext) -> Starknet {
        let mut rpc = Starknet::new(backend, Arc::new(TestTransactionProvider), Default::default(), None, ctx);
        rpc.set_rpc_unsafe_enabled(true);
        rpc
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
    async fn set_block_header_accepts_matching_open_preconfirmed_block() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        add_test_block(&backend, 0, vec![]);
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
        global_spawn_rayon_task({
            let backend = backend.clone();
            let custom_header = custom_header.clone();
            move || {
                backend.write_access().new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                    block_number: 1,
                    block_timestamp: custom_header.timestamp.into(),
                    gas_prices: custom_header.gas_prices.clone(),
                    ..Default::default()
                }))
            }
        })
        .await
        .expect("new preconfirmed block should succeed");

        let rpc = make_starknet(backend.clone(), ServiceContext::default());

        rpc.set_block_header(custom_header.clone()).await.expect("set block header should succeed");

        assert_eq!(backend.get_custom_header(custom_header.block_n), Some(custom_header));
    }

    #[tokio::test]
    async fn set_block_header_rejects_mismatched_open_preconfirmed_block() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        add_test_block(&backend, 0, vec![]);
        global_spawn_rayon_task({
            let backend = backend.clone();
            move || {
                backend.write_access().new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                    block_number: 1,
                    ..Default::default()
                }))
            }
        })
        .await
        .expect("new preconfirmed block should succeed");

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

        let err = rpc
            .set_block_header(custom_header.clone())
            .await
            .expect_err("set block header should fail when PRE_CONFIRMED header differs");

        let preconfirmed = backend.block_view_on_preconfirmed().expect("preconfirmed block should exist");

        assert_ne!(err.code(), 0);
        assert_eq!(preconfirmed.block_number(), custom_header.block_n);
        assert_eq!(preconfirmed.header().block_timestamp.0, 0);
        assert_eq!(preconfirmed.header().gas_prices, GasPrices::default());
        assert_eq!(backend.get_custom_header(custom_header.block_n), None);
    }
}
