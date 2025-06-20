#[derive(Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct TxnStatus {
    pub transaction_hash: starknet_types_core::felt::Felt,
    pub status: crate::v0_7_1::TxnStatus,
}
