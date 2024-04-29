use serde::{Deserialize, Serialize};
use starknet_api::core::{ClassHash, Nonce};

use crate::storage_handler::history::History;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct StorageContractData {
    pub class_hash: History<ClassHash>,
    pub nonce: History<Nonce>,
}
