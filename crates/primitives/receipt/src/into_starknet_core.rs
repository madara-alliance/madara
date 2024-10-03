use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, Event, ExecutionResources,
    ExecutionResult, FeePayment, InvokeTransactionReceipt, L1Gas, L1HandlerTransactionReceipt, MsgToL1, PriceUnit,
    TransactionReceipt,
};

impl From<starknet_core::types::TransactionReceipt> for TransactionReceipt {
    fn from(receipt: starknet_core::types::TransactionReceipt) -> Self {
        match receipt {
            starknet_core::types::TransactionReceipt::Invoke(receipt) => TransactionReceipt::Invoke(receipt.into()),
            starknet_core::types::TransactionReceipt::L1Handler(receipt) => {
                TransactionReceipt::L1Handler(receipt.into())
            }
            starknet_core::types::TransactionReceipt::Declare(receipt) => TransactionReceipt::Declare(receipt.into()),
            starknet_core::types::TransactionReceipt::Deploy(receipt) => TransactionReceipt::Deploy(receipt.into()),
            starknet_core::types::TransactionReceipt::DeployAccount(receipt) => {
                TransactionReceipt::DeployAccount(receipt.into())
            }
        }
    }
}

impl TransactionReceipt {
    pub fn to_starknet_core(
        self,
        finality_status: starknet_core::types::TransactionFinalityStatus,
    ) -> starknet_core::types::TransactionReceipt {
        match self {
            TransactionReceipt::Invoke(receipt) => {
                starknet_core::types::TransactionReceipt::Invoke(receipt.to_starknet_core(finality_status))
            }
            TransactionReceipt::L1Handler(receipt) => {
                starknet_core::types::TransactionReceipt::L1Handler(receipt.to_starknet_core(finality_status))
            }
            TransactionReceipt::Declare(receipt) => {
                starknet_core::types::TransactionReceipt::Declare(receipt.to_starknet_core(finality_status))
            }
            TransactionReceipt::Deploy(receipt) => {
                starknet_core::types::TransactionReceipt::Deploy(receipt.to_starknet_core(finality_status))
            }
            TransactionReceipt::DeployAccount(receipt) => {
                starknet_core::types::TransactionReceipt::DeployAccount(receipt.to_starknet_core(finality_status))
            }
        }
    }
}

impl From<starknet_core::types::InvokeTransactionReceipt> for InvokeTransactionReceipt {
    fn from(receipt: starknet_core::types::InvokeTransactionReceipt) -> Self {
        Self {
            transaction_hash: receipt.transaction_hash,
            actual_fee: receipt.actual_fee.into(),
            messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
            events: receipt.events.into_iter().map(Event::from).collect(),
            execution_resources: receipt.execution_resources.into(),
            execution_result: receipt.execution_result.into(),
        }
    }
}

impl InvokeTransactionReceipt {
    pub fn to_starknet_core(
        self,
        finality_status: starknet_core::types::TransactionFinalityStatus,
    ) -> starknet_core::types::InvokeTransactionReceipt {
        starknet_core::types::InvokeTransactionReceipt {
            transaction_hash: self.transaction_hash,
            actual_fee: self.actual_fee.into(),
            finality_status,
            messages_sent: self.messages_sent.into_iter().map(starknet_core::types::MsgToL1::from).collect(),
            events: self.events.into_iter().map(starknet_core::types::Event::from).collect(),
            execution_resources: self.execution_resources.into(),
            execution_result: self.execution_result.into(),
        }
    }
}

impl From<starknet_core::types::L1HandlerTransactionReceipt> for L1HandlerTransactionReceipt {
    fn from(receipt: starknet_core::types::L1HandlerTransactionReceipt) -> Self {
        Self {
            message_hash: receipt.message_hash.try_into().unwrap_or_default(),
            transaction_hash: receipt.transaction_hash,
            actual_fee: receipt.actual_fee.into(),
            messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
            events: receipt.events.into_iter().map(Event::from).collect(),
            execution_resources: receipt.execution_resources.into(),
            execution_result: receipt.execution_result.into(),
        }
    }
}

