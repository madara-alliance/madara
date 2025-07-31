use crate::transaction::{
    DeclareTransaction, DeployAccountTransaction, DeployTransaction, InvokeTransaction, L1HandlerTransaction,
    Transaction,
};
use mp_receipt::{Event, L1Gas, MsgToL1, MsgToL2};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
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
            transaction_hash: *transaction_receipt.transaction_hash(),
            transaction_index: index,
            actual_fee: transaction_receipt.actual_fee().amount,
            execution_resources: transaction_receipt.execution_resources().clone().into(),
            l2_to_l1_messages: transaction_receipt.messages_sent().to_vec(),
            l1_to_l2_consumed_message,
            events: transaction_receipt.events().to_vec(),
            execution_status,
            revert_error,
        }
    }

    pub fn into_mp(self, tx: &Transaction) -> mp_receipt::TransactionReceipt {
        match tx {
            Transaction::Invoke(tx) => mp_receipt::TransactionReceipt::Invoke(self.into_mp_invoke(tx)),
            Transaction::L1Handler(tx) => mp_receipt::TransactionReceipt::L1Handler(self.into_mp_l1_handler(tx)),
            Transaction::Declare(tx) => mp_receipt::TransactionReceipt::Declare(self.into_mp_declare(tx)),
            Transaction::Deploy(tx) => mp_receipt::TransactionReceipt::Deploy(self.into_mp_deploy(tx)),
            Transaction::DeployAccount(tx) => {
                mp_receipt::TransactionReceipt::DeployAccount(self.into_mp_deploy_account(tx))
            }
        }
    }

    fn into_mp_invoke(self, tx: &InvokeTransaction) -> mp_receipt::InvokeTransactionReceipt {
        mp_receipt::InvokeTransactionReceipt {
            transaction_hash: self.transaction_hash,
            actual_fee: fee_payment(self.actual_fee, tx.version()),
            messages_sent: self.l2_to_l1_messages,
            events: self.events,
            execution_resources: self.execution_resources.into(),
            execution_result: execution_result(self.execution_status, self.revert_error),
        }
    }

    fn into_mp_l1_handler(self, tx: &L1HandlerTransaction) -> mp_receipt::L1HandlerTransactionReceipt {
        let (from_address, payload) = tx.calldata.split_first().map(|(a, b)| (*a, b)).unwrap_or((Felt::ZERO, &[]));
        let message_to_l2 = starknet_core::types::MsgToL2 {
            from_address: from_address.try_into().unwrap_or(
                Felt::ZERO.try_into().expect("Failed to cnvert Felt::ZERO to an EthAddress, this should not happen!"),
            ),
            to_address: tx.contract_address,
            selector: tx.entry_point_selector,
            payload: payload.to_vec(),
            nonce: tx.nonce.try_into().unwrap_or_default(),
        };
        let message_hash = message_to_l2.hash();

        mp_receipt::L1HandlerTransactionReceipt {
            message_hash,
            transaction_hash: self.transaction_hash,
            actual_fee: fee_payment(self.actual_fee, tx.version()),
            messages_sent: self.l2_to_l1_messages,
            events: self.events,
            execution_resources: self.execution_resources.into(),
            execution_result: execution_result(self.execution_status, self.revert_error),
        }
    }

    fn into_mp_declare(self, tx: &DeclareTransaction) -> mp_receipt::DeclareTransactionReceipt {
        mp_receipt::DeclareTransactionReceipt {
            transaction_hash: self.transaction_hash,
            actual_fee: fee_payment(self.actual_fee, tx.version()),
            messages_sent: self.l2_to_l1_messages,
            events: self.events,
            execution_resources: self.execution_resources.into(),
            execution_result: execution_result(self.execution_status, self.revert_error),
        }
    }

    fn into_mp_deploy(self, tx: &DeployTransaction) -> mp_receipt::DeployTransactionReceipt {
        mp_receipt::DeployTransactionReceipt {
            transaction_hash: self.transaction_hash,
            actual_fee: fee_payment(self.actual_fee, tx.version()),
            messages_sent: self.l2_to_l1_messages,
            events: self.events,
            execution_resources: self.execution_resources.into(),
            execution_result: execution_result(self.execution_status, self.revert_error),
            contract_address: tx.contract_address,
        }
    }

    fn into_mp_deploy_account(self, tx: &DeployAccountTransaction) -> mp_receipt::DeployAccountTransactionReceipt {
        mp_receipt::DeployAccountTransactionReceipt {
            transaction_hash: self.transaction_hash,
            actual_fee: fee_payment(self.actual_fee, tx.version()),
            messages_sent: self.l2_to_l1_messages,
            events: self.events,
            execution_resources: self.execution_resources.into(),
            execution_result: execution_result(self.execution_status, self.revert_error),
            contract_address: match tx {
                DeployAccountTransaction::V1(tx) => tx.contract_address,
                DeployAccountTransaction::V3(tx) => tx.sender_address,
            },
        }
    }
}

