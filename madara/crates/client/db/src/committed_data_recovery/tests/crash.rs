//! Process death between durable import phases must leave a resumable prefix, never a false admission.
use super::*;
use crate::{
    rocksdb::{DbWriteMode, RocksDBConfig},
    MadaraBackendConfig, MadaraStorageWrite,
};
use blockifier::execution::syscalls::committed_data::CommittedDataSet;
use mp_gateway::committed_data::CommittedDataVersion;
use std::path::Path;

fn open(path: &Path, wal: bool) -> Arc<MadaraBackend> {
    MadaraBackend::open_rocksdb(
        path,
        mp_chain_config::ChainConfig::madara_test().into(),
        MadaraBackendConfig::default(),
        RocksDBConfig { write_mode: DbWriteMode { wal, fsync: false }, ..Default::default() },
        Default::default(),
    )
    .unwrap()
}

fn initial_values() -> Vec<(Felt, Felt)> {
    vec![(Felt::ONE, Felt::TWO), (Felt::TWO, Felt::MAX)]
}

fn context(patch: bool) -> CommittedDataUpdate {
    CommittedDataUpdate {
        application_id: Felt::ONE,
        dataset_id: Felt::TWO,
        base_root: patch.then(|| CommittedDataSet::new(initial_values()).unwrap().root()),
    }
}

fn submitted(patch: bool) -> Vec<(Felt, Felt)> {
    if patch {
        vec![(Felt::ONE, Felt::THREE)]
    } else {
        initial_values()
    }
}

fn expected(patch: bool) -> CommittedDataSet {
    let mut values = initial_values();
    if patch {
        values[0].1 = Felt::THREE;
    }
    CommittedDataSet::new(values).unwrap()
}

fn transaction() -> mp_transactions::validated::ValidatedTransaction {
    let sender = crate::test_utils::devnet_account_address();
    crate::test_utils::validated_transaction(
        mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V3(crate::test_utils::invoke_v3(
            sender,
            Felt::ZERO,
        ))),
        Felt::THREE,
        sender,
        None,
    )
}

/// Each test case runs this entry point in a fresh process. Even WAL-disabled chain writes
/// must not weaken the dedicated synchronous committed-data import/admission writes.
#[cfg(unix)]
#[tokio::test]
async fn import_crash_child() {
    let Ok(path) = std::env::var("MADARA_IMPORT_CRASH_PATH") else { return };
    let phase: usize = std::env::var("MADARA_IMPORT_CRASH_PHASE").unwrap().parse().unwrap();
    let patch = std::env::var("MADARA_IMPORT_CRASH_PATCH").unwrap() == "true";
    let wal = std::env::var("MADARA_IMPORT_CRASH_WAL").unwrap() == "true";
    let backend = open(Path::new(&path), wal);
    if patch {
        let root = expected(false).root();
        backend.update_committed_data(context(false), root, initial_values()).await.unwrap();
        let id = backend.journal_committed_data(Some(context(false)), root, initial_values()).await.unwrap();
        backend.db.mark_recovery_exported(&id).unwrap();
    }
    let update = context(patch);
    let root = expected(patch).root();
    if phase >= 1 {
        match update.base_root {
            Some(base) => backend.db.patch_committed_data_dataset(base, root, submitted(patch), None).unwrap(),
            None => backend.db.import_committed_data_values(root, submitted(patch), None).unwrap(),
        }
    }
    if phase >= 2 {
        backend
            .db
            .write_committed_data_version(
                CommittedDataVersion { application_id: update.application_id, dataset_id: update.dataset_id, root },
                !patch,
            )
            .unwrap();
    }
    if phase >= 3 {
        let id = backend.journal_committed_data(Some(update), root, submitted(patch)).await.unwrap();
        if phase >= 4 {
            backend.db.write_recovery_admission(&transaction(), std::slice::from_ref(&id), true).unwrap();
        }
        if phase >= 5 {
            backend.db.mark_recovery_exported(&id).unwrap();
        }
    }
    // SAFETY: deliberately kill only this test child, without Drop or a final DB flush.
    unsafe { libc::raise(libc::SIGKILL) };
    unreachable!("SIGKILL must terminate the child");
}

