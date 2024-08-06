mod common;
use common::temp_db;
use dc_db::{block_db::TxIndex, db_block_id::DbBlockId};
use dp_block::{
    chain_config::ChainConfig, header::PendingHeader, DeoxysBlockInfo, DeoxysBlockInner, DeoxysMaybePendingBlock, DeoxysPendingBlockInfo, Header
};
use dp_receipt::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, InvokeTransactionReceipt,
    L1HandlerTransactionReceipt,
};
use dp_state_update::StateDiff;
use dp_transactions::{
    DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeployAccountTransactionV1,
    DeployAccountTransactionV3, DeployTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction,
};
use starknet_core::types::Felt;

#[tokio::test]
async fn test_chain_info() {
    let db = temp_db().await;
    let chain_config = db.backend().chain_config();
    assert_eq!(chain_config.chain_id, ChainConfig::test_config().chain_id);
}

#[tokio::test]
async fn test_store_block() {
    const BLOCK_ID_0: DbBlockId = DbBlockId::BlockN(0);

    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.get_block(&BLOCK_ID_0).unwrap().is_none());

    let block = block_zero();
    let state_diff = state_diff_zero();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

    assert_eq!(backend.get_block_hash(&BLOCK_ID_0).unwrap().unwrap(), block.info.block_hash().unwrap());
    assert_eq!(backend.get_block_info(&BLOCK_ID_0).unwrap().unwrap(), block.info);
    assert_eq!(backend.get_block_inner(&BLOCK_ID_0).unwrap().unwrap(), block.inner);
    assert_eq!(backend.get_block(&BLOCK_ID_0).unwrap().unwrap(), block);
    assert_eq!(backend.get_block_state_diff(&BLOCK_ID_0).unwrap().unwrap(), state_diff);
}

#[tokio::test]
async fn test_store_pending_block() {
    const BLOCK_ID_PENDING: DbBlockId = DbBlockId::Pending;

    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.get_block(&BLOCK_ID_PENDING).unwrap().is_none());

    let block = block_pending();
    let state_diff = state_diff_pending();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

    assert!(backend.get_block_hash(&BLOCK_ID_PENDING).unwrap().is_none());
    assert_eq!(backend.get_block_info(&BLOCK_ID_PENDING).unwrap().unwrap(), block.info);
    assert_eq!(backend.get_block_inner(&BLOCK_ID_PENDING).unwrap().unwrap(), block.inner);
    assert_eq!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap(), block);
    assert_eq!(backend.get_block_state_diff(&BLOCK_ID_PENDING).unwrap().unwrap(), state_diff);
}

#[tokio::test]
async fn test_store_block_transactions() {
    let db = temp_db().await;
    let backend = db.backend();

    let block = block_zero();
    let state_diff = state_diff_zero();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();
    backend.store_block(block_one(), state_diff_one(), vec![]).unwrap();

    let tx_hash_1 = block.info.tx_hashes()[1];
    assert_eq!(backend.find_tx_hash_block_info(&tx_hash_1).unwrap().unwrap(), (block.info.clone(), TxIndex(1)));
    assert_eq!(backend.find_tx_hash_block(&tx_hash_1).unwrap().unwrap(), (block, TxIndex(1)));
}

fn block_zero() -> DeoxysMaybePendingBlock {
    let transactions = vec![
        InvokeTransactionV0::default().into(),
        L1HandlerTransaction::default().into(),
        DeclareTransactionV0::default().into(),
        DeployTransaction::default().into(),
        DeployAccountTransactionV1::default().into(),
    ];

    let transaction_receipts = vec![
        InvokeTransactionReceipt::default().into(),
        L1HandlerTransactionReceipt::default().into(),
        DeclareTransactionReceipt::default().into(),
        DeployTransactionReceipt::default().into(),
        DeployAccountTransactionReceipt::default().into(),
    ];

    let block_inner = DeoxysBlockInner::new(transactions, transaction_receipts);

    let tx_hashes = vec![Felt::from(0), Felt::from(1), Felt::from(2), Felt::from(3), Felt::from(4)];
    let block_info = DeoxysBlockInfo::new(Header::default(), tx_hashes, Felt::from(0));

    DeoxysMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn state_diff_zero() -> StateDiff {
    StateDiff::default()
}

fn block_one() -> DeoxysMaybePendingBlock {
    let transactions = vec![
        InvokeTransactionV1::default().into(),
        L1HandlerTransaction::default().into(),
        DeclareTransactionV1::default().into(),
        DeployTransaction::default().into(),
        DeployAccountTransactionV3::default().into(),
    ];

    let transaction_receipts = vec![
        InvokeTransactionReceipt::default().into(),
        L1HandlerTransactionReceipt::default().into(),
        DeclareTransactionReceipt::default().into(),
        DeployTransactionReceipt::default().into(),
        DeployAccountTransactionReceipt::default().into(),
    ];

    let block_inner = DeoxysBlockInner::new(transactions, transaction_receipts);

    let tx_hashes = vec![Felt::from(10), Felt::from(11), Felt::from(12), Felt::from(13), Felt::from(14)];
    let block_info = DeoxysBlockInfo::new(Header::default(), tx_hashes, Felt::from(1));

    DeoxysMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn state_diff_one() -> StateDiff {
    StateDiff::default()
}

fn block_pending() -> DeoxysMaybePendingBlock {
    let transactions = vec![
        InvokeTransactionV3::default().into(),
        L1HandlerTransaction::default().into(),
        DeclareTransactionV2::default().into(),
        DeployTransaction::default().into(),
        DeployAccountTransactionV3::default().into(),
    ];

    let transaction_receipts = vec![
        InvokeTransactionReceipt::default().into(),
        L1HandlerTransactionReceipt::default().into(),
        DeclareTransactionReceipt::default().into(),
        DeployTransactionReceipt::default().into(),
        DeployAccountTransactionReceipt::default().into(),
    ];

    let block_inner = DeoxysBlockInner::new(transactions, transaction_receipts);

    let tx_hashes = vec![Felt::from(20), Felt::from(21), Felt::from(22), Felt::from(23), Felt::from(24)];
    let block_info = DeoxysPendingBlockInfo::new(PendingHeader::default(), tx_hashes);

    DeoxysMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn state_diff_pending() -> StateDiff {
    StateDiff::default()
}
