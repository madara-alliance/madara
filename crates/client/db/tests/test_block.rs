mod common;

use common::*;
use mc_db::db_block_id::DbBlockIdResolvable;
use mc_db::{block_db::TxIndex, db_block_id::DbBlockId};
use mp_block::BlockId;
use mp_block::{
    header::PendingHeader, Header, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock, MadaraPendingBlockInfo,
};
use mp_chain_config::ChainConfig;
use mp_receipt::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, InvokeTransactionReceipt,
    L1HandlerTransactionReceipt,
};
use mp_state_update::StateDiff;
use mp_transactions::{
    DeclareTransactionV0, DeclareTransactionV1, DeclareTransactionV2, DeployAccountTransactionV1,
    DeployAccountTransactionV3, DeployTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3,
    L1HandlerTransaction,
};
use mp_utils::tests_common::*;
use rstest::*;
use starknet_types_core::felt::Felt;

#[rstest]
#[tokio::test]
async fn test_chain_info(_set_workdir: ()) {
    let db = temp_db().await;
    let chain_config = db.backend().chain_config();
    assert_eq!(chain_config.chain_id, ChainConfig::test_config().unwrap().chain_id);
}

#[rstest]
#[tokio::test]
async fn test_block_id(_set_workdir: ()) {
    let db = temp_db().await;
    let backend = db.backend();

    let block = finalized_block_zero();
    let block_hash = block.info.block_hash().unwrap();
    let state_diff = finalized_state_diff_zero();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();
    backend.store_block(pending_block_one(), pending_state_diff_one(), vec![]).unwrap();

    assert_eq!(backend.resolve_block_id(&BlockId::Hash(block_hash)).unwrap().unwrap(), DbBlockId::BlockN(0));
    assert_eq!(backend.resolve_block_id(&BlockId::Number(0)).unwrap().unwrap(), DbBlockId::BlockN(0));
    assert_eq!(backend.resolve_block_id(&DbBlockId::Pending).unwrap().unwrap(), DbBlockId::Pending);
}

#[rstest]
#[tokio::test]
async fn test_block_id_not_found(_set_workdir: ()) {
    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.resolve_block_id(&BlockId::Hash(Felt::from(0))).unwrap().is_none());
}

#[rstest]
#[tokio::test]
async fn test_store_block(_set_workdir: ()) {
    const BLOCK_ID_0: DbBlockId = DbBlockId::BlockN(0);

    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.get_block(&BLOCK_ID_0).unwrap().is_none());

    let block = finalized_block_zero();
    let state_diff = finalized_state_diff_zero();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

    assert_eq!(backend.get_block_hash(&BLOCK_ID_0).unwrap().unwrap(), block.info.block_hash().unwrap());
    assert_eq!(BLOCK_ID_0.resolve_db_block_id(backend).unwrap().unwrap(), BLOCK_ID_0);
    assert_eq!(backend.get_block_info(&BLOCK_ID_0).unwrap().unwrap(), block.info);
    assert_eq!(backend.get_block_inner(&BLOCK_ID_0).unwrap().unwrap(), block.inner);
    assert_eq!(backend.get_block(&BLOCK_ID_0).unwrap().unwrap(), block);
    assert_eq!(backend.get_block_n(&BLOCK_ID_0).unwrap().unwrap(), 0);
    assert_eq!(backend.get_block_state_diff(&BLOCK_ID_0).unwrap().unwrap(), state_diff);
}

#[rstest]
#[tokio::test]
async fn test_store_pending_block(_set_workdir: ()) {
    const BLOCK_ID_PENDING: DbBlockId = DbBlockId::Pending;

    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.get_block(&BLOCK_ID_PENDING).unwrap().is_some()); // Pending block should always be there

    let block = pending_block_one();
    let state_diff = pending_state_diff_one();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

    assert!(backend.get_block_hash(&BLOCK_ID_PENDING).unwrap().is_none());
    assert_eq!(backend.get_block_info(&BLOCK_ID_PENDING).unwrap().unwrap(), block.info);
    assert_eq!(backend.get_block_inner(&BLOCK_ID_PENDING).unwrap().unwrap(), block.inner);
    assert_eq!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap(), block);
    assert_eq!(backend.get_block_state_diff(&BLOCK_ID_PENDING).unwrap().unwrap(), state_diff);
}

#[rstest]
#[tokio::test]
async fn test_erase_pending_block(_set_workdir: ()) {
    const BLOCK_ID_PENDING: DbBlockId = DbBlockId::Pending;

    let db = temp_db().await;
    let backend = db.backend();

    backend.store_block(finalized_block_zero(), finalized_state_diff_zero(), vec![]).unwrap();
    backend.store_block(pending_block_one(), pending_state_diff_one(), vec![]).unwrap();
    backend.clear_pending_block().unwrap();

    assert!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap().inner.transactions.is_empty());
    assert!(
        backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap().info.as_pending().unwrap().header.parent_block_hash
            == finalized_block_zero().info.as_nonpending().unwrap().block_hash,
        "fake pending block parent hash must match with latest block in db"
    );

    backend.store_block(finalized_block_one(), finalized_state_diff_one(), vec![]).unwrap();

    let block_pending = pending_block_two();
    let state_diff = pending_state_diff_two();
    backend.store_block(block_pending.clone(), state_diff.clone(), vec![]).unwrap();

    assert!(backend.get_block_hash(&BLOCK_ID_PENDING).unwrap().is_none());
    assert_eq!(backend.get_block_info(&BLOCK_ID_PENDING).unwrap().unwrap(), block_pending.info);
    assert_eq!(backend.get_block_inner(&BLOCK_ID_PENDING).unwrap().unwrap(), block_pending.inner);
    assert_eq!(backend.get_block(&BLOCK_ID_PENDING).unwrap().unwrap(), block_pending);
    assert_eq!(backend.get_block_state_diff(&BLOCK_ID_PENDING).unwrap().unwrap(), state_diff);
}

