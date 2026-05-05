// Re-export unchanged WebSocket types from v0.10.0
pub use crate::v0_10_0::{EmittedEventWithFinality, FinalityStatus, ReorgData, TxnStatusWithoutL1};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxnWithHashAndStatus {
    #[serde(flatten)]
    pub transaction: super::TxnWithHashAndProofFacts,
    pub finality_status: TxnStatusWithoutL1,
}

/// Subscription tag for controlling response fields (NEW in v0.10.2)
///
/// Used with WebSocket subscriptions to request additional optional fields
/// in the subscription messages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionTag {
    /// Include proof_facts in INVOKE_TXN_V3 transactions
    #[serde(rename = "INCLUDE_PROOF_FACTS")]
    IncludeProofFacts,
}
