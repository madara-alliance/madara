use mc_db::MadaraBackend;
use mc_external_db::{
    config::ExternalDbConfig, metrics::ExternalDbMetrics, mongodb::MempoolTransactionDocument, writer::ExternalDbWorker,
};
use mp_block::FullBlockWithoutCommitments;
use mp_convert::{Felt, FeltHexDisplay};
use mp_receipt::{ExecutionResult, InvokeTransactionReceipt, TransactionReceipt};
use mp_transactions::{
    validated::{TxTimestamp, ValidatedTransaction},
    DataAvailabilityMode, InvokeTransaction, InvokeTransactionV3, ResourceBounds, ResourceBoundsMapping, Transaction,
};
use mp_utils::service::ServiceContext;
use std::sync::Arc;
use std::time::Duration;
use testcontainers::{core::IntoContainerPort, runners::AsyncRunner, GenericImage};

fn make_validated_tx_with_hash(tx_hash: Felt, sender: Felt) -> ValidatedTransaction {
    let tx = Transaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3 {
        sender_address: sender,
        calldata: Default::default(),
        signature: Default::default(),
        nonce: Default::default(),
        resource_bounds: ResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 1 },
            l2_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 1 },
            l1_data_gas: None,
        },
        tip: 0,
        paymaster_data: vec![],
        account_deployment_data: vec![],
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        fee_data_availability_mode: DataAvailabilityMode::L1,
    }));

    ValidatedTransaction {
        transaction: tx,
        paid_fee_on_l1: None,
        contract_address: sender,
        arrived_at: TxTimestamp::now(),
        declared_class: None,
        hash: tx_hash,
        charge_fee: true,
    }
}

fn make_block_with_hash(tx_hash: Felt, sender: Felt) -> FullBlockWithoutCommitments {
    let tx = Transaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3 {
        sender_address: sender,
        calldata: Default::default(),
        signature: Default::default(),
        nonce: Default::default(),
        resource_bounds: ResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 1 },
            l2_gas: ResourceBounds { max_amount: 1, max_price_per_unit: 1 },
            l1_data_gas: None,
        },
        tip: 0,
        paymaster_data: vec![],
        account_deployment_data: vec![],
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        fee_data_availability_mode: DataAvailabilityMode::L1,
    }));

    let receipt = TransactionReceipt::Invoke(InvokeTransactionReceipt {
        transaction_hash: tx_hash,
        execution_result: ExecutionResult::Succeeded,
        ..Default::default()
    });

    FullBlockWithoutCommitments {
        header: Default::default(),
        state_diff: Default::default(),
        transactions: vec![mp_block::TransactionWithReceipt { transaction: tx, receipt }],
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

#[tokio::test(flavor = "multi_thread")]
async fn e2e_outbox_to_mongo_and_retention() {
    let image = GenericImage::new("mongo", "7.0").with_exposed_port(27017.tcp());
    let node = image.start().await.unwrap();
    let port = node.get_host_port_ipv4(27017).await.unwrap();
    let uri = format!("mongodb://127.0.0.1:{port}");

    wait_for_mongo_ready(&uri).await;

    let backend = MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
    let metrics = Arc::new(ExternalDbMetrics::register());
    let mut config = ExternalDbConfig::new(uri.clone());
    config.batch_size = 10;
    config.flush_interval_ms = 100;
    config.retention_delay_secs = 1;
    config.retention_tick_secs = 1;
    let worker = ExternalDbWorker::new(config.clone(), backend.clone(), "MADARA_TEST".to_string(), metrics);

    let sender = Felt::from_hex_unchecked("0x1234");
    let tx_hash = Felt::from_hex_unchecked("0xabc");
    let tx = make_validated_tx_with_hash(tx_hash, sender);
    backend.write_external_outbox(&tx).unwrap();

    let ctx = ServiceContext::new();
    let handle = tokio::spawn(worker.run(ctx.clone()));

    let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
    let collection = client
        .database(&config.database_name)
        .collection::<MempoolTransactionDocument>(&config.collection_name);
    let tx_id = format!("{}", tx_hash.hex_display());

    let mut attempts = 0;
    loop {
        if collection.find_one(mongodb::bson::doc! { "_id": &tx_id }).await.unwrap().is_some() {
            break;
        }
        attempts += 1;
        if attempts > 30 {
            panic!("Transaction not found in MongoDB in time");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let remaining: Vec<_> = backend
        .get_external_outbox_transactions(10)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(remaining.is_empty());

    let block = make_block_with_hash(tx_hash, sender);
    backend.write_access().add_full_block_with_classes(&block, &[], true).unwrap();
    backend.set_latest_l1_confirmed(Some(0)).unwrap();

    let mut attempts = 0;
    loop {
        if collection.find_one(mongodb::bson::doc! { "_id": &tx_id }).await.unwrap().is_none() {
            break;
        }
        attempts += 1;
        if attempts > 40 {
            panic!("Transaction not deleted by retention in time");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    ctx.cancel_global();
    handle.await.unwrap().unwrap();
}
