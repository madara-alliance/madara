use std::sync::Arc;

use blockifier::transaction::transactions::L1HandlerTransaction;
use jsonrpsee::core::RpcResult;
use log::error;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::core::Nonce;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{Calldata, Fee, TransactionVersion};
use starknet_core::types::{BlockId, FeeEstimate, MsgFromL1};

use crate::errors::StarknetRpcApiError;
use crate::madara_backend_client::get_block_by_block_hash;
use crate::{utils, Starknet, StarknetReadRpcApiServer};

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
pub async fn estimate_message_fee<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    message: MsgFromL1,
    block_id: BlockId,
) -> RpcResult<FeeEstimate>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        log::error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    // create a block context from block header
    let fee_token_address = starknet.client.runtime_api().fee_token_addresses(substrate_block_hash).map_err(|e| {
        log::error!("Failed to retrieve fee token address: {e}");
        StarknetRpcApiError::InternalServerError
    })?;
    let block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
    let block_header = block.header().clone();
    let block_context =
        block_header.into_block_context(fee_token_address, starknet_api::core::ChainId("SN_MAIN".to_string()));

    let block_number = starknet.block_number().map_err(|e| {
        log::error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;
    let chain_id = Felt252Wrapper(starknet.chain_id()?.0);

    let transaction = convert_message_into_tx::<H>(message, chain_id, Some(block_number));

    let message_fee = utils::execution::estimate_message_fee(transaction, &block_context).map_err(|e| {
        error!("Function execution failed: {:#?}", e);
        StarknetRpcApiError::ContractError
    })?;

    let estimate_message_fee = FeeEstimate {
        gas_consumed: message_fee.gas_consumed,
        gas_price: message_fee.gas_price,
        data_gas_consumed: message_fee.data_gas_consumed,
        data_gas_price: message_fee.data_gas_price,
        overall_fee: message_fee.overall_fee,
        unit: message_fee.unit,
    };

    Ok(estimate_message_fee)
}

pub fn convert_message_into_tx<H: HasherT + Send + Sync + 'static>(
    message: MsgFromL1,
    chain_id: Felt252Wrapper,
    block_number: Option<u64>,
) -> L1HandlerTransaction {
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

    L1HandlerTransaction { tx, tx_hash, paid_fee_on_l1: Fee(10) } //TODO: fix with real fee
}
