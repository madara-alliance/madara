use super::*;
use crate::committed_data_recovery::restore_transactions;
use blockifier::execution::syscalls::committed_data::CommittedDataSet;
use mc_db::test_utils;
use mc_mempool::{Mempool, MempoolConfig};
use mp_convert::{Felt, ToFelt};
use mp_transactions::validated::TxTimestamp;

async fn recovery_backend() -> (Arc<MadaraBackend>, mc_devnet::DevnetKeys) {
    let backend = MadaraBackend::open_for_testing(mp_chain_config::ChainConfig::madara_test().into());
    let mut genesis = mc_devnet::ChainGenesisDescription::base_config().unwrap();
    let keys = genesis.add_devnet_contracts(1).unwrap();
    genesis.build_and_store(&backend).await.unwrap();
    (backend, keys)
}

struct ArchivedTransaction {
    config: ExternalDbConfig,
    client: MongoClient,
    tx: ValidatedTransaction,
    document: MempoolTransactionDocument,
    root: Felt,
}

impl ArchivedTransaction {
    async fn new() -> Self {
        let mut config = ExternalDbConfig::new(std::env::var("RECOVERY_TEST_MONGO_URI").expect("disposable Mongo URI"));
        config.database_name = format!("recovery_transactions_{}", Uuid::new_v4().simple());
        let client = MongoClient::new(&config).await.unwrap();
        let database = client.client().database(&config.database_name);
        let sink = MongoSink {
            collection: client.collection().clone(),
            recovery: CommittedDataArchive::new(&database, &config.collection_name),
        };
        let (source, keys) = recovery_backend().await;
        let account = &keys.0[0];
        let mut transaction =
            Transaction::Invoke(InvokeTransaction::V3(test_utils::invoke_v3(account.address, Felt::ZERO)));
        let hash = transaction.compute_hash(
            source.chain_config().chain_id.to_felt(),
            source.chain_config().latest_protocol_version,
            false,
        );
        let signature = account.secret.sign(&hash).unwrap();
        let Transaction::Invoke(InvokeTransaction::V3(invoke)) = &mut transaction else { unreachable!() };
        invoke.signature = vec![signature.r, signature.s].into();
        let mut tx = test_utils::validated_transaction(transaction, hash, account.address, None);
        tx.arrived_at = TxTimestamp(1); // Intentionally older than the normal mempool TTL.
        let entries = vec![(Felt::ONE, Felt::from(42u32))];
        let root = CommittedDataSet::new(entries.clone()).unwrap().root();
        source.import_committed_data_snapshot(root, entries.clone()).await.unwrap();
        let id = source.journal_committed_data(None, root, entries).await.unwrap();
        source.db.write_recovery_admission(&tx, &[id], true).unwrap();
        let worker = ExternalDbWorker::new(
            config.clone(),
            source.clone(),
            source.chain_config().chain_id.to_string(),
            Arc::new(ExternalDbMetrics::register()),
        );
        assert_eq!(worker.drain_outbox_once(&sink).await.unwrap().inserted, 1);
        assert_eq!(source.get_external_outbox_transactions(10).count(), 0);
        let document = client.collection().find_one(bson::doc! {}).await.unwrap().unwrap();
        // Only the archive remains available to restore; both source handles are dropped here.
        Self { config, client, tx, document, root }
    }

    async fn drop_database(self) {
        self.client.client().database(&self.config.database_name).drop().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires disposable MongoDB; explicitly run by task-test-madara.yml"]
async fn mongo_transaction_recovery_restores_admission_and_reconciles_executed_nonce() {
    let archive = ArchivedTransaction::new().await;
    let mut duplicate = archive.document.clone();
    duplicate.id.bytes = Uuid::new_v4().as_bytes().to_vec();
    let mut resubmitted = archive.tx.clone();
    resubmitted.arrived_at = TxTimestamp(2);
    duplicate.arrived_at = bson::DateTime::from_millis(2);
    duplicate.raw_transaction.bytes = bincode::serialize(&resubmitted).unwrap();
    archive.client.collection().insert_one(duplicate).await.unwrap();
    let (target, _) = recovery_backend().await;
    let before = TxTimestamp::now();
    assert_eq!(restore_transactions(&target, &archive.config, &[archive.tx.hash]).await.unwrap(), 1);
    let saved = target.get_saved_mempool_transactions().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].arrived_at >= before && saved[0].arrived_at <= TxTimestamp::now());
    let mut expected = archive.tx.clone();
    expected.arrived_at = saved[0].arrived_at;
    assert_eq!(saved, vec![expected]);
    assert_eq!(target.db.recovery_dependencies(archive.tx.hash).unwrap(), archive.document.committed_data);
    assert_eq!(target.get_external_outbox_transactions(10).count(), 0);
    assert!(target.committed_data_witness(archive.root, Felt::ONE).unwrap().unwrap().verify());

    let stopped = ServiceContext::new_for_testing();
    stopped.cancel_global();
    let mempool = Mempool::new(target.clone(), MempoolConfig::default());
    mempool.run_mempool_task(stopped.clone()).await.unwrap();
    assert!(mempool.is_transaction_in_mempool(&archive.tx.hash));
    drop(mempool);

