#![doc = include_str!("crate_docs.md")]

#[cfg(test)]
pub mod test_utils;
pub mod utils;
pub mod versions;

mod block_id;
mod constants;
mod errors;
mod metrics;
mod types;

use jsonrpsee::RpcModule;
use mc_db::MadaraBackend;
use mc_mempool::{
    Mempool, PreConfirmationStatus, TransactionStatus as MempoolTransactionStatus, WatchTransactionStatus,
};
use mc_submit_tx::{SubmitTransaction, TransactionLookup};
use mp_transactions::{validated::ValidatedTransaction, Transaction};
use mp_utils::service::ServiceContext;
use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicU64, atomic::Ordering, Arc},
    time::Instant,
};

pub use errors::{StarknetRpcApiError, StarknetRpcResult};

#[derive(Debug, Default)]
pub struct StarknetSubscriptionIdProvider {
    next: AtomicU64,
}

impl jsonrpsee::server::IdProvider for StarknetSubscriptionIdProvider {
    fn next_id(&self) -> jsonrpsee::types::SubscriptionId<'static> {
        self.next.fetch_add(1, Ordering::Relaxed).to_string().into()
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxStatusSnapshot {
    Received,
    Candidate,
    PreConfirmed,
    AcceptedOnL2,
    AcceptedOnL1,
}

pub enum TxStatusWatchUpdate {
    Status(Option<TxStatusSnapshot>),
    Closed,
}

pub trait TxStatusWatch: Send {
    fn take_current(&mut self) -> Option<TxStatusSnapshot>;
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = TxStatusWatchUpdate> + Send + '_>>;
}

pub trait TxStatusWatcher: Send + Sync {
    fn watch_transaction_status(&self, transaction_hash: mp_convert::Felt) -> Option<Box<dyn TxStatusWatch + Send>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewTransactionsWatchError {
    Lagged,
}

pub type NewTransactionsWatchOutput = Result<Option<Arc<ValidatedTransaction>>, NewTransactionsWatchError>;
pub type NewTransactionsWatchFuture<'a> = Pin<Box<dyn Future<Output = NewTransactionsWatchOutput> + Send + 'a>>;

pub trait NewTransactionsWatch: Send {
    fn recv(&mut self) -> NewTransactionsWatchFuture<'_>;
}

pub trait NewTransactionsWatcher: Send + Sync {
    fn watch_new_transactions(&self) -> Option<Box<dyn NewTransactionsWatch + Send>>;
}

pub(crate) fn normalize_sender_address_filter(
    sender_address: Option<Vec<starknet_types_core::felt::Felt>>,
) -> Option<HashSet<starknet_types_core::felt::Felt>> {
    sender_address.and_then(|addresses| {
        let addresses = addresses.into_iter().collect::<HashSet<_>>();
        (!addresses.is_empty()).then_some(addresses)
    })
}

pub(crate) fn transaction_matches_sender(
    transaction: &Transaction,
    sender_address: Option<&HashSet<starknet_types_core::felt::Felt>>,
) -> bool {
    let Some(sender_address) = sender_address else {
        return true;
    };
    if sender_address.is_empty() {
        return true;
    }

    match transaction {
        Transaction::Invoke(inner) => sender_address.contains(inner.sender_address()),
        Transaction::L1Handler(inner) => sender_address.contains(&inner.contract_address),
        Transaction::Declare(inner) => sender_address.contains(inner.sender_address()),
        Transaction::Deploy(inner) => sender_address.contains(&inner.calculate_contract_address()),
        Transaction::DeployAccount(inner) => sender_address.contains(&inner.calculate_contract_address()),
    }
}

fn tx_status_snapshot(status: Option<MempoolTransactionStatus>) -> Option<TxStatusSnapshot> {
    match status {
        Some(MempoolTransactionStatus::Preconfirmed(PreConfirmationStatus::Received(_))) => {
            Some(TxStatusSnapshot::Received)
        }
        Some(MempoolTransactionStatus::Preconfirmed(PreConfirmationStatus::Candidate { .. })) => {
            Some(TxStatusSnapshot::Candidate)
        }
        Some(MempoolTransactionStatus::Preconfirmed(PreConfirmationStatus::Executed { .. })) => {
            Some(TxStatusSnapshot::PreConfirmed)
        }
        Some(MempoolTransactionStatus::Confirmed { is_on_l1, .. }) => {
            Some(if is_on_l1 { TxStatusSnapshot::AcceptedOnL1 } else { TxStatusSnapshot::AcceptedOnL2 })
        }
        None => None,
    }
}

impl<D: mc_db::MadaraStorageRead> TxStatusWatch for WatchTransactionStatus<D> {
    fn take_current(&mut self) -> Option<TxStatusSnapshot> {
        let snapshot = tx_status_snapshot(WatchTransactionStatus::current(self).clone());
        WatchTransactionStatus::refresh(self);
        snapshot
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = TxStatusWatchUpdate> + Send + '_>> {
        Box::pin(async move {
            WatchTransactionStatus::recv(self)
                .await
                .map(|status| TxStatusWatchUpdate::Status(tx_status_snapshot(status.clone())))
                .unwrap_or(TxStatusWatchUpdate::Closed)
        })
    }
}

impl<D: mc_db::MadaraStorageRead> TxStatusWatcher for Mempool<D> {
    fn watch_transaction_status(&self, transaction_hash: mp_convert::Felt) -> Option<Box<dyn TxStatusWatch + Send>> {
        let watch = self.watch_transaction_status(transaction_hash).ok()?;
        Some(Box::new(watch))
    }
}

struct BroadcastNewTransactionsWatch {
    receiver: tokio::sync::broadcast::Receiver<Arc<ValidatedTransaction>>,
}

impl NewTransactionsWatch for BroadcastNewTransactionsWatch {
    fn recv(&mut self) -> NewTransactionsWatchFuture<'_> {
        Box::pin(async move {
            match self.receiver.recv().await {
                Ok(tx) => Ok(Some(tx)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => Err(NewTransactionsWatchError::Lagged),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => Ok(None),
            }
        })
    }
}

impl<D: mc_db::MadaraStorageRead + mc_db::MadaraStorageWrite> NewTransactionsWatcher for Mempool<D> {
    fn watch_new_transactions(&self) -> Option<Box<dyn NewTransactionsWatch + Send>> {
        Some(Box::new(BroadcastNewTransactionsWatch { receiver: self.subscribe_new_transactions() }))
    }
}

/// A Starknet RPC server for Madara
#[derive(Clone)]
pub struct Starknet {
    backend: Arc<MadaraBackend>,
    pub(crate) mempool: Option<Arc<Mempool>>,
    ws_handles: Arc<WsSubscribeHandles>,
    pub(crate) pre_v0_9_preconfirmed_as_pending: bool,
    pub(crate) transaction_submitter: Arc<dyn SubmitTransaction>,
    pub(crate) transaction_lookup: Arc<dyn TransactionLookup>,
    pub(crate) tx_status_watcher: Option<Arc<dyn TxStatusWatcher>>,
    pub(crate) new_transactions_watcher: Option<Arc<dyn NewTransactionsWatcher>>,
    storage_proof_config: StorageProofConfig,
    pub(crate) block_prod_handle: Option<mc_block_production::BlockProductionHandle>,
    pub ctx: ServiceContext,
    pub(crate) rpc_unsafe_enabled: bool,
}

impl Starknet {
    pub fn new(
        backend: Arc<MadaraBackend>,
        transaction_submitter: Arc<dyn SubmitTransaction>,
        transaction_lookup: Arc<dyn TransactionLookup>,
        storage_proof_config: StorageProofConfig,
        block_prod_handle: Option<mc_block_production::BlockProductionHandle>,
        ctx: ServiceContext,
    ) -> Self {
        let ws_handles = Arc::new(WsSubscribeHandles::new());
        Self {
            backend,
            mempool: None,
            ws_handles,
            transaction_submitter,
            transaction_lookup,
            tx_status_watcher: None,
            new_transactions_watcher: None,
            storage_proof_config,
            block_prod_handle,
            ctx,
            pre_v0_9_preconfirmed_as_pending: false,
            rpc_unsafe_enabled: false,
        }
    }

