#![cfg(test)]

use crate::MadaraBackend;
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use starknet_api::{
    core::ContractAddress,
    executable_transaction::AccountTransaction,
    transaction::{InvokeTransaction, InvokeTransactionV3, TransactionHash},
};

fn make_validated_tx(seed: u64) -> ValidatedTransaction {
    let tx_hash = TransactionHash(seed.into());
    let contract_address = Felt::from_hex_unchecked(
        "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d",
    );

    ValidatedTransaction::from_starknet_api(
        AccountTransaction::Invoke(starknet_api::executable_transaction::InvokeTransaction {
            tx: InvokeTransaction::V3(InvokeTransactionV3 {
                sender_address: ContractAddress::try_from(contract_address).unwrap(),
                resource_bounds: Default::default(),
                tip: Default::default(),
                signature: Default::default(),
                nonce: Default::default(),
                calldata: Default::default(),
                nonce_data_availability_mode: Default::default(),
                fee_data_availability_mode: Default::default(),
                paymaster_data: Default::default(),
                account_deployment_data: Default::default(),
            }),
            tx_hash,
        }),
        TxTimestamp::now(),
        None,
        true,
    )
}

#[tokio::test]
async fn outbox_write_read_roundtrip() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let tx = make_validated_tx(1);

    backend.write_external_outbox(&tx).unwrap();

    let outbox: Vec<_> = backend
        .get_external_outbox_transactions(10)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0], tx);
}

#[tokio::test]
async fn outbox_delete_removes_entry() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let tx = make_validated_tx(2);

    backend.write_external_outbox(&tx).unwrap();
    backend.delete_external_outbox(tx.hash).unwrap();

    let outbox: Vec<_> = backend
        .get_external_outbox_transactions(10)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(outbox.is_empty());
}

#[tokio::test]
async fn outbox_iter_bounded_batch() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    for seed in 10..13 {
        let tx = make_validated_tx(seed);
        backend.write_external_outbox(&tx).unwrap();
    }

    let outbox: Vec<_> = backend
        .get_external_outbox_transactions(2)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(outbox.len(), 2);

    let all_outbox: Vec<_> = backend
        .get_external_outbox_transactions(10)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(all_outbox.len(), 3);
}

#[tokio::test]
async fn outbox_duplicate_write_replaces_entry() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let tx = make_validated_tx(42);

    backend.write_external_outbox(&tx).unwrap();

    let mut updated = tx.clone();
    updated.arrived_at = TxTimestamp::now();
    updated.charge_fee = false;

    backend.write_external_outbox(&updated).unwrap();

    let outbox: Vec<_> = backend
        .get_external_outbox_transactions(10)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0], updated);
}
