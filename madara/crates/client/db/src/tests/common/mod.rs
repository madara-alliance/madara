use mp_block::header::PreconfirmedHeader;
use mp_block::{Header, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock, MadaraPreconfirmedBlockInfo};
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
use starknet_api::felt;
use starknet_types_core::felt::Felt;

pub mod temp_db {
    use crate::DatabaseService;
    use mp_chain_config::ChainConfig;
    pub async fn temp_db() -> DatabaseService {
        let chain_config = std::sync::Arc::new(ChainConfig::madara_test());
        DatabaseService::open_for_testing(chain_config)
    }
}

pub fn finalized_block_zero(header: Header) -> MadaraMaybePendingBlock {
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
    let block_info = MadaraBlockInfo::new(header, tx_hashes, felt!("0x12345"));

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

pub fn finalized_state_diff_zero() -> StateDiff {
    StateDiff::default()
}

pub fn finalized_block_one() -> MadaraMaybePendingBlock {
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

pub fn finalized_state_diff_one() -> StateDiff {
    StateDiff::default()
}

pub fn pending_block_one() -> MadaraMaybePendingBlock {
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
    let block_info = MadaraPreconfirmedBlockInfo::new(PreconfirmedHeader::default(), tx_hashes);

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

pub fn pending_state_diff_one() -> StateDiff {
    StateDiff::default()
}

pub fn pending_block_two() -> MadaraMaybePendingBlock {
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
    let header = PreconfirmedHeader { parent_block_hash: Felt::from(1), ..Default::default() };
    let block_info = MadaraPreconfirmedBlockInfo::new(header, tx_hashes);

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

pub fn pending_state_diff_two() -> StateDiff {
    StateDiff::default()
}
