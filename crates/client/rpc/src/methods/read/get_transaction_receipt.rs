use blockifier::execution::contract_class::{ContractClass as ContractClassBf, ContractClassV1 as ContractClassV1Bf};
use blockifier::transaction::objects::TransactionExecutionInfo;
use jsonrpsee::core::error::Error;
use jsonrpsee::core::RpcResult;
use log::error;
use mc_db::DeoxysBackend;
use mc_genesis_data_provider::GenesisProvider;
use mc_sync::l2::get_pending_block;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::{DeclareTransaction, Transaction as TransactionMp, UserOrL1HandlerTransaction, UserTransaction};
use mp_types::block::{DBlockT, DHashT};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::api_core::ClassHash;
use starknet_core::types::{
    BlockId, BlockTag, DeclareTransactionReceipt, DeployAccountTransactionReceipt, ExecutionResources, ExecutionResult,
    FieldElement, Hash256, InvokeTransactionReceipt, L1HandlerTransactionReceipt, MaybePendingTransactionReceipt,
    PendingDeclareTransactionReceipt, PendingDeployAccountTransactionReceipt, PendingInvokeTransactionReceipt,
    PendingL1HandlerTransactionReceipt, PendingTransactionReceipt, TransactionFinalityStatus, TransactionReceipt,
};

use crate::errors::StarknetRpcApiError;
use crate::utils::{
    blockifier_call_info_to_starknet_resources, extract_events_from_call_info, extract_messages_from_call_info,
    get_block_by_block_hash, tx_hash_compute, tx_hash_retrieve,
};
use crate::{Felt, Starknet, StarknetReadRpcApiServer};

