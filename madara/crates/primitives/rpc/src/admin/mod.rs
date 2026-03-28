use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

use crate::v0_10_0::InvokeTxnV3;
use crate::v0_7_1::{Address, DeprecatedContractClass, InvokeTxnV0, InvokeTxnV1, Signature};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BroadcastedDeclareTxnV0 {
    /// The class to be declared
    pub contract_class: DeprecatedContractClass,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: Address,
    pub signature: Signature,
    pub is_query: bool,
}

impl BroadcastedDeclareTxnV0 {
    pub fn is_query(&self) -> bool {
        self.is_query
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BroadcastedInvokeTxnV3 {
    #[serde(flatten)]
    pub inner: InvokeTxnV3,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_facts: Option<Vec<Felt>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum BroadcastedInvokeTxn {
    #[serde(rename = "0x0")]
    V0(InvokeTxnV0),
    #[serde(rename = "0x1")]
    V1(InvokeTxnV1),
    #[serde(rename = "0x3")]
    V3(BroadcastedInvokeTxnV3),
    #[serde(rename = "0x100000000000000000000000000000000")]
    QueryV0(InvokeTxnV0),
    #[serde(rename = "0x100000000000000000000000000000001")]
    QueryV1(InvokeTxnV1),
    #[serde(rename = "0x100000000000000000000000000000003")]
    QueryV3(BroadcastedInvokeTxnV3),
}

impl BroadcastedInvokeTxn {
    pub fn is_query(&self) -> bool {
        matches!(self, Self::QueryV0(_) | Self::QueryV1(_) | Self::QueryV3(_))
    }

    pub fn version(&self) -> Felt {
        match self {
            Self::V0(_) | Self::QueryV0(_) => Felt::ZERO,
            Self::V1(_) | Self::QueryV1(_) => Felt::ONE,
            Self::V3(_) | Self::QueryV3(_) => Felt::THREE,
        }
    }
}
