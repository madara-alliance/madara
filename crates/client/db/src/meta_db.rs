use std::sync::Arc;

use deoxys_runtime::opaque::DHashT;
// Substrate
use parity_scale_codec::{Decode, Encode};

use crate::{Column, DbError, DB, DatabaseExt};

/// Allow interaction with the meta db
///
/// The meta db store the tips of the synced chain.
/// In case of forks, there can be multiple tips.
pub struct MetaDb {
    pub(crate) db: Arc<DB>,
}

impl MetaDb {
    pub(crate) fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Retrieve the current tips of the synced chain
    pub fn current_syncing_tips(&self) -> Result<Vec<DHashT>, DbError> {
        let column = self.db.get_column(Column::Meta);

        match self.db.get_cf(&column, crate::static_keys::CURRENT_SYNCING_TIPS)? {
            Some(raw) => Ok(Vec::<DHashT>::decode(&mut &raw[..])?),
            None => Ok(Vec::new()),
        }
    }

    /// Store the current tips of the synced chain
    pub fn write_current_syncing_tips(&self, tips: Vec<DHashT>) -> Result<(), DbError> {
        let column = self.db.get_column(Column::Meta);

        self.db.put_cf(&column, crate::static_keys::CURRENT_SYNCING_TIPS, &tips.encode())?;
        Ok(())
    }
}