fn get_transaction_receipt_finalized<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    chain_id: Felt,
    substrate_block_hash: DHashT,
    transaction_hash: FieldElement,
) -> RpcResult<MaybePendingTransactionReceipt>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let block = get_block_by_block_hash(client.client.as_ref(), substrate_block_hash)?;
    let block_header = block.header();
    let block_number = block_header.block_number;
    let block_hash: Felt252Wrapper = block_header.hash::<H>();

    // computes the previous SUBSTRATE block hash
    let previous_block_hash = previous_block_hash(client, block_number)?;

    let block_txs_hashes = if let Some(tx_hashes) = client.get_cached_transaction_hashes(block_hash.into()) {
        tx_hash_retrieve(tx_hashes)
    } else {
        // WHY IN SAINT FUCK IS mc
        tx_hash_compute::<H>(&block, chain_id)
    };

    let (tx_index, _) = block_txs_hashes.into_iter().enumerate().find(|(_, hash)| hash == &transaction_hash).unwrap();

    let transaction = block.transactions().get(tx_index).ok_or_else(|| {
        log::error!("Failed to retrieve transaction at index {tx_index} from block with hash {block_hash:?}");
        StarknetRpcApiError::InternalServerError
    })?;

    // TODO: remove this line when deploy is supported
    if let TransactionMp::Deploy(_) = transaction {
        log::error!("re executing a deploy transaction is not supported yet");
        return Err(StarknetRpcApiError::UnimplementedMethod.into());
    }

    let transactions = transactions(client, substrate_block_hash, chain_id, &block, block_number, tx_index)?;

    let execution_infos = execution_infos(client, previous_block_hash, transactions)?;

    // TODO(#1291): compute message hash correctly to L1HandlerTransactionReceipt
    let message_hash: Hash256 = Hash256::from_felt(&FieldElement::default());

    let actual_fee = execution_infos.actual_fee.0.into();

    let finality_status = if block_number <= mc_sync::l1::ETHEREUM_STATE_UPDATE.read().unwrap().block_number {
        TransactionFinalityStatus::AcceptedOnL1
    } else {
        TransactionFinalityStatus::AcceptedOnL2
    };

    let execution_result = match execution_infos.revert_error.clone() {
        Some(err) => ExecutionResult::Reverted { reason: err },
        None => ExecutionResult::Succeeded,
    };

    let execution_resources = match execution_infos.execute_call_info {
        Some(ref call_info) => blockifier_call_info_to_starknet_resources(call_info),
        None => ExecutionResources {
            steps: 0,
            memory_holes: None,
            range_check_builtin_applications: 0,
            pedersen_builtin_applications: 0,
            poseidon_builtin_applications: 0,
            ec_op_builtin_applications: 0,
            ecdsa_builtin_applications: 0,
            bitwise_builtin_applications: 0,
            keccak_builtin_applications: 0,
        },
    };

    let events = match execution_infos.execute_call_info {
        Some(ref call_info) => extract_events_from_call_info(call_info),
        None => vec![],
    };

    let messages_sent = match execution_infos.execute_call_info {
        Some(ref call_info) => extract_messages_from_call_info(call_info),
        None => vec![],
    };

    // TODO: use actual execution ressources
    let receipt = match transaction {
        mp_transactions::Transaction::Declare(_) => TransactionReceipt::Declare(DeclareTransactionReceipt {
            transaction_hash,
            actual_fee,
            finality_status,
            block_hash: block_hash.into(),
            block_number,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        mp_transactions::Transaction::DeployAccount(tx) => {
            TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
                transaction_hash,
                actual_fee,
                finality_status,
                block_hash: block_hash.into(),
                block_number,
                messages_sent,
                events,
                execution_resources,
                execution_result,
                contract_address: tx.get_account_address(),
            })
        }
        mp_transactions::Transaction::Invoke(_) => TransactionReceipt::Invoke(InvokeTransactionReceipt {
            transaction_hash,
            actual_fee,
            finality_status,
            block_hash: block_hash.into(),
            block_number,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        mp_transactions::Transaction::L1Handler(_) => TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
            message_hash,
            transaction_hash,
            actual_fee,
            finality_status,
            block_hash: block_hash.into(),
            block_number,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        _ => unreachable!("Deploy transactions are not supported"),
    };

    Ok(MaybePendingTransactionReceipt::Receipt(receipt))
}

fn get_transaction_receipt_pending<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    chain_id: Felt,
    substrate_block_hash: DHashT,
    transaction_hash: FieldElement,
) -> RpcResult<MaybePendingTransactionReceipt>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let block = get_pending_block()
        .ok_or(Error::Custom("Failed to retrieve pending block, node not yet synchronized".to_string()))?;
    let block_header = block.header();
    let block_number = block_header.block_number;
    let block_hash: Felt252Wrapper = block_header.hash::<H>();

    // computes the previous SUBSTRATE block hash
    let previous_block_hash = previous_block_hash(client, block_number)?;

    let block_txs_hashes = tx_hash_compute::<H>(&block, chain_id);

    let (tx_index, _) = block_txs_hashes.into_iter().enumerate().find(|(_, hash)| hash == &transaction_hash).unwrap();

    let transaction = block.transactions().get(tx_index).ok_or_else(|| {
        log::error!("Failed to retrieve transaction at index {tx_index} from block with hash {block_hash:?}");
        StarknetRpcApiError::InternalServerError
    })?;

    // TODO: remove this line when deploy is supported
    if let TransactionMp::Deploy(_) = transaction {
        log::error!("re executing a deploy transaction is not supported yet");
        return Err(StarknetRpcApiError::UnimplementedMethod.into());
    }

    let transactions = transactions(client, substrate_block_hash, chain_id, &block, block_number, tx_index)?;

    let execution_infos = execution_infos(client, previous_block_hash, transactions)?;

    // TODO(#1291): compute message hash correctly to L1HandlerTransactionReceipt
    let message_hash: Hash256 = Hash256::from_felt(&FieldElement::default());

    let actual_fee = execution_infos.actual_fee.0.into();

    let execution_result = match execution_infos.revert_error.clone() {
        Some(err) => ExecutionResult::Reverted { reason: err },
        None => ExecutionResult::Succeeded,
    };

    let execution_resources = match execution_infos.execute_call_info {
        Some(ref call_info) => blockifier_call_info_to_starknet_resources(call_info),
        None => ExecutionResources {
            steps: 0,
            memory_holes: None,
            range_check_builtin_applications: 0,
            pedersen_builtin_applications: 0,
            poseidon_builtin_applications: 0,
            ec_op_builtin_applications: 0,
            ecdsa_builtin_applications: 0,
            bitwise_builtin_applications: 0,
            keccak_builtin_applications: 0,
        },
    };

    let events = match execution_infos.execute_call_info {
        Some(ref call_info) => extract_events_from_call_info(call_info),
        None => vec![],
    };

    let messages_sent = match execution_infos.execute_call_info {
        Some(ref call_info) => extract_messages_from_call_info(call_info),
        None => vec![],
    };

    // TODO: use actual execution ressources
    let receipt = match transaction {
        mp_transactions::Transaction::Declare(_) => {
            PendingTransactionReceipt::Declare(PendingDeclareTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
            })
        }
        mp_transactions::Transaction::DeployAccount(tx) => {
            PendingTransactionReceipt::DeployAccount(PendingDeployAccountTransactionReceipt {
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
                contract_address: tx.get_account_address(),
            })
        }
        mp_transactions::Transaction::Invoke(_) => PendingTransactionReceipt::Invoke(PendingInvokeTransactionReceipt {
            transaction_hash,
            actual_fee,
            messages_sent,
            events,
            execution_resources,
            execution_result,
        }),
        mp_transactions::Transaction::L1Handler(_) => {
            PendingTransactionReceipt::L1Handler(PendingL1HandlerTransactionReceipt {
                message_hash,
                transaction_hash,
                actual_fee,
                messages_sent,
                events,
                execution_resources,
                execution_result,
            })
        }
        _ => unreachable!("Pending deploy transactions are not supported"),
    };

    Ok(MaybePendingTransactionReceipt::PendingReceipt(receipt))
}

