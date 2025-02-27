use primitive_types::H256;
use starknet_types_core::felt::Felt;

use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, Event, ExecutionResources,
    ExecutionResult, FeePayment, InvokeTransactionReceipt, L1Gas, L1HandlerTransactionReceipt, MsgToL1, PriceUnit,
    TransactionReceipt,
};

impl TransactionReceipt {
    pub fn to_starknet_types(
        self,
        finality_status: starknet_types_rpc::TxnFinalityStatus,
    ) -> starknet_types_rpc::TxnReceipt<Felt> {
        match self {
            TransactionReceipt::Invoke(receipt) => {
                starknet_types_rpc::TxnReceipt::Invoke(receipt.to_starknet_types(finality_status))
            }
            TransactionReceipt::L1Handler(receipt) => {
                starknet_types_rpc::TxnReceipt::L1Handler(receipt.to_starknet_types(finality_status))
            }
            TransactionReceipt::Declare(receipt) => {
                starknet_types_rpc::TxnReceipt::Declare(receipt.to_starknet_types(finality_status))
            }
            TransactionReceipt::Deploy(receipt) => {
                starknet_types_rpc::TxnReceipt::Deploy(receipt.to_starknet_types(finality_status))
            }
            TransactionReceipt::DeployAccount(receipt) => {
                starknet_types_rpc::TxnReceipt::DeployAccount(receipt.to_starknet_types(finality_status))
            }
        }
    }
}

