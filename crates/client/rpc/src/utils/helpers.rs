use anyhow::Result;
use mc_sync::l1::ETHEREUM_STATE_UPDATE;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use mp_types::block::{DBlockT, DHashT};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::hash::StarkFelt;
use starknet_api::state::ThinStateDiff;
use starknet_api::transaction as stx;
use starknet_core::types::{
    BlockId, BlockStatus, ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, FieldElement, NonceUpdate,
    ReplacedClassItem, StateDiff, StorageEntry,
};

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

pub fn previous_substrate_block_hash<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    substrate_block_hash: DHashT,
) -> Result<DHashT, StarknetRpcApiError>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
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
        starknet.substrate_block_hash_from_starknet_block(BlockId::Number(previous_block_number)).map_err(|e| {
            log::error!("Failed to retrieve previous block substrate hash: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

    Ok(substrate_block_hash)
}

/// Returns a [`StateDiff`] from a [`ThinStateDiff`]
pub(crate) fn to_rpc_state_diff(thin_state_diff: ThinStateDiff) -> StateDiff {
    let nonces = thin_state_diff
        .nonces
        .iter()
        .map(|x| NonceUpdate {
            contract_address: Felt252Wrapper::from(x.0.0.0).into(),
            nonce: Felt252Wrapper::from(x.1.0).into(),
        })
        .collect();

    let storage_diffs = thin_state_diff
        .storage_diffs
        .iter()
        .map(|x| ContractStorageDiffItem {
            address: Felt252Wrapper::from(x.0.0.0).into(),
            storage_entries: x
                .1
                .iter()
                .map(|y| StorageEntry {
                    key: Felt252Wrapper::from(y.0.0.0).into(),
                    value: Felt252Wrapper::from(*y.1).into(),
                })
                .collect(),
        })
        .collect();

    let deprecated_declared_classes =
        thin_state_diff.deprecated_declared_classes.iter().map(|x| Felt252Wrapper::from(x.0).into()).collect();

    let declared_classes = thin_state_diff
        .declared_classes
        .iter()
        .map(|x| DeclaredClassItem {
            class_hash: Felt252Wrapper::from(x.0.0).into(),
            compiled_class_hash: Felt252Wrapper::from(x.1.0).into(),
        })
        .collect();

    let deployed_contracts = thin_state_diff
        .deployed_contracts
        .iter()
        .map(|x| DeployedContractItem {
            address: Felt252Wrapper::from(x.0.0.0).into(),
            class_hash: Felt252Wrapper::from(x.1.0).into(),
        })
        .collect();

    let replaced_classes = thin_state_diff
        .replaced_classes
        .iter()
        .map(|x| ReplacedClassItem {
            contract_address: Felt252Wrapper::from(x.0.0.0).into(),
            class_hash: Felt252Wrapper::from(x.1.0).into(),
        })
        .collect();

    StateDiff {
        nonces,
        storage_diffs,
        deprecated_declared_classes,
        declared_classes,
        deployed_contracts,
        replaced_classes,
    }
}
