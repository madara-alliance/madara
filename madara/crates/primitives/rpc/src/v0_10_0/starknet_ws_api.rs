use serde::{Deserialize, Serialize};

use super::{EmittedEvent, TxnExecutionStatus, TxnFinalityStatus, TxnHash, TxnStatus};

pub use crate::v0_9_0::{FinalityStatus, ReorgData, TxnStatusWithoutL1, TxnWithHashAndStatus};

/// An emitted event with finality status for WebSocket subscriptions
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EmittedEventWithFinality {
    #[serde(flatten)]
    pub emitted_event: EmittedEvent,
    pub finality_status: TxnFinalityStatus,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct NewTxnStatus {
    pub transaction_hash: TxnHash,
    pub status: WsTxnStatusResult,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct WsTxnStatusResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<TxnExecutionStatus>,
    pub finality_status: TxnStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}
