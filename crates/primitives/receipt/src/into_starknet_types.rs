use starknet_types_core::felt::Felt;

use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, Event, ExecutionResources,
    ExecutionResult, FeePayment, InvokeTransactionReceipt, L1Gas, L1HandlerTransactionReceipt, MsgToL1, PriceUnit,
    TransactionReceipt,
};

// impl From<starknet_types_rpc::TxnReceipt<Felt>> for TransactionReceipt {
//     fn from(receipt: starknet_types_rpc::TxnReceipt<Felt>) -> Self {
//         match receipt {
//             starknet_types_rpc::TxnReceipt::<Felt>::Invoke(receipt) => TransactionReceipt::Invoke(receipt.into()),
//             starknet_types_rpc::TxnReceipt::<Felt>::L1Handler(receipt) => TransactionReceipt::L1Handler(receipt.into()),
//             starknet_types_rpc::TxnReceipt::<Felt>::Declare(receipt) => TransactionReceipt::Declare(receipt.into()),
//             starknet_types_rpc::TxnReceipt::<Felt>::Deploy(receipt) => TransactionReceipt::Deploy(receipt.into()),
//             starknet_types_rpc::TxnReceipt::<Felt>::DeployAccount(receipt) => {
//                 TransactionReceipt::DeployAccount(receipt.into())
//             }
//         }
//     }
// }

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

// impl From<starknet_types_rpc::InvokeTxnReceipt<Felt>> for InvokeTransactionReceipt {
//     fn from(receipt: starknet_types_rpc::InvokeTxnReceipt<Felt>) -> Self {
//         Self {
//             transaction_hash: receipt.common_receipt_properties.transaction_hash,
//             actual_fee: receipt.common_receipt_properties.actual_fee.into(),
//             messages_sent: receipt.common_receipt_properties.messages_sent.into_iter().map(MsgToL1::from).collect(),
//             events: receipt.common_receipt_properties.events.into_iter().map(Event::from).collect(),
//             execution_resources: receipt.common_receipt_properties.execution_resources.into(),
//             execution_result: receipt.common_receipt_properties.anon.into(),
//         }
//     }
// }

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
                anon: self.execution_result.into(),
            },
        }
    }
}

// impl From<starknet_types_rpc::L1HandlerTransactionReceipt> for L1HandlerTransactionReceipt {
//     fn from(receipt: starknet_types_rpc::L1HandlerTransactionReceipt) -> Self {
//         Self {
//             message_hash: receipt.message_hash.try_into().unwrap_or_default(),
//             transaction_hash: receipt.transaction_hash,
//             actual_fee: receipt.actual_fee.into(),
//             messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
//             events: receipt.events.into_iter().map(Event::from).collect(),
//             execution_resources: receipt.execution_resources.into(),
//             execution_result: receipt.execution_result.into(),
//         }
//     }
// }

// FIXME: The `message_hash` field is currently defined as a u64 in the RPC schema,
// which is insufficient for representing the full 256-bit hash value.
// See: https://github.com/starknet-io/types-rs/issues/103
impl L1HandlerTransactionReceipt {
    pub fn to_starknet_types(
        self,
        finality_status: starknet_types_rpc::TxnFinalityStatus,
    ) -> starknet_types_rpc::L1HandlerTxnReceipt<Felt> {
        starknet_types_rpc::L1HandlerTxnReceipt::<Felt> {
            message_hash: 0, // Placeholder until the field is updated in the schema
            common_receipt_properties: starknet_types_rpc::CommonReceiptProperties {
                actual_fee: self.actual_fee.into(),
                events: self.events.into_iter().map(starknet_types_rpc::Event::from).collect(),
                execution_resources: self.execution_resources.into(),
                finality_status,
                messages_sent: self.messages_sent.into_iter().map(starknet_types_rpc::MsgToL1::from).collect(),
                transaction_hash: self.transaction_hash,
                anon: self.execution_result.into(),
            },
        }
    }
}

// impl From<starknet_types_rpc::DeclareTransactionReceipt> for DeclareTransactionReceipt {
//     fn from(receipt: starknet_types_rpc::DeclareTransactionReceipt) -> Self {
//         Self {
//             transaction_hash: receipt.transaction_hash,
//             actual_fee: receipt.actual_fee.into(),
//             messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
//             events: receipt.events.into_iter().map(Event::from).collect(),
//             execution_resources: receipt.execution_resources.into(),
//             execution_result: receipt.execution_result.into(),
//         }
//     }
// }

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
                anon: self.execution_result.into(),
            },
        }
    }
}

