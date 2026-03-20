use crate::cli::RunCmd;
use crate::OrchestratorError;
use orchestrator_ethereum_da_client::EthereumDaValidatedArgs;
use orchestrator_starknet_da_client::StarknetDaValidatedArgs;
use orchestrator_utils::env_utils::resolve_secret_from_file;
use url::Url;

#[derive(Debug, Clone)]
pub enum DAConfig {
    Ethereum(EthereumDaValidatedArgs),
    Starknet(StarknetDaValidatedArgs),
}

impl TryFrom<RunCmd> for DAConfig {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        match (run_cmd.ethereum_da_args.da_on_ethereum, run_cmd.starknet_da_args.da_on_starknet) {
            (true, true) => Err(OrchestratorError::SetupCommandError(
                "Both Ethereum and Starknet DA layers cannot be enabled at the same time".to_string(),
            )),
            (false, false) => {
                Err(OrchestratorError::SetupCommandError("At least one DA layer must be enabled".to_string()))
            }
            (true, false) => {
                // Resolve secret: _FILE env var takes precedence over direct value
                let ethereum_da_rpc_url = match resolve_secret_from_file("MADARA_ORCHESTRATOR_ETHEREUM_DA_RPC_URL")
                    .map_err(OrchestratorError::SetupCommandError)?
                {
                    Some(url_str) => Url::parse(&url_str).map_err(|e| {
                        OrchestratorError::SetupCommandError(format!(
                            "Invalid Ethereum DA RPC URL from secret file: {}",
                            e
                        ))
                    })?,
                    None => run_cmd.ethereum_da_args.ethereum_da_rpc_url.ok_or_else(|| {
                        OrchestratorError::SetupCommandError("Ethereum RPC URL is missing".to_string())
                    })?,
                };

                Ok(DAConfig::Ethereum(EthereumDaValidatedArgs { ethereum_da_rpc_url }))
            }
            (false, true) => {
                // Resolve secret: _FILE env var takes precedence over direct value
                let starknet_da_rpc_url = match resolve_secret_from_file("MADARA_ORCHESTRATOR_STARKNET_DA_RPC_URL")
                    .map_err(OrchestratorError::SetupCommandError)?
                {
                    Some(url_str) => Url::parse(&url_str).map_err(|e| {
                        OrchestratorError::SetupCommandError(format!(
                            "Invalid Starknet DA RPC URL from secret file: {}",
                            e
                        ))
                    })?,
                    None => run_cmd.starknet_da_args.starknet_da_rpc_url.ok_or_else(|| {
                        OrchestratorError::SetupCommandError("Starknet RPC url is missing".to_string())
                    })?,
                };

                Ok(DAConfig::Starknet(StarknetDaValidatedArgs { starknet_da_rpc_url }))
            }
        }
    }
}
