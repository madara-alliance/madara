use crate::cli::RunCmd;
use crate::OrchestratorError;
use alloy::primitives::Address;
use orchestrator_ethereum_settlement_client::EthereumSettlementValidatedArgs;
use orchestrator_starknet_settlement_client::StarknetSettlementValidatedArgs;
use std::str::FromStr as _;

#[derive(Clone, Debug)]
pub enum SettlementConfig {
    Ethereum(EthereumSettlementValidatedArgs),
    Starknet(StarknetSettlementValidatedArgs),
}

impl TryFrom<RunCmd> for SettlementConfig {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        match (run_cmd.ethereum_settlement_args.settle_on_ethereum, run_cmd.starknet_settlement_args.settle_on_starknet)
        {
            (true, true) => Err(OrchestratorError::SetupCommandError(
                "Cannot use both Ethereum and Starknet settlement layers".to_string(),
            )),
            (false, false) => Err(OrchestratorError::SetupCommandError(
                "Must use either Ethereum or Starknet settlement layer".to_string(),
            )),
            (true, false) => {
                let l1_core_contract_address =
                    Address::from_str(&run_cmd.ethereum_settlement_args.l1_core_contract_address.clone().ok_or(
                        OrchestratorError::SetupCommandError("L1 core contract address is required".to_string()),
                    )?)?;
                let max_gas_price_mul_factor = run_cmd.ethereum_settlement_args.max_gas_price_mul_factor;
                let ethereum_operator_address = Address::from_slice(
                    &hex::decode(
                        run_cmd
                            .ethereum_settlement_args
                            .starknet_operator_address
                            .clone()
                            .ok_or_else(|| {
                                OrchestratorError::SetupCommandError(
                                    "Starknet operator address is required".to_string(),
                                )
                            })?
                            .strip_prefix("0x")
                            .ok_or_else(|| {
                                OrchestratorError::SetupCommandError("Invalid Starknet operator address".to_string())
                            })?,
                    )
                    .unwrap_or_else(|_| panic!("Invalid Starknet operator address")),
                );

                let ethereum_params = EthereumSettlementValidatedArgs {
                    ethereum_rpc_url: run_cmd.ethereum_settlement_args.ethereum_rpc_url.clone().ok_or_else(|| {
                        OrchestratorError::SetupCommandError("Ethereum RPC URL is required".to_string())
                    })?,
                    ethereum_private_key: run_cmd.ethereum_settlement_args.ethereum_private_key.clone().ok_or_else(
                        || OrchestratorError::SetupCommandError("Ethereum private key is required".to_string()),
                    )?,
                    l1_core_contract_address,
                    starknet_operator_address: ethereum_operator_address,
                    max_gas_price_mul_factor,
                };
                Ok(Self::Ethereum(ethereum_params))
            }
            (false, true) => {
                let starknet_params = StarknetSettlementValidatedArgs {
                    starknet_rpc_url: run_cmd.starknet_settlement_args.starknet_rpc_url.clone().ok_or_else(|| {
                        OrchestratorError::SetupCommandError("Starknet RPC URL is required".to_string())
                    })?,
                    starknet_private_key: run_cmd.starknet_settlement_args.starknet_private_key.clone().ok_or_else(
                        || OrchestratorError::SetupCommandError("Starknet private key is required".to_string()),
                    )?,
                    starknet_account_address: run_cmd
                        .starknet_settlement_args
                        .starknet_account_address
                        .clone()
                        .ok_or_else(|| {
                            OrchestratorError::SetupCommandError("Starknet account address is required".to_string())
                        })?,
                    starknet_cairo_core_contract_address: run_cmd
                        .starknet_settlement_args
                        .starknet_cairo_core_contract_address
                        .clone()
                        .ok_or_else(|| {
                            OrchestratorError::SetupCommandError(
                                "Starknet Cairo core contract address is required".to_string(),
                            )
                        })?,
                    starknet_finality_retry_wait_in_secs: run_cmd
                        .starknet_settlement_args
                        .starknet_finality_retry_wait_in_secs
                        .unwrap_or(6),
                };
                Ok(Self::Starknet(starknet_params))
            }
        }
    }
}
