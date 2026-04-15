use std::path::Path;

use crate::cli::RunCmd;
use crate::OrchestratorError;
use orchestrator_atlantic_service::AtlanticValidatedArgs;
use orchestrator_sharp_service::SharpValidatedArgs;
use orchestrator_utils::env_utils::resolve_secret_from_file;
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

                // mTLS material is file-only. clap already enforced presence via
                // required_if_eq, so these Options must be Some here.
                let user_crt_path = sharp_args.sharp_user_crt_file.ok_or_else(|| {
                    OrchestratorError::RunCommandError("Sharp user certificate file is required".to_string())
                })?;
                let user_key_path = sharp_args
                    .sharp_user_key_file
                    .ok_or_else(|| OrchestratorError::RunCommandError("Sharp user key file is required".to_string()))?;
                let server_crt_path = sharp_args.sharp_server_crt_file.ok_or_else(|| {
                    OrchestratorError::RunCommandError("Sharp server certificate file is required".to_string())
                })?;

                // Read the PEM files eagerly so wrong paths fail at startup rather
                // than at first SHARP request.
                let sharp_user_crt = read_pem_file("Sharp user certificate", &user_crt_path)?;
                let sharp_user_key = read_pem_file("Sharp user key", &user_key_path)?;
                let sharp_server_crt = read_pem_file("Sharp server certificate", &server_crt_path)?;

                Ok(Self::Sharp(SharpValidatedArgs {
                    sharp_customer_id: sharp_args.sharp_customer_id.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp customer ID is required".to_string())
                    })?,
                    sharp_url: sharp_args
                        .sharp_url
                        .ok_or_else(|| OrchestratorError::RunCommandError("Sharp URL is required".to_string()))?,
                    sharp_user_crt,
                    sharp_user_key,
                    sharp_rpc_node_url: sharp_args.sharp_rpc_node_url.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp RPC node URL is required".to_string())
                    })?,
                    sharp_server_crt,
                    gps_verifier_contract_address: sharp_args.gps_verifier_contract_address.ok_or_else(|| {
                        OrchestratorError::RunCommandError("GPS verifier contract address is required".to_string())
                    })?,
                    sharp_settlement_layer: sharp_args.sharp_settlement_layer.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Sharp settlement layer is required".to_string())
                    })?,
                    sharp_offchain_proof: sharp_args.sharp_offchain_proof.unwrap_or(false),
                }))
            }
            (false, true) => {
                let atlantic_args = run_cmd.atlantic_args;
                // NOTE: Just making sure Cairo Verifier Program Hash is there for L3
                if run_cmd.layer == Layer::L3 && atlantic_args.cairo_verifier_program_hash.is_none() {
                    return Err(OrchestratorError::RunCommandError(
                        "Cairo verifier program hash is required for L3".to_string(),
                    ));
                }
                // Resolve secret: _FILE env var takes precedence over direct value
                let atlantic_api_key = resolve_secret_from_file("MADARA_ORCHESTRATOR_ATLANTIC_API_KEY")
                    .map_err(OrchestratorError::RunCommandError)?
                    .or(atlantic_args.atlantic_api_key)
                    .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic API key is required".to_string()))?;

                Ok(Self::Atlantic(AtlanticValidatedArgs {
                    atlantic_api_key,
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
                    atlantic_sharp_prover: atlantic_args.atlantic_sharp_prover.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic sharp prover is required".to_string())
                    })?,
                    atlantic_artifacts_base_url: atlantic_args.atlantic_artifacts_base_url.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic artifacts base URL is required".to_string())
                    })?,
                }))
            }
        }
    }
}

/// Read a PEM file eagerly at startup so misconfigurations fail fast.
/// Returns the raw UTF-8 content (PEM is ASCII).
fn read_pem_file(label: &str, path: &Path) -> Result<String, OrchestratorError> {
    std::fs::read_to_string(path).map_err(|e| {
        OrchestratorError::RunCommandError(format!("Failed to read {} file at {}: {}", label, path.display(), e))
    })
}
