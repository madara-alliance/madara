//! Converts types from [`starknet_providers`] to deoxys's expected types.

use std::collections::HashMap;
use std::str::FromStr;

use dc_db::storage_updates::DbClassUpdate;
use dp_block::header::{GasPrices, L1DataAvailabilityMode, PendingHeader};
use dp_block::{
    DeoxysBlock, DeoxysBlockInfo, DeoxysBlockInner, DeoxysPendingBlock, DeoxysPendingBlockInfo, Header, StarknetVersion,
};
use dp_class::{ClassHash, ClassInfo, ConvertedClass, ToCompiledClass};
use dp_convert::felt_to_u128;
use dp_receipt::{Event, TransactionReceipt};
use dp_transactions::MAIN_CHAIN_ID;
use rayon::prelude::*;
use starknet_core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, PendingStateUpdate,
    ReplacedClassItem, StateDiff as StateDiffCore, StorageEntry,
};
use starknet_providers::sequencer::models::state_update::{
    DeclaredContract, DeployedContract, StateDiff as StateDiffProvider, StorageDiff as StorageDiffProvider,
};
use starknet_providers::sequencer::models::{self as p, StateUpdate as StateUpdateProvider};
use starknet_types_core::felt::Felt;

use crate::commitments::calculate_tx_and_event_commitments;
use crate::l2::L2SyncError;

pub fn convert_inner(
    txs: Vec<p::TransactionType>,
    receipts: Vec<p::ConfirmedTransactionReceipt>,
) -> Result<DeoxysBlockInner, L2SyncError> {
    // converts starknet_provider transactions and events to dp_transactions and starknet_api events
    let transactions_receipts = Iterator::zip(receipts.into_iter(), txs.iter())
        .map(|(tx_receipts, tx)| TransactionReceipt::from_provider(tx_receipts, tx))
        .collect::<Vec<_>>();
    let transactions = txs.into_iter().map(|tx| tx.try_into()).collect::<Result<_, _>>().unwrap();

    Ok(DeoxysBlockInner::new(transactions, transactions_receipts))
}

/// This function does not check block hashes and such
pub fn convert_pending(block: p::Block, _chain_id: Felt) -> Result<DeoxysPendingBlock, L2SyncError> {
    let block_inner = convert_inner(block.transactions, block.transaction_receipts)?;

    let header = PendingHeader {
        parent_block_hash: block.parent_block_hash,
        block_timestamp: block.timestamp,
        sequencer_address: block.sequencer_address.unwrap_or(Felt::ZERO),
        protocol_version: protocol_version(block.starknet_version),
        l1_gas_price: resource_price(block.l1_gas_price, block.l1_data_gas_price),
        l1_da_mode: l1_da_mode(block.l1_da_mode),
    };

    // TODO tx_hash

    // let ((_transaction_commitment, txs_hashes), event_commitment) =
    //     memory_transaction_commitment(&block_inner.transactions, &events, chain_id, block_number);

    Ok(DeoxysPendingBlock::new(DeoxysPendingBlockInfo::new(header, vec![]), block_inner))
}

/// Compute heavy, this should only be called in a rayon ctx
pub fn convert_and_verify_block(block: p::Block, chain_id: Felt) -> Result<DeoxysBlock, L2SyncError> {
    let block_inner = convert_inner(block.transactions, block.transaction_receipts)?;

    // converts starknet_provider transactions and events to dp_transactions and starknet_api events
    let events = events(&block_inner.receipts);

    let block_hash = block.block_hash.ok_or(L2SyncError::BlockFormat("No block hash provided".into()))?;
    let block_number = block.block_number.ok_or(L2SyncError::BlockFormat("No block number provided".into()))?;

    let global_state_root = block.state_root.ok_or(L2SyncError::BlockFormat("No state root provided".into()))?;
    let transaction_count = block_inner.transactions.len() as u128;
    let event_count = events.len() as u128;

    let ((transaction_commitment, txs_hashes), event_commitment) =
        calculate_tx_and_event_commitments(&block_inner.transactions, &events, chain_id, block_number);

    let header = Header {
        parent_block_hash: block.parent_block_hash,
        block_timestamp: block.timestamp,
        sequencer_address: block.sequencer_address.unwrap_or(Felt::ZERO),
        protocol_version: protocol_version(block.starknet_version),
        l1_gas_price: resource_price(block.l1_gas_price, block.l1_data_gas_price),
        l1_da_mode: l1_da_mode(block.l1_da_mode),
        block_number,
        global_state_root,
        transaction_count,
        transaction_commitment,
        event_count,
        event_commitment,
    };

    let computed_block_hash = header.hash(chain_id);
    // mismatched block hash is allowed for blocks 1466..=2242 on mainnet
    if computed_block_hash != block_hash && !((1466..=2242).contains(&block_number) && chain_id == MAIN_CHAIN_ID) {
        if event_commitment != block.event_commitment.unwrap() {
            log::warn!(
                "Mismatched event commitment({}): expected 0x{:x}, got 0x{:x}",
                block_number,
                event_commitment,
                block.event_commitment.unwrap()
            );
        }
        if transaction_commitment != block.transaction_commitment.unwrap() {
            log::warn!(
                "Mismatched transaction commitment({}): expected 0x{:x}, got 0x{:x}",
                block_number,
                transaction_commitment,
                block.transaction_commitment.unwrap()
            );
        }
        return Err(L2SyncError::MismatchedBlockHash(block_number));
    }

    Ok(DeoxysBlock::new(DeoxysBlockInfo::new(header, txs_hashes, block_hash), block_inner))
}

