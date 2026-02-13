use crate::{versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};
use anyhow::Context;
use jsonrpsee::core::{async_trait, RpcResult};
use mc_db::MadaraStorageRead;
use mc_submit_tx::{SubmitL1HandlerTransaction, SubmitTransaction};
use mp_block::header::CustomHeader;
use mp_convert::Felt;
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::{L1HandlerTransactionResult, L1HandlerTransactionWithFee};
use mp_utils::service::{MadaraServiceId, MadaraServiceStatus, SERVICE_GRACE_PERIOD};
use std::time::Duration;
use tokio::time::Instant;

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
        let stop_svcs: [MadaraServiceId; 7] = [
            MadaraServiceId::Database,
            MadaraServiceId::L1Sync,
            MadaraServiceId::L2Sync,
            MadaraServiceId::BlockProduction,
            MadaraServiceId::RpcUser,
            MadaraServiceId::Gateway,
            // MadaraServiceId::Telemetry,
            // MadaraServiceId::Analytics,
            MadaraServiceId::Mempool,
        ];

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
        let timeout = SERVICE_GRACE_PERIOD + Duration::from_secs(5);
        let deadline = Instant::now() + timeout;
        let mut last_log = Instant::now();
        let log_interval = Duration::from_secs(1);

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

            tokio::time::sleep(Duration::from_millis(200)).await;
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
        let ctx = self.ctx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            ctx.cancel_global();
        });

        Ok(())
    }

    async fn add_l1_handler_message(
        &self,
        l1_handler_message: L1HandlerTransactionWithFee,
    ) -> RpcResult<L1HandlerTransactionResult> {
        Ok(self
            .block_prod_handle
            .as_ref()
            .unwrap()
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

        self.backend.set_custom_header(custom_block_headers);
        Ok(())
    }
}