// impl From<starknet_types_rpc::DeployTransactionReceipt> for DeployTransactionReceipt {
//     fn from(receipt: starknet_types_rpc::DeployTransactionReceipt) -> Self {
//         Self {
//             transaction_hash: receipt.transaction_hash,
//             actual_fee: receipt.actual_fee.into(),
//             messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
//             events: receipt.events.into_iter().map(Event::from).collect(),
//             execution_resources: receipt.execution_resources.into(),
//             execution_result: receipt.execution_result.into(),
//             contract_address: receipt.contract_address,
//         }
//     }
// }

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
                anon: self.execution_result.into(),
            },
        }
    }
}

// impl From<starknet_types_rpc::DeployAccountTransactionReceipt> for DeployAccountTransactionReceipt {
//     fn from(receipt: starknet_types_rpc::DeployAccountTransactionReceipt) -> Self {
//         Self {
//             transaction_hash: receipt.transaction_hash,
//             actual_fee: receipt.actual_fee.into(),
//             messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
//             events: receipt.events.into_iter().map(Event::from).collect(),
//             execution_resources: receipt.execution_resources.into(),
//             execution_result: receipt.execution_result.into(),
//             contract_address: receipt.contract_address,
//         }
//     }
// }

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
                anon: self.execution_result.into(),
            },
        }
    }
}

// impl From<starknet_types_rpc::FeePayment<Felt>> for FeePayment {
//     fn from(payment: starknet_types_rpc::FeePayment<Felt>) -> Self {
//         Self { amount: payment.amount, unit: payment.unit.into() }
//     }
// }

impl From<FeePayment> for starknet_types_rpc::FeePayment<Felt> {
    fn from(fee: FeePayment) -> Self {
        Self { amount: fee.amount, unit: fee.unit.into() }
    }
}

// impl From<starknet_types_rpc::PriceUnit> for PriceUnit {
//     fn from(unit: starknet_types_rpc::PriceUnit) -> Self {
//         match unit {
//             starknet_types_rpc::PriceUnit::Fri => PriceUnit::Fri,
//             starknet_types_rpc::PriceUnit::Wei => PriceUnit::Wei,
//         }
//     }
// }

impl From<PriceUnit> for starknet_types_rpc::PriceUnit {
    fn from(unit: PriceUnit) -> Self {
        match unit {
            PriceUnit::Wei => starknet_types_rpc::PriceUnit::Wei,
            PriceUnit::Fri => starknet_types_rpc::PriceUnit::Fri,
        }
    }
}

// impl From<starknet_types_rpc::MsgToL1<Felt>> for MsgToL1 {
//     fn from(msg: starknet_types_rpc::MsgToL1<Felt>) -> Self {
//         Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
//     }
// }

impl From<MsgToL1> for starknet_types_rpc::MsgToL1<Felt> {
    fn from(msg: MsgToL1) -> Self {
        Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
    }
}

// impl From<starknet_types_rpc::Event<Felt>> for Event {
//     fn from(event: starknet_types_rpc::Event<Felt>) -> Self {
//         Self { from_address: event.from_address, keys: event.keys, data: event.data }
//     }
// }

impl From<Event> for starknet_types_rpc::Event<Felt> {
    fn from(event: Event) -> Self {
        Self {
            from_address: event.from_address,
            event_content: starknet_types_rpc::EventContent { keys: event.keys, data: event.data },
        }
    }
}

// impl From<starknet_types_rpc::ExecutionResources> for ExecutionResources {
//     fn from(resources: starknet_types_rpc::ExecutionResources) -> Self {
//         let computation_resources = resources.computation_resources;
//         Self {
//             steps: computation_resources.steps,
//             memory_holes: computation_resources.memory_holes,
//             range_check_builtin_applications: computation_resources.range_check_builtin_applications,
//             pedersen_builtin_applications: computation_resources.pedersen_builtin_applications,
//             poseidon_builtin_applications: computation_resources.poseidon_builtin_applications,
//             ec_op_builtin_applications: computation_resources.ec_op_builtin_applications,
//             ecdsa_builtin_applications: computation_resources.ecdsa_builtin_applications,
//             bitwise_builtin_applications: computation_resources.bitwise_builtin_applications,
//             keccak_builtin_applications: computation_resources.keccak_builtin_applications,
//             segment_arena_builtin: computation_resources.segment_arena_builtin,
//             data_availability: resources.data_availability.into(),
//             total_gas_consumed: Default::default(),
//         }
//     }
// }