    // The restored chain can already contain this nonce: restart must discard its saved admission.
    let mut executed = mc_db::preconfirmed::PreconfirmedExecutedTransaction {
        transaction: mp_block::TransactionWithReceipt {
            transaction: archive.tx.transaction.clone(),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                transaction_hash: archive.tx.hash,
                ..Default::default()
            }),
        },
        state_diff: Default::default(),
        declared_class: None,
        arrived_at: archive.tx.arrived_at,
        paid_fee_on_l1: None,
    };
    executed.state_diff.nonces.insert(archive.tx.contract_address, Felt::ONE);
    target
        .write_access()
        .new_preconfirmed(mc_db::preconfirmed::PreconfirmedBlock::new_with_content(
            mp_block::header::PreconfirmedHeader { block_number: 1, ..Default::default() },
            [executed],
            [],
        ))
        .unwrap();
    let restarted = Mempool::new(target.clone(), MempoolConfig::default());
    restarted.run_mempool_task(stopped).await.unwrap();
    assert!(restarted.is_empty().await);
    assert_eq!(target.get_saved_mempool_transactions().count(), 0);
    archive.drop_database().await;
}

/// Each rejection starts with its own valid archive and must leave no transaction admission.
#[rstest::rstest]
#[case::missing("missing", "Missing recoverable transaction")]
#[case::wrong_chain("wrong_chain", "Missing recoverable transaction")]
#[case::conflicting_payload("conflicting_payload", "Conflicting transaction archive records")]
#[case::conflicting_dependencies("conflicting_dependencies", "Conflicting transaction archive records")]
#[case::stored_hash("stored_hash", "Recovery transaction hash mismatch")]
#[case::computed_hash("computed_hash", "Recovery transaction hash mismatch")]
#[case::too_many_dependencies("too_many_dependencies", "Invalid recovery dependency count")]
#[case::missing_dependency("missing_dependency", "Missing complete recovery manifest")]
#[case::corrupt_dependency("corrupt_dependency", "Recovery chunk digest mismatch")]
#[tokio::test]
#[ignore = "requires disposable MongoDB; explicitly run by task-test-madara.yml"]
async fn mongo_transaction_recovery_rejects_invalid_archives_before_admission(#[case] case: &str, #[case] error: &str) {
    let archive = ArchivedTransaction::new().await;
    // Each case starts with the same valid document, so failures cannot mask each other.
    archive.client.collection().delete_many(bson::doc! {}).await.unwrap();
    let mut document = archive.document.clone();
    let mut tx = archive.tx.clone();
    match case {
        "missing" => {}
        "wrong_chain" => document.chain_id = "other-chain".into(),
        "conflicting_payload" | "conflicting_dependencies" => {
            archive.client.collection().insert_one(document.clone()).await.unwrap();
            document.id.bytes = Uuid::new_v4().as_bytes().to_vec();
            if case == "conflicting_payload" {
                tx.charge_fee = !tx.charge_fee;
            } else {
                document.committed_data = vec!["a".repeat(64)];
            }
        }
        "stored_hash" => tx.hash = Felt::ZERO,
        "computed_hash" => {
            let Transaction::Invoke(InvokeTransaction::V3(invoke)) = &mut tx.transaction else { unreachable!() };
            invoke.nonce = Felt::ONE;
        }
        "too_many_dependencies" => document.committed_data = vec![document.committed_data[0].clone(); 5],
        "missing_dependency" => document.committed_data = vec!["a".repeat(64)],
        "corrupt_dependency" => {
            let database = archive.client.client().database(&archive.config.database_name);
            let recovery = CommittedDataArchive::new(&database, &archive.config.collection_name);
            let record = recovery.download(&document.committed_data[0]).await.unwrap();
            database
                .collection::<bson::Document>(&format!("{}_committed_chunks_v1", archive.config.collection_name))
                .update_one(
                    bson::doc! {"_id": &record.manifest.chunks[0]},
                    bson::doc! {"$set":{"payload":bson::Binary {
                        subtype:bson::spec::BinarySubtype::Generic, bytes:vec![0,1,2],
                    }}},
                )
                .await
                .unwrap();
        }
        _ => unreachable!(),
    }
    if case != "missing" {
        document.raw_transaction.bytes = bincode::serialize(&tx).unwrap();
        archive.client.collection().insert_one(document).await.unwrap();
    }
    let (target, _) = recovery_backend().await;
    let failure = restore_transactions(&target, &archive.config, &[archive.tx.hash]).await.unwrap_err();
    assert!(format!("{failure:#}").contains(error), "{case}: {failure:#}");
    assert_eq!(target.get_saved_mempool_transactions().count(), 0, "{case}");
    assert!(target.db.recovery_dependencies(archive.tx.hash).unwrap().is_empty(), "{case}");
    assert_eq!(target.get_external_outbox_transactions(10).count(), 0, "{case}");
    archive.drop_database().await;
}
