use crate::cli::snos::SNOSCliArgs;
use blockifier::blockifier_versioned_constants::VersionedConstants;
use url::Url;

#[derive(Debug, Clone)]
pub struct SNOSParams {
    pub rpc_for_snos: Url,
    pub snos_full_output: bool,
    pub strk_fee_token_address: String,
    pub eth_fee_token_address: String,
    pub versioned_constants: Option<VersionedConstants>,
}

impl From<SNOSCliArgs> for SNOSParams {
    fn from(args: SNOSCliArgs) -> Self {
        Self {
            // For legacy compatibility: if rpc_for_snos is None, use a placeholder
            // In practice, this should never be None when using the legacy path
            // as the config/preset system is now required
            rpc_for_snos: args
                .rpc_for_snos
                .unwrap_or_else(|| "http://localhost:9944".parse().expect("Failed to parse default RPC URL")),
            snos_full_output: args.snos_full_output,
            strk_fee_token_address: args.strk_fee_token_address,
            eth_fee_token_address: args.eth_fee_token_address,
            versioned_constants: args.versioned_constants,
        }
    }
}
