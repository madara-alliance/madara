use std::path::PathBuf;

use blockifier::blockifier_versioned_constants::VersionedConstants;
use clap::Args;
use generate_pie::utils::load_versioned_constants;
use url::Url;

/// Loads an explicit constants override, rejecting unreadable or empty configuration files.
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

    /// Madara admin RPC supplying exact-root witnesses to SNOS; separate from Pathfinder.
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
}
