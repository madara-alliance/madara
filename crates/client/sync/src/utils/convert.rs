//! Converts types from [`starknet_providers`] to madara's expected types.

use dc_db::storage_updates::DbClassUpdate;
use dp_block::header::{GasPrices, L1DataAvailabilityMode, PendingHeader};
use dp_block::{
    Header, MadaraBlock, MadaraBlockInfo, MadaraBlockInner, MadaraPendingBlock, MadaraPendingBlockInfo, StarknetVersion,
};
use dp_class::{ClassInfo, ConvertedClass, ToCompiledClass};
use dp_convert::felt_to_u128;
use dp_receipt::{Event, TransactionReceipt};
use dp_state_update::StateDiff;
use dp_transactions::MAIN_CHAIN_ID;
use rayon::prelude::*;
use starknet_types_core::felt::Felt;

use crate::commitments::{
    calculate_transaction_hash, memory_event_commitment, memory_receipt_commitment, memory_transaction_commitment,
};
use crate::l2::L2SyncError;

pub fn convert_inner(
    txs: Vec<starknet_providers::sequencer::models::TransactionType>,
    receipts: Vec<starknet_providers::sequencer::models::ConfirmedTransactionReceipt>,
) -> Result<MadaraBlockInner, L2SyncError> {
    // converts starknet_provider transactions and events to dp_transactions and starknet_api events
    let transactions_receipts = Iterator::zip(receipts.into_iter(), txs.iter())
        .map(|(tx_receipts, tx)| TransactionReceipt::from_provider(tx_receipts, tx))
        .collect::<Vec<_>>();
    let transactions = txs.into_iter().map(|tx| tx.try_into()).collect::<Result<_, _>>()?;

    Ok(MadaraBlockInner::new(transactions, transactions_receipts))
}

/// This function only does tx hash computation.
pub fn convert_pending(
    block: starknet_providers::sequencer::models::Block,
    state_diff: starknet_core::types::StateDiff,
    chain_id: Felt,
) -> Result<(MadaraPendingBlock, StateDiff), L2SyncError> {
    let block_inner = convert_inner(block.transactions, block.transaction_receipts)?;
    let converted_state_diff = state_diff.into();

    let header = PendingHeader {
        parent_block_hash: block.parent_block_hash,
        block_timestamp: block.timestamp,
        sequencer_address: block.sequencer_address.unwrap_or(Felt::ZERO),
        protocol_version: protocol_version(block.starknet_version)?,
        l1_gas_price: resource_price(block.l1_gas_price, block.l1_data_gas_price)?,
        l1_da_mode: l1_da_mode(block.l1_da_mode),
    };
    let tx_hashes = block_inner.transactions.iter().map(|tx| calculate_transaction_hash(tx, chain_id, None)).collect();

    Ok((MadaraPendingBlock::new(MadaraPendingBlockInfo::new(header, tx_hashes), block_inner), converted_state_diff))
}

/// Compute heavy, this should only be called in a rayon ctx
pub fn convert_and_verify_block(
    block: starknet_providers::sequencer::models::Block,
    state_diff: starknet_core::types::StateDiff,
    chain_id: Felt,
) -> Result<(MadaraBlock, StateDiff), L2SyncError> {
    let block_inner = convert_inner(block.transactions, block.transaction_receipts)?;
    let converted_state_diff: StateDiff = state_diff.into();

    let block_hash = block.block_hash.ok_or(L2SyncError::BlockFormat("No block hash provided".into()))?;
    let block_number = block.block_number.ok_or(L2SyncError::BlockFormat("No block number provided".into()))?;

    let global_state_root = block.state_root.ok_or(L2SyncError::BlockFormat("No state root provided".into()))?;
    let starknet_version = protocol_version(block.starknet_version)?;

    // compute the 4 commitments in parallel
    let BlockCommitments {
        transaction_commitment,
        event_commitment,
        receipt_commitment,
        state_diff_commitment,
        tx_hashes,
        transaction_count,
        event_count,
        state_diff_length,
    } = compute_commitments_for_block(&block_inner, &converted_state_diff, starknet_version, chain_id, block_number);

    let header = Header::new(
        block.parent_block_hash,
        block_number,
        global_state_root,
        block.sequencer_address.unwrap_or(Felt::ZERO),
        block.timestamp,
        transaction_count,
        transaction_commitment,
        event_count,
        event_commitment,
        state_diff_length,
        state_diff_commitment,
        receipt_commitment,
        starknet_version,
        resource_price(block.l1_gas_price, block.l1_data_gas_price)?,
        l1_da_mode(block.l1_da_mode),
    );

    let computed_block_hash = header.compute_hash(chain_id);

    // mismatched block hash is allowed for blocks 1466..=2242 on mainnet
    if computed_block_hash != block_hash && !((1466..=2242).contains(&block_number) && chain_id == MAIN_CHAIN_ID) {
        return Err(L2SyncError::MismatchedBlockHash(block_number));
    }

    Ok((MadaraBlock::new(MadaraBlockInfo::new(header, tx_hashes, block_hash), block_inner), converted_state_diff))
}

