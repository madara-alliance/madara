//! Starknet pallet custom types.
use std::collections::HashMap;

use blockifier::execution::contract_class::ContractClass;
use blockifier::transaction::objects::{FeeType, HasRelatedFeeType};
use blockifier::transaction::transaction_execution::Transaction;
use mp_felt::Felt252Wrapper;
use sp_core::ConstU32;
use sp_std::vec::Vec;
use starknet_api::core::{ClassHash, ContractAddress};
use starknet_api::state::StorageKey;
use starknet_api::transaction::{Event, Fee, MessageToL1, TransactionHash};
use starknet_core::types::PriceUnit as Price;

/// Contract Storage Key
pub type ContractStorageKey = (ContractAddress, StorageKey);

/// Make this configurable. Max transaction/block
pub type MaxTransactionsPendingBlock = ConstU32<1073741824>;

pub type ContractClassMapping = HashMap<ClassHash, ContractClass>;

/// Type wrapper for a storage slot.
pub type StorageSlot = (StorageKey, Felt252Wrapper);

pub type CasmClassHash = ClassHash;
pub type SierraClassHash = ClassHash;

/// Declare Transaction Output
#[derive(Clone, Debug, PartialEq, Eq, parity_scale_codec::Encode, parity_scale_codec::Decode, scale_info::TypeInfo)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct DeployAccountTransactionOutput {
    /// Transaction hash
    pub transaction_hash: Felt252Wrapper,
    /// Contract Address
    pub contract_address: ContractAddress,
}

/// Build invoke transaction for transfer utils
pub struct BuildTransferInvokeTransaction {
    pub sender_address: Felt252Wrapper,
    pub token_address: Felt252Wrapper,
    pub recipient: Felt252Wrapper,
    pub amount_low: Felt252Wrapper,
    pub amount_high: Felt252Wrapper,
    pub nonce: Felt252Wrapper,
}

#[derive(Clone, Debug, PartialEq, Eq, parity_scale_codec::Encode, parity_scale_codec::Decode, scale_info::TypeInfo)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct TransactionOutput {
    pub transaction_hash: TransactionHash,
    pub actual_fee: Fee,
    pub messages_sent: Vec<MessageToL1>,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, PartialEq, Eq, parity_scale_codec::Encode, parity_scale_codec::Decode, scale_info::TypeInfo)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub enum PriceUnit {
    #[serde(rename = "WEI")]
    Wei,
    #[serde(rename = "FRI")]
    Fri,
}

#[derive(Clone, Debug, PartialEq, Eq, parity_scale_codec::Encode, parity_scale_codec::Decode, scale_info::TypeInfo)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct FeeEstimate {
    pub gas_consumed: Felt252Wrapper,
    pub gas_price: Felt252Wrapper,
    pub data_gas_consumed: Felt252Wrapper,
    pub data_gas_price: Felt252Wrapper,
    pub overall_fee: Felt252Wrapper,
    pub unit: PriceUnit,
}

pub fn fee_type(transaction: &Transaction) -> FeeType {
    match transaction {
        Transaction::AccountTransaction(tx) => tx.fee_type(),
        Transaction::L1HandlerTransaction(tx) => tx.fee_type(),
    }
}

impl From<PriceUnit> for Price {
    fn from(unit: PriceUnit) -> Self {
        match unit {
            PriceUnit::Wei => Price::Wei,
            PriceUnit::Fri => Price::Fri,
        }
    }
}
