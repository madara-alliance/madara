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
    async fn revert_to(&self, block_hash: Felt) -> RpcResult<()> {
        // Check if unsafe RPC methods are enabled
        if !self.rpc_unsafe_enabled {
            return Err(StarknetRpcApiError::ErrUnexpectedError {
                error: "This method requires the --rpc-unsafe flag to be enabled".to_string().into(),
            }
            .into());
        }

        // Get the block number for the target hash
        let target_block_n = self.backend.db.find_block_hash(&block_hash)
            .context("Failed to find block number for revert target")
            .map_err(StarknetRpcApiError::from)?
            .ok_or_else(|| StarknetRpcApiError::ErrUnexpectedError {
                error: format!("Block with hash {:#x} not found", block_hash).into()
            })?;

        // Check if snap sync was used and if target block is before snap sync range
        if let Some(snap_sync_latest_block) = self.backend.get_snap_sync_latest_block()
            .context("Failed to check snap sync status")
            .map_err(StarknetRpcApiError::from)?
        {
            if target_block_n < snap_sync_latest_block {
                return Err(StarknetRpcApiError::ErrUnexpectedError {
                    error: format!(
                        "Cannot revert to block {} because snap sync was used up to block {}. Trie data is only available from block {} onwards.",
                        target_block_n, snap_sync_latest_block, snap_sync_latest_block
                    ).into()
                }.into());
            }
        }

        self.backend.revert_to(&block_hash).map_err(StarknetRpcApiError::from)?;
        let fresh_chain_tip = self
            .backend
            .db
            .get_chain_tip()
            .context("Failed to get chain tip after revert")
            .map_err(StarknetRpcApiError::from)?;
        let backend_chain_tip = mc_db::ChainTip::from_storage(fresh_chain_tip);
        self.backend.chain_tip.send_replace(backend_chain_tip);
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
