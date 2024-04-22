use blockifier::context::BlockContext;
use blockifier::execution::contract_class::{ContractClass as ContractClassBf, ContractClassV1 as ContractClassV1Bf};
use blockifier::transaction::objects::TransactionExecutionInfo;
use blockifier::transaction::transaction_execution as btx;
use jsonrpsee::core::RpcResult;
use mc_db::storage_handler::{self, StorageView};
use mc_db::DeoxysBackend;
use mc_genesis_data_provider::GenesisProvider;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_types::block::{DBlockT, DHashT};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::core::{ClassHash, ContractAddress};
use starknet_api::transaction::{
    DeclareTransaction, DeployAccountTransaction, InvokeTransaction, L1HandlerTransaction, Transaction, TransactionHash,
};
use starknet_core::types::{
    BlockId, ComputationResources, DataAvailabilityResources, DataResources, DeclareTransactionReceipt,
    DeployAccountTransactionReceipt, ExecutionResources, ExecutionResult, FieldElement, Hash256,
    InvokeTransactionReceipt, L1HandlerTransactionReceipt, TransactionFinalityStatus, TransactionReceipt,
    TransactionReceiptWithBlockInfo,
};

use crate::errors::StarknetRpcApiError;
use crate::utils::{
    blockifier_call_info_to_starknet_resources, extract_events_from_call_info, extract_messages_from_call_info,
    get_block_by_block_hash, tx_hash_compute, tx_hash_retrieve,
};
use crate::{Felt, Starknet};

pub fn get_transaction_receipt_finalized<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    chain_id: Felt,
    substrate_block_hash: DHashT,
    transaction_hash: FieldElement,
) -> RpcResult<TransactionReceiptWithBlockInfo>
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
    let block_header = block.header().clone();
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

    // deploy transaction was not supported by blockifier
    if let Transaction::Deploy(_) = transaction {
        log::error!("re executing a deploy transaction is not supported yet");
        return Err(StarknetRpcApiError::UnimplementedMethod.into());
    }

    let transactions = transactions(client, substrate_block_hash, chain_id, &block, block_number, tx_index)?;

    let fee_token_address = client.client.runtime_api().fee_token_addresses(substrate_block_hash).map_err(|e| {
        log::error!("Failed to retrieve fee token address: {e}");
        StarknetRpcApiError::InternalServerError
    })?;
    // TODO: convert the real chain_id in String
    let block_context =
        block_header.into_block_context(fee_token_address, starknet_api::core::ChainId("SN_MAIN".to_string()));
    let execution_infos = execution_infos(client, previous_block_hash, transactions, &block_context)?;

    // TODO(#1291): compute message hash correctly to L1HandlerTransactionReceipt
    let message_hash: Hash256 = Hash256::from_felt(&FieldElement::default());

    // TODO: implement fee in Fri when Blockifier will support it
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

    let events = match execution_infos.execute_call_info {
        Some(ref call_info) => extract_events_from_call_info(call_info),
        None => vec![],
    };

    let messages_sent = match execution_infos.execute_call_info {
        Some(ref call_info) => extract_messages_from_call_info(call_info),
        None => vec![],
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
        Transaction::DeployAccount(_) => TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
            transaction_hash,
            actual_fee,
            finality_status,
            messages_sent,
            events,
            execution_resources,
            execution_result,
            // TODO: retrieve account address
            contract_address: FieldElement::default(),
        }),
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

    let block_info = starknet_core::types::ReceiptBlock::Block { block_hash: block_hash.0, block_number };

    Ok(TransactionReceiptWithBlockInfo { receipt, block: block_info })
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
) -> RpcResult<Vec<btx::Transaction>>
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
            .filter(|tx| !matches!(tx, Transaction::Deploy(_))) // deploy transaction was not supported by blockifier
            .map(|tx| match tx {
                Transaction::Invoke(invoke_tx) => {
					tx_invoke_transaction(invoke_tx.clone(), invoke_tx.compute_hash::<H>(Felt252Wrapper::from(chain_id.0), false, Some(block_number)))
                }
                // TODO: add real contract address param here
                Transaction::DeployAccount(deploy_account_tx) => {
					tx_deploy_account(deploy_account_tx.clone(), deploy_account_tx.compute_hash::<H>(Felt252Wrapper::from(chain_id.0), false, Some(block_number)), ContractAddress::default())
                }
                Transaction::Declare(declare_tx) => {
					tx_declare(client, substrate_block_hash, declare_tx.clone(), block_number)
                }
                Transaction::L1Handler(l1_handler) => {
					tx_l1_handler::<H>(chain_id, block_number, l1_handler.clone())
                }
                _ => unreachable!("Deploy transactions are not supported"),
            })
            .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

fn tx_invoke_transaction(tx: InvokeTransaction, hash: TransactionHash) -> RpcResult<btx::Transaction> {
    Ok(btx::Transaction::AccountTransaction(blockifier::transaction::account_transaction::AccountTransaction::Invoke(
        blockifier::transaction::transactions::InvokeTransaction { tx, tx_hash: hash, only_query: false },
    )))
}

fn tx_deploy_account(
    tx: DeployAccountTransaction,
    hash: TransactionHash,
    contract_address: ContractAddress,
) -> RpcResult<btx::Transaction> {
    Ok(btx::Transaction::AccountTransaction(
        blockifier::transaction::account_transaction::AccountTransaction::DeployAccount(
            blockifier::transaction::transactions::DeployAccountTransaction {
                tx,
                tx_hash: hash,
                only_query: false,
                contract_address,
            },
        ),
    ))
}

