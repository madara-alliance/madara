use mp_block::H160;
use mp_receipt::{Event, L1Gas};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedReceipt {
    pub transaction_hash: Felt,
    pub transaction_index: u64,
    pub actual_fee: Felt,
    pub execution_resources: ExecutionResources,
    pub l2_to_l1_messages: Vec<MsgToL1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l1_to_l2_consumed_message: Option<MsgToL2>,
    pub events: Vec<Event>,
    pub execution_status: ExecutionStatus,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revert_error: Option<String>,
}

impl ConfirmedReceipt {
    pub fn new(
        transaction_receipt: mp_receipt::TransactionReceipt,
        l1_to_l2_consumed_message: Option<MsgToL2>,
        index: u64,
    ) -> Self {
        let (execution_status, revert_error) = match transaction_receipt.execution_result() {
            mp_receipt::ExecutionResult::Succeeded => (ExecutionStatus::Succeeded, None),
            mp_receipt::ExecutionResult::Reverted { reason } => (ExecutionStatus::Reverted, Some(reason)),
        };

        Self {
            transaction_hash: transaction_receipt.transaction_hash(),
            transaction_index: index,
            actual_fee: transaction_receipt.actual_fee().amount,
            execution_resources: transaction_receipt.execution_resources().clone().into(),
            l2_to_l1_messages: transaction_receipt.messages_sent().iter().map(|msg| msg.clone().into()).collect(),
            l1_to_l2_consumed_message,
            events: transaction_receipt.events().to_vec(),
            execution_status,
            revert_error,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResources {
    pub builtin_instance_counter: BuiltinCounters,
    pub n_steps: u64,
    pub n_memory_holes: u64,
    pub data_availability: Option<L1Gas>,
    pub total_gas_consumed: Option<L1Gas>,
}

impl From<mp_receipt::ExecutionResources> for ExecutionResources {
    fn from(resources: mp_receipt::ExecutionResources) -> Self {
        Self {
            builtin_instance_counter: BuiltinCounters {
                output_builtin: 0,
                pedersen_builtin: resources.pedersen_builtin_applications.unwrap_or(0),
                range_check_builtin: resources.range_check_builtin_applications.unwrap_or(0),
                ecdsa_builtin: resources.ecdsa_builtin_applications.unwrap_or(0),
                bitwise_builtin: resources.bitwise_builtin_applications.unwrap_or(0),
                ec_op_builtin: resources.ec_op_builtin_applications.unwrap_or(0),
                keccak_builtin: resources.keccak_builtin_applications.unwrap_or(0),
                poseidon_builtin: resources.poseidon_builtin_applications.unwrap_or(0),
                segment_arena_builtin: resources.segment_arena_builtin.unwrap_or(0),
                add_mod_builtin: 0,
                mul_mod_builtin: 0,
            },
            n_steps: resources.steps,
            n_memory_holes: resources.memory_holes.unwrap_or(0),
            data_availability: Some(resources.data_availability),
            total_gas_consumed: Some(resources.total_gas_consumed),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct BuiltinCounters {
    #[serde(skip_serializing_if = "is_zero")]
    pub output_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub pedersen_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub range_check_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub ecdsa_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub bitwise_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub ec_op_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub keccak_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub poseidon_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub segment_arena_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub add_mod_builtin: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub mul_mod_builtin: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MsgToL1 {
    pub from_address: Felt,
    pub to_address: H160,
    pub payload: Vec<Felt>,
}

impl From<mp_receipt::MsgToL1> for MsgToL1 {
    fn from(msg: mp_receipt::MsgToL1) -> Self {
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(&msg.to_address.to_bytes_be()[..20]);
        let to_address = H160::from_slice(&bytes);
        Self { from_address: msg.from_address, to_address, payload: msg.payload }
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MsgToL2 {
    pub from_address: H160,
    pub to_address: Felt,
    pub selector: Felt,
    pub payload: Vec<Felt>,
    pub nonce: Option<Felt>,
}

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionStatus {
    #[default]
    Succeeded,
    Reverted,
}