impl L1HandlerTransactionReceipt {
    pub fn to_starknet_core(
        self,
        finality_status: starknet_core::types::TransactionFinalityStatus,
    ) -> starknet_core::types::L1HandlerTransactionReceipt {
        starknet_core::types::L1HandlerTransactionReceipt {
            message_hash: self.message_hash.into(),
            transaction_hash: self.transaction_hash,
            actual_fee: self.actual_fee.into(),
            finality_status,
            messages_sent: self.messages_sent.into_iter().map(starknet_core::types::MsgToL1::from).collect(),
            events: self.events.into_iter().map(starknet_core::types::Event::from).collect(),
            execution_resources: self.execution_resources.into(),
            execution_result: self.execution_result.into(),
        }
    }
}

impl From<starknet_core::types::DeclareTransactionReceipt> for DeclareTransactionReceipt {
    fn from(receipt: starknet_core::types::DeclareTransactionReceipt) -> Self {
        Self {
            transaction_hash: receipt.transaction_hash,
            actual_fee: receipt.actual_fee.into(),
            messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
            events: receipt.events.into_iter().map(Event::from).collect(),
            execution_resources: receipt.execution_resources.into(),
            execution_result: receipt.execution_result.into(),
        }
    }
}

impl DeclareTransactionReceipt {
    pub fn to_starknet_core(
        self,
        finality_status: starknet_core::types::TransactionFinalityStatus,
    ) -> starknet_core::types::DeclareTransactionReceipt {
        starknet_core::types::DeclareTransactionReceipt {
            transaction_hash: self.transaction_hash,
            actual_fee: self.actual_fee.into(),
            finality_status,
            messages_sent: self.messages_sent.into_iter().map(starknet_core::types::MsgToL1::from).collect(),
            events: self.events.into_iter().map(starknet_core::types::Event::from).collect(),
            execution_resources: self.execution_resources.into(),
            execution_result: self.execution_result.into(),
        }
    }
}

impl From<starknet_core::types::DeployTransactionReceipt> for DeployTransactionReceipt {
    fn from(receipt: starknet_core::types::DeployTransactionReceipt) -> Self {
        Self {
            transaction_hash: receipt.transaction_hash,
            actual_fee: receipt.actual_fee.into(),
            messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
            events: receipt.events.into_iter().map(Event::from).collect(),
            execution_resources: receipt.execution_resources.into(),
            execution_result: receipt.execution_result.into(),
            contract_address: receipt.contract_address,
        }
    }
}

impl DeployTransactionReceipt {
    pub fn to_starknet_core(
        self,
        finality_status: starknet_core::types::TransactionFinalityStatus,
    ) -> starknet_core::types::DeployTransactionReceipt {
        starknet_core::types::DeployTransactionReceipt {
            transaction_hash: self.transaction_hash,
            actual_fee: self.actual_fee.into(),
            finality_status,
            messages_sent: self.messages_sent.into_iter().map(starknet_core::types::MsgToL1::from).collect(),
            events: self.events.into_iter().map(starknet_core::types::Event::from).collect(),
            execution_resources: self.execution_resources.into(),
            execution_result: self.execution_result.into(),
            contract_address: self.contract_address,
        }
    }
}

impl From<starknet_core::types::DeployAccountTransactionReceipt> for DeployAccountTransactionReceipt {
    fn from(receipt: starknet_core::types::DeployAccountTransactionReceipt) -> Self {
        Self {
            transaction_hash: receipt.transaction_hash,
            actual_fee: receipt.actual_fee.into(),
            messages_sent: receipt.messages_sent.into_iter().map(MsgToL1::from).collect(),
            events: receipt.events.into_iter().map(Event::from).collect(),
            execution_resources: receipt.execution_resources.into(),
            execution_result: receipt.execution_result.into(),
            contract_address: receipt.contract_address,
        }
    }
}

