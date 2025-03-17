use clap::Args;
use url::Url;

#[derive(Debug, Clone, Args)]
#[group(requires_all = ["ethereum_rpc_url", "ethereum_private_key", "l1_core_contract_address", "starknet_operator_address"])]
pub struct EthereumSettlementCliArgs {
    /// Use the Ethereum settlement layer.
    #[arg(long)]
    pub settle_on_ethereum: bool,

    /// The URL of the Ethereum RPC node.
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", long)]
    pub ethereum_rpc_url: Option<Url>,

    /// The private key of the Ethereum account.
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY", long)]
    pub ethereum_private_key: Option<String>,

    /// The address of the L1 core contract.
    #[arg(env = "MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS", long)]
    pub l1_core_contract_address: Option<String>,

    /// The address of the Starknet operator.
    #[arg(env = "MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS", long)]
    pub starknet_operator_address: Option<String>,
}
