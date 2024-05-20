use anyhow::Result;
use mc_sync::l1::ETHEREUM_STATE_UPDATE;
use mp_block::DeoxysBlock;
use mp_hashers::HasherT;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use mp_types::block::{DBlockT, DHashT};
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction as stx;
use starknet_core::types::{BlockId, BlockStatus, FieldElement};

use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::errors::StarknetRpcApiError;
use crate::{Felt, Starknet};

pub(crate) fn tx_hash_retrieve(tx_hashes: Vec<StarkFelt>) -> Vec<FieldElement> {
    // safe to unwrap because we know that the StarkFelt is a valid FieldElement
    tx_hashes.iter().map(|tx_hash| FieldElement::from_bytes_be(&tx_hash.0).unwrap()).collect()
}

pub(crate) fn tx_hash_compute<H>(block: &DeoxysBlock, chain_id: Felt) -> Vec<FieldElement>
where
    H: HasherT + Send + Sync + 'static,
{
    // safe to unwrap because we know that the StarkFelt is a valid FieldElement
    block
        .transactions_hashes::<H>(chain_id.0.into(), Some(block.header().block_number))
        .map(|tx_hash| FieldElement::from_bytes_be(&tx_hash.0.0).unwrap())
        .collect()
}

pub(crate) fn tx_conv(
    txs: &[stx::Transaction],
    tx_hashes: Vec<FieldElement>,
) -> Vec<starknet_core::types::Transaction> {
    txs.iter().zip(tx_hashes).map(|(tx, hash)| to_starknet_core_tx(tx.clone(), hash)).collect()
}

pub(crate) fn status(block_number: u64) -> BlockStatus {
    if block_number <= ETHEREUM_STATE_UPDATE.read().unwrap().block_number {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    }
}

#[allow(dead_code)]
pub fn previous_substrate_block_hash<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    substrate_block_hash: DHashT,
) -> Result<DHashT, StarknetRpcApiError>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> ,
    H: HasherT + Send + Sync + 'static,
{
    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash).map_err(|e| {
        log::error!("Failed to get block for block hash {substrate_block_hash}: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let block_number = starknet_block.header().block_number;
    let previous_block_number = match block_number {
        0 => 0,
        _ => block_number - 1,
    };
    let substrate_block_hash =
        starknet.substrate_block_hash_from_starknet_block(BlockId::Number(previous_block_number))?;

    Ok(substrate_block_hash)
}