impl DeployAccountTransactionReceipt {
    pub fn to_starknet_core(
        self,
        finality_status: starknet_core::types::TransactionFinalityStatus,
    ) -> starknet_core::types::DeployAccountTransactionReceipt {
        starknet_core::types::DeployAccountTransactionReceipt {
            transaction_hash: self.transaction_hash,
            actual_fee: self.actual_fee.into(),
            finality_status,
            messages_sent: self.messages_sent.into_iter().map(starknet_core::types::MsgToL1::from).collect(),
            events: self.events.into_iter().map(starknet_core::types::Event::from).collect(),
            execution_resources: self.execution_resources.into(),
            execution_result: self.execution_result.into(),
            contract_address: self.contract_address,
        }
    }
}

impl From<starknet_core::types::FeePayment> for FeePayment {
    fn from(payment: starknet_core::types::FeePayment) -> Self {
        Self { amount: payment.amount, unit: payment.unit.into() }
    }
}

impl From<FeePayment> for starknet_core::types::FeePayment {
    fn from(fee: FeePayment) -> Self {
        Self { amount: fee.amount, unit: fee.unit.into() }
    }
}

impl From<starknet_core::types::PriceUnit> for PriceUnit {
    fn from(unit: starknet_core::types::PriceUnit) -> Self {
        match unit {
            starknet_core::types::PriceUnit::Fri => PriceUnit::Fri,
            starknet_core::types::PriceUnit::Wei => PriceUnit::Wei,
        }
    }
}

impl From<PriceUnit> for starknet_core::types::PriceUnit {
    fn from(unit: PriceUnit) -> Self {
        match unit {
            PriceUnit::Wei => starknet_core::types::PriceUnit::Wei,
            PriceUnit::Fri => starknet_core::types::PriceUnit::Fri,
        }
    }
}

impl From<starknet_core::types::MsgToL1> for MsgToL1 {
    fn from(msg: starknet_core::types::MsgToL1) -> Self {
        Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
    }
}

impl From<MsgToL1> for starknet_core::types::MsgToL1 {
    fn from(msg: MsgToL1) -> Self {
        Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
    }
}

impl From<starknet_core::types::Event> for Event {
    fn from(event: starknet_core::types::Event) -> Self {
        Self { from_address: event.from_address, keys: event.keys, data: event.data }
    }
}

impl From<Event> for starknet_core::types::Event {
    fn from(event: Event) -> Self {
        Self { from_address: event.from_address, keys: event.keys, data: event.data }
    }
}

impl From<starknet_core::types::ExecutionResources> for ExecutionResources {
    fn from(resources: starknet_core::types::ExecutionResources) -> Self {
        let computation_resources = resources.computation_resources;
        Self {
            steps: computation_resources.steps,
            memory_holes: computation_resources.memory_holes,
            range_check_builtin_applications: computation_resources.range_check_builtin_applications,
            pedersen_builtin_applications: computation_resources.pedersen_builtin_applications,
            poseidon_builtin_applications: computation_resources.poseidon_builtin_applications,
            ec_op_builtin_applications: computation_resources.ec_op_builtin_applications,
            ecdsa_builtin_applications: computation_resources.ecdsa_builtin_applications,
            bitwise_builtin_applications: computation_resources.bitwise_builtin_applications,
            keccak_builtin_applications: computation_resources.keccak_builtin_applications,
            segment_arena_builtin: computation_resources.segment_arena_builtin,
            data_availability: resources.data_resources.data_availability.into(),
            total_gas_consumed: Default::default(),
        }
    }
}

impl From<ExecutionResources> for starknet_core::types::ExecutionResources {
    fn from(resources: ExecutionResources) -> Self {
        Self {
            computation_resources: starknet_core::types::ComputationResources {
                steps: resources.steps,
                memory_holes: resources.memory_holes,
                range_check_builtin_applications: resources.range_check_builtin_applications,
                pedersen_builtin_applications: resources.pedersen_builtin_applications,
                poseidon_builtin_applications: resources.poseidon_builtin_applications,
                ec_op_builtin_applications: resources.ec_op_builtin_applications,
                ecdsa_builtin_applications: resources.ecdsa_builtin_applications,
                bitwise_builtin_applications: resources.bitwise_builtin_applications,
                keccak_builtin_applications: resources.keccak_builtin_applications,
                segment_arena_builtin: resources.segment_arena_builtin,
            },
            data_resources: starknet_core::types::DataResources {
                data_availability: resources.data_availability.into(),
            },
        }
    }
}