#[rstest]
#[tokio::test]
async fn test_store_latest_block(_set_workdir: ()) {
    let db = temp_db().await;
    let backend = db.backend();

    backend.store_block(finalized_block_zero(), finalized_state_diff_zero(), vec![]).unwrap();

    let latest_block = finalized_block_one();
    backend.store_block(latest_block.clone(), finalized_state_diff_one(), vec![]).unwrap();

    assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
}

#[rstest]
#[tokio::test]
async fn test_latest_confirmed_block(_set_workdir: ()) {
    let db = temp_db().await;
    let backend = db.backend();

    assert!(backend.get_l1_last_confirmed_block().unwrap().is_none());

    backend.write_last_confirmed_block(0).unwrap();

    assert_eq!(backend.get_l1_last_confirmed_block().unwrap().unwrap(), 0);
}

#[rstest]
#[tokio::test]
async fn test_store_block_transactions(_set_workdir: ()) {
    let db = temp_db().await;
    let backend = db.backend();

    let block = finalized_block_zero();
    let state_diff = finalized_state_diff_zero();

    backend.store_block(block.clone(), state_diff.clone(), vec![]).unwrap();

    let tx_hash_1 = block.info.tx_hashes()[1];
    assert_eq!(backend.find_tx_hash_block_info(&tx_hash_1).unwrap().unwrap(), (block.info.clone(), TxIndex(1)));
    assert_eq!(backend.find_tx_hash_block(&tx_hash_1).unwrap().unwrap(), (block, TxIndex(1)));
}

#[rstest]
#[tokio::test]
async fn test_store_block_transactions_pending(_set_workdir: ()) {
    let db = temp_db().await;
    let backend = db.backend();

    backend.store_block(finalized_block_zero(), finalized_state_diff_zero(), vec![]).unwrap();

    let block_pending = pending_block_one();
    backend.store_block(block_pending.clone(), pending_state_diff_one(), vec![]).unwrap();

    let tx_hash_1 = block_pending.info.tx_hashes()[1];
    assert_eq!(backend.find_tx_hash_block_info(&tx_hash_1).unwrap().unwrap(), (block_pending.info.clone(), TxIndex(1)));
    assert_eq!(backend.find_tx_hash_block(&tx_hash_1).unwrap().unwrap(), (block_pending, TxIndex(1)));
}

fn finalized_block_zero() -> MadaraMaybePendingBlock {
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

    let block_inner = MadaraBlockInner::new(transactions, transaction_receipts);

    let tx_hashes = vec![Felt::from(0), Felt::from(1), Felt::from(2), Felt::from(3), Felt::from(4)];
    let block_info = MadaraBlockInfo::new(Header::default(), tx_hashes, Felt::from(0));

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn finalized_state_diff_zero() -> StateDiff {
    StateDiff::default()
}

fn finalized_block_one() -> MadaraMaybePendingBlock {
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

    let block_inner = MadaraBlockInner::new(transactions, transaction_receipts);

    let tx_hashes = vec![Felt::from(10), Felt::from(11), Felt::from(12), Felt::from(13), Felt::from(14)];
    let header = Header { block_number: 1, ..Default::default() };
    let block_info = MadaraBlockInfo::new(header, tx_hashes, Felt::from(1));

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn finalized_state_diff_one() -> StateDiff {
    StateDiff::default()
}

fn pending_block_one() -> MadaraMaybePendingBlock {
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

    let block_inner = MadaraBlockInner::new(transactions, transaction_receipts);

    let tx_hashes = vec![Felt::from(20), Felt::from(21), Felt::from(22), Felt::from(23), Felt::from(24)];
    let block_info = MadaraPendingBlockInfo::new(PendingHeader::default(), tx_hashes);

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn pending_state_diff_one() -> StateDiff {
    StateDiff::default()
}

fn pending_block_two() -> MadaraMaybePendingBlock {
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

    let block_inner = MadaraBlockInner::new(transactions, transaction_receipts);

    let tx_hashes = vec![Felt::from(20), Felt::from(21), Felt::from(22), Felt::from(23), Felt::from(24)];
    let header = PendingHeader { parent_block_hash: Felt::from(1), ..Default::default() };
    let block_info = MadaraPendingBlockInfo::new(header, tx_hashes);

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

fn pending_state_diff_two() -> StateDiff {
    StateDiff::default()
}
