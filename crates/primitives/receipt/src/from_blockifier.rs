use blockifier::transaction::{
    account_transaction::AccountTransaction,
    objects::{FeeType, GasVector, HasRelatedFeeType, TransactionExecutionInfo},
    transaction_execution::Transaction,
};
use cairo_vm::types::builtin_name::BuiltinName;
use dp_convert::ToFelt;
use starknet_types_core::felt::Felt;

use crate::{
    DataAvailabilityResources, DeclareTransactionReceipt, DeployAccountTransactionReceipt, Event, ExecutionResources,
    ExecutionResult, FeePayment, InvokeTransactionReceipt, MsgToL1, PriceUnit, TransactionReceipt,
};

fn blockifier_tx_fee_type(tx: &Transaction) -> FeeType {
    match tx {
        Transaction::AccountTransaction(tx) => tx.fee_type(),
        Transaction::L1HandlerTransaction(tx) => tx.fee_type(),
    }
}
fn blockifier_tx_hash(tx: &Transaction) -> Felt {
    match tx {
        Transaction::AccountTransaction(tx) => match tx {
            AccountTransaction::Declare(tx) => tx.tx_hash.to_felt(),
            AccountTransaction::DeployAccount(tx) => tx.tx_hash.to_felt(),
            AccountTransaction::Invoke(tx) => tx.tx_hash.to_felt(),
        },
        Transaction::L1HandlerTransaction(tx) => tx.tx_hash.to_felt(),
    }
}

pub fn from_blockifier_execution_info(res: &TransactionExecutionInfo, tx: &Transaction) -> TransactionReceipt {
    let price_unit = match blockifier_tx_fee_type(tx) {
        FeeType::Eth => PriceUnit::Wei,
        FeeType::Strk => PriceUnit::Fri,
    };

    let actual_fee = FeePayment { amount: res.transaction_receipt.fee.into(), unit: price_unit };
    let transaction_hash = blockifier_tx_hash(tx);

    // `non_optional_call_infos` recursively returns the call infos for all calls in this tx.

    let events = res
        .non_optional_call_infos()
        .flat_map(|call| {
            call.execution.events.iter().map(|event| Event {
                from_address: call.call.storage_address.to_felt(),
                keys: event.event.keys.iter().map(|k| k.0).collect(),
                data: event.event.data.0.clone(),
            })
        })
        .collect();
    let messages_sent = res
        .non_optional_call_infos()
        .flat_map(|call| {
            call.execution.l2_to_l1_messages.iter().map(|message| MsgToL1 {
                from_address: call.call.storage_address.to_felt(),
                to_address: message.message.to_address.into(),
                payload: message.message.payload.0.clone(),
            })
        })
        .collect();

    let get_applications = |resource| {
        res.non_optional_call_infos()
            .map(|call| call.resources.builtin_instance_counter.get(resource).map(|el| *el as u64))
            .sum()
    };

    let memory_holes = res.non_optional_call_infos().map(|call| call.resources.n_memory_holes as u64).sum();

    let execution_resources = ExecutionResources {
        steps: res.non_optional_call_infos().map(|call| call.resources.n_steps as u64).sum(),
        memory_holes: if memory_holes == 0 { None } else { Some(memory_holes) },
        range_check_builtin_applications: get_applications(&BuiltinName::range_check),
        pedersen_builtin_applications: get_applications(&BuiltinName::pedersen),
        poseidon_builtin_applications: get_applications(&BuiltinName::poseidon),
        ec_op_builtin_applications: get_applications(&BuiltinName::ec_op),
        ecdsa_builtin_applications: get_applications(&BuiltinName::ecdsa),
        bitwise_builtin_applications: get_applications(&BuiltinName::bitwise),
        keccak_builtin_applications: get_applications(&BuiltinName::keccak),
        segment_arena_builtin: get_applications(&BuiltinName::segment_arena),
        data_availability: res.transaction_receipt.da_gas.into(),
        total_gas_consumed: res.transaction_receipt.gas.into(),
    };

    let execution_result = if let Some(reason) = &res.revert_error {
        ExecutionResult::Reverted { reason: reason.into() }
    } else {
        ExecutionResult::Succeeded
    };

    match tx {
        Transaction::AccountTransaction(AccountTransaction::Declare(_)) => {
            TransactionReceipt::Declare(DeclareTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
            })
        }
        Transaction::AccountTransaction(AccountTransaction::DeployAccount(tx)) => {
            TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
                contract_address: tx.contract_address.to_felt(),
            })
        }
        Transaction::AccountTransaction(AccountTransaction::Invoke(_)) => {
            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
            })
        }
        Transaction::L1HandlerTransaction(_tx) => unimplemented!("l1 handler tx"),
        // Transaction::L1HandlerTransactionv(tx) => TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
        //     transaction_hash,
        //     actual_fee,
        //     messages_sent,
        //     events,
        //     execution_resources,
        //     execution_result,
        //     message_hash: todo!(),
        // }),
    }
}

impl From<GasVector> for DataAvailabilityResources {
    fn from(value: GasVector) -> Self {
        DataAvailabilityResources { l1_gas: value.l1_gas as _, l1_data_gas: value.l1_data_gas as _ }
    }
}