    pub fn set_pre_v0_9_preconfirmed_as_pending(&mut self, value: bool) {
        self.pre_v0_9_preconfirmed_as_pending = value;
    }

    pub fn set_rpc_unsafe_enabled(&mut self, value: bool) {
        self.rpc_unsafe_enabled = value;
    }

    pub fn set_tx_status_watcher(&mut self, watcher: Option<Arc<dyn TxStatusWatcher>>) {
        self.tx_status_watcher = watcher;
    }

    pub fn set_new_transactions_watcher(&mut self, watcher: Option<Arc<dyn NewTransactionsWatcher>>) {
        self.new_transactions_watcher = watcher;
    }

    pub fn set_mempool(&mut self, mempool: Arc<Mempool>) {
        self.mempool = Some(mempool);
    }

    #[cfg(test)]
    pub(crate) fn active_ws_subscription_count(&self) -> usize {
        self.ws_handles.handles.len()
    }
}

/// Returns the RpcModule merged with all the supported RPC versions.
pub fn rpc_api_user(starknet: &Starknet) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    rpc_api.merge(versions::user::v0_7_1::StarknetReadRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_7_1::StarknetTraceRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_1::StarknetWriteRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_8_1::StarknetTraceRpcApiV0_8_1Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_9_0::StarknetReadRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_9_0::StarknetWriteRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_9_0::StarknetTraceRpcApiV0_9_0Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_10_0::StarknetReadRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetWriteRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_0::StarknetTraceRpcApiV0_10_0Server::into_rpc(starknet.clone()))?;

