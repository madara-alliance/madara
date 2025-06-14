//! Starknet RPC server API implementation
//!
//! It uses the madara client and backend in order to answer queries.

mod constants;
mod errors;
#[cfg(test)]
pub mod test_utils;
mod types;
pub mod utils;
pub mod versions;

use jsonrpsee::RpcModule;
use mc_db::db_block_id::DbBlockIdResolvable;
use mc_db::MadaraBackend;
use mc_submit_tx::SubmitTransaction;
use mp_block::{BlockId, BlockTag, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo};
use mp_chain_config::ChainConfig;
use mp_convert::ToFelt;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use utils::ResultExt;

pub use errors::{StarknetRpcApiError, StarknetRpcResult};

/// Limits to the storage proof endpoint.
#[derive(Clone, Debug)]
pub struct StorageProofConfig {
    /// Max keys that cna be used in a storage proof.
    pub max_keys: usize,
    /// Max tries that can be used in a storage proof.
    pub max_tries: usize,
    /// How many blocks in the past can we get a storage proof for.
    pub max_distance: u64,
}

impl Default for StorageProofConfig {
    fn default() -> Self {
        Self { max_keys: 1024, max_tries: 5, max_distance: 0 }
    }
}

/// A Starknet RPC server for Madara
#[derive(Clone)]
pub struct Starknet {
    backend: Arc<MadaraBackend>,
    ws_handles: Arc<WsSubscribeHandles>,
    pub(crate) add_transaction_provider: Arc<dyn SubmitTransaction>,
    storage_proof_config: StorageProofConfig,
    pub(crate) block_prod_handle: Option<mc_block_production::BlockProductionHandle>,
    pub ctx: ServiceContext,
}

impl Starknet {
    pub fn new(
        backend: Arc<MadaraBackend>,
        add_transaction_provider: Arc<dyn SubmitTransaction>,
        storage_proof_config: StorageProofConfig,
        block_prod_handle: Option<mc_block_production::BlockProductionHandle>,
        ctx: ServiceContext,
    ) -> Self {
        let ws_handles = Arc::new(WsSubscribeHandles::new());
        Self { backend, ws_handles, add_transaction_provider, storage_proof_config, block_prod_handle, ctx }
    }

    pub fn clone_backend(&self) -> Arc<MadaraBackend> {
        Arc::clone(&self.backend)
    }

    pub fn clone_chain_config(&self) -> Arc<ChainConfig> {
        Arc::clone(self.backend.chain_config())
    }

    pub fn get_block_info(
        &self,
        block_id: &impl DbBlockIdResolvable,
    ) -> StarknetRpcResult<MadaraMaybePendingBlockInfo> {
        self.backend
            .get_block_info(block_id)
            .or_internal_server_error("Error getting block from storage")?
            .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    pub fn get_block_n(&self, block_id: &impl DbBlockIdResolvable) -> StarknetRpcResult<u64> {
        self.backend
            .get_block_n(block_id)
            .or_internal_server_error("Error getting block from storage")?
            .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    pub fn get_block(&self, block_id: &impl DbBlockIdResolvable) -> StarknetRpcResult<MadaraMaybePendingBlock> {
        self.backend
            .get_block(block_id)
            .or_internal_server_error("Error getting block from storage")?
            .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    pub fn chain_id(&self) -> Felt {
        self.backend.chain_config().chain_id.clone().to_felt()
    }

    pub fn current_block_number(&self) -> StarknetRpcResult<u64> {
        self.get_block_n(&BlockId::Tag(BlockTag::Latest))
    }

    pub fn get_l1_last_confirmed_block(&self) -> StarknetRpcResult<u64> {
        Ok(self
            .backend
            .get_l1_last_confirmed_block()
            .or_internal_server_error("Error getting L1 last confirmed block")?
            .unwrap_or_default())
    }
}

/// Returns the RpcModule merged with all the supported RPC versions.
pub fn rpc_api_user(starknet: &Starknet) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    rpc_api.merge(versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_0::StarknetReadRpcApiV0_8_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_7_1::StarknetTraceRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_0::StarknetWsRpcApiV0_8_0Server::into_rpc(starknet.clone()))?;

    Ok(rpc_api)
}

pub fn rpc_api_admin(starknet: &Starknet) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    rpc_api.merge(versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraServicesRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;

    Ok(rpc_api)
}

pub(crate) struct WsSubscribeHandles {
    /// Keeps track of all ws connection handles.
    ///
    /// This can be used to request the closure of a ws connection.
    ///
    /// ## Preventing Leaks
    ///
    /// Since connections can be ended abruptly and the jsonrpsee api does not allow use to retrieve
    /// the subscription id in those cases, we periodically remove any dangling handles. This is
    /// done upon call to [`subscription_register`].
    ///
    /// ## Thread Safety
    ///
    /// We do not use a DashMap here as its `iter` method "May deadlock if called when holding a
    /// mutable reference into the map."
    ///
    /// [`subscription_register`]: Self::add_new_handle
    handles: tokio::sync::RwLock<std::collections::HashMap<u64, WsSubscribeContext>>,
}

impl WsSubscribeHandles {
    pub fn new() -> Self {
        Self { handles: tokio::sync::RwLock::new(std::collections::HashMap::new()) }
    }

    pub async fn subscription_register(&self, id: jsonrpsee::types::SubscriptionId<'static>) -> WsSubscribeContext {
        let handle = WsSubscribeContext(std::sync::Arc::new(tokio::sync::Notify::new()));

        let id = match id {
            jsonrpsee::types::SubscriptionId::Num(id) => id,
            jsonrpsee::types::SubscriptionId::Str(_) => {
                unreachable!("Jsonrpsee middleware has been configured to use u64 subscription ids")
            }
        };

        let mut lock = self.handles.write().await;
        lock.retain(|_, handle| std::sync::Arc::strong_count(&handle.0) != 1);
        lock.insert(id, handle.clone());

        handle
    }

    pub async fn subscription_close(&self, id: u64) -> bool {
        if let Some(handle) = self.handles.write().await.remove(&id) {
            handle.0.notify_one();
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
pub(crate) struct WsSubscribeContext(std::sync::Arc<tokio::sync::Notify>);

impl WsSubscribeContext {
    pub async fn run_until_cancelled<T, F>(&self, f: F) -> Option<T>
    where
        T: Sized,
        F: std::future::Future<Output = T>,
    {
        tokio::select! {
            res = f => Some(res),
            _ = self.0.notified() => None
        }
    }
}
