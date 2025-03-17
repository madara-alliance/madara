use crate::error::SettlementClientError;
use crate::gas_price::L1BlockMetrics;
use crate::messaging::L1toL2MessagingEventData;
use crate::state_update::StateUpdate;
use async_trait::async_trait;
use futures::Stream;
use mc_db::l1_db::LastSyncedEventBlock;
use mc_db::MadaraBackend;
#[cfg(test)]
use mockall::automock;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

pub enum ClientType {
    ETH,
    STARKNET,
}

// Test types in a separate module
#[cfg(test)]
pub mod test_types {
    use super::*;
    use futures::stream::BoxStream;

    #[derive(Debug, Default, PartialEq)]
    pub struct DummyConfig;

    pub type DummyStream = BoxStream<'static, Result<L1toL2MessagingEventData, SettlementClientError>>;
}

// Use different automock configurations based on the build type
#[cfg_attr(test, automock(

    type Config = test_types::DummyConfig;
    type StreamType = test_types::DummyStream;
))]
/// A trait defining the interface for settlement layer clients (Ethereum L1, Starknet).
///
/// This trait provides the core functionality needed to:
/// - Monitor and sync state between L1 and L2
/// - Process cross-chain messaging
/// - Verify state updates
/// - Manage gas pricing
///
/// # Type Parameters
///
/// * `Config` - Configuration type specific to the settlement layer implementation
/// * `Error` - Client-specific error type that must be convertible to SettlementClientError
/// * `StreamType` - Stream implementation for processing L1 events
///
/// # Error Handling
///
/// Each implementation should:
/// - Define its own error type that implements `Into<SettlementClientError>`
/// - Use appropriate error variants for different failure scenarios
/// - Provide context in error messages
/// - Handle both client-specific and common error cases
///
/// # Stream Requirements
///
/// The `StreamType` must be a stream that:
/// - Produces `Result<L1toL2MessagingEventData, SettlementClientError>`
/// - Handles gaps in event sequences (via `Option`)
/// - Manages errors during event processing
/// - Implements `Send` for thread safety
///
/// # Implementation Notes
///
/// Implementors should ensure their `StreamType`:
/// - Properly orders events by block number
/// - Handles network interruptions gracefully
/// - Maintains consistency in event processing
/// - Provides backpressure when needed
#[async_trait]
pub trait SettlementClientTrait: Send + Sync {
    /// Configuration type used to initialize the client
    type Config;

    /// Stream type for processing L1 events
    ///
    /// This type represents an asynchronous sequence of L1 events that need to be
    /// processed by the L2 chain. The stream can:
    /// - Return None to indicate end of current batch
    /// - Return Some(Err) for processing/network errors
    /// - Return Some(Ok) for valid events
    type StreamType: Stream<Item = Result<L1toL2MessagingEventData, SettlementClientError>> + Send;

    fn get_client_type(&self) -> ClientType;
    async fn get_latest_block_number(&self) -> Result<u64, SettlementClientError>;
    async fn get_last_event_block_number(&self) -> Result<u64, SettlementClientError>;
    async fn get_last_verified_block_number(&self) -> Result<u64, SettlementClientError>;

    /// Retrieves the last state root from the settlement layer
    ///
    /// TODO: Implementations should convert their native types to Felt.
    /// TODO: Add tests to verify this conversion is correct.
    async fn get_last_verified_state_root(&self) -> Result<Felt, SettlementClientError>;
    async fn get_last_verified_block_hash(&self) -> Result<Felt, SettlementClientError>;

    /// Retrieves the initial state from the settlement layer
    ///
    /// This is called once during node startup to synchronize the initial state,
    /// except during testing where it may cause issues with test environments.
    async fn get_current_core_contract_state(&self) -> Result<StateUpdate, SettlementClientError>;

    /// Listens for and processes state update events from the settlement layer
    ///
    /// # Arguments
    /// * `backend` - Database backend for persisting state updates
    /// * `ctx` - Service context for managing the event loop
    /// * `l1_block_metrics` - Metrics for tracking L1 block processing
    async fn listen_for_update_state_events(
        &self,
        backend: Arc<MadaraBackend>,
        ctx: ServiceContext,
        l1_block_metrics: Arc<L1BlockMetrics>,
    ) -> Result<(), SettlementClientError>;

    /// Returns the current gas prices from the settlement layer
    ///
    /// Returns a tuple of (base_fee, data_gas_price)
    async fn get_gas_prices(&self) -> Result<(u128, u128), SettlementClientError>;

    /// Computes the hash of a messaging event for verification purposes
    fn get_messaging_hash(&self, event: &L1toL2MessagingEventData) -> Result<Vec<u8>, SettlementClientError>;

    /// Get cancellation status of an L1 to L2 message
    ///
    /// # Arguments
    /// * `msg_hash` - Hash of L1 to L2 message
    ///
    /// # Returns
    /// * `Felt::ZERO` - Message has not been cancelled
    /// * Other value - Timestamp when the message was cancelled
    async fn get_l1_to_l2_message_cancellations(&self, msg_hash: &[u8]) -> Result<Felt, SettlementClientError>;

    // ============================================================
    // Stream Implementations :
    // ============================================================

    /// Creates a stream of messaging events from the settlement layer
    ///
    /// # Arguments
    /// * `last_synced_event_block` - Contains information about the last block that was
    ///    successfully processed, used as starting point for the new stream
    ///
    /// This stream is used to process cross-chain messages in order, handling:
    /// - Message sequencing and ordering
    /// - Gap detection in event sequences
    /// - Error handling and recovery
    /// - Backpressure for event processing
    async fn get_messaging_stream(
        &self,
        last_synced_event_block: LastSyncedEventBlock,
    ) -> Result<Self::StreamType, SettlementClientError>;
}
