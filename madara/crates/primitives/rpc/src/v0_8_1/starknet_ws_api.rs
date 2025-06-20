#[derive(Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct TxnStatus {
    pub transaction_hash: starknet_types_core::felt::Felt,
    pub status: crate::v0_7_1::TxnStatus,
}

#[allow(clippy::large_enum_variant)]
#[derive(Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum PendingTxnInfo {
    Hash(starknet_types_core::felt::Felt),
    Full(crate::v0_7_1::Txn),
}
