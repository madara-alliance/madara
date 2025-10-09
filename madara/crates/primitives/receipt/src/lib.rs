use mp_chain_config::StarknetVersion;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use starknet_core::utils::starknet_keccak;
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, Poseidon, StarkHash},
};

pub mod from_blockifier;

mod to_starknet_types;

pub use from_blockifier::from_blockifier_execution_info;
pub use starknet_core::types::Hash256;

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
    pub fn transaction_hash(&self) -> &Felt {
        match self {
            TransactionReceipt::Invoke(receipt) => &receipt.transaction_hash,
            TransactionReceipt::L1Handler(receipt) => &receipt.transaction_hash,
            TransactionReceipt::Declare(receipt) => &receipt.transaction_hash,
            TransactionReceipt::Deploy(receipt) => &receipt.transaction_hash,
            TransactionReceipt::DeployAccount(receipt) => &receipt.transaction_hash,
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

    pub fn data_availability(&self) -> &GasVector {
        match self {
            TransactionReceipt::Invoke(receipt) => &receipt.execution_resources.data_availability,
            TransactionReceipt::L1Handler(receipt) => &receipt.execution_resources.data_availability,
            TransactionReceipt::Declare(receipt) => &receipt.execution_resources.data_availability,
            TransactionReceipt::Deploy(receipt) => &receipt.execution_resources.data_availability,
            TransactionReceipt::DeployAccount(receipt) => &receipt.execution_resources.data_availability,
        }
    }

    pub fn total_gas_consumed(&self) -> &GasVector {
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

    pub fn events_mut(&mut self) -> &mut Vec<Event> {
        match self {
            TransactionReceipt::Invoke(receipt) => &mut receipt.events,
            TransactionReceipt::L1Handler(receipt) => &mut receipt.events,
            TransactionReceipt::Declare(receipt) => &mut receipt.events,
            TransactionReceipt::Deploy(receipt) => &mut receipt.events,
            TransactionReceipt::DeployAccount(receipt) => &mut receipt.events,
        }
    }

    pub fn into_events(self) -> Vec<Event> {
        match self {
            TransactionReceipt::Invoke(receipt) => receipt.events,
            TransactionReceipt::L1Handler(receipt) => receipt.events,
            TransactionReceipt::Declare(receipt) => receipt.events,
            TransactionReceipt::Deploy(receipt) => receipt.events,
            TransactionReceipt::DeployAccount(receipt) => receipt.events,
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

    pub fn execution_resources(&self) -> &ExecutionResources {
        match self {
            TransactionReceipt::Invoke(receipt) => &receipt.execution_resources,
            TransactionReceipt::L1Handler(receipt) => &receipt.execution_resources,
            TransactionReceipt::Declare(receipt) => &receipt.execution_resources,
            TransactionReceipt::Deploy(receipt) => &receipt.execution_resources,
            TransactionReceipt::DeployAccount(receipt) => &receipt.execution_resources,
        }
    }

    pub fn contract_address(&self) -> Option<&Felt> {
        match self {
            TransactionReceipt::Deploy(receipt) => Some(&receipt.contract_address),
            TransactionReceipt::DeployAccount(receipt) => Some(&receipt.contract_address),
            _ => None,
        }
    }

    pub fn compute_hash(&self) -> Felt {
        Poseidon::hash_array(&[
            *self.transaction_hash(),
            self.actual_fee().amount,
            compute_messages_sent_hash(self.messages_sent()),
            self.execution_result().compute_hash(),
            Felt::ZERO, // L2 gas consumption.
            self.total_gas_consumed().l1_gas.into(),
            self.total_gas_consumed().l1_data_gas.into(),
        ])
    }

    pub fn l2_gas_used(&self) -> u128 {
        self.execution_resources().total_gas_consumed.l2_gas
    }

    pub fn as_l1_handler(&self) -> Option<&L1HandlerTransactionReceipt> {
        match self {
            TransactionReceipt::L1Handler(r) => Some(r),
            _ => None,
        }
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

#[serde_with::serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1HandlerTransactionReceipt {
    #[serde_as(as = "mp_convert::hash256_serde::Hash256Serde")]
    pub message_hash: Hash256,
    pub transaction_hash: Felt,
    pub actual_fee: FeePayment,
    pub messages_sent: Vec<MsgToL1>,
    pub events: Vec<Event>,
    pub execution_resources: ExecutionResources,
    pub execution_result: ExecutionResult,
}

// TODO: we shouldnt need to have default impls for these types (it's used in tests)
// Implement default by hand as [`Hash256`] does not impl Default.
impl Default for L1HandlerTransactionReceipt {
    fn default() -> Self {
        Self {
            message_hash: Hash256::from_bytes(Default::default()),
            transaction_hash: Default::default(),
            actual_fee: Default::default(),
            messages_sent: Default::default(),
            events: Default::default(),
            execution_resources: Default::default(),
            execution_result: Default::default(),
        }
    }
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
#[serde(deny_unknown_fields)]
pub struct MsgToL1 {
    pub from_address: Felt,
    pub to_address: Felt,
    pub payload: Vec<Felt>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MsgToL2 {
    pub from_address: Felt,
    pub to_address: Felt,
    pub selector: Felt,
    pub payload: Vec<Felt>,
    pub nonce: Option<Felt>,
}

// Specification reference: https://docs.starknet.io/architecture-and-concepts/network-architecture/messaging-mechanism/#hashing_l1-l2
// Example implementation in starknet-rs: https://github.com/xJonathanLEI/starknet-rs/blob/master/starknet-core/src/types/msg.rs#L28
//
// Key Differences:
// - In starknet-rs, padding is applied to the `from_address` and `nonce` fields. This is necessary because the `from_address` is an Ethereum address (20 bytes) and the `nonce` is a u64 (8 bytes).
// - In this implementation, padding for `from_address` and `nonce` is not required. Both fields are converted to `felt252`, which naturally fits the required size.
// - Padding is only applied to the payload length, which is a u64 (8 bytes), to ensure proper alignment.
impl MsgToL2 {
    pub fn compute_hash(&self) -> Hash256 {
        let mut hasher = Keccak256::new();
        hasher.update(self.from_address.to_bytes_be());
        hasher.update(self.to_address.to_bytes_be());
        hasher.update(self.nonce.unwrap_or_default().to_bytes_be());
        hasher.update(self.selector.to_bytes_be());
        hasher.update([0u8; 24]); // Padding
        hasher.update((self.payload.len() as u64).to_be_bytes());
        self.payload.iter().for_each(|felt| hasher.update(felt.to_bytes_be()));
        let bytes = hasher.finalize().as_slice().try_into().expect("Byte array length mismatch");
        Hash256::from_bytes(bytes)
    }
}

/// Event with transaction hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventWithTransactionHash {
    pub transaction_hash: Felt,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub from_address: Felt,
    pub keys: Vec<Felt>,
    pub data: Vec<Felt>,
}

impl Event {
    /// Calculate the hash of the event.
    pub fn compute_hash(&self, transaction_hash: Felt, starknet_version: StarknetVersion) -> Felt {
        if starknet_version < StarknetVersion::V0_13_2 {
            self.compute_hash_pedersen()
        } else {
            self.compute_hash_poseidon(&transaction_hash)
        }
    }

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
    pub memory_holes: u64,
    pub range_check_builtin_applications: u64,
    pub pedersen_builtin_applications: u64,
    pub poseidon_builtin_applications: u64,
    pub ec_op_builtin_applications: u64,
    pub ecdsa_builtin_applications: u64,
    pub bitwise_builtin_applications: u64,
    pub keccak_builtin_applications: u64,
    pub segment_arena_builtin: u64,
    pub data_availability: GasVector,
    pub total_gas_consumed: GasVector,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GasVector {
    pub l1_gas: u128,
    pub l1_data_gas: u128,
    pub l2_gas: u128,
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
    pub fn revert_reason(&self) -> Option<&str> {
        match self {
            Self::Succeeded => None,
            Self::Reverted { reason } => Some(reason),
        }
    }

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
                memory_holes: 10,
                range_check_builtin_applications: 11,
                pedersen_builtin_applications: 12,
                poseidon_builtin_applications: 13,
                ec_op_builtin_applications: 14,
                ecdsa_builtin_applications: 15,
                bitwise_builtin_applications: 16,
                keccak_builtin_applications: 17,
                segment_arena_builtin: 18,
                data_availability: GasVector { l1_gas: 19, l1_data_gas: 20, l2_gas: 21 },
                total_gas_consumed: GasVector { l1_gas: 22, l1_data_gas: 23, l2_gas: 24 },
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
        let expected_hash =
            Felt::from_hex_unchecked("0x00c89474a9007dc060aed76caf8b30b927cfea1ebce2d134b943b8d7121004e4");

        assert_eq!(hash, expected_hash,);
    }

    #[test]
    fn test_execution_result_compute_hash() {
        let succeeded = ExecutionResult::Succeeded;
        let reverted = ExecutionResult::Reverted { reason: "reason".to_string() };
        let expected_hash =
            Felt::from_hex_unchecked("0x3da776b48d37b131aef5221e52de092c5693df8fdf02fd7acf293a075aa3be4");

        assert_eq!(succeeded.compute_hash(), Felt::ZERO);
        assert_eq!(reverted.compute_hash(), expected_hash,);
    }

    #[test]
    fn test_event_compute_hash_pedersen() {
        let event = Event {
            from_address: Felt::from(1),
            keys: vec![Felt::from(2), Felt::from(3)],
            data: vec![Felt::from(4), Felt::from(5)],
        };

        let hash = event.compute_hash_pedersen();
        let expected_hash =
            Felt::from_hex_unchecked("0x770591674368ef723f40ddef92ee7cf2fcf3e244afa15cd0a703fce83a415c1");

        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn test_event_compute_hash_poseidon() {
        let event = Event {
            from_address: Felt::from(1),
            keys: vec![Felt::from(2), Felt::from(3)],
            data: vec![Felt::from(4), Felt::from(5)],
        };
        let transaction_hash = Felt::from(6);

        let hash = event.compute_hash_poseidon(&transaction_hash);
        let expected_hash =
            Felt::from_hex_unchecked("0x38be6cfe9175d3f260d1886bf932e1b67415c6c0e8b8a6de565326493c5524f");

        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn test_transaction_receipt_compute_hash() {
        let receipt: TransactionReceipt = dummy_invoke_receipt().into();
        let hash = receipt.compute_hash();
        let expected_hash =
            Felt::from_hex_unchecked("0x1148dd71676e6e2716fa87e39b09ed67abaa8dc3dea5e4260a7f3618eb22dd7");
        assert_eq!(hash, expected_hash);

        let receipt: TransactionReceipt = dummy_declare_receipt().into();
        let hash = receipt.compute_hash();
        assert_eq!(hash, expected_hash);

        let receipt: TransactionReceipt = dummy_deploy_receipt().into();
        let hash = receipt.compute_hash();
        assert_eq!(hash, expected_hash);

        let receipt: TransactionReceipt = dummy_deploy_account_receipt().into();
        let hash = receipt.compute_hash();
        assert_eq!(hash, expected_hash);

        let receipt: TransactionReceipt = dummy_l1_handler_receipt().into();
        let hash = receipt.compute_hash();
        let expected_hash =
            Felt::from_hex_unchecked("0x3e1d6370d26591a5e69de6ffc6d46ef936b3b664afa6f2f1b46a6f4029e604");
        assert_eq!(hash, expected_hash);
    }

    fn dummy_messages() -> Vec<MsgToL1> {
        vec![
            MsgToL1 {
                from_address: Felt::from(1),
                to_address: Felt::from(2),
                payload: vec![Felt::from(3), Felt::from(4)],
            },
            MsgToL1 {
                from_address: Felt::from(5),
                to_address: Felt::from(6),
                payload: vec![Felt::from(7), Felt::from(8)],
            },
        ]
    }

    fn dummy_events() -> Vec<Event> {
        vec![
            Event {
                from_address: Felt::from(1),
                keys: vec![Felt::from(2), Felt::from(3)],
                data: vec![Felt::from(4), Felt::from(5)],
            },
            Event {
                from_address: Felt::from(6),
                keys: vec![Felt::from(7), Felt::from(8)],
                data: vec![Felt::from(9), Felt::from(10)],
            },
        ]
    }

    fn dummy_execution_ressources() -> ExecutionResources {
        ExecutionResources {
            steps: 1,
            memory_holes: 2,
            range_check_builtin_applications: 3,
            pedersen_builtin_applications: 4,
            poseidon_builtin_applications: 5,
            ec_op_builtin_applications: 6,
            ecdsa_builtin_applications: 7,
            bitwise_builtin_applications: 8,
            keccak_builtin_applications: 9,
            segment_arena_builtin: 10,
            data_availability: GasVector { l1_gas: 11, l1_data_gas: 12, l2_gas: 13 },
            total_gas_consumed: GasVector { l1_gas: 14, l1_data_gas: 15, l2_gas: 16 },
        }
    }

    pub(crate) fn dummy_invoke_receipt() -> InvokeTransactionReceipt {
        InvokeTransactionReceipt {
            transaction_hash: Felt::from(1),
            actual_fee: FeePayment { amount: Felt::from(2), unit: PriceUnit::Wei },
            messages_sent: dummy_messages(),
            events: dummy_events(),
            execution_resources: dummy_execution_ressources(),
            execution_result: ExecutionResult::Reverted { reason: "aborted".to_string() },
        }
    }

    pub(crate) fn dummy_l1_handler_receipt() -> L1HandlerTransactionReceipt {
        L1HandlerTransactionReceipt {
            message_hash: Hash256::from_hex("0x1").unwrap(),
            transaction_hash: Felt::from(2),
            actual_fee: FeePayment { amount: Felt::from(3), unit: PriceUnit::Wei },
            messages_sent: dummy_messages(),
            events: dummy_events(),
            execution_resources: dummy_execution_ressources(),
            execution_result: ExecutionResult::Reverted { reason: "aborted".to_string() },
        }
    }

    pub(crate) fn dummy_declare_receipt() -> DeclareTransactionReceipt {
        DeclareTransactionReceipt {
            transaction_hash: Felt::from(1),
            actual_fee: FeePayment { amount: Felt::from(2), unit: PriceUnit::Wei },
            messages_sent: dummy_messages(),
            events: dummy_events(),
            execution_resources: dummy_execution_ressources(),
            execution_result: ExecutionResult::Reverted { reason: "aborted".to_string() },
        }
    }

    pub(crate) fn dummy_deploy_receipt() -> DeployTransactionReceipt {
        DeployTransactionReceipt {
            transaction_hash: Felt::from(1),
            actual_fee: FeePayment { amount: Felt::from(2), unit: PriceUnit::Wei },
            messages_sent: dummy_messages(),
            events: dummy_events(),
            execution_resources: dummy_execution_ressources(),
            execution_result: ExecutionResult::Reverted { reason: "aborted".to_string() },
            contract_address: Felt::from(3),
        }
    }

    pub(crate) fn dummy_deploy_account_receipt() -> DeployAccountTransactionReceipt {
        DeployAccountTransactionReceipt {
            transaction_hash: Felt::from(1),
            actual_fee: FeePayment { amount: Felt::from(2), unit: PriceUnit::Wei },
            messages_sent: dummy_messages(),
            events: dummy_events(),
            execution_resources: dummy_execution_ressources(),
            execution_result: ExecutionResult::Reverted { reason: "aborted".to_string() },
            contract_address: Felt::from(3),
        }
    }
}
