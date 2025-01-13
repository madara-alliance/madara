use ethereum_settlement_client::EthereumSettlementValidatedArgs;
use starknet_settlement_client::StarknetSettlementValidatedArgs;

pub mod ethereum;
pub mod starknet;

#[derive(Clone, Debug)]
pub enum SettlementValidatedArgs {
    Ethereum(EthereumSettlementValidatedArgs),
    Starknet(StarknetSettlementValidatedArgs),
}
