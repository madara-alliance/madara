mod from_blockifier;
mod from_starknet_core;
mod from_starknet_provider;
mod to_starknet_core;
pub use from_blockifier::from_blockifier_execution_info;

use serde::{Deserialize, Serialize};
use starknet_core::utils::starknet_keccak;
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, Poseidon, StarkHash},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionReceipt {
    Invoke(InvokeTransactionReceipt),
    L1Handler(L1HandlerTransactionReceipt),
    Declare(DeclareTransactionReceipt),
    Deploy(DeployTransactionReceipt),
    DeployAccount(DeployAccountTransactionReceipt),
}

impl From<InvokeTransactionReceipt> for TransactionReceipt {
    fn from(receipt: InvokeTransactionReceipt) -> Self {
        TransactionReceipt::Invoke(receipt)
    }
}

impl From<L1HandlerTransactionReceipt> for TransactionReceipt {
    fn from(receipt: L1HandlerTransactionReceipt) -> Self {
        TransactionReceipt::L1Handler(receipt)
    }
}

impl From<DeclareTransactionReceipt> for TransactionReceipt {
    fn from(receipt: DeclareTransactionReceipt) -> Self {
        TransactionReceipt::Declare(receipt)
    }
}

impl From<DeployTransactionReceipt> for TransactionReceipt {
    fn from(receipt: DeployTransactionReceipt) -> Self {
        TransactionReceipt::Deploy(receipt)
    }
}

impl From<DeployAccountTransactionReceipt> for TransactionReceipt {
    fn from(receipt: DeployAccountTransactionReceipt) -> Self {
        TransactionReceipt::DeployAccount(receipt)
    }
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

    pub fn actual_fee(&self) -> &FeePayment {
        match self {
            TransactionReceipt::Invoke(receipt) => &receipt.actual_fee,
            TransactionReceipt::L1Handler(receipt) => &receipt.actual_fee,
            TransactionReceipt::Declare(receipt) => &receipt.actual_fee,
            TransactionReceipt::Deploy(receipt) => &receipt.actual_fee,
            TransactionReceipt::DeployAccount(receipt) => &receipt.actual_fee,
        }
    }

    pub fn data_availability(&self) -> &DataAvailabilityResources {
        match self {
            TransactionReceipt::Invoke(receipt) => &receipt.execution_resources.data_availability,
            TransactionReceipt::L1Handler(receipt) => &receipt.execution_resources.data_availability,
            TransactionReceipt::Declare(receipt) => &receipt.execution_resources.data_availability,
            TransactionReceipt::Deploy(receipt) => &receipt.execution_resources.data_availability,
            TransactionReceipt::DeployAccount(receipt) => &receipt.execution_resources.data_availability,
        }
    }

