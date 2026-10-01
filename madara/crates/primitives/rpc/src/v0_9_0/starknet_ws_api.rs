use serde::{Deserialize, Serialize};

use super::{BlockHash, BlockNumber, EmittedEvent, TxnFinalityStatus, TxnStatus, TxnWithHash};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ReorgData {
    pub starting_block_hash: BlockHash,
    pub starting_block_number: BlockNumber,
    pub ending_block_hash: BlockHash,
    pub ending_block_number: BlockNumber,
}

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

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum TxnStatusWithoutL1 {
    #[serde(rename = "RECEIVED")]
    Received,
    #[serde(rename = "CANDIDATE")]
    Candidate,
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,
}

impl From<TxnStatusWithoutL1> for TxnStatus {
    fn from(value: TxnStatusWithoutL1) -> Self {
        match value {
            TxnStatusWithoutL1::Received => Self::Received,
            TxnStatusWithoutL1::Candidate => Self::Candidate,
            TxnStatusWithoutL1::PreConfirmed => Self::PreConfirmed,
            TxnStatusWithoutL1::AcceptedOnL2 => Self::AcceptedOnL2,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TxnWithHashAndStatus {
    #[serde(flatten)]
    pub transaction: TxnWithHash,
    pub finality_status: TxnStatusWithoutL1,
}
