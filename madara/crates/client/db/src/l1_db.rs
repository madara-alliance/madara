use crate::{Column, DatabaseExt, MadaraBackend, MadaraStorageError, WriteBatchWithTransaction};
use alloy::primitives::U256;
use bigdecimal::ToPrimitive;
use mp_block::header::GasPrices;
use mp_block::L1GasQuote;
use mp_convert::Felt;
use mp_receipt::L1HandlerTransactionReceipt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use num_traits::Zero;

pub const LAST_SYNCED_L1_EVENT_BLOCK: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";

/// We add method in MadaraBackend to be able to handle L1->L2 messaging related data
impl MadaraBackend {
    /// Also removed the given txns from the pending column.
    pub fn l1_db_save_transactions<'a>(
        &self,
        txs: impl IntoIterator<Item = (&'a L1HandlerTransaction, &'a L1HandlerTransactionReceipt)>,
    ) -> Result<(), MadaraStorageError> {
        let mut batch = WriteBatchWithTransaction::default();
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        let on_l2_cf = self.db.get_column(Column::CoreContractNonceToTxnHash);

        for (txn, receipt) in txs {
            let key = txn.nonce.to_be_bytes();
            batch.delete_cf(&pending_cf, key);
            batch.put_cf(&on_l2_cf, key, receipt.transaction_hash.to_bytes_be());
        }

        self.db.write_opt(batch, &self.writeopts_no_wal)?;
        Ok(())
    }

    /// If the message is already pending, this will overwrite it.
    pub fn add_pending_message_to_l2(&self, msg: L1HandlerTransactionWithFee) -> Result<(), MadaraStorageError> {
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        self.db.put_cf_opt(
            &pending_cf,
            msg.tx.nonce.to_be_bytes(),
            bincode::serialize(&msg)?,
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    /// If the message does not exist, this does nothing.
    pub fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<(), MadaraStorageError> {
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        self.db.delete_cf_opt(&pending_cf, core_contract_nonce.to_be_bytes(), &self.writeopts_no_wal)?;
        Ok(())
    }

    pub fn get_pending_message_to_l2(
        &self,
        core_contract_nonce: u64,
    ) -> Result<Option<L1HandlerTransactionWithFee>, MadaraStorageError> {
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        self.db.get_cf(&pending_cf, core_contract_nonce.to_be_bytes())?;
        let Some(res) = self.db.get_pinned_cf(&pending_cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        Ok(Some(bincode::deserialize(&res)?))
    }

    pub fn get_next_pending_message_to_l2(
        &self,
        start_nonce: u64,
    ) -> Result<Option<L1HandlerTransactionWithFee>, MadaraStorageError> {
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        let binding = start_nonce.to_be_bytes();
        let mode = rocksdb::IteratorMode::From(&binding, rocksdb::Direction::Forward);
        let mut iter = self.db.iterator_cf(&pending_cf, mode);

        match iter.next() {
            Some(res) => Ok(Some(bincode::deserialize(&res?.1)?)),
            None => Ok(None),
        }
    }

    pub fn get_l1_handler_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        let on_l2_cf = self.db.get_column(Column::CoreContractNonceToTxnHash);
        let Some(res) = self.db.get_pinned_cf(&on_l2_cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        Ok(Some(Felt::from_bytes_be(
            res[..].try_into().map_err(|_| MadaraStorageError::InconsistentStorage("Malformated felt".into()))?,
        )))
    }

    #[cfg(feature = "testing")]
    pub fn set_l1_handler_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
        txn_hash: Felt,
    ) -> Result<(), MadaraStorageError> {
        let on_l2_cf = self.db.get_column(Column::CoreContractNonceToTxnHash);
        self.db.put_cf_opt(
            &on_l2_cf,
            core_contract_nonce.to_be_bytes(),
            txn_hash.to_bytes_be(),
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    /// Set the latest l1_block synced for the messaging worker.
    pub fn set_l1_messaging_sync_tip(&self, l1_block_n: u64) -> Result<(), MadaraStorageError> {
        let meta_cf = self.db.get_column(Column::BlockStorageMeta);
        self.db.put_cf_opt(&meta_cf, LAST_SYNCED_L1_EVENT_BLOCK, l1_block_n.to_be_bytes(), &self.writeopts_no_wal)?;
        Ok(())
    }

    /// Get the latest l1_block synced for the messaging worker.
    pub fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>, MadaraStorageError> {
        let meta_cf = self.db.get_column(Column::BlockStorageMeta);
        let Some(data) = self.db.get_pinned_cf(&meta_cf, LAST_SYNCED_L1_EVENT_BLOCK)? else { return Ok(None) };
        Ok(Some(u64::from_be_bytes(
            data[..]
                .try_into()
                .map_err(|_| MadaraStorageError::InconsistentStorage("Malformated saved l1_block_n".into()))?,
        )))
    }

    pub fn set_last_l1_gas_quote(&self, l1_gas_quote: L1GasQuote) {
        self.watch_gas_quote.send_replace(Some(l1_gas_quote));
    }

    pub fn get_last_l1_gas_quote(&self) -> Option<L1GasQuote> {
        self.watch_gas_quote.borrow().clone()
    }

    #[cfg(feature = "testing")]
    pub fn set_l1_gas_quote_for_testing(&self) {
        use mp_convert::FixedPoint;

        let l1_gas_quote = L1GasQuote { l1_gas_price: 128, l1_data_gas_price: 128, strk_per_eth: FixedPoint::one() };
        self.set_last_l1_gas_quote(l1_gas_quote);
    }

    pub fn calculate_gas_prices(
        &self,
        previous_strk_l2_gas_price: u128,
        previous_l2_gas_used: u64,
    ) -> anyhow::Result<GasPrices> {

        let l1_gas_quote = self.get_last_l1_gas_quote().ok_or_else(|| {
            anyhow::anyhow!(
                "No L1 gas quote available. Ensure that the L1 gas quote is set before calculating gas prices."
            )
        })?;
        let eth_l1_gas_price = l1_gas_quote.l1_gas_price;
        let eth_l1_data_gas_price = l1_gas_quote.l1_data_gas_price;
        let strk_per_eth = {
            let strk_per_eth = l1_gas_quote.strk_per_eth;
            bigdecimal::BigDecimal::new(strk_per_eth.value().into(), strk_per_eth.decimals().into())
        };
        let strk_l1_gas_price = (&bigdecimal::BigDecimal::from(eth_l1_gas_price) * &strk_per_eth)
            .to_u128()
            .ok_or(anyhow::anyhow!("Failed to convert STRK L1 gas price to u128"))?;
        let strk_l1_data_gas_price = (&bigdecimal::BigDecimal::from(eth_l1_data_gas_price) * &strk_per_eth)
            .to_u128()
            .ok_or(anyhow::anyhow!("Failed to convert STRK L1 data gas price to u128"))?;

        let l2_gas_target = self.chain_config().l2_gas_target;
        let max_change_denominator = self.chain_config().l2_gas_price_max_change_denominator;
        let strk_l2_gas_price = calculate_gas_price(
            previous_strk_l2_gas_price,
            previous_l2_gas_used,
            l2_gas_target,
            max_change_denominator,
        )
            .max(self.chain_config().min_l2_gas_price);
        if strk_per_eth.is_zero() {
            return Err(anyhow::anyhow!("STRK per ETH is zero, cannot calculate gas prices"));
        }
        let eth_l2_gas_price = (&bigdecimal::BigDecimal::from(strk_l2_gas_price) / &strk_per_eth)
            .to_u128()
            .ok_or(anyhow::anyhow!("Failed to convert ETH L2 gas price to u128"))?;

        Ok(GasPrices {
            eth_l1_gas_price,
            strk_l1_gas_price,
            eth_l1_data_gas_price,
            strk_l1_data_gas_price,
            eth_l2_gas_price,
            strk_l2_gas_price,
        })
    }
}

fn calculate_gas_price(
    previous_gas_price: u128,
    previous_gas_used: u64,
    target_gas_used: u64,
    max_change_denominator: u64,
) -> u128 {
    assert!(max_change_denominator > 0, "max_change_denominator must be greater than 0");
    assert!(target_gas_used > 0, "target_gas_used must be greater than 0");
    let delta = previous_gas_used.abs_diff(target_gas_used);
    let price_change = ((U256::from(previous_gas_price)).saturating_mul(U256::from(delta)))
        .checked_div(U256::from((target_gas_used as u128).saturating_mul(max_change_denominator as u128)))
        .expect("Failed to calculate price change")
        .try_into()
        .expect("Failed to convert price change to u128");

    if previous_gas_used > target_gas_used {
        previous_gas_price.saturating_add(price_change)
    } else {
        previous_gas_price.saturating_sub(price_change)
    }
}
