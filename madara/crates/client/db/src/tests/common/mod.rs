use mp_block::header::PendingHeader;
use mp_block::{Header, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock, MadaraPendingBlockInfo};
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

#[rstest::fixture]
fn transactions() -> Vec<mp_transactions::Transaction> {
    vec![
        InvokeTransactionV0::default().into(),
        L1HandlerTransaction::default().into(),
        DeclareTransactionV0::default().into(),
        DeployTransaction::default().into(),
        DeployAccountTransactionV1::default().into(),
    ]
}

#[rstest::fixture]
fn transaction_receipts() -> Vec<mp_receipt::TransactionReceipt> {
    vec![
        InvokeTransactionReceipt::default().into(),
        L1HandlerTransactionReceipt::default().into(),
        DeclareTransactionReceipt::default().into(),
        DeployTransactionReceipt::default().into(),
        DeployAccountTransactionReceipt::default().into(),
    ]
}

#[rstest::fixture]
pub fn block(
    #[default(0)] block_number: u64,
    transactions: Vec<mp_transactions::Transaction>,
    transaction_receipts: Vec<mp_receipt::TransactionReceipt>,
) -> MadaraMaybePendingBlock {
    let block_i = block_number * 10;
    let block_inner = MadaraBlockInner::new(transactions, transaction_receipts);
    let tx_hashes = vec![
        Felt::from(block_i),
        Felt::from(block_i + 1),
        Felt::from(block_i + 2),
        Felt::from(block_i + 3),
        Felt::from(block_i + 4),
    ];
    let header = Header { block_number, ..Default::default() };
    let block_info = MadaraBlockInfo::new(header, tx_hashes, felt!("0x12345"));

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}

#[rstest::fixture]
pub fn state_diff() -> StateDiff {
    StateDiff::default()
}

#[rstest::fixture]
pub fn block_pending(
    #[allow(unused)]
    #[default(0)]
    parent_block_n: u64,
    #[from(block)]
    #[with(parent_block_n)]
    parent: MadaraMaybePendingBlock,
    transactions: Vec<mp_transactions::Transaction>,
    transaction_receipts: Vec<mp_receipt::TransactionReceipt>,
) -> MadaraMaybePendingBlock {
    let block_inner = MadaraBlockInner::new(transactions, transaction_receipts);

    let tx_hashes = vec![Felt::from(20), Felt::from(21), Felt::from(22), Felt::from(23), Felt::from(24)];
    let block_info = MadaraPendingBlockInfo::new(
        PendingHeader {
            parent_block_hash: parent.info.block_hash().unwrap(),
            parent_block_number: parent.info.block_n(),
            ..Default::default()
        },
        tx_hashes,
    );

    MadaraMaybePendingBlock { info: block_info.into(), inner: block_inner }
}
