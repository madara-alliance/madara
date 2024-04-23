use blockifier::execution::contract_class::ClassInfo;
use blockifier::transaction::transaction_execution as btx;
use jsonrpsee::core::RpcResult;
use mp_hashers::HasherT;
use mp_types::block::{DBlockT, DHashT};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{Transaction, TransactionHash};
use starknet_ff::FieldElement;

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

pub(crate) fn blockifier_transactions<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
    transaction_with_hash: Vec<(Transaction, FieldElement)>,
) -> RpcResult<Vec<btx::Transaction>>
where
    A: ChainApi<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let transactions = transaction_with_hash
            .iter()
            .filter(|(tx, _)| !matches!(tx, Transaction::Deploy(_))) // deploy transaction was not supported by blockifier
            .map(|(tx, hash)| to_blockifier_transactions(client, tx, &TransactionHash(StarkFelt::new_unchecked(hash.to_bytes_be())), substrate_block_hash))
            .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

/// Convert an starknet-api Transaction to a blockifier Transaction
///
/// **note:** this function does not support deploy transaction
/// because it is not supported by blockifier
pub(crate) fn to_blockifier_transactions<A, BE, G, C, P, H>(
    client: &Starknet<A, BE, G, C, P, H>,
    transaction: &Transaction,
    tx_hash: &TransactionHash,
    substrate_block_hash: DHashT,
) -> RpcResult<btx::Transaction>
where
    A: ChainApi<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let paid_fee_on_l1 = match transaction {
        Transaction::L1Handler(_) => Some(starknet_api::transaction::Fee(1_000_000_000_000)),
        _ => None,
    };

    let class_info = match transaction {
        Transaction::Declare(declare_tx) => {
            let class_hash = declare_tx.class_hash();
            let class = client
                .overrides
                .for_block_hash(client.client.as_ref(), substrate_block_hash)
                .contract_class_by_class_hash(substrate_block_hash, class_hash)
                .ok_or_else(|| {
                    log::error!("Failed to retrieve contract class from hash '{class_hash}'");
                    StarknetRpcApiError::InternalServerError
                })?;

            // TODO: retrieve sierra_program_length and abi_length when they are stored in the storage
            Some(ClassInfo::new(&class, 0, 0).map_err(|_| {
                log::error!("Mismatch between the length of the sierra program and the class version");
                StarknetRpcApiError::InternalServerError
            })?)
        }
        _ => None,
    };

    btx::Transaction::from_api(transaction.clone(), *tx_hash, class_info, paid_fee_on_l1, None, false).map_err(|_| {
        log::error!("Failed to convert transaction to blockifier transaction");
        StarknetRpcApiError::InternalServerError.into()
    })
}
