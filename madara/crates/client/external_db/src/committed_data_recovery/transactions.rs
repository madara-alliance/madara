//! Restore selected transaction admissions after authenticating their dataset dependencies.
use super::CommittedDataArchive;
use anyhow::{ensure, Context, Result};
use bincode::Options;
use mc_db::{MadaraBackend, MadaraStorageWrite};
use mongodb::bson::doc;
use mp_convert::{Felt, FeltHexDisplay, ToFelt};
use mp_gateway::committed_data::MAX_DATASETS_PER_TRANSACTION;
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use std::sync::Arc;

/// Mongo BSON documents are limited to 16 MiB including metadata; a stored binary payload
/// cannot be larger. Keep a decode allocation bound even for malformed archived bincode.
const MAX_RECOVERY_TRANSACTION_BYTES: u64 = 16 * 1024 * 1024;

fn decode_recovery_transaction(bytes: &[u8]) -> Result<ValidatedTransaction> {
    let mut tx: ValidatedTransaction = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_RECOVERY_TRANSACTION_BYTES)
        .reject_trailing_bytes()
        .deserialize(bytes)?;
    // Resubmissions have different admission times. Recovery assigns a fresh time after comparison.
    tx.arrived_at = TxTimestamp::UNIX_EPOCH;
    Ok(tx)
}

/// Stage only explicitly selected backed-up committed-data transactions. Mempool startup
/// reconciles them against the restored chain, including nonces already executed there.
/// A fresh local arrival timestamp avoids silently dropping disaster-recovery work by TTL;
/// Mongo retains the original arrival timestamp for audit and replay ordering.
pub async fn restore_transactions(
    backend: &Arc<MadaraBackend>,
    config: &crate::config::ExternalDbConfig,
    hashes: &[Felt],
) -> Result<usize> {
    use futures::TryStreamExt;
    let client = crate::mongodb::MongoClient::new(config).await?;
    let archive = CommittedDataArchive::new(&client.client().database(&config.database_name), &config.collection_name);
    let chain_id = backend.chain_config().chain_id.to_string();
    let mut restored = 0;
    for hash in hashes {
        let mut cursor = client
            .collection()
            .find(doc! {
                "tx_hash": format!("{}", hash.hex_display()),
                "chain_id": &chain_id,
                "committed_data.0": {"$exists": true},
            })
            .sort(doc! {"arrived_at": 1, "_id": 1})
            .await?;
        let document = cursor.try_next().await?.context("Missing recoverable transaction in Mongo archive")?;
        let mut tx = decode_recovery_transaction(&document.raw_transaction.bytes)?;
        while let Some(duplicate) = cursor.try_next().await? {
            ensure!(
                decode_recovery_transaction(&duplicate.raw_transaction.bytes)? == tx
                    && duplicate.committed_data == document.committed_data,
                "Conflicting transaction archive records"
            );
        }
        ensure!(
            (1..=MAX_DATASETS_PER_TRANSACTION).contains(&document.committed_data.len()),
            "Invalid recovery dependency count"
        );
        ensure!(
            tx.hash == *hash
                && tx.transaction.compute_hash(
                    backend.chain_config().chain_id.to_felt(),
                    backend.chain_config().latest_protocol_version,
                    false
                ) == *hash,
            "Recovery transaction hash mismatch"
        );
        archive.restore(backend, &document.committed_data).await?;
        tx.arrived_at = TxTimestamp::now();
        backend.db.write_recovery_admission(&tx, &document.committed_data, false)?;
        restored += 1;
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_transactions::{InvokeTransaction, Transaction};
    #[test]
    fn recovery_comparison_ignores_only_arrival_time() {
        let sender = mc_db::test_utils::devnet_account_address();
        let mut original = mc_db::test_utils::validated_transaction(
            Transaction::Invoke(InvokeTransaction::V3(mc_db::test_utils::invoke_v3(sender, Felt::ONE))),
            Felt::ONE,
            sender,
            None,
        );
        original.arrived_at = TxTimestamp(100);
        let original_bytes = bincode::serialize(&original).unwrap();
        let expected = decode_recovery_transaction(&original_bytes).unwrap();
        let mut resubmitted = original.clone();
        resubmitted.arrived_at = TxTimestamp(200);
        let resubmitted_bytes = bincode::serialize(&resubmitted).unwrap();
        assert_ne!(original_bytes, resubmitted_bytes);
        assert_eq!(expected, decode_recovery_transaction(&resubmitted_bytes).unwrap());

        let mut changed_transaction = resubmitted.clone();
        changed_transaction.transaction =
            Transaction::Invoke(InvokeTransaction::V3(mc_db::test_utils::invoke_v3(sender, Felt::TWO)));
        let mut changed_fee = resubmitted.clone();
        changed_fee.charge_fee = !changed_fee.charge_fee;
        let mut changed_l1_fee = resubmitted.clone();
        changed_l1_fee.paid_fee_on_l1 = Some(1);
        let mut changed_address = resubmitted.clone();
        changed_address.contract_address = Felt::ZERO;
        let mut changed_hash = resubmitted;
        changed_hash.hash = Felt::TWO;
        for changed in [changed_transaction, changed_fee, changed_l1_fee, changed_address, changed_hash] {
            assert_ne!(expected, decode_recovery_transaction(&bincode::serialize(&changed).unwrap()).unwrap());
        }

        assert!(decode_recovery_transaction(&original_bytes[..original_bytes.len() - 1]).is_err());
        let mut trailing = original_bytes;
        trailing.push(0);
        assert!(decode_recovery_transaction(&trailing).is_err());
    }
}
