use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner, WriteBatchWithTransaction},
    storage::{L1ToL2MessageIndexEntry, L1ToL2MessagesByL1TxHash},
};
use mp_convert::Felt;
use mp_convert::L1TransactionHash;
use mp_receipt::L1HandlerTransactionReceipt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use rocksdb::ReadOptions;

/// <core_contract_nonce 8 bytes> => bincode(pending message)
pub const L1_TO_L2_PENDING_MESSAGE_BY_NONCE: Column =
    Column::new("l1_to_l2_pending_message_by_nonce").set_point_lookup();
/// <core_contract_nonce 8 bytes> => <l1_tx_hash 32 bytes>
pub const L1_TO_L2_L1_TXN_HASH_BY_NONCE: Column = Column::new("l1_to_l2_l1_txn_hash_by_nonce").set_point_lookup();
/// <core_contract_nonce 8 bytes> => txn hash
pub const L1_TO_L2_TXN_HASH_BY_NONCE: Column = Column::new("l1_to_l2_txn_hash_by_nonce").set_point_lookup();

/// <l1_tx_hash 32 bytes> + <core_contract_nonce 8 bytes> => <l2_tx_hash 32 bytes> | empty
///
/// This is the secondary index used by `starknet_getMessagesStatus` to avoid O(N) scans at RPC time.
/// - Key prefix: 32 bytes of `l1_tx_hash` allows prefix iteration for all messages sent by the same L1 tx.
/// - Value:
///   - empty => "seen/received marker" (message emitted on L1, but no consumed L1-handler tx known yet)
///   - 32 bytes => consumed L1-handler L2 transaction hash (felt bytes)
pub const L1_TO_L2_L2_TXN_HASH_BY_L1_TXN_HASH_AND_NONCE: Column =
    Column::new("l1_to_l2_l2_txn_hash_by_l1_txn_hash_and_nonce").with_prefix_extractor_len(32);