#[cfg(unix)]
#[rstest::rstest]
#[case::before_import(0)]
#[case::tree_without_catalog(1)]
#[case::catalog_without_journal(2)]
#[case::journal_without_admission(3)]
#[case::admitted_without_export_ack(4)]
#[case::export_acknowledged(5)]
#[tokio::test]
async fn killed_import_recovers_prefix_and_retry_completes(
    #[case] phase: usize,
    #[values(false, true)] patch: bool,
    #[values(false, true)] wal: bool,
) {
    use std::os::unix::process::ExitStatusExt;
    let directory = tempfile::tempdir().unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "committed_data_recovery::tests::crash::import_crash_child", "--nocapture"])
        .env("MADARA_IMPORT_CRASH_PATH", directory.path())
        .env("MADARA_IMPORT_CRASH_PHASE", phase.to_string())
        .env("MADARA_IMPORT_CRASH_PATCH", patch.to_string())
        .env("MADARA_IMPORT_CRASH_WAL", wal.to_string())
        .output()
        .unwrap();
    assert_eq!(child.status.signal(), Some(libc::SIGKILL), "{}", String::from_utf8_lossy(&child.stderr));
    let backend = open(directory.path(), wal);
    let tree = expected(patch);
    let root = tree.root();
    let update = context(patch);
    assert_eq!(backend.db.get_committed_data_count(root).unwrap().is_some(), phase >= 1);
    assert_eq!(
        backend
            .db
            .has_committed_data_version(CommittedDataVersion {
                application_id: update.application_id,
                dataset_id: update.dataset_id,
                root,
            })
            .unwrap(),
        phase >= 2
    );
    assert_eq!(
        backend.db.find_recovery_record(&namespace_key(Some(update), root).unwrap()).unwrap().is_some(),
        phase >= 3
    );
    assert_eq!(backend.get_saved_mempool_transactions().count(), usize::from(phase >= 4));
    assert_eq!(backend.get_external_outbox_transactions(10).count(), usize::from(phase >= 4));
    assert_eq!(backend.db.recovery_dependencies(Felt::THREE).unwrap().is_empty(), phase < 4);
    assert_eq!(backend.db.recovery_pending_bytes().unwrap() > 0, (3..5).contains(&phase));
    if phase < 3 {
        assert!(
            backend.recovery_reference(Some(update), root).is_err(),
            "An unjournaled map cannot authorize admission"
        );
    }
    // A client cannot distinguish a lost response from a lost write. Repeat both import and
    // journal; the same immutable identity must survive, including an already exported record.
    backend.update_committed_data(update, root, submitted(patch)).await.unwrap();
    let id = backend.journal_committed_data(Some(update), root, submitted(patch)).await.unwrap();
    assert_eq!(backend.journal_committed_data(Some(update), root, submitted(patch)).await.unwrap(), id);
    if phase < 4 {
        backend.db.write_recovery_admission(&transaction(), std::slice::from_ref(&id), true).unwrap();
    }
    assert_eq!(backend.db.recovery_dependencies(Felt::THREE).unwrap(), vec![id.clone()]);
    if phase == 5 {
        assert!(backend.db.get_recovery_record(&id).unwrap().is_none(), "Retry must not resurrect acknowledged bytes");
    } else {
        assert_eq!(backend.db.get_recovery_record(&id).unwrap().unwrap().decode().unwrap(), submitted(patch));
    }
    backend.db.mark_recovery_exported(&id).unwrap();
    backend.db.mark_recovery_exported(&id).unwrap();
    while backend.db.maintain_committed_data_history(Some(u64::MAX)).unwrap() {}
    drop(backend);
    // A second restart after payload deletion and proof pruning still serves authenticated
    // historical values and retains the durable transaction dependency for operator replay.
    let backend = open(directory.path(), wal);
    assert_eq!(backend.db.recovery_pending_bytes().unwrap(), 0);
    assert_eq!(backend.db.recovery_dependencies(Felt::THREE).unwrap(), vec![id]);
    assert_eq!(backend.get_saved_mempool_transactions().count(), 1);
    assert_eq!(backend.get_external_outbox_transactions(10).count(), 1);
    for (key, value) in tree.entries() {
        let proof = backend.committed_data_witness(root, *key).unwrap().unwrap();
        assert_eq!(proof.value, *value);
        assert!(proof.verify());
    }
    if patch {
        let base = expected(false);
        let proof = backend.committed_data_witness(base.root(), Felt::ONE).unwrap().unwrap();
        assert_eq!(proof.value, Felt::TWO);
        assert!(proof.verify());
    }
}
