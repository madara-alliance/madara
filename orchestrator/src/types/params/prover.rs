use std::path::Path;

use crate::cli::prover::atlantic::AtlanticCliArgs;
use crate::cli::prover::mock::MockCliArgs;
use crate::cli::prover::sharp::SharpCliArgs;
use crate::cli::settlement::ethereum::EthereumSettlementCliArgs;
use crate::cli::RunCmd;
use crate::OrchestratorError;
use alloy::primitives::Address;
use orchestrator_atlantic_service::AtlanticValidatedArgs;
use orchestrator_mock_service::MockValidatedArgs;
use orchestrator_sharp_service::SharpValidatedArgs;
use orchestrator_utils::env_utils::resolve_secret_from_file;
use orchestrator_utils::layer::Layer;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum ProverConfig {
    Sharp(SharpValidatedArgs),
    Atlantic(AtlanticValidatedArgs),
    Mock(MockValidatedArgs),
}

/// Lightweight tag for selecting prover-specific code paths in handlers.
///
/// Prefer this over matching `ProverConfig` directly when the handler only needs
/// to know *which* prover is active, not its full validated config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProverKind {
    Sharp,
    Atlantic,
    Mock,
}

impl ProverConfig {
    pub fn kind(&self) -> ProverKind {
        match self {
            ProverConfig::Sharp(_) => ProverKind::Sharp,
            ProverConfig::Atlantic(_) => ProverKind::Atlantic,
            ProverConfig::Mock(_) => ProverKind::Mock,
        }
    }
}

impl TryFrom<RunCmd> for ProverConfig {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        let selected = [run_cmd.sharp_args.sharp, run_cmd.atlantic_args.atlantic, run_cmd.mock_args.mock]
            .iter()
            .filter(|&&x| x)
            .count();
        if selected > 1 {
            return Err(OrchestratorError::RunCommandError(
                "Only one of --sharp, --atlantic, --mock may be selected".to_string(),
            ));
        }
        if selected == 0 {
            return Err(OrchestratorError::RunCommandError(
                "Must select one of --sharp, --atlantic, --mock".to_string(),
            ));
        }

        if run_cmd.sharp_args.sharp {
            return validate_sharp(run_cmd.sharp_args, run_cmd.layer).map(ProverConfig::Sharp);
        }

        if run_cmd.atlantic_args.atlantic {
            return validate_atlantic(run_cmd.atlantic_args, run_cmd.layer).map(ProverConfig::Atlantic);
        }

        validate_mock(run_cmd.mock_args, &run_cmd.ethereum_settlement_args, run_cmd.layer).map(ProverConfig::Mock)
    }
}

fn validate_sharp(args: SharpCliArgs, layer: Layer) -> Result<SharpValidatedArgs, OrchestratorError> {
    if layer == Layer::L3 {
        return Err(OrchestratorError::RunCommandError(
            "SHARP prover is L2-only (fact registration happens on L1)".to_string(),
        ));
    }

    let user_crt_path = args
        .sharp_user_crt_file
        .ok_or_else(|| OrchestratorError::RunCommandError("Sharp user certificate file is required".to_string()))?;
    let user_key_path = args
        .sharp_user_key_file
        .ok_or_else(|| OrchestratorError::RunCommandError("Sharp user key file is required".to_string()))?;
    let server_crt_path = args
        .sharp_server_crt_file
        .ok_or_else(|| OrchestratorError::RunCommandError("Sharp server certificate file is required".to_string()))?;

    Ok(SharpValidatedArgs {
        sharp_customer_id: args
            .sharp_customer_id
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp customer ID is required".to_string()))?,
        sharp_url: args
            .sharp_url
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp URL is required".to_string()))?,
        sharp_user_crt: read_pem_file("Sharp user certificate", &user_crt_path)?,
        sharp_user_key: read_pem_file("Sharp user key", &user_key_path)?,
        sharp_rpc_node_url: args
            .sharp_rpc_node_url
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp RPC node URL is required".to_string()))?,
        sharp_server_crt: read_pem_file("Sharp server certificate", &server_crt_path)?,
        gps_verifier_contract_address: args.gps_verifier_contract_address.ok_or_else(|| {
            OrchestratorError::RunCommandError("GPS verifier contract address is required".to_string())
        })?,
        sharp_settlement_layer: args
            .sharp_settlement_layer
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp settlement layer is required".to_string()))?,
    })
}

