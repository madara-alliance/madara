//! MongoDB document models for mempool transactions.

use serde::{Deserialize, Serialize};

/// Resource bounds for V3 transactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceBoundsDoc {
    pub l1_gas_max_amount: u64,
    pub l1_gas_max_price: String,
    pub l2_gas_max_amount: u64,
    pub l2_gas_max_price: String,
    pub l1_data_gas_max_amount: Option<u64>,
    pub l1_data_gas_max_price: Option<String>,
}

/// MongoDB document for a mempool transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolTransactionDocument {
    /// Unique document id (UUID)
    #[serde(rename = "_id")]
    pub id: bson::Binary,

    /// Transaction hash (indexed, non-unique)
    pub tx_hash: String,

    /// Transaction type: "INVOKE", "DECLARE", "DEPLOY_ACCOUNT", "L1_HANDLER", "DEPLOY"
    pub tx_type: String,

    /// Transaction version: "V0", "V1", "V2", "V3"
    pub tx_version: String,

    /// Sender address (indexed)
    pub sender_address: String,

    /// Contract address (for deploy/L1Handler, same as sender for others)
    pub contract_address: String,

    /// Transaction nonce (as hex string)
    pub nonce: String,

    /// Block number when transaction was included (if known)
    pub block_number: Option<u64>,

    /// Arrival timestamp in mempool (indexed)
    pub arrived_at: bson::DateTime,

    /// Insertion timestamp into MongoDB
    pub inserted_at: bson::DateTime,

    /// Fee information (V0-V2)
    pub max_fee: Option<String>,

    /// Tip (V3 only)
    pub tip: Option<u64>,

    /// Resource bounds (V3 only)
    pub resource_bounds: Option<ResourceBoundsDoc>,

    /// Transaction signature
    pub signature: Vec<String>,

    /// Calldata (for Invoke transactions)
    pub calldata: Option<Vec<String>>,

    /// Class hash (for Declare/DeployAccount)
    pub class_hash: Option<String>,

    /// Compiled class hash (for Declare V2+)
    pub compiled_class_hash: Option<String>,

    /// Constructor calldata (for DeployAccount)
    pub constructor_calldata: Option<Vec<String>>,

    /// Contract address salt (for DeployAccount)
    pub contract_address_salt: Option<String>,

    /// Entry point selector (for L1Handler)
    pub entry_point_selector: Option<String>,

    /// Paid fee on L1 (for L1Handler only)
    pub paid_fee_on_l1: Option<String>,

    /// Full serialized ValidatedTransaction as bincode (for replay capability)
    pub raw_transaction: bson::Binary,

    /// Chain ID for multi-chain deployments
    pub chain_id: String,

    /// Status: "PENDING" (insert-only, can be extended later)
    pub status: String,

    /// Whether fee charging is enabled
    pub charge_fee: bool,
}