impl From<ExecutionResources> for starknet_types_rpc::ExecutionResources {
    fn from(resources: ExecutionResources) -> Self {
        Self {
            bitwise_builtin_applications: resources.bitwise_builtin_applications,
            ec_op_builtin_applications: resources.ec_op_builtin_applications,
            ecdsa_builtin_applications: resources.ecdsa_builtin_applications,
            keccak_builtin_applications: resources.keccak_builtin_applications,
            memory_holes: resources.memory_holes,
            pedersen_builtin_applications: resources.pedersen_builtin_applications,
            poseidon_builtin_applications: resources.poseidon_builtin_applications,
            range_check_builtin_applications: resources.range_check_builtin_applications,
            segment_arena_builtin: resources.segment_arena_builtin,
            steps: resources.steps,
            data_availability: resources.data_availability.into(),
        }
    }
}

// impl From<starknet_types_rpc::DataAvailability> for L1Gas {
//     fn from(resources: starknet_types_rpc::DataAvailability) -> Self {
//         Self { l1_gas: resources.l1_gas.into(), l1_data_gas: resources.l1_data_gas.into() }
//     }
// }

impl From<L1Gas> for starknet_types_rpc::DataAvailability {
    fn from(resources: L1Gas) -> Self {
        Self { l1_gas: resources.l1_gas, l1_data_gas: resources.l1_data_gas }
    }
}

// impl From<starknet_types_rpc::Anonymous> for ExecutionResult {
//     fn from(result: starknet_types_rpc::Anonymous) -> Self {
//         match result {
//             starknet_types_rpc::Anonymous::Successful(_) => ExecutionResult::Succeeded,
//             starknet_types_rpc::Anonymous::Reverted(starknet_types_rpc::RevertedCommonReceiptProperties {
//                 revert_reason,
//                 ..
//             }) => ExecutionResult::Reverted { reason: revert_reason },
//         }
//     }
// }

impl From<ExecutionResult> for starknet_types_rpc::Anonymous {
    fn from(result: ExecutionResult) -> Self {
        match result {
            ExecutionResult::Succeeded => {
                starknet_types_rpc::Anonymous::Successful(starknet_types_rpc::SuccessfulCommonReceiptProperties {
                    execution_status: "SUCCEEDED".to_string(),
                })
            }
            ExecutionResult::Reverted { reason } => {
                starknet_types_rpc::Anonymous::Reverted(starknet_types_rpc::RevertedCommonReceiptProperties {
                    execution_status: "REVERTED".to_string(),
                    revert_reason: reason,
                })
            }
        }
    }
}

// #[cfg(test)]
// mod test {
//     use super::*;
//     use crate::{
//         tests::dummy_declare_receipt, tests::dummy_deploy_account_receipt, tests::dummy_deploy_receipt,
//         tests::dummy_invoke_receipt, tests::dummy_l1_handler_receipt,
//     };

//     #[test]
//     fn test_into_starknet_types_receipt() {
//         let receipt: TransactionReceipt = dummy_invoke_receipt().into();
//         let core_receipt = receipt.clone().to_starknet_types(starknet_types_rpc::TxnFinalityStatus::L1);
//         let receipt_back: TransactionReceipt = core_receipt.into();
//         assert_eq!(receipt, receipt_back);

//         let receipt: TransactionReceipt = dummy_l1_handler_receipt().into();
//         let core_receipt = receipt.clone().to_starknet_types(starknet_types_rpc::TxnFinalityStatus::L1);
//         let receipt_back: TransactionReceipt = core_receipt.into();
//         assert_eq!(receipt, receipt_back);

//         let receipt: TransactionReceipt = dummy_declare_receipt().into();
//         let core_receipt = receipt.clone().to_starknet_types(starknet_types_rpc::TxnFinalityStatus::L1);
//         let receipt_back: TransactionReceipt = core_receipt.into();
//         assert_eq!(receipt, receipt_back);

//         let receipt: TransactionReceipt = dummy_deploy_receipt().into();
//         let core_receipt = receipt.clone().to_starknet_types(starknet_types_rpc::TxnFinalityStatus::L1);
//         let receipt_back: TransactionReceipt = core_receipt.into();
//         assert_eq!(receipt, receipt_back);

//         let receipt: TransactionReceipt = dummy_deploy_account_receipt().into();
//         let core_receipt = receipt.clone().to_starknet_types(starknet_types_rpc::TxnFinalityStatus::L1);
//         let receipt_back: TransactionReceipt = core_receipt.into();
//         assert_eq!(receipt, receipt_back);
//     }
// }