    pub fn total_gas_consumed(&self) -> &DataAvailabilityResources {
        match self {
            TransactionReceipt::Invoke(receipt) => &receipt.execution_resources.total_gas_consumed,
            TransactionReceipt::L1Handler(receipt) => &receipt.execution_resources.total_gas_consumed,
            TransactionReceipt::Declare(receipt) => &receipt.execution_resources.total_gas_consumed,
            TransactionReceipt::Deploy(receipt) => &receipt.execution_resources.total_gas_consumed,
            TransactionReceipt::DeployAccount(receipt) => &receipt.execution_resources.total_gas_consumed,
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

    pub fn compute_hash(&self) -> Felt {
        Poseidon::hash_array(&[
            self.transaction_hash(),
            self.actual_fee().amount,
            compute_messages_sent_hash(self.messages_sent()),
            self.execution_result().compute_hash(),
            Felt::ZERO, // L2 gas consumption.
            self.total_gas_consumed().l1_gas.into(),
            self.total_gas_consumed().l1_data_gas.into(),
        ])
    }
}

fn compute_messages_sent_hash(messages: &[MsgToL1]) -> Felt {
    let messages_len_as_felt: Felt = (messages.len() as u64).into();

    let elements: Vec<Felt> = std::iter::once(messages_len_as_felt)
        .chain(messages.iter().flat_map(|msg| {
            let payload_len_as_felt = (msg.payload.len() as u64).into();
            std::iter::once(msg.from_address)
                .chain(std::iter::once(msg.to_address))
                .chain(std::iter::once(payload_len_as_felt))
                .chain(msg.payload.iter().cloned())
        }))
        .collect();

    Poseidon::hash_array(&elements)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTransactionReceipt {
    pub transaction_hash: Felt, // This can be retrieved from the transaction itself.
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareTransactionReceipt {
    pub transaction_hash: Felt,
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployTransactionReceipt {
    pub transaction_hash: Felt,
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
    pub contract_address: Felt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployAccountTransactionReceipt {
    pub transaction_hash: Felt,
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
    pub contract_address: Felt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeePayment {
    pub amount: Felt,
    pub unit: PriceUnit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceUnit {
    #[default]
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

impl Event {
    /// Calculate the hash of the event.
    pub fn compute_hash_pedersen(&self) -> Felt {
        let keys_hash = Pedersen::hash_array(&self.keys);
        let data_hash = Pedersen::hash_array(&self.data);
        Pedersen::hash_array(&[self.from_address, keys_hash, data_hash])
    }

    pub fn compute_hash_poseidon(&self, transaction_hash: &Felt) -> Felt {
        let keys_len_as_felt = (self.keys.len() as u64).into();
        let data_len_as_felt = (self.data.len() as u64).into();

        let elements: Vec<Felt> = std::iter::once(self.from_address)
            .chain(std::iter::once(*transaction_hash))
            .chain(std::iter::once(keys_len_as_felt).chain(self.keys.iter().cloned()))
            .chain(std::iter::once(data_len_as_felt).chain(self.data.iter().cloned()))
            .collect();

        Poseidon::hash_array(&elements)
    }
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
    pub total_gas_consumed: DataAvailabilityResources,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DataAvailabilityResources {
    pub l1_gas: u64,
    pub l1_data_gas: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionResult {
    #[default]
    Succeeded,
    Reverted {
        reason: String,
    },
}

impl ExecutionResult {
    fn compute_hash(&self) -> Felt {
        match self {
            ExecutionResult::Succeeded => Felt::ZERO,
            ExecutionResult::Reverted { reason } => starknet_keccak(reason.as_bytes()),
        }
    }
}

#[cfg(test)]
mod tests {
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
                total_gas_consumed: DataAvailabilityResources { l1_gas: 21, l1_data_gas: 22 },
            },
            execution_result: ExecutionResult::Succeeded,
        });

        let encoded_receipt = bincode::serialize(&receipt).unwrap();
        let decoded_receipt: TransactionReceipt = bincode::deserialize(&encoded_receipt).unwrap();
        assert_eq!(receipt, decoded_receipt);
    }

    #[test]
    fn test_compute_messages_sent_hash() {
        let msg1 = MsgToL1 { from_address: Felt::ZERO, to_address: Felt::ONE, payload: vec![Felt::TWO, Felt::THREE] };
        let msg2 =
            MsgToL1 { from_address: Felt::ONE, to_address: Felt::TWO, payload: vec![Felt::THREE, Felt::from(4)] };

        let hash = compute_messages_sent_hash(&[msg1, msg2]);

        assert_eq!(
            hash,
            Felt::from_hex_unchecked("0x00c89474a9007dc060aed76caf8b30b927cfea1ebce2d134b943b8d7121004e4")
        );
    }

    #[test]
    fn test_execution_result_compute_hash() {
        let succeeded = ExecutionResult::Succeeded;
        let reverted = ExecutionResult::Reverted { reason: "reason".to_string() };

        assert_eq!(succeeded.compute_hash(), Felt::ZERO);
        assert_eq!(
            reverted.compute_hash(),
            Felt::from_hex_unchecked("0x3da776b48d37b131aef5221e52de092c5693df8fdf02fd7acf293a075aa3be4")
        );
    }

    #[test]
    fn test_transaction_receipt_compute_hash() {
        let receipt = TransactionReceipt::Invoke(InvokeTransactionReceipt {
            transaction_hash: Felt::from(1),
            actual_fee: FeePayment { amount: Felt::from(2), unit: PriceUnit::Wei },
            messages_sent: vec![
                MsgToL1 {
                    from_address: Felt::from(3),
                    to_address: Felt::from(4),
                    payload: vec![Felt::from(5), Felt::from(6)],
                },
                MsgToL1 {
                    from_address: Felt::from(7),
                    to_address: Felt::from(8),
                    payload: vec![Felt::from(9), Felt::from(10)],
                },
            ],
            events: vec![],
            execution_resources: ExecutionResources {
                steps: 0,
                memory_holes: None,
                range_check_builtin_applications: None,
                pedersen_builtin_applications: None,
                poseidon_builtin_applications: None,
                ec_op_builtin_applications: None,
                ecdsa_builtin_applications: None,
                bitwise_builtin_applications: None,
                keccak_builtin_applications: None,
                segment_arena_builtin: None,
                data_availability: DataAvailabilityResources { l1_gas: 11, l1_data_gas: 12 },
                total_gas_consumed: DataAvailabilityResources { l1_gas: 13, l1_data_gas: 14 },
            },
            execution_result: ExecutionResult::Reverted { reason: "aborted".to_string() },
        });

        let hash = receipt.compute_hash();

        assert_eq!(hash, Felt::from_hex_unchecked("0x26feec2eb76b63633bc4c1af780813b52a0809a1dae192f53397385da95b8"));
    }
}
