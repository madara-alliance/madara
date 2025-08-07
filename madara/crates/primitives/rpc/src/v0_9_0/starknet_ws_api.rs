use serde::{Deserialize, Serialize};

use super::{EmittedEvent, TxnFinalityStatus};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Default)]
pub enum FinalityStatus {
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,
    #[default]
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct EmittedEventWithFinality {
    #[serde(flatten)]
    pub emmitted_event: EmittedEvent,
    #[serde(flatten)]
    pub finality_status: TxnFinalityStatus,
}
