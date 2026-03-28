use serde::{Deserialize, Serialize};

use super::{EmittedEvent, TxnFinalityStatus};

pub use crate::v0_9_0::{FinalityStatus, ReorgData, TxnStatusWithoutL1, TxnWithHashAndStatus};

/// An emitted event with finality status for WebSocket subscriptions
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EmittedEventWithFinality {
    #[serde(flatten)]
    pub emitted_event: EmittedEvent,
    #[serde(flatten)]
    pub finality_status: TxnFinalityStatus,
}
