use crate::cli::RunCmd;
use crate::OrchestratorError;
use orchestrator_atlantic_service::{types::AtlanticQueryStep, AtlanticValidatedArgs};
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
                if run_cmd.layer == Layer::L3 && atlantic_args.cairo_verifier_program_hash.is_none() {
                    return Err(OrchestratorError::RunCommandError(
                        "Cairo verifier program hash is required for L3".to_string(),
                    ));
                }

                let atlantic_result = atlantic_args
                    .atlantic_verifier_result
                    .ok_or_else(|| OrchestratorError::SetupCommandError("Atlantic result is required".to_string()))?;

                if run_cmd.layer == Layer::L3 && !matches!(&atlantic_result, &AtlanticQueryStep::ProofVerificationOnL2)
                {
                    return Err(OrchestratorError::RunCommandError(
                        "Atlantic result must be PROOF_VERIFICATION_ON_L2 for L3".to_string(),
                    ));
                }

                let atlantic_sharp_prover = atlantic_args.atlantic_sharp_prover.ok_or_else(|| {
                    OrchestratorError::RunCommandError("Atlantic sharp prover is required".to_string())
                })?;

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
                    atlantic_result,
                    cairo_verifier_program_hash: atlantic_args.cairo_verifier_program_hash,
                    atlantic_sharp_prover,
                    atlantic_artifacts_base_url: atlantic_args.atlantic_artifacts_base_url.ok_or_else(|| {
                        OrchestratorError::RunCommandError("Atlantic artifacts base URL is required".to_string())
                    })?,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn l3_atlantic_run_cmd(extra_args: &[&str]) -> RunCmd {
        let mut args = vec![
            "orchestrator",
            "--aws",
            "--aws-s3",
            "--aws-sqs",
            "--aws-sns",
            "--da-on-starknet",
            "--starknet-da-rpc-url",
            "http://localhost:9944",
            "--settle-on-starknet",
            "--starknet-rpc-url",
            "http://localhost:9944",
            "--starknet-private-key",
            "0x1",
            "--starknet-account-address",
            "0x1",
            "--starknet-cairo-core-contract-address",
            "0x1",
            "--starknet-finality-retry-wait-in-secs",
            "1",
            "--atlantic",
            "--atlantic-api-key",
            "test-api-key",
            "--atlantic-service-url",
            "http://localhost:8080",
            "--atlantic-rpc-node-url",
            "http://localhost:9944",
            "--atlantic-verifier-contract-address",
            "0x1",
            "--atlantic-settlement-layer",
            "starknet",
            "--atlantic-mock-fact-hash",
            "true",
            "--atlantic-prover-type",
            "atlantic",
            "--atlantic-network",
            "TESTNET",
            "--cairo-verifier-program-hash",
            "0x123",
            "--madara-rpc-url",
            "http://localhost:9944",
            "--madara-version",
            "0.14.0",
            "--rpc-for-snos",
            "http://localhost:9944",
            "--max-batch-time-seconds",
            "60",
            "--layer",
            "l3",
        ];
        args.extend_from_slice(extra_args);

        RunCmd::try_parse_from(args).expect("valid L3 Atlantic CLI args")
    }

    #[test]
    fn l3_atlantic_requires_proof_verification_on_l2() {
        let err = ProverConfig::try_from(l3_atlantic_run_cmd(&[])).expect_err("default proof generation is invalid");

        assert!(err.to_string().contains("Atlantic result must be PROOF_VERIFICATION_ON_L2 for L3"));
    }

    #[test]
    fn l3_atlantic_accepts_proof_verification_on_l2() {
        let config =
            ProverConfig::try_from(l3_atlantic_run_cmd(&["--atlantic-verifier-result", "proof-verification-on-l2"]))
                .expect("proof verification on L2 should be valid for L3");

        let ProverConfig::Atlantic(args) = config else {
            panic!("expected Atlantic prover config");
        };

        assert!(matches!(args.atlantic_result, AtlanticQueryStep::ProofVerificationOnL2));
    }
}
