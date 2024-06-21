mod from_starknet_core;
mod from_starknet_provider;
mod to_starknet_core;

use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionReceipt {
    Invoke(InvokeTransactionReceipt),
    L1Handler(L1HandlerTransactionReceipt),
    Declare(DeclareTransactionReceipt),
    Deploy(DeployTransactionReceipt),
    DeployAccount(DeployAccountTransactionReceipt),
}

impl TransactionReceipt {
    pub fn transaction_hash(&self) -> Felt {
        match self {
            TransactionReceipt::Invoke(receipt) => receipt.transaction_hash,
            TransactionReceipt::L1Handler(receipt) => receipt.transaction_hash,
            TransactionReceipt::Declare(receipt) => receipt.transaction_hash,
            TransactionReceipt::Deploy(receipt) => receipt.transaction_hash,
            TransactionReceipt::DeployAccount(receipt) => receipt.transaction_hash,
        }
    }

    pub fn messages_sent(&self) -> &[MsgToL1] {
        match self {
            TransactionReceipt::Invoke(receipt) => &receipt.messages_sent,
            TransactionReceipt::L1Handler(receipt) => &receipt.messages_sent,
            TransactionReceipt::Declare(receipt) => &receipt.messages_sent,
            TransactionReceipt::Deploy(receipt) => &receipt.messages_sent,
            TransactionReceipt::DeployAccount(receipt) => &receipt.messages_sent,
        }
    }

    pub fn events(&self) -> &[Event] {
        match self {
            TransactionReceipt::Invoke(receipt) => &receipt.events,
            TransactionReceipt::L1Handler(receipt) => &receipt.events,
            TransactionReceipt::Declare(receipt) => &receipt.events,
            TransactionReceipt::Deploy(receipt) => &receipt.events,
            TransactionReceipt::DeployAccount(receipt) => &receipt.events,
        }
    }

    pub fn execution_result(&self) -> ExecutionResult {
        match self {
            TransactionReceipt::Invoke(receipt) => receipt.execution_result.clone(),
            TransactionReceipt::L1Handler(receipt) => receipt.execution_result.clone(),
            TransactionReceipt::Declare(receipt) => receipt.execution_result.clone(),
            TransactionReceipt::Deploy(receipt) => receipt.execution_result.clone(),
            TransactionReceipt::DeployAccount(receipt) => receipt.execution_result.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTransactionReceipt {
    pub transaction_hash: Felt, // This can be retrieved from the transaction itself.
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1HandlerTransactionReceipt {
    // normally this would be a Hash256, but the serde implementation doesn't work with bincode.
    pub message_hash: Felt,
    pub transaction_hash: Felt,
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareTransactionReceipt {
    pub transaction_hash: Felt,
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployTransactionReceipt {
    pub transaction_hash: Felt,
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
    pub contract_address: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployAccountTransactionReceipt {
    pub transaction_hash: Felt,
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
    pub contract_address: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeePayment {
    pub amount: Felt,
    pub unit: PriceUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceUnit {
    Wei,
    Fri,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MsgToL1 {
    pub from_address: Felt,
    pub to_address: Felt,
    pub payload: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub from_address: Felt,
    pub keys: Vec<Felt>,
    pub data: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExecutionResources {
    pub steps: u64,
    pub memory_holes: Option<u64>,
    pub range_check_builtin_applications: Option<u64>,
    pub pedersen_builtin_applications: Option<u64>,
    pub poseidon_builtin_applications: Option<u64>,
    pub ec_op_builtin_applications: Option<u64>,
    pub ecdsa_builtin_applications: Option<u64>,
    pub bitwise_builtin_applications: Option<u64>,
    pub keccak_builtin_applications: Option<u64>,
    pub segment_arena_builtin: Option<u64>,
    pub data_availability: DataAvailabilityResources,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DataAvailabilityResources {
    pub l1_gas: u64,
    pub l1_data_gas: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionResult {
    Succeeded,
    Reverted { reason: String },
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_bincode_transaction_receipt() {
        let receipt = TransactionReceipt::Invoke(InvokeTransactionReceipt {
            transaction_hash: Felt::from(1),
            actual_fee: FeePayment { amount: Felt::from(2), unit: PriceUnit::Wei },
            messages_sent: vec![MsgToL1 {
                from_address: Felt::from(3),
                to_address: Felt::from(4),
                payload: vec![Felt::from(5)],
            }],
            events: vec![Event { from_address: Felt::from(6), keys: vec![Felt::from(7)], data: vec![Felt::from(8)] }],
            execution_resources: ExecutionResources {
                steps: 9,
                memory_holes: Some(10),
                range_check_builtin_applications: Some(11),
                pedersen_builtin_applications: Some(12),
                poseidon_builtin_applications: Some(13),
                ec_op_builtin_applications: Some(14),
                ecdsa_builtin_applications: Some(15),
                bitwise_builtin_applications: Some(16),
                keccak_builtin_applications: Some(17),
                segment_arena_builtin: Some(18),
                data_availability: DataAvailabilityResources { l1_gas: 19, l1_data_gas: 20 },
            },
            execution_result: ExecutionResult::Succeeded,
        });

        let encoded_receipt = bincode::serialize(&receipt).unwrap();
        let decoded_receipt: TransactionReceipt = bincode::deserialize(&encoded_receipt).unwrap();
        assert_eq!(receipt, decoded_receipt);
    }
}