fn execution_result(status: ExecutionStatus, reason: Option<String>) -> mp_receipt::ExecutionResult {
    match status {
        ExecutionStatus::Succeeded => mp_receipt::ExecutionResult::Succeeded,
        ExecutionStatus::Reverted => mp_receipt::ExecutionResult::Reverted { reason: reason.unwrap_or_default() },
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct ExecutionResources {
    pub builtin_instance_counter: BuiltinCounters,
    pub n_steps: u64,
    pub n_memory_holes: u64,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_availability: Option<L1Gas>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_gas_consumed: Option<L1Gas>,
}

impl From<mp_receipt::ExecutionResources> for ExecutionResources {
    fn from(resources: mp_receipt::ExecutionResources) -> Self {
        fn none_if_zero(gas: L1Gas) -> Option<L1Gas> {
            if gas.l1_gas == 0 && gas.l1_data_gas == 0 {
                None
            } else {
                Some(gas)
            }
        }

        Self {
            builtin_instance_counter: BuiltinCounters {
                output_builtin: 0,
                pedersen_builtin: resources.pedersen_builtin_applications,
                range_check_builtin: resources.range_check_builtin_applications,
                ecdsa_builtin: resources.ecdsa_builtin_applications,
                bitwise_builtin: resources.bitwise_builtin_applications,
                ec_op_builtin: resources.ec_op_builtin_applications,
                keccak_builtin: resources.keccak_builtin_applications,
                poseidon_builtin: resources.poseidon_builtin_applications,
                segment_arena_builtin: resources.segment_arena_builtin,
                add_mod_builtin: 0,
                mul_mod_builtin: 0,
            },
            n_steps: resources.steps,
            n_memory_holes: resources.memory_holes,
            data_availability: none_if_zero(resources.data_availability),
            total_gas_consumed: none_if_zero(resources.total_gas_consumed),
        }
    }
}

impl From<ExecutionResources> for mp_receipt::ExecutionResources {
    fn from(resources: ExecutionResources) -> Self {
        let BuiltinCounters {
            output_builtin: _,
            pedersen_builtin,
            range_check_builtin,
            ecdsa_builtin,
            bitwise_builtin,
            ec_op_builtin,
            keccak_builtin,
            poseidon_builtin,
            segment_arena_builtin,
            add_mod_builtin: _,
            mul_mod_builtin: _,
        } = resources.builtin_instance_counter;

        Self {
            steps: resources.n_steps,
            memory_holes: resources.n_memory_holes,
            range_check_builtin_applications: range_check_builtin,
            pedersen_builtin_applications: pedersen_builtin,
            poseidon_builtin_applications: poseidon_builtin,
            ec_op_builtin_applications: ec_op_builtin,
            ecdsa_builtin_applications: ecdsa_builtin,
            bitwise_builtin_applications: bitwise_builtin,
            keccak_builtin_applications: keccak_builtin,
            segment_arena_builtin,
            data_availability: resources.data_availability.unwrap_or_default(),
            total_gas_consumed: resources.total_gas_consumed.unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
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
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionStatus {
    #[default]
    Succeeded,
    Reverted,
}

fn fee_payment(fee: Felt, tx_version: u8) -> mp_receipt::FeePayment {
    mp_receipt::FeePayment {
        amount: fee,
        unit: if tx_version < 3 { mp_receipt::PriceUnit::Wei } else { mp_receipt::PriceUnit::Fri },
    }
}
