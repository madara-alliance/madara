use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, Event, ExecutionResources,
    ExecutionResult, FeePayment, GasVector, InvokeTransactionReceipt, L1HandlerTransactionReceipt, MsgToL1, PriceUnit,
    TransactionReceipt,
};

impl TransactionReceipt {
    pub fn to_rpc_v0_7(self, finality_status: mp_rpc::v0_7_1::TxnFinalityStatus) -> mp_rpc::v0_7_1::TxnReceipt {
        match self {
            TransactionReceipt::Invoke(receipt) => {
                mp_rpc::v0_7_1::TxnReceipt::Invoke(receipt.to_rpc_v0_7(finality_status))
            }
            TransactionReceipt::L1Handler(receipt) => {
                mp_rpc::v0_7_1::TxnReceipt::L1Handler(receipt.to_rpc_v0_7(finality_status))
            }
            TransactionReceipt::Declare(receipt) => {
                mp_rpc::v0_7_1::TxnReceipt::Declare(receipt.to_rpc_v0_7(finality_status))
            }
            TransactionReceipt::Deploy(receipt) => {
                mp_rpc::v0_7_1::TxnReceipt::Deploy(receipt.to_rpc_v0_7(finality_status))
            }
            TransactionReceipt::DeployAccount(receipt) => {
                mp_rpc::v0_7_1::TxnReceipt::DeployAccount(receipt.to_rpc_v0_7(finality_status))
            }
        }
    }
    pub fn to_rpc_v0_9(self, finality_status: mp_rpc::v0_9_0::TxnFinalityStatus) -> mp_rpc::v0_9_0::TxnReceipt {
        match self {
            TransactionReceipt::Invoke(receipt) => {
                mp_rpc::v0_9_0::TxnReceipt::Invoke(receipt.to_rpc_v0_9(finality_status))
            }
            TransactionReceipt::L1Handler(receipt) => {
                mp_rpc::v0_9_0::TxnReceipt::L1Handler(receipt.to_rpc_v0_9(finality_status))
            }
            TransactionReceipt::Declare(receipt) => {
                mp_rpc::v0_9_0::TxnReceipt::Declare(receipt.to_rpc_v0_9(finality_status))
            }
            TransactionReceipt::Deploy(receipt) => {
                mp_rpc::v0_9_0::TxnReceipt::Deploy(receipt.to_rpc_v0_9(finality_status))
            }
            TransactionReceipt::DeployAccount(receipt) => {
                mp_rpc::v0_9_0::TxnReceipt::DeployAccount(receipt.to_rpc_v0_9(finality_status))
            }
        }
    }
}