pub struct BlockCommitments {
    pub transaction_commitment: Felt,
    pub transaction_count: u64,
    pub event_commitment: Felt,
    pub event_count: u64,
    pub state_diff_length: u64,
    pub state_diff_commitment: Felt,
    pub receipt_commitment: Felt,
    pub tx_hashes: Vec<Felt>,
}

pub fn compute_commitments_for_block(
    block_inner: &MadaraBlockInner,
    state_diff: &StateDiff,
    starknet_version: StarknetVersion,
    chain_id: Felt,
    block_number: u64,
) -> BlockCommitments {
    let events_with_tx_hash = events_with_tx_hash(&block_inner.receipts);

    let tasks_tx_and_event_commitment = || {
        rayon::join(
            || memory_transaction_commitment(&block_inner.transactions, chain_id, starknet_version, block_number),
            || memory_event_commitment(&events_with_tx_hash, starknet_version),
        )
    };
    let tasks_receipt_and_state_diff_commitment =
        || rayon::join(|| memory_receipt_commitment(&block_inner.receipts), || state_diff.compute_hash());
    let (((transaction_commitment, tx_hashes), event_commitment), (receipt_commitment, state_diff_commitment)) =
        rayon::join(tasks_tx_and_event_commitment, tasks_receipt_and_state_diff_commitment);

    BlockCommitments {
        transaction_commitment,
        transaction_count: block_inner.transactions.len() as _,
        event_commitment,
        event_count: events_with_tx_hash.len() as _,
        receipt_commitment,
        state_diff_commitment,
        state_diff_length: state_diff.len() as _,
        tx_hashes,
    }
}

fn protocol_version(version: Option<String>) -> Result<StarknetVersion, L2SyncError> {
    match version {
        None => Ok(StarknetVersion::default()),
        Some(version) => version.parse().map_err(L2SyncError::InvalidStarknetVersion),
    }
}

/// Converts the l1 gas price and l1 data gas price to a GasPrices struct, if the l1 gas price is
/// not 0. If the l1 gas price is 0, returns None.
/// The other prices are converted to NonZeroU128, with 0 being converted to 1.
fn resource_price(
    l1_gas_price: starknet_core::types::ResourcePrice,
    l1_data_gas_price: starknet_core::types::ResourcePrice,
) -> Result<GasPrices, L2SyncError> {
    Ok(GasPrices {
        eth_l1_gas_price: felt_to_u128(&l1_gas_price.price_in_wei)
            .map_err(|_| L2SyncError::GasPriceOutOfBounds(l1_gas_price.price_in_wei))?,
        strk_l1_gas_price: felt_to_u128(&l1_gas_price.price_in_fri)
            .map_err(|_| L2SyncError::GasPriceOutOfBounds(l1_gas_price.price_in_fri))?,
        eth_l1_data_gas_price: felt_to_u128(&l1_data_gas_price.price_in_wei)
            .map_err(|_| L2SyncError::GasPriceOutOfBounds(l1_data_gas_price.price_in_wei))?,
        strk_l1_data_gas_price: felt_to_u128(&l1_data_gas_price.price_in_fri)
            .map_err(|_| L2SyncError::GasPriceOutOfBounds(l1_data_gas_price.price_in_fri))?,
    })
}

fn l1_da_mode(mode: starknet_core::types::L1DataAvailabilityMode) -> L1DataAvailabilityMode {
    match mode {
        starknet_core::types::L1DataAvailabilityMode::Calldata => L1DataAvailabilityMode::Calldata,
        starknet_core::types::L1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
    }
}

fn events_with_tx_hash(receipts: &[TransactionReceipt]) -> Vec<(Felt, Event)> {
    receipts
        .iter()
        .flat_map(|receipt| receipt.events().iter().map(move |event| (receipt.transaction_hash(), event.clone())))
        .collect()
}

#[derive(thiserror::Error, Debug)]
pub enum ConvertClassError {
    #[error("Mismatched class hash, expected {expected:#x}; got {got:#x}")]
    MismatchedClassHash { expected: Felt, got: Felt },
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

            // TODO(class_hash): uncomment this when the class hashes are computed correctly accross the entire state
            // let expected =
            //     contract_class.class_hash().map_err(|e| ConvertClassError::ComputeClassHashError(e.to_string()))?;
            // if class_hash != expected {
            //     log::warn!("Mismatched class hash: 0x{:x}", class_update.class_hash);
            //     // return Err(ConvertClassError::MismatchedClassHash { expected, got: class_hash });
            // }

            let compiled_class =
                contract_class.compile().map_err(|e| ConvertClassError::CompilationClassError(e.to_string()))?;

            let class_info =
                ClassInfo { contract_class: contract_class.into(), block_number: block_n, compiled_class_hash };

            Ok(ConvertedClass { class_infos: (class_hash, class_info), class_compiled: (class_hash, compiled_class) })
        })
        .collect::<Result<Vec<_>, _>>()
}
