use crate::{
    DataAvailabilityResources, DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt,
    Event, ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, L1HandlerTransactionReceipt,
    MsgToL1, PriceUnit, TransactionReceipt,
};

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

impl From<FeePayment> for starknet_core::types::FeePayment {
    fn from(fee: FeePayment) -> Self {
        Self { amount: fee.amount, unit: fee.unit.into() }
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

impl From<MsgToL1> for starknet_core::types::MsgToL1 {
    fn from(msg: MsgToL1) -> Self {
        Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
    }
}

impl From<Event> for starknet_core::types::Event {
    fn from(event: Event) -> Self {
        Self { from_address: event.from_address, keys: event.keys, data: event.data }
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

impl From<DataAvailabilityResources> for starknet_core::types::DataAvailabilityResources {
    fn from(resources: DataAvailabilityResources) -> Self {
        Self { l1_gas: resources.l1_gas, l1_data_gas: resources.l1_data_gas }
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
