//! Madara settlement client. This crate manages communication between Madara nodes and their
//! settlement layer (Ethereum L1 or Starknet L2), synchronizing state updates, processing
//! cross-chain messages, and monitoring gas prices.
//!
//! # Overview
//!
//! The settlement client acts as a bridge between Madara and its settlement layer, ensuring
//! that the L2/L3 chain remains synchronized with L1/L2 state. It handles state verification,
//! message passing and gas calculation through specialized workers that operate concurrently.
//!
//! The client is split into two main layers: the outer [`L1ClientImpl`] coordinates workers
//! and manages persistence, while the inner [`SettlementLayerProvider`] implementations
//! handle protocol-specific communication with Ethereum or Starknet.
//!
//! # Settlement Layer Abstraction
//!
//! The [`SettlementLayerProvider`] trait provides a unified interface for different settlement
//! layers:
//!
//! - **Ethereum**: Connects to L1 via JSON-RPC, monitoring the Starknet Core Contract.
//! - **Starknet**: Connects to L2 via JSON-RPC, monitoring an appchain contract.
//!
//! Each implementation handles protocol-specific details like event formats, gas calculations,
//! and message hashing algorithms.
//!
//! # State Synchronization
//!
//! State updates flow from the settlement layer to Madara through a dedicated worker:
//!
//! 1. **Initial state retrieval**: On startup, fetch the latest confirmed state.
//! 2. **Event subscription**: Listen for `LogStateUpdate` events from the core contract.
//! 3. **State validation**: Verify global root, block hash, and block number.
//! 4. **Database update**: Persist the confirmed state for block production.
//! 5. **Metrics update**: Record L1 block number and timing information.
//!
//! The state update worker ensures Madara always knows the latest L1-confirmed state,
//! which it uses for block production, block sync and certain RPC methods.
//!
//! # L1→L2 Message Processing
//!
//! Messages from L1 to L2 are processed as follows:
//!
//! 1. **Event monitoring**: Watch for `LogMessageToL2` (Ethereum) or `MessageSent` (Starknet) events.
//! 2. **Message validation**: Check if the message is:
//!    - Not already processed (check nonce in database).
//!    - Not cancelled (query settlement layer).
//!    - Still pending in the core contract.
//! 3. **Database storage**: Add valid messages to the pending queue.
//! 4. **Consumer notification**: Wake up any waiting message consumers.
//! 5. **Block inclusion**: Messages are consumed during block production.
//!
//! # Message Cancellation Handling
//!
//! The client supports L1→L2 message cancellation as per the following flow:
//!
//! 1. **Cancellation detection**: Check for `MessageToL2Canceled` or cancellation timestamps.
//! 2. **Message filtering**: Skip cancelled messages during synchronization.
//! 3. **Database cleanup**: Remove cancelled messages from the pending queue.
//!
//! Cancellation checks happen both during initial sync and before message consumption
//! to ensure cancelled messages never enter blocks.
//!
//! # Gas Price Monitoring
//!
//! Gas prices are updated through a dedicated worker:
//!
//! 1. **Settlement layer query**: Fetch current gas prices and blob fees.
//! 2. **Oracle consultation**: Get STRK/ETH exchange rate if not fixed.
//! 3. **Price calculation**: Compute L1 gas quote for transaction pricing.
//! 4. **Database update**: Store latest prices for transaction processing.
//!
//! The gas price worker polls at regular intervals with retry logic and timeout protection to
//! ensure price feeds remain up-to-date.
//!
//! # Worker Coordination
//!
//! The settlement client runs three concurrent workers managed by [`run_sync_worker`]:
//!
//! ## State Update Worker
//!
//! - Monitors settlement layer for state updates
//! - Updates L1 head state for other components
//! - Manages block confirmation status
//!
//! ## Messaging Worker
//!
//! - Syncs L1→L2 messages from settlement layer
//! - Validates message status and handles cancellations
//! - Maintains message ordering by nonce
//!
//! ## Gas Price Worker
//!
//! - Polls settlement layer for gas prices
//! - Queries price oracles for exchange rates
//! - Updates gas pricing for transaction fees
//!
//! All workers share a [`ServiceContext`] for coordinated shutdown.
//!
//! # Reading from the Settlement Client
//!
//! The client provides several interfaces for reading data:
//!
//! ## Message Consumption via SettlementClient
//!
//! The [`SettlementClient`] trait provides a stream of pending L1→L2 messages for block
//! production. Messages are validated before delivery and removed from the pending queue
//! once consumed.
//!
//! ```no_run
//! let mut consumer = settlement_client.create_message_to_l2_consumer();
//! while let Some(Ok(message)) = consumer.next().await {
//!     // Include message in block
//! }
//! ```
//!
//! ## State Updates via L1HeadReceiver
//!
//! The `L1HeadReceiver` watch channel broadcasts the latest L1-confirmed state. Components
//! can subscribe to track the L1 head without polling.
//!
//! ```no_run
//! // Get the current L1 head state
//! let current_state = l1_head_receiver.borrow();
//! if let Some(state) = current_state.as_ref() {
//!     println!("Current L1 block: {:?}", state.block_number);
//! }
//!
//! // Wait for state updates
//! while l1_head_receiver.changed().await.is_ok() {
//!     let state_update = l1_head_receiver.borrow();
//!     if let Some(state) = state_update.as_ref() {
//!         // React to new L1 confirmed state
//!         println!("New L1 state: block #{:?}, root: {}",
//!                  state.block_number, state.global_root);
//!     }
//! }
//! ```
//!
//! ## Direct Provider Access
//!
//! The underlying [`SettlementLayerProvider`] can be accessed directly for custom queries
//! like checking message cancellation status or calculating message hashes.
//!
//! [`L1ClientImpl`]: crate::L1ClientImpl
//! [`SettlementLayerProvider`]: crate::client::SettlementLayerProvider
//! [`SettlementClient`]: crate::SettlementClient
//! [`run_sync_worker`]: crate::L1ClientImpl::run_sync_worker
//! [`SettlementClientError`]: crate::error::SettlementClientError
//! [`ServiceContext`]: mp_utils::service::ServiceContext

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

    pub fn provider(&self) -> Arc<dyn SettlementLayerProvider> {
        self.provider.clone()
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
