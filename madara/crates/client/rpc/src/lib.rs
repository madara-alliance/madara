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
use mc_db::MadaraBackend;
use mc_submit_tx::SubmitTransaction;
use mp_utils::service::ServiceContext;
use std::sync::Arc;

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
    /// Stale handles are removed each time a subscription is dropped to keep the backing map from
    /// growing to an unbounded size. Note that there is no hard upper limit on the size of the map,
    /// other than those set in the RPC middleware, but at least this way we clean up connections on
    /// close.
    ///
    /// ## Thread Safety
    ///
    /// From the [DashMap] docs:
    ///
    /// > Documentation mentioning locking behaviour acts in the reference frame of the calling
    /// > thread. This means that it is safe to ignore it across multiple threads.
    ///
    /// And from [DashMap::entry]:
    ///
    /// > Locking behaviour: May deadlock if called when holding any sort of reference into the map.
    ///
    /// This is fine in our case as we do not maintain references to a map in the same thread while
    /// mutating it and instead operate directly on-value by sharing the map inside an [Arc].
    ///
    /// [DashMap]: dashmap::DashMap
    /// [DashMap::entry]: dashmap::DashMap::entry
    /// [Arc]: std::sync::Arc
    handles: std::sync::Arc<dashmap::DashMap<u64, std::sync::Arc<tokio::sync::Notify>>>,
}

impl WsSubscribeHandles {
    pub fn new() -> Self {
        Self { handles: std::sync::Arc::new(dashmap::DashMap::new()) }
    }

    pub async fn subscription_register(&self, id: jsonrpsee::types::SubscriptionId<'static>) -> WsSubscriptionGuard {
        let id = match id {
            jsonrpsee::types::SubscriptionId::Num(id) => id,
            jsonrpsee::types::SubscriptionId::Str(_) => {
                unreachable!("Jsonrpsee middleware has been configured to use u64 subscription ids")
            }
        };

        let handle = std::sync::Arc::new(tokio::sync::Notify::new());
        let map = std::sync::Arc::clone(&self.handles);

        self.handles.insert(id, std::sync::Arc::clone(&handle));

        WsSubscriptionGuard { id, handle, map }
    }

    pub async fn subscription_close(&self, id: u64) -> bool {
        if let Some((_, handle)) = self.handles.remove(&id) {
            handle.notify_one();
            true
        } else {
            false
        }
    }
}

pub(crate) struct WsSubscriptionGuard {
    id: u64,
    handle: std::sync::Arc<tokio::sync::Notify>,
    map: std::sync::Arc<dashmap::DashMap<u64, std::sync::Arc<tokio::sync::Notify>>>,
}

impl WsSubscriptionGuard {
    pub async fn cancelled(&self) {
        self.handle.notified().await
    }
}

impl Drop for WsSubscriptionGuard {
    fn drop(&mut self) {
        self.map.remove(&self.id);
    }
}
