use std::path::Path;

use crate::cli::prover::mock::MockCliArgs;
use crate::cli::prover::sharp::SharpCliArgs;
use crate::cli::settlement::ethereum::EthereumSettlementCliArgs;
use crate::cli::RunCmd;
use crate::OrchestratorError;
use alloy::primitives::Address;
use orchestrator_mock_service::MockValidatedArgs;
use orchestrator_sharp_service::SharpValidatedArgs;
use orchestrator_utils::env_utils::resolve_secret_from_file;
use orchestrator_utils::layer::Layer;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum ProverConfig {
    Sharp(SharpValidatedArgs),
    Mock(MockValidatedArgs),
}

/// Lightweight tag for selecting prover-specific code paths in handlers.
///
/// Prefer this over matching `ProverConfig` directly when the handler only needs
/// to know *which* prover is active, not its full validated config.
///
/// Also used by clap as the typed value of the `--prover` / `MADARA_ORCHESTRATOR_PROVER`
/// argument. `rename_all = "snake_case"` makes the accepted values lower-cased
/// (`sharp`, `mock`), which matches the strings used by `required_if_eq`
/// on the per-prover CLI structs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "snake_case")]
pub enum ProverKind {
    Sharp,
    Mock,
}

impl ProverConfig {
    pub fn kind(&self) -> ProverKind {
        match self {
            ProverConfig::Sharp(_) => ProverKind::Sharp,
            ProverConfig::Mock(_) => ProverKind::Mock,
        }
    }
}

impl ProverKind {
    /// Bounded, stable label for logs and Prometheus metrics.
    pub fn label(self) -> &'static str {
        match self {
            ProverKind::Sharp => "sharp",
            ProverKind::Mock => "mock",
        }
    }
}

impl TryFrom<RunCmd> for ProverConfig {
    type Error = OrchestratorError;
    fn try_from(run_cmd: RunCmd) -> Result<Self, Self::Error> {
        // Prover selection comes from the single typed `--prover` arg
        // (env: `MADARA_ORCHESTRATOR_PROVER`). Clap validates the enum value at
        // parse time, so "exactly one of sharp/mock" is already
        // guaranteed — we just dispatch to the right per-prover validator,
        // which in turn checks that its required sub-envs are present.
        match run_cmd.prover {
            ProverKind::Sharp => validate_sharp(run_cmd.sharp_args, run_cmd.layer).map(ProverConfig::Sharp),
            ProverKind::Mock => validate_mock(
                run_cmd.mock_args,
                &run_cmd.ethereum_settlement_args,
                run_cmd.layer,
                run_cmd.store_audit_artifacts,
            )
            .map(ProverConfig::Mock),
        }
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

fn validate_mock(
    args: MockCliArgs,
    eth_args: &EthereumSettlementCliArgs,
    layer: Layer,
    store_audit_artifacts: bool,
) -> Result<MockValidatedArgs, OrchestratorError> {
    if layer == Layer::L3 {
        return Err(OrchestratorError::RunCommandError("Mock prover is L2-only".to_string()));
    }
    if store_audit_artifacts {
        return Err(OrchestratorError::RunCommandError(
            "Mock prover does not produce proofs; --store-audit-artifacts is not supported with --mock".to_string(),
        ));
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

/// Read PEM content eagerly so invalid paths fail at startup instead of on first request.
fn read_pem_file(label: &str, path: &Path) -> Result<String, OrchestratorError> {
    std::fs::read_to_string(path).map_err(|e| {
        OrchestratorError::RunCommandError(format!("Failed to read {} file at {}: {}", label, path.display(), e))
    })
}