fn previous_block_hash<A, BE, G, C, P, H>(client: &Starknet<A, BE, G, C, P, H>, block_number: u64) -> RpcResult<DHashT>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let previous_block_hash =
        client.substrate_block_hash_from_starknet_block(BlockId::Number(block_number - 1)).map_err(|e| {
            log::error!("Failed to retrieve previous substrate block hash: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

    Ok(previous_block_hash)
}

fn transactions<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
    chain_id: Felt,
    block: &DeoxysBlock,
    block_number: u64,
    tx_index: usize,
) -> RpcResult<Vec<UserOrL1HandlerTransaction>>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let transactions = block
            .transactions()
            .iter()
            .take(tx_index + 1)
            .filter(|tx| !matches!(tx, TransactionMp::Deploy(_))) // TODO: remove this line when deploy is supported
            .map(|tx| match tx {
                TransactionMp::Invoke(invoke_tx) => {
					tx_invoke_transaction(invoke_tx.clone())
                }
                TransactionMp::DeployAccount(deploy_account_tx) => {
					tx_deploy_account(deploy_account_tx.clone())
                }
                TransactionMp::Declare(declare_tx) => {
					tx_declare(client, substrate_block_hash, declare_tx.clone())
                }
                TransactionMp::L1Handler(l1_handler) => {
					tx_l1_handler::<H>(chain_id, block_number, l1_handler.clone())
                }
                TransactionMp::Deploy(_) => todo!(),
            })
            .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

fn tx_invoke_transaction(tx: mp_transactions::InvokeTransaction) -> RpcResult<UserOrL1HandlerTransaction> {
    Ok(UserOrL1HandlerTransaction::User(UserTransaction::Invoke(tx)))
}

fn tx_deploy_account(tx: mp_transactions::DeployAccountTransaction) -> RpcResult<UserOrL1HandlerTransaction> {
    Ok(UserOrL1HandlerTransaction::User(UserTransaction::DeployAccount(tx)))
}

fn tx_declare<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
    declare_tx: mp_transactions::DeclareTransaction,
) -> RpcResult<UserOrL1HandlerTransaction>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let class_hash = ClassHash::from(*declare_tx.class_hash());

    match declare_tx {
        DeclareTransaction::V0(_) | DeclareTransaction::V1(_) => {
            tx_declare_v0v1(client, substrate_block_hash, declare_tx, class_hash)
        }
        DeclareTransaction::V2(_) => tx_declare_v2(declare_tx, class_hash),
    }
}

fn tx_declare_v0v1<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
    declare_tx: mp_transactions::DeclareTransaction,
    class_hash: ClassHash,
) -> RpcResult<UserOrL1HandlerTransaction>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let contract_class = client
        .overrides
        .for_block_hash(client.client.as_ref(), substrate_block_hash)
        .contract_class_by_class_hash(substrate_block_hash, class_hash)
        .ok_or_else(|| {
            log::error!("Failed to retrieve contract class from hash '{class_hash}'");
            StarknetRpcApiError::InternalServerError
        })?;

    Ok(UserOrL1HandlerTransaction::User(UserTransaction::Declare(declare_tx, contract_class)))
}

