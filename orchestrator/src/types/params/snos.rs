use crate::cli::snos::SNOSCliArgs;
use blockifier::blockifier_versioned_constants::VersionedConstants;
use url::Url;

#[derive(Debug, Clone)]
pub struct SNOSParams {
    pub committed_data_activation_block: Option<u64>,
    pub committed_data_readers: starknet_api::committed_data::CommittedDataReaders,
    pub committed_data_rpc_url: Option<Url>,
    pub rpc_for_snos: Url,
    pub rpc_for_snos_backup: Option<Url>,
    pub snos_full_output: bool,
    pub versioned_constants: Option<VersionedConstants>,
}

impl From<SNOSCliArgs> for SNOSParams {
    fn from(args: SNOSCliArgs) -> Self {
        Self {
            committed_data_activation_block: args.committed_data_activation_block,
            committed_data_readers: args.committed_data_readers,
            committed_data_rpc_url: args.committed_data_rpc_url,
            rpc_for_snos: args.rpc_for_snos,
            rpc_for_snos_backup: args.rpc_for_snos_backup,
            snos_full_output: args.snos_full_output,
            versioned_constants: args.versioned_constants,
        }
    }
}
