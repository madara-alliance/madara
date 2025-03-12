// Comment out external crate imports
// use ethereum_settlement_client::EthereumSettlementValidatedArgs;
// use starknet_settlement_client::StarknetSettlementValidatedArgs;

use alloy::primitives::Address;
use url::Url;

pub mod ethereum;
pub mod starknet;

// Define local structs for now
#[derive(Debug, Clone)]
pub struct EthereumSettlementValidatedArgs {
    pub ethereum_rpc_url: Url,

    pub ethereum_private_key: String,

    pub l1_core_contract_address: Address,

    pub starknet_operator_address: Address,
}

#[derive(Debug, Clone)]
pub struct StarknetSettlementValidatedArgs {
    pub starknet_rpc_url: Url,
    pub starknet_private_key: String,
    pub starknet_account_address: String,
    pub starknet_cairo_core_contract_address: String,
    pub starknet_finality_retry_wait_in_secs: u64,
}

#[derive(Clone, Debug)]
pub enum SettlementValidatedArgs {
    Ethereum(EthereumSettlementValidatedArgs),
    Starknet(StarknetSettlementValidatedArgs),
}
