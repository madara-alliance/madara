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
///
/// Also used by clap as the typed value of the `--prover` / `MADARA_ORCHESTRATOR_PROVER`
/// argument. `rename_all = "snake_case"` makes the accepted values lower-cased
/// (`sharp`, `atlantic`, `mock`), which matches the strings used by `required_if_eq`
/// on the per-prover CLI structs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "snake_case")]
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
        // Prover selection comes from the single typed `--prover` arg
        // (env: `MADARA_ORCHESTRATOR_PROVER`). Clap validates the enum value at
        // parse time, so "exactly one of sharp/atlantic/mock" is already
        // guaranteed — we just dispatch to the right per-prover validator,
        // which in turn checks that its required sub-envs are present.
        match run_cmd.prover {
            ProverKind::Sharp => validate_sharp(run_cmd.sharp_args).map(ProverConfig::Sharp),
            ProverKind::Atlantic => validate_atlantic(run_cmd.atlantic_args, run_cmd.layer).map(ProverConfig::Atlantic),
            ProverKind::Mock => validate_mock(run_cmd.mock_args, &run_cmd.ethereum_settlement_args, run_cmd.layer)
                .map(ProverConfig::Mock),
        }
    }
}

fn validate_sharp(args: SharpCliArgs) -> Result<SharpValidatedArgs, OrchestratorError> {
    Ok(SharpValidatedArgs {
        sharp_customer_id: args
            .sharp_customer_id
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp customer ID is required".to_string()))?,
        sharp_url: args
            .sharp_url
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp URL is required".to_string()))?,
        sharp_user_crt: args
            .sharp_user_crt
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp user certificate is required".to_string()))?,
        sharp_user_key: args
            .sharp_user_key
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp user key is required".to_string()))?,
        sharp_rpc_node_url: args
            .sharp_rpc_node_url
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp RPC node URL is required".to_string()))?,
        sharp_server_crt: args
            .sharp_server_crt
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp server certificate is required".to_string()))?,
        sharp_proof_layout: args
            .sharp_proof_layout
            .ok_or_else(|| OrchestratorError::RunCommandError("Sharp proof layout is required".to_string()))?,
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

    // Prefer the `_FILE` variant when set. This lets deployments keep a dummy
    // value in `MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY` (e.g. "0x123" to
    // satisfy other consumers) while sourcing the real key from a mounted
    // file — typical AWS Secrets Manager / CSI Secrets Store setup.
    // `resolve_secret_from_file` also trims trailing whitespace, which
    // avoids the common `Invalid Ethereum private key: odd number of digits`
    // error when the secret file ends with `\n`.
    let ethereum_private_key = resolve_secret_from_file("MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY")
        .map_err(OrchestratorError::RunCommandError)?
        .or_else(|| eth_args.ethereum_private_key.clone())
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
