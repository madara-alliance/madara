use blockifier::execution::contract_class::{ContractClass as ContractClassBf, ContractClassV1 as ContractClassV1Bf};
use blockifier::transaction::transaction_execution as btx;
use jsonrpsee::core::RpcResult;
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

use crate::errors::StarknetRpcApiError;
use crate::{Felt, Starknet};

pub(crate) fn blockifier_transactions<A, BE, G, C, P, H>(
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
					tx_declare(client, substrate_block_hash, declare_tx.clone())
                }
                Transaction::L1Handler(l1_handler) => {
					tx_l1_handler::<H>(chain_id, block_number, l1_handler.clone())
                }
                Transaction::Deploy(_) => todo!(),
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
            tx_declare_v0v1(client, substrate_block_hash, declare_tx, class_hash)
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
    let _contract_class = client
        .overrides
        .for_block_hash(client.client.as_ref(), substrate_block_hash)
        .contract_class_by_class_hash(substrate_block_hash, class_hash)
        .ok_or_else(|| {
            log::error!("Failed to retrieve contract class from hash '{class_hash}'");
            StarknetRpcApiError::InternalServerError
        })?;

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
            tx_declare_v0v1(client, substrate_block_hash, declare_tx, class_hash)
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
