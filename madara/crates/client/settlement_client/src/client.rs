use crate::error::SettlementClientError;
use crate::messaging::MessageToL2WithMetadata;
use crate::state_update::{StateUpdate, StateUpdateWorker};
use async_trait::async_trait;
use futures::stream::BoxStream;
use mp_transactions::L1HandlerTransactionWithFee;
use mp_utils::service::ServiceContext;
use starknet_types_core::felt::Felt;

pub enum ClientType {
    Eth,
    Starknet,
}

// Test types in a separate module
#[cfg(test)]
pub mod test_types {
    #[derive(Debug, Default, PartialEq)]
    pub struct DummyConfig;
}

// Use different automock configurations based on the build type
#[cfg_attr(test, mockall::automock(
    type Config = test_types::DummyConfig;
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
    fn get_client_type(&self) -> ClientType;
    async fn get_latest_block_number(&self) -> Result<u64, SettlementClientError>;
    async fn get_last_event_block_number(&self) -> Result<u64, SettlementClientError>;

    /// Retrieves the initial state from the settlement layer
    ///
    /// This is called once during node startup to synchronize the initial state,
    /// except during testing where it may cause issues with test environments.
    ///
    // TODO: Implementations should convert their native types to Felt.
    // TODO: Add tests to verify this conversion is correct.
    async fn get_current_core_contract_state(&self) -> Result<StateUpdate, SettlementClientError>;

    /// Listens for and processes state update events from the settlement layer
    ///
    /// # Arguments
    /// * `backend` - Database backend for persisting state updates
    /// * `ctx` - Service context for managing the event loop
    /// * `l1_block_metrics` - Metrics for tracking L1 block processing
    async fn listen_for_update_state_events(
        &self,
        ctx: ServiceContext,
        worker: StateUpdateWorker,
    ) -> Result<(), SettlementClientError>;

    /// Returns the current gas prices from the settlement layer
    ///
    /// Returns a tuple of (base_fee, data_gas_price)
    async fn get_gas_prices(&self) -> Result<(u128, u128), SettlementClientError>;

    /// Computes the hash of a messaging event for verification purposes
    fn get_messaging_hash(&self, event: &L1HandlerTransactionWithFee) -> Result<Vec<u8>, SettlementClientError>;

    /// Get cancellation status of an L1 to L2 message
    ///
    /// # Arguments
    /// * `msg_hash` - Hash of L1 to L2 message
    ///
    /// # Returns
    /// * `Felt::ZERO` - Message has not been cancelled
    /// * Other value - Timestamp when the message was cancelled
    async fn get_l1_to_l2_message_cancellation(&self, msg_hash: &[u8]) -> Result<Felt, SettlementClientError>;
    /// Get cancellation status of an L1 to L2 message
    ///
    /// This function query the core contract to know if a L1->L2 still exists in the contract.
    /// # Arguments
    ///
    /// - msg_hash : Hash of L1 to L2 message
    ///
    /// # Return
    ///
    /// - A felt representing a timestamp :
    ///     - 0 if the message has not been cancelled
    ///     - timestamp of the cancellation if it has been cancelled
    /// - An Error if the call fail
    async fn check_l1_to_l2_message_exists(&self, msg_hash: &[u8]) -> Result<bool, SettlementClientError>;

    /// Return a block timestamp in second.
    ///
    /// # Arguments
    /// * `l1_block_n` - Block number
    ///
    /// # Returns
    /// * Block timestamp in seconds
    async fn get_block_n_timestamp(&self, l1_block_n: u64) -> Result<u64, SettlementClientError>;

    // ============================================================
    // Stream Implementations :
    // ============================================================

    /// Creates a stream listening to L1 to L2 events.
    ///
    /// # Arguments
    /// * `from_l1_block_n` - Start returning events from this block_n.
    /// * `end_l1_block_n` - Stop returning events at this block_n. None to keep continuing.
    async fn l1_to_l2_messages_stream(
        &self,
        from_l1_block_n: u64,
        end_l1_block_n: Option<u64>,
    ) -> Result<BoxStream<'static, Result<MessageToL2WithMetadata, SettlementClientError>>, SettlementClientError>;
}