fn tx_declare<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
    declare_tx: DeclareTransaction,
    block_number: u64,
) -> RpcResult<btx::Transaction>
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
    let class_hash = ClassHash(Felt252Wrapper::from(*declare_tx.class_hash()).into());

    match declare_tx {
        DeclareTransaction::V0(_) | DeclareTransaction::V1(_) => {
            tx_declare_v0v1(client, substrate_block_hash, declare_tx, class_hash, block_number)
        }
        DeclareTransaction::V2(_) => tx_declare_v2(declare_tx, class_hash),
        DeclareTransaction::V3(_) => todo!("implement DeclareTransaction::V3"),
    }
}

fn tx_declare_v0v1<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
    declare_tx: DeclareTransaction,
    class_hash: ClassHash,
    block_number: u64,
) -> RpcResult<btx::Transaction>
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
    let Ok(Some(_contract_class)) = storage_handler::contract_class().get_at(&class_hash, block_number) else {
        log::error!("Failed to retrieve contract class from hash '{class_hash}'");
        return Err(StarknetRpcApiError::InternalServerError.into());
    };

    // let class_info = ClassInfo::new(
    //     &contract_class,
    //     contract_class.
    //     contract_class
    // )
    // .unwrap();

    // Ok(btx::Transaction::AccountTransaction(
    //     blockifier::transaction::account_transaction::AccountTransaction::Declare(
    //     blockifier::transaction::transactions::DeclareTransaction::new(
    //         declare_tx,
    //         declare_tx.compute_hash::<H>(Felt252Wrapper::from(class_hash.0).into(), false, None),
    //         class_info,
    //     ).unwrap()
    // )))
    // TODO: Correct this that was used as a place holder to compile
    match declare_tx {
        DeclareTransaction::V0(_) | DeclareTransaction::V1(_) => {
            tx_declare_v0v1(client, substrate_block_hash, declare_tx, class_hash, block_number)
        }
        DeclareTransaction::V2(_) => tx_declare_v2(declare_tx, class_hash),
        DeclareTransaction::V3(_) => todo!("implement DeclareTransaction::V3"),
    }
}

fn tx_declare_v2(declare_tx: DeclareTransaction, _class_hash: ClassHash) -> RpcResult<btx::Transaction> {
    // Welcome to type hell! This 3-part conversion will take you through the extenses
    // of a codebase so thick it might as well be pasta -yum!
    // Also should no be a problem as a declare transaction *should* not be able to
    // reference a contract class created on the same block (this kind of issue
    // might otherwise arise for `pending` blocks)
    // TODO: change this contract class to the correct one
    let contract_class = starknet_api::state::ContractClass::default();

    let contract_class = mp_transactions::utils::sierra_to_casm_contract_class(contract_class).map_err(|e| {
        log::error!("Failed to convert the SierraContractClass to CasmContractClass: {e}");
        StarknetRpcApiError::InternalServerError
    })?;

    let _contract_class = ContractClassBf::V1(ContractClassV1Bf::try_from(contract_class).map_err(|e| {
        log::error!("Failed to convert the compiler CasmContractClass to blockifier CasmContractClass: {e}");
        StarknetRpcApiError::InternalServerError
    })?);

    // Ok(btx::Transaction::AccountTransaction(
    //     blockifier::transaction::account_transaction::AccountTransaction::Declare(
    //     blockifier::transaction::transactions::DeclareTransaction::new(
    //         declare_tx,
    //         declare_tx.compute_hash::<PedersenHasher>(Felt252Wrapper::from(class_hash.0).into(),
    // false, None),         contract_class,
    //     ).unwrap()
    // )))
    // TODO: Correct this that was used as a place holder to compile
    match declare_tx {
        DeclareTransaction::V0(_) => todo!(),
        DeclareTransaction::V1(_) => todo!(),
        DeclareTransaction::V2(_) => tx_declare_v2(declare_tx, _class_hash),
        DeclareTransaction::V3(_) => todo!("implement DeclareTransaction::V3"),
    }
}

fn tx_l1_handler<H>(chain_id: Felt, block_number: u64, l1_handler: L1HandlerTransaction) -> RpcResult<btx::Transaction>
where
    H: HasherT + Send + Sync + 'static,
{
    let chain_id = chain_id.0.into();
    let tx_hash = l1_handler.compute_hash::<H>(chain_id, false, Some(block_number));
    let paid_fee = DeoxysBackend::l1_handler_paid_fee().get_fee_paid_for_l1_handler_tx(tx_hash.0).map_err(|e| {
        log::error!("Failed to retrieve fee paid on l1 for tx with hash `{tx_hash:?}`: {e}");
        StarknetRpcApiError::InternalServerError
    })?;

    Ok(btx::Transaction::L1HandlerTransaction(blockifier::transaction::transactions::L1HandlerTransaction {
        tx: l1_handler,
        tx_hash,
        paid_fee_on_l1: paid_fee,
    }))
}

fn execution_infos<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    previous_block_hash: DHashT,
    transactions: Vec<btx::Transaction>,
    block_context: &BlockContext,
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
        .re_execute_transactions(previous_block_hash, prev, last, block_context)
        .map_err(|e| {
            log::error!("Failed to execute runtime API call: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            log::error!("Failed to reexecute the transactions: {e:?}");
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
) -> RpcResult<TransactionReceiptWithBlockInfo>
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

    get_transaction_receipt_finalized(starknet, chain_id, substrate_block_hash.unwrap(), transaction_hash)
}
