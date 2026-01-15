// Re-export unchanged WebSocket types from v0.10.0
pub use crate::v0_10_0::EmittedEventWithFinality;

use serde::{Deserialize, Serialize};

/// Subscription tag for controlling response fields (NEW in v0.10.1)
///
/// Used with WebSocket subscriptions to request additional optional fields
/// in the subscription messages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionTag {
    /// Include proof_facts in INVOKE_TXN_V3 transactions
    #[serde(rename = "INCLUDE_PROOF_FACTS")]
    IncludeProofFacts,
}
