use assert_matches::assert_matches;
use mp_receipt::{ExecutionResources, FeePayment, TransactionReceipt};
use mp_transactions::Transaction;
use std::sync::Arc;

use mc_block_import::{
    BlockImportResult, BlockImporter, BlockValidationContext, PendingBlockImportResult, UnverifiedFullBlock,
    UnverifiedHeader, UnverifiedPendingFullBlock,
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mp_block::{
    header::{GasPrices, PendingHeader},
    Header, MadaraMaybePendingBlockInfo,
};
use mp_chain_config::{ChainConfig, StarknetVersion};
use mp_state_update::StateDiff;
use starknet_core::types::Felt;

#[tokio::test]
async fn import_one_empty_block_full() {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());
    let block_importer = BlockImporter::new(backend.clone());

    let block = UnverifiedFullBlock {
        unverified_block_number: None,
        header: UnverifiedHeader {
            parent_block_hash: None,
            sequencer_address: Felt::ONE,
            block_timestamp: 12345,
            protocol_version: StarknetVersion::LATEST,
            l1_gas_price: GasPrices::default(),
            l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
        },
        state_diff: StateDiff::default(),
        transactions: vec![],
        receipts: vec![],
        declared_classes: vec![],
        commitments: Default::default(),
    };

    let validation = BlockValidationContext {
        trust_transaction_hashes: false,
        chain_id: chain_config.chain_id.clone(),
        trust_global_tries: false,
        trust_class_hashes: false,
    };

    let pre_validated_block =
        block_importer.pre_validate(block, validation.clone()).await.expect("pre_validate failed");
    let block_import_result =
        block_importer.verify_apply(pre_validated_block, validation).await.expect("verify_apply failed");

    let expected_header = Header {
        parent_block_hash: Felt::ZERO,
        block_number: 0,
        global_state_root: Felt::ZERO,
        sequencer_address: Felt::ONE,
        block_timestamp: 12345,
        transaction_count: 0,
        transaction_commitment: Felt::ZERO,
        event_count: 0,
        event_commitment: Felt::ZERO,
        state_diff_length: 0,
        state_diff_commitment: Felt::from_hex_unchecked(
            "0x49973925542c74a9d9ff0efaa98c61e1225d0aedb708092433cbbb20836d30a",
        ),
        receipt_commitment: Felt::ZERO,
        protocol_version: StarknetVersion::LATEST,
        l1_gas_price: GasPrices::default(),
        l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
    };

    // TODO: check values
    let expected_block_import_result = BlockImportResult {
        header: expected_header.clone(),
        block_hash: Felt::from_hex_unchecked("0x25633210372dce37323bbee7443e7388e8637e33a0e8376f214dc376e88fa79"),
    };

    // Check backend update
    let block = backend.get_block(&DbBlockId::BlockN(0)).expect("get_block failed");
    assert!(block.is_some());
    assert_matches!(block.clone().unwrap().info, MadaraMaybePendingBlockInfo::NotPending(_));
    assert_eq!(block.clone().unwrap().info.tx_hashes().len(), 0);
    assert_eq!(block.unwrap().info.as_nonpending().unwrap().header, expected_header);

    assert_eq!(block_import_result, expected_block_import_result);
}

#[tokio::test]
async fn import_one_empty_block_pending() {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());
    let block_importer = BlockImporter::new(backend.clone());

    let block = UnverifiedPendingFullBlock {
        header: UnverifiedHeader {
            parent_block_hash: None,
            sequencer_address: Felt::ONE,
            block_timestamp: 12345,
            protocol_version: StarknetVersion::LATEST,
            l1_gas_price: GasPrices::default(),
            l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
        },
        state_diff: StateDiff::default(),
        transactions: vec![],
        receipts: vec![],
        declared_classes: vec![],
    };

    let validation = BlockValidationContext {
        trust_transaction_hashes: false,
        chain_id: chain_config.chain_id.clone(),
        trust_global_tries: false,
        trust_class_hashes: false,
    };

    let pre_validated_block =
        block_importer.pre_validate_pending(block, validation.clone()).await.expect("pre_validate_pending failed");
    let block_import_result = block_importer
        .verify_apply_pending(pre_validated_block, validation)
        .await
        .expect("verify_apply_pending failed");

    // TODO: update once it has values
    let expected_block_import_result = PendingBlockImportResult {};

    // Check backend update
    let block = backend.get_block(&DbBlockId::Pending).expect("get_block failed");
    assert!(block.is_some());
    assert_matches!(block.clone().unwrap().info, MadaraMaybePendingBlockInfo::Pending(_));
    assert_eq!(block.clone().unwrap().info.tx_hashes().len(), 0);
    assert_eq!(
        block.unwrap().info.as_pending().unwrap().header,
        PendingHeader {
            parent_block_hash: Felt::ZERO,
            sequencer_address: Felt::ONE,
            block_timestamp: 12345,
            protocol_version: StarknetVersion::LATEST,
            l1_gas_price: GasPrices::default(),
            l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
        }
    );

    assert_eq!(block_import_result, expected_block_import_result);
}

