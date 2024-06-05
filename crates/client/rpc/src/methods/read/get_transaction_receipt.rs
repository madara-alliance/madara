use blockifier::context::BlockContext;
use blockifier::transaction::objects::TransactionExecutionInfo;
use blockifier::transaction::transaction_execution as btx;
use jsonrpsee::core::RpcResult;
use mc_db::DeoxysBackend;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_types::block::{DBlockT, DHashT};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_blockchain::HeaderBackend;
use starknet_api::core::{calculate_contract_address, ContractAddress};
use starknet_api::transaction::Transaction;
use starknet_core::types::{
    ComputationResources, DataAvailabilityResources, DataResources, DeclareTransactionReceipt,
    DeployAccountTransactionReceipt, ExecutionResources, ExecutionResult, FieldElement, Hash256,
    InvokeTransactionReceipt, L1HandlerTransactionReceipt, TransactionFinalityStatus, TransactionReceipt,
    TransactionReceiptWithBlockInfo,
};

use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::errors::StarknetRpcApiError;
use crate::utils::call_info::{
    blockifier_call_info_to_starknet_resources, extract_events_from_call_info, extract_messages_from_call_info,
};
use crate::utils::execution::{block_context, re_execute_transactions};
use crate::utils::helpers::{block_hash_from_block_n, tx_hash_retrieve, txs_hashes_from_block_hash};
use crate::utils::transaction::blockifier_transactions;
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
pub async fn get_transaction_receipt<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    transaction_hash: FieldElement,
) -> RpcResult<TransactionReceiptWithBlockInfo>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    // get the substrate block hash from the transaction hash
    let substrate_block_hash = DeoxysBackend::mapping()
        .substrate_block_hash_from_transaction_hash(Felt252Wrapper::from(transaction_hash).into())
        .map_err(|e| {
            log::error!("Failed to get substrate block hash from transaction hash: {}", e);
            StarknetRpcApiError::InternalServerError
        })?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    get_transaction_receipt_finalized(starknet, substrate_block_hash, transaction_hash)
}

pub fn get_transaction_receipt_finalized<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    substrate_block_hash: DHashT,
    transaction_hash: FieldElement,
) -> RpcResult<TransactionReceiptWithBlockInfo>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
    let block_header = block.header();
    let block_number = block_header.block_number;
    let starknet_block_hash = block_hash_from_block_n(block_number)?;

    let block_context = block_context(starknet.client.as_ref(), substrate_block_hash)?;

    let block_txs_hashes = tx_hash_retrieve(txs_hashes_from_block_hash(starknet_block_hash)?);

    // retrieve the transaction index in the block with the transaction hash
    let (tx_index, _) =
        block_txs_hashes.iter().enumerate().find(|(_, hash)| *hash == &transaction_hash).ok_or_else(|| {
            log::error!("Failed to retrieve transaction index from block with hash {starknet_block_hash:?}");
            StarknetRpcApiError::InternalServerError
        })?;

    let transaction = block.transactions().get(tx_index).ok_or_else(|| {
        log::error!("Failed to retrieve transaction at index {tx_index} from block with hash {starknet_block_hash:?}");
        StarknetRpcApiError::InternalServerError
    })?;

    // deploy transaction was not supported by blockifier
    if let Transaction::Deploy(_) = transaction {
        log::error!("re-executing a deploy transaction is not supported");
        return Err(StarknetRpcApiError::UnimplementedMethod.into());
    }

    // create a vector of tuples with the transaction and its hash, up to the current transaction index
    let transaction_with_hash =
        block.transactions().iter().cloned().zip(block_txs_hashes.iter().cloned()).take(tx_index + 1).collect();

    let transactions_blockifier = blockifier_transactions(transaction_with_hash)?;

    let execution_infos = execution_infos(transactions_blockifier, &block_context)?;

    let receipt = receipt(transaction, &execution_infos, transaction_hash, block_number)?;

    let block_info = starknet_core::types::ReceiptBlock::Block { block_hash: starknet_block_hash, block_number };

    Ok(TransactionReceiptWithBlockInfo { receipt, block: block_info })
}

pub(crate) fn execution_infos(
    transactions: Vec<btx::Transaction>,
    block_context: &BlockContext,
) -> RpcResult<TransactionExecutionInfo> {
    let (last, prev) = match transactions.split_last() {
        Some((last, prev)) => (vec![last.clone()], prev.to_vec()),
        None => (transactions, vec![]),
    };

    let execution_infos = re_execute_transactions(prev, last, block_context)
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
    transaction: &Transaction,
    execution_infos: &TransactionExecutionInfo,
    transaction_hash: FieldElement,
    block_number: u64,
) -> RpcResult<TransactionReceipt> {
    let message_hash: Hash256 = Hash256::from_felt(&FieldElement::default());

    let actual_fee = starknet_core::types::FeePayment {
        amount: execution_infos.actual_fee.0.into(),
        unit: starknet_core::types::PriceUnit::Wei,
    };

    let finality_status = if block_number <= mc_sync::l1::ETHEREUM_STATE_UPDATE.read().unwrap().block_number {
        TransactionFinalityStatus::AcceptedOnL1
    } else {
        TransactionFinalityStatus::AcceptedOnL2
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
                contract_address: FieldElement::from_bytes_be(&contract_address.0 .0 .0).unwrap(),
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