impl InvokeTransactionReceipt {
    pub fn to_starknet_types(
        self,
        finality_status: starknet_types_rpc::TxnFinalityStatus,
    ) -> starknet_types_rpc::InvokeTxnReceipt<Felt> {
        starknet_types_rpc::InvokeTxnReceipt::<Felt> {
            common_receipt_properties: starknet_types_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(starknet_types_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(starknet_types_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

// FIXME: The `message_hash` field is currently defined as a u64 in the RPC schema,
// which is insufficient for representing the full 256-bit hash value.
// See: https://github.com/starknet-io/types-rs/issues/103
impl L1HandlerTransactionReceipt {
    pub fn to_starknet_types(
        self,
        finality_status: starknet_types_rpc::TxnFinalityStatus,
    ) -> starknet_types_rpc::L1HandlerTxnReceipt<Felt> {
        starknet_types_rpc::L1HandlerTxnReceipt::<Felt> {
            // We have to manually convert the H256 bytes to a hex hash as the
            // impl of Display for H256 skips the middle bytes.
            message_hash: hash_as_string(self.message_hash),
            common_receipt_properties: starknet_types_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(starknet_types_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(starknet_types_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

/// Gets the **full** string hex representation of an [H256].
///
/// This is necessary as the default implementation of [ToString] for [H256]
/// will keep only the first and last 2 bytes, eliding the rest with '...'.
fn hash_as_string(message_hash: H256) -> String {
    use std::fmt::Write;

    // 32 bytes x 2 (1 hex char = 4 bits) + 2 (for 0x)
    let mut acc = String::with_capacity(68);
    acc.push_str("0x");

    message_hash.as_fixed_bytes().iter().fold(acc, |mut acc, b| {
        write!(&mut acc, "{b:02x}").expect("Pre-allocated");
        acc
    })
}

impl DeclareTransactionReceipt {
    pub fn to_starknet_types(
        self,
        finality_status: starknet_types_rpc::TxnFinalityStatus,
    ) -> starknet_types_rpc::DeclareTxnReceipt<Felt> {
        starknet_types_rpc::DeclareTxnReceipt::<Felt> {
            common_receipt_properties: starknet_types_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(starknet_types_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(starknet_types_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl DeployTransactionReceipt {
    pub fn to_starknet_types(
        self,
        finality_status: starknet_types_rpc::TxnFinalityStatus,
    ) -> starknet_types_rpc::DeployTxnReceipt<Felt> {
        starknet_types_rpc::DeployTxnReceipt::<Felt> {
            contract_address: self.contract_address,
            common_receipt_properties: starknet_types_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(starknet_types_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(starknet_types_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl DeployAccountTransactionReceipt {
    pub fn to_starknet_types(
        self,
        finality_status: starknet_types_rpc::TxnFinalityStatus,
    ) -> starknet_types_rpc::DeployAccountTxnReceipt<Felt> {
        starknet_types_rpc::DeployAccountTxnReceipt::<Felt> {
            contract_address: self.contract_address,
            common_receipt_properties: starknet_types_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(starknet_types_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(starknet_types_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl From<FeePayment> for starknet_types_rpc::FeePayment<Felt> {
    fn from(fee: FeePayment) -> Self {
        Self { amount: fee.amount, unit: fee.unit.into() }
    }
}

impl From<PriceUnit> for starknet_types_rpc::PriceUnit {
    fn from(unit: PriceUnit) -> Self {
        match unit {
            PriceUnit::Wei => starknet_types_rpc::PriceUnit::Wei,
            PriceUnit::Fri => starknet_types_rpc::PriceUnit::Fri,
        }
    }
}

impl From<MsgToL1> for starknet_types_rpc::MsgToL1<Felt> {
    fn from(msg: MsgToL1) -> Self {
        Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
    }
}

impl From<Event> for starknet_types_rpc::Event<Felt> {
    fn from(event: Event) -> Self {
        Self {
            from_address: event.from_address,
            event_content: starknet_types_rpc::EventContent { keys: event.keys, data: event.data },
        }
    }
}

impl From<ExecutionResources> for starknet_types_rpc::ExecutionResources {
    fn from(resources: ExecutionResources) -> Self {
        Self {
            bitwise_builtin_applications: nullify_zero(resources.bitwise_builtin_applications),
            ec_op_builtin_applications: nullify_zero(resources.ec_op_builtin_applications),
            ecdsa_builtin_applications: nullify_zero(resources.ecdsa_builtin_applications),
            keccak_builtin_applications: nullify_zero(resources.keccak_builtin_applications),
            memory_holes: nullify_zero(resources.memory_holes),
            pedersen_builtin_applications: nullify_zero(resources.pedersen_builtin_applications),
            poseidon_builtin_applications: nullify_zero(resources.poseidon_builtin_applications),
            range_check_builtin_applications: nullify_zero(resources.range_check_builtin_applications),
            segment_arena_builtin: nullify_zero(resources.segment_arena_builtin),
            steps: resources.steps,
            data_availability: resources.data_availability.into(),
        }
    }
}

fn nullify_zero(u: u64) -> Option<u64> {
    match u {
        0 => None,
        _ => Some(u),
    }
}

impl From<L1Gas> for starknet_types_rpc::DataAvailability {
    fn from(resources: L1Gas) -> Self {
        Self { l1_gas: resources.l1_gas, l1_data_gas: resources.l1_data_gas }
    }
}

impl From<ExecutionResult> for starknet_types_rpc::ExecutionStatus {
    fn from(result: ExecutionResult) -> Self {
        match result {
            ExecutionResult::Succeeded => starknet_types_rpc::ExecutionStatus::Successful,
            ExecutionResult::Reverted { reason } => starknet_types_rpc::ExecutionStatus::Reverted(reason),
        }
    }
}

#[cfg(test)]
mod test {
    use primitive_types::H256;

    use crate::{to_starknet_types::hash_as_string, L1HandlerTransactionReceipt};

    #[test]
    fn test_hash_as_string() {
        let mut hash = String::with_capacity(68);
        hash.push_str("0x");
        hash.push_str(&"f".repeat(64));
        assert_eq!(hash_as_string(H256::from_slice(&[u8::MAX; 32])), hash);
    }

    /// The default implementation of [ToString] for [H256] will keep only the
    /// first and last 2 bytes, eliding the rest with '...'. This test makes
    /// sure this is not the case and we are using [hash_as_string] instead.
    #[test]
    fn test_l1_tx_receipt_full_hash() {
        let l1_transaction_receipt =
            L1HandlerTransactionReceipt { message_hash: H256::from_slice(&[u8::MAX; 32]), ..Default::default() };
        let message_hash =
            l1_transaction_receipt.to_starknet_types(starknet_types_rpc::TxnFinalityStatus::L1).message_hash;

        let mut hash = String::with_capacity(68);
        hash.push_str("0x");
        hash.push_str(&"f".repeat(64));
        assert_eq!(message_hash, hash);
        assert!(!message_hash.contains("."));
    }
}
