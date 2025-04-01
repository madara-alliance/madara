use orchestrator_ethereum_da_client::EthereumDaValidatedArgs;
use crate::cli::da::ethereum::EthereumDaCliArgs;
use crate::OrchestratorError;

#[derive(Debug, Clone)]
pub enum DAConfig {
    Ethereum(EthereumDaValidatedArgs),
}

impl TryFrom<EthereumDaCliArgs> for DAConfig {
    type Error = OrchestratorError;
    fn try_from(ethereum_da_cli_args: EthereumDaCliArgs) -> Result<Self, Self::Error> {
        if !ethereum_da_cli_args.da_on_ethereum {
            return Err(OrchestratorError::SetupCommandError("Ethereum DA is not enabled".to_string()));
        }
        Ok(Self::Ethereum(EthereumDaValidatedArgs {
            ethereum_da_rpc_url: ethereum_da_cli_args.ethereum_da_rpc_url.ok_or_else(|| OrchestratorError::SetupCommandError("Ethereum DA RPC URL is required".to_string()))?,
        }))
    }
}