fn protocol_version(version: Option<String>) -> StarknetVersion {
    version.map(|version| StarknetVersion::from_str(&version).unwrap_or_default()).unwrap_or_default()
}

/// Converts the l1 gas price and l1 data gas price to a GasPrices struct, if the l1 gas price is
/// not 0. If the l1 gas price is 0, returns None.
/// The other prices are converted to NonZeroU128, with 0 being converted to 1.
fn resource_price(
    l1_gas_price: starknet_core::types::ResourcePrice,
    l1_data_gas_price: starknet_core::types::ResourcePrice,
) -> GasPrices {
    GasPrices {
        eth_l1_gas_price: felt_to_u128(&l1_gas_price.price_in_wei).unwrap(),
        strk_l1_gas_price: felt_to_u128(&l1_gas_price.price_in_fri).unwrap(),
        eth_l1_data_gas_price: felt_to_u128(&l1_data_gas_price.price_in_wei).unwrap(),
        strk_l1_data_gas_price: felt_to_u128(&l1_data_gas_price.price_in_fri).unwrap(),
    }
}

fn l1_da_mode(mode: starknet_core::types::L1DataAvailabilityMode) -> L1DataAvailabilityMode {
    match mode {
        starknet_core::types::L1DataAvailabilityMode::Calldata => L1DataAvailabilityMode::Calldata,
        starknet_core::types::L1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
    }
}

fn events(receipts: &[TransactionReceipt]) -> Vec<Event> {
    receipts.iter().flat_map(TransactionReceipt::events).cloned().collect()
}

pub fn state_update(state_update: StateUpdateProvider) -> PendingStateUpdate {
    let old_root = state_update.old_root;
    let state_diff = state_diff(state_update.state_diff);

    // StateUpdateCore { block_hash, old_root, new_root, state_diff }
    PendingStateUpdate { old_root, state_diff }
}

fn state_diff(state_diff: StateDiffProvider) -> StateDiffCore {
    let storage_diffs = storage_diffs(state_diff.storage_diffs);
    let deprecated_declared_classes = state_diff.old_declared_contracts;
    let declared_classes = declared_classes(state_diff.declared_classes);
    let deployed_contracts = deployed_contracts(state_diff.deployed_contracts);
    let replaced_classes = replaced_classes(state_diff.replaced_classes);
    let nonces = nonces(state_diff.nonces);

    StateDiffCore {
        storage_diffs,
        deprecated_declared_classes,
        declared_classes,
        deployed_contracts,
        replaced_classes,
        nonces,
    }
}

fn storage_diffs(storage_diffs: HashMap<Felt, Vec<StorageDiffProvider>>) -> Vec<ContractStorageDiffItem> {
    storage_diffs
        .into_iter()
        .map(|(address, entries)| ContractStorageDiffItem { address, storage_entries: storage_entries(entries) })
        .collect()
}

fn storage_entries(storage_entries: Vec<StorageDiffProvider>) -> Vec<StorageEntry> {
    storage_entries.into_iter().map(|StorageDiffProvider { key, value }| StorageEntry { key, value }).collect()
}

fn declared_classes(declared_classes: Vec<DeclaredContract>) -> Vec<DeclaredClassItem> {
    declared_classes
        .into_iter()
        .map(|DeclaredContract { class_hash, compiled_class_hash }| DeclaredClassItem {
            class_hash,
            compiled_class_hash,
        })
        .collect()
}

fn deployed_contracts(deplyed_contracts: Vec<DeployedContract>) -> Vec<DeployedContractItem> {
    deplyed_contracts
        .into_iter()
        .map(|DeployedContract { address, class_hash }| DeployedContractItem { address, class_hash })
        .collect()
}

fn replaced_classes(replaced_classes: Vec<DeployedContract>) -> Vec<ReplacedClassItem> {
    replaced_classes
        .into_iter()
        .map(|DeployedContract { address, class_hash }| ReplacedClassItem { contract_address: address, class_hash })
        .collect()
}

fn nonces(nonces: HashMap<Felt, Felt>) -> Vec<NonceUpdate> {
    // TODO: make sure the order is `contract_address` -> `nonce`
    // and not `nonce` -> `contract_address`
    nonces.into_iter().map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce }).collect()
}

#[derive(thiserror::Error, Debug)]
pub enum ConvertClassError {
    #[error("Mismatched class hash: {0}")]
    MismatchedClassHash(Felt),
    #[error("Compute class hash error: {0}")]
    ComputeClassHashError(String),
    #[error("Compilation class error: {0}")]
    CompilationClassError(String),
}

pub fn convert_and_verify_class(
    classes: Vec<DbClassUpdate>,
    block_n: Option<u64>,
) -> Result<Vec<ConvertedClass>, ConvertClassError> {
    classes
        .into_par_iter()
        .map(|class_update| {
            let DbClassUpdate { class_hash, contract_class, compiled_class_hash } = class_update;

            if class_hash
                != contract_class.class_hash().map_err(|e| ConvertClassError::ComputeClassHashError(e.to_string()))?
            {
                return Err(ConvertClassError::MismatchedClassHash(class_update.class_hash));
            }

            let compiled_class =
                contract_class.compile().map_err(|e| ConvertClassError::CompilationClassError(e.to_string()))?;

            let class_info =
                ClassInfo { contract_class: contract_class.into(), block_number: block_n, compiled_class_hash };

            Ok(ConvertedClass {
                class_infos: (class_hash, class_info),
                class_compiled: (compiled_class_hash, compiled_class),
            })
        })
        .collect::<Result<Vec<_>, _>>()
}
