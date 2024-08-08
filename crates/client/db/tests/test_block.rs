mod common;
use common::temp_db;
use dc_db::db_block_id::DbBlockIdResolvable;
use dc_db::{block_db::TxIndex, db_block_id::DbBlockId};
use dp_block::BlockId;
use dp_block::{
    chain_config::ChainConfig, header::PendingHeader, DeoxysBlockInfo, DeoxysBlockInner, DeoxysMaybePendingBlock,
    DeoxysPendingBlockInfo, Header,
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
use starknet_types_core::felt::Felt;

#[tokio::test]
async fn test_chain_info() {
    let db = temp_db().await;
    let chain_config = db.backend().chain_config();
    assert_eq!(chain_config.chain_id, ChainConfig::test_config().chain_id);
}

#[tokio::test]
async fn test_block_id() {
    let db = temp_db().await;
    let backend = db.backend();

    let block = block_zero();
    let block_hash = block.info.block_hash().unwrap();
    let state_diff = state_diff_zero();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();
    backend.store_block(block_pending_one(), state_diff_pending_one(), vec![]).unwrap();

    assert_eq!(backend.resolve_block_id(&BlockId::Hash(block_hash)).unwrap().unwrap(), DbBlockId::BlockN(0));
    assert_eq!(backend.resolve_block_id(&BlockId::Number(0)).unwrap().unwrap(), DbBlockId::BlockN(0));
    assert_eq!(backend.resolve_block_id(&DbBlockId::Pending).unwrap().unwrap(), DbBlockId::Pending);
}

#[tokio::test]
async fn test_block_id_not_found() {
    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.resolve_block_id(&BlockId::Hash(Felt::from(0))).unwrap().is_none());
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
    assert_eq!(BLOCK_ID_0.resolve_db_block_id(backend).unwrap().unwrap(), BLOCK_ID_0);
    assert_eq!(backend.get_block_info(&BLOCK_ID_0).unwrap().unwrap(), block.info);
    assert_eq!(backend.get_block_inner(&BLOCK_ID_0).unwrap().unwrap(), block.inner);
    assert_eq!(backend.get_block(&BLOCK_ID_0).unwrap().unwrap(), block);
    assert_eq!(backend.get_block_n(&BLOCK_ID_0).unwrap().unwrap(), 0);
    assert_eq!(backend.get_block_state_diff(&BLOCK_ID_0).unwrap().unwrap(), state_diff);
}

#[tokio::test]
async fn test_store_pending_block() {
    const BLOCK_ID_PENDING: DbBlockId = DbBlockId::Pending;

    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.get_block(&BLOCK_ID_PENDING).unwrap().is_none());

    let block = block_pending_one();
    let state_diff = state_diff_pending_one();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

    assert!(backend.get_block_hash(&BLOCK_ID_PENDING).unwrap().is_none());
    assert_eq!(backend.get_block_info(&BLOCK_ID_PENDING).unwrap().unwrap(), block.info);
    assert_eq!(backend.get_block_inner(&BLOCK_ID_PENDING).unwrap().unwrap(), block.inner);
    assert_eq!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap(), block);
    assert_eq!(backend.get_block_state_diff(&BLOCK_ID_PENDING).unwrap().unwrap(), state_diff);
}

#[tokio::test]
async fn test_erase_pending_block() {
    const BLOCK_ID_PENDING: DbBlockId = DbBlockId::Pending;

    let db = temp_db().await;
    let backend = db.backend();

    backend.store_block(block_zero(), state_diff_zero(), vec![]).unwrap();
    backend.store_block(block_pending_one(), state_diff_pending_one(), vec![]).unwrap();
    backend.clear_pending_block().unwrap();

    assert!(backend.get_block(&BLOCK_ID_PENDING).unwrap().is_none());

    backend.store_block(block_one(), state_diff_one(), vec![]).unwrap();

    let block = block_pending_two();
    let state_diff = state_diff_pending_two();
    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

    assert!(backend.get_block_hash(&BLOCK_ID_PENDING).unwrap().is_none());
    assert_eq!(backend.get_block_info(&BLOCK_ID_PENDING).unwrap().unwrap(), block.info);
    assert_eq!(backend.get_block_inner(&BLOCK_ID_PENDING).unwrap().unwrap(), block.inner);
    assert_eq!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap(), block);
    assert_eq!(backend.get_block_state_diff(&BLOCK_ID_PENDING).unwrap().unwrap(), state_diff);
}

#[tokio::test]
async fn test_store_latest_block() {
    let db = temp_db().await;
    let backend = db.backend();

    backend.store_block(block_zero(), state_diff_zero(), vec![]).unwrap();

    let latest_block = block_one();
    backend.store_block(latest_block.clone(), state_diff_one(), vec![]).unwrap();

    assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
}

#[tokio::test]
async fn test_latest_confirmed_block() {
    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.get_l1_last_confirmed_block().unwrap().is_none());

    backend.write_last_confirmed_block(0).unwrap();

    assert_eq!(backend.get_l1_last_confirmed_block().unwrap().unwrap(), 0);
}

#[tokio::test]
async fn test_store_block_transactions() {
    let db = temp_db().await;
    let backend = db.backend();

    let block = block_zero();
    let state_diff = state_diff_zero();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

    let tx_hash_1 = block.info.tx_hashes()[1];
    assert_eq!(backend.find_tx_hash_block_info(&tx_hash_1).unwrap().unwrap(), (block.info.clone(), TxIndex(1)));
    assert_eq!(backend.find_tx_hash_block(&tx_hash_1).unwrap().unwrap(), (block, TxIndex(1)));
}

#[tokio::test]
async fn test_store_block_transactions_pending() {
    let db = temp_db().await;
    let backend = db.backend();

    backend.store_block(block_zero(), state_diff_zero(), vec![]).unwrap();

    let block_pending = block_pending_one();
    backend.store_block(block_pending.clone(), state_diff_pending_one(), vec![]).unwrap();

    let tx_hash_1 = block_pending.info.tx_hashes()[1];
    assert_eq!(backend.find_tx_hash_block_info(&tx_hash_1).unwrap().unwrap(), (block_pending.info.clone(), TxIndex(1)));
    assert_eq!(backend.find_tx_hash_block(&tx_hash_1).unwrap().unwrap(), (block_pending, TxIndex(1)));
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
    let header = Header { block_number: 1, ..Default::default() };
    let block_info = DeoxysBlockInfo::new(header, tx_hashes, Felt::from(1));

    DeoxysMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn state_diff_one() -> StateDiff {
    StateDiff::default()
}

fn block_pending_one() -> DeoxysMaybePendingBlock {
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

fn state_diff_pending_one() -> StateDiff {
    StateDiff::default()
}

fn block_pending_two() -> DeoxysMaybePendingBlock {
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
    let header = PendingHeader { parent_block_hash: Felt::from(1), ..Default::default() };
    let block_info = DeoxysPendingBlockInfo::new(header, tx_hashes);

    DeoxysMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn state_diff_pending_two() -> StateDiff {
    StateDiff::default()
}
