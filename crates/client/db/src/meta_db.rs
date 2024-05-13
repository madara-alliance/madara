use std::sync::Arc;

use mp_types::block::DHashT;
// Substrate
use parity_scale_codec::{Decode, Encode};

use crate::{Column, DatabaseExt, DbError, DB};

/// Allow interaction with the meta db
///
/// The meta db store the tips of the synced chain.
/// In case of forks, there can be multiple tips.
pub struct MetaDb {
    pub(crate) db: Arc<DB>,
}

const CURRENT_SYNCING_TIPS: &[u8] = b"CURRENT_SYNCING_TIPS";
const CURRENT_SYNC_BLOCK: &[u8] = b"CURRENT_SYNC_BLOCK";

impl MetaDb {
    pub(crate) fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Retrieve the current tips of the synced chain
    pub fn current_syncing_tips(&self) -> Result<Vec<DHashT>, DbError> {
        let column = self.db.get_column(Column::Meta);

        match self.db.get_cf(&column, CURRENT_SYNCING_TIPS)? {
            Some(raw) => Ok(Vec::<DHashT>::decode(&mut &raw[..])?),
            None => Ok(Vec::new()),
        }
    }

    /// Store the current tips of the synced chain
    pub fn write_current_syncing_tips(&self, tips: Vec<DHashT>) -> Result<(), DbError> {
        let column = self.db.get_column(Column::Meta);

        self.db.put_cf(&column, CURRENT_SYNCING_TIPS, tips.encode())?;
        Ok(())
    }

    pub fn current_sync_block(&self) -> Result<u64, DbError> {
        let res = self.db.get_cf(&self.db.get_column(Column::Meta), CURRENT_SYNC_BLOCK)?;
        log::debug!("current_sync_block {res:?}");

        if let Some(res) = res {
            Ok(u64::from_be_bytes(
                res.try_into().map_err(|_| DbError::Format("current sync block should be a u64".into()))?,
            ))
        } else {
            Ok(0)
        }
    }

    pub fn set_current_sync_block(&self, sync_block: u64) -> Result<(), DbError> {
        log::debug!("set_current_sync_block {sync_block}");
        self.db.put_cf(&self.db.get_column(Column::Meta), CURRENT_SYNC_BLOCK, u64::to_be_bytes(sync_block))?;
        Ok(())
    }
}
