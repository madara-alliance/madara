use blockifier::context::BlockContext;
use blockifier::transaction::objects::TransactionExecutionInfo;
use blockifier::transaction::transaction_execution as btx;
use dc_db::mapping_db::BlockStorageType;
use dp_felt::FeltWrapper;
use jsonrpsee::core::RpcResult;
use starknet_api::core::{calculate_contract_address, ContractAddress};
use starknet_api::transaction::{Transaction, TransactionHash};
use starknet_core::types::{
    ComputationResources, DataAvailabilityResources, DataResources, DeclareTransactionReceipt,
    DeployAccountTransactionReceipt, ExecutionResources, ExecutionResult, FieldElement, Hash256,
    InvokeTransactionReceipt, L1HandlerTransactionReceipt, TransactionFinalityStatus, TransactionReceipt,
    TransactionReceiptWithBlockInfo,
};

use crate::errors::StarknetRpcApiError;
use crate::utils::call_info::{
    blockifier_call_info_to_starknet_resources, extract_events_from_call_info, extract_messages_from_call_info,
};
use crate::utils::execution::{block_context, re_execute_transactions};
use crate::utils::transaction::blockifier_transactions;
use crate::utils::{OptionExt, ResultExt};
use crate::Starknet;

/// Get the transaction receipt by the transaction hash.
///
/// This function retrieves the transaction receipt for a specific transaction identified by its
/// hash. The transaction receipt includes information about the execution status of the
/// transaction, events generated during its execution, and other relevant details.
///
/// ### Arguments
///
/// * `transaction_hash` - The hash of the requested transaction. This parameter specifies the
///   transaction for which the receipt is requested.
///
/// ### Returns
///
/// Returns a transaction receipt, which can be one of two types:
/// - `TransactionReceipt` if the transaction has been processed and has a receipt.
/// - `PendingTransactionReceipt` if the transaction is pending and the receipt is not yet
///   available.
///
/// ### Errors
///
/// The function may return a `TXN_HASH_NOT_FOUND` error if the specified transaction hash is
/// not found.
pub async fn get_transaction_receipt(
    starknet: &Starknet,
    transaction_hash: FieldElement,
) -> RpcResult<TransactionReceiptWithBlockInfo> {
    let (block, tx_info) = starknet
        .block_storage()
        .find_tx_hash_block(&TransactionHash(transaction_hash.into_stark_felt()))
        .or_internal_server_error("Error getting block from tx_hash")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_index = tx_info.tx_index;

    let block_context = block_context(starknet, block.info())?;
    let transaction = block.transactions().get(tx_index).ok_or_else_internal_server_error(|| {
        format!("Failed to retrieve transaction at index {tx_index} index from block_n {}", block.block_n())
    })?;

    // deploy transaction was not supported by blockifier
    if let Transaction::Deploy(_) = transaction {
        log::error!("re-executing a deploy transaction is not supported");
        return Err(StarknetRpcApiError::UnimplementedMethod.into());
    }

    // create a vector of tuples with the transaction and its hash, up to the current transaction index
    let transaction_with_hash = block
        .transactions()
        .iter()
        .cloned()
        .zip(block.tx_hashes().iter().map(FeltWrapper::into_field_element))
        .take(tx_index + 1)
        .collect();

    let transactions_blockifier = blockifier_transactions(starknet, transaction_with_hash)?;

    let execution_infos = execution_infos(starknet, transactions_blockifier, &block_context)?;

    let receipt = receipt(starknet, transaction, &execution_infos, transaction_hash, &tx_info.storage_type)?;

    let block = match tx_info.storage_type {
        BlockStorageType::Pending => starknet_core::types::ReceiptBlock::Pending,
        BlockStorageType::BlockN(block_number) => {
            let block_hash = block.block_hash().into_field_element();
            starknet_core::types::ReceiptBlock::Block { block_hash, block_number }
        }
    };

    Ok(TransactionReceiptWithBlockInfo { receipt, block })
}

