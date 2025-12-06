use crate::cli::RunCmd;
use crate::OrchestratorError;
use orchestrator_atlantic_service::AtlanticValidatedArgs;
use orchestrator_sharp_service::SharpValidatedArgs;
use orchestrator_utils::layer::Layer;

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
                // NOTE: Just making sure Cairo Verifier Program Hash is there for L3
                if run_cmd.layer.as_ref() == Some(&Layer::L3) && atlantic_args.cairo_verifier_program_hash.is_none() {
                    return Err(OrchestratorError::RunCommandError(
                        "Cairo verifier program hash is required for L3".to_string(),
                    ));
                }
                Ok(Self::Atlantic(AtlanticValidatedArgs {
                    atlantic_api_key: atlantic_args.atlantic_api_key.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic API key is required".to_string())
                    })?,
                    atlantic_service_url: atlantic_args
                        .atlantic_service_url
                        .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic URL is required".to_string()))?,
                    atlantic_rpc_node_url: atlantic_args.atlantic_rpc_node_url.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic RPC node URL is required".to_string())
                    })?,
                    atlantic_verifier_contract_address: atlantic_args.atlantic_verifier_contract_address.ok_or_else(
                        || {
                            OrchestratorError::RunCommandError(
                                "Atlantic verifier contract address is required".to_string(),
                            )
                        },
                    )?,
                    atlantic_settlement_layer: atlantic_args.atlantic_settlement_layer.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic settlement layer is required".to_string())
                    })?,
                    atlantic_mock_fact_hash: atlantic_args.atlantic_mock_fact_hash.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic mock fact hash is required".to_string())
                    })?,
                    atlantic_prover_type: atlantic_args.atlantic_prover_type.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic prover type is required".to_string())
                    })?,
                    atlantic_network: atlantic_args.atlantic_network.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic network is required".to_string())
                    })?,
                    atlantic_cairo_vm: atlantic_args.atlantic_verifier_cairo_vm.ok_or_else(|| {
                        OrchestratorError::SetupCommandError("Atlantic cairo vm is required".to_string())
                    })?,
                    atlantic_result: atlantic_args.atlantic_verifier_result.ok_or_else(|| {
                        OrchestratorError::SetupCommandError("Atlantic result is required".to_string())
                    })?,
                    cairo_verifier_program_hash: atlantic_args.cairo_verifier_program_hash,
                }))
            }
        }
    }
}
