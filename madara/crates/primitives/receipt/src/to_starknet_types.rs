use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, Event, ExecutionResources,
    ExecutionResult, FeePayment, GasVector, InvokeTransactionReceipt, L1HandlerTransactionReceipt, MsgToL1, PriceUnit,
    TransactionReceipt,
};

impl TransactionReceipt {
    pub fn to_starknet_types(self, finality_status: mp_rpc::TxnFinalityStatus) -> mp_rpc::TxnReceipt {
        match self {
            TransactionReceipt::Invoke(receipt) => {
                mp_rpc::TxnReceipt::Invoke(receipt.to_starknet_types(finality_status))
            }
            TransactionReceipt::L1Handler(receipt) => {
                mp_rpc::TxnReceipt::L1Handler(receipt.to_starknet_types(finality_status))
            }
            TransactionReceipt::Declare(receipt) => {
                mp_rpc::TxnReceipt::Declare(receipt.to_starknet_types(finality_status))
            }
            TransactionReceipt::Deploy(receipt) => {
                mp_rpc::TxnReceipt::Deploy(receipt.to_starknet_types(finality_status))
            }
            TransactionReceipt::DeployAccount(receipt) => {
                mp_rpc::TxnReceipt::DeployAccount(receipt.to_starknet_types(finality_status))
            }
        }
    }
}

impl InvokeTransactionReceipt {
    pub fn to_starknet_types(self, finality_status: mp_rpc::TxnFinalityStatus) -> mp_rpc::InvokeTxnReceipt {
        mp_rpc::InvokeTxnReceipt {
            common_receipt_properties: mp_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::MsgToL1::from).collect(),
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
    pub fn to_starknet_types(self, finality_status: mp_rpc::TxnFinalityStatus) -> mp_rpc::L1HandlerTxnReceipt {
        mp_rpc::L1HandlerTxnReceipt {
            // We have to manually convert the H256 bytes to a hex hash as the
            // impl of Display for H256 skips the middle bytes.
            message_hash: format!("{}", self.message_hash),
            common_receipt_properties: mp_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl DeclareTransactionReceipt {
    pub fn to_starknet_types(self, finality_status: mp_rpc::TxnFinalityStatus) -> mp_rpc::DeclareTxnReceipt {
        mp_rpc::DeclareTxnReceipt {
            common_receipt_properties: mp_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl DeployTransactionReceipt {
    pub fn to_starknet_types(self, finality_status: mp_rpc::TxnFinalityStatus) -> mp_rpc::DeployTxnReceipt {
        mp_rpc::DeployTxnReceipt {
            contract_address: self.contract_address,
            common_receipt_properties: mp_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl DeployAccountTransactionReceipt {
    pub fn to_starknet_types(self, finality_status: mp_rpc::TxnFinalityStatus) -> mp_rpc::DeployAccountTxnReceipt {
        mp_rpc::DeployAccountTxnReceipt {
            contract_address: self.contract_address,
            common_receipt_properties: mp_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl From<FeePayment> for mp_rpc::FeePayment {
    fn from(fee: FeePayment) -> Self {
        Self { amount: fee.amount, unit: fee.unit.into() }
    }
}

impl From<PriceUnit> for mp_rpc::PriceUnit {
    fn from(unit: PriceUnit) -> Self {
        match unit {
            PriceUnit::Wei => mp_rpc::PriceUnit::Wei,
            PriceUnit::Fri => mp_rpc::PriceUnit::Fri,
        }
    }
}

impl From<MsgToL1> for mp_rpc::MsgToL1 {
    fn from(msg: MsgToL1) -> Self {
        Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
    }
}

impl From<Event> for mp_rpc::Event {
    fn from(event: Event) -> Self {
        Self {
            from_address: event.from_address,
            event_content: mp_rpc::EventContent { keys: event.keys, data: event.data },
        }
    }
}

impl From<ExecutionResources> for mp_rpc::ExecutionResources {
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

impl From<GasVector> for mp_rpc::DataAvailability {
    fn from(resources: GasVector) -> Self {
        Self { l1_gas: resources.l1_gas, l1_data_gas: resources.l1_data_gas }
    }
}

impl From<ExecutionResult> for mp_rpc::ExecutionStatus {
    fn from(result: ExecutionResult) -> Self {
        match result {
            ExecutionResult::Succeeded => mp_rpc::ExecutionStatus::Successful,
            ExecutionResult::Reverted { reason } => mp_rpc::ExecutionStatus::Reverted(reason),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::L1HandlerTransactionReceipt;
    use starknet::core::types::Hash256;

    #[test]
    fn test_hash_as_string() {
        let mut hash = String::with_capacity(68);
        hash.push_str("0x");
        hash.push_str(&"f".repeat(64));
        assert_eq!(format!("{}", Hash256::from_bytes([u8::MAX; 32])), hash);
    }

    #[test]
    fn test_l1_tx_receipt_full_hash() {
        let l1_transaction_receipt =
            L1HandlerTransactionReceipt { message_hash: Hash256::from_bytes([u8::MAX; 32]), ..Default::default() };
        let message_hash = l1_transaction_receipt.to_starknet_types(mp_rpc::TxnFinalityStatus::L1).message_hash;

        let mut hash = String::with_capacity(68);
        hash.push_str("0x");
        hash.push_str(&"f".repeat(64));
        assert_eq!(message_hash, hash);
        assert!(!message_hash.contains("."));
    }
}
