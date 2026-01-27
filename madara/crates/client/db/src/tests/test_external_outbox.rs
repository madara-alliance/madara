#![cfg(test)]

use crate::tests::test_utils;
use crate::MadaraBackend;
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use mp_transactions::{
    validated::TxTimestamp, DeclareTransaction, DeployAccountTransaction, InvokeTransaction, Transaction,
};

#[tokio::test]
async fn outbox_write_read_roundtrip() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let sender = test_utils::devnet_account_address();
    let (declare_tx, declared_class) = test_utils::declare_v3(sender, Felt::from_hex_unchecked("0x2"));
    let txs = vec![
        test_utils::validated_transaction(
            Transaction::Invoke(InvokeTransaction::V3(test_utils::invoke_v3(sender, Felt::from_hex_unchecked("0x1")))),
            Felt::from_hex_unchecked("0x1"),
            sender,
            None,
        ),
        test_utils::validated_transaction(
            Transaction::Declare(DeclareTransaction::V3(declare_tx)),
            Felt::from_hex_unchecked("0x2"),
            sender,
            Some(declared_class),
        ),
        test_utils::validated_transaction(
            Transaction::DeployAccount(DeployAccountTransaction::V3(test_utils::deploy_account_v3(
                Felt::from_hex_unchecked("0x3"),
            ))),
            Felt::from_hex_unchecked("0x3"),
            sender,
            None,
        ),
        test_utils::validated_transaction(
            Transaction::Deploy(test_utils::deploy_tx()),
            Felt::from_hex_unchecked("0x4"),
            sender,
            None,
        ),
        test_utils::validated_l1_handler(Felt::from_hex_unchecked("0x5")),
    ];

    for tx in &txs {
        backend.write_external_outbox(tx).unwrap();
    }

    let outbox: Vec<_> = backend.get_external_outbox_transactions(10).collect::<Result<Vec<_>, _>>().unwrap();

    assert_eq!(outbox.len(), txs.len());
    for tx in txs {
        assert!(outbox.iter().any(|stored| stored.hash == tx.hash && stored.transaction == tx.transaction));
    }
}

#[tokio::test]
async fn outbox_delete_removes_entry() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let tx = test_utils::validated_transaction(
        Transaction::Invoke(InvokeTransaction::V3(test_utils::invoke_v3(
            Felt::from_hex_unchecked("0x1234"),
            Felt::from_hex_unchecked("0x10"),
        ))),
        Felt::from_hex_unchecked("0x10"),
        Felt::from_hex_unchecked("0x1234"),
        None,
    );

    backend.write_external_outbox(&tx).unwrap();
    backend.delete_external_outbox(tx.hash).unwrap();

    let outbox: Vec<_> = backend.get_external_outbox_transactions(10).collect::<Result<Vec<_>, _>>().unwrap();

    assert!(outbox.is_empty());
}

#[tokio::test]
async fn outbox_iter_bounded_batch() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    for seed in 10..13 {
        let tx = test_utils::validated_transaction(
            Transaction::Invoke(InvokeTransaction::V3(test_utils::invoke_v3(
                Felt::from_hex_unchecked("0x1234"),
                Felt::from_hex_unchecked(&format!("0x{seed:x}")),
            ))),
            Felt::from_hex_unchecked(&format!("0x{seed:x}")),
            Felt::from_hex_unchecked("0x1234"),
            None,
        );
        backend.write_external_outbox(&tx).unwrap();
    }

    let outbox: Vec<_> = backend.get_external_outbox_transactions(2).collect::<Result<Vec<_>, _>>().unwrap();

    assert_eq!(outbox.len(), 2);

    let all_outbox: Vec<_> = backend.get_external_outbox_transactions(10).collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(all_outbox.len(), 3);
}

#[tokio::test]
async fn outbox_duplicate_write_replaces_entry() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let tx = test_utils::validated_transaction(
        Transaction::Invoke(InvokeTransaction::V3(test_utils::invoke_v3(
            Felt::from_hex_unchecked("0x1234"),
            Felt::from_hex_unchecked("0x2a"),
        ))),
        Felt::from_hex_unchecked("0x2a"),
        Felt::from_hex_unchecked("0x1234"),
        None,
    );

    backend.write_external_outbox(&tx).unwrap();

    let mut updated = tx.clone();
    updated.arrived_at = TxTimestamp::now();
    updated.charge_fee = false;

    backend.write_external_outbox(&updated).unwrap();

    let outbox: Vec<_> = backend.get_external_outbox_transactions(10).collect::<Result<Vec<_>, _>>().unwrap();

    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0], updated);
}