fn tx_declare_v2(
    declare_tx: mp_transactions::DeclareTransaction,
    class_hash: ClassHash,
) -> RpcResult<UserOrL1HandlerTransaction> {
    // Welcome to type hell! This 3-part conversion will take you through the extenses
    // of a codebase so thick it might as well be pasta -yum!
    // Also should no be a problem as a declare transaction *should* not be able to
    // reference a contract class created on the same block (this kind of issue
    // might otherwise arise for `pending` blocks)
    let contract_class = DeoxysBackend::sierra_classes()
        .get_sierra_class(class_hash)
        .map_err(|e| {
            log::error!("Failed to fetch sierra class with hash {class_hash}: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .ok_or_else(|| {
            log::error!("The sierra class with hash {class_hash} is not present in db backend");
            StarknetRpcApiError::InternalServerError
        })?;

    let contract_class = mp_transactions::utils::sierra_to_casm_contract_class(contract_class).map_err(|e| {
        log::error!("Failed to convert the SierraContractClass to CasmContractClass: {e}");
        StarknetRpcApiError::InternalServerError
    })?;

    let contract_class = ContractClassBf::V1(ContractClassV1Bf::try_from(contract_class).map_err(|e| {
        log::error!("Failed to convert the compiler CasmContractClass to blockifier CasmContractClass: {e}");
        StarknetRpcApiError::InternalServerError
    })?);

    Ok(UserOrL1HandlerTransaction::User(UserTransaction::Declare(declare_tx, contract_class)))
}

fn tx_l1_handler<H>(
    chain_id: Felt,
    block_number: u64,
    l1_handler: mp_transactions::HandleL1MessageTransaction,
) -> RpcResult<UserOrL1HandlerTransaction>
where
    H: HasherT + Send + Sync + 'static,
{
    let chain_id = chain_id.0.into();
    let tx_hash = l1_handler.compute_hash::<H>(chain_id, false, Some(block_number));
    let paid_fee =
        DeoxysBackend::l1_handler_paid_fee().get_fee_paid_for_l1_handler_tx(tx_hash.into()).map_err(|e| {
            log::error!("Failed to retrieve fee paid on l1 for tx with hash `{tx_hash:?}`: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

    Ok(UserOrL1HandlerTransaction::L1Handler(l1_handler, paid_fee))
}

fn execution_infos<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    previous_block_hash: DHashT,
    transactions: Vec<UserOrL1HandlerTransaction>,
) -> RpcResult<TransactionExecutionInfo>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let (last, prev) = match transactions.split_last() {
        Some((last, prev)) => (vec![last.clone()], prev.to_vec()),
        None => (transactions, vec![]),
    };

    let execution_infos = client
        .client
        .runtime_api()
        .re_execute_transactions(previous_block_hash, prev, last)
        .map_err(|e| {
            log::error!("Failed to execute runtime API call: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            log::error!("Failed to reexecute the transactions: {e:?}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|_| {
            log::error!("One of the transaction failed during it's reexecution");
            StarknetRpcApiError::InternalServerError
        })?
        .pop()
        .ok_or_else(|| {
            log::error!("No execution info returned for the last transaction");
            StarknetRpcApiError::InternalServerError
        })?;

    Ok(execution_infos)
}

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
pub async fn get_transaction_receipt<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    transaction_hash: FieldElement,
) -> RpcResult<MaybePendingTransactionReceipt>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = DeoxysBackend::mapping()
        .block_hash_from_transaction_hash(Felt252Wrapper::from(transaction_hash).into())
        .map_err(|e| {
            log::error!("Failed to retrieve substrate block hash: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

    let chain_id = starknet.chain_id()?;

    match substrate_block_hash {
        Some(substrate_block_hash) => {
            get_transaction_receipt_finalized(starknet, chain_id, substrate_block_hash, transaction_hash)
        }
        None => {
            let substrate_block_hash =
                starknet.substrate_block_hash_from_starknet_block(BlockId::Tag(BlockTag::Latest)).map_err(|e| {
                    error!("'{e}'");
                    StarknetRpcApiError::BlockNotFound
                })?;

            get_transaction_receipt_pending(starknet, chain_id, substrate_block_hash, transaction_hash)
        }
    }
}
