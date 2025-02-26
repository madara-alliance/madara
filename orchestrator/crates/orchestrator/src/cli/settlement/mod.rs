use orchestrator_ethereum_settlement_client::EthereumSettlementValidatedArgs;
use orchestrator_starknet_settlement_client::StarknetSettlementValidatedArgs;

pub mod ethereum;
pub mod starknet;

#[derive(Clone, Debug)]
pub enum SettlementValidatedArgs {
    Ethereum(EthereumSettlementValidatedArgs),
    Starknet(StarknetSettlementValidatedArgs),
}
