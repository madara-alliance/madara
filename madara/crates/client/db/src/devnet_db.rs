use crate::{Column, MadaraBackend, MadaraStorageError};
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

pub const DEVNET_KEYS: &[u8] = b"DEVNET_KEYS";

type Result<T, E = MadaraStorageError> = std::result::Result<T, E>;

#[derive(Clone, Serialize, Deserialize)]
pub struct DevnetPredeployedContractAccount {
    pub address: Felt,
    pub secret: Felt,
    pub pubkey: Felt,
    pub class_hash: Felt,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DevnetPredeployedKeys(pub Vec<DevnetPredeployedContractAccount>);

impl MadaraBackend {
    /// Get the devnet predeployed contracts keys.
    #[tracing::instrument(skip(self), fields(module = "DevnetDB"))]
    pub fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        let col = self.db.get_column(Column::Devnet);
        let Some(res) = self.db.get_cf(&col, DEVNET_KEYS)? else {
            return Ok(None);
        };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    /// Set the devnet predeployed contracts keys.
    #[tracing::instrument(skip(self, devnet_keys), fields(module = "DevnetDB"))]
    pub fn set_devnet_predeployed_keys(&self, devnet_keys: DevnetPredeployedKeys) -> Result<()> {
        let nonce_column = self.db.get_column(Column::Devnet);
        self.db.put_cf_opt(&nonce_column, DEVNET_KEYS, bincode::serialize(&devnet_keys)?, &self.writeopts_no_wal)?;
        Ok(())
    }
}
