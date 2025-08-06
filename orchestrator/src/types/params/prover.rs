use crate::cli::RunCmd;
use crate::OrchestratorError;
use orchestrator_atlantic_service::AtlanticValidatedArgs;
use orchestrator_sharp_service::SharpValidatedArgs;

#[derive(Debug, Clone)]
pub enum ProverConfig {
    Sharp(SharpValidatedArgs),
    Atlantic(AtlanticValidatedArgs),
}

impl TryFrom<RunCmd> for ProverConfig {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        match (run_cmd.sharp_args.sharp, run_cmd.atlantic_args.atlantic) {
            (true, true) => {
                Err(OrchestratorError::RunCommandError("Cannot use both Sharp and Atlantic provers".to_string()))
            }
            (false, false) => {
                Err(OrchestratorError::RunCommandError("Must use either Sharp or Atlantic prover".to_string()))
            }
            (true, false) => {
                let sharp_args = run_cmd.sharp_args;
                Ok(Self::Sharp(SharpValidatedArgs {
                    sharp_customer_id: sharp_args.sharp_customer_id.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp customer ID is required".to_string())
                    })?,
                    sharp_url: sharp_args
                        .sharp_url
                        .ok_or_else(|| OrchestratorError::RunCommandError("Sharp URL is required".to_string()))?,
                    sharp_user_crt: sharp_args.sharp_user_crt.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp user certificate is required".to_string())
                    })?,
                    sharp_user_key: sharp_args
                        .sharp_user_key
                        .ok_or_else(|| OrchestratorError::RunCommandError("Sharp user key is required".to_string()))?,
                    sharp_rpc_node_url: sharp_args.sharp_rpc_node_url.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp RPC node URL is required".to_string())
                    })?,
                    sharp_server_crt: sharp_args.sharp_server_crt.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp server certificate is required".to_string())
                    })?,
                    sharp_proof_layout: sharp_args.sharp_proof_layout.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp proof layout is required".to_string())
                    })?,
                    gps_verifier_contract_address: sharp_args.gps_verifier_contract_address.ok_or_else(|| {
                        OrchestratorError::RunCommandError("GPS verifier contract address is required".to_string())
                    })?,
                    sharp_settlement_layer: sharp_args.sharp_settlement_layer.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp settlement layer is required".to_string())
                    })?,
                }))
            }
            (false, true) => {
                let atlantic_args = run_cmd.atlantic_args;

                // Check if mock mode is enabled
                let mock_fact_hash = atlantic_args.atlantic_mock_fact_hash.ok_or_else(|| {
                    OrchestratorError::RunCommandError("Atlantic mock fact hash is required".to_string())
                })?;

                let is_mock_mode = mock_fact_hash.eq("true");

                if is_mock_mode {
                    // Mock mode: provide sensible defaults for missing parameters
                    Ok(Self::Atlantic(AtlanticValidatedArgs {
                        atlantic_api_key: atlantic_args.atlantic_api_key.unwrap_or_else(|| "mock-api-key".to_string()),
                        atlantic_service_url: atlantic_args
                            .atlantic_service_url
                            .unwrap_or_else(|| "http://unused-in-mock-mode.com".parse().unwrap()),
                        atlantic_rpc_node_url: atlantic_args
                            .atlantic_rpc_node_url
                            .unwrap_or_else(|| "http://unused-in-mock-mode.com".parse().unwrap()),
                        atlantic_verifier_contract_address: atlantic_args
                            .atlantic_verifier_contract_address
                            .unwrap_or_else(|| "0x0000000000000000000000000000000000000000".to_string()),
                        atlantic_settlement_layer: atlantic_args
                            .atlantic_settlement_layer
                            .unwrap_or_else(|| "ethereum".to_string()),
                        atlantic_mock_fact_hash: mock_fact_hash,
                        atlantic_prover_type: atlantic_args
                            .atlantic_prover_type
                            .unwrap_or_else(|| "starkware".to_string()),
                        atlantic_network: atlantic_args.atlantic_network.unwrap_or_else(|| "TESTNET".to_string()),
                        cairo_verifier_program_hash: None,
                    }))
                } else {
                    Ok(Self::Atlantic(AtlanticValidatedArgs {
                        atlantic_api_key: atlantic_args.atlantic_api_key.ok_or_else(|| {
                            OrchestratorError::RunCommandError(
                                "Atlantic API key is required for production mode".to_string(),
                            )
                        })?,
                        atlantic_service_url: atlantic_args.atlantic_service_url.ok_or_else(|| {
                            OrchestratorError::RunCommandError(
                                "Atlantic service URL is required for production mode".to_string(),
                            )
                        })?,
                        atlantic_rpc_node_url: atlantic_args.atlantic_rpc_node_url.ok_or_else(|| {
                            OrchestratorError::RunCommandError(
                                "Atlantic RPC node URL is required for production mode".to_string(),
                            )
                        })?,
                        atlantic_verifier_contract_address: atlantic_args
                            .atlantic_verifier_contract_address
                            .ok_or_else(|| {
                                OrchestratorError::RunCommandError(
                                    "Atlantic verifier contract address is required for production mode".to_string(),
                                )
                            })?,
                        atlantic_settlement_layer: atlantic_args.atlantic_settlement_layer.ok_or_else(|| {
                            OrchestratorError::RunCommandError(
                                "Atlantic settlement layer is required for production mode".to_string(),
                            )
                        })?,
                        atlantic_mock_fact_hash: mock_fact_hash,
                        atlantic_prover_type: atlantic_args.atlantic_prover_type.ok_or_else(|| {
                            OrchestratorError::RunCommandError(
                                "Atlantic prover type is required for production mode".to_string(),
                            )
                        })?,
                        atlantic_network: atlantic_args.atlantic_network.ok_or_else(|| {
                            OrchestratorError::RunCommandError(
                                "Atlantic network is required for production mode".to_string(),
                            )
                        })?,
                        cairo_verifier_program_hash: atlantic_args.cairo_verifier_program_hash,
                    }))
                }
            }
        }
    }
}
