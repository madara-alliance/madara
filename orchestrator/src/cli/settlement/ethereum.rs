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

    /// The amount of time in seconds to wait for state update txns
    /// Doesn't require an env variable
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_FINALITY_RETRY_WAIT_IN_SECS", long, default_value = "60")]
    pub ethereum_finality_retry_wait_in_secs: Option<u64>,

    #[arg(env = "MADARA_ORCHESTRATOR_EIP1559_MAX_GAS_MUL_FACTOR", long, default_value = "1.5")]
    pub max_gas_price_mul_factor: f64,

    /// Disable PeerDAS (PeerDAS is a feature introduced in Fusaka upgrade which changes the way we settle on Ethereum).
    /// https://ethereum.org/roadmap/fusaka
    /// https://notes.ethereum.org/@fradamt/das-fork-choice
    /// Whether settling on Ethereum mainnet (true) or Sepolia testnet (false).
    /// Mainnet uses blob proofs (pre-Fusaka), Sepolia uses cell proofs (post-Fusaka).
    #[arg(env = "MADARA_ORCHESTRATOR_ETHEREUM_DISABLE_PEERDAS", long, default_value = "false")]
    pub disable_peerdas: bool,
}