fn validate_atlantic(args: AtlanticCliArgs, layer: Layer) -> Result<AtlanticValidatedArgs, OrchestratorError> {
    if layer == Layer::L3 && args.cairo_verifier_program_hash.is_none() {
        return Err(OrchestratorError::RunCommandError("Cairo verifier program hash is required for L3".to_string()));
    }

    let atlantic_api_key = resolve_secret_from_file("MADARA_ORCHESTRATOR_ATLANTIC_API_KEY")
        .map_err(OrchestratorError::RunCommandError)?
        .or(args.atlantic_api_key)
        .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic API key is required".to_string()))?;

    Ok(AtlanticValidatedArgs {
        atlantic_api_key,
        atlantic_service_url: args
            .atlantic_service_url
            .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic URL is required".to_string()))?,
        atlantic_rpc_node_url: args
            .atlantic_rpc_node_url
            .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic RPC node URL is required".to_string()))?,
        atlantic_verifier_contract_address: args.atlantic_verifier_contract_address.ok_or_else(|| {
            OrchestratorError::RunCommandError("Atlantic verifier contract address is required".to_string())
        })?,
        atlantic_settlement_layer: args
            .atlantic_settlement_layer
            .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic settlement layer is required".to_string()))?,
        atlantic_mock_fact_hash: args
            .atlantic_mock_fact_hash
            .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic mock fact hash is required".to_string()))?,
        atlantic_prover_type: args
            .atlantic_prover_type
            .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic prover type is required".to_string()))?,
        atlantic_network: args
            .atlantic_network
            .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic network is required".to_string()))?,
        atlantic_cairo_vm: args
            .atlantic_verifier_cairo_vm
            .ok_or_else(|| OrchestratorError::SetupCommandError("Atlantic cairo vm is required".to_string()))?,
        atlantic_result: args
            .atlantic_verifier_result
            .ok_or_else(|| OrchestratorError::SetupCommandError("Atlantic result is required".to_string()))?,
        cairo_verifier_program_hash: args.cairo_verifier_program_hash,
        atlantic_sharp_prover: args
            .atlantic_sharp_prover
            .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic sharp prover is required".to_string()))?,
        atlantic_artifacts_base_url: args
            .atlantic_artifacts_base_url
            .ok_or_else(|| OrchestratorError::RunCommandError("Atlantic artifacts base URL is required".to_string()))?,
    })
}

fn validate_mock(
    args: MockCliArgs,
    eth_args: &EthereumSettlementCliArgs,
    layer: Layer,
) -> Result<MockValidatedArgs, OrchestratorError> {
    if layer == Layer::L3 {
        return Err(OrchestratorError::RunCommandError("Mock prover is L2-only".to_string()));
    }

    let ethereum_rpc_url = eth_args
        .ethereum_rpc_url
        .clone()
        .ok_or_else(|| OrchestratorError::RunCommandError("Mock prover requires Ethereum RPC URL".to_string()))?;
    let ethereum_private_key = eth_args
        .ethereum_private_key
        .clone()
        .ok_or_else(|| OrchestratorError::RunCommandError("Mock prover requires Ethereum private key".to_string()))?;
    // Parse once here so malformed keys surface as a clean CLI validation error
    let ethereum_signer: alloy::signers::local::PrivateKeySigner = ethereum_private_key
        .parse()
        .map_err(|e| OrchestratorError::RunCommandError(format!("Invalid Ethereum private key: {e}")))?;

    let verifier_address = match args.mock_verifier_address {
        Some(s) => Some(Address::from_str(&s).map_err(|e| {
            OrchestratorError::RunCommandError(format!("Invalid MADARA_ORCHESTRATOR_MOCK_VERIFIER_ADDRESS: {e}"))
        })?),
        None => None,
    };

    Ok(MockValidatedArgs { verifier_address, ethereum_rpc_url, ethereum_signer })
}

/// Read PEM content eagerly so invalid paths fail at startup instead of on first request.
fn read_pem_file(label: &str, path: &Path) -> Result<String, OrchestratorError> {
    std::fs::read_to_string(path).map_err(|e| {
        OrchestratorError::RunCommandError(format!("Failed to read {} file at {}: {}", label, path.display(), e))
    })
}