#[tokio::test]
async fn import_block_with_txs() {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());
    let block_importer = BlockImporter::new(backend.clone());

    let block = get_unverified_full_block();

    let validation = BlockValidationContext {
        trust_transaction_hashes: false,
        chain_id: chain_config.chain_id.clone(),
        trust_global_tries: false,
        trust_class_hashes: false,
    };

    let pre_validated_block =
        block_importer.pre_validate(block, validation.clone()).await.expect("pre_validate failed");
    let block_import_result =
        block_importer.verify_apply(pre_validated_block, validation).await.expect("verify_apply failed");

    let expected_header = Header {
        parent_block_hash: Felt::ZERO,
        block_number: 0,
        global_state_root: Felt::ZERO,
        sequencer_address: Felt::ONE,
        block_timestamp: 12345,
        transaction_count: 1,
        transaction_commitment: Felt::from_hex_unchecked(
            "0x1e8a951b99c806a8e7873eab2583e786424bd6a4d9ad614be1f03e21d48a0e8",
        ),
        event_count: 0,
        event_commitment: Felt::ZERO,
        state_diff_length: 0,
        state_diff_commitment: Felt::from_hex_unchecked(
            "0x49973925542c74a9d9ff0efaa98c61e1225d0aedb708092433cbbb20836d30a",
        ),
        receipt_commitment: Felt::from_hex_unchecked(
            "0x2879fdc4d9521339b5015d67494aa183352f76d4a441f64bbde534f14c8be3a",
        ),
        protocol_version: StarknetVersion::LATEST,
        l1_gas_price: GasPrices::default(),
        l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
    };

    // TODO: check values
    let expected_block_import_result = BlockImportResult {
        header: expected_header.clone(),
        block_hash: Felt::from_hex_unchecked("0x103d1b1c094127c8a82756f912877dae9df868df1a8a96f40b975a6281b3fac"),
    };

    // Check backend update
    let block = backend.get_block(&DbBlockId::BlockN(0)).expect("get_block failed");
    assert!(block.is_some());
    assert_matches!(block.clone().unwrap().info, MadaraMaybePendingBlockInfo::NotPending(_));
    assert_eq!(block.clone().unwrap().info.tx_hashes().len(), 1);
    assert_eq!(block.unwrap().info.as_nonpending().unwrap().header, expected_header);

    assert_eq!(block_import_result, expected_block_import_result);
}

#[tokio::test]
async fn import_multiple_blocks_with_txs() {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());
    let block_importer = BlockImporter::new(backend.clone());

    let block = get_unverified_full_block();

    let validation = BlockValidationContext {
        trust_transaction_hashes: false,
        chain_id: chain_config.chain_id.clone(),
        trust_global_tries: false,
        trust_class_hashes: false,
    };

    let pre_validated_block =
        block_importer.pre_validate(block, validation.clone()).await.expect("pre_validate failed");
    let _block_import_result =
        block_importer.verify_apply(pre_validated_block, validation.clone()).await.expect("verify_apply failed");

    let another_block = get_unverified_full_block();

    let pre_validated_block =
        block_importer.pre_validate(another_block, validation.clone()).await.expect("pre_validate failed");
    let block_import_result =
        block_importer.verify_apply(pre_validated_block, validation).await.expect("verify_apply failed");

    let expected_header = Header {
        parent_block_hash: Felt::from_hex_unchecked(
            "0x103d1b1c094127c8a82756f912877dae9df868df1a8a96f40b975a6281b3fac",
        ),
        block_number: 1,
        global_state_root: Felt::ZERO,
        sequencer_address: Felt::ONE,
        block_timestamp: 12345,
        transaction_count: 1,
        transaction_commitment: Felt::from_hex_unchecked(
            "0x1e8a951b99c806a8e7873eab2583e786424bd6a4d9ad614be1f03e21d48a0e8",
        ),
        event_count: 0,
        event_commitment: Felt::ZERO,
        state_diff_length: 0,
        state_diff_commitment: Felt::from_hex_unchecked(
            "0x49973925542c74a9d9ff0efaa98c61e1225d0aedb708092433cbbb20836d30a",
        ),
        receipt_commitment: Felt::from_hex_unchecked(
            "0x2879fdc4d9521339b5015d67494aa183352f76d4a441f64bbde534f14c8be3a",
        ),
        protocol_version: StarknetVersion::LATEST,
        l1_gas_price: GasPrices::default(),
        l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
    };

    // TODO: check values
    let expected_block_import_result = BlockImportResult {
        header: expected_header.clone(),
        block_hash: Felt::from_hex_unchecked("0x3f0077f0cb907794d82a7766776d64b68d671570bca5f8eaefabd634e9f2e6c"),
    };

    assert_eq!(block_import_result, expected_block_import_result);

    let block = backend.get_block(&DbBlockId::BlockN(0)).expect("get_block failed");
    assert!(block.is_some());
}

fn get_unverified_full_block() -> UnverifiedFullBlock {
    UnverifiedFullBlock {
        unverified_block_number: None,
        header: UnverifiedHeader {
            parent_block_hash: None,
            sequencer_address: Felt::ONE,
            block_timestamp: 12345,
            protocol_version: StarknetVersion::LATEST,
            l1_gas_price: GasPrices::default(),
            l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
        },
        state_diff: StateDiff::default(),
        transactions: vec![Transaction::Invoke(mp_transactions::InvokeTransaction::V1(
            mp_transactions::InvokeTransactionV1 {
                sender_address: Felt::ONE,
                calldata: vec![],
                max_fee: Felt::MAX,
                signature: vec![],
                nonce: Felt::ZERO,
            },
        ))],
        receipts: vec![TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
            transaction_hash: Felt::from_hex_unchecked(
                "0x4cab3e98facc3c319e88dc594e3b16324106609179275c14d499d95d96f666e",
            ),
            actual_fee: FeePayment::default(),
            messages_sent: vec![],
            events: vec![],
            execution_resources: ExecutionResources::default(),
            execution_result: mp_receipt::ExecutionResult::default(),
        })],
        declared_classes: vec![],
        commitments: Default::default(),
    }
}