    rpc_api.merge(versions::user::v0_10_2::StarknetReadRpcApiV0_10_2Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_2::StarknetWriteRpcApiV0_10_2Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::user::v0_10_2::StarknetTraceRpcApiV0_10_2Server::into_rpc(starknet.clone()))?;

    Ok(rpc_api)
}

pub fn rpc_api_admin(starknet: &Starknet) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    rpc_api.merge(versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    if starknet.rpc_unsafe_enabled {
        rpc_api.merge(versions::admin::v0_1_0::MadaraMempoolRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    }
    rpc_api.merge(versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraServicesRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;
    rpc_api.merge(versions::admin::v0_1_0::MadaraReadRpcApiV0_1_0Server::into_rpc(starknet.clone()))?;

    Ok(rpc_api)
}

struct WsSubscriptionHandle {
    cancelled: tokio::sync::watch::Sender<bool>,
}

impl WsSubscriptionHandle {
    fn new() -> (Self, tokio::sync::watch::Receiver<bool>) {
        let (cancelled, receiver) = tokio::sync::watch::channel(false);
        (Self { cancelled }, receiver)
    }

    fn cancel(&self) {
        let _ = self.cancelled.send(true);
    }

    #[cfg(test)]
    async fn cancelled(&self) {
        let mut cancelled = self.cancelled.subscribe();
        while !*cancelled.borrow_and_update() {
            if cancelled.changed().await.is_err() {
                return;
            }
        }
    }
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
    handles: std::sync::Arc<dashmap::DashMap<u64, std::sync::Arc<WsSubscriptionHandle>>>,
    counts_by_method: std::sync::Arc<dashmap::DashMap<&'static str, u64>>,
}

impl WsSubscribeHandles {
    pub fn new() -> Self {
        Self {
            handles: std::sync::Arc::new(dashmap::DashMap::new()),
            counts_by_method: std::sync::Arc::new(dashmap::DashMap::new()),
        }
    }

    // FIXME(subscriptions): Remove this #[allow(unused)] once subscriptions are back.
    #[allow(unused)]
    pub async fn subscription_register(
        &self,
        id: jsonrpsee::types::SubscriptionId<'static>,
        method: &'static str,
    ) -> WsSubscriptionGuard {
        let id = match id {
            jsonrpsee::types::SubscriptionId::Num(id) => id,
            jsonrpsee::types::SubscriptionId::Str(id) => {
                id.parse().expect("Starknet subscription ids should be numeric strings")
            }
        };

        let (handle, cancelled) = WsSubscriptionHandle::new();
        let handle = std::sync::Arc::new(handle);
        let map = std::sync::Arc::clone(&self.handles);

        self.handles.insert(id, std::sync::Arc::clone(&handle));
        let method_count = self.increment_method_count(method);
        let metrics = crate::metrics::ws_metrics();
        metrics.record_subscription_opened(method);
        metrics.record_active_subscriptions(self.handles.len() as u64);
        metrics.record_active_subscriptions_for_method(method, method_count);
        tracing::info!(
            "WS subscription opened: method={} subscription_id={} active_subscriptions={} active_method_subscriptions={}",
            method,
            id,
            self.handles.len(),
            method_count
        );

        WsSubscriptionGuard {
            id,
            method,
            opened_at: Instant::now(),
            _handle: handle,
            cancelled,
            map,
            counts_by_method: std::sync::Arc::clone(&self.counts_by_method),
        }
    }

    pub async fn subscription_close(&self, id: u64) -> bool {
        if let Some((_, handle)) = self.handles.remove(&id) {
            tracing::info!("WS subscription close requested: subscription_id={} reason=starknet_unsubscribe", id);
            handle.cancel();
            true
        } else {
            tracing::warn!(
                "WS subscription close requested for unknown subscription: subscription_id={} reason=starknet_unsubscribe",
                id
            );
            false
        }
    }

    fn increment_method_count(&self, method: &'static str) -> u64 {
        let mut count = self.counts_by_method.entry(method).or_insert(0);
        *count += 1;
        *count
    }
}

pub(crate) struct WsSubscriptionGuard {
    id: u64,
    method: &'static str,
    opened_at: Instant,
    // Keep the registered handle alive until this guard is dropped.
    _handle: std::sync::Arc<WsSubscriptionHandle>,
    cancelled: tokio::sync::watch::Receiver<bool>,
    map: std::sync::Arc<dashmap::DashMap<u64, std::sync::Arc<WsSubscriptionHandle>>>,
    counts_by_method: std::sync::Arc<dashmap::DashMap<&'static str, u64>>,
}

impl WsSubscriptionGuard {
    pub async fn cancelled(&self) {
        let mut cancelled = self.cancelled.clone();
        while !*cancelled.borrow_and_update() {
            if cancelled.changed().await.is_err() {
                return;
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }
}

pub(crate) async fn close_ws_subscription(
    starknet: &Starknet,
    subscription_id: jsonrpsee::types::SubscriptionId<'_>,
    parse_error_context: &'static str,
) -> Result<(), errors::StarknetWsApiError> {
    use crate::errors::ErrorExtWs;

    let subscription_id = match subscription_id {
        jsonrpsee::types::SubscriptionId::Num(id) => id,
        jsonrpsee::types::SubscriptionId::Str(id) => id.parse().or_internal_server_error(parse_error_context)?,
    };

    let _ = starknet.ws_handles.subscription_close(subscription_id).await;
    Ok(())
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum LiveConfirmedHeadResolution {
    Block(Box<mp_block::MadaraBlockInfo>),
    Reorg(mc_db::ReorgNotification),
    RetryBackfill,
}

pub(crate) fn try_recv_live_reorg(
    reorgs: &mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>,
    missed_reorg_error: impl FnOnce() -> errors::StarknetWsApiError,
) -> Result<Option<mc_db::ReorgNotification>, errors::StarknetWsApiError> {
    match reorgs.try_recv() {
        Ok(reorg) => Ok(Some(reorg)),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => Err(missed_reorg_error()),
        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => Err(errors::StarknetWsApiError::Internal),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => Ok(None),
    }
}

pub(crate) fn resolve_live_confirmed_head(
    backend: &std::sync::Arc<mc_db::MadaraBackend>,
    reorgs: &mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>,
    next_block_n: u64,
    missed_reorg_error: impl FnOnce() -> errors::StarknetWsApiError,
) -> Result<LiveConfirmedHeadResolution, errors::StarknetWsApiError> {
    use crate::errors::ErrorExtWs;

    if let Some(reorg) = try_recv_live_reorg(reorgs, missed_reorg_error)? {
        return Ok(LiveConfirmedHeadResolution::Reorg(reorg));
    }

    let Some(block_view) = backend.block_view_on_confirmed(next_block_n) else {
        return Ok(LiveConfirmedHeadResolution::RetryBackfill);
    };
    let block_info = block_view
        .get_block_info()
        .or_else_internal_server_error(|| format!("Failed to retrieve block info for block {next_block_n}"))?;

    if block_info.header.block_number != next_block_n {
        let err = format!("Retrieved mismatched block {}, expected {next_block_n}", block_info.header.block_number);
        return Err(errors::StarknetWsApiError::internal_server_error(err));
    }

    Ok(LiveConfirmedHeadResolution::Block(Box::new(block_info)))
}

impl Drop for WsSubscriptionGuard {
    fn drop(&mut self) {
        self.map.remove(&self.id);
        let method_count = if let Some(mut count) = self.counts_by_method.get_mut(self.method) {
            *count = count.saturating_sub(1);
            *count
        } else {
            0
        };
        let age = self.opened_at.elapsed();
        let metrics = crate::metrics::ws_metrics();
        metrics.record_subscription_closed(self.method);
        metrics.record_subscription_duration(self.method, age.as_secs_f64());
        metrics.record_active_subscriptions(self.map.len() as u64);
        metrics.record_active_subscriptions_for_method(self.method, method_count);
        tracing::info!(
            "WS subscription closed: method={} subscription_id={} age_secs={} active_subscriptions={} active_method_subscriptions={}",
            self.method,
            self.id,
            age.as_secs(),
            self.map.len(),
            method_count
        );
    }
}

#[cfg(test)]
mod tests;