impl From<starknet_core::types::DataAvailabilityResources> for L1Gas {
    fn from(resources: starknet_core::types::DataAvailabilityResources) -> Self {
        Self { l1_gas: resources.l1_gas.into(), l1_data_gas: resources.l1_data_gas.into() }
    }
}

impl From<L1Gas> for starknet_core::types::DataAvailabilityResources {
    fn from(resources: L1Gas) -> Self {
        Self { l1_gas: resources.l1_gas as u64, l1_data_gas: resources.l1_data_gas as u64 }
    }
}

impl From<starknet_core::types::ExecutionResult> for ExecutionResult {
    fn from(result: starknet_core::types::ExecutionResult) -> Self {
        match result {
            starknet_core::types::ExecutionResult::Succeeded => ExecutionResult::Succeeded,
            starknet_core::types::ExecutionResult::Reverted { reason } => ExecutionResult::Reverted { reason },
        }
    }
}

impl From<ExecutionResult> for starknet_core::types::ExecutionResult {
    fn from(result: ExecutionResult) -> Self {
        match result {
            ExecutionResult::Succeeded => starknet_core::types::ExecutionResult::Succeeded,
            ExecutionResult::Reverted { reason } => starknet_core::types::ExecutionResult::Reverted { reason },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        tests::dummy_declare_receipt, tests::dummy_deploy_account_receipt, tests::dummy_deploy_receipt,
        tests::dummy_invoke_receipt, tests::dummy_l1_handler_receipt,
    };

    #[test]
    fn test_into_starknet_core_receipt() {
        let receipt: TransactionReceipt = dummy_invoke_receipt().into();
        let core_receipt =
            receipt.clone().to_starknet_core(starknet_core::types::TransactionFinalityStatus::AcceptedOnL1);
        assert_eq!(core_receipt.finality_status(), &starknet_core::types::TransactionFinalityStatus::AcceptedOnL1);
        let receipt_back: TransactionReceipt = core_receipt.into();
        assert_eq!(receipt, receipt_back);

        let receipt: TransactionReceipt = dummy_l1_handler_receipt().into();
        let core_receipt =
            receipt.clone().to_starknet_core(starknet_core::types::TransactionFinalityStatus::AcceptedOnL2);
        assert_eq!(core_receipt.finality_status(), &starknet_core::types::TransactionFinalityStatus::AcceptedOnL2);
        let receipt_back: TransactionReceipt = core_receipt.into();
        assert_eq!(receipt, receipt_back);

        let receipt: TransactionReceipt = dummy_declare_receipt().into();
        let core_receipt =
            receipt.clone().to_starknet_core(starknet_core::types::TransactionFinalityStatus::AcceptedOnL1);
        assert_eq!(core_receipt.finality_status(), &starknet_core::types::TransactionFinalityStatus::AcceptedOnL1);
        let receipt_back: TransactionReceipt = core_receipt.into();
        assert_eq!(receipt, receipt_back);

        let receipt: TransactionReceipt = dummy_deploy_receipt().into();
        let core_receipt =
            receipt.clone().to_starknet_core(starknet_core::types::TransactionFinalityStatus::AcceptedOnL2);
        assert_eq!(core_receipt.finality_status(), &starknet_core::types::TransactionFinalityStatus::AcceptedOnL2);
        let receipt_back: TransactionReceipt = core_receipt.into();
        assert_eq!(receipt, receipt_back);

        let receipt: TransactionReceipt = dummy_deploy_account_receipt().into();
        let core_receipt =
            receipt.clone().to_starknet_core(starknet_core::types::TransactionFinalityStatus::AcceptedOnL1);
        assert_eq!(core_receipt.finality_status(), &starknet_core::types::TransactionFinalityStatus::AcceptedOnL1);
        let receipt_back: TransactionReceipt = core_receipt.into();
        assert_eq!(receipt, receipt_back);
    }
}