impl RocksDBStorageInner {
    /// Also removed the given txns from the pending column.
    pub(super) fn messages_to_l2_write_trasactions<'a>(
        &self,
        txs: impl IntoIterator<Item = (&'a L1HandlerTransaction, &'a L1HandlerTransactionReceipt)>,
    ) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        let l1_tx_hash_by_nonce_cf = self.get_column(L1_TO_L2_L1_TXN_HASH_BY_NONCE);
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        let by_l1_tx_hash_cf = self.get_column(L1_TO_L2_L2_TXN_HASH_BY_L1_TXN_HASH_AND_NONCE);

        for (txn, receipt) in txs {
            let key = txn.nonce.to_be_bytes();
            batch.delete_cf(&pending_cf, key);
            batch.put_cf(&on_l2_cf, key, receipt.transaction_hash.to_bytes_be());

            // If the L1 tx hash mapping exists, fill the secondary index too.
            if let Some(l1_tx_hash_bytes) = self.db.get_pinned_cf(&l1_tx_hash_by_nonce_cf, key)? {
                if l1_tx_hash_bytes.len() != 32 {
                    bail!("Invalid l1->l2 nonce->l1_tx_hash value length: expected 32, got {}", l1_tx_hash_bytes.len());
                }
                let l1_tx_hash = L1TransactionHash(l1_tx_hash_bytes.as_ref().try_into().expect("slice len checked"));
                let by_l1_key = Self::message_to_l2_by_l1_tx_key(&l1_tx_hash, txn.nonce);
                batch.put_cf(&by_l1_tx_hash_cf, by_l1_key, receipt.transaction_hash.to_bytes_be());
            }
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    fn message_to_l2_by_l1_tx_key(l1_tx_hash: &L1TransactionHash, core_contract_nonce: u64) -> [u8; 40] {
        let mut key = [0u8; 40];
        key[..32].copy_from_slice(&l1_tx_hash.0);
        key[32..].copy_from_slice(&core_contract_nonce.to_be_bytes());
        key
    }

    pub(super) fn get_message_to_l2_index_entry(
        &self,
        l1_tx_hash: &L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<Option<L1ToL2MessageIndexEntry>> {
        let cf = self.get_column(L1_TO_L2_L2_TXN_HASH_BY_L1_TXN_HASH_AND_NONCE);
        let key = Self::message_to_l2_by_l1_tx_key(l1_tx_hash, core_contract_nonce);
        let Some(existing) = self.db.get_pinned_cf(&cf, key)? else { return Ok(None) };
        Ok(Some(match existing.len() {
            0 => L1ToL2MessageIndexEntry::Seen,
            32 => L1ToL2MessageIndexEntry::Consumed(Felt::from_bytes_be(
                existing[..].try_into().expect("slice len checked"),
            )),
            n => bail!("Invalid l1->l2 message value length: expected 0 or 32, got {n}"),
        }))
    }

    /// Insert the `(l1_tx_hash, nonce)` key with an empty marker value, if it is missing.
    ///
    /// Returns:
    /// - `Ok(true)` if the marker was inserted.
    /// - `Ok(false)` if the key already existed (empty marker or consumed tx hash).
    pub(super) fn insert_message_to_l2_seen_marker(
        &self,
        l1_tx_hash: &L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<bool> {
        let cf = self.get_column(L1_TO_L2_L2_TXN_HASH_BY_L1_TXN_HASH_AND_NONCE);
        let key = Self::message_to_l2_by_l1_tx_key(l1_tx_hash, core_contract_nonce);
        if self.db.get_pinned_cf(&cf, key)?.is_some() {
            return Ok(false);
        }
        self.db.put_cf_opt(&cf, key, [], &self.writeopts)?;
        Ok(true)
    }

    /// Write/update the consumed L1-handler L2 transaction hash for `(l1_tx_hash, nonce)`.
    pub(super) fn write_message_to_l2_consumed_txn_hash(
        &self,
        l1_tx_hash: &L1TransactionHash,
        core_contract_nonce: u64,
        l2_tx_hash: &Felt,
    ) -> Result<()> {
        let cf = self.get_column(L1_TO_L2_L2_TXN_HASH_BY_L1_TXN_HASH_AND_NONCE);
        let key = Self::message_to_l2_by_l1_tx_key(l1_tx_hash, core_contract_nonce);
        self.db.put_cf_opt(&cf, key, l2_tx_hash.to_bytes_be(), &self.writeopts)?;
        Ok(())
    }

    /// Get all messages (nonces) for a given L1 tx, in L1 sending order (nonce order).
    ///
    /// Returns:
    /// - `Ok(None)` if the L1 tx hash is unknown to the node.
    /// - `Ok(Some(vec))` for known hashes (values may be `None` for not-yet-consumed messages).
    pub(super) fn get_messages_to_l2_by_l1_tx_hash(
        &self,
        l1_tx_hash: &L1TransactionHash,
    ) -> Result<Option<L1ToL2MessagesByL1TxHash>> {
        let cf = self.get_column(L1_TO_L2_L2_TXN_HASH_BY_L1_TXN_HASH_AND_NONCE);
        let prefix = l1_tx_hash.0.as_slice();

        // We use RocksDB's standard iterator here (owned key/value) since we need to stop once
        // the prefix no longer matches.
        let iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::From(prefix, rocksdb::Direction::Forward));

        let mut out = Vec::new();
        for kv in iter {
            let (key, val) = kv?;
            if !key.starts_with(prefix) {
                break;
            }
            if key.len() != 40 {
                bail!("Invalid l1->l2 message key length: expected 40, got {}", key.len());
            }
            let nonce = u64::from_be_bytes(key[32..40].try_into().expect("slice len checked"));
            let l2_hash = match val.len() {
                0 => None,
                32 => Some(Felt::from_bytes_be(val[..].try_into().expect("slice len checked"))),
                n => bail!("Invalid l1->l2 message value length: expected 0 or 32, got {n}"),
            };
            out.push((nonce, l2_hash));
        }

        if out.is_empty() {
            Ok(None)
        } else {
            Ok(Some(out))
        }
    }

    pub(super) fn write_l1_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
        l1_tx_hash: &L1TransactionHash,
    ) -> Result<()> {
        let cf = self.get_column(L1_TO_L2_L1_TXN_HASH_BY_NONCE);
        self.db.put_cf_opt(&cf, core_contract_nonce.to_be_bytes(), l1_tx_hash.0, &self.writeopts)?;
        Ok(())
    }

    pub(super) fn get_l1_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<L1TransactionHash>> {
        let cf = self.get_column(L1_TO_L2_L1_TXN_HASH_BY_NONCE);
        let Some(res) = self.db.get_pinned_cf(&cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        if res.len() != 32 {
            bail!("Invalid l1->l2 nonce->l1_tx_hash value length: expected 32, got {}", res.len());
        }
        Ok(Some(L1TransactionHash(res.as_ref().try_into().expect("slice len checked"))))
    }

    /// If the message is already pending, this will overwrite it.
    pub(super) fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        self.db.put_cf_opt(&pending_cf, msg.tx.nonce.to_be_bytes(), super::serialize(&msg)?, &self.writeopts)?;
        Ok(())
    }

    /// If the message does not exist, this does nothing.
    pub(super) fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        self.db.delete_cf_opt(&pending_cf, core_contract_nonce.to_be_bytes(), &self.writeopts)?;
        Ok(())
    }

    pub(super) fn get_pending_message_to_l2(
        &self,
        core_contract_nonce: u64,
    ) -> Result<Option<L1HandlerTransactionWithFee>> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        self.db.get_pinned_cf(&pending_cf, core_contract_nonce.to_be_bytes())?;
        let Some(res) = self.db.get_pinned_cf(&pending_cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        Ok(Some(super::deserialize(&res)?))
    }

    pub(super) fn get_next_pending_message_to_l2(
        &self,
        start_nonce: u64,
    ) -> Result<Option<L1HandlerTransactionWithFee>> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        let binding = start_nonce.to_be_bytes();
        let mode = rocksdb::IteratorMode::From(&binding, rocksdb::Direction::Forward);
        let mut iter = DBIterator::new_cf(&self.db, &pending_cf, ReadOptions::default(), mode)
            .into_iter_values(|v| super::deserialize(v));

        iter.next().transpose()?.transpose().map_err(Into::into)
    }

    pub(super) fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        let Some(res) = self.db.get_pinned_cf(&on_l2_cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        Ok(Some(Felt::from_bytes_be(res[..].try_into().context("Deserializing felt")?)))
    }

    pub(super) fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        self.db.put_cf_opt(&on_l2_cf, core_contract_nonce.to_be_bytes(), txn_hash.to_bytes_be(), &self.writeopts)?;
        Ok(())
    }

    pub(super) fn message_to_l2_remove_txns(
        &self,
        core_contract_nonces: impl IntoIterator<Item = u64>,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        for core_contract_nonce in core_contract_nonces {
            batch.delete_cf(&on_l2_cf, core_contract_nonce.to_be_bytes());
        }
        Ok(())
    }
}
