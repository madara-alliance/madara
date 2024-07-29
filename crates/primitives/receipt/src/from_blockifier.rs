use blockifier::transaction::{objects::{ExecutionResourcesTraits, TransactionExecutionInfo}, transaction_types::TransactionType};
use cairo_vm::types::builtin_name::BuiltinName;
use starknet_core::types::{FeePayment, Felt};

use crate::{DeclareTransactionReceipt, DeployAccountTransactionReceipt, Event, ExecutionResources, ExecutionResult, InvokeTransactionReceipt, L1HandlerTransactionReceipt, MsgToL1, TransactionReceipt};

pub fn from_blockifier_execution_info(res: &TransactionExecutionInfo, tx_ty: TransactionType, transaction_hash: Felt) -> TransactionReceipt {
    log::info!("{res:?} -- {tx_ty:?} -- {transaction_hash:#x}");
    let actual_fee = todo!();
    let events = res.non_optional_call_infos().flat_map(|call| call.execution.events.iter()).map(|event| {
        Event {
            from_address: todo!(),
            keys: event.event.keys.into_iter().map(|k| k.0).collect(),
            data: event.event.data.0,
        }
    }).collect();
    let messages_sent = res.non_optional_call_infos().flat_map(|call| call.execution.l2_to_l1_messages.iter()).map(|message| {
        MsgToL1 { from_address: todo!(), to_address: message.message.to_address.into(), payload: message.message.payload.0 }
    }).collect();

    let get_applications = |resource| {
        res.non_optional_call_infos().map(|call| call.resources.builtin_instance_counter.get(resource).map(|el| *el as u64)).sum()
    };

    let memory_holes = res.non_optional_call_infos().map(|call| call.resources.n_memory_holes as u64).sum();

    let execution_resources = ExecutionResources {
        steps: res.non_optional_call_infos().map(|call| call.resources.n_steps as u64).sum(),
        memory_holes: if memory_holes == 0 { None } else { Some(memory_holes) }, // unsure??
        range_check_builtin_applications: get_applications(&BuiltinName::range_check),
        pedersen_builtin_applications: get_applications(&BuiltinName::pedersen),
        poseidon_builtin_applications: get_applications(&BuiltinName::poseidon),
        ec_op_builtin_applications: get_applications(&BuiltinName::ec_op),
        ecdsa_builtin_applications: get_applications(&BuiltinName::ecdsa),
        bitwise_builtin_applications: get_applications(&BuiltinName::bitwise),
        keccak_builtin_applications: get_applications(&BuiltinName::keccak),
        segment_arena_builtin: get_applications(&BuiltinName::segment_arena),
        data_availability: todo!(),
        total_gas_consumed: todo!(),
    };

    let execution_result = if let Some(reason) = res.revert_error {
        ExecutionResult::Reverted { reason }
    } else {
        ExecutionResult::Succeeded
    };


    
    match tx_ty {
        TransactionType::Declare => TransactionReceipt::Declare(DeclareTransactionReceipt {
            transaction_hash,
            actual_fee,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        TransactionType::DeployAccount => TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
            transaction_hash,
            actual_fee,
            messages_sent,
            events,
            execution_resources,
            execution_result,
            contract_address: todo!(),
        }),
        TransactionType::InvokeFunction => TransactionReceipt::Invoke(InvokeTransactionReceipt {
            transaction_hash,
            actual_fee,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        TransactionType::L1Handler => TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
            message_hash: todo!(),
            transaction_hash,
            actual_fee,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
    }
}
