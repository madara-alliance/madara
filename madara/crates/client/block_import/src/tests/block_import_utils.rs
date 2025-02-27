use mp_block::header::{BlockTimestamp, GasPrices, L1DataAvailabilityMode};
use mp_block::Header;
use mp_chain_config::StarknetVersion;
use mp_state_update::StateDiff;
use starknet_types_core::felt::Felt;

use crate::{
    BlockValidationContext, PreValidatedBlock, PreValidatedPendingBlock, UnverifiedCommitments, UnverifiedFullBlock,
    UnverifiedHeader, ValidatedCommitments,
};
use starknet_api::{core::ChainId, felt};

/// Creates a dummy UnverifiedHeader for testing purposes.
///
/// This function generates an UnverifiedHeader with predefined values,
/// useful for creating consistent test scenarios.
pub fn create_dummy_unverified_header() -> UnverifiedHeader {
    UnverifiedHeader {
        parent_block_hash: Some(felt!("0x1")),
        sequencer_address: felt!("0x2"),
        block_timestamp: BlockTimestamp(12345),
        protocol_version: StarknetVersion::new(0, 13, 2, 0),
        l1_gas_price: GasPrices {
            eth_l1_gas_price: 14,
            strk_l1_gas_price: 15,
            eth_l1_data_gas_price: 16,
            strk_l1_data_gas_price: 17,
        },
        l1_da_mode: L1DataAvailabilityMode::Blob,
    }
}

/// Creates dummy ValidatedCommitments for testing purposes.
///
/// This function generates ValidatedCommitments with predefined values,
/// useful for creating consistent test scenarios.
pub fn create_dummy_commitments() -> ValidatedCommitments {
    ValidatedCommitments {
        transaction_count: 1,
        transaction_commitment: felt!("0x6"),
        event_count: 2,
        event_commitment: felt!("0x7"),
        state_diff_length: 3,
        state_diff_commitment: felt!("0x8"),
        receipt_commitment: felt!("0x9"),
    }
}

/// Creates a BlockValidationContext for testing purposes.
///
/// This function generates a BlockValidationContext with customizable
/// ignore_block_order flag, useful for testing different validation scenarios.
pub fn create_validation_context(ignore_block_order: bool) -> BlockValidationContext {
    BlockValidationContext {
        chain_id: ChainId::Other("something".to_string()),
        ignore_block_order,
        trust_global_tries: false,
        trust_transaction_hashes: false,
        trust_class_hashes: false,
    }
}

/// Creates a dummy Header for testing purposes.
///
/// This function generates a Header with predefined values,
/// useful for creating consistent test scenarios.
pub fn create_dummy_header() -> Header {
    Header {
        parent_block_hash: felt!("0x1"),
        sequencer_address: felt!("0x2"),
        block_number: 1,
        global_state_root: felt!("0xa"),
        transaction_count: 0,
        transaction_commitment: felt!("0x0"),
        event_count: 0,
        event_commitment: felt!("0x0"),
        state_diff_length: Some(0),
        state_diff_commitment: Some(felt!("0x0")),
        receipt_commitment: Some(felt!("0x0")),
        block_timestamp: BlockTimestamp(12345),
        protocol_version: StarknetVersion::new(0, 13, 2, 0),
        l1_gas_price: GasPrices {
            eth_l1_gas_price: 14,
            strk_l1_gas_price: 15,
            eth_l1_data_gas_price: 16,
            strk_l1_data_gas_price: 17,
        },
        l1_da_mode: L1DataAvailabilityMode::Blob,
    }
}

/// Creates a dummy PreValidatedBlock for testing purposes.
///
/// This function generates a PreValidatedBlock with predefined values,
/// useful for testing update_tries scenarios.
pub fn create_dummy_block() -> PreValidatedBlock {
    PreValidatedBlock {
        header: create_dummy_unverified_header(),
        unverified_block_hash: None,
        unverified_block_number: Some(1),
        unverified_global_state_root: Some(felt!("0xa")),
        commitments: create_dummy_commitments(),
        transactions: vec![],
        receipts: vec![],
        state_diff: StateDiff::default(),
        converted_classes: Default::default(),
    }
}

/// Creates a dummy PreValidatedBlock for testing purposes.
///
/// This function generates a PreValidatedBlock with predefined values,
/// useful for testing update_tries scenarios.
pub fn create_dummy_unverified_full_block() -> UnverifiedFullBlock {
    UnverifiedFullBlock {
        header: UnverifiedHeader {
            parent_block_hash: Some(Felt::ZERO),
            sequencer_address: Felt::ZERO,
            block_timestamp: BlockTimestamp(0),
            protocol_version: StarknetVersion::default(),
            l1_gas_price: GasPrices::default(),
            l1_da_mode: L1DataAvailabilityMode::Blob,
        },
        transactions: vec![],
        unverified_block_number: Some(0),
        state_diff: StateDiff::default(),
        receipts: vec![],
        declared_classes: vec![],
        commitments: UnverifiedCommitments::default(),
        trusted_converted_classes: vec![],
    }
}

/// Creates a dummy PreValidatedPendingBlock for testing purposes.
pub fn create_dummy_pending_block() -> PreValidatedPendingBlock {
    PreValidatedPendingBlock {
        header: UnverifiedHeader {
            parent_block_hash: Some(felt!("0x1")),
            sequencer_address: felt!("0x2"),
            block_timestamp: BlockTimestamp(12345),
            protocol_version: StarknetVersion::new(0, 13, 2, 0),
            l1_gas_price: GasPrices {
                eth_l1_gas_price: 14,
                strk_l1_gas_price: 15,
                eth_l1_data_gas_price: 16,
                strk_l1_data_gas_price: 17,
            },
            l1_da_mode: L1DataAvailabilityMode::Blob,
        },
        transactions: vec![],
        receipts: vec![],
        state_diff: StateDiff::default(),
        converted_classes: Default::default(),
    }
}
