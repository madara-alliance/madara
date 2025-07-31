use crate::{
    prelude::*, rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner, WriteBatchWithTransaction}, storage::{ChainTip, DevnetPredeployedKeys, StoredChainInfo}, view::PreconfirmedBlockTransaction
};
use rocksdb::{IteratorMode, ReadOptions};

pub const META_COLUMN: Column = Column::new("meta").set_point_lookup();
pub const PRECONFIRMED_COLUMN: Column = Column::new("preconfirmed");

const META_DEVNET_KEYS_KEY: &[u8] = b"DEVNET_KEYS";
const META_LAST_SYNCED_L1_EVENT_BLOCK_KEY: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";
const META_CONFIRMED_ON_L1_TIP_KEY: &[u8] = b"CONFIRMED_ON_L1_TIP";
const META_CHAIN_TIP_KEY: &[u8] = b"CHAIN_TIP";
const META_CHAIN_INFO_KEY: &[u8] = b"CHAIN_INFO";

impl RocksDBStorageInner {
    /// Set the latest l1_block synced for the messaging worker.
    #[tracing::instrument(skip(self))]
    pub(super) fn write_l1_messaging_sync_tip(&self, block_n: u64) -> Result<()> {
        self.db.put_cf_opt(
            &self.get_column(META_COLUMN),
            META_LAST_SYNCED_L1_EVENT_BLOCK_KEY,
            block_n.to_be_bytes(),
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    /// Get the latest l1_block synced for the messaging worker.
    #[tracing::instrument(skip(self))]
    pub(super) fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>> {
        let Some(data) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_LAST_SYNCED_L1_EVENT_BLOCK_KEY)?
        else {
            return Ok(None);
        };
        Ok(Some(u64::from_be_bytes(data[..].try_into().context("Malformated block_n in DB")?)))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn write_confirmed_on_l1_tip(&self, block_n: u64) -> Result<()> {
        self.db.put_cf_opt(
            &self.get_column(META_COLUMN),
            META_CONFIRMED_ON_L1_TIP_KEY,
            block_n.to_be_bytes(),
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_confirmed_on_l1_tip(&self) -> Result<Option<u64>> {
        let Some(data) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_CONFIRMED_ON_L1_TIP_KEY)? else {
            return Ok(None);
        };
        Ok(Some(u64::from_be_bytes(data[..].try_into().context("Malformated block_n in DB")?)))
    }

    /// Get the devnet predeployed contracts keys.
    #[tracing::instrument(skip(self))]
    pub(super) fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        let Some(res) = self.db.get_cf(&self.get_column(META_COLUMN), META_DEVNET_KEYS_KEY)? else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    /// Set the devnet predeployed contracts keys.
    #[tracing::instrument(skip(self, devnet_keys))]
    pub(super) fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()> {
        self.db.put_cf_opt(
            &self.get_column(META_COLUMN),
            META_DEVNET_KEYS_KEY,
            bincode::serialize(&devnet_keys)?,
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn write_chain_tip(&self, chain_tip: &ChainTip) -> Result<()> {
        self.db.put_cf_opt(
            &self.get_column(META_COLUMN),
            META_CHAIN_TIP_KEY,
            super::serialize_to_smallvec::<[u8; 128]>(chain_tip)?,
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    #[tracing::instrument(skip(self, txs))]
    pub(super) fn append_preconfirmed_content(
        &self,
        start_tx_index: u64,
        txs: &[PreconfirmedBlockTransaction],
    ) -> Result<()> {
        let col = self.get_column(PRECONFIRMED_COLUMN);
        let mut batch = WriteBatchWithTransaction::default();
        for (i, value) in txs.iter().enumerate() {
            let tx_index = start_tx_index + i as u64;
            let tx_index = u16::try_from(tx_index).context("Converting tx_index to u16")?;
            batch.put_cf(&col, &tx_index.to_be_bytes(), bincode::serialize(&value)?);
        }
        self.db.write_opt(batch, &self.writeopts_no_wal)?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_chain_tip(&self) -> Result<Option<ChainTip>> {
        let Some(res) = self.db.get_cf(&self.get_column(META_COLUMN), META_CHAIN_TIP_KEY)? else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_preconfirmed_content(&self) -> impl Iterator<Item = Result<PreconfirmedBlockTransaction>> + '_ {
        let iter = DBIterator::new_cf(
            &self.db,
            &self.get_column(PRECONFIRMED_COLUMN),
            ReadOptions::default(),
            IteratorMode::Start,
        )
        .into_iter_values(|bytes| bincode::deserialize::<PreconfirmedBlockTransaction>(bytes))
        .map(|res| Ok(res??));
        iter
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_stored_chain_info(&self) -> Result<Option<StoredChainInfo>> {
        let Some(res) = self.db.get_cf(&self.get_column(META_COLUMN), META_CHAIN_INFO_KEY)? else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }
}
