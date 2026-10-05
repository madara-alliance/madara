use crate::cli::snos::SNOSCliArgs;
use blockifier::blockifier_versioned_constants::VersionedConstants;
use url::Url;

#[derive(Debug, Clone)]
pub struct SNOSParams {
    /// Local execution permission forwarded to both SNOS replay and Cairo OS input.
    pub use_committed_data: bool,
    /// Operator-controlled endpoint for exact-root private witnesses, independent of Pathfinder.
    pub committed_data_rpc_url: Option<Url>,
    pub rpc_for_snos: Url,
    pub rpc_for_snos_backup: Option<Url>,
    pub snos_full_output: bool,
    pub versioned_constants: Option<VersionedConstants>,
}

impl From<SNOSCliArgs> for SNOSParams {
    fn from(args: SNOSCliArgs) -> Self {
        Self {
            use_committed_data: args.use_committed_data,
            committed_data_rpc_url: args.committed_data_rpc_url,
            rpc_for_snos: args.rpc_for_snos,
            rpc_for_snos_backup: args.rpc_for_snos_backup,
            snos_full_output: args.snos_full_output,
            versioned_constants: args.versioned_constants,
        }
    }
}