pub(crate) fn execution_infos(
    starknet: &Starknet,
    transactions: Vec<btx::Transaction>,
    block_context: &BlockContext,
) -> RpcResult<TransactionExecutionInfo> {
    // TODO: fix this with vec
    // let (last, prev) = match transactions.split_last() {
    //     Some((last, prev)) => (vec![last.clone()], prev.to_vec()),
    //     None => (transactions, vec![]),
    // };

    let last = transactions;
    let prev = vec![];

    let execution_infos = re_execute_transactions(starknet, prev, last, block_context)
        .map_err(|e| {
            log::error!("Failed to re-execute transactions: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .pop()
        .ok_or_else(|| {
            log::error!("No execution info returned for the last transaction");
            StarknetRpcApiError::InternalServerError
        })?;

    Ok(execution_infos)
}

pub fn receipt(
    starknet: &Starknet,
    transaction: &Transaction,
    execution_infos: &TransactionExecutionInfo,
    transaction_hash: FieldElement,
    is_pending: &BlockStorageType,
) -> RpcResult<TransactionReceipt> {
    let message_hash: Hash256 = Hash256::from_felt(&FieldElement::default());

    let actual_fee = starknet_core::types::FeePayment {
        amount: execution_infos.actual_fee.0.into(),
        unit: starknet_core::types::PriceUnit::Wei,
    };

    let finality_status = match is_pending {
        BlockStorageType::Pending => TransactionFinalityStatus::AcceptedOnL2,
        BlockStorageType::BlockN(block_n) if *block_n <= starknet.get_l1_last_confirmed_block()? => {
            TransactionFinalityStatus::AcceptedOnL1
        }
        BlockStorageType::BlockN(_) => TransactionFinalityStatus::AcceptedOnL2,
    };

    let execution_result = match execution_infos.revert_error.clone() {
        Some(err) => ExecutionResult::Reverted { reason: err },
        None => ExecutionResult::Succeeded,
    };

    // no execution resources for declare transactions
    let execution_resources = match execution_infos.execute_call_info {
        Some(ref call_info) => blockifier_call_info_to_starknet_resources(call_info),
        None => ExecutionResources {
            computation_resources: ComputationResources {
                steps: 0,
                memory_holes: None,
                range_check_builtin_applications: None,
                pedersen_builtin_applications: None,
                poseidon_builtin_applications: None,
                ec_op_builtin_applications: None,
                ecdsa_builtin_applications: None,
                bitwise_builtin_applications: None,
                keccak_builtin_applications: None,
                segment_arena_builtin: None,
            },
            data_resources: DataResources {
                data_availability: DataAvailabilityResources { l1_gas: 0, l1_data_gas: 0 },
            },
        },
    };

    // no events or messages sent for declare transactions
    let (events, messages_sent) = match execution_infos.execute_call_info.as_ref() {
        None => (vec![], vec![]),
        Some(call_info) => (extract_events_from_call_info(call_info), extract_messages_from_call_info(call_info)),
    };

    let receipt = match transaction {
        Transaction::Declare(_) => TransactionReceipt::Declare(DeclareTransactionReceipt {
            transaction_hash,
            actual_fee,
            finality_status,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        Transaction::DeployAccount(deploy_account) => {
            let contract_address = calculate_contract_address(
                deploy_account.contract_address_salt(),
                deploy_account.class_hash(),
                &deploy_account.constructor_calldata(),
                ContractAddress::default(),
            )
            .map_err(|e| {
                log::error!("Failed to calculate contract address: {e}");
                StarknetRpcApiError::InternalServerError
            })?;
            TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
                transaction_hash,
                actual_fee,
                finality_status,
                messages_sent,
                events,
                execution_resources,
                execution_result,
                // Safe to unwrap because StarkFelt is same as FieldElement
                contract_address: (*contract_address.key()).into(),
            })
        }
        Transaction::Invoke(_) => TransactionReceipt::Invoke(InvokeTransactionReceipt {
            transaction_hash,
            actual_fee,
            finality_status,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        Transaction::L1Handler(_) => TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
            message_hash,
            transaction_hash,
            actual_fee,
            finality_status,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        _ => unreachable!("Deploy transactions are not supported"),
    };

    Ok(receipt)
}
