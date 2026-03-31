#[derive(Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ReorgData {
    pub starting_block_hash: crate::v0_8_1::BlockHash,
    pub starting_block_number: crate::v0_8_1::BlockNumber,
    pub ending_block_hash: crate::v0_8_1::BlockHash,
    pub ending_block_number: crate::v0_8_1::BlockNumber,
}

#[derive(Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct NewTxnStatus {
    pub transaction_hash: starknet_types_core::felt::Felt,
    pub status: crate::v0_7_1::TxnStatus,
}

#[allow(clippy::large_enum_variant)]
#[derive(Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum PendingTxnInfo {
    Hash(starknet_types_core::felt::Felt),
    Full(crate::v0_7_1::Txn),
}
