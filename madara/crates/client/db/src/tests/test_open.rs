#![cfg(test)]
use crate::{rocksdb::RocksDBConfig, MadaraBackend, MadaraBackendConfig};
use mc_class_exec::config::NativeConfig;
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
use mp_chain_config::ChainConfig;
use mp_receipt::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, InvokeTransactionReceipt,
    L1HandlerTransactionReceipt,
};
use mp_transactions::{
    DeclareTransactionV0, DeployAccountTransactionV1, DeployTransaction, InvokeTransactionV0, L1HandlerTransaction,
};
use std::sync::Arc;

pub fn dummy_block(header: PreconfirmedHeader) -> FullBlockWithoutCommitments {
    FullBlockWithoutCommitments {
        header,
        state_diff: Default::default(),
        transactions: vec![
            TransactionWithReceipt {
                transaction: InvokeTransactionV0::default().into(),
                receipt: InvokeTransactionReceipt::default().into(),
            },
            TransactionWithReceipt {
                transaction: L1HandlerTransaction::default().into(),
                receipt: L1HandlerTransactionReceipt::default().into(),
            },
            TransactionWithReceipt {
                transaction: DeclareTransactionV0::default().into(),
                receipt: DeclareTransactionReceipt::default().into(),
            },
            TransactionWithReceipt {
                transaction: DeployTransaction::default().into(),
                receipt: DeployTransactionReceipt::default().into(),
            },
            TransactionWithReceipt {
                transaction: DeployAccountTransactionV1::default().into(),
                receipt: DeployAccountTransactionReceipt::default().into(),
            },
        ],
        events: vec![],
    }
}

#[tokio::test]
async fn test_open_different_chain_id() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    {
        let chain_config = std::sync::Arc::new(ChainConfig::starknet_integration());
        let cairo_native_config = Arc::new(NativeConfig::default());
        let _db = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            chain_config,
            MadaraBackendConfig::default(),
            RocksDBConfig::default(),
            cairo_native_config,
        )
        .unwrap();
    }
    let chain_config = std::sync::Arc::new(ChainConfig::madara_test());
    let cairo_native_config = Arc::new(NativeConfig::default());
    assert!(MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        MadaraBackendConfig::default(),
        RocksDBConfig::default(),
        cairo_native_config,
    )
    .is_err());
}
#[tokio::test]
async fn test_chain_info() {
    let db = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let chain_config = db.chain_config();
    assert_eq!(chain_config.chain_id, ChainConfig::madara_test().chain_id);
}
