use mc_db::MadaraBackend;
use mc_external_db::{
    config::ExternalDbConfig, metrics::ExternalDbMetrics, mongodb::MempoolTransactionDocument, test_utils,
    writer::ExternalDbWorker,
};
use mp_block::FullBlockWithoutCommitments;
use mp_convert::{Felt, FeltHexDisplay};
use mp_receipt::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, ExecutionResult, InvokeTransactionReceipt,
    L1HandlerTransactionReceipt, TransactionReceipt,
};
use mp_transactions::{
    validated::{TxTimestamp, ValidatedTransaction},
    DeclareTransaction, DeployAccountTransaction, InvokeTransaction, Transaction,
};
use mp_utils::service::ServiceContext;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use testcontainers::{core::IntoContainerPort, runners::AsyncRunner, ContainerAsync, GenericImage};
use tokio::sync::OnceCell;

fn make_validated_tx_with_hash(tx_hash: Felt, sender: Felt) -> ValidatedTransaction {
    test_utils::validated_transaction(
        Transaction::Invoke(InvokeTransaction::V3(test_utils::invoke_v3(sender, Felt::from_hex_unchecked("0x1")))),
        tx_hash,
        sender,
        None,
    )
}

fn make_block(transactions: Vec<mp_block::TransactionWithReceipt>) -> FullBlockWithoutCommitments {
    FullBlockWithoutCommitments {
        header: Default::default(),
        state_diff: Default::default(),
        transactions,
        events: Default::default(),
    }
}

async fn wait_for_mongo_ready(uri: &str) {
    let mut attempts = 0;
    loop {
        if let Ok(client) = mongodb::Client::with_uri_str(uri).await {
            if client.database("admin").run_command(mongodb::bson::doc! { "ping": 1 }).await.is_ok() {
                break;
            }
        }
        attempts += 1;
        if attempts > 20 {
            panic!("MongoDB did not become ready in time");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

struct MongoFixture {
    _node: ContainerAsync<GenericImage>,
    uri: String,
}

static MONGO_FIXTURE: OnceCell<MongoFixture> = OnceCell::const_new();

async fn mongo_fixture() -> &'static MongoFixture {
    MONGO_FIXTURE
        .get_or_init(|| async {
            let image = GenericImage::new("mongo", "7.0").with_exposed_port(27017.tcp());
            let node = image.start().await.unwrap();
            let port = node.get_host_port_ipv4(27017).await.unwrap();
            let uri = format!("mongodb://127.0.0.1:{port}");
            wait_for_mongo_ready(&uri).await;
            MongoFixture { _node: node, uri }
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_outbox_to_mongo_and_retention() {
    let fixture = mongo_fixture().await;
    let uri = fixture.uri.clone();

    let backend = MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
    let metrics = Arc::new(ExternalDbMetrics::register());
    let mut config = ExternalDbConfig::new(uri.clone());
    let unique_suffix = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();
    config.database_name = format!("madara_test_{unique_suffix}");
    config.batch_size = 10;
    config.flush_interval_ms = 100;
    config.retention_delay_secs = 1;
    config.retention_tick_secs = 1;
    let worker = ExternalDbWorker::new(config.clone(), backend.clone(), "MADARA_TEST".to_string(), metrics);

    let sender = test_utils::devnet_account_address();
    let invoke_hash = Felt::from_hex_unchecked("0xabc");
    let declare_hash = Felt::from_hex_unchecked("0xdef");
    let deploy_account_hash = Felt::from_hex_unchecked("0x111");
    let l1_handler_hash = Felt::from_hex_unchecked("0x222");

    let invoke_tx = make_validated_tx_with_hash(invoke_hash, sender);
    let (declare_tx, declared_class) = test_utils::declare_v3(sender, Felt::from_hex_unchecked("0x2"));
    let declare_tx = ValidatedTransaction {
        transaction: Transaction::Declare(DeclareTransaction::V3(declare_tx)),
        paid_fee_on_l1: None,
        contract_address: sender,
        arrived_at: TxTimestamp::now(),
        declared_class: Some(declared_class),
        hash: declare_hash,
        charge_fee: true,
    };
    let deploy_account_tx = ValidatedTransaction {
        transaction: Transaction::DeployAccount(DeployAccountTransaction::V3(test_utils::deploy_account_v3(
            Felt::from_hex_unchecked("0x3"),
        ))),
        paid_fee_on_l1: None,
        contract_address: sender,
        arrived_at: TxTimestamp::now(),
        declared_class: None,
        hash: deploy_account_hash,
        charge_fee: true,
    };
    let l1_handler_tx = test_utils::validated_l1_handler(l1_handler_hash);

    let txs = vec![invoke_tx, declare_tx, deploy_account_tx, l1_handler_tx];
    for tx in &txs {
        backend.write_external_outbox(tx).unwrap();
    }

    let ctx = ServiceContext::new();
    let handle = tokio::spawn(worker.run(ctx.clone()));

    let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
    let collection =
        client.database(&config.database_name).collection::<MempoolTransactionDocument>(&config.collection_name);
    let tx_ids: Vec<String> = txs.iter().map(|tx| format!("{}", tx.hash.hex_display())).collect();

    let mut attempts = 0;
    loop {
        let mut found = 0;
        for tx_id in &tx_ids {
            if collection.find_one(mongodb::bson::doc! { "_id": tx_id }).await.unwrap().is_some() {
                found += 1;
            }
        }
        if found == tx_ids.len() {
            break;
        }
        attempts += 1;
        if attempts > 30 {
            panic!("Transactions not found in MongoDB in time");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let remaining: Vec<_> = backend.get_external_outbox_transactions(10).collect::<Result<Vec<_>, _>>().unwrap();
    assert!(remaining.is_empty());

    let block_transactions = vec![
        mp_block::TransactionWithReceipt {
            transaction: txs[0].transaction.clone(),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: invoke_hash,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        },
        mp_block::TransactionWithReceipt {
            transaction: txs[1].transaction.clone(),
            receipt: TransactionReceipt::Declare(DeclareTransactionReceipt {
                transaction_hash: declare_hash,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        },
        mp_block::TransactionWithReceipt {
            transaction: txs[2].transaction.clone(),
            receipt: TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt {
                transaction_hash: deploy_account_hash,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        },
        mp_block::TransactionWithReceipt {
            transaction: txs[3].transaction.clone(),
            receipt: TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
                transaction_hash: l1_handler_hash,
                execution_result: ExecutionResult::Succeeded,
                ..Default::default()
            }),
        },
    ];
    let block = make_block(block_transactions);
    backend.write_access().add_full_block_with_classes(&block, &[], true).unwrap();
    backend.set_latest_l1_confirmed(Some(0)).unwrap();

    let mut attempts = 0;
    loop {
        let mut remaining = 0;
        for tx_id in &tx_ids {
            if collection.find_one(mongodb::bson::doc! { "_id": tx_id }).await.unwrap().is_some() {
                remaining += 1;
            }
        }
        if remaining == 0 {
            break;
        }
        attempts += 1;
        if attempts > 40 {
            panic!("Transactions not deleted by retention in time");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    ctx.cancel_global();
    handle.await.unwrap().unwrap();
}
