use crate::cli::RunCmd;
use crate::OrchestratorError;
use orchestrator_ethereum_da_client::EthereumDaValidatedArgs;

#[derive(Debug, Clone)]
pub enum DAConfig {
    Ethereum(EthereumDaValidatedArgs),
}

impl TryFrom<RunCmd> for DAConfig {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        if !run_cmd.ethereum_da_args.da_on_ethereum {
            return Err(OrchestratorError::SetupCommandError("Ethereum DA is not enabled".to_string()));
        }
        Ok(Self::Ethereum(EthereumDaValidatedArgs {
            ethereum_da_rpc_url: run_cmd
                .ethereum_da_args
                .ethereum_da_rpc_url
                .ok_or_else(|| OrchestratorError::SetupCommandError("Ethereum DA RPC URL is required".to_string()))?,
        }))
    }
}
