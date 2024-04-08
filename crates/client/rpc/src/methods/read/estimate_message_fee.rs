use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::core::Nonce;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{Calldata, Fee, TransactionVersion};
use starknet_core::types::{BlockId, FeeEstimate, MsgFromL1};
use std::sync::Arc;
use blockifier::transaction::transactions::L1HandlerTransaction;

use crate::errors::StarknetRpcApiError;
use crate::{Starknet, StarknetReadRpcApiServer};

/// Estimate the L2 fee of a message sent on L1
///
/// # Arguments
///
/// * `message` - the message to estimate
/// * `block_id` - hash, number (height), or tag of the requested block
///
/// # Returns
///
/// * `FeeEstimate` - the fee estimation (gas consumed, gas price, overall fee, unit)
///
/// # Errors
///
/// BlockNotFound : If the specified block does not exist.
/// ContractNotFound : If the specified contract address does not exist.
/// ContractError : If there is an error with the contract.
pub async fn estimate_message_fee<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    message: MsgFromL1,
    block_id: BlockId,
) -> RpcResult<FeeEstimate>
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
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        log::error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;
    let block_number = starknet.block_number().map_err(|e| {
        log::error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;
    let chain_id = Felt252Wrapper(starknet.chain_id()?.0);

    let transaction = convert_message_into_tx::<H>(message, chain_id, Some(block_number));

    let message_fee = starknet
        .client
        .runtime_api()
        .estimate_message_fee(substrate_block_hash, transaction)
        .map_err(|e| {
            error!("Runtime Api error: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            error!("Function execution failed: {:#?}", e);
            StarknetRpcApiError::ContractError
        })?;

    let estimate_message_fee = FeeEstimate {
        gas_consumed: message_fee.gas_consumed.0,
        gas_price: message_fee.gas_price.0,
        data_gas_consumed: message_fee.data_gas_consumed.0,
        data_gas_price: message_fee.data_gas_price.0,
        overall_fee: message_fee.overall_fee.0,
        unit: message_fee.unit.into(),
    };

    Ok(estimate_message_fee)
}

pub fn convert_message_into_tx<H: HasherT + Send + Sync + 'static>(
    message: MsgFromL1,
    chain_id: Felt252Wrapper,
    block_number: Option<u64>,
) -> L1HandlerTransaction {
    let transaction = {
        let calldata = std::iter::once(Felt252Wrapper::from(message.from_address).into())
            .chain(message.payload.into_iter().map(|felt| Felt252Wrapper::from(felt).into()))
            .collect();
        let tx = starknet_api::transaction::L1HandlerTransaction {
            version: TransactionVersion::ZERO,
            nonce: Nonce(StarkFelt::ZERO),
            contract_address: Felt252Wrapper::from(message.to_address).into(),
            entry_point_selector: Felt252Wrapper::from(message.entry_point_selector).into(),
            calldata: Calldata(Arc::new(calldata)),
        };
        let tx_hash = tx.compute_hash::<H>(chain_id, true, block_number);

        L1HandlerTransaction { tx, tx_hash, paid_fee_on_l1: Fee(10) }
    };
    transaction
}