impl InvokeTransactionReceipt {
    pub fn to_rpc_v0_7(self, finality_status: mp_rpc::v0_7_1::TxnFinalityStatus) -> mp_rpc::v0_7_1::InvokeTxnReceipt {
        mp_rpc::v0_7_1::InvokeTxnReceipt {
            common_receipt_properties: mp_rpc::v0_7_1::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_7_1::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_7_1::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
    pub fn to_rpc_v0_9(self, finality_status: mp_rpc::v0_9_0::TxnFinalityStatus) -> mp_rpc::v0_9_0::InvokeTxnReceipt {
        mp_rpc::v0_9_0::InvokeTxnReceipt {
            common_receipt_properties: mp_rpc::v0_9_0::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_9_0::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_9_0::MsgToL1::from).collect(),
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
    pub fn to_rpc_v0_7(
        self,
        finality_status: mp_rpc::v0_7_1::TxnFinalityStatus,
    ) -> mp_rpc::v0_7_1::L1HandlerTxnReceipt {
        mp_rpc::v0_7_1::L1HandlerTxnReceipt {
            // We have to manually convert the H256 bytes to a hex hash as the
            // impl of Display for H256 skips the middle bytes.
            message_hash: format!("{}", self.message_hash),
            common_receipt_properties: mp_rpc::v0_7_1::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_7_1::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_7_1::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
    pub fn to_rpc_v0_9(
        self,
        finality_status: mp_rpc::v0_9_0::TxnFinalityStatus,
    ) -> mp_rpc::v0_9_0::L1HandlerTxnReceipt {
        mp_rpc::v0_9_0::L1HandlerTxnReceipt {
            // We have to manually convert the H256 bytes to a hex hash as the
            // impl of Display for H256 skips the middle bytes.
            message_hash: format!("{}", self.message_hash),
            common_receipt_properties: mp_rpc::v0_9_0::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_9_0::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_9_0::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl DeclareTransactionReceipt {
    pub fn to_rpc_v0_7(self, finality_status: mp_rpc::v0_7_1::TxnFinalityStatus) -> mp_rpc::v0_7_1::DeclareTxnReceipt {
        mp_rpc::v0_7_1::DeclareTxnReceipt {
            common_receipt_properties: mp_rpc::v0_7_1::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_7_1::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_7_1::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
    pub fn to_rpc_v0_9(self, finality_status: mp_rpc::v0_9_0::TxnFinalityStatus) -> mp_rpc::v0_9_0::DeclareTxnReceipt {
        mp_rpc::v0_9_0::DeclareTxnReceipt {
            common_receipt_properties: mp_rpc::v0_9_0::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_9_0::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_9_0::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl DeployTransactionReceipt {
    pub fn to_rpc_v0_7(self, finality_status: mp_rpc::v0_7_1::TxnFinalityStatus) -> mp_rpc::v0_7_1::DeployTxnReceipt {
        mp_rpc::v0_7_1::DeployTxnReceipt {
            contract_address: self.contract_address,
            common_receipt_properties: mp_rpc::v0_7_1::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_7_1::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_7_1::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
    pub fn to_rpc_v0_9(self, finality_status: mp_rpc::v0_9_0::TxnFinalityStatus) -> mp_rpc::v0_9_0::DeployTxnReceipt {
        mp_rpc::v0_9_0::DeployTxnReceipt {
            contract_address: self.contract_address,
            common_receipt_properties: mp_rpc::v0_9_0::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_9_0::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_9_0::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl DeployAccountTransactionReceipt {
    pub fn to_rpc_v0_7(
        self,
        finality_status: mp_rpc::v0_7_1::TxnFinalityStatus,
    ) -> mp_rpc::v0_7_1::DeployAccountTxnReceipt {
        mp_rpc::v0_7_1::DeployAccountTxnReceipt {
            contract_address: self.contract_address,
            common_receipt_properties: mp_rpc::v0_7_1::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_7_1::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_7_1::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
    pub fn to_rpc_v0_9(
        self,
        finality_status: mp_rpc::v0_9_0::TxnFinalityStatus,
    ) -> mp_rpc::v0_9_0::DeployAccountTxnReceipt {
        mp_rpc::v0_9_0::DeployAccountTxnReceipt {
            contract_address: self.contract_address,
            common_receipt_properties: mp_rpc::v0_9_0::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(mp_rpc::v0_9_0::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(mp_rpc::v0_9_0::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                execution_status: self.execution_result.into(),
            },
        }
    }
}

impl From<FeePayment> for mp_rpc::v0_7_1::FeePayment {
    fn from(fee: FeePayment) -> Self {
        Self { amount: fee.amount, unit: fee.unit.into() }
    }
}

impl From<PriceUnit> for mp_rpc::v0_7_1::PriceUnit {
    fn from(unit: PriceUnit) -> Self {
        match unit {
            PriceUnit::Wei => mp_rpc::v0_7_1::PriceUnit::Wei,
            PriceUnit::Fri => mp_rpc::v0_7_1::PriceUnit::Fri,
        }
    }
}

impl From<FeePayment> for mp_rpc::v0_9_0::FeePayment {
    fn from(fee: FeePayment) -> Self {
        Self { amount: fee.amount, unit: fee.unit.into() }
    }
}

impl From<PriceUnit> for mp_rpc::v0_9_0::PriceUnit {
    fn from(unit: PriceUnit) -> Self {
        match unit {
            PriceUnit::Wei => mp_rpc::v0_9_0::PriceUnit::Wei(mp_rpc::v0_9_0::PriceUnitWei::Wei),
            PriceUnit::Fri => mp_rpc::v0_9_0::PriceUnit::Fri(mp_rpc::v0_9_0::PriceUnitFri::Fri),
        }
    }
}

impl From<MsgToL1> for mp_rpc::v0_7_1::MsgToL1 {
    fn from(msg: MsgToL1) -> Self {
        Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
    }
}

impl From<Event> for mp_rpc::v0_7_1::Event {
    fn from(event: Event) -> Self {
        Self {
            from_address: event.from_address,
            event_content: mp_rpc::v0_7_1::EventContent { keys: event.keys, data: event.data },
        }
    }
}

impl From<ExecutionResources> for mp_rpc::v0_7_1::ExecutionResources {
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

impl From<GasVector> for mp_rpc::v0_7_1::DataAvailability {
    fn from(resources: GasVector) -> Self {
        Self { l1_gas: resources.l1_gas, l1_data_gas: resources.l1_data_gas }
    }
}

impl From<ExecutionResult> for mp_rpc::v0_7_1::ExecutionStatus {
    fn from(result: ExecutionResult) -> Self {
        match result {
            ExecutionResult::Succeeded => mp_rpc::v0_7_1::ExecutionStatus::Successful,
            ExecutionResult::Reverted { reason } => mp_rpc::v0_7_1::ExecutionStatus::Reverted(reason),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::L1HandlerTransactionReceipt;
    use starknet_core::types::Hash256;

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
        let message_hash = l1_transaction_receipt.to_rpc_v0_7(mp_rpc::v0_7_1::TxnFinalityStatus::L1).message_hash;

        let mut hash = String::with_capacity(68);
        hash.push_str("0x");
        hash.push_str(&"f".repeat(64));
        assert_eq!(message_hash, hash);
        assert!(!message_hash.contains("."));
    }
}
