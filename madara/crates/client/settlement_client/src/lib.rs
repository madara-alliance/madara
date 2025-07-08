use crate::{
    client::SettlementLayerProvider,
    eth::{EthereumClient, EthereumClientConfig},
    messages_to_l2_consumer::MessagesToL2Consumer,
    starknet::{StarknetClient, StarknetClientConfig},
};
use anyhow::Context;
use futures::{
    stream::{self, BoxStream},
    StreamExt,
};
use mc_db::MadaraBackend;
use mp_transactions::L1HandlerTransactionWithFee;
use std::sync::Arc;
use tokio::sync::Notify;
use url::Url;

mod client;
pub mod error;
mod eth;
pub mod gas_price;
mod messages_to_l2_consumer;
mod messaging;
pub(crate) mod starknet;
pub mod state_update;
pub mod sync;
mod utils;

/// Interface consumed by downstream crates for opeations that they may want to do with the settlement layer.
pub trait SettlementClient: Send + Sync + 'static {
    /// Create a stream consuming pending messages to l2.
    fn create_message_to_l2_consumer(&self) -> BoxStream<'static, anyhow::Result<L1HandlerTransactionWithFee>>;
}

pub struct L1ClientImpl {
    provider: Arc<dyn SettlementLayerProvider>,
    backend: Arc<MadaraBackend>,
    notify_new_message_to_l2: Arc<Notify>,
}

impl L1ClientImpl {
    fn new(backend: Arc<MadaraBackend>, provider: Arc<dyn SettlementLayerProvider>) -> Self {
        Self { provider, backend, notify_new_message_to_l2: Default::default() }
    }

    pub async fn new_ethereum(
        backend: Arc<MadaraBackend>,
        rpc_url: Url,
        core_contract_address: String,
    ) -> anyhow::Result<Self> {
        let provider = EthereumClient::new(EthereumClientConfig { rpc_url, core_contract_address })
            .await
            .context("Creating ethereum client")?;
        Ok(Self::new(backend, Arc::new(provider)))
    }

    pub async fn new_starknet(
        backend: Arc<MadaraBackend>,
        rpc_url: Url,
        core_contract_address: String,
    ) -> anyhow::Result<Self> {
        let provider = StarknetClient::new(StarknetClientConfig { rpc_url, core_contract_address })
            .await
            .context("Creating starknet client")?;
        Ok(Self::new(backend, Arc::new(provider)))
    }
}

impl SettlementClient for L1ClientImpl {
    fn create_message_to_l2_consumer(&self) -> BoxStream<'static, anyhow::Result<L1HandlerTransactionWithFee>> {
        let consumer = MessagesToL2Consumer::new(
            self.backend.clone(),
            self.provider.clone(),
            self.notify_new_message_to_l2.clone(),
        );
        stream::unfold(consumer, |mut consumer| async move { Some((consumer.consume_next_or_wait().await, consumer)) })
            .boxed()
    }
}

/// This is the implementation that is used when the L1 sync is disabled.
pub struct L1SyncDisabledClient;
impl SettlementClient for L1SyncDisabledClient {
    fn create_message_to_l2_consumer(&self) -> BoxStream<'static, anyhow::Result<L1HandlerTransactionWithFee>> {
        stream::empty().boxed()
    }
}

#[cfg(feature = "testing")]
#[derive(Clone, Debug)]
pub struct L1ClientMock {
    sender: tokio::sync::mpsc::UnboundedSender<L1HandlerTransactionWithFee>,
    pending_l1_to_l2_messages:
        Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<L1HandlerTransactionWithFee>>>,
}
#[cfg(feature = "testing")]
impl SettlementClient for L1ClientMock {
    fn create_message_to_l2_consumer(&self) -> BoxStream<'static, anyhow::Result<L1HandlerTransactionWithFee>> {
        stream::unfold(self.pending_l1_to_l2_messages.clone(), |recv| async move {
            let tx = recv.lock().await.recv().await;
            tx.map(|tx| (Ok(tx), recv))
        })
        .boxed()
    }
}
#[cfg(feature = "testing")]
impl Default for L1ClientMock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "testing")]
impl L1ClientMock {
    pub fn new() -> Self {
        let (sender, recv) = tokio::sync::mpsc::unbounded_channel();
        Self { sender, pending_l1_to_l2_messages: tokio::sync::Mutex::new(recv).into() }
    }
    pub fn add_tx(&self, tx: L1HandlerTransactionWithFee) {
        self.sender.send(tx).unwrap()
    }
}
