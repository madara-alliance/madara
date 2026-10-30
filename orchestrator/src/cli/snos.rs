use std::path::PathBuf;

use blockifier::blockifier_versioned_constants::VersionedConstants;
use clap::Args;
use generate_pie::utils::load_versioned_constants;
use url::Url;

pub(crate) fn parse_constants(path: &str) -> Result<VersionedConstants, String> {
    load_versioned_constants(Some(path))?
        .ok_or_else(|| format!("Failed to load versioned constants from file: {}", PathBuf::from(path).display()))
}

/// Rejects unsupported witness transports at startup instead of failing every SNOS job later.
fn parse_committed_data_rpc_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "Invalid committed-data RPC URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Committed-data witness RPC requires HTTP or HTTPS".into());
    }
    Ok(url)
}

#[derive(Debug, Clone, Args)]
#[group(requires_all = ["rpc_for_snos"])]
pub struct SNOSCliArgs {
    /// Permit committed-data reads during SNOS account replay; disabled reads reject execution.
    /// This does not make L1-handler reads supported by the pinned sequencer.
    #[arg(env = "MADARA_ORCHESTRATOR_USE_COMMITTED_DATA", long, default_value_t = false)]
    pub use_committed_data: bool,

    /// Madara feeder-gateway base URL ending in /feeder_gateway/, supplying exact-root witnesses.
    /// Separate from Pathfinder. Legacy JSON-RPC witness endpoints remain supported by SNOS.
    #[arg(env = "MADARA_ORCHESTRATOR_COMMITTED_DATA_RPC_URL", long, requires = "use_committed_data", value_parser = parse_committed_data_rpc_url)]
    pub committed_data_rpc_url: Option<Url>,

    /// Whether to use full output or not
    #[arg(env = "MADARA_ORCHESTRATOR_SNOS_FULL_OUTPUT", long, default_value = "false")]
    pub snos_full_output: bool,

    /// The RPC URL for SNOS.
    #[arg(env = "MADARA_ORCHESTRATOR_RPC_FOR_SNOS", long)]
    pub rpc_for_snos: Url,

    /// Optional backup RPC URL for retried SNOS jobs.
    #[arg(env = "MADARA_ORCHESTRATOR_RPC_FOR_SNOS_BACKUP", long)]
    pub rpc_for_snos_backup: Option<Url>,

    /// Path to a JSON file containing versioned constants to override the default Starknet constants.
    /// By default, versioned constants are picked from the official Starknet constants loaded in blockifier.
    /// Use this argument to override those defaults with custom versioned constants from a file.
    #[arg(env = "MADARA_ORCHESTRATOR_VERSIONED_CONSTANTS_PATH", long, required = false, value_parser = parse_constants)]
    pub versioned_constants: Option<VersionedConstants>,
}

impl SNOSCliArgs {
    /// Validate the committed-data execution path before constructing runtime services.
    /// SHARP and Mock share local SNOS replay and aggregation; Mock substitutes proof submission.
    pub(crate) fn validate_committed_data(
        &self,
        prover: crate::types::params::prover::ProverKind,
        layer: &crate::types::Layer,
    ) -> anyhow::Result<()> {
        use crate::types::{params::prover::ProverKind, Layer};
        if !self.use_committed_data {
            anyhow::ensure!(
                self.committed_data_rpc_url.is_none(),
                "A committed-data witness RPC requires use_committed_data=true"
            );
            return Ok(());
        }
        anyhow::ensure!(
            matches!(prover, ProverKind::Sharp | ProverKind::Mock) && *layer == Layer::L2,
            "Committed-data reads require the SHARP or Mock L2 local-aggregator path"
        );
        anyhow::ensure!(
            self.committed_data_rpc_url.is_some(),
            "Committed-data reads require a Madara witness RPC for SNOS replay"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Command {
        #[command(flatten)]
        snos: SNOSCliArgs,
    }

    // Strip environment bindings so the developer's shell cannot change parser assertions.
    fn parse(arguments: &[&str]) -> Result<Command, clap::Error> {
        use clap::{CommandFactory, FromArgMatches};
        let command = Command::command().mut_args(|arg| arg.env(None::<&str>));
        let matches = command.try_get_matches_from(arguments)?;
        Command::from_arg_matches(&matches)
    }

    #[test]
    fn committed_data_defaults_to_disabled() {
        assert!(!parse(&["test", "--rpc-for-snos", "http://localhost:9545"]).unwrap().snos.use_committed_data);
    }

    #[test]
    fn committed_data_enabled_accepts_a_witness_endpoint() {
        let args = parse(&[
            "test",
            "--rpc-for-snos",
            "http://localhost:9545",
            "--use-committed-data",
            "--committed-data-rpc-url",
            "https://localhost:9944",
        ])
        .unwrap()
        .snos;
        assert_eq!(args.committed_data_rpc_url.unwrap().as_str(), "https://localhost:9944/");
    }

    #[test]
    fn committed_data_witness_endpoint_requires_permission() {
        assert!(parse(&[
            "test",
            "--rpc-for-snos",
            "http://localhost:9545",
            "--committed-data-rpc-url",
            "http://localhost:9944",
        ])
        .is_err());
    }

    #[test]
    fn committed_data_witness_endpoint_rejects_unsupported_transport() {
        let error = parse(&[
            "test",
            "--rpc-for-snos",
            "http://localhost:9545",
            "--use-committed-data",
            "--committed-data-rpc-url",
            "ftp://localhost/private",
        ])
        .err()
        .expect("unsupported transport must fail CLI parsing");
        assert!(error.to_string().contains("requires HTTP or HTTPS"));
    }
    #[test]
    fn committed_data_execution_path_configuration_matrix() {
        use crate::types::{params::prover::ProverKind, Layer};
        let mut args = parse(&["test", "--rpc-for-snos", "http://localhost:9545"]).unwrap().snos;
        for enabled in [false, true] {
            for endpoint in [None, Some(Url::parse("http://localhost:9944").unwrap())] {
                args.use_committed_data = enabled;
                args.committed_data_rpc_url = endpoint;
                for prover in [ProverKind::Sharp, ProverKind::Atlantic, ProverKind::Mock] {
                    for layer in [Layer::L2, Layer::L3] {
                        let expected = if enabled {
                            args.committed_data_rpc_url.is_some()
                                && matches!(prover, ProverKind::Sharp | ProverKind::Mock)
                                && layer == Layer::L2
                        } else {
                            args.committed_data_rpc_url.is_none()
                        };
                        assert_eq!(
                            args.validate_committed_data(prover, &layer).is_ok(),
                            expected,
                            "enabled={enabled}, endpoint={}, prover={prover:?}, layer={layer:?}",
                            args.committed_data_rpc_url.is_some()
                        );
                    }
                }
            }
        }
    }
}
