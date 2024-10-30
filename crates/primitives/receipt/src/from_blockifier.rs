use blockifier::execution::call_info::CallInfo;
use blockifier::transaction::{
    account_transaction::AccountTransaction,
    objects::{FeeType, GasVector, HasRelatedFeeType, TransactionExecutionInfo},
    transaction_execution::Transaction,
    transactions::L1HandlerTransaction,
};
use cairo_vm::types::builtin_name::BuiltinName;
use starknet_core::types::MsgToL2;
use starknet_types_core::felt::Felt;
use thiserror::Error;

use crate::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, Event, ExecutionResources, ExecutionResult, FeePayment,
    InvokeTransactionReceipt, L1Gas, L1HandlerTransactionReceipt, MsgToL1, PriceUnit, TransactionReceipt,
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
            AccountTransaction::Declare(tx) => tx.tx_hash.0,
            AccountTransaction::DeployAccount(tx) => tx.tx_hash.0,
            AccountTransaction::Invoke(tx) => tx.tx_hash.0,
        },
        Transaction::L1HandlerTransaction(tx) => tx.tx_hash.0,
    }
}

#[derive(Debug, Error)]
pub enum L1HandlerMessageError {
    #[error("Empty calldata")]
    EmptyCalldata,
    #[error("From address out of range")]
    FromAddressOutOfRange,
    #[error("Invalid nonce")]
    InvalidNonce,
}

fn get_l1_handler_message_hash(tx: &L1HandlerTransaction) -> Result<Felt, L1HandlerMessageError> {
    let (from_address, payload) = tx.tx.calldata.0.split_first().ok_or(L1HandlerMessageError::EmptyCalldata)?;

    let from_address = (*from_address).try_into().map_err(|_| L1HandlerMessageError::FromAddressOutOfRange)?;

    let nonce = tx.tx.nonce.0.to_bigint().try_into().map_err(|_| L1HandlerMessageError::InvalidNonce)?;

    let message = MsgToL2 {
        from_address,
        to_address: tx.tx.contract_address.into(),
        selector: tx.tx.entry_point_selector.0,
        payload: payload.into(),
        nonce,
    };
    Ok(Felt::from_bytes_le(message.hash().as_bytes()))
}

pub fn from_blockifier_execution_info(res: &TransactionExecutionInfo, tx: &Transaction) -> TransactionReceipt {
    let price_unit = match blockifier_tx_fee_type(tx) {
        FeeType::Eth => PriceUnit::Wei,
        FeeType::Strk => PriceUnit::Fri,
    };

    let actual_fee = FeePayment { amount: res.transaction_receipt.fee.into(), unit: price_unit };
    let transaction_hash = blockifier_tx_hash(tx);

    let message_hash = match tx {
        Transaction::L1HandlerTransaction(tx) => match get_l1_handler_message_hash(tx) {
            Ok(hash) => Some(hash),
            Err(err) => {
                log::error!("Error getting l1 handler message hash: {:?}", err);
                None
            }
        },
        _ => None,
    };

    fn recursive_call_info_iter(res: &TransactionExecutionInfo) -> impl Iterator<Item = &CallInfo> {
        res
            .non_optional_call_infos() // all root callinfos
            .flat_map(|call_info| call_info.iter()) // flatmap over the roots' recursive inner call infos
    }

    let messages_sent = recursive_call_info_iter(res)
        .flat_map(|call| {
            call.execution.l2_to_l1_messages.iter().map(|message| MsgToL1 {
                from_address: call.call.storage_address.into(),
                to_address: message.message.to_address.into(),
                payload: message.message.payload.0.clone(),
            })
        })
        .collect();
    let events = recursive_call_info_iter(res)
        .flat_map(|call| {
            call.execution.events.iter().map(|event| Event {
                from_address: call.call.storage_address.into(),
                keys: event.event.keys.iter().map(|k| k.0).collect(),
                data: event.event.data.0.clone(),
            })
        })
        .collect();

    // Note: these should not be iterated over recursively because they include the inner calls

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
                contract_address: tx.contract_address.into(),
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
        Transaction::L1HandlerTransaction(_tx) => TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
            transaction_hash,
            actual_fee,
            messages_sent,
            events,
            execution_resources,
            execution_result,
            message_hash: message_hash.unwrap(), // it's a safe unwrap because it would've panicked earlier if it was Err
        }),
    }
}

impl From<GasVector> for L1Gas {
    fn from(value: GasVector) -> Self {
        L1Gas { l1_gas: value.l1_gas as _, l1_data_gas: value.l1_data_gas as _ }
    }
}
