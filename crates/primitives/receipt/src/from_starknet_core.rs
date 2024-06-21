use crate::{
    DataAvailabilityResources, DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt,
    Event, ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, L1HandlerTransactionReceipt,
    MsgToL1, PriceUnit, TransactionReceipt,
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

impl From<starknet_core::types::FeePayment> for FeePayment {
    fn from(payment: starknet_core::types::FeePayment) -> Self {
        Self { amount: payment.amount, unit: payment.unit.into() }
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

impl From<starknet_core::types::MsgToL1> for MsgToL1 {
    fn from(msg: starknet_core::types::MsgToL1) -> Self {
        Self { from_address: msg.from_address, to_address: msg.to_address, payload: msg.payload }
    }
}

impl From<starknet_core::types::Event> for Event {
    fn from(event: starknet_core::types::Event) -> Self {
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
        }
    }
}

impl From<starknet_core::types::DataAvailabilityResources> for DataAvailabilityResources {
    fn from(resources: starknet_core::types::DataAvailabilityResources) -> Self {
        Self { l1_gas: resources.l1_gas, l1_data_gas: resources.l1_data_gas }
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